//! The Agents list, as Eli's `eli-agents` mod builds it (`feature-inventory.md` §6;
//! `cfg/mods/eli-agents/main.js`, `watch.js`, `names.js`, `colors.js`), as pure logic: what rows
//! the sidebar shows, in what order, and what each says. The app draws [`List`] and publishes its
//! order ([`List::rows_order`], [`List::nav`]) to the model as [`crate::Event::AgentOrder`].
//!
//! - Agents are terminals in a pane of the layout that report a status (herdr-app's `agents()`),
//!   in space, tab, pane order. Agents on hidden machines leave the list but are still tracked.
//! - A Pi orchestrator's workers (`dotfiles.pi_pr_watch`) nest under it; sub-agents (core
//!   `parent`) nest under their agent with tree lines.
//! - Priority view: Pinned, then Needs you, Done, Working, Idle, each agent at its most urgent
//!   sub-agent's or worker's rank, ties by the last state change, then the change sequence.
//!   Grouped view: Pinned, then space-name headers in model order.
//! - Section headers never fold (Eli folds them by accident, `docs/plan.md`); sub-agents do.
//!
//! [`Tracker`] keeps what outlives one build: each agent's last state change (saved by the app,
//! so times survive restarts), the done agent being looked at (`hold`), the folded sub-agent
//! lists and the change sequence.

use std::collections::{BTreeMap, BTreeSet};

use herdr_proto::{StatusState, Terminal};
use herdr_shared::{Displayed, labels, machine_of};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::Model;
use crate::world::World;

/// Eli's Pi label: a session's orchestration role, ticket and watched PRs (JSON).
pub const WATCH_LABEL: &str = "dotfiles.pi_pr_watch";
/// Shared keys the list reads and writes, by the names Eli's mods gave them.
pub const SORT_KEY: &str = "agents.sort";
pub const ORDER_KEY: &str = "dotfiles.agents.order";
pub const HIDDEN_MACHINES_KEY: &str = "dotfiles.agents.hidden-machines";
pub const SPACE_COLORS_KEY: &str = "space.colors";
/// Gone agents' times are forgotten after this long (ms).
pub const FORGET_AFTER_MS: u64 = 30 * 86_400_000;
/// How often the times on screen refresh, in seconds.
pub const REFRESH_EVERY: f64 = 30.0;

/// How the list is sorted (shared `agents.sort`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Priority,
    Grouped,
}

impl Sort {
    pub const ALL: [Sort; 2] = [Sort::Priority, Sort::Grouped];

    /// The shared value; anything else is priority.
    pub fn parse(value: Option<&str>) -> Sort {
        match value {
            Some("grouped") => Sort::Grouped,
            _ => Sort::Priority,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Sort::Priority => "priority",
            Sort::Grouped => "grouped",
        }
    }

    /// The sort menu's label.
    pub fn label(self) -> &'static str {
        match self {
            Sort::Priority => "Priority",
            Sort::Grouped => "Grouped by space",
        }
    }
}

/// What the list needs of a terminal's program (the app's program table).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    /// The table's key: `pi`, `claude`, `shell`, …
    pub key: String,
    /// For people: "Pi", "Claude Code", …
    pub name: String,
    /// Named by the agent's own status report.
    pub agent: bool,
}

/// What a build reads besides the model.
pub struct Input<'a> {
    pub sort: Sort,
    /// The raw shared `agents.sort` (for the order's signature, as herdr-app writes it).
    pub sort_value: Option<&'a str>,
    /// Machines whose agents leave the list (`""` is this computer).
    pub hidden_machines: &'a BTreeSet<String>,
    /// Shared `space.colors`, by space id.
    pub space_colors: &'a BTreeMap<String, String>,
    /// Wall clock, ms since the epoch.
    pub now_ms: u64,
    /// A terminal's program.
    pub program: &'a dyn Fn(&Terminal) -> Program,
    /// The terminal a peek shows, if any: its sub-agent row is raised.
    pub peeked: Option<&'a str>,
}

/// A tree line in front of a sub-agent: nothing, `│`, `├` or `└`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tree {
    Blank,
    Pipe,
    Middle,
    Last,
}

/// A worker's elbow under its orchestrator: `├` (more follow) or `└`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Branch {
    Middle,
    Last,
}

/// A row's sub-agent fold control. Folded, it says how many and the most urgent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    pub open: bool,
    pub count: usize,
    pub top: Option<Displayed>,
}

/// The PRs a Pi session watches, as one chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prs {
    pub first: String,
    /// How many more ("+N").
    pub more: usize,
    /// The whole list, `· paused` when paused.
    pub tooltip: String,
    pub paused: bool,
}

/// A section header: Pinned, a priority section or a space. Not clickable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub id: String,
    pub label: String,
    /// Colors the label (Needs you, Done, Working); `None` is dim.
    pub status: Option<Displayed>,
    /// How many agents, in the priority view.
    pub count: Option<usize>,
    /// The first header sits right under the top row.
    pub first: bool,
}

/// An agent's two-line row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRow {
    pub address: String,
    pub pane: String,
    pub title: String,
    pub status: Displayed,
    pub focused: bool,
    pub pinned: bool,
    /// Under a priority header, blocked shows a compact "!".
    pub compact: bool,
    /// Its jump label (1–9, 0, then letters).
    pub label: String,
    /// The space's name on line 2, and its text color (`#rrggbb`).
    pub space: Option<String>,
    pub space_color: Option<String>,
    /// An orchestrator's Jira ticket.
    pub ticket: Option<String>,
    /// How many workers nest under it.
    pub workers: usize,
    pub prs: Option<Prs>,
    /// The machine of a remote agent.
    pub machine: Option<String>,
    /// Since its last state change (`<1m`, `5m`, `2h`, `3d`).
    pub ago: Option<String>,
    /// A worker's elbow under its orchestrator.
    pub line: Option<Branch>,
    /// Its sub-agents' fold control, when it has sub-agents and no workers.
    pub fold: Option<Fold>,
}

/// A sub-agent's one-line row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubRow {
    pub address: String,
    pub name: String,
    pub lines: Vec<Tree>,
    /// Under a worker: whether its orchestrator's line goes on past it.
    pub lead: Option<bool>,
    /// Its status word as the list shows it (never done; `None` draws nothing).
    pub status: Option<Displayed>,
    pub compact: bool,
    /// The peeked terminal.
    pub on: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header(Header),
    Agent(Box<AgentRow>),
    Sub(SubRow),
}

/// Why the list is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Empty {
    /// No agent reports a status.
    NoAgents,
    /// Every agent is on a hidden machine.
    AllHidden,
}

impl Empty {
    pub fn message(self) -> &'static str {
        match self {
            Empty::NoAgents => {
                "No agent status yet. Agents show up here when they report their status. Older \
                 versions and agents inside tmux may not."
            }
            Empty::AllHidden => {
                "Every agent is on a hidden machine. Ctrl+G, / chooses which machines show."
            }
        }
    }
}

/// One build of the list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct List {
    pub sort: Sort,
    pub rows: Vec<Row>,
    /// The agents on rows, in order: the row numbers and jump labels.
    pub rows_order: Vec<String>,
    /// Navigation order: every listed agent and all its workers, folded or not.
    pub nav: Vec<String>,
    /// The published order (shared `dotfiles.agents.order`).
    pub published: Value,
    /// Labels have two letters, so their column is wider.
    pub wide_labels: bool,
    pub empty: Option<Empty>,
    /// The space colors to keep, when they differ from what was stored.
    pub space_colors: Option<BTreeMap<String, String>>,
}

/// An agent's last state change, as `eli-agents` keeps it (`{s, at}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changed {
    pub s: String,
    pub at: u64,
}

/// What outlives one build.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tracker {
    /// Each agent's last state change, done and idle counting as one.
    pub changed: BTreeMap<String, Changed>,
    /// `changed` moved since it was last saved.
    pub dirty: bool,
    /// The focused agent and its rank when focus arrived (`hold`).
    held: Option<(String, u8)>,
    /// Agents whose sub-agents are folded away.
    pub folded: BTreeSet<String>,
    /// Each agent's last status word and its change sequence.
    seqs: BTreeMap<String, (Option<StatusState>, u64)>,
    /// Each agent's status in the last build.
    last: BTreeMap<String, Displayed>,
    next_seq: u64,
}

/// herdr-app's attention rank: blocked 4, done 3, working 2, idle 1, nothing 0.
pub fn rank(status: Option<Displayed>) -> u8 {
    match status {
        Some(Displayed::Blocked) => 4,
        Some(Displayed::Done) => 3,
        Some(Displayed::Working) => 2,
        Some(Displayed::Idle) => 1,
        None => 0,
    }
}

fn section(rank: u8) -> (&'static str, Option<Displayed>) {
    match rank {
        4 => ("Needs you", Some(Displayed::Blocked)),
        3 => ("Done", Some(Displayed::Done)),
        2 => ("Working", Some(Displayed::Working)),
        _ => ("Idle", None),
    }
}

/// How long ago, as the list says it.
pub fn duration(ms: u64) -> String {
    let m = ms / 60_000;
    if m < 1 {
        return "<1m".into();
    }
    if m < 60 {
        return format!("{m}m");
    }
    let h = m / 60;
    if h < 24 {
        format!("{h}h")
    } else {
        format!("{}d", h / 24)
    }
}

/// The fixed colors the space color names stand for (not the theme's ANSI palette).
const MARKS: [(&str, &str); 6] = [
    ("ansi6", "#22a99a"),
    ("ansi3", "#e3814f"),
    ("ansi4", "#5b8def"),
    ("ansi5", "#8b7cf6"),
    ("ansi2", "#3fae6a"),
    ("ansi1", "#d9649a"),
];

/// The colors to keep for `spaces` (`(id, name)`, in order), as the shipped picker assigns them
/// (`cfg/mods/eli-agents/colors.js`): a space without one gets the color fewest spaces use, once;
/// home (`~`) gets none. Stored colors of spaces that are gone go, except on machines whose
/// layout isn't read yet (`loaded`), so a slow machine keeps its colors. `None` when nothing
/// changed.
pub fn assign_space_colors(
    spaces: &[(String, String)],
    stored: &BTreeMap<String, String>,
    loaded: impl Fn(&str) -> bool,
) -> Option<BTreeMap<String, String>> {
    let ids: BTreeSet<&str> = spaces.iter().map(|(id, _)| id.as_str()).collect();
    let mut kept: BTreeMap<String, String> = stored
        .iter()
        .filter(|(id, _)| ids.contains(id.as_str()) || !loaded(id))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut changed = kept.len() != stored.len();
    for (id, name) in spaces {
        if kept.get(id).is_some_and(|c| !c.is_empty()) || name == "~" {
            continue;
        }
        let uses = |c: &str| kept.values().filter(|v| v.as_str() == c).count();
        let mut best = MARKS[0].0;
        for (c, _) in &MARKS[1..] {
            if uses(c) < uses(best) {
                best = c;
            }
        }
        kept.insert(id.clone(), best.to_owned());
        changed = true;
    }
    changed.then_some(kept)
}

/// A stored space color as `#rrggbb`: a color name's fixed mark, or a picked color as is. `None`
/// when it is neither (home's empty one).
pub fn space_mark(value: &str) -> Option<&str> {
    MARKS
        .iter()
        .find(|(name, _)| *name == value)
        .map(|(_, hex)| *hex)
        .or_else(|| {
            (value.len() == 7
                && value.starts_with('#')
                && value[1..].chars().all(|c| c.is_ascii_hexdigit()))
            .then_some(value)
        })
}

/// A space color as text: its mark lifted 28% toward `#e8e8e8`. `None` when it has none.
pub fn space_text_color(value: &str) -> Option<String> {
    let hex = space_mark(value)?;
    let channel = |i: usize| {
        let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
        (f64::from(v) * 0.72 + f64::from(0xe8u8) * 0.28).round() as u8
    };
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        channel(1),
        channel(3),
        channel(5)
    ))
}

/// A Pi session's watch label, trusted only while the terminal runs Pi.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Watch {
    pub role: Option<String>,
    pub orchestration: Option<String>,
    pub ticket: Option<String>,
    pub prs: Vec<String>,
    pub sha: Option<String>,
    pub paused: bool,
}

impl Watch {
    /// The label of a terminal whose program is `program`.
    pub fn of(terminal: &Terminal, program: &Program) -> Option<Watch> {
        if program.key != "pi" {
            return None;
        }
        let raw = terminal.label(WATCH_LABEL).filter(|v| !v.is_empty())?;
        let value: Value = serde_json::from_str(raw).ok()?;
        let obj = value.as_object()?;
        let string = |k: &str| match obj.get(k) {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        };
        let truthy = |v: Option<&Value>| match v {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
            Some(Value::String(s)) => !s.is_empty(),
            Some(_) => true,
        };
        let prs = match obj.get("prs") {
            Some(Value::Array(list)) => list
                .iter()
                .filter_map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        Some(Watch {
            role: string("role"),
            orchestration: string("orchestration").filter(|s| !s.is_empty()),
            ticket: string("ticket").filter(|s| !s.is_empty()),
            prs,
            sha: string("sha").filter(|s| !s.is_empty()),
            paused: truthy(obj.get("paused")),
        })
    }

    /// Everything watched: the PRs, then the sha.
    fn watched(&self) -> Vec<String> {
        self.prs.iter().cloned().chain(self.sha.clone()).collect()
    }

    /// "#1901, tool#7 · paused", or `None` when nothing is watched.
    pub fn text(&self) -> Option<String> {
        let watched = self.watched();
        if watched.is_empty() {
            return None;
        }
        let paused = if self.paused { " · paused" } else { "" };
        Some(format!("{}{paused}", watched.join(", ")))
    }

    fn chip(&self) -> Option<Prs> {
        let watched = self.watched();
        Some(Prs {
            first: watched.first()?.clone(),
            more: watched.len() - 1,
            tooltip: self.text()?,
            paused: self.paused,
        })
    }
}

/// A Pi session's name from its terminal title (`π - <name> - <folder>`), for an unnamed tab
/// (`cfg/mods/eli-agents/names.js` `piSessionName`).
pub fn pi_session_name(title: &str, cwd: Option<&str>) -> Option<String> {
    let rest = title.strip_prefix("π - ")?;
    let folder = cwd
        .map(|c| c.trim_end_matches('/'))
        .and_then(|c| c.rsplit('/').next())
        .filter(|f| !f.is_empty());
    if Some(rest) == folder {
        return None;
    }
    let end = match folder.map(|f| format!(" - {f}")) {
        Some(suffix) if rest.ends_with(&suffix) => Some(rest.len() - suffix.len()),
        _ => rest.rfind(" - "),
    }?;
    if end == 0 {
        return None;
    }
    let name: String = rest[..end].chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Leading characters that are neither letters nor digits go, then outer spaces.
fn clean(s: &str) -> &str {
    s.trim_start_matches(|c: char| !c.is_alphanumeric()).trim()
}

fn numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// One pane as `panes()` describes it.
struct PaneInfo<'a> {
    tab: &'a str,
    title: &'a str,
    cwd: Option<&'a str>,
    app: Option<&'a str>,
    program: Program,
    focused: bool,
}

/// One agent as `agents()` describes it.
#[derive(Clone)]
struct Agent<'a> {
    address: &'a str,
    pane: &'a str,
    space: &'a str,
    space_name: String,
    tab: &'a str,
    status: Displayed,
    program: Program,
    title: &'a str,
    seq: u64,
    pinned: bool,
    focused: bool,
}

/// One tab as `spaces()` describes it.
struct TabInfo<'a> {
    id: &'a str,
    index: usize,
    /// Its name, or its position when nobody named it.
    name: String,
    named: bool,
}

/// The terminal that started `t`, among the terminals: as given, else the same id on `t`'s own
/// machine (a remote record may name its machine by host name or core id,
/// `herdr-core-protocol.md` §5).
pub(crate) fn parent_of<'a>(world: &'a World, t: &'a Terminal) -> Option<&'a str> {
    let parent = t.parent.as_deref().filter(|p| !p.is_empty())?;
    if world.terminal(parent).is_some() {
        return Some(parent);
    }
    let id = parent.rsplit('/').next().unwrap_or(parent);
    let mine = match machine_of(&t.address) {
        Some(m) => format!("{m}/{id}"),
        None => id.to_owned(),
    };
    world
        .terminal(&mine)
        .map(|found| found.address.as_str())
        .or(Some(parent))
}

impl Tracker {
    /// A tracker with saved times (`{address: {s, at}}`, as `eli-agents` stores them).
    pub fn with_times(changed: BTreeMap<String, Changed>) -> Tracker {
        Tracker {
            changed,
            ..Tracker::default()
        }
    }

    /// Folds or unfolds an agent's sub-agents.
    pub fn toggle_fold(&mut self, address: &str) {
        if !self.folded.remove(address) {
            self.folded.insert(address.to_owned());
        }
    }

    /// Stamps agents whose state changed; done and idle count as one. Gone agents' old times go.
    fn track(&mut self, everyone: &[Agent], now: u64) {
        let mut dirty = false;
        for a in everyone {
            let s = match a.status {
                Displayed::Done | Displayed::Idle => "idle",
                Displayed::Working => "working",
                Displayed::Blocked => "blocked",
            };
            if self.changed.get(a.address).is_some_and(|c| c.s == s) {
                continue;
            }
            self.changed.insert(
                a.address.to_owned(),
                Changed {
                    s: s.to_owned(),
                    at: now,
                },
            );
            dirty = true;
        }
        if !dirty {
            return;
        }
        let listed: BTreeSet<&str> = everyone.iter().map(|a| a.address).collect();
        self.changed.retain(|k, c| {
            listed.contains(k.as_str()) || now.saturating_sub(c.at) <= FORGET_AFTER_MS
        });
        self.dirty = true;
    }

    /// Bumps an agent's sequence each time its status word changes.
    fn seq(&mut self, address: &str, word: Option<&StatusState>) -> u64 {
        match self.seqs.get(address) {
            Some((w, seq)) if w.as_ref() == word => *seq,
            _ => {
                self.next_seq += 1;
                self.seqs
                    .insert(address.to_owned(), (word.cloned(), self.next_seq));
                self.next_seq
            }
        }
    }

    /// Looking at a done agent makes it idle; it stays with Done until you leave it. Arriving,
    /// or any change but done turning idle, sets what it was. Focusing marks it seen in the
    /// same step, so arriving on an agent that was done in the last build holds it as done.
    fn hold(&mut self, list: &[Agent]) {
        let cur = list.iter().find(|a| a.focused);
        let same = cur.is_some_and(|c| self.held.as_ref().is_some_and(|(h, _)| h == c.address));
        if !same || cur.is_some_and(|c| c.status != Displayed::Idle) {
            self.held = cur.map(|c| {
                let was_done = !same
                    && c.status == Displayed::Idle
                    && self.last.get(c.address) == Some(&Displayed::Done);
                let status = if was_done { Displayed::Done } else { c.status };
                (c.address.to_owned(), rank(Some(status)))
            });
        }
        self.last = list
            .iter()
            .map(|a| (a.address.to_owned(), a.status))
            .collect();
    }

    fn rank(&self, a: &Agent) -> u8 {
        let held_done = self
            .held
            .as_ref()
            .is_some_and(|(h, r)| h == a.address && *r == rank(Some(Displayed::Done)));
        if held_done && a.status == Displayed::Idle {
            rank(Some(Displayed::Done))
        } else {
            rank(Some(a.status))
        }
    }

    fn since(&self, address: &str) -> u64 {
        self.changed.get(address).map_or(0, |c| c.at)
    }

    /// Builds the list.
    pub fn build(&mut self, model: &Model, input: &Input) -> List {
        let world = model.world();
        let attention = model.attention();
        let home = model.home();
        let focused_pane = model.focused_pane();
        let active = model.active_space().map(|s| s.id.clone());
        let program = input.program;

        // panes(), agents() and spaces() as herdr-app gives them to mods.
        let mut panes: Vec<PaneInfo> = Vec::new();
        let mut everyone: Vec<Agent> = Vec::new();
        let mut spaces: Vec<(String, String)> = Vec::new();
        let mut tabs: Vec<(String, Vec<TabInfo>)> = Vec::new();
        for space in world.spaces() {
            let space_name = world.space_name(space, home);
            spaces.push((space.id.clone(), space_name.clone()));
            let mut infos = Vec::new();
            for (ci, tab) in space.tabs.iter().enumerate() {
                infos.push(TabInfo {
                    id: &tab.id,
                    index: ci + 1,
                    name: crate::world::tab_name(space, tab),
                    named: !tab.name.is_empty(),
                });
                for pane in tab.root.panes() {
                    let Some(address) = pane.terminal() else {
                        continue;
                    };
                    let focused = active.as_deref() == Some(space.id.as_str())
                        && focused_pane.as_deref() == Some(pane.id.as_str());
                    let record = world.terminal(address);
                    let prog = record.map(program).unwrap_or_default();
                    let title = record.map_or("", |t| t.title.as_str());
                    panes.push(PaneInfo {
                        tab: &tab.id,
                        title,
                        cwd: world.folder_of(address),
                        app: record
                            .and_then(|t| t.status.as_ref())
                            .map(|s| s.app.as_str())
                            .filter(|a| !a.is_empty()),
                        program: prog.clone(),
                        focused,
                    });
                    let Some(status) = attention.displayed(address) else {
                        continue;
                    };
                    let word = record.and_then(|t| t.status.as_ref()).map(|s| &s.state);
                    let seq = self.seq(address, word);
                    everyone.push(Agent {
                        address,
                        pane: &pane.id,
                        space: &space.id,
                        space_name: space_name.clone(),
                        tab: &tab.id,
                        status,
                        program: prog,
                        title,
                        seq,
                        pinned: record.is_some_and(|t| labels::pinned(&t.labels)),
                        focused,
                    });
                }
            }
            tabs.push((space.id.clone(), infos));
        }

        let hidden = input.hidden_machines;
        let machine_key = |a: &str| machine_of(a).unwrap_or("").to_owned();
        let visible: Vec<Agent> = everyone
            .iter()
            .filter(|a| !hidden.contains(&machine_key(a.address)))
            .cloned()
            .collect();
        // Every agent's status (sub-agents' and workers' ranks read it).
        let listed: BTreeMap<&str, Displayed> =
            everyone.iter().map(|a| (a.address, a.status)).collect();
        self.track(&everyone, input.now_ms);
        self.hold(&visible);
        let by_priority = input.sort == Sort::Priority;

        // Every terminal, in the order they were made, for the sub-agent trees.
        let mut all: Vec<&Terminal> = world.terminals().collect();
        all.sort_by(|a, b| {
            (a.created_ms, a.address.len(), &a.address).cmp(&(
                b.created_ms,
                b.address.len(),
                &b.address,
            ))
        });
        let mut kids: BTreeMap<&str, Vec<&Terminal>> = BTreeMap::new();
        for t in &all {
            if !t.is_live() {
                continue;
            }
            if let Some(parent) = parent_of(world, t) {
                kids.entry(parent).or_default().push(t);
            }
        }
        // A sub-agent's word as the list counts it: error is blocked, done is idle.
        let kid_status = |t: &Terminal| -> Option<Displayed> {
            let status = listed.get(t.address.as_str()).copied().or_else(|| {
                t.status.as_ref().and_then(|s| match &s.state {
                    StatusState::Working => Some(Displayed::Working),
                    StatusState::Blocked | StatusState::Error => Some(Displayed::Blocked),
                    StatusState::Idle | StatusState::Done => Some(Displayed::Idle),
                    StatusState::Other(_) => None,
                })
            });
            status.map(reported)
        };
        let tree = |root: &str| -> Vec<(&Terminal, Vec<Tree>)> {
            let mut out = Vec::new();
            walk(&kids, root, &[], &mut out);
            out
        };
        let kids_rank = |address: &str| -> u8 {
            tree(address)
                .iter()
                .map(|(t, _)| rank(kid_status(t)))
                .max()
                .unwrap_or(0)
        };

        // Pi orchestrations: workers nest under their orchestrator, in list order.
        let watches: BTreeMap<&str, Watch> = visible
            .iter()
            .filter_map(|a| {
                let t = world.terminal(a.address)?;
                Some((a.address, Watch::of(t, &a.program)?))
            })
            .collect();
        let mut orchestrators: BTreeMap<&str, &str> = BTreeMap::new();
        for a in &visible {
            if let Some(w) = watches.get(a.address)
                && w.role.as_deref() == Some("orchestrator")
                && let Some(o) = w.orchestration.as_deref()
            {
                orchestrators.entry(o).or_insert(a.address);
            }
        }
        let mut workers: BTreeMap<&str, Vec<Agent>> = BTreeMap::new();
        let mut nested: BTreeSet<&str> = BTreeSet::new();
        for a in &visible {
            let Some(w) = watches.get(a.address) else {
                continue;
            };
            if w.role.as_deref() != Some("worker") {
                continue;
            }
            let Some(parent) = w
                .orchestration
                .as_deref()
                .and_then(|o| orchestrators.get(o))
            else {
                continue;
            };
            if *parent == a.address {
                continue;
            }
            workers.entry(parent).or_default().push(a.clone());
            nested.insert(a.address);
        }
        let workers_of = |a: &str| workers.get(a).cloned().unwrap_or_default();

        // A worker moves its orchestrator like a sub-agent: needing you or working lifts it, a
        // done worker doesn't.
        let urgency: BTreeMap<&str, u8> = visible
            .iter()
            .map(|a| {
                let own = self.rank(a).max(kids_rank(a.address));
                let from_workers = workers_of(a.address)
                    .iter()
                    .map(|w| rank(Some(reported(w.status))).max(kids_rank(w.address)))
                    .max()
                    .unwrap_or(0);
                (a.address, own.max(from_workers))
            })
            .collect();
        let rank_of = |a: &Agent| urgency.get(a.address).copied().unwrap_or(0);

        let mut list: Vec<Agent> = visible
            .iter()
            .filter(|a| !nested.contains(a.address))
            .cloned()
            .collect();
        if by_priority {
            list.sort_by(|x, y| {
                rank_of(y)
                    .cmp(&rank_of(x))
                    .then(self.since(y.address).cmp(&self.since(x.address)))
                    .then(y.seq.cmp(&x.seq))
            });
        }
        let (pinned, rest): (Vec<Agent>, Vec<Agent>) = list.into_iter().partition(|a| a.pinned);
        let list: Vec<Agent> = pinned.iter().chain(&rest).cloned().collect();

        // An orchestrator opens only while it or a worker is focused; other rows fold by hand.
        let open = |a: &Agent| {
            let ws = workers_of(a.address);
            if ws.is_empty() {
                !self.folded.contains(a.address)
            } else {
                a.focused || ws.iter().any(|w| w.focused)
            }
        };
        let order: Vec<&Agent> = list
            .iter()
            .flat_map(|a| {
                let ws = if open(a) {
                    workers.get(a.address).map(Vec::as_slice).unwrap_or(&[])
                } else {
                    &[]
                };
                std::iter::once(a).chain(ws.iter())
            })
            .collect();
        let nav: Vec<String> = list
            .iter()
            .flat_map(|a| {
                let ws = workers.get(a.address).map(Vec::as_slice).unwrap_or(&[]);
                std::iter::once(a.address).chain(ws.iter().map(|w| w.address))
            })
            .map(str::to_owned)
            .collect();
        let rows_order: Vec<String> = order.iter().map(|a| a.address.to_owned()).collect();
        let inputs = {
            let focused = visible.iter().find(|a| a.focused).map_or("", |a| a.address);
            let mut parts = vec![
                input.sort_value.unwrap_or("").to_owned(),
                focused.to_owned(),
            ];
            parts.extend(visible.iter().map(|a| {
                format!(
                    "{}:{}:{}",
                    a.address,
                    a.status.as_str(),
                    if a.pinned { 1 } else { 0 }
                )
            }));
            parts.join("\n")
        };
        let published = serde_json::json!({"inputs": inputs, "addresses": rows_order});
        let jump = crate::mode::jump_labels(order.len());
        let wide_labels = jump.last().is_some_and(|l| l.len() > 1);
        let label_of: BTreeMap<&str, &str> = order
            .iter()
            .zip(&jump)
            .map(|(a, l)| (a.address, l.as_str()))
            .collect();

        // Space colors.
        let space_colors =
            assign_space_colors(&spaces, input.space_colors, |id| world.loaded_for(id));
        let colors = space_colors.as_ref().unwrap_or(input.space_colors);
        let space_color =
            |space: &str| colors.get(space).and_then(|v| space_text_color(v.as_str()));

        // Titles: a named tab, else the agent's own title; never an automatic tab number unless
        // two rows in a space would otherwise read the same.
        let mut leads: BTreeMap<&str, &PaneInfo> = BTreeMap::new();
        for p in &panes {
            let replace = match leads.get(p.tab) {
                None => true,
                Some(cur) => !cur.program.agent && (p.program.agent || (!cur.focused && p.focused)),
            };
            if replace {
                leads.insert(p.tab, p);
            }
        }
        let mut tab_labels: BTreeMap<&str, String> = BTreeMap::new();
        for (_, infos) in &tabs {
            let names = tab_names(infos, &panes);
            for tab in infos {
                let name = names.get(tab.id).cloned().unwrap_or_default();
                let label = if tab.named || name != tab.name {
                    name
                } else {
                    title(tab, leads.get(tab.id).copied())
                };
                tab_labels.insert(tab.id, label);
            }
        }
        let label = |a: &Agent| -> String {
            tab_labels.get(a.tab).cloned().unwrap_or_else(|| {
                if a.title.is_empty() {
                    a.program.name.clone()
                } else {
                    a.title.to_owned()
                }
            })
        };
        let mut detail: BTreeMap<&str, Option<String>> = order
            .iter()
            .map(|a| {
                let l = label(a);
                (a.address, (!numeric(&l)).then_some(l))
            })
            .collect();
        let mut seen: BTreeMap<(String, String), usize> = BTreeMap::new();
        for a in &order {
            let d = detail[a.address].clone().unwrap_or_default();
            *seen.entry((a.space_name.clone(), d)).or_default() += 1;
        }
        for a in &order {
            let crowded = seen
                .get(&(a.space_name.clone(), String::new()))
                .is_some_and(|n| *n > 1);
            if detail[a.address].is_none() && crowded {
                detail.insert(a.address, Some(label(a)));
            }
        }

        let sub_row = |t: &Terminal, lines: Vec<Tree>, lead: Option<bool>| -> Row {
            let prog = program(t);
            let name = [t.name.as_deref(), t.template.as_deref()]
                .into_iter()
                .flatten()
                .find(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| (!prog.name.is_empty()).then(|| prog.name.clone()))
                .unwrap_or_else(|| t.address.clone());
            Row::Sub(SubRow {
                address: t.address.clone(),
                name,
                lines,
                lead,
                status: kid_status(t),
                compact: by_priority,
                on: input.peeked == Some(t.address.as_str()),
            })
        };
        let agent_row =
            |a: &Agent, subs: &[(&Terminal, Vec<Tree>)], ws: usize, line: Option<Branch>| {
                let foldable = if ws > 0 { &[][..] } else { subs };
                let fold = (!foldable.is_empty()).then(|| Fold {
                    open: !self.folded.contains(a.address),
                    count: foldable.len(),
                    top: foldable.iter().filter_map(|(t, _)| kid_status(t)).fold(
                        None,
                        |m: Option<Displayed>, s| {
                            if rank(Some(s)) > rank(m) { Some(s) } else { m }
                        },
                    ),
                });
                let l = label(a);
                let title = detail[a.address].clone().unwrap_or_else(|| {
                    if numeric(&l) && !a.program.name.is_empty() {
                        a.program.name.clone()
                    } else {
                        l
                    }
                });
                let full = by_priority || a.pinned;
                let watch = watches.get(a.address);
                let space = (line.is_none() && full).then(|| a.space_name.clone());
                let when = self.since(a.address);
                AgentRow {
                    address: a.address.to_owned(),
                    pane: a.pane.to_owned(),
                    title,
                    status: a.status,
                    focused: a.focused,
                    pinned: a.pinned,
                    compact: by_priority && !a.pinned,
                    label: label_of.get(a.address).copied().unwrap_or("").to_owned(),
                    space_color: space.as_ref().and_then(|_| space_color(a.space)),
                    space,
                    ticket: if line.is_none() && ws > 0 {
                        watch.and_then(|w| w.ticket.clone())
                    } else {
                        None
                    },
                    workers: ws,
                    prs: watch.and_then(Watch::chip),
                    machine: machine_of(a.address).map(str::to_owned),
                    ago: (when > 0).then(|| duration(input.now_ms.saturating_sub(when))),
                    line,
                    fold,
                }
            };
        let with_kids = |a: &Agent| -> Vec<Row> {
            let subs = tree(a.address);
            let ws = workers_of(a.address);
            let mut out = vec![Row::Agent(Box::new(agent_row(a, &subs, ws.len(), None)))];
            if !open(a) {
                return out;
            }
            out.extend(subs.into_iter().map(|(t, lines)| sub_row(t, lines, None)));
            for (i, w) in ws.iter().enumerate() {
                let last = i + 1 == ws.len();
                let branch = if last { Branch::Last } else { Branch::Middle };
                out.push(Row::Agent(Box::new(agent_row(w, &[], 0, Some(branch)))));
                for (t, lines) in tree(w.address) {
                    out.push(sub_row(t, lines, Some(!last)));
                }
            }
            out
        };

        // Groups in list order: Pinned, then spaces (grouped) or priority sections. Agents of a
        // space are next to each other, so neighbours make a group.
        let mut groups: Vec<(Header, Vec<&Agent>)> = Vec::new();
        if !pinned.is_empty() {
            groups.push((
                Header {
                    id: "pinned".into(),
                    label: "Pinned".into(),
                    status: None,
                    count: None,
                    first: false,
                },
                list.iter().filter(|a| a.pinned).collect(),
            ));
        }
        for a in list.iter().filter(|a| !a.pinned) {
            let r = match rank_of(a) {
                0 => 1,
                r => r,
            };
            let id = if by_priority {
                format!("sec-{r}")
            } else {
                format!("sp-{}", a.space)
            };
            if let Some((h, members)) = groups.last_mut()
                && h.id == id
            {
                members.push(a);
                continue;
            }
            let (label, status) = if by_priority {
                let (l, s) = section(r);
                (l.to_owned(), s)
            } else {
                (a.space_name.clone(), None)
            };
            groups.push((
                Header {
                    id,
                    label,
                    status,
                    count: None,
                    first: false,
                },
                vec![a],
            ));
        }
        let mut rows = Vec::new();
        for (i, (mut header, members)) in groups.into_iter().enumerate() {
            header.first = i == 0;
            header.count = by_priority.then_some(members.len());
            rows.push(Row::Header(header));
            for a in members {
                rows.extend(with_kids(a));
            }
        }

        let why = if everyone.is_empty() {
            Empty::NoAgents
        } else {
            Empty::AllHidden
        };
        let empty = list.is_empty().then_some(why);
        List {
            sort: input.sort,
            rows,
            rows_order,
            nav,
            published,
            wide_labels,
            empty,
            space_colors,
        }
    }
}

/// A sub-agent's or worker's status as it counts for its parent: its done is idle, since it
/// reports to the parent and the parent's done is the news.
fn reported(status: Displayed) -> Displayed {
    match status {
        Displayed::Done => Displayed::Idle,
        s => s,
    }
}

/// Live sub-agents of `address`, depth first, each with the tree lines in front of it. At most
/// 32 deep, so a parent cycle ends.
fn walk<'t>(
    kids: &BTreeMap<&str, Vec<&'t Terminal>>,
    address: &str,
    lead: &[Tree],
    out: &mut Vec<(&'t Terminal, Vec<Tree>)>,
) {
    if lead.len() >= 32 {
        return;
    }
    let Some(list) = kids.get(address) else {
        return;
    };
    for (i, k) in list.iter().enumerate() {
        let last = i + 1 == list.len();
        let mut lines = lead.to_vec();
        lines.push(if last { Tree::Last } else { Tree::Middle });
        out.push((*k, lines));
        let mut next = lead.to_vec();
        next.push(if last { Tree::Blank } else { Tree::Pipe });
        walk(kids, &k.address, &next, out);
    }
}

/// A tab's title from its lead pane: the program's live title when the tab is unnamed or named
/// after its program, and the title says something (`main.js` `title`).
fn title(tab: &TabInfo, lead: Option<&PaneInfo>) -> String {
    let Some(l) = lead else {
        return tab.name.clone();
    };
    let prog = l.program.name.as_str();
    let live = clean(l.title);
    // A path says nothing. (The mod tests the cleaned title, whose leading `/` or `~` is already
    // gone, so its test missed Unix paths; the raw title is tested too.)
    let is_path = |s: &str| {
        s.starts_with(['~', '/'])
            || (s.len() > 2
                && s.as_bytes()[0].is_ascii_alphabetic()
                && &s.as_bytes()[1..3] == b":\\")
    };
    let path = is_path(live) || is_path(l.title.trim());
    let telling = !live.is_empty()
        && live != prog
        && !path
        && !live.contains('@')
        && live.to_lowercase() != l.program.key;
    if (tab.name == prog || !tab.named) && telling {
        live.to_owned()
    } else {
        tab.name.clone()
    }
}

/// Each tab's name, with Pi session names for unnamed tabs, numbered apart when they collide
/// (`names.js` `tabNames`).
fn tab_names<'a>(tabs: &'a [TabInfo], panes: &[PaneInfo]) -> BTreeMap<&'a str, String> {
    let mut automatic: BTreeSet<&str> = BTreeSet::new();
    let mut names: BTreeMap<&str, String> = BTreeMap::new();
    for tab in tabs {
        let mut name = tab.name.clone();
        if !tab.named {
            let lead = panes
                .iter()
                .find(|p| p.tab == tab.id && (p.program.key == "pi" || p.app == Some("pi")));
            if let Some(session) = lead.and_then(|p| pi_session_name(p.title, p.cwd)) {
                automatic.insert(tab.id);
                name = session;
            }
        }
        names.insert(tab.id, name);
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for name in names.values() {
        *counts.entry(name.clone()).or_default() += 1;
    }
    let mut used: BTreeSet<String> = names.values().cloned().collect();
    for tab in tabs {
        let name = names[tab.id].clone();
        if !automatic.contains(tab.id) || counts.get(&name).copied().unwrap_or(0) < 2 {
            continue;
        }
        let suffix = format!("{name} · {}", tab.index);
        let mut unique = suffix.clone();
        let mut attempt = 2;
        while used.contains(&unique) {
            unique = format!("{suffix}.{attempt}");
            attempt += 1;
        }
        used.insert(unique.clone());
        names.insert(tab.id, unique);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_space_colors_read_as_their_marks() {
        assert_eq!(space_mark("ansi6"), Some("#22a99a"));
        assert_eq!(space_mark("#123456"), Some("#123456"));
        assert_eq!(space_mark(""), None);
        assert_eq!(space_mark("teal"), None);
    }

    #[test]
    fn durations_read_as_the_list_writes_them() {
        assert_eq!(duration(0), "<1m");
        assert_eq!(duration(59_999), "<1m");
        assert_eq!(duration(60_000), "1m");
        assert_eq!(duration(59 * 60_000), "59m");
        assert_eq!(duration(60 * 60_000), "1h");
        assert_eq!(duration(23 * 3_600_000 + 59 * 60_000), "23h");
        assert_eq!(duration(49 * 3_600_000), "2d");
    }

    #[test]
    fn spaces_get_the_least_used_color_once_and_home_none() {
        let spaces: Vec<(String, String)> = [("a", "x"), ("b", "~"), ("c", "y"), ("d", "z")]
            .iter()
            .map(|(i, n)| (i.to_string(), n.to_string()))
            .collect();
        let stored: BTreeMap<String, String> = [("c".into(), "ansi6".into())].into();
        let kept = assign_space_colors(&spaces, &stored, |_| true).unwrap();
        assert_eq!(kept.get("c").map(String::as_str), Some("ansi6"), "kept");
        assert_eq!(kept.get("a").map(String::as_str), Some("ansi3"));
        assert_eq!(kept.get("b"), None, "home gets none");
        assert_eq!(kept.get("d").map(String::as_str), Some("ansi4"));
        assert_eq!(assign_space_colors(&spaces, &kept, |_| true), None);
        // A gone space's color goes, unless its machine isn't read yet.
        let mut more = kept.clone();
        more.insert("devbox/sp9".into(), "ansi5".into());
        more.insert("gone".into(), "ansi5".into());
        let pruned = assign_space_colors(&spaces, &more, |id| !id.starts_with("devbox/")).unwrap();
        assert!(pruned.contains_key("devbox/sp9") && !pruned.contains_key("gone"));
        // Picked colors stay as they are.
        assert_eq!(space_text_color("#000000").as_deref(), Some("#414141"));
        assert_eq!(space_text_color("ansi6").as_deref(), Some("#59bbb0"));
        assert_eq!(space_text_color("nope"), None);
    }

    #[test]
    fn pi_session_names_come_from_the_title() {
        let name = |t: &str, cwd: Option<&str>| pi_session_name(t, cwd);
        assert_eq!(
            name("π - fix the build - repo", Some("/x/repo/")).as_deref(),
            Some("fix the build")
        );
        assert_eq!(name("π - repo", Some("/x/repo")), None, "unnamed");
        assert_eq!(name("π - a - b - c", None).as_deref(), Some("a - b"));
        assert_eq!(name("zsh", None), None);
        assert_eq!(name("π -  - repo", Some("/repo")), None);
    }

    #[test]
    fn pi_watch_labels_are_trusted_only_on_pi() {
        let mut t = Terminal::default();
        t.labels.insert(
            WATCH_LABEL.into(),
            r##"{"role":"orchestrator","orchestration":"o1","ticket":"FCC-1","prs":["#12","tool#7"],"sha":"abc","paused":true}"##.into(),
        );
        let pi = Program {
            key: "pi".into(),
            ..Default::default()
        };
        let w = Watch::of(&t, &pi).unwrap();
        assert_eq!(w.text().as_deref(), Some("#12, tool#7, abc · paused"));
        let chip = w.chip().unwrap();
        assert_eq!((chip.first.as_str(), chip.more), ("#12", 2));
        assert!(Watch::of(&t, &Program::default()).is_none(), "not Pi");
        t.labels.insert(WATCH_LABEL.into(), "[1]".into());
        assert!(Watch::of(&t, &pi).is_none(), "not an object");
    }
}
