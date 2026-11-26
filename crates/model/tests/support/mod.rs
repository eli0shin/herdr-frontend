//! A fake core for driving [`Model::step`]: it answers document rounds under compare-and-set and
//! terminal calls, and posts the events a real core would, so tests read back what was written.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};

use herdr_proto::methods::{DocGetResult, DocSetResult};
use herdr_proto::{ErrorKind, RpcError, Status, StatusState, Terminal, TerminalState};
use herdr_shared::{Document, Round};
use model::{Chord, Effect, Event, Input, Model, Request, TerminalUpdate};
use serde_json::{Value, json};

pub const HOME: &str = "/home/me";

pub struct Core {
    pub docs: BTreeMap<Option<String>, (Value, u64)>,
    pub terminals: BTreeMap<String, Terminal>,
    pub calls: Vec<Request>,
    next: u32,
    /// Fail every document call for these machines.
    pub down: Vec<Option<String>>,
    /// The app clock, in seconds, passed with every step.
    pub now: f64,
}

pub fn error(kind: ErrorKind, message: &str) -> RpcError {
    RpcError {
        code: -32000,
        message: message.into(),
        kind,
        hint: None,
        data: Default::default(),
    }
}

impl Core {
    pub fn new(doc: Value, terminals: Vec<Terminal>) -> Core {
        let mut docs = BTreeMap::new();
        docs.insert(None, (doc, 1));
        Core {
            docs,
            terminals: terminals
                .into_iter()
                .map(|t| (t.address.clone(), t))
                .collect(),
            calls: Vec::new(),
            next: 100,
            down: Vec::new(),
            now: 0.0,
        }
    }

    pub fn doc(&self, machine: Option<&str>) -> Document {
        let value = self.docs[&machine.map(str::to_owned)].0.clone();
        Document::from_value(value).expect("a document herdr-app can read")
    }

    pub fn version(&self, machine: Option<&str>) -> u64 {
        self.docs[&machine.map(str::to_owned)].1
    }

    /// Another client writes the document.
    pub fn write_elsewhere(&mut self, machine: Option<&str>, value: Value) -> Event {
        let entry = self.docs.get_mut(&machine.map(str::to_owned)).unwrap();
        entry.0 = value;
        entry.1 += 1;
        Event::Terminals(TerminalUpdate {
            docs: vec![(machine.map(str::to_owned), entry.1)],
            ..Default::default()
        })
    }

    pub fn snapshot(&self) -> Event {
        Event::Terminals(TerminalUpdate {
            resynced: true,
            terminals: self.terminals.values().cloned().collect(),
            ..Default::default()
        })
    }

    pub fn kills(&self) -> Vec<String> {
        self.calls
            .iter()
            .filter_map(|c| match c {
                Request::Kill(k) => Some(k.terminal.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn runs(&self) -> Vec<herdr_proto::methods::TerminalRun> {
        self.calls
            .iter()
            .filter_map(|c| match c {
                Request::Run(r) => Some((**r).clone()),
                _ => None,
            })
            .collect()
    }

    fn update(t: Terminal) -> Input {
        Event::Terminals(TerminalUpdate {
            terminals: vec![t],
            ..Default::default()
        })
        .into()
    }

    /// The reply to a call, and the event the core posts with it (sent first).
    fn answer(&mut self, request: &Request) -> (Result<Box<Terminal>, RpcError>, Option<Input>) {
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
                    ..Default::default()
                };
                self.terminals.insert(address, t.clone());
                (Ok(Box::new(t.clone())), Some(Core::update(t)))
            }
            Request::Get(get) => match self.terminals.get(&get.terminal) {
                Some(t) => (Ok(Box::new(t.clone())), None),
                None => (Err(error(ErrorKind::NotFound, "no such terminal")), None),
            },
            Request::Kill(kill) => match self.terminals.get_mut(&kill.terminal) {
                Some(t) => {
                    t.state = TerminalState::Finished;
                    t.reason = Some(herdr_proto::Reason::Killed);
                    let t = t.clone();
                    (Ok(Box::new(t.clone())), Some(Core::update(t)))
                }
                None => (Err(error(ErrorKind::NotFound, "no such terminal")), None),
            },
            Request::Keep(keep) => match self.terminals.get_mut(&keep.terminal) {
                Some(t) => {
                    t.keep = keep.keep;
                    let t = t.clone();
                    (Ok(Box::new(t.clone())), Some(Core::update(t)))
                }
                None => (Err(error(ErrorKind::NotFound, "no such terminal")), None),
            },
            Request::Label(label) => match self.terminals.get_mut(&label.terminal) {
                Some(t) => {
                    for (k, v) in &label.labels {
                        if v.is_empty() {
                            t.labels.remove(k);
                        } else {
                            t.labels.insert(k.clone(), v.clone());
                        }
                    }
                    let t = t.clone();
                    (Ok(Box::new(t.clone())), Some(Core::update(t)))
                }
                None => (Err(error(ErrorKind::NotFound, "no such terminal")), None),
            },
        }
    }
}

/// Steps `input`, then answers every round and call it asks for until none is left. Returns
/// every other effect, in order.
pub fn drive(m: &mut Model, core: &mut Core, input: impl Into<Input>) -> Vec<Effect> {
    let mut queue: VecDeque<Input> = VecDeque::from([input.into()]);
    let mut out = Vec::new();
    while let Some(input) = queue.pop_front() {
        for effect in m.step(core.now, input) {
            match effect {
                Effect::Doc { machine, round } => {
                    let down = core.down.contains(&machine);
                    match round {
                        Round::Read(_) => {
                            let result = if down {
                                Err(error(
                                    ErrorKind::Other("unreachable".into()),
                                    "machine down",
                                ))
                            } else {
                                let (value, version) =
                                    core.docs.get(&machine).cloned().unwrap_or((Value::Null, 0));
                                Ok(DocGetResult {
                                    value,
                                    version,
                                    ..Default::default()
                                })
                            };
                            queue.push_back(Event::DocRead { machine, result }.into());
                        }
                        Round::Write(set) => {
                            let entry =
                                core.docs.entry(machine.clone()).or_insert((Value::Null, 0));
                            let result = if set.version != entry.1 {
                                Err(error(ErrorKind::Conflict, "conflict"))
                            } else {
                                entry.0 = set.value;
                                entry.1 += 1;
                                Ok(DocSetResult {
                                    version: entry.1,
                                    ..Default::default()
                                })
                            };
                            queue.push_back(Event::DocWritten { machine, result }.into());
                        }
                    }
                }
                Effect::Call { token, request } => {
                    core.calls.push(request.clone());
                    let (result, event) = core.answer(&request);
                    if let Some(event) = event {
                        queue.push_back(event);
                    }
                    queue.push_back(Event::Reply { token, result }.into());
                }
                other => out.push(other),
            }
        }
    }
    out
}

/// A model connected to `core`, its layout read.
pub fn connected(core: &mut Core) -> Model {
    let mut m = Model::new(HOME);
    drive(&mut m, core, Event::Connected);
    let snapshot = core.snapshot();
    drive(&mut m, core, snapshot);
    m
}

pub fn key(m: &mut Model, core: &mut Core, keys: &str, terminal: bool) -> Vec<Effect> {
    let mut out = Vec::new();
    for chord in Chord::parse_sequence(keys).unwrap() {
        out.extend(drive(m, core, Event::Key { chord, terminal }));
    }
    out
}

pub fn shell(address: &str, cwd: &str) -> Terminal {
    Terminal {
        address: address.into(),
        cwd: cwd.into(),
        keep: true,
        ..Default::default()
    }
}

pub fn agent(address: &str, state: StatusState) -> Terminal {
    Terminal {
        address: address.into(),
        cwd: "/home/me/code/x".into(),
        keep: true,
        status: Some(Status {
            state,
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn pane(id: &str, terminal: &str) -> Value {
    json!({"id": id, "type": "block", "content": {"terminal": terminal}})
}

/// A tab whose root row holds `children`.
pub fn tab(id: &str, children: Vec<Value>) -> Value {
    json!({"id": id, "name": "", "root": {"id": id, "type": "row", "children": children}})
}

pub fn space(id: &str, tabs: Vec<Value>) -> Value {
    json!({"id": id, "name": "", "folder": "/home/me/code/x", "tabs": tabs})
}

pub fn doc(spaces: Vec<Value>) -> Value {
    json!({ "spaces": spaces })
}

/// The ids of a space's tabs in the document.
pub fn tab_ids(doc: &Document, space: &str) -> Vec<String> {
    doc.space(space)
        .map(|s| s.tabs.iter().map(|t| t.id.clone()).collect())
        .unwrap_or_default()
}
