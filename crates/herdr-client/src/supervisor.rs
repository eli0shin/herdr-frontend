//! The supervisor owns the connection: it connects, subscribes for a fresh snapshot, pumps events
//! into a [`TerminalSet`], and reconnects with backoff when the connection drops
//! (`herdr-core-protocol.md` §1 "Reconnect").
//!
//! Each connection gets the next generation. Every [`Update`], [`Connection`] and stream carries
//! it, so answers from an older connection can be told apart and ignored.

use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use herdr_proto::methods::EventsSubscribe;
use herdr_proto::{Changes, Hello, TerminalSet};

use crate::connection::{Canceller, ConnectOptions, Connection};
use crate::error::Error;
use crate::stream::{EventItem, EventStream};
use crate::transport::Transport;
use crate::{Wake, no_wake};

/// How long the events pump waits before checking whether to stop.
const PUMP_TICK: Duration = Duration::from_millis(250);

/// Delays between connect attempts: `first`, doubling up to `max` (herdr-app retries about once a
/// second).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    pub first: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff {
            first: Duration::from_millis(250),
            max: Duration::from_secs(1),
        }
    }
}

impl Backoff {
    /// The delay before attempt `failures + 1`.
    pub fn delay(&self, failures: u32) -> Duration {
        let factor = 1u32.checked_shl(failures.min(16)).unwrap_or(u32::MAX);
        self.first.saturating_mul(factor).min(self.max)
    }
}

/// What the supervisor needs to connect.
#[derive(Clone, Debug)]
pub struct SupervisorOptions {
    pub transport: Transport,
    /// The handshake checks and call timeout. `generation` and `wake` are the supervisor's.
    pub connect: ConnectOptions,
    pub backoff: Backoff,
}

impl SupervisorOptions {
    pub fn new(transport: Transport) -> Self {
        SupervisorOptions {
            transport,
            connect: ConnectOptions::default(),
            backoff: Backoff::default(),
        }
    }
}

/// What happened, in order. Drain them with [`Supervisor::try_recv`] after the wake hook fires.
#[derive(Clone, Debug)]
pub enum Update {
    /// A new connection is up and subscribed, and its snapshot replaced every terminal. Read every
    /// document again, and reattach every view on [`Supervisor::connection`] with
    /// `resumed: true` (rule 8).
    Connected { generation: u64, hello: Box<Hello> },
    /// Events changed the terminals ([`Supervisor::with_terminals`] has them). With
    /// `changes.resynced`, a `missed` snapshot replaced everything: read every document again.
    Changed { generation: u64, changes: Changes },
    /// The connection dropped. Calls on it failed and its streams got `Lost`; views stay and wait.
    Lost { generation: u64, error: Error },
    /// A connect attempt failed; the next one is in `retry_in`.
    Unavailable { error: Error, retry_in: Duration },
}

/// Runs the connection on its own thread. Dropping it stops the thread and closes the connection.
pub struct Supervisor {
    shared: Arc<Shared>,
    updates: mpsc::Receiver<Update>,
    thread: Option<JoinHandle<()>>,
}

/// A cheap, shareable view of the supervisor for other threads (a [`crate::MachineWatcher`]).
#[derive(Clone)]
pub struct SupervisorHandle {
    shared: Arc<Shared>,
}

/// Called when the connection or the terminal list changes, so watchers poll at once.
pub(crate) type Poke = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    wake: Wake,
    pokes: Mutex<Vec<Weak<dyn Fn() + Send + Sync>>>,
}

#[derive(Default)]
struct State {
    stop: bool,
    generation: u64,
    connection: Option<Connection>,
    /// The connection being opened, until it is subscribed: stopping ends its handshake or
    /// subscribe wait at once instead of waiting out their timeouts.
    opening: Option<Canceller>,
    terminals: TerminalSet,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Supervisor {
    /// Starts connecting at once. `wake` is called from the supervisor's thread whenever an
    /// [`Update`] is ready, and is the wake hook of every connection it opens.
    pub fn start(options: SupervisorOptions, wake: Wake) -> Supervisor {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            wake,
            pokes: Mutex::new(Vec::new()),
        });
        let (tx, updates) = mpsc::channel();
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("hc-supervisor".into())
                .spawn(move || run(&shared, &options, &tx))
                .expect("spawning the supervisor thread")
        };
        Supervisor {
            shared,
            updates,
            thread: Some(thread),
        }
    }

    /// The next update, if one is waiting.
    pub fn try_recv(&self) -> Option<Update> {
        self.updates.try_recv().ok()
    }

    /// The next update, waiting up to `timeout` (worker threads and tests).
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Update> {
        self.updates.recv_timeout(timeout).ok()
    }

    pub fn handle(&self) -> SupervisorHandle {
        SupervisorHandle {
            shared: self.shared.clone(),
        }
    }

    /// The current connection, if one is up.
    pub fn connection(&self) -> Option<Connection> {
        self.handle().connection()
    }

    /// The current connection's generation; 0 before the first.
    pub fn generation(&self) -> u64 {
        self.handle().generation()
    }

    /// Reads the terminals, kept current by the events pump. Keep `f` short: the pump waits.
    pub fn with_terminals<R>(&self, f: impl FnOnce(&TerminalSet) -> R) -> R {
        self.handle().with_terminals(f)
    }

    /// Stops the thread and closes the connection.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let (connection, opening) = {
            let mut state = lock(&self.shared.state);
            state.stop = true;
            (state.connection.take(), state.opening.take())
        };
        self.shared.changed.notify_all();
        if let Some(opening) = opening {
            opening.cancel();
        }
        if let Some(connection) = connection {
            connection.close();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl SupervisorHandle {
    pub fn connection(&self) -> Option<Connection> {
        lock(&self.shared.state).connection.clone()
    }

    pub fn generation(&self) -> u64 {
        lock(&self.shared.state).generation
    }

    pub fn with_terminals<R>(&self, f: impl FnOnce(&TerminalSet) -> R) -> R {
        f(&lock(&self.shared.state).terminals)
    }

    /// Calls `poke` on every new connection and whenever terminals come or go. It is dropped when
    /// the last strong reference to it goes.
    pub(crate) fn on_poke(&self, poke: &Poke) {
        let mut pokes = lock(&self.shared.pokes);
        pokes.retain(|p| p.strong_count() > 0);
        pokes.push(Arc::downgrade(poke));
    }
}

impl Shared {
    fn stopped(&self) -> bool {
        lock(&self.state).stop
    }

    fn poke(&self) {
        let pokes: Vec<_> = lock(&self.pokes).iter().filter_map(Weak::upgrade).collect();
        for poke in pokes {
            poke();
        }
    }

    /// Sleeps `delay` unless stopped meanwhile; true if stopped.
    fn sleep(&self, delay: Duration) -> bool {
        let state = lock(&self.state);
        let (state, _) = self
            .changed
            .wait_timeout_while(state, delay, |s| !s.stop)
            .unwrap_or_else(PoisonError::into_inner);
        state.stop
    }
}

fn run(shared: &Shared, options: &SupervisorOptions, updates: &mpsc::Sender<Update>) {
    let post = |update: Update| {
        let _ = updates.send(update);
        (shared.wake)();
    };
    let mut failures = 0u32;
    while !shared.stopped() {
        match connect(shared, options) {
            Ok((connection, events)) => {
                failures = 0;
                let generation = connection.generation();
                post(Update::Connected {
                    generation,
                    hello: Box::new(connection.hello().clone()),
                });
                shared.poke();
                let error = pump(shared, events, generation, &post);
                {
                    let mut state = lock(&shared.state);
                    if state
                        .connection
                        .as_ref()
                        .is_some_and(|c| c.generation() == generation)
                    {
                        state.connection = None;
                    }
                }
                connection.close();
                if shared.stopped() {
                    return;
                }
                post(Update::Lost { generation, error });
            }
            Err(error) => {
                let retry_in = options.backoff.delay(failures);
                failures = failures.saturating_add(1);
                if shared.stopped() {
                    return;
                }
                post(Update::Unavailable { error, retry_in });
                if shared.sleep(retry_in) {
                    return;
                }
            }
        }
    }
}

/// Opens the next generation's connection and subscribes on it (rule 3: before any document is
/// read). The snapshot replaces every terminal, since `seq` restarts with each core.
fn connect(
    shared: &Shared,
    options: &SupervisorOptions,
) -> Result<(Connection, EventStream), Error> {
    let generation = lock(&shared.state).generation + 1;
    let connect = ConnectOptions {
        generation,
        wake: shared.wake.clone(),
        ..options.connect.clone()
    };
    // Either this sees `stop`, or `shutdown` sees the canceller: a connect never outlives a stop.
    let opening = |canceller: Canceller| {
        let mut state = lock(&shared.state);
        state.opening = Some(canceller);
        !state.stop
    };
    let subscribed = Connection::open_cancellable(&options.transport, &connect, &opening).and_then(
        |connection| {
            connection
                .subscribe_with(EventsSubscribe::default(), no_wake())
                .wait()
                .map(|subscribed| (connection.clone(), subscribed))
                .inspect_err(|_| connection.close())
        },
    );
    let mut state = lock(&shared.state);
    state.opening = None;
    let (connection, (snapshot, events)) = subscribed?;
    if state.stop {
        drop(state);
        connection.close();
        return Err(Error::Closed);
    }
    state.generation = generation;
    state.terminals = TerminalSet::new(snapshot.seq, snapshot.terminals);
    state.connection = Some(connection.clone());
    Ok((connection, events))
}

/// Applies events until the stream ends. Each batch (what is ready at once) posts one `Changed`.
fn pump(shared: &Shared, mut events: EventStream, generation: u64, post: &dyn Fn(Update)) -> Error {
    loop {
        let Some(first) = events.next_timeout(PUMP_TICK) else {
            if shared.stopped() {
                return Error::Closed;
            }
            continue;
        };
        let mut changes = Changes::default();
        let mut membership = false;
        let mut end = None;
        {
            let mut state = lock(&shared.state);
            let mut item = Some(first);
            while let Some(next) = item.take() {
                match next {
                    EventItem::Notification(notification) => {
                        let before = state.terminals.len();
                        state.terminals.apply_into(notification, &mut changes);
                        membership |= state.terminals.len() != before;
                    }
                    EventItem::Malformed(_) => {}
                    EventItem::Closed(_) => {
                        end = Some(Error::Protocol("the core ended the subscription".into()));
                        break;
                    }
                    EventItem::Lost => {
                        end = Some(Error::Lost);
                        break;
                    }
                }
                item = events.try_next();
            }
        }
        membership |= !changes.removed.is_empty() || changes.resynced;
        if !changes.is_empty() {
            post(Update::Changed {
                generation,
                changes,
            });
        }
        if membership {
            shared.poke();
        }
        if let Some(end) = end {
            return end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_its_cap() {
        let backoff = Backoff {
            first: Duration::from_millis(250),
            max: Duration::from_secs(1),
        };
        let delays: Vec<_> = (0..5).map(|n| backoff.delay(n).as_millis()).collect();
        assert_eq!(delays, [250, 500, 1000, 1000, 1000]);
        assert_eq!(backoff.delay(u32::MAX), Duration::from_secs(1));
    }

    /// How long a stop may take: far below the 30 s handshake and 10 s call timeouts it must not
    /// wait out.
    const PROMPT: Duration = Duration::from_secs(2);

    fn timed(f: impl FnOnce()) -> Duration {
        let start = std::time::Instant::now();
        f();
        start.elapsed()
    }

    #[cfg(unix)]
    #[test]
    fn stopping_ends_a_connect_stuck_in_its_handshake() {
        let core = crate::test_core::FakeCore::serve(|peer| peer.hold());
        let supervisor = Supervisor::start(SupervisorOptions::new(core.transport()), no_wake());
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            supervisor.try_recv().is_none(),
            "the handshake should still be waiting"
        );
        let took = timed(|| supervisor.stop());
        assert!(took < PROMPT, "stop took {took:?}");
    }

    #[cfg(unix)]
    #[test]
    fn dropping_ends_a_connect_stuck_in_its_subscribe() {
        let core = crate::test_core::FakeCore::serve(|mut peer| {
            if peer.answer_hello().is_some() {
                peer.ignore();
            }
        });
        let supervisor = Supervisor::start(SupervisorOptions::new(core.transport()), no_wake());
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            supervisor.try_recv().is_none(),
            "the subscribe should still be waiting"
        );
        assert!(supervisor.connection().is_none());
        let took = timed(|| drop(supervisor));
        assert!(took < PROMPT, "drop took {took:?}");
    }

    #[test]
    fn a_missing_hc_reports_unavailable_and_retries() {
        let mut options =
            SupervisorOptions::new(Transport::Hc(crate::HcCommand::new("/nonexistent/hc")));
        options.backoff = Backoff {
            first: Duration::from_millis(10),
            max: Duration::from_millis(10),
        };
        let supervisor = Supervisor::start(options, no_wake());
        for _ in 0..2 {
            match supervisor.recv_timeout(Duration::from_secs(5)) {
                Some(Update::Unavailable {
                    error: Error::Io(_),
                    ..
                }) => {}
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(supervisor.generation(), 0);
        assert!(supervisor.connection().is_none());
        supervisor.stop();
    }
}
