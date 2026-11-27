// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! The control surface Eli's tools drive (`docs/cli.md`): `herdr-frontend ls | do | status`,
//! compatible with `herdr-app ls | do | status` for the commands those tools use.
//!
//! - [`Model::ls`] and [`Model::status`] read the model; they change nothing.
//! - A `do` command is a [`Call`], stepped like any other input. Its answer comes back as
//!   [`Effect::Answer`], in the same step or, for `space.new`, once `terminal.run` returned the
//!   new terminal.
//!
//! Arguments follow herdr-app's rules (`app:src/commands.rs`): unknown names fail and list the
//! valid ones, target ids must be strings, null means omitted, and a required argument that is
//! missing or blank fails. A script is never asked anything: a rename without a name and a close
//! without `confirmed` fail instead of opening a dialog.

use herdr_shared::{Intent, Layout, Node, fresh_id_on, machine_of};
use serde_json::{Map, Value, json};

use crate::action::Popup;
use crate::effect::{Effect, Effects};
use crate::mode::{Mode, PopupState};
use crate::model::{Launch, Model};
use crate::world::{SpaceExt, tab_name};

/// The layout herdr-frontend shows: Eli's, the only one (`AGENTS.md` scope).
pub const LAYOUT: &str = "eli";

/// Matches a `do` call's answer to the call ([`Effect::Answer`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallId(pub u64);

/// One `do <command> <args>` from a tool. `args` is an object, or null for none.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub id: CallId,
    pub name: String,
    pub args: Value,
}

/// One argument of a [`Command`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arg {
    pub name: &'static str,
    pub required: bool,
}

/// A command `do` serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    pub args: &'static [Arg],
    pub about: &'static str,
}

const fn req(name: &'static str) -> Arg {
    Arg {
        name,
        required: true,
    }
}

const fn opt(name: &'static str) -> Arg {
    Arg {
        name,
        required: false,
    }
}

/// Every command `do` serves, with herdr-app's argument names (`app:src/commands.rs`).
pub const COMMANDS: &[Command] = &[
    Command {
        name: "space.new",
        args: &[
            opt("name"),
            opt("folder"),
            opt("tab"),
            opt("argv"),
            opt("task"),
            opt("machine"),
            opt("focus"),
        ],
        about: "Make a space: its name (default: none, so it's called after where its first \
                terminal is), the folder its terminal starts in (default: the focused terminal's \
                on that machine, else home), its first tab's name (default: none, so it's called \
                by its number), and the machine it lives on (`hc machine ls`; default and empty \
                string: this computer). It is shown at once; focus false keeps the shown space \
                and tab (with no space shown, the new space is shown). argv and task are not \
                served yet.",
    },
    Command {
        name: "space.close",
        args: &[opt("id"), opt("confirmed")],
        about: "Close a space (default: the shown one) and end its terminals. Needs confirmed \
                true.",
    },
    Command {
        name: "space.rename",
        args: &[opt("id"), opt("name")],
        about: "Rename a space (default: the shown one). Needs name; an empty name: called after \
                where its first terminal is again.",
    },
    Command {
        name: "space.select",
        args: &[opt("index"), opt("id")],
        about: "Show a space (by position, 1 = first, or id).",
    },
    Command {
        name: "tab.rename",
        args: &[opt("id"), opt("name")],
        about: "Rename a tab (default: the shown one). Needs name; an empty name: called by its \
                number again.",
    },
    Command {
        name: "pane.close",
        args: &[opt("pane")],
        about: "Close a pane (default: the focused one) and end its terminal. A tab's last pane \
                closes the tab; a space's last pane fails, as closing the space needs \
                space.close with confirmed.",
    },
    Command {
        name: "agent.focus",
        args: &[req("terminal")],
        about: "Show an agent: focus its pane, showing its tab and space. One without a pane \
                (a sub-agent) is peeked instead.",
    },
    Command {
        name: "terminal.peek",
        args: &[req("terminal")],
        about: "Peek at a running terminal: show it over the window, with keep-as-tab and \
                close, without moving focus.",
    },
    Command {
        name: "layout.switch",
        args: &[opt("name")],
        about: "Show a layout. herdr-frontend has one, \"eli\": naming it, or none, succeeds; \
                other names fail.",
    },
];

/// The command called `name`.
pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// The commands as data, for `herdr-frontend commands` (herdr-app's `commands` shape).
pub fn listing() -> Value {
    Value::Array(
        COMMANDS
            .iter()
            .map(|c| {
                let args: Vec<Value> = c
                    .args
                    .iter()
                    .map(|a| json!({ "name": a.name, "required": a.required }))
                    .collect();
                json!({ "name": c.name, "args": args, "about": c.about })
            })
            .collect(),
    )
}

/// Rejects misspelled arguments and non-string target ids before a command can fall back to
/// the current target (`app:src/commands.rs` `validate_args`).
pub fn validate_args(c: &Command, args: &Value) -> Result<(), String> {
    let valid = if c.args.is_empty() {
        "none".to_owned()
    } else {
        c.args.iter().map(|a| a.name).collect::<Vec<_>>().join(", ")
    };
    match args {
        Value::Null => Ok(()),
        Value::Object(args) => {
            let unknown: Vec<&str> = args
                .keys()
                .filter(|key| !c.args.iter().any(|a| a.name == key.as_str()))
                .map(String::as_str)
                .collect();
            if !unknown.is_empty() {
                return Err(format!(
                    "{}: unknown argument(s): {}; valid arguments: {valid}",
                    c.name,
                    unknown.join(", ")
                ));
            }
            for (key, value) in args {
                if matches!(
                    key.as_str(),
                    "id" | "pane" | "space" | "terminal" | "target"
                ) && !value.is_string()
                    && !value.is_null()
                {
                    return Err(format!(
                        "{}: argument {key} must be a string (or null to omit it)",
                        c.name
                    ));
                }
            }
            Ok(())
        }
        _ => Err(format!(
            "{}: arguments must be an object; valid arguments: {valid}",
            c.name
        )),
    }
}

/// The first required argument that's missing; blank strings count as missing.
fn missing(c: &Command, args: &Value) -> Option<&'static str> {
    c.args
        .iter()
        .filter(|a| a.required)
        .map(|a| a.name)
        .find(|k| match args.get(k) {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.trim().is_empty(),
            _ => false,
        })
}

/// `name:value`, as keys name commands: the value fills the first argument, and a number fills
/// `index` where there is one (`app:src/commands.rs` `shorthand`).
fn shorthand(c: &Command, value: &str) -> Value {
    let index = value
        .parse::<u64>()
        .ok()
        .filter(|_| c.args.iter().any(|a| a.name == "index"));
    match (index, c.args.first()) {
        (Some(i), _) => json!({ "index": i }),
        (None, Some(a)) => json!({ a.name: value }),
        (None, None) => Value::Null,
    }
}

/// Reads a call's command and arguments: the command it names and its arguments, checked.
fn resolve(name: &str, args: Value) -> Result<(&'static Command, Value), String> {
    if let Some((head, value)) = name.split_once(':')
        && let Some(c) = find(head)
    {
        validate_args(c, &args)?;
        let mut short = shorthand(c, value);
        if let (Some(short), Some(extra)) = (short.as_object_mut(), args.as_object()) {
            short.extend(extra.clone());
        }
        return resolve(head, short);
    }
    let c = find(name).ok_or_else(|| {
        format!("unknown command {name:?}: see `herdr-frontend commands` for the ones it serves")
    })?;
    validate_args(c, &args)?;
    if let Some(arg) = missing(c, &args) {
        return Err(format!("{name} needs {arg}"));
    }
    Ok((c, args))
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn owned(args: &Value, key: &str) -> Option<String> {
    arg(args, key).map(str::to_owned)
}

/// Whether a new space is shown: `focus`, true when omitted; `None` when it isn't a boolean.
fn focus(args: &Value) -> Option<bool> {
    match args.get("focus") {
        None | Some(Value::Null) => Some(true),
        Some(v) => v.as_bool(),
    }
}

fn ok() -> Value {
    json!({ "ok": true })
}

impl Model {
    /// `herdr-frontend ls`: spaces, their tabs and terminals, in herdr-app's shape
    /// (`app:src/window.rs` `world`). Sub-agents are listed under the terminal that started them.
    pub fn ls(&self) -> Value {
        let world = &self.world;
        let terminal = |pane: &str, address: &str| {
            let record = world.terminal(address);
            let app = record
                .and_then(|t| t.status.as_ref())
                .map(|s| s.app.clone())
                .filter(|app| !app.is_empty());
            let mut sub_agents: Vec<&str> = world
                .live()
                .filter(|t| t.parent.as_deref() == Some(address))
                .map(|t| t.address.as_str())
                .collect();
            sub_agents.sort();
            json!({
                "terminal": address,
                "pane": pane,
                "status": self.attention.displayed(address).map(|d| d.as_str()),
                "app": app,
                "name": record.and_then(|t| t.name.clone()),
                "sub_agents": sub_agents,
            })
        };
        let active = self.active_space().map(|s| s.id.clone());
        let spaces: Vec<Value> = world
            .spaces()
            .map(|space| {
                let selected = self.view.selected_tab(world, space).map(|t| t.id.clone());
                let tabs: Vec<Value> = space
                    .tabs
                    .iter()
                    .map(|tab| {
                        let terminals: Vec<Value> = tab
                            .root
                            .terminals()
                            .iter()
                            .filter_map(|n| Some(terminal(&n.id, n.terminal()?)))
                            .collect();
                        json!({
                            "id": tab.id,
                            "name": tab_name(space, tab),
                            "active": selected.as_deref() == Some(tab.id.as_str()),
                            "settled": self.view.settled(&tab.id),
                            "terminals": terminals,
                        })
                    })
                    .collect();
                json!({
                    "id": space.id,
                    "name": world.space_name(space, &self.home),
                    "folder": space.folder,
                    "machine": machine_of(&space.id),
                    "active": active.as_deref() == Some(space.id.as_str()),
                    "tabs": tabs,
                })
            })
            .collect();
        let recent: Vec<Value> = self
            .view
            .recent
            .iter()
            .map(|r| json!({ "name": r.name, "folder": r.folder }))
            .collect();
        json!({ "spaces": spaces, "recent": recent })
    }

    /// `herdr-frontend status`: what this screen shows, in herdr-app's shape
    /// (`app:src/window.rs` `control`), less what only the window knows (`zoom`, `font_size`,
    /// `ui_font_size` and file errors), which the app adds.
    pub fn status(&self) -> Map<String, Value> {
        let mut overlays: Vec<&str> = match &self.mode {
            Mode::Popup(PopupState::List(list)) => vec![list.popup.as_str()],
            Mode::Popup(PopupState::Jump(_)) => vec![Popup::AgentJump.as_str()],
            Mode::Popup(PopupState::Prompt(_)) => vec!["prompt"],
            Mode::Popup(PopupState::Confirm(_)) => vec!["confirm"],
            Mode::Popup(PopupState::Menu(_)) => vec!["menu"],
            Mode::Copy(_) => vec!["copy-mode"],
            Mode::Normal | Mode::Prefix { .. } => vec![],
        };
        // Herdr app's id for the peek's dialog, under any popup opened over it.
        if self.peek.is_some() {
            overlays.insert(0, "terminal-peek");
        }
        let errors: Vec<String> = self
            .world
            .machines()
            .filter_map(|(key, m)| {
                let why = match m.mirror.layout() {
                    Layout::Unavailable(why) | Layout::Blocked(why) | Layout::Undecodable(why) => {
                        why
                    }
                    Layout::Loading | Layout::Ready => return None,
                };
                Some(format!(
                    "layout {}: {why}",
                    key.as_deref().unwrap_or("local")
                ))
            })
            .collect();
        let mut status = Map::new();
        status.insert(
            "space".into(),
            json!(self.active_space().map(|s| s.id.clone())),
        );
        status.insert("tab".into(), json!(self.shown_tab().map(|t| t.id.clone())));
        status.insert("focused_pane".into(), json!(self.focused_pane()));
        status.insert("layout".into(), json!(LAYOUT));
        status.insert("agent".into(), json!(self.shown_agent()));
        status.insert("floats".into(), json!({}));
        status.insert("overlays".into(), json!(overlays));
        status.insert("errors".into(), json!(errors));
        status
    }

    /// The agent on screen: the focused terminal when it is one, else the first in the agent
    /// order (`app:src/window.rs` `shown_agent`).
    fn shown_agent(&self) -> Option<String> {
        self.focused_terminal()
            .filter(|a| self.attention.displayed(a).is_some())
            .map(str::to_owned)
            .or_else(|| self.agent_order().into_iter().next())
    }

    /// Runs a `do` call. Every call is answered exactly once, now or after its launch.
    pub(crate) fn control(&mut self, call: Call, fx: &mut Effects) {
        let id = call.id;
        let answer = match resolve(&call.name, call.args) {
            Ok((command, args)) => self.command(id, command.name, &args, fx),
            Err(error) => Some(Err(error)),
        };
        if let Some(answer) = answer {
            answer_call(fx, id, answer);
        }
    }

    /// One checked command. `None`: the answer comes later (a launch under way).
    fn command(
        &mut self,
        id: CallId,
        name: &str,
        a: &Value,
        fx: &mut Effects,
    ) -> Option<Result<Value, String>> {
        Some(match name {
            "space.new" => return self.control_new_space(id, a, fx),
            "space.close" => self.control_close_space(a, fx),
            "space.rename" => (|| {
                let space = owned(a, "id")
                    .or_else(|| self.active_space().map(|s| s.id.clone()))
                    .ok_or("no space to rename")?;
                let name = arg(a, "name").ok_or("space.rename needs name (empty: automatic)")?;
                if self.world.space(&space).is_none() {
                    return Err(format!("no space {space:?}"));
                }
                self.layout_ready_for(machine_of(&space))?;
                let name = name.trim().to_owned();
                self.edit(Intent::RenameSpace { space, name }, fx);
                Ok(ok())
            })(),
            "space.select" => {
                let id = owned(a, "id").or_else(|| {
                    let i = a.get("index").and_then(Value::as_u64)? as usize;
                    let space = self.world.spaces().nth(i.checked_sub(1)?)?;
                    Some(space.id.clone())
                });
                match id.filter(|id| self.world.space(id).is_some()) {
                    Some(id) => {
                        self.view.active = Some(id);
                        Ok(ok())
                    }
                    None => Err("no such space".into()),
                }
            }
            "tab.rename" => (|| {
                let tab = owned(a, "id")
                    .or_else(|| self.shown_tab().map(|t| t.id.clone()))
                    .ok_or("no tab to rename")?;
                let name = arg(a, "name").ok_or("tab.rename needs name (empty: its number)")?;
                if self.world.tab(&tab).is_none() {
                    return Err(format!("no tab {tab:?}"));
                }
                self.layout_ready_for(machine_of(&tab))?;
                let name = name.trim().to_owned();
                self.edit(Intent::RenameTab { tab, name }, fx);
                Ok(ok())
            })(),
            "pane.close" => self.control_close_pane(a, fx),
            "agent.focus" => {
                let terminal = arg(a, "terminal").unwrap_or_default().to_owned();
                if self.world.is_live(&terminal) {
                    self.focus_agent(&terminal, fx);
                    Ok(ok())
                } else {
                    Err(format!("no running terminal {terminal:?}"))
                }
            }
            "terminal.peek" => {
                let terminal = arg(a, "terminal").unwrap_or_default().to_owned();
                if self.world.is_live(&terminal) {
                    self.open_peek(&terminal, fx);
                    Ok(ok())
                } else {
                    Err(format!("no running terminal {terminal:?}"))
                }
            }
            "layout.switch" => match arg(a, "name") {
                None | Some(LAYOUT) => Ok(ok()),
                Some(other) => Err(format!(
                    "no layout {other:?}: herdr-frontend has only the {LAYOUT:?} layout"
                )),
            },
            _ => unreachable!("every command in COMMANDS has a handler: {name}"),
        })
    }

    /// `space.new`: starts its terminal, and answers `{space, tab, terminal}` once the space is
    /// in the layout (`app:src/window.rs` `new_space`).
    fn control_new_space(
        &mut self,
        id: CallId,
        a: &Value,
        fx: &mut Effects,
    ) -> Option<Result<Value, String>> {
        let Some(focus) = focus(a) else {
            return Some(Err("space.new focus is true or false".into()));
        };
        let has = |key| a.get(key).is_some_and(|v| !v.is_null());
        if has("argv") || has("task") {
            return Some(Err(
                "space.new: argv and task are not served by herdr-frontend yet".into(),
            ));
        }
        // Scripts get this computer unless they name a machine.
        let machine = owned(a, "machine").filter(|m| !m.is_empty());
        if let Err(error) = self.layout_ready_for(machine.as_deref()) {
            return Some(Err(error));
        }
        let m = machine.as_deref();
        // The focused terminal's folder only means something on its own machine.
        let here = self
            .focused_terminal()
            .filter(|t| machine_of(t) == m)
            .and_then(|t| self.world.folder_of(t))
            .map(str::to_owned);
        let folder = owned(a, "folder").or(here).unwrap_or_else(|| {
            if m.is_some() {
                String::new()
            } else {
                self.home.clone()
            }
        });
        let launch = Launch::Space {
            space: fresh_id_on("sp", m),
            name: arg(a, "name").unwrap_or_default().trim().to_owned(),
            folder: folder.clone(),
            tab: fresh_id_on("tb", m),
            tab_name: arg(a, "tab").unwrap_or_default().trim().to_owned(),
            pane: fresh_id_on("p", m),
            focus,
        };
        self.launch(machine, &folder, launch, Vec::new(), Some(id), fx);
        None
    }

    /// `space.close`: needs `confirmed: true`, since it ends the space's terminals and a script
    /// can't be asked.
    fn control_close_space(&mut self, a: &Value, fx: &mut Effects) -> Result<Value, String> {
        let space = owned(a, "id")
            .or_else(|| self.active_space().map(|s| s.id.clone()))
            .ok_or("no space to close")?;
        if self.world.space(&space).is_none() {
            return Err(format!("no space {space:?}"));
        }
        if a.get("confirmed").and_then(Value::as_bool) != Some(true) {
            return Err("space.close ends its terminals: pass confirmed: true".into());
        }
        self.layout_ready_for(machine_of(&space))?;
        self.close_space(&space, fx);
        Ok(ok())
    }

    /// `pane.close`: a tab's last pane closes the tab; a space's last pane would close the
    /// space, which needs `space.close` with `confirmed` (`app:src/window.rs` `pane.close`,
    /// `tab.close`).
    fn control_close_pane(&mut self, a: &Value, fx: &mut Effects) -> Result<Value, String> {
        let pane = owned(a, "pane")
            .or_else(|| self.focused_pane())
            .ok_or("no pane to close")?;
        let tab = self
            .world
            .tab_of(&pane)
            .filter(|t| t.root.find(&pane).is_some_and(Node::is_pane))
            .ok_or_else(|| format!("no pane {pane:?}"))?;
        self.layout_ready_for(machine_of(&pane))?;
        if tab.root.panes().len() > 1 {
            let address = self.world.address_of(&pane).map(str::to_owned);
            if self.edit(Intent::RemovePane { pane }, fx) {
                self.kill(address.into_iter().collect(), fx);
            }
            return Ok(ok());
        }
        let tab = tab.id.clone();
        let space = self.world.space_of(&tab).filter(|s| s.tab(&tab).is_some());
        if space.is_none_or(|s| s.tabs.len() == 1) {
            return Err("space.close ends its terminals: pass confirmed: true".into());
        }
        let addresses = self
            .world
            .tab(&tab)
            .map(|t| t.root.addresses())
            .unwrap_or_default();
        if self.edit(Intent::CloseTab { tab }, fx) {
            self.kill(addresses, fx);
        }
        Ok(ok())
    }

    /// Whether a machine's layout can take an edit now (`app:src/window.rs`
    /// `require_document`); when not, why. Edits and launches need the core connection too.
    fn layout_ready_for(&self, machine: Option<&str>) -> Result<(), String> {
        if !self.connected {
            return Err("herdr-frontend is not connected to herdr-core".into());
        }
        let unavailable = "This machine's layout is unavailable. Try again when it reconnects.";
        let Some(m) = self.world.machine(machine) else {
            let name = machine.unwrap_or_default();
            return Err(format!(
                "herdr-frontend has no machine called {name}. See hc machine ls; machines added \
                 with --temp stay out of it."
            ));
        };
        match m.mirror.layout() {
            Layout::Ready if m.reachable() => Ok(()),
            Layout::Unavailable(why) | Layout::Blocked(why) | Layout::Undecodable(why) => {
                Err(why.clone())
            }
            Layout::Ready | Layout::Loading => Err(unavailable.into()),
        }
    }

    /// The new space's first pane is in the layout: answers its call, if a call started it.
    pub(crate) fn launched_space(
        &mut self,
        space: &str,
        tab: &str,
        address: &str,
        placed: bool,
        answer: Option<CallId>,
        fx: &mut Effects,
    ) {
        let Some(id) = answer else { return };
        let reply = if placed {
            Ok(json!({ "space": space, "tab": tab, "terminal": address }))
        } else {
            Err(format!(
                "terminal {address} started, but the layout could not take its space"
            ))
        };
        answer_call(fx, id, reply);
    }
}

/// Pushes a call's answer: its result, or `{error}`.
pub(crate) fn answer_call(fx: &mut Effects, id: CallId, result: Result<Value, String>) {
    let reply = result.unwrap_or_else(|error| json!({ "error": error }));
    fx.push(Effect::Answer { id, reply });
}
