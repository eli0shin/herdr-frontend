//! Machines (`feature-inventory.md` §9), as pure rules: the machine filter's rows and what its
//! keys do to the hidden list (Eli's `agent-machines` mod, `filter.js`), how a machine's line
//! reads in the Machines dialog and what to run when it needs you (Herdr app's Settings →
//! Machines), the machine menu, the ssh config's hosts and what a failed `machine.add` tells the
//! person (the shipped `machine-add` mod). The window draws them and carries out the calls.
//!
//! A machine is keyed by its name in the filter, `""` for this computer, as the mod stores
//! `dotfiles.agents.hidden-machines`: the hidden keys, so a machine connected later shows until
//! it is hidden.

use std::collections::{BTreeMap, BTreeSet};

use herdr_proto::types::{MachineState, MachineView};
use herdr_proto::{ErrorKind, RpcError};

use crate::action::{Action, Popup};
use crate::filter;
use crate::mode::{Menu, MenuIcon, MenuItem, MenuTarget};
use crate::world::World;

/// What this computer is called wherever machines are listed.
pub const THIS_COMPUTER: &str = "This computer";

// ---- The machine filter (`ctrl-g /`, `cfg/mods/agent-machines`) --------------------------------

/// One machine filter row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterRow {
    /// `None` for "All machines"; `Some("")` for this computer.
    pub key: Option<String>,
    pub label: String,
    /// Agents on it, hidden or not.
    pub agents: usize,
    /// Its checkbox: its agents show ("All machines": nothing is hidden).
    pub shown: bool,
    /// Known only from an agent or the hidden list, or its line isn't up.
    pub offline: bool,
}

/// The filter's rows: "All machines", this computer, the machines `machine.list` has (in
/// machine order: by name, as their spaces are), then any machine known only from an agent or
/// the hidden list (`filter.js` `machineRows`). `agents` counts every agent by machine key.
pub fn filter_rows(
    world: &World,
    agents: &BTreeMap<String, usize>,
    hidden: &BTreeSet<String>,
) -> Vec<FilterRow> {
    let known: Vec<(String, bool)> = world
        .machines()
        .filter_map(|(key, m)| Some((key.clone()?, m.reachable())))
        .collect();
    let mut keys: Vec<(String, bool)> = vec![(String::new(), false)];
    keys.extend(known.iter().map(|(name, up)| (name.clone(), !up)));
    for key in agents.keys().chain(hidden) {
        if !keys.iter().any(|(k, _)| k == key) {
            keys.push((key.clone(), true));
        }
    }
    let mut rows = vec![FilterRow {
        key: None,
        label: "All machines".into(),
        agents: agents.values().sum(),
        shown: hidden.is_empty(),
        offline: false,
    }];
    rows.extend(keys.into_iter().map(|(key, offline)| FilterRow {
        label: if key.is_empty() {
            THIS_COMPUTER.into()
        } else {
            key.clone()
        },
        agents: agents.get(&key).copied().unwrap_or(0),
        shown: !hidden.contains(&key),
        offline,
        key: Some(key),
    }));
    rows
}

/// The hidden list after a pick on row `i`: "All machines" shows every machine; Space (`only`
/// false) shows or hides the row's machine; Enter (`only`) hides every listed machine but it
/// (`main.js` `apply`, `filter.js` `toggleMachine`, `onlyMachine`).
pub fn hidden_after(
    rows: &[FilterRow],
    hidden: &BTreeSet<String>,
    i: usize,
    only: bool,
) -> BTreeSet<String> {
    let Some(Some(key)) = rows.get(i).map(|r| r.key.as_ref()) else {
        return BTreeSet::new();
    };
    if only {
        return rows
            .iter()
            .filter_map(|r| r.key.clone())
            .filter(|k| k != key)
            .collect();
    }
    let mut next = hidden.clone();
    if !next.remove(key) {
        next.insert(key.clone());
    }
    next
}

// ---- Machine status (Herdr app's Settings → Machines) ------------------------------------------

/// A word for a shell, quoted only when it must be (`machine-add/main.js` `shellArg`).
pub fn shell_arg(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// A machine's line state as people read it: `needs_login` is "needs login".
pub fn state_word(state: &MachineState) -> String {
    state.as_str().replace(['_', '-'], " ")
}

/// How a machine's status dot reads: up, on its way, or not working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Up,
    Connecting,
    Down,
}

pub fn line(state: &MachineState) -> Line {
    match state {
        MachineState::Connected => Line::Up,
        MachineState::Connecting => Line::Connecting,
        _ => Line::Down,
    }
}

/// The line under a machine's name: how it is reached, its state and its hc version
/// (`settings/main.js` `machineRow`).
pub fn detail(view: &MachineView) -> String {
    let version = if view.machine.version.is_empty() {
        "hc version unknown".to_owned()
    } else {
        format!("hc {}", view.machine.version)
    };
    format!(
        "{} · {} · {version}",
        view.machine.target,
        state_word(&view.state)
    )
}

/// Something only the person can do for a machine, and the command that does it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Need {
    pub text: String,
    pub command: String,
}

/// What a machine needs from the person: a login, a core update, or (on Linux, where the
/// server ends programs at logout) lingering turned on. The core's `why` says why one is
/// unreachable.
pub fn needs(view: &MachineView) -> Vec<Need> {
    let name = shell_arg(&view.machine.name);
    let mut out = Vec::new();
    match view.state {
        MachineState::NeedsLogin => out.push(Need {
            text: "needs a login: run".into(),
            command: format!("hc machine login {name}"),
        }),
        MachineState::NeedsUpdate => out.push(Need {
            text: "needs a core update: run".into(),
            command: format!("hc machine update {name}"),
        }),
        _ => {}
    }
    if view.machine.keeps_running_after_logout == Some(false) {
        let user = view
            .machine
            .logout_linger_user
            .as_deref()
            .unwrap_or("<user>");
        out.push(Need {
            text: format!(
                "agents on {} stop when you disconnect: run this there once",
                view.machine.name
            ),
            command: format!("sudo loginctl enable-linger -- {}", shell_arg(user)),
        });
    }
    out
}

/// Why a machine that isn't connected isn't, when the core says.
pub fn why(view: &MachineView) -> Option<&str> {
    (view.state != MachineState::Connected)
        .then_some(view.why.as_deref())
        .flatten()
        .filter(|w| !w.is_empty())
}

/// A machine action the Machines dialog's menu runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineAction {
    /// Its menu ([`machine_menu`]).
    Menu(String),
    /// Dial it now (`machine.connect`).
    Connect(String),
    /// Ask for its new name (the prompt).
    Rename(String),
    /// Ask before forgetting it (the confirm dialog).
    Remove(String),
    /// Add the machine ssh reaches this way (`machine.add`).
    Add(String),
}

/// A machine call for the window to make; its answer comes back to the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineCall {
    Add(String),
    Connect(String),
    Rename { machine: String, to: String },
    Remove(String),
}

impl MachineCall {
    /// The machine it is about (for an add, its target).
    pub fn machine(&self) -> &str {
        match self {
            MachineCall::Add(m) | MachineCall::Connect(m) | MachineCall::Remove(m) => m,
            MachineCall::Rename { machine, .. } => machine,
        }
    }

    /// What it says while it runs.
    pub fn busy(&self) -> &'static str {
        match self {
            MachineCall::Add(_) | MachineCall::Connect(_) => "Connecting…",
            MachineCall::Rename { .. } => "Renaming…",
            MachineCall::Remove(_) => "Removing…",
        }
    }

    /// What it says once done (`settings/main.js` `machineAction`).
    pub fn done(&self) -> String {
        match self {
            MachineCall::Add(target) => format!("Added {target}"),
            MachineCall::Connect(m) => format!("Connecting to {m}"),
            MachineCall::Rename { machine, to } => format!("Renamed {machine} to {to}"),
            MachineCall::Remove(m) => format!("Removed {m}; its terminals keep running there"),
        }
    }
}

/// The menu for one machine in the Machines dialog: Connect while it isn't, a copy of each
/// command it needs run, Rename and Remove. Connecting and copying go back to the dialog.
pub fn machine_menu(view: &MachineView) -> Menu {
    let name = view.machine.name.clone();
    let back = |action: Action| vec![action, Action::Open(Popup::Machines)];
    let mut items = Vec::new();
    if view.state != MachineState::Connected {
        items.push(MenuItem::new(
            "Connect now",
            MenuIcon::Machine,
            back(Action::Machine(MachineAction::Connect(name.clone()))),
        ));
    }
    for need in needs(view) {
        items.push(MenuItem::new(
            format!("Copy {}", need.command),
            MenuIcon::Copy,
            back(Action::CopyText(need.command)),
        ));
    }
    items.push(MenuItem::new(
        "Rename",
        MenuIcon::Rename,
        vec![Action::Machine(MachineAction::Rename(name.clone()))],
    ));
    items.push(MenuItem::new(
        "Remove",
        MenuIcon::Close,
        vec![Action::Machine(MachineAction::Remove(name.clone()))],
    ));
    Menu::new(MenuTarget::Machine(name), items, 300.0)
}

// ---- Adding a machine (the shipped `machine-add` mod) ------------------------------------------

/// A named `Host` of the ssh config (patterns aren't hosts one can pick).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Host {
    pub alias: String,
    /// Its other names on the same `Host` line.
    pub aliases: Vec<String>,
    pub hostname: Option<String>,
    pub user: Option<String>,
}

impl Host {
    /// The line beside its name: `user@hostname`, else its other names.
    pub fn sub(&self) -> String {
        let at: Vec<&str> = [self.user.as_deref(), self.hostname.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if !at.is_empty() {
            at.join("@")
        } else if !self.aliases.is_empty() {
            format!("also {}", self.aliases.join(", "))
        } else {
            String::new()
        }
    }
}

/// How deep `Include`s nest before ssh stops.
const INCLUDE_DEPTH: usize = 16;

/// The hosts an ssh config names, with those of the files it includes (`machine-add/main.js`
/// `parseConfig`). `include` is given an `Include` line's value and returns the texts of the
/// files it names, in order.
pub fn ssh_hosts(text: &str, include: &mut dyn FnMut(&str) -> Vec<String>) -> Vec<Host> {
    let mut hosts = Vec::new();
    parse_ssh(text, include, &mut hosts, 0);
    hosts
}

fn parse_ssh(
    text: &str,
    include: &mut dyn FnMut(&str) -> Vec<String>,
    hosts: &mut Vec<Host>,
    depth: usize,
) {
    let mut current: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let split = line
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(line.len());
        let (key, rest) = line.split_at(split);
        let key = key.to_ascii_lowercase();
        let value = rest
            .trim_start()
            .strip_prefix('=')
            .unwrap_or(rest)
            .trim()
            .to_owned();
        match key.as_str() {
            "host" | "match" => {
                current = None;
                if key == "host" {
                    let names: Vec<&str> = value
                        .split_whitespace()
                        .filter(|h| !h.contains(['*', '?', '!']))
                        .filter(|h| {
                            !hosts
                                .iter()
                                .any(|x| x.alias == *h || x.aliases.iter().any(|a| a == h))
                        })
                        .collect();
                    if let Some((first, rest)) = names.split_first() {
                        hosts.push(Host {
                            alias: (*first).to_owned(),
                            aliases: rest.iter().map(|s| (*s).to_owned()).collect(),
                            ..Host::default()
                        });
                        current = Some(hosts.len() - 1);
                    }
                }
            }
            "include" if depth < INCLUDE_DEPTH => {
                for text in include(&value) {
                    parse_ssh(&text, include, hosts, depth + 1);
                }
            }
            "hostname" => {
                if let Some(i) = current {
                    hosts[i].hostname = Some(value);
                }
            }
            "user" => {
                if let Some(i) = current {
                    hosts[i].user = Some(value);
                }
            }
            _ => {}
        }
    }
}

/// Whether a machine was added under this target (its target, name or host).
pub fn added(world: &World, target: &str) -> bool {
    world.machines().any(|(_, m)| {
        m.status.as_ref().is_some_and(|v| {
            v.machine.target == target || v.machine.name == target || v.machine.host == target
        })
    })
}

/// One row of the add dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddRow {
    /// How ssh reaches it: what `machine.add` is given.
    pub target: String,
    /// Typed into the field rather than read from the ssh config.
    pub typed: bool,
    /// Beside its name: `user@host`, or its other names.
    pub sub: String,
}

/// The add dialog's rows: the ssh config's hosts not added yet that the field matches (Eli's
/// matching, the alias first), then what is typed, when no host goes by that name.
pub fn add_rows(world: &World, hosts: &[Host], query: &str) -> Vec<AddRow> {
    let open: Vec<&Host> = hosts.iter().filter(|h| !added(world, &h.alias)).collect();
    let mut rows: Vec<AddRow> = filter::rank(query, &open, |h| {
        let mut others = h.aliases.clone();
        others.extend(h.hostname.clone());
        others.extend(h.user.clone());
        (h.alias.clone(), others)
    })
    .into_iter()
    .map(|i| AddRow {
        target: open[i].alias.clone(),
        typed: false,
        sub: open[i].sub(),
    })
    .collect();
    let typed = query.trim();
    let named = |h: &Host| h.alias == typed || h.aliases.iter().any(|a| a == typed);
    if !typed.is_empty() && !hosts.iter().any(named) {
        rows.push(AddRow {
            target: typed.to_owned(),
            typed: true,
            sub: String::new(),
        });
    }
    rows
}

/// What a failed `machine.add` tells the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddFailure {
    pub message: String,
    /// It needs answers only a terminal can take (a password, a new host key, installing
    /// herdr-core there): this command, run in a terminal, does it.
    pub command: Option<String>,
}

/// A failed `machine.add`, as `machine-add/main.js` `add` explains it.
pub fn add_failure(target: &str, error: &RpcError) -> AddFailure {
    let asks = matches!(
        error.kind,
        ErrorKind::HcMissing | ErrorKind::Incompatible | ErrorKind::NeedsLogin
    ) || matches!(&error.kind, ErrorKind::Other(k) if ["login_needed", "auth_failed", "host_key"].contains(&k.as_str()))
        || {
            let message = error.message.to_lowercase();
            [
                "password",
                "passphrase",
                "host key",
                "permission denied",
                "login",
            ]
            .iter()
            .any(|w| message.contains(w))
        };
    if asks {
        let command = format!("hc machine add {}", shell_arg(target));
        AddFailure {
            message: format!(
                "Couldn't add {target} here: it needs you to answer something (a password, a new \
                 host key, or installing herdr-core there). Run `{command}` in a terminal. \
                 `hc machine --help` shows the options."
            ),
            command: Some(command),
        }
    } else {
        let message = error.message.trim();
        AddFailure {
            message: if message.is_empty() {
                "it didn't work".into()
            } else {
                message.to_owned()
            },
            command: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_proto::types::Machine;

    fn view(name: &str, state: MachineState) -> MachineView {
        MachineView {
            state,
            since_ms: None,
            why: None,
            machine: Machine {
                name: name.into(),
                target: name.into(),
                version: "0.1.22".into(),
                ..Default::default()
            },
        }
    }

    fn set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|k| (*k).to_owned()).collect()
    }

    fn world(machines: &[MachineView]) -> World {
        let mut world = World::default();
        world.set_machines(machines);
        world
    }

    #[test]
    fn rows_list_this_computer_then_known_machines_then_ones_known_only_from_agents_or_hidden() {
        let w = world(&[
            view("omarchy", MachineState::Unreachable),
            view("devbox", MachineState::Connected),
        ]);
        let agents: BTreeMap<String, usize> =
            [("".into(), 2), ("devbox".into(), 1), ("gone".into(), 3)].into();
        let rows = filter_rows(&w, &agents, &set(&["old"]));
        let labels: Vec<_> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "All machines",
                "This computer",
                "devbox",
                "omarchy",
                "gone",
                "old"
            ]
        );
        let offline: Vec<_> = rows.iter().map(|r| r.offline).collect();
        assert_eq!(offline, [false, false, false, true, true, true]);
        let counts: Vec<_> = rows.iter().map(|r| r.agents).collect();
        assert_eq!(counts, [6, 2, 1, 0, 3, 0]);
        assert!(!rows[0].shown, "something is hidden");
        assert!(!rows[5].shown && rows[2].shown);
        assert_eq!(rows[1].key.as_deref(), Some(""));
    }

    #[test]
    fn space_toggles_enter_shows_only_one_and_all_machines_shows_every_one() {
        let w = world(&[view("devbox", MachineState::Connected)]);
        let rows = filter_rows(&w, &BTreeMap::new(), &BTreeSet::new());
        assert!(rows[0].shown);
        let hidden = hidden_after(&rows, &BTreeSet::new(), 2, false);
        assert_eq!(hidden, set(&["devbox"]));
        assert_eq!(hidden_after(&rows, &hidden, 2, false), set(&[]));
        assert_eq!(hidden_after(&rows, &hidden, 1, true), set(&["devbox"]));
        assert_eq!(hidden_after(&rows, &BTreeSet::new(), 2, true), set(&[""]));
        assert_eq!(hidden_after(&rows, &hidden, 0, false), set(&[]));
        assert_eq!(hidden_after(&rows, &hidden, 0, true), set(&[]));
    }

    #[test]
    fn shell_words_are_quoted_only_when_they_must_be() {
        assert_eq!(shell_arg("can@10.0.0.7:2222"), "can@10.0.0.7:2222");
        assert_eq!(shell_arg("my box"), "'my box'");
        assert_eq!(shell_arg("it's"), r"'it'\''s'");
        assert_eq!(shell_arg(""), "''");
    }

    #[test]
    fn a_machine_says_what_it_needs_and_its_menu_offers_it() {
        let mut v = view("devbox", MachineState::NeedsLogin);
        v.machine.keeps_running_after_logout = Some(false);
        v.machine.logout_linger_user = Some("eli".into());
        assert_eq!(detail(&v), "devbox · needs login · hc 0.1.22");
        let needs = needs(&v);
        assert_eq!(needs[0].command, "hc machine login devbox");
        assert_eq!(needs[1].command, "sudo loginctl enable-linger -- eli");
        let menu = machine_menu(&v);
        let labels: Vec<_> = menu.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Connect now",
                "Copy hc machine login devbox",
                "Copy sudo loginctl enable-linger -- eli",
                "Rename",
                "Remove"
            ]
        );
        assert_eq!(
            menu.items[1].actions,
            [
                Action::CopyText("hc machine login devbox".into()),
                Action::Open(Popup::Machines)
            ]
        );
        let up = machine_menu(&view("devbox", MachineState::Connected));
        assert_eq!(up.items.len(), 2, "a connected machine needs nothing");
        let mut update = view("devbox", MachineState::NeedsUpdate);
        update.why = Some("too old".into());
        assert_eq!(super::needs(&update)[0].command, "hc machine update devbox");
        assert_eq!(why(&update), Some("too old"));
        let mut fine = view("devbox", MachineState::Connected);
        fine.why = Some("stale".into());
        assert_eq!(why(&fine), None);
    }

    #[test]
    fn the_ssh_config_names_hosts_with_their_includes_and_skips_patterns() {
        let main = "\
# mine
Host devbox db
  HostName 10.0.0.7
  User can
Host *.internal !nope
  User x
Host=omarchy
Include config.d/*
Match host foo
  User ignored
";
        let mut asked = Vec::new();
        let hosts = ssh_hosts(main, &mut |value| {
            asked.push(value.to_owned());
            vec!["Host work-mac\n  HostName mac.local\nHost devbox\n".into()]
        });
        assert_eq!(asked, ["config.d/*"]);
        let names: Vec<_> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(names, ["devbox", "omarchy", "work-mac"]);
        assert_eq!(hosts[0].sub(), "can@10.0.0.7");
        assert_eq!(hosts[0].aliases, ["db"]);
        assert_eq!(hosts[1].sub(), "");
        assert_eq!(hosts[2].sub(), "mac.local");
    }

    #[test]
    fn includes_stop_sixteen_deep() {
        let mut depth = 0;
        let hosts = ssh_hosts("Include again", &mut |_| {
            depth += 1;
            vec![format!("Host h{depth}\nInclude again")]
        });
        assert_eq!(hosts.len(), INCLUDE_DEPTH);
    }

    #[test]
    fn the_add_rows_leave_out_added_hosts_match_the_field_and_offer_what_is_typed() {
        let w = world(&[view("devbox", MachineState::Connected)]);
        let hosts = vec![
            Host {
                alias: "devbox".into(),
                ..Host::default()
            },
            Host {
                alias: "omarchy".into(),
                user: Some("eli".into()),
                hostname: Some("10.0.0.9".into()),
                ..Host::default()
            },
            Host {
                alias: "work-mac".into(),
                aliases: vec!["wm".into()],
                ..Host::default()
            },
        ];
        let targets = |q| -> Vec<String> {
            add_rows(&w, &hosts, q)
                .into_iter()
                .map(|r| r.target)
                .collect()
        };
        assert_eq!(targets(""), ["omarchy", "work-mac"]);
        assert_eq!(targets("work"), ["work-mac", "work"]);
        assert_eq!(targets("work-mac"), ["work-mac"]);
        assert_eq!(targets("wm"), ["work-mac"], "an alias is that host");
        assert_eq!(targets("10.0.0.9"), ["omarchy", "10.0.0.9"]);
        let typed = add_rows(&w, &hosts, " can@vps ");
        assert_eq!(typed.last().unwrap().target, "can@vps");
        assert!(typed.last().unwrap().typed);
    }

    #[test]
    fn a_failed_add_that_needs_answers_says_what_to_run() {
        let error = |kind, message: &str| RpcError {
            code: -32000,
            message: message.into(),
            kind,
            hint: None,
            data: Default::default(),
        };
        let missing = add_failure("my box", &error(ErrorKind::HcMissing, "no hc there"));
        assert_eq!(missing.command.as_deref(), Some("hc machine add 'my box'"));
        assert!(
            missing
                .message
                .contains("Run `hc machine add 'my box'` in a terminal")
        );
        let password = add_failure("vps", &error(ErrorKind::Unreachable, "Permission denied"));
        assert!(password.command.is_some());
        let other = add_failure("vps", &error(ErrorKind::Unreachable, " no route to host "));
        assert_eq!(other.message, "no route to host");
        assert_eq!(other.command, None);
    }
}
