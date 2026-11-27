//! One framed connection to a core (`herdr-core-protocol.md` §1 and §7 "Threading").
//!
//! Each connection runs a reader thread (it decodes frames, answers calls by id and routes stream
//! frames by channel, and never blocks), a writer thread (it drains a queue, so a core that stops
//! reading never blocks the caller), a timer thread (call timeouts and `$/cancelRequest`), and, for
//! `hc connect`, a stderr thread that keeps the startup error.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::time::{Duration, Instant};

use herdr_proto::frame::{CONTROL, MAX_PAYLOAD};
use herdr_proto::hello::feature;
use herdr_proto::methods::{AttachResult, EventsSubscribe, SubscribeResult, TerminalAttach};
use herdr_proto::rpc::{Call, Id, Incoming, Request, Response, cancel_request};
use herdr_proto::{
    Capabilities, Decoder, Frame, Hello, HelloParams, Kind, LineSplitter, MIN_CORE, Snapshot,
    Version, hello_line,
};
use serde_json::Value;

use crate::error::Error;
use crate::pending::{Completer, Pending, pending};
use crate::stream::{AttachStream, EventStream, StreamItem};
use crate::transport::{Closer, Opened, StderrLog, Transport};
use crate::{Wake, no_wake};

/// How long a call waits for its reply by default, as in herdr-app.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Extra credit granted to an events stream at subscribe, on top of the core's 256 KiB window, as
/// herdr-app does: a burst of events (a `missed` snapshot) shouldn't stall on the first round trip.
pub const EVENTS_EXTRA_CREDIT: u32 = 1 << 20;

/// Stream frames kept for channels with no route yet, across all channels.
const EARLY_LIMIT: usize = 8 << 20;

/// Bytes read from the transport at a time.
const READ_CHUNK: usize = 64 * 1024;

/// The methods herdr-frontend calls: herdr-app's minimum set (`app:src/hc.rs` `USES`) plus
/// `machine.list`. A core without one of them needs an update.
pub const USES: &[&str] = &[
    "terminal.run",
    "terminal.list",
    "terminal.kill",
    "terminal.label",
    "terminal.attach",
    "terminal.keep",
    "doc.get",
    "doc.set",
    "events.subscribe",
    "machine.list",
];

/// The `hello.features` this client can't work without.
pub const NEEDS: &[&str] = &[feature::FRAMED, feature::ATTACH, feature::EVENTS];

/// What a connection checks at the handshake, and how it behaves.
#[derive(Clone)]
pub struct ConnectOptions {
    /// The oldest core accepted (rule 10).
    pub min_version: Version,
    /// Methods the core must have.
    pub uses: Vec<String>,
    /// Features the core must have.
    pub needs: Vec<String>,
    /// How long the transport may take to answer `hello` (`hc connect` may start a core first).
    pub handshake_timeout: Duration,
    /// A call's timeout unless it asks for another.
    pub call_timeout: Duration,
    /// The supervisor's connection generation, carried by every handle.
    pub generation: u64,
    /// Called (from a connection thread) whenever a call is answered, a stream has something
    /// new, or the connection is lost. Rune's `Waker` fits.
    pub wake: Wake,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        ConnectOptions {
            min_version: MIN_CORE,
            uses: USES.iter().map(|m| (*m).to_owned()).collect(),
            needs: NEEDS.iter().map(|f| (*f).to_owned()).collect(),
            handshake_timeout: Duration::from_secs(30),
            call_timeout: CALL_TIMEOUT,
            generation: 1,
            wake: no_wake(),
        }
    }
}

impl std::fmt::Debug for ConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectOptions")
            .field("min_version", &self.min_version)
            .field("uses", &self.uses)
            .field("needs", &self.needs)
            .field("handshake_timeout", &self.handshake_timeout)
            .field("call_timeout", &self.call_timeout)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

/// A framed connection to a core. Cloning shares it; it closes when the last clone drops (or on
/// [`Connection::close`]). Streams opened on it end with `Lost` when it goes.
#[derive(Clone)]
pub struct Connection {
    handle: Arc<Handle>,
}

/// Closes the transport when the last `Connection` goes.
struct Handle {
    inner: Arc<Inner>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.inner.shutdown();
    }
}

/// What the connection's threads and its handles share.
pub(crate) struct Inner {
    generation: u64,
    wake: Wake,
    call_timeout: Duration,
    hello: OnceLock<Hello>,
    capabilities: OnceLock<Capabilities>,
    out: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    routes: Mutex<Routes>,
    gone: Condvar,
    next_id: AtomicU64,
    timers: Mutex<Timers>,
    timer_ready: Condvar,
    closer: Mutex<Option<Box<dyn Closer>>>,
    closing: AtomicBool,
    stderr: Option<StderrLog>,
}

#[derive(Default)]
struct Routes {
    /// Why the connection ended; `None` while it is up.
    dead: Option<Error>,
    calls: HashMap<Id, CallRoute>,
    streams: HashMap<u32, StreamRoute>,
    /// Frames for channels with no route yet, delivered when the route comes.
    early: HashMap<u32, Vec<StreamItem>>,
    early_bytes: usize,
    /// Channels this side CLOSEd: their frames are dropped until the core's CLOSE.
    detached: HashSet<u32>,
    /// Attach or subscribe calls that timed out: a late reply's channel is CLOSEd.
    abandoned: HashSet<Id>,
}

struct CallRoute {
    method: &'static str,
    timeout: Option<Duration>,
    deadline: Option<Instant>,
    opens_stream: Option<Wake>,
    complete: Box<dyn FnOnce(Result<Reply, Error>) + Send>,
}

/// A successful reply, and the stream it opened, if it opened one.
struct Reply {
    value: Value,
    stream: Option<mpsc::Receiver<StreamItem>>,
}

struct StreamRoute {
    tx: mpsc::Sender<StreamItem>,
    wake: Wake,
}

#[derive(Default)]
struct Timers {
    deadlines: BTreeSet<(Instant, Id)>,
    stop: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Connection {
    /// Opens `transport`, says `hello {framed: true, generation: 1}` and checks the reply against
    /// `options` (rule 10). Blocks until the core answers or `options.handshake_timeout` passes,
    /// so never call it on the UI thread.
    pub fn open(transport: &Transport, options: &ConnectOptions) -> Result<Connection, Error> {
        Connection::open_with(transport.open()?, options, &|_| true)
    }

    /// `open`, handing `opening` a [`Canceller`] before the handshake starts, so another thread
    /// can end a connect that is stuck waiting for `hello`. If `opening` returns false, the
    /// connect is abandoned at once with `Closed`.
    pub(crate) fn open_cancellable(
        transport: &Transport,
        options: &ConnectOptions,
        opening: &dyn Fn(Canceller) -> bool,
    ) -> Result<Connection, Error> {
        Connection::open_with(transport.open()?, options, opening)
    }

    pub(crate) fn open_with(
        opened: Opened,
        options: &ConnectOptions,
        opening: &dyn Fn(Canceller) -> bool,
    ) -> Result<Connection, Error> {
        let Opened {
            read,
            mut write,
            closer,
            stderr,
        } = opened;
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        let inner = Arc::new(Inner {
            generation: options.generation,
            wake: options.wake.clone(),
            call_timeout: options.call_timeout,
            hello: OnceLock::new(),
            capabilities: OnceLock::new(),
            out: Mutex::new(Some(out_tx)),
            routes: Mutex::new(Routes::default()),
            gone: Condvar::new(),
            next_id: AtomicU64::new(1),
            timers: Mutex::new(Timers::default()),
            timer_ready: Condvar::new(),
            closer: Mutex::new(Some(closer)),
            closing: AtomicBool::new(false),
            stderr,
        });
        let connection = Connection {
            handle: Arc::new(Handle {
                inner: inner.clone(),
            }),
        };

        if !opening(Canceller(inner.clone())) {
            inner.shutdown();
            return Err(Error::Closed);
        }

        // Plain mode until the hello reply: the request goes out before the writer starts.
        if let Err(e) = write.write_all(&hello_line()).and_then(|()| write.flush()) {
            return Err(inner.startup_error().unwrap_or(e.into()));
        }
        let (hello_tx, hello_rx) = mpsc::channel();
        spawn("hc-writer", move || write_frames(write, out_rx))?;
        let reader = inner.clone();
        spawn("hc-reader", move || read_frames(&reader, read, hello_tx))?;

        let hello = match hello_rx.recv_timeout(options.handshake_timeout) {
            Ok(Ok(hello)) => hello,
            Ok(Err(e)) => {
                inner.shutdown();
                return Err(e);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                inner.shutdown();
                return Err(Error::Timeout {
                    method: HelloParams::METHOD.into(),
                    after: options.handshake_timeout,
                });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                inner.shutdown();
                return Err(inner.startup_error().unwrap_or(Error::Lost));
            }
        };
        let uses: Vec<&str> = options.uses.iter().map(String::as_str).collect();
        if let Err(mismatch) = hello.check(options.min_version, &uses) {
            inner.shutdown();
            return Err(mismatch.into());
        }
        if !hello.framed {
            inner.shutdown();
            return Err(Error::Protocol(
                "the core did not switch to framed mode".into(),
            ));
        }
        let missing: Vec<String> = options
            .needs
            .iter()
            .filter(|f| !hello.has_feature(f))
            .cloned()
            .collect();
        if !missing.is_empty() {
            inner.shutdown();
            return Err(Error::MissingFeatures(missing));
        }
        let timers = inner.clone();
        spawn("hc-timer", move || run_timers(&timers))?;
        Ok(connection)
    }

    fn inner(&self) -> &Arc<Inner> {
        &self.handle.inner
    }

    /// The core's `hello` reply.
    pub fn hello(&self) -> &Hello {
        self.inner().hello()
    }

    pub fn capabilities(&self) -> Capabilities {
        self.inner().capabilities()
    }

    /// The supervisor's generation for this connection. Answers from an older one are stale.
    pub fn generation(&self) -> u64 {
        self.inner().generation
    }

    pub fn is_alive(&self) -> bool {
        lock(&self.inner().routes).dead.is_none()
    }

    /// Why the connection ended, once it has.
    pub fn lost(&self) -> Option<Error> {
        lock(&self.inner().routes).dead.clone()
    }

    /// Blocks until the connection ends or `timeout` passes; true if it ended.
    pub fn wait_lost(&self, timeout: Duration) -> bool {
        let routes = lock(&self.inner().routes);
        let (routes, _) = self
            .inner()
            .gone
            .wait_timeout_while(routes, timeout, |r| r.dead.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        routes.dead.is_some()
    }

    /// What `hc connect` wrote on stderr so far.
    pub fn stderr(&self) -> String {
        self.inner()
            .stderr
            .as_ref()
            .map(StderrLog::text)
            .unwrap_or_default()
    }

    /// Ends the connection now: pending calls fail with `Closed` and streams get `Lost`.
    pub fn close(&self) {
        self.inner().shutdown();
    }

    /// Calls a method, with the default timeout (10 s).
    pub fn call<C: Call>(&self, params: C) -> Pending<C::Result>
    where
        C::Result: Send + 'static,
    {
        self.call_timeout(params, Some(self.inner().call_timeout))
    }

    /// Calls a method. `None` waits as long as the core takes (`terminal.wait`, `machine.exec`):
    /// only a lost connection ends it.
    pub fn call_timeout<C: Call>(&self, params: C, timeout: Option<Duration>) -> Pending<C::Result>
    where
        C::Result: Send + 'static,
    {
        let (pending, completer) = pending();
        self.inner().request(&params, timeout, None, move |reply| {
            completer.complete(reply.and_then(|reply| decode::<C>(reply.value)));
        });
        pending
    }

    /// `terminal.attach`. The stream wakes the connection's wake hook; see [`Connection::attach_with`].
    pub fn attach(&self, params: TerminalAttach) -> Pending<AttachStream> {
        self.attach_with(params, self.inner().wake.clone())
    }

    /// `terminal.attach`, with the stream waking `wake` when it has something new: a no-op for a
    /// worker thread that blocks on it.
    pub fn attach_with(&self, params: TerminalAttach, wake: Wake) -> Pending<AttachStream> {
        let (pending, completer): (_, Completer<AttachStream>) = pending();
        let inner = self.inner().clone();
        let timeout = Some(inner.call_timeout);
        self.inner()
            .request(&params, timeout, Some(wake), move |reply| {
                completer.complete(reply.and_then(|reply| {
                    let rx = reply.stream.expect("an attach reply opens a stream");
                    let result: AttachResult = decode::<TerminalAttach>(reply.value)?;
                    let caps = inner.capabilities().stream(&result);
                    Ok(AttachStream::new(inner, result, caps, rx))
                }));
            });
        pending
    }

    /// `events.subscribe`: the snapshot and the events after it. The stream wakes the connection's
    /// wake hook; see [`Connection::subscribe_with`].
    pub fn subscribe(&self, params: EventsSubscribe) -> Pending<(Snapshot, EventStream)> {
        self.subscribe_with(params, self.inner().wake.clone())
    }

    /// `events.subscribe`, with the stream waking `wake` when it has something new.
    pub fn subscribe_with(
        &self,
        params: EventsSubscribe,
        wake: Wake,
    ) -> Pending<(Snapshot, EventStream)> {
        let (pending, completer): (_, Completer<(Snapshot, EventStream)>) = pending();
        let inner = self.inner().clone();
        let timeout = Some(inner.call_timeout);
        self.inner()
            .request(&params, timeout, Some(wake), move |reply| {
                completer.complete(reply.and_then(|reply| {
                    let rx = reply.stream.expect("a subscribe reply opens a stream");
                    let result: SubscribeResult = decode::<EventsSubscribe>(reply.value)?;
                    let channel = result.channel.expect("checked by the reader");
                    inner.send(Frame::credit(channel, EVENTS_EXTRA_CREDIT));
                    let snapshot = Snapshot {
                        seq: result.seq,
                        terminals: result.terminals,
                        extra: result.extra,
                    };
                    Ok((snapshot, EventStream::new(inner, channel, rx)))
                }));
            });
        pending
    }
}

/// Ends a connection from another thread, including one still in its handshake: whatever waits
/// on it (the hello, a call) fails with `Closed`.
#[derive(Clone)]
pub(crate) struct Canceller(Arc<Inner>);

impl Canceller {
    pub(crate) fn cancel(&self) {
        self.0.shutdown();
    }
}

fn decode<C: Call>(value: Value) -> Result<C::Result, Error> {
    Response {
        id: None,
        outcome: Ok(value),
    }
    .into_result::<C>()
    .map_err(Error::Rpc)
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Result<(), Error> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map(drop)
        .map_err(Error::from)
}

impl Inner {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    fn hello(&self) -> &Hello {
        self.hello.get().expect("set at the handshake")
    }

    fn capabilities(&self) -> Capabilities {
        *self
            .capabilities
            .get_or_init(|| self.hello().capabilities())
    }

    fn startup_error(&self) -> Option<Error> {
        self.stderr
            .as_ref()?
            .startup_error(Duration::from_secs(1))
            .map(Error::Startup)
    }

    /// Queues a frame for the writer. A dead connection drops it.
    pub(crate) fn send(&self, frame: Frame) {
        match frame.encode() {
            Ok(bytes) => self.send_bytes(bytes),
            Err(e) => debug_assert!(false, "callers split payloads: {e}"),
        }
    }

    fn send_bytes(&self, bytes: Vec<u8>) {
        if let Some(out) = lock(&self.out).as_ref() {
            let _ = out.send(bytes);
        }
    }

    /// Sends `payload` as frames of `kind`, split to fit the largest frame.
    pub(crate) fn send_split(&self, channel: u32, kind: Kind, payload: &[u8]) {
        for chunk in payload.chunks(MAX_PAYLOAD) {
            self.send(Frame::new(channel, kind, chunk));
        }
    }

    fn request<P: serde::Serialize + Call>(
        &self,
        params: &P,
        timeout: Option<Duration>,
        opens_stream: Option<Wake>,
        complete: impl FnOnce(Result<Reply, Error>) + Send + 'static,
    ) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = Request {
            jsonrpc: herdr_proto::rpc::JSONRPC.into(),
            id,
            method: P::METHOD.into(),
            params,
        };
        let frame = match Frame::data(CONTROL, request.to_bytes()).encode() {
            Ok(frame) => frame,
            Err(e) => {
                complete(Err(Error::Protocol(format!("{}: {e}", P::METHOD))));
                return;
            }
        };
        let deadline = timeout.map(|t| Instant::now() + t);
        {
            let mut routes = lock(&self.routes);
            if let Some(dead) = &routes.dead {
                let dead = dead.clone();
                drop(routes);
                complete(Err(dead));
                return;
            }
            routes.calls.insert(
                id,
                CallRoute {
                    method: P::METHOD,
                    timeout,
                    deadline,
                    opens_stream,
                    complete: Box::new(complete),
                },
            );
        }
        if let Some(deadline) = deadline {
            let mut timers = lock(&self.timers);
            let first = timers.deadlines.first().is_none_or(|(d, _)| deadline < *d);
            timers.deadlines.insert((deadline, id));
            if first {
                self.timer_ready.notify_one();
            }
        }
        self.send_bytes(frame);
    }

    /// Detaches a stream this side is done with: drops its route and CLOSEs its channel.
    pub(crate) fn detach(&self, channel: u32) {
        let mut routes = lock(&self.routes);
        if routes.dead.is_some() {
            return;
        }
        routes.streams.remove(&channel);
        routes.early.remove(&channel);
        routes.detached.insert(channel);
        drop(routes);
        self.send(Frame::close(channel));
    }

    /// Ends the transport. The reader then sees the end and fails everything still waiting.
    fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        if let Some(mut closer) = lock(&self.closer).take() {
            closer.close();
        }
    }

    /// The connection ended: fail every call, give every stream `Lost`, stop the writer and timer.
    fn die(&self, why: Error) {
        let why = if self.closing.load(Ordering::SeqCst) {
            Error::Closed
        } else {
            why
        };
        let (calls, streams) = {
            let mut routes = lock(&self.routes);
            if routes.dead.is_some() {
                return;
            }
            routes.dead = Some(why.clone());
            routes.early.clear();
            (
                std::mem::take(&mut routes.calls),
                std::mem::take(&mut routes.streams),
            )
        };
        self.gone.notify_all();
        lock(&self.out).take();
        {
            let mut timers = lock(&self.timers);
            timers.stop = true;
            timers.deadlines.clear();
        }
        self.timer_ready.notify_all();
        if let Some(mut closer) = lock(&self.closer).take() {
            closer.close();
        }
        for (_, route) in calls {
            (route.complete)(Err(why.clone()));
        }
        for (_, stream) in streams {
            let _ = stream.tx.send(StreamItem::Lost);
            (stream.wake)();
        }
        (self.wake)();
    }

    fn on_reply(&self, response: Response) {
        let Some(id) = response.id else { return };
        let mut routes = lock(&self.routes);
        let Some(route) = routes.calls.remove(&id) else {
            // A late reply to an attach or subscribe that timed out: its stream is unwanted.
            if routes.abandoned.remove(&id)
                && let Ok(value) = &response.outcome
                && let Some(channel) = stream_channel(value)
            {
                routes.early.remove(&channel);
                routes.detached.insert(channel);
                drop(routes);
                self.send(Frame::close(channel));
            }
            return;
        };
        let outcome = match (response.outcome, route.opens_stream) {
            (Err(e), _) => Err(Error::Rpc(e)),
            (Ok(value), None) => Ok(Reply {
                value,
                stream: None,
            }),
            (Ok(value), Some(wake)) => match stream_channel(&value) {
                // Register the route before reading on, so no frame of the stream is missed.
                Some(channel) => {
                    let (tx, rx) = mpsc::channel();
                    for item in routes.early.remove(&channel).unwrap_or_default() {
                        routes.early_bytes -= item.len();
                        let _ = tx.send(item);
                    }
                    routes.streams.insert(channel, StreamRoute { tx, wake });
                    Ok(Reply {
                        value,
                        stream: Some(rx),
                    })
                }
                None => Err(Error::Protocol("a stream reply without a channel".into())),
            },
        };
        drop(routes);
        if let Some(deadline) = route.deadline {
            lock(&self.timers).deadlines.remove(&(deadline, id));
        }
        (route.complete)(outcome);
        (self.wake)();
    }

    fn on_stream_frame(&self, frame: Frame) {
        let item = match frame.kind {
            Kind::Data => StreamItem::Data(frame.payload),
            Kind::Resize => match frame.resize_grid() {
                Some((cols, rows)) => StreamItem::Resize { cols, rows },
                None => return,
            },
            Kind::Close => StreamItem::Closed(frame.close_reason().unwrap_or_default()),
            // The core never sends the others to a viewer; skip what isn't known, as it does.
            _ => return,
        };
        let channel = frame.channel;
        let closing = matches!(item, StreamItem::Closed(_));
        let mut routes = lock(&self.routes);
        if routes.detached.contains(&channel) {
            if closing {
                routes.detached.remove(&channel);
            }
            return;
        }
        if let Some(stream) = routes.streams.get(&channel) {
            let wake = stream.wake.clone();
            if stream.tx.send(item).is_err() || closing {
                routes.streams.remove(&channel);
            }
            drop(routes);
            wake();
            return;
        }
        // No route yet: keep it for the reply that opens the channel.
        if routes.early_bytes + item.len() <= EARLY_LIMIT {
            routes.early_bytes += item.len();
            routes.early.entry(channel).or_default().push(item);
        }
    }

    fn expire(&self, id: Id) {
        let mut routes = lock(&self.routes);
        let Some(route) = routes.calls.remove(&id) else {
            return;
        };
        if route.opens_stream.is_some() {
            routes.abandoned.insert(id);
        }
        drop(routes);
        self.send(Frame::data(CONTROL, cancel_request(id)));
        (route.complete)(Err(Error::Timeout {
            method: route.method.into(),
            after: route.timeout.unwrap_or_default(),
        }));
        (self.wake)();
    }
}

fn stream_channel(value: &Value) -> Option<u32> {
    value
        .get("channel")
        .and_then(Value::as_u64)
        .and_then(|c| u32::try_from(c).ok())
        .filter(|c| *c != CONTROL)
}

fn write_frames(mut write: Box<dyn Write + Send>, frames: mpsc::Receiver<Vec<u8>>) {
    while let Ok(bytes) = frames.recv() {
        if write
            .write_all(&bytes)
            .and_then(|()| write.flush())
            .is_err()
        {
            break;
        }
    }
    // Ending stdin (or the socket's write half) tells the core this side is gone.
    drop(write);
}

fn read_frames(
    inner: &Inner,
    mut read: Box<dyn Read + Send>,
    hello: mpsc::Sender<Result<Hello, Error>>,
) {
    let mut buf = vec![0u8; READ_CHUNK];
    let mut decoder = Decoder::new();
    match read_hello(inner, &mut read, &mut buf) {
        Ok((reply, rest)) => {
            let _ = inner.hello.set(reply.clone());
            let _ = hello.send(Ok(reply));
            decoder.push(&rest);
        }
        Err(e) => {
            let e = match e {
                _ if inner.closing.load(Ordering::SeqCst) => Error::Closed,
                Error::Lost => inner.startup_error().unwrap_or(Error::Lost),
                e => e,
            };
            let _ = hello.send(Err(e.clone()));
            inner.die(e);
            return;
        }
    }
    drop(hello);
    let why = loop {
        loop {
            match decoder.next_frame() {
                Ok(Some(frame)) if frame.channel == CONTROL => {
                    if frame.kind == Kind::Data
                        && let Ok(Incoming::Response(response)) = Incoming::parse(&frame.payload)
                    {
                        inner.on_reply(response);
                    }
                }
                Ok(Some(frame)) => inner.on_stream_frame(frame),
                Ok(None) => break,
                Err(e) => {
                    inner.die(Error::Protocol(e.to_string()));
                    return;
                }
            }
        }
        match read.read(&mut buf) {
            Ok(0) => break Error::Lost,
            Ok(n) => decoder.push(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break Error::Lost,
        }
    };
    inner.die(why);
}

/// Reads the plain-mode hello reply line. Returns it and the framed bytes read after it.
fn read_hello(
    inner: &Inner,
    read: &mut Box<dyn Read + Send>,
    buf: &mut [u8],
) -> Result<(Hello, Vec<u8>), Error> {
    let mut lines = LineSplitter::new();
    let line = loop {
        if let Some(line) = lines.next_line() {
            break line.to_vec();
        }
        match read.read(buf) {
            Ok(0) => return Err(Error::Lost),
            Ok(n) => lines.push(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                return Err(if inner.closing.load(Ordering::SeqCst) {
                    Error::Closed
                } else {
                    e.into()
                });
            }
        }
    };
    let response = match Incoming::parse(&line) {
        Ok(Incoming::Response(response)) => response,
        _ => {
            return Err(Error::Protocol(format!(
                "not a hello reply: {}",
                String::from_utf8_lossy(&line)
            )));
        }
    };
    let hello = response.into_result::<HelloParams>()?;
    Ok((hello, lines.into_rest()))
}

fn run_timers(inner: &Inner) {
    let mut timers = lock(&inner.timers);
    loop {
        if timers.stop {
            return;
        }
        let now = Instant::now();
        match timers.deadlines.first().copied() {
            Some((deadline, id)) if deadline <= now => {
                timers.deadlines.remove(&(deadline, id));
                drop(timers);
                inner.expire(id);
                timers = lock(&inner.timers);
            }
            Some((deadline, _)) => {
                timers = inner
                    .timer_ready
                    .wait_timeout(timers, deadline - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            None => {
                timers = inner
                    .timer_ready
                    .wait(timers)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::EventItem;
    use herdr_proto::methods::{TerminalGet, TerminalWait};
    use herdr_proto::rpc::ErrorKind;
    use herdr_proto::{Mismatch, Notification};
    use serde_json::json;
    use std::io::{PipeReader, PipeWriter};

    /// The core's end of an in-memory connection, driven by the test.
    struct FakeCore {
        read: PipeReader,
        decoder: Decoder,
        write: Arc<Mutex<Option<PipeWriter>>>,
    }

    struct FakeCloser(Arc<Mutex<Option<PipeWriter>>>);

    impl Closer for FakeCloser {
        fn close(&mut self) {
            lock(&self.0).take();
        }
    }

    fn hello_result() -> Value {
        json!({
            "generation": 1, "version": "0.1.22", "core": {"id": "c1", "name": "fake"},
            "framed": true, "methods": USES,
            "features": ["framed", "attach", "events", "automatic_input", "resize_notifications"],
        })
    }

    fn fake_transport() -> (Opened, FakeCore) {
        let (client_read, core_write) = std::io::pipe().unwrap();
        let (core_read, client_write) = std::io::pipe().unwrap();
        let write = Arc::new(Mutex::new(Some(core_write)));
        let opened = Opened {
            read: Box::new(client_read),
            write: Box::new(client_write),
            closer: Box::new(FakeCloser(write.clone())),
            stderr: None,
        };
        let core = FakeCore {
            read: core_read,
            decoder: Decoder::new(),
            write,
        };
        (opened, core)
    }

    /// Opens a connection to a fake core that answers hello with `reply` (a whole message).
    fn open_fake(reply: Value, options: ConnectOptions) -> (Result<Connection, Error>, FakeCore) {
        let (opened, mut core) = fake_transport();
        let opening =
            std::thread::spawn(move || Connection::open_with(opened, &options, &|_| true));
        let hello = core.hello_line();
        assert_eq!(
            hello,
            json!({"jsonrpc": "2.0", "id": 0, "method": "hello", "params": {"framed": true, "generation": 1}})
        );
        core.send_raw(&[serde_json::to_vec(&reply).unwrap(), b"\n".to_vec()].concat());
        (opening.join().unwrap(), core)
    }

    fn connect_fake() -> (Connection, FakeCore) {
        let (conn, core) = open_fake(
            json!({"jsonrpc": "2.0", "id": 0, "result": hello_result()}),
            ConnectOptions::default(),
        );
        (conn.unwrap(), core)
    }

    impl FakeCore {
        fn hello_line(&mut self) -> Value {
            let mut line = Vec::new();
            let mut byte = [0u8];
            while self.read.read_exact(&mut byte).is_ok() && byte[0] != b'\n' {
                line.push(byte[0]);
            }
            serde_json::from_slice(&line).unwrap()
        }

        fn send_raw(&self, bytes: &[u8]) {
            if let Some(write) = lock(&self.write).as_mut() {
                write.write_all(bytes).unwrap();
            }
        }

        fn send(&self, frame: Frame) {
            self.send_raw(&frame.encode().unwrap());
        }

        fn reply(&self, id: &Value, result: Value) {
            let message = json!({"jsonrpc": "2.0", "id": id, "result": result});
            self.send(Frame::data(0, serde_json::to_vec(&message).unwrap()));
        }

        /// The next frame the client sent.
        fn frame(&mut self) -> Frame {
            let mut buf = [0u8; 4096];
            loop {
                if let Some(frame) = self.decoder.next_frame().unwrap() {
                    return frame;
                }
                let n = self.read.read(&mut buf).unwrap();
                assert!(n > 0, "the client hung up");
                self.decoder.push(&buf[..n]);
            }
        }

        /// The next JSON-RPC message the client sent.
        fn message(&mut self) -> Value {
            let frame = self.frame();
            assert_eq!((frame.channel, frame.kind), (0, Kind::Data), "{frame:?}");
            serde_json::from_slice(&frame.payload).unwrap()
        }

        fn hang_up(&self) {
            lock(&self.write).take();
        }
    }

    fn get(terminal: &str) -> TerminalGet {
        TerminalGet {
            terminal: terminal.into(),
            ..Default::default()
        }
    }

    fn attach(terminal: &str) -> TerminalAttach {
        TerminalAttach {
            terminal: terminal.into(),
            cols: 80,
            rows: 24,
            ..Default::default()
        }
    }

    fn data(item: Option<StreamItem>) -> Vec<u8> {
        match item {
            Some(StreamItem::Data(bytes)) => bytes,
            other => panic!("{other:?}"),
        }
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn the_handshake_reads_hello_and_capabilities() {
        let (conn, _core) = connect_fake();
        assert_eq!(conn.hello().version, "0.1.22");
        assert_eq!(conn.hello().core.name, "fake");
        assert_eq!(conn.generation(), 1);
        let caps = conn.capabilities();
        assert!(caps.framed && caps.attach && caps.events && caps.automatic_input);
        assert!(conn.is_alive());
    }

    #[test]
    fn the_handshake_refuses_incompatible_cores() {
        let mut old = hello_result();
        old["version"] = json!("0.1.21");
        let old = json!({"jsonrpc": "2.0", "id": 0, "result": old});
        let (conn, _core) = open_fake(old, ConnectOptions::default());
        assert!(
            matches!(conn, Err(Error::Incompatible(Mismatch::TooOld { .. }))),
            "{:?}",
            conn.err()
        );

        let rejected = json!({"jsonrpc": "2.0", "id": 0, "error": {"code": -32000, "message": "generation 1 unsupported", "data": {"kind": "unsupported_generation"}}});
        let (conn, _core) = open_fake(rejected, ConnectOptions::default());
        match conn {
            Err(e) => assert_eq!(e.kind(), Some(&ErrorKind::UnsupportedGeneration)),
            Ok(_) => panic!("connected"),
        }

        let mut no_attach = hello_result();
        no_attach["features"] = json!(["framed", "events"]);
        let (conn, _core) = open_fake(
            json!({"jsonrpc": "2.0", "id": 0, "result": no_attach}),
            ConnectOptions::default(),
        );
        assert!(
            matches!(&conn, Err(Error::MissingFeatures(f)) if f == &["attach"]),
            "{:?}",
            conn.err()
        );
    }

    #[test]
    fn a_core_that_closes_before_hello_is_lost() {
        let (opened, mut core) = fake_transport();
        let opening = std::thread::spawn(move || {
            Connection::open_with(opened, &ConnectOptions::default(), &|_| true)
        });
        core.hello_line();
        core.hang_up();
        assert!(matches!(opening.join().unwrap(), Err(Error::Lost)));
    }

    #[test]
    fn startup_errors_come_from_stderr() {
        let (mut opened, mut core) = fake_transport();
        let (err_read, mut err_write) = std::io::pipe().unwrap();
        opened.stderr = Some(StderrLog::capture(err_read).unwrap());
        let opening = std::thread::spawn(move || {
            Connection::open_with(opened, &ConnectOptions::default(), &|_| true)
        });
        core.hello_line();
        err_write
            .write_all(br#"{"error":{"kind":"config","message":"bad config","hint":"fix it"}}"#)
            .unwrap();
        drop(err_write);
        core.hang_up();
        match opening.join().unwrap() {
            Err(Error::Startup(e)) => {
                assert_eq!(e.kind, ErrorKind::Config);
                assert_eq!(e.to_string(), "bad config (fix it)");
            }
            other => panic!("{:?}", other.err()),
        }
    }

    #[test]
    fn replies_are_routed_by_id_in_any_order() {
        let (conn, mut core) = connect_fake();
        let first = conn.call(get("t1"));
        let second = conn.call(get("t2"));
        let a = core.message();
        let b = core.message();
        assert_eq!(a["method"], "terminal.get");
        assert_eq!(a["params"], json!({"terminal": "t1"}));
        assert_ne!(a["id"], b["id"]);
        core.reply(&b["id"], json!({"address": "t2"}));
        core.reply(&a["id"], json!({"address": "t1"}));
        assert_eq!(first.wait().unwrap().address, "t1");
        assert_eq!(second.wait().unwrap().address, "t2");

        let failing = conn.call(get("t9"));
        let request = core.message();
        let error = json!({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32000, "message": "no t9", "data": {"kind": "not_found"}}});
        core.send(Frame::data(0, serde_json::to_vec(&error).unwrap()));
        assert_eq!(
            failing.wait().unwrap_err().kind(),
            Some(&ErrorKind::NotFound)
        );
    }

    #[test]
    fn frames_before_the_attach_reply_are_kept() {
        let (conn, mut core) = connect_fake();
        let pending = conn.attach(attach("t1"));
        let request = core.message();
        assert_eq!(request["method"], "terminal.attach");
        core.send(Frame::data(3, b"early ".to_vec()));
        core.reply(
            &request["id"],
            json!({"channel": 3, "terminal": {"address": "t1"}, "automatic_input": true, "resize_notifications": false}),
        );
        core.send(Frame::data(3, b"late".to_vec()));
        let mut stream = pending.wait().unwrap();
        assert_eq!(stream.channel(), 3);
        assert_eq!(stream.terminal().address, "t1");
        assert!(stream.caps().automatic_input);
        assert_eq!(data(stream.recv_timeout(WAIT)), b"early ");
        assert_eq!(data(stream.recv_timeout(WAIT)), b"late");
    }

    #[test]
    fn a_stream_sends_input_resize_focus_facts_and_credit() {
        let (conn, mut core) = connect_fake();
        let pending = conn.attach(attach("t1"));
        let request = core.message();
        core.reply(
            &request["id"],
            json!({"channel": 5, "terminal": {"address": "t1"}, "automatic_input": false}),
        );
        let stream = pending.wait().unwrap();
        stream.input(b"ls\r");
        stream.automatic(b"\x1b[1;1R");
        stream.resize(100, 30);
        stream.focus(true);
        stream.facts(&herdr_proto::Display {
            cell: Some([8, 16]),
            ..Default::default()
        });
        stream.credit(1234);
        assert_eq!(core.frame(), Frame::data(5, b"ls\r".to_vec()));
        // The attach reply didn't confirm automatic input: engine replies go as DATA.
        assert_eq!(core.frame(), Frame::data(5, b"\x1b[1;1R".to_vec()));
        assert_eq!(core.frame(), Frame::resize(5, 100, 30));
        assert_eq!(core.frame(), Frame::focus(5, true));
        assert_eq!(core.frame().kind, Kind::Facts);
        assert_eq!(core.frame(), Frame::credit(5, 1234));
        drop(stream);
        assert_eq!(core.frame(), Frame::close(5));
    }

    #[test]
    fn automatic_input_goes_as_its_own_kind_when_confirmed() {
        let (conn, mut core) = connect_fake();
        let pending = conn.attach(attach("t1"));
        let request = core.message();
        core.reply(
            &request["id"],
            json!({"channel": 2, "terminal": {"address": "t1"}, "automatic_input": true}),
        );
        let stream = pending.wait().unwrap();
        stream.automatic(b"\x1b[?62c");
        assert_eq!(
            core.frame(),
            Frame::automatic_input(2, b"\x1b[?62c".to_vec())
        );
    }

    #[test]
    fn a_closed_stream_is_not_closed_again() {
        let (conn, mut core) = connect_fake();
        let pending = conn.attach(attach("t1"));
        let request = core.message();
        core.reply(
            &request["id"],
            json!({"channel": 4, "terminal": {"address": "t1"}}),
        );
        let mut stream = pending.wait().unwrap();
        core.send(Frame::new(
            4,
            Kind::Close,
            br#"{"reason":"exited","exit_code":0}"#.to_vec(),
        ));
        match stream.recv_timeout(WAIT) {
            Some(StreamItem::Closed(reason)) => assert_eq!(reason.exit_code, Some(0)),
            other => panic!("{other:?}"),
        }
        assert!(stream.is_ended());
        assert_eq!(stream.recv_timeout(Duration::from_millis(10)), None);
        drop(stream);
        // The next thing the core hears is the next call, not a CLOSE.
        let _next = conn.call(get("t1"));
        assert_eq!(core.message()["method"], "terminal.get");
    }

    #[test]
    fn a_timed_out_call_is_cancelled() {
        let (conn, mut core) = connect_fake();
        let pending = conn.call_timeout(
            TerminalWait {
                terminal: Some("t1".into()),
                ..Default::default()
            },
            Some(Duration::from_millis(50)),
        );
        let request = core.message();
        match pending.wait() {
            Err(Error::Timeout { method, after }) => {
                assert_eq!(method, "terminal.wait");
                assert_eq!(after, Duration::from_millis(50));
            }
            other => panic!("{other:?}"),
        }
        let cancel = core.message();
        assert_eq!(
            cancel,
            json!({"jsonrpc": "2.0", "method": "$/cancelRequest", "params": {"id": request["id"]}})
        );
        // A late reply is dropped, and the connection goes on.
        core.reply(&request["id"], json!({}));
        let next = conn.call(get("t1"));
        let request = core.message();
        core.reply(&request["id"], json!({"address": "t1"}));
        assert_eq!(next.wait().unwrap().address, "t1");
    }

    #[test]
    fn a_late_attach_reply_detaches_its_stream() {
        let (fast, mut core) = {
            let fast = ConnectOptions {
                call_timeout: Duration::from_millis(50),
                ..ConnectOptions::default()
            };
            let (conn, core) = open_fake(
                json!({"jsonrpc": "2.0", "id": 0, "result": hello_result()}),
                fast,
            );
            (conn.unwrap(), core)
        };
        let pending = fast.attach(attach("t1"));
        let request = core.message();
        assert!(matches!(pending.wait(), Err(Error::Timeout { .. })));
        assert_eq!(core.message()["method"], "$/cancelRequest");
        core.reply(
            &request["id"],
            json!({"channel": 7, "terminal": {"address": "t1"}}),
        );
        core.send(Frame::data(7, b"unwanted".to_vec()));
        assert_eq!(core.frame(), Frame::close(7));
    }

    #[test]
    fn losing_the_connection_fails_calls_and_streams() {
        let (conn, mut core) = connect_fake();
        let pending = conn.attach(attach("t1"));
        let request = core.message();
        core.reply(
            &request["id"],
            json!({"channel": 1, "terminal": {"address": "t1"}}),
        );
        let mut stream = pending.wait().unwrap();
        let call = conn.call(get("t1"));
        let _ = core.message();
        core.hang_up();
        assert!(matches!(call.wait(), Err(Error::Lost)));
        assert_eq!(stream.recv_timeout(WAIT), Some(StreamItem::Lost));
        assert!(conn.wait_lost(WAIT));
        assert!(matches!(conn.lost(), Some(Error::Lost)));
        assert!(matches!(conn.call(get("t1")).wait(), Err(Error::Lost)));
    }

    #[test]
    fn closing_fails_what_waits_with_closed() {
        let (conn, mut core) = connect_fake();
        let call = conn.call(get("t1"));
        let _ = core.message();
        conn.close();
        assert!(matches!(call.wait(), Err(Error::Closed)));
        assert!(!conn.is_alive());
    }

    #[test]
    fn the_wake_hook_fires_for_replies_and_stream_frames() {
        let wakes = Arc::new(AtomicU64::new(0));
        let counted = wakes.clone();
        let options = ConnectOptions {
            wake: Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
            ..ConnectOptions::default()
        };
        let (conn, mut core) = open_fake(
            json!({"jsonrpc": "2.0", "id": 0, "result": hello_result()}),
            options,
        );
        let conn = conn.unwrap();
        let mut call = conn.call(get("t1"));
        let request = core.message();
        core.reply(&request["id"], json!({"address": "t1"}));
        assert!(call.wait_timeout(WAIT).unwrap().is_ok());
        // The reader completes the call before it wakes, so a woken UI finds the reply: the wake
        // can land just after the wait returns.
        let deadline = std::time::Instant::now() + WAIT;
        while wakes.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(wakes.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn events_are_parsed_across_frames_and_credited() {
        let (conn, mut core) = connect_fake();
        let pending = conn.subscribe(EventsSubscribe::default());
        let request = core.message();
        assert_eq!(request["method"], "events.subscribe");
        core.reply(
            &request["id"],
            json!({"seq": 4, "terminals": [{"address": "t1"}], "channel": 9}),
        );
        let (snapshot, mut events) = pending.wait().unwrap();
        assert_eq!(snapshot.seq, 4);
        assert_eq!(snapshot.terminals[0].address, "t1");
        assert_eq!(core.frame(), Frame::credit(9, EVENTS_EXTRA_CREDIT));
        let line = br#"{"jsonrpc":"2.0","method":"heartbeat","params":{"seq":4}}"#;
        let (a, b) = line.split_at(20);
        core.send(Frame::data(9, a.to_vec()));
        core.send(Frame::data(9, [b, b"\nnot json\n"].concat()));
        assert_eq!(
            events.next_timeout(WAIT),
            Some(EventItem::Notification(Notification::Heartbeat { seq: 4 }))
        );
        assert!(matches!(
            events.next_timeout(WAIT),
            Some(EventItem::Malformed(_))
        ));
        assert_eq!(core.frame(), Frame::credit(9, a.len() as u32));
        assert_eq!(core.frame(), Frame::credit(9, (b.len() + 10) as u32));
        core.hang_up();
        assert_eq!(events.next_timeout(WAIT), Some(EventItem::Lost));
        assert_eq!(events.next_timeout(Duration::from_millis(10)), None);
    }
}
