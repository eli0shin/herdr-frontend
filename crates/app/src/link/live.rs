//! The real core: herdr-client's [`Supervisor`] and [`MachineWatcher`], calls answered through
//! [`Pending::try_take`], and one thread that reads every attach stream.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::task::{Context, Poll, Wake};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine;
use herdr_client::{
    AttachStream, Connection, Error, MachineUpdate, MachineWatcher, POLL_EVERY, Pending,
    StreamControl, StreamItem, Supervisor, SupervisorOptions, Transport, Update,
};
use herdr_proto::methods::{
    DocGetResult, DocSetResult, ExecResult, RunResult, TerminalAttach, TerminalPut,
};
use herdr_proto::{CloseReason, RpcError, Terminal};
use herdr_shared::Round;
use model::{Event, Request, TerminalUpdate, Token};

use super::{Link, News, Out, Upload, unavailable};

/// How much of one stream's output the reader moves before it looks at the others.
const TURN_BYTES: usize = 1 << 20;

/// The largest file `terminal.put` takes (`herdr-core-protocol.md` §3).
const PUT_MAX_BYTES: u64 = 8 << 20;

/// herdr-core through herdr-client. Nothing connects until [`Link::start`].
pub struct Live {
    options: SupervisorOptions,
    supervisor: Option<Supervisor>,
    machines: Option<MachineWatcher>,
    streams: Option<Streams>,
    /// The connection the news so far belongs to; answers from older ones are dropped.
    generation: u64,
    calls: Vec<(Token, CallAnswer)>,
    docs: Vec<(Option<String>, DocAnswer)>,
    controls: HashMap<String, StreamControl>,
    /// Answers known at once (no connection), for the next poll.
    ready: Vec<News>,
    /// Files copied beside terminals, from the threads that send them.
    puts: (Sender<News>, Receiver<News>),
    wake: Option<rune::Waker>,
}

enum CallAnswer {
    Run(Pending<RunResult>),
    Record(Pending<Terminal>),
    Exec(Pending<ExecResult>),
}

enum DocAnswer {
    Read(Pending<DocGetResult>),
    Write(Pending<DocSetResult>),
}

impl Live {
    /// A link that connects through `transport` (`hc connect`, or the socket).
    pub fn new(transport: Transport) -> Live {
        Live::with_options(SupervisorOptions::new(transport))
    }

    pub fn with_options(options: SupervisorOptions) -> Live {
        Live {
            options,
            supervisor: None,
            machines: None,
            streams: None,
            generation: 0,
            calls: Vec::new(),
            docs: Vec::new(),
            controls: HashMap::new(),
            ready: Vec::new(),
            puts: mpsc::channel(),
            wake: None,
        }
    }

    fn connection(&self) -> Option<Connection> {
        let connection = self.supervisor.as_ref()?.connection()?;
        (connection.generation() == self.generation).then_some(connection)
    }
}

/// A client error in the model's terms.
fn rpc(error: Error) -> RpcError {
    match error {
        Error::Rpc(error) => error,
        other => unavailable(other.to_string()),
    }
}

impl Link for Live {
    fn start(&mut self, wake: rune::Waker) {
        if self.supervisor.is_some() {
            return;
        }
        self.wake = Some(wake.clone());
        let supervisor = Supervisor::start(self.options.clone(), wake.clone());
        self.machines = Some(MachineWatcher::start(
            supervisor.handle(),
            POLL_EVERY,
            wake.clone(),
        ));
        self.streams = Some(Streams::start(wake));
        self.supervisor = Some(supervisor);
    }

    fn poll(&mut self, news: &mut Vec<News>) {
        news.append(&mut self.ready);
        news.extend(self.puts.1.try_iter());
        let Some(supervisor) = &self.supervisor else {
            return;
        };
        while let Some(update) = supervisor.try_recv() {
            match update {
                Update::Connected { generation, .. } => {
                    self.generation = generation;
                    self.controls.clear();
                    news.push(News::Connected);
                    let update = supervisor.with_terminals(|set| TerminalUpdate {
                        resynced: true,
                        terminals: set.iter().cloned().collect(),
                        ..Default::default()
                    });
                    news.push(News::Event(Event::Terminals(update)));
                }
                Update::Changed {
                    generation,
                    changes,
                } if generation == self.generation => {
                    let update = supervisor
                        .with_terminals(|set| TerminalUpdate::from_changes(&changes, set));
                    news.push(News::Event(Event::Terminals(update)));
                    if !changes.restarted.is_empty() {
                        news.push(News::Restarted(changes.restarted.into_iter().collect()));
                    }
                }
                Update::Changed { .. } => {}
                Update::Lost { generation, error } if generation == self.generation => {
                    self.controls.clear();
                    news.push(News::Lost(error.to_string()));
                }
                Update::Lost { .. } => {}
                Update::Unavailable { error, .. } => {
                    news.push(News::Unavailable(error.to_string()))
                }
            }
        }
        if let Some(machines) = &self.machines {
            while let Some(update) = machines.try_recv() {
                if let MachineUpdate::Machines(list) = update
                    && list.generation == self.generation
                {
                    let views = list.machines.into_iter().map(|m| m.view).collect();
                    news.push(News::Event(Event::Machines(views)));
                }
            }
        }
        self.calls.retain_mut(|(token, answer)| {
            let token = *token;
            let event = match answer {
                CallAnswer::Run(p) => p.try_take().map(|r| Event::Reply {
                    token,
                    result: r.map(|r| Box::new(r.terminal)).map_err(rpc),
                }),
                CallAnswer::Record(p) => p.try_take().map(|r| Event::Reply {
                    token,
                    result: r.map(Box::new).map_err(rpc),
                }),
                CallAnswer::Exec(p) => p.try_take().map(|r| Event::Executed {
                    token,
                    result: r.map_err(rpc),
                }),
            };
            let Some(event) = event else { return true };
            news.push(News::Event(event));
            false
        });
        self.docs.retain_mut(|(machine, answer)| {
            let event = match answer {
                DocAnswer::Read(p) => p.try_take().map(|result| Event::DocRead {
                    machine: machine.clone(),
                    result: result.map_err(rpc),
                }),
                DocAnswer::Write(p) => p.try_take().map(|result| Event::DocWritten {
                    machine: machine.clone(),
                    result: result.map_err(rpc),
                }),
            };
            let Some(event) = event else { return true };
            news.push(News::Event(event));
            false
        });
        if let Some(streams) = &self.streams {
            while let Ok(item) = streams.news.try_recv() {
                match item {
                    StreamNews::Attached { address, result } => {
                        let result = match result {
                            Ok((record, control)) if control.generation() == self.generation => {
                                self.controls.insert(address.clone(), control);
                                Ok(record)
                            }
                            Ok(_) => Err("attached on an older connection".into()),
                            Err(error) => Err(error),
                        };
                        news.push(News::Attached { address, result });
                    }
                    StreamNews::Ended { address, reason } => {
                        self.controls.remove(&address);
                        news.push(News::Ended { address, reason });
                    }
                }
            }
        }
    }

    fn call(&mut self, token: Token, request: Request) {
        let Some(connection) = self.connection() else {
            let error = unavailable("not connected");
            self.ready.push(News::Event(match request {
                Request::Exec(_) => Event::Executed {
                    token,
                    result: Err(error),
                },
                _ => Event::Reply {
                    token,
                    result: Err(error),
                },
            }));
            return;
        };
        let answer = match request {
            Request::Run(run) => CallAnswer::Run(connection.call(*run)),
            Request::Get(get) => CallAnswer::Record(connection.call(get)),
            Request::Kill(kill) => CallAnswer::Record(connection.call(kill)),
            Request::Keep(keep) => CallAnswer::Record(connection.call(keep)),
            Request::Label(label) => CallAnswer::Record(connection.call(label)),
            Request::Exec(exec) => CallAnswer::Exec(connection.call(exec)),
        };
        self.calls.push((token, answer));
    }

    fn doc(&mut self, machine: Option<String>, round: Round) {
        let Some(connection) = self.connection() else {
            let error = unavailable("not connected");
            self.ready.push(News::Event(match round {
                Round::Read(_) => Event::DocRead {
                    machine,
                    result: Err(error),
                },
                Round::Write(_) => Event::DocWritten {
                    machine,
                    result: Err(error),
                },
            }));
            return;
        };
        let answer = match round {
            Round::Read(get) => DocAnswer::Read(connection.call(get)),
            Round::Write(set) => DocAnswer::Write(connection.call(set)),
        };
        self.docs.push((machine, answer));
    }

    fn attach(&mut self, params: TerminalAttach, writer: terminal::Writer, fresh: bool) {
        let address = params.terminal.clone();
        let (Some(connection), Some(streams)) = (self.connection(), &self.streams) else {
            self.ready.push(News::Attached {
                address,
                result: Err("not connected".into()),
            });
            return;
        };
        self.controls.remove(&address);
        let pending = connection.attach_with(params, streams.wake());
        streams.command(Command::Attach {
            address,
            pending,
            writer,
            fresh,
        });
    }

    fn send(&mut self, address: &str, out: Out) {
        let Some(control) = self.controls.get(address) else {
            return;
        };
        match out {
            Out::Input(bytes) => control.input(&bytes),
            Out::Automatic(bytes) => control.automatic(&bytes),
            Out::Resize { cols, rows } => control.resize(cols, rows),
            Out::Focus(gained) => control.focus(gained),
            Out::Facts(display) => control.facts(&display),
        }
    }

    fn detach(&mut self, address: &str) {
        self.controls.remove(address);
        if let Some(streams) = &self.streams {
            streams.command(Command::Detach(address.to_owned()));
        }
    }

    fn put(&mut self, address: &str, uploads: Vec<Upload>) {
        let Some(connection) = self.connection() else {
            return;
        };
        let (news, wake, address) = (self.puts.0.clone(), self.wake.clone(), address.to_owned());
        // Reading and sending the files is I/O: never on the UI thread.
        let _ = std::thread::Builder::new()
            .name("hc-put".into())
            .spawn(move || {
                let paths = uploads
                    .iter()
                    .filter_map(|upload| put(&connection, &address, upload))
                    .collect();
                let _ = news.send(News::Put { address, paths });
                if let Some(wake) = wake {
                    wake();
                }
            });
    }
}

/// Sends one file beside a terminal; where it landed, or `None` when it couldn't be read, is
/// too big, or the core refused it.
fn put(connection: &Connection, address: &str, upload: &Upload) -> Option<String> {
    let data = match upload {
        Upload::File(file) => {
            if std::fs::metadata(file).ok()?.len() > PUT_MAX_BYTES {
                return None;
            }
            std::fs::read(file).ok()?.into()
        }
        Upload::Bytes { data, .. } if data.len() as u64 > PUT_MAX_BYTES => return None,
        Upload::Bytes { data, .. } => data.clone(),
    };
    let put = TerminalPut {
        terminal: address.to_owned(),
        name: upload.name()?,
        data: base64::engine::general_purpose::STANDARD.encode(data),
        ..Default::default()
    };
    connection.call(put).wait().ok().map(|r| r.path)
}

/// Wakes the stream reader: its streams have something, or an attach was answered.
#[derive(Default)]
struct Signal {
    set: Mutex<bool>,
    changed: Condvar,
}

impl Signal {
    fn notify(&self) {
        *self.set.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_one();
    }

    /// Waits until notified, or `timeout`.
    fn wait(&self, timeout: Duration) {
        let set = self.set.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut set, _) = self
            .changed
            .wait_timeout_while(set, timeout, |set| !*set)
            .unwrap_or_else(PoisonError::into_inner);
        *set = false;
    }
}

impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.notify();
    }
}

enum Command {
    Attach {
        address: String,
        pending: Pending<AttachStream>,
        writer: terminal::Writer,
        fresh: bool,
    },
    Detach(String),
    Stop,
}

enum StreamNews {
    Attached {
        address: String,
        result: Result<(Box<Terminal>, StreamControl), String>,
    },
    Ended {
        address: String,
        reason: Option<CloseReason>,
    },
}

/// The thread that reads every attach stream: it moves each terminal's output into its terminal
/// worker, which returns the credit once it parsed the bytes (rule 2), so the UI only hears when a
/// stream starts or ends, and a worker that falls behind holds its stream to the credit window.
struct Streams {
    signal: Arc<Signal>,
    commands: Sender<Command>,
    news: Receiver<StreamNews>,
    thread: Option<JoinHandle<()>>,
}

impl Streams {
    fn start(wake: rune::Waker) -> Streams {
        let signal = Arc::new(Signal::default());
        let (commands, inbox) = mpsc::channel();
        let (outbox, news) = mpsc::channel();
        let thread = {
            let signal = signal.clone();
            std::thread::Builder::new()
                .name("hc-streams".into())
                .spawn(move || read_streams(&signal, &inbox, &outbox, &wake))
                .expect("spawning the stream reader")
        };
        Streams {
            signal,
            commands,
            news,
            thread: Some(thread),
        }
    }

    /// The wake hook for a stream it reads.
    fn wake(&self) -> herdr_client::Wake {
        let signal = self.signal.clone();
        Arc::new(move || signal.notify())
    }

    fn command(&self, command: Command) {
        let _ = self.commands.send(command);
        self.signal.notify();
    }
}

impl Drop for Streams {
    fn drop(&mut self) {
        self.command(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Reading {
    stream: AttachStream,
    writer: terminal::Writer,
}

fn read_streams(
    signal: &Arc<Signal>,
    inbox: &Receiver<Command>,
    outbox: &Sender<StreamNews>,
    wake: &rune::Waker,
) {
    let post = |news: StreamNews| {
        let _ = outbox.send(news);
        wake();
    };
    let waker = std::task::Waker::from(signal.clone());
    let mut cx = Context::from_waker(&waker);
    let mut attaching: Vec<(String, Pending<AttachStream>, terminal::Writer, bool)> = Vec::new();
    let mut reading: HashMap<String, Reading> = HashMap::new();
    let mut more = false;
    loop {
        if !more {
            signal.wait(Duration::from_millis(250));
        }
        more = false;
        while let Ok(command) = inbox.try_recv() {
            match command {
                Command::Attach {
                    address,
                    pending,
                    writer,
                    fresh,
                } => {
                    reading.remove(&address);
                    attaching.retain(|(a, ..)| *a != address);
                    attaching.push((address, pending, writer, fresh));
                }
                Command::Detach(address) => {
                    reading.remove(&address);
                    attaching.retain(|(a, ..)| *a != address);
                }
                Command::Stop => return,
            }
        }
        attaching.retain_mut(|(address, pending, writer, fresh)| {
            let Poll::Ready(result) = Pin::new(&mut *pending).poll(&mut cx) else {
                return true;
            };
            match result {
                Ok(stream) => {
                    if *fresh {
                        writer.reset();
                    }
                    post(StreamNews::Attached {
                        address: address.clone(),
                        result: Ok((Box::new(stream.terminal().clone()), stream.control())),
                    });
                    reading.insert(
                        address.clone(),
                        Reading {
                            stream,
                            writer: writer.clone(),
                        },
                    );
                    // Its first paint may already be waiting.
                    more = true;
                }
                Err(error) => post(StreamNews::Attached {
                    address: address.clone(),
                    result: Err(error.to_string()),
                }),
            }
            false
        });
        reading.retain(|address, r| {
            let mut moved = 0;
            while moved < TURN_BYTES {
                match r.stream.try_recv() {
                    Some(StreamItem::Data(bytes)) => {
                        moved += bytes.len();
                        let n = bytes.len();
                        // Credit once the worker parsed them, not when they're queued: a parser
                        // that falls behind then holds the core to its credit window.
                        let control = r.stream.control();
                        r.writer
                            .write_then(bytes, Box::new(move || control.credit(n)));
                    }
                    // Only observers hear RESIZE; this client follows `size` events.
                    Some(StreamItem::Resize { .. }) => {}
                    Some(StreamItem::Closed(reason)) => {
                        post(StreamNews::Ended {
                            address: address.clone(),
                            reason: Some(reason),
                        });
                        return false;
                    }
                    Some(StreamItem::Lost) => {
                        post(StreamNews::Ended {
                            address: address.clone(),
                            reason: None,
                        });
                        return false;
                    }
                    None => return true,
                }
            }
            more = true;
            true
        });
    }
}
