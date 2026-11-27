//! An in-process core for headless scenes and tests. It answers on the next [`Link::poll`], so a
//! step's effects and their answers settle within one pump of the app, with no threads.

use std::cell::{RefCell, RefMut};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use herdr_proto::methods::{DocGetResult, DocSetResult, ExecResult, TerminalAttach};
use herdr_proto::{ErrorKind, Reason, RpcError, Terminal, TerminalState};
use herdr_shared::Round;
use model::{Event, Request, TerminalUpdate, Token};
use serde_json::Value;

use super::{Link, News, Out, Upload};

/// The fake core's state, open to the test that made it.
#[derive(Default)]
pub struct FakeCore {
    /// Each machine's `herdr-app` document and its version.
    pub docs: BTreeMap<Option<String>, (Value, u64)>,
    pub terminals: BTreeMap<String, Terminal>,
    /// What each terminal's program has drawn: its first paint on attach.
    pub screens: BTreeMap<String, Vec<u8>>,
    /// Typed input comes back as output, as a shell's line discipline echoes it.
    pub echo: bool,
    /// Document rounds fail, as on a machine that can't be reached.
    pub docs_down: bool,
    /// Every attach, in order.
    pub attaches: Vec<TerminalAttach>,
    /// Every frame sent on a stream, in order.
    pub sent: Vec<(String, Out)>,
    pub detached: Vec<String>,
    pub calls: Vec<Request>,
    /// Every [`Link::put`]: the terminal and the files. Each lands at `/there/<file name>`.
    pub puts: Vec<(String, Vec<Upload>)>,
    /// Documents written (`doc.set` that took).
    pub writes: usize,
    /// What `machine.exec` answers; `None`: it exits 0 with no output.
    pub exec: Option<Result<ExecResult, RpcError>>,
    connected: bool,
    news: VecDeque<News>,
    writers: BTreeMap<String, terminal::Writer>,
    next: u32,
}

/// A handle on a [`FakeCore`]: the app's link, and the test's view of it.
#[derive(Clone, Default)]
pub struct Fake(Rc<RefCell<FakeCore>>);

fn not_found() -> RpcError {
    RpcError {
        code: -32000,
        message: "no such terminal".into(),
        kind: ErrorKind::NotFound,
        hint: None,
        data: Default::default(),
    }
}

impl Fake {
    /// A core with this machine's `herdr-app` document and these terminals, which connects as
    /// soon as the app starts it.
    pub fn new(doc: Value, terminals: Vec<Terminal>) -> Fake {
        let fake = Fake::default();
        {
            let mut core = fake.core();
            core.docs.insert(None, (doc, 1));
            core.terminals = terminals
                .into_iter()
                .map(|t| (t.address.clone(), t))
                .collect();
        }
        fake
    }

    pub fn core(&self) -> RefMut<'_, FakeCore> {
        self.0.borrow_mut()
    }

    /// The frames sent to one terminal, in order.
    pub fn sent_to(&self, address: &str) -> Vec<Out> {
        self.0
            .borrow()
            .sent
            .iter()
            .filter(|(a, _)| a == address)
            .map(|(_, out)| out.clone())
            .collect()
    }

    /// A terminal's program writes `bytes`.
    pub fn output(&self, address: &str, bytes: &[u8]) {
        let core = self.0.borrow();
        if let Some(writer) = core.writers.get(address) {
            writer.write(bytes.to_vec());
        }
    }

    /// A terminal changes (its status, labels, title): the core posts the new record.
    pub fn update(&self, t: Terminal) {
        let mut core = self.core();
        core.terminals.insert(t.address.clone(), t.clone());
        core.changed(t);
    }

    /// Another viewer resized a terminal: the core says so with a `size` event.
    pub fn resize_elsewhere(&self, address: &str, cols: u16, rows: u16) {
        self.core().set_size(address, cols, rows);
    }

    /// The connection drops; the next poll reconnects with a fresh snapshot.
    pub fn drop_connection(&self) {
        let mut core = self.core();
        core.connected = false;
        let ended: Vec<String> = core.writers.keys().cloned().collect();
        core.writers.clear();
        core.news.push_back(News::Lost("dropped".into()));
        for address in ended {
            core.news.push_back(News::Ended {
                address,
                reason: None,
            });
        }
        core.connect();
    }
}

impl FakeCore {
    fn connect(&mut self) {
        self.connected = true;
        self.news.push_back(News::Connected);
        self.news
            .push_back(News::Event(Event::Terminals(TerminalUpdate {
                resynced: true,
                terminals: self.terminals.values().cloned().collect(),
                ..Default::default()
            })));
    }

    fn changed(&mut self, t: Terminal) {
        self.news
            .push_back(News::Event(Event::Terminals(TerminalUpdate {
                terminals: vec![t],
                ..Default::default()
            })));
    }

    fn set_size(&mut self, address: &str, cols: u16, rows: u16) {
        let Some(t) = self.terminals.get_mut(address) else {
            return;
        };
        if (t.cols, t.rows) == (cols, rows) {
            return;
        }
        t.cols = cols;
        t.rows = rows;
        let t = t.clone();
        self.changed(t);
    }

    fn answer(&mut self, request: &Request) -> Result<Box<Terminal>, RpcError> {
        match request {
            Request::Run(run) => {
                self.next += 1;
                let address = match &run.on {
                    Some(m) => format!("{m}/t{}", self.next),
                    None => format!("t{}", self.next),
                };
                let t = Terminal {
                    address: address.clone(),
                    labels: run.labels.clone(),
                    cwd: run.cwd.clone().unwrap_or_default(),
                    keep: run.keep,
                    cols: 80,
                    rows: 24,
                    ..Default::default()
                };
                self.terminals.insert(address, t.clone());
                self.changed(t.clone());
                Ok(Box::new(t))
            }
            Request::Get(get) => self
                .terminals
                .get(&get.terminal)
                .cloned()
                .map(Box::new)
                .ok_or_else(not_found),
            Request::Kill(kill) => {
                let t = self
                    .terminals
                    .get_mut(&kill.terminal)
                    .ok_or_else(not_found)?;
                t.state = TerminalState::Finished;
                t.reason = Some(Reason::Killed);
                let t = t.clone();
                self.changed(t.clone());
                Ok(Box::new(t))
            }
            Request::Keep(keep) => {
                let t = self
                    .terminals
                    .get_mut(&keep.terminal)
                    .ok_or_else(not_found)?;
                t.keep = keep.keep;
                let t = t.clone();
                self.changed(t.clone());
                Ok(Box::new(t))
            }
            Request::Exec(_) => unreachable!("answered with Event::Executed"),
            Request::Label(label) => {
                let t = self
                    .terminals
                    .get_mut(&label.terminal)
                    .ok_or_else(not_found)?;
                for (k, v) in &label.labels {
                    if v.is_empty() {
                        t.labels.remove(k);
                    } else {
                        t.labels.insert(k.clone(), v.clone());
                    }
                }
                let t = t.clone();
                self.changed(t.clone());
                Ok(Box::new(t))
            }
        }
    }
}

impl Link for Fake {
    fn start(&mut self, _wake: rune::Waker) {
        let mut core = self.core();
        if !core.connected {
            core.connect();
        }
    }

    fn poll(&mut self, news: &mut Vec<News>) {
        news.extend(self.core().news.drain(..));
    }

    fn call(&mut self, token: Token, request: Request) {
        let mut core = self.core();
        core.calls.push(request.clone());
        if let Request::Exec(_) = request {
            let result = core.exec.clone().unwrap_or_else(|| {
                Ok(ExecResult {
                    code: Some(0),
                    ..Default::default()
                })
            });
            core.news
                .push_back(News::Event(Event::Executed { token, result }));
            return;
        }
        let result = core.answer(&request);
        core.news
            .push_back(News::Event(Event::Reply { token, result }));
    }

    fn doc(&mut self, machine: Option<String>, round: Round) {
        let mut core = self.core();
        if core.docs_down {
            let result = Err(super::unavailable("the machine can't be reached"));
            let event = match round {
                Round::Read(_) => Event::DocRead { machine, result },
                Round::Write(_) => Event::DocWritten {
                    machine,
                    result: Err(super::unavailable("the machine can't be reached")),
                },
            };
            core.news.push_back(News::Event(event));
            return;
        }
        let event = match round {
            Round::Read(_) => {
                let (value, version) = core.docs.get(&machine).cloned().unwrap_or((Value::Null, 0));
                Event::DocRead {
                    machine,
                    result: Ok(DocGetResult {
                        value,
                        version,
                        ..Default::default()
                    }),
                }
            }
            Round::Write(set) => {
                let entry = core.docs.entry(machine.clone()).or_insert((Value::Null, 0));
                let result = if set.version == entry.1 {
                    entry.0 = set.value;
                    entry.1 += 1;
                    let version = entry.1;
                    core.writes += 1;
                    Ok(DocSetResult {
                        version,
                        ..Default::default()
                    })
                } else {
                    Err(RpcError {
                        code: -32000,
                        message: "conflict".into(),
                        kind: ErrorKind::Conflict,
                        hint: None,
                        data: Default::default(),
                    })
                };
                Event::DocWritten { machine, result }
            }
        };
        core.news.push_back(News::Event(event));
    }

    fn attach(&mut self, params: TerminalAttach, writer: terminal::Writer, fresh: bool) {
        let mut core = self.core();
        let address = params.terminal.clone();
        core.attaches.push(params);
        let result = match core.terminals.get(&address) {
            Some(t) if t.state == TerminalState::Live => {
                if fresh {
                    writer.reset();
                }
                if let Some(screen) = core.screens.get(&address) {
                    writer.write(screen.clone());
                }
                Ok(Box::new(t.clone()))
            }
            _ => Err("no such terminal".to_owned()),
        };
        if result.is_ok() {
            core.writers.insert(address.clone(), writer);
        }
        core.news.push_back(News::Attached { address, result });
    }

    fn send(&mut self, address: &str, out: Out) {
        let mut core = self.core();
        if !core.writers.contains_key(address) {
            return;
        }
        match &out {
            // One viewer: what it reports is the size.
            Out::Resize { cols, rows } => core.set_size(address, *cols, *rows),
            Out::Input(bytes) if core.echo => {
                let echoed: Vec<u8> = bytes
                    .iter()
                    .flat_map(|&b| {
                        if b == b'\r' {
                            vec![b'\r', b'\n']
                        } else {
                            vec![b]
                        }
                    })
                    .collect();
                if let Some(writer) = core.writers.get(address) {
                    writer.write(echoed);
                }
            }
            _ => {}
        }
        core.sent.push((address.to_owned(), out));
    }

    fn detach(&mut self, address: &str) {
        let mut core = self.core();
        if core.writers.remove(address).is_some() {
            core.detached.push(address.to_owned());
        }
    }

    fn put(&mut self, address: &str, uploads: Vec<Upload>) {
        let mut core = self.core();
        let paths = uploads
            .iter()
            .filter_map(Upload::name)
            .map(|name| format!("/there/{name}"))
            .collect();
        core.puts.push((address.to_owned(), uploads));
        core.news.push_back(News::Put {
            address: address.to_owned(),
            paths,
        });
    }
}
