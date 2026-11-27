//! The app's state and its one transition: [`Model::step`] takes an [`Event`] (core news, a
//! key, a reply, the clock) or an [`Action`] and returns the [`Effects`] to carry out.
//!
//! After every step the model settles: it re-reads the machines' shown layouts, reconciles the
//! view with them ([`View::reconcile`]), applies Eli's focus-after-close rule, feeds the jump
//! list, marks what the person is looking at as seen (and shares it as `herdr.seen`), asks for
//! the next document rounds, starts adoption and closed-pane lookups, and says when focus moved
//! or the view needs saving.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use herdr_proto::methods::{
    DocGetResult, DocSetResult, ExecResult, ReadResult, TerminalGet, TerminalKeep, TerminalKill,
    TerminalLabel, TerminalRead, TerminalRun,
};
use herdr_proto::types::{CoreInfo, MachineState, MachineView};
use herdr_proto::{Changes, CloseReason, RpcError, Terminal, TerminalSet};
use herdr_shared::attention::Seen;
use herdr_shared::{Alert, Raise};
use herdr_shared::{
    Attention, Axis, Content, DOCUMENT, EditError, Intent, Layout, Orphan, Report, SeenTracker,
    SeenWrite, Space, Tab, adoption, closed, labels, local, machine_of, qualify,
};

use crate::action::{Action, AppCommand, Direction};
use crate::control::{Call, CallId, answer_call};
use crate::effect::{Effect, Effects, Focus, Level, Request, TerminalCommand, Toast, Token};
use crate::folders::{self, NewSpace};
use crate::geometry::{self, Geometry, Rect};
use crate::keys::{Chord, Context, Key, Keymap, Lookup, Mods, Press};
use crate::mode::{self, Mode, PREFIX_TIMEOUT};
use crate::view::{Before, SeenRecord, View};
use crate::world::{SpaceExt, World, tab_name};

/// Eli's label on the review terminal (`feature-inventory.md` §13, [`crate::review`]): its tab
/// stays out of the tab bar and tab keys.
pub const REVIEW_LABEL: &str = "dotfiles.review";
/// Eli's Neovim bridge label (§5): Ctrl+H/J/K/L go through Neovim first where it is set. Its
/// value is the Neovim's RPC server (`v:servername`).
pub const EDITOR_LABEL: &str = "dotfiles.navigator.editor";
/// Neovim's request to move to the next pane, written at a window edge: `<n>:<n>:<direction>`,
/// unique on every press (`dot/home/.config/nvim/lua/herdr-app-navigator.lua`).
pub const NAVIGATOR_LABEL: &str = "dotfiles.navigator";
/// How long `nvim --remote-send` may take before the move counts as failed, in seconds
/// (`cfg/mods/navigator-status/navigation.js`).
pub const NAVIGATE_TIMEOUT: f64 = 2.0;
/// How long a machine whose layout call failed waits before reading again, in seconds.
pub const RETRY: f64 = 2.0;

/// One input to [`Model::step`].
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Event(Event),
    Action(Action),
    /// A `do` command from the control socket (`docs/cli.md`), answered with
    /// [`Effect::Answer`].
    Control(Call),
}

impl From<Event> for Input {
    fn from(event: Event) -> Self {
        Input::Event(event)
    }
}

impl From<Action> for Input {
    fn from(action: Action) -> Self {
        Input::Action(action)
    }
}

impl From<Call> for Input {
    fn from(call: Call) -> Self {
        Input::Control(call)
    }
}

/// What changed among the core's terminals, built from a supervisor batch
/// ([`TerminalUpdate::from_changes`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerminalUpdate {
    /// A fresh snapshot replaced everything: `terminals` is every terminal, and every layout is
    /// read again.
    pub resynced: bool,
    /// Terminals added or changed (all of them when `resynced`).
    pub terminals: Vec<Terminal>,
    pub removed: Vec<String>,
    /// `herdr-app` document versions announced, per machine.
    pub docs: Vec<(Option<String>, u64)>,
    /// The highest `herdr.seen` that went by per terminal ([`Changes::seen`]), so this screen
    /// knows which of its seen writes landed.
    pub seen: BTreeMap<String, u64>,
}

impl TerminalUpdate {
    /// The update for one batch of [`Changes`] applied to `set`.
    pub fn from_changes(changes: &Changes, set: &TerminalSet) -> TerminalUpdate {
        let terminals = if changes.resynced {
            set.iter().cloned().collect()
        } else {
            changes
                .updated
                .iter()
                .chain(&changes.restarted)
                .filter_map(|a| set.get(a).cloned())
                .collect()
        };
        TerminalUpdate {
            resynced: changes.resynced,
            terminals,
            removed: changes.removed.iter().cloned().collect(),
            docs: changes
                .docs
                .iter()
                .filter(|(key, _)| key.name == DOCUMENT)
                .map(|(key, version)| (key.machine.clone(), *version))
                .collect(),
            seen: changes.seen.clone(),
        }
    }
}

/// News for the model.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An [`Effect::WakeAt`] came due (timers are events). The time is [`Model::step`]'s.
    Tick,
    /// A key press. `terminal`: a terminal pane's grid has the keyboard.
    Key {
        press: Press,
        terminal: bool,
    },
    /// A new connection is up (its snapshot follows as a resynced [`Event::Terminals`]).
    Connected,
    /// The connection dropped.
    Lost,
    Terminals(TerminalUpdate),
    /// `machine.list`.
    Machines(Vec<MachineView>),
    /// Who the connected core is (`hello.core`): a record's `parent` may name this machine by it.
    Core(CoreInfo),
    DocRead {
        machine: Option<String>,
        result: Result<DocGetResult, RpcError>,
    },
    DocWritten {
        machine: Option<String>,
        result: Result<DocSetResult, RpcError>,
    },
    /// The reply to an [`Effect::Call`]. Run, get, kill and keep all answer with a terminal
    /// record (for `terminal.run`, the `RunResult`'s).
    Reply {
        token: Token,
        result: Result<Box<Terminal>, RpcError>,
    },
    /// The reply to an [`Effect::Call`] of [`Request::Exec`].
    Executed {
        token: Token,
        result: Result<ExecResult, RpcError>,
    },
    /// The reply to an [`Effect::Call`] of [`Request::Read`].
    Read {
        token: Token,
        result: Result<ReadResult, RpcError>,
    },
    /// A pane's attach stream ended.
    StreamClosed {
        address: String,
        reason: CloseReason,
    },
    /// The window became the foreground window, or stopped being it.
    WindowActive(bool),
    /// The tab area and the split gap, for pane directions.
    Geometry(Geometry),
    /// The Agents list's shown rows (for numbers and jump labels) and its navigation order (rows
    /// plus every worker).
    AgentOrder {
        rows: Vec<String>,
        order: Vec<String>,
    },
    /// Copy mode's yank in this terminal copied text: copy mode ends. A yank that copies
    /// nothing sends no event, and copy mode goes on.
    CopyYanked(String),
    /// How many rows the open list popup shows now.
    PopupRows(usize),
    /// Where the open list popup's cursor goes: where it starts when it opens or its query
    /// changes, or the row a click picked.
    PopupCursor(usize),
}

impl Event {
    /// A key press with one spelling.
    pub fn key(press: impl Into<Press>, terminal: bool) -> Event {
        Event::Key {
            press: press.into(),
            terminal,
        }
    }
}

/// A launch waiting for `terminal.run`: the ids are made before the call, the pane goes in the
/// layout after it (never an address before `terminal.run` returned it).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Launch {
    Space {
        space: String,
        name: String,
        folder: String,
        tab: String,
        tab_name: String,
        pane: String,
        /// Show it once placed (`space.new {focus: false}` leaves the shown space shown).
        focus: bool,
    },
    Tab {
        space: String,
        tab: String,
        /// Its name; empty: called by its number.
        name: String,
        pane: String,
        after: Option<String>,
    },
    Split {
        target: String,
        axis: Axis,
        pane: String,
        container: String,
    },
}

/// A call whose reply the model waits for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pending {
    Launch {
        machine: Option<String>,
        launch: Launch,
        /// The control call waiting for it.
        answer: Option<CallId>,
    },
    Adopt(Orphan),
    CheckClosed {
        pane: String,
        address: String,
    },
    /// The peek's "↗ keep" giving `address` a pane: its labels, then `keep`, then the tab
    /// `tab` with pane `pane` in `space` (`app:src/window.rs` `keep_terminal`).
    Keep(crate::peek::Keep),
    /// Ctrl+H/J/K/L handed to the Neovim in `address`, focused in `pane` of `tab` then.
    Navigate {
        address: String,
        pane: String,
        tab: Option<String>,
        direction: Direction,
    },
    /// One of the new-space picker's lookups, for its `opening`th opening.
    Folders {
        opening: u64,
        lookup: folders::Lookup,
    },
    /// "Send back to <parent>": its labels are cleared (`labelled` once that answered), then
    /// keep is turned off, then its pane goes (`cfg/AGENTS.md`: `terminal.park`).
    Park {
        pane: String,
        terminal: String,
        labelled: bool,
    },
    /// A `herdr.seen` write; one that fails is tried again after the core's next news.
    Seen(SeenWrite),
    /// A Cmd-clicked path being looked up on its pane's machine ([`crate::links`]).
    Link {
        pane: String,
        local: bool,
        lookup: crate::links::Lookup,
    },
    /// The last lines of a terminal this screen started that exited with `code` right away,
    /// for the "Terminal failed to start" notice.
    LaunchFailed {
        code: i32,
    },
}

/// herdr-frontend's state.
#[derive(Clone, Debug)]
pub struct Model {
    pub(crate) world: World,
    pub(crate) view: View,
    pub(crate) mode: Mode,
    pub(crate) keymap: Keymap,
    pub(crate) attention: Attention,
    /// What this screen shares as `herdr.seen`.
    pub(crate) seen: SeenTracker,
    /// Terminals whose seen write failed: tried again after the core's next news, not in the
    /// failure's own step, so a write that keeps failing can't loop.
    pub(crate) seen_failed: BTreeSet<String>,
    /// Saved attention not yet matched to a listed agent.
    pub(crate) seen_restore: BTreeMap<String, SeenRecord>,
    pub(crate) home: String,
    pub(crate) now: f64,
    pub(crate) connected: bool,
    /// A full listing arrived on this connection.
    pub(crate) listed: bool,
    pub(crate) window_active: bool,
    pub(crate) geometry: Geometry,
    pub(crate) published_rows: Option<Vec<String>>,
    pub(crate) published_order: Option<Vec<String>>,
    /// Focused panes, most recent last, for `pane.go`.
    pub(crate) pane_history: Vec<String>,
    /// The terminal the peek overlay shows (`feature-inventory.md` §15). It has the keyboard
    /// while no popup does; the focused pane stays focused behind it.
    pub(crate) peek: Option<String>,
    /// The terminal `cmd-r` came to the review from, to go back to (§13).
    pub(crate) review_back: Option<String>,
    /// This computer's login shell (`$SHELL`), which a tab's program runs in ([`crate::shell`]).
    pub(crate) shell: String,
    /// Each live terminal's last [`NAVIGATOR_LABEL`], so a request is acted on once.
    pub(crate) navigator: BTreeMap<String, Option<String>>,
    next_token: u64,
    pub(crate) pending: BTreeMap<Token, Pending>,
    /// Terminals with an adoption or closed-pane lookup under way.
    pub(crate) busy: BTreeSet<String>,
    /// Terminals a lookup said are still there, until they show up live again.
    pub(crate) not_closed: BTreeSet<String>,
    /// Terminals this screen started, and when.
    pub(crate) births: BTreeMap<String, f64>,
    /// Machines that may wait before their next layout round, until when.
    pub(crate) retry_at: BTreeMap<Option<String>, f64>,
    /// Something that may need adoption or closed-pane lookups changed.
    pub(crate) scan: bool,
    /// The new-space folder picker's folders, from its latest opening.
    pub(crate) new_space: Option<NewSpace>,
    /// How many times the new-space picker has opened.
    pub(crate) openings: u64,
    last_focus: Option<Focus>,
    last_saved: View,
}

impl Model {
    /// A model with no saved view. `home` is the home folder: a new space's folder when there's
    /// no other, and `~` in names.
    pub fn new(home: impl Into<String>) -> Model {
        Model::with_view(home, View::default())
    }

    /// A model restoring a saved view (read with [`View::from_json`]).
    pub fn with_view(home: impl Into<String>, mut view: View) -> Model {
        let seen_restore = std::mem::take(&mut view.seen);
        let mut last_saved = view.clone();
        last_saved.seen = seen_restore.clone();
        Model {
            world: World::new(),
            view,
            mode: Mode::Normal,
            keymap: Keymap::eli(),
            attention: Attention::default(),
            seen: SeenTracker::default(),
            seen_failed: BTreeSet::new(),
            seen_restore,
            home: home.into(),
            now: 0.0,
            connected: false,
            listed: false,
            window_active: true,
            geometry: Geometry::default(),
            published_rows: None,
            published_order: None,
            pane_history: Vec::new(),
            peek: None,
            review_back: None,
            shell: "/bin/sh".into(),
            navigator: BTreeMap::new(),
            next_token: 0,
            pending: BTreeMap::new(),
            busy: BTreeSet::new(),
            not_closed: BTreeSet::new(),
            births: BTreeMap::new(),
            retry_at: BTreeMap::new(),
            scan: false,
            new_space: None,
            openings: 0,
            last_focus: None,
            last_saved,
        }
    }

    /// This computer's login shell (`$SHELL`; `/bin/sh` until set).
    pub fn set_shell(&mut self, shell: impl Into<String>) {
        self.shell = shell.into();
    }

    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    pub fn attention(&self) -> &Attention {
        &self.attention
    }

    pub fn now(&self) -> f64 {
        self.now
    }

    /// Connected, with the core's full listing in: what's missing from the terminals is gone.
    pub fn listed(&self) -> bool {
        self.connected && self.listed
    }

    pub fn home(&self) -> &str {
        &self.home
    }

    pub fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    /// The new-space folder picker's machine and folders, from its latest opening (a pick is
    /// resolved after the picker closed).
    pub fn folder_picker(&self) -> Option<&NewSpace> {
        self.new_space.as_ref()
    }

    pub fn active_space(&self) -> Option<&Space> {
        self.view.active_space(&self.world)
    }

    pub fn shown_tab(&self) -> Option<&Tab> {
        self.view.shown_tab(&self.world)
    }

    /// The focused pane of the tab on screen.
    pub fn focused_pane(&self) -> Option<String> {
        self.view.focused(&self.world)
    }

    /// The terminal in the focused pane.
    pub fn focused_terminal(&self) -> Option<&str> {
        let pane = self.focused_pane()?;
        self.world.address_of(&pane)
    }

    /// The panes on screen and where they sit (only the zoomed one when zoomed).
    pub fn pane_rects(&self) -> Vec<(String, Rect)> {
        self.shown_tab()
            .map(|tab| geometry::blocks(self.view.shown_root(tab), &self.geometry))
            .unwrap_or_default()
    }

    /// The terminal the keyboard types into: the peeked one while the peek is open, else the
    /// focused pane's. Terminal commands (copy, paste, `send:`) and copy mode go to it.
    pub fn keyboard_terminal(&self) -> Option<&str> {
        self.peek.as_deref().or_else(|| self.focused_terminal())
    }

    /// Terminals drawn on screen now: the shown tab's, and the peeked one.
    pub fn visible_terminals(&self) -> Vec<String> {
        let mut shown = self
            .shown_tab()
            .map(|tab| self.view.shown_root(tab).addresses())
            .unwrap_or_default();
        if let Some(peek) = &self.peek
            && !shown.contains(peek)
        {
            shown.push(peek.clone());
        }
        shown
    }

    /// A tab the tab bar and tab keys leave out: one with an agent's terminal, or the review's
    /// (`cfg/mods/eli-tabs/main.js:55-60`, `navigator-status/tab-order.js`).
    pub fn is_hidden_tab(&self, tab: &Tab) -> bool {
        tab.root.addresses().iter().any(|a| {
            self.world.is_agent(a) || self.world.terminal(a).is_some_and(crate::review::is_review)
        })
    }

    /// The tabs the bar shows in a space, in order (`visibleTabs`).
    pub fn shown_tabs<'a>(&'a self, space: &'a Space) -> Vec<&'a Tab> {
        space
            .tabs
            .iter()
            .filter(|t| !self.is_hidden_tab(t))
            .collect()
    }

    /// The agents in navigation order: the Agents list's published order, then agents it leaves
    /// out in layout order. Before the list publishes one, layout order.
    pub fn agent_order(&self) -> Vec<String> {
        let placed: Vec<String> = self
            .world
            .tabs()
            .flat_map(|t| t.root.addresses())
            .filter(|a| self.world.is_agent(a))
            .collect();
        let Some(order) = &self.published_order else {
            return placed;
        };
        let mut out: Vec<String> = order
            .iter()
            .filter(|a| self.world.is_agent(a))
            .cloned()
            .collect();
        for a in placed {
            if !out.contains(&a) {
                out.push(a);
            }
        }
        out
    }

    /// The top of `address`'s chain of live parents: the agent a sub-agent belongs to, where
    /// `agent.next` and `agent.prev` step from when it has focus (Herdr app's `root_agent`).
    pub(crate) fn root_agent(&self, address: &str) -> String {
        let mut at = address;
        // A cycle of parents ends.
        for _ in 0..32 {
            let parent = self
                .world
                .terminal(at)
                .and_then(|t| crate::agents::parent_of(&self.world, t))
                .filter(|p| self.world.terminal(p).is_some_and(|t| t.is_live()));
            match parent {
                Some(p) => at = p,
                None => break,
            }
        }
        at.to_owned()
    }

    /// The Agents list's rows, for row numbers and jump labels: as published, minus agents gone
    /// since; before the list publishes, [`Model::agent_order`].
    pub fn agent_rows(&self) -> Vec<String> {
        match &self.published_rows {
            Some(rows) => rows
                .iter()
                .filter(|a| self.world.is_agent(a))
                .cloned()
                .collect(),
            None => self.agent_order(),
        }
    }

    /// The view to save: this screen's choices plus every agent's attention.
    pub fn saved_view(&self) -> View {
        let mut view = self.view.clone();
        view.jumps = Default::default();
        view.seen = self.seen_restore.clone();
        for t in self.world.live() {
            if let Some(agent) = self.attention.get(&t.address)
                && agent.word.is_some()
            {
                view.seen.insert(
                    t.address.clone(),
                    SeenRecord {
                        word: agent.word.clone(),
                        seen: agent.seen,
                    },
                );
            }
        }
        view
    }

    /// The one transition. `now` is the app clock in seconds (Rune's `Tx::clock`), read when
    /// the input arrived: every deadline (the prefix timeout, the jump list's dwell, retries)
    /// is measured against it, so it comes with every input. It never runs backwards.
    pub fn step(&mut self, now: f64, input: impl Into<Input>) -> Effects {
        self.now = self.now.max(now);
        // A prefix that timed out is gone before the next key is read.
        if let Mode::Prefix { due, .. } = self.mode
            && self.now >= due
        {
            self.mode = Mode::Normal;
        }
        let before = self.before();
        let mut fx = Effects {
            list: Vec::new(),
            handled: true,
        };
        match input.into() {
            Input::Event(event) => self.event(event, &mut fx),
            Input::Action(action) => self.run(action, &mut fx),
            Input::Control(call) => self.control(call, &mut fx),
        }
        self.settle(&before, &mut fx);
        fx
    }

    pub(crate) fn token(&mut self) -> Token {
        self.next_token += 1;
        Token(self.next_token)
    }

    pub(crate) fn call(&mut self, fx: &mut Effects, request: Request, pending: Option<Pending>) {
        let token = self.token();
        if let Some(pending) = pending {
            self.pending.insert(token, pending);
        }
        fx.push(Effect::Call { token, request });
    }

    fn before(&self) -> Before {
        let mut before = Before::capture(&self.view, &self.world);
        before.agents = self
            .agent_order()
            .into_iter()
            .filter_map(|a| {
                let pane = self.world.pane_of(&a)?;
                let tab = self.world.tab_of(&pane)?.id.clone();
                Some((a, tab))
            })
            .collect();
        before
    }

    /// Takes an edit to a machine's layout; it shows at once. False when it didn't apply.
    pub(crate) fn edit(&mut self, intent: Intent, fx: &mut Effects) -> bool {
        let machine = intent_machine(&intent).map(str::to_owned);
        let Some(m) = self.world.machine_mut(machine.as_deref()) else {
            return false;
        };
        match m.mirror.edit(intent) {
            Ok(()) => {
                m.refresh();
                true
            }
            Err(EditError::NoEffect) => false,
            Err(EditError::Unavailable) => {
                fx.push(Effect::Toast(unavailable(m.mirror.layout())));
                false
            }
        }
    }

    /// Whether a machine's layout can take an edit now; when not, says so. A terminal started
    /// for a layout that can't take its pane would be left kept and pane-less, so a launch asks
    /// first (`app:src/window.rs` `require_document`).
    pub(crate) fn layout_ready(&self, machine: Option<&str>, fx: &mut Effects) -> bool {
        if self.world.machine(machine).is_none() {
            return false;
        }
        match self.refused_on(machine) {
            None => true,
            Some(toast) => {
                fx.push(Effect::Toast(toast));
                false
            }
        }
    }

    /// The toast for a machine's layout that can't take an edit now; `None` when it can, or
    /// when there is no such machine.
    fn refused_on(&self, machine: Option<&str>) -> Option<Toast> {
        let m = self.world.machine(machine)?;
        (*m.mirror.layout() != Layout::Ready)
            .then(|| self.login_or(machine, unavailable(m.mirror.layout())))
    }

    /// `toast` about something that failed on `machine`, unless that machine needs a login: then
    /// the login card that says what to run, with Copy (Herdr app's failed New space toast, as
    /// `eli-toast` draws it).
    pub(crate) fn login_or(&self, machine: Option<&str>, toast: Toast) -> Toast {
        let needs_login = self
            .world
            .machine(machine)
            .and_then(|m| m.status.as_ref())
            .is_some_and(|v| v.state == MachineState::NeedsLogin);
        match machine {
            Some(name) if needs_login => Toast::login(name),
            _ => toast,
        }
    }

    /// Why the layout holding `id` (a node, tab or space) can't take an edit now, as the toast
    /// that says so; `None` when it can, or when no machine holds it. A gesture that edits as it
    /// goes (a divider drag) asks once, on press, as Herdr app's `require_document` does, instead
    /// of a toast for every move.
    pub fn edit_refused(&self, id: &str) -> Option<Toast> {
        self.refused_on(machine_of(id))
    }

    /// Ends terminals (`close_targets`: on cores 0.1.13 and later, which this client requires,
    /// only these; the core ends their `die_with_parent` children).
    pub(crate) fn kill(&mut self, addresses: Vec<String>, fx: &mut Effects) {
        let world = &self.world;
        let targets = closed::close_targets(
            addresses,
            world.live(),
            |a| world.pane_of(a).is_some(),
            |_| false,
        );
        for terminal in targets {
            self.call(
                fx,
                Request::Kill(TerminalKill {
                    terminal,
                    ..Default::default()
                }),
                None,
            );
        }
    }

    /// Starts a terminal for a pane on `machine`, in `folder`, labeled for its space and tab,
    /// running `argv` (empty: the core's default shell). `answer`: the control call to answer
    /// once the pane is placed (or the launch failed).
    pub(crate) fn launch(
        &mut self,
        machine: Option<String>,
        folder: &str,
        launch: Launch,
        argv: Vec<String>,
        answer: Option<CallId>,
        fx: &mut Effects,
    ) {
        if !self.layout_ready(machine.as_deref(), fx) {
            if let Some(id) = answer {
                answer_call(fx, id, Err("the layout can't take a new pane now".into()));
            }
            return;
        }
        let (space, tab) = match &launch {
            Launch::Space { space, tab, .. } | Launch::Tab { space, tab, .. } => {
                (space.clone(), tab.clone())
            }
            Launch::Split { target, .. } => {
                let tab = self.world.tab_of(target).map(|t| t.id.clone());
                let space = self.world.space_of(target).map(|s| s.id.clone());
                let (Some(space), Some(tab)) = (space, tab) else {
                    return;
                };
                (space, tab)
            }
        };
        let m = machine.as_deref();
        let env = [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("TERM_PROGRAM", "herdr"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let run = TerminalRun {
            argv,
            cwd: (!folder.is_empty()).then(|| folder.to_owned()),
            env,
            labels: labels::for_new_pane(local(&space, m), local(&tab, m)),
            keep: true,
            on: machine.clone(),
            ..Default::default()
        };
        self.call(
            fx,
            Request::Run(Box::new(run)),
            Some(Pending::Launch {
                machine,
                launch,
                answer,
            }),
        );
    }

    /// The connection dropped or came back: launches under way may never be answered, so their
    /// control calls get an error now.
    fn drop_answers(&mut self, fx: &mut Effects) {
        // The folder picker stops waiting for lookups that may never answer.
        if let Some(ns) = &mut self.new_space {
            ns.waiting.clear();
        }
        for pending in self.pending.values_mut() {
            if let Pending::Launch { answer, .. } = pending
                && let Some(id) = answer.take()
            {
                answer_call(fx, id, Err("the connection to herdr-core dropped".into()));
            }
        }
    }

    fn event(&mut self, event: Event, fx: &mut Effects) {
        match event {
            // The clock already moved, and an overdue prefix was dropped.
            Event::Tick => {}
            Event::Key { press, terminal } => self.key(press, terminal, fx),
            Event::Connected => {
                self.connected = true;
                self.listed = false;
                self.retry_at.clear();
                self.drop_answers(fx);
                self.pending.clear();
                self.busy.clear();
                // Seen writes under way may never be answered: the new listing decides, and
                // what is on screen is looked at again.
                self.seen = SeenTracker::default();
                self.seen_failed.clear();
                for (_, m) in self.world.machines_mut() {
                    m.mirror.reconnected();
                    m.refresh();
                }
            }
            Event::Lost => {
                self.connected = false;
                self.listed = false;
                self.drop_answers(fx);
            }
            Event::Terminals(update) => self.terminals(update, fx),
            Event::Machines(list) => {
                self.world.set_machines(&list);
                self.scan = true;
            }
            Event::Core(core) => self.world.set_core(core),
            Event::DocRead { machine, result } => {
                let Some(m) = self.world.machine_mut(machine.as_deref()) else {
                    return;
                };
                let report = m.mirror.finish_read(result);
                m.refresh();
                self.reported(machine, report, fx);
            }
            Event::DocWritten { machine, result } => {
                let Some(m) = self.world.machine_mut(machine.as_deref()) else {
                    return;
                };
                let report = m.mirror.finish_write(result);
                m.refresh();
                self.reported(machine, report, fx);
            }
            Event::Reply { token, result } => self.reply(token, result, fx),
            Event::Executed { token, result } => self.executed(token, result, fx),
            Event::Read { token, result } => {
                if let Some(Pending::LaunchFailed { code }) = self.pending.remove(&token) {
                    let output = result.map(|r| r.text).unwrap_or_default();
                    fx.push(Effect::Toast(Toast::new(
                        Level::Error,
                        "Terminal failed to start",
                        closed::launch_failure_body(code, &output),
                    )));
                }
            }
            Event::StreamClosed { address, reason } => {
                if closed::stream_closed(&reason) {
                    let created = self.world.terminal(&address).map(|t| t.created_ms);
                    self.launch_failed(&address, &reason, created, fx);
                    if let Some(pane) = self.world.pane_of(&address) {
                        self.edit(Intent::RemovePane { pane }, fx);
                    }
                }
            }
            Event::WindowActive(active) => self.window_active = active,
            Event::Geometry(geometry) => self.geometry = geometry,
            Event::AgentOrder { rows, order } => {
                self.published_rows = Some(rows);
                self.published_order = Some(order);
            }
            Event::CopyYanked(address) => {
                if matches!(&self.mode, Mode::Copy(copy) if copy.address == address) {
                    self.close_mode(fx);
                }
            }
            Event::PopupRows(rows) => {
                if let Mode::Popup(mode::PopupState::List(list)) = &mut self.mode {
                    list.set_rows(rows);
                }
            }
            Event::PopupCursor(at) => match &mut self.mode {
                Mode::Popup(mode::PopupState::List(list)) => {
                    list.cursor = at.min(list.rows.saturating_sub(1));
                }
                Mode::Popup(mode::PopupState::Menu(menu)) => {
                    menu.cursor = at.min(menu.items.len().saturating_sub(1));
                }
                _ => {}
            },
        }
    }

    /// A finished layout round: tell the person what was given up, and pace a machine whose
    /// calls fail.
    fn reported(&mut self, machine: Option<String>, report: Report, fx: &mut Effects) {
        self.scan = true;
        if let Some(notice) = report.notice {
            fx.push(Effect::Toast(Toast::new(
                Level::Warning,
                "Layout unavailable",
                notice,
            )));
        }
        if !report.dropped.is_empty() {
            fx.push(Effect::Toast(Toast::new(
                Level::Warning,
                "Layout changed elsewhere",
                format!("{} edit(s) no longer applied", report.dropped.len()),
            )));
        }
        let failing = self
            .world
            .machine(machine.as_deref())
            .is_some_and(|m| matches!(m.mirror.layout(), Layout::Unavailable(_)));
        if failing {
            let due = self.now + RETRY;
            self.retry_at.insert(machine, due);
            fx.push(Effect::WakeAt(due));
        } else {
            self.retry_at.remove(&machine);
        }
    }

    fn terminals(&mut self, update: TerminalUpdate, fx: &mut Effects) {
        if update.resynced {
            self.world.replace_terminals(update.terminals);
            self.not_closed.clear();
            for (_, m) in self.world.machines_mut() {
                m.mirror.note_missed();
            }
            self.listed = true;
        } else {
            self.world
                .update_terminals(update.terminals, &update.removed);
        }
        for (machine, version) in update.docs {
            if let Some(m) = self.world.machine_mut(machine.as_deref()) {
                m.mirror.note_doc(version);
            }
        }
        let live: BTreeSet<&str> = self.world.live().map(|t| t.address.as_str()).collect();
        self.not_closed.retain(|a| !live.contains(a.as_str()));
        self.seen.echoed(&update.seen, update.resynced);
        self.seen.retain(|a| live.contains(a));
        self.seen_failed.clear();
        self.scan = true;
        self.navigator_requests(update.resynced);

        // Attention: statuses and turns, what is on screen, and the alerts they raise.
        let visible: BTreeSet<String> = if self.window_active {
            self.visible_terminals().into_iter().collect()
        } else {
            BTreeSet::new()
        };
        let (world, tracker) = (&self.world, &self.seen);
        let seen = world.live().map(|t| Seen {
            terminal: t,
            in_pane: world.pane_of(&t.address).is_some(),
            looking: visible.contains(&t.address),
            seen_turn: tracker.seen_turn(t),
        });
        let raises = self.attention.update(seen);
        // Saved attention applies once its agent is listed with the same word.
        let restore = std::mem::take(&mut self.seen_restore);
        for (address, record) in restore {
            if let Some(agent) = self.attention.get_mut(&address) {
                if agent.word == record.word {
                    agent.seen = record.seen;
                }
            } else if !self.reachable_for(&address) || !self.listed {
                self.seen_restore.insert(address, record);
            }
        }
        for raise in raises {
            // An alert brings a settled tab back, toast or not.
            if let Some(tab) = self
                .world
                .pane_of(&raise.terminal)
                .and_then(|p| self.world.tab_of(&p))
                .map(|t| t.id.clone())
            {
                self.view.unsettle(&tab);
            }
            fx.push(Effect::Alert(raise));
        }
    }

    /// The toast an alert shows (`app:src/window.rs` `alert`), or `None` when the person is
    /// looking at that agent. Titled "<name> needs you" (an error) or "<name> finished", with
    /// "<space> / <tab>" below, and a click goes to the agent. The name is the terminal's own,
    /// else `program_name` of its status's `app`, else "An agent".
    pub fn alert_toast(
        &self,
        raise: &Raise,
        program_name: impl Fn(&str) -> String,
    ) -> Option<Toast> {
        if !raise.toast {
            return None;
        }
        let record = self.world.terminal(&raise.terminal);
        let name = record
            .and_then(|t| t.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| {
                record
                    .and_then(|t| t.status.as_ref())
                    .map(|s| s.app.as_str())
                    .filter(|a| !a.is_empty())
                    .map(program_name)
            })
            .unwrap_or_else(|| "An agent".to_owned());
        // Only an agent in a pane alerts (never a sub-agent), so its pane says where it is.
        let world = &self.world;
        let place = world.pane_of(&raise.terminal).and_then(|pane| {
            let (space, tab) = (world.space_of(&pane)?, world.tab_of(&pane)?);
            Some(format!(
                "{} / {}",
                world.space_name(space, &self.home),
                tab_name(space, tab)
            ))
        });
        let (title, level) = match raise.alert {
            Alert::Blocked => (format!("{name} needs you"), Level::Error),
            Alert::Done => (format!("{name} finished"), Level::Info),
        };
        Some(
            Toast::new(level, title, place.unwrap_or_default())
                .on_click(Action::FocusAgent(raise.terminal.clone())),
        )
    }

    /// Whether the machine an id belongs to is reachable now.
    fn reachable_for(&self, id: &str) -> bool {
        self.world
            .machine(machine_of(id))
            .is_some_and(|m| m.reachable())
    }

    fn reply(&mut self, token: Token, result: Result<Box<Terminal>, RpcError>, fx: &mut Effects) {
        let Some(pending) = self.pending.remove(&token) else {
            return;
        };
        match pending {
            Pending::Launch {
                machine,
                launch,
                answer,
            } => match result {
                Ok(record) => self.launched(machine, launch, *record, answer, fx),
                Err(error) => {
                    if let Some(id) = answer {
                        answer_call(fx, id, Err(error.message.clone()));
                    }
                    let toast = self.login_or(
                        machine.as_deref(),
                        Toast::new(Level::Error, "Couldn't start a terminal", error.message),
                    );
                    fx.push(Effect::Toast(toast));
                }
            },
            Pending::Adopt(orphan) => {
                self.busy.remove(&orphan.terminal);
                let Ok(record) = result else { return };
                if !adoption::adoptable(&record, &orphan)
                    || self.world.pane_of(&orphan.terminal).is_some()
                {
                    return;
                }
                if adoption::needs_keep(&record) {
                    self.call(
                        fx,
                        Request::Keep(TerminalKeep {
                            terminal: orphan.terminal.clone(),
                            keep: true,
                            ..Default::default()
                        }),
                        None,
                    );
                }
                let folder = adoption::folder(Some(&record), None, &self.home);
                let after = self.view.spaces.get(&orphan.space).cloned();
                self.edit(orphan.intent(folder, after), fx);
            }
            Pending::Park {
                pane,
                terminal,
                labelled,
            } => match result {
                Ok(_) if !labelled => self.call(
                    fx,
                    Request::Keep(TerminalKeep {
                        terminal: terminal.clone(),
                        keep: false,
                        ..Default::default()
                    }),
                    Some(Pending::Park {
                        pane,
                        terminal,
                        labelled: true,
                    }),
                ),
                Ok(_) => {
                    self.edit(Intent::RemovePane { pane }, fx);
                }
                Err(error) => fx.push(Effect::Toast(Toast::new(
                    Level::Error,
                    "Couldn't send it back",
                    error.message,
                ))),
            },
            Pending::CheckClosed { pane, address } => {
                self.busy.remove(&address);
                if closed::closed_by_lookup(result.as_deref()) {
                    if let Ok(record) = &result {
                        let reason = CloseReason {
                            reason: record.reason.clone(),
                            exit_code: record.exit_code,
                            finished_ms: record.finished_ms,
                            ..Default::default()
                        };
                        self.launch_failed(&address, &reason, Some(record.created_ms), fx);
                    }
                    self.edit(Intent::RemovePane { pane }, fx);
                } else {
                    self.not_closed.insert(address);
                }
            }
            Pending::Seen(write) => {
                if result.is_err() {
                    self.seen.failed(&write);
                    self.seen_failed.insert(write.terminal.clone());
                    if let Some(t) = self.world.terminal(&write.terminal) {
                        let seen_turn = self.seen.seen_turn(t);
                        if let Some(agent) = self.attention.get_mut(&write.terminal) {
                            agent.set_turn(t.turn, seen_turn);
                        }
                    }
                }
            }
            Pending::Keep(keep) => self.kept(keep, result, fx),
            // Exec calls answer with `Event::Executed`.
            Pending::Navigate { .. }
            | Pending::Folders { .. }
            | Pending::Link { .. }
            | Pending::LaunchFailed { .. } => {}
        }
    }

    /// A `machine.exec` came back: the Neovim navigation, the new-space folder lookup or the
    /// link lookup it was for.
    fn executed(&mut self, token: Token, result: Result<ExecResult, RpcError>, fx: &mut Effects) {
        let Some(pending) = self.pending.remove(&token) else {
            return;
        };
        match pending {
            Pending::Navigate {
                address,
                pane,
                tab,
                direction,
            } => self.navigated(address, pane, tab, direction, result, fx),
            Pending::Folders { opening, lookup } => {
                // What it printed when it exited 0, else nothing (`new-space/main.js` `output`).
                let output = match &result {
                    Ok(r) if r.succeeded() => r.stdout_text(),
                    _ => String::new(),
                };
                if let Some(ns) = self.new_space.as_mut().filter(|ns| ns.opening == opening) {
                    ns.answered(lookup, output);
                }
            }
            Pending::Link {
                pane,
                local,
                lookup,
            } => self.link_found(&pane, local, &lookup, result, fx),
            Pending::Launch { .. }
            | Pending::Adopt(_)
            | Pending::CheckClosed { .. }
            | Pending::Park { .. }
            | Pending::Seen(_)
            | Pending::Keep(_)
            | Pending::LaunchFailed { .. } => {}
        }
    }

    /// A terminal ended (`reason`, started at `created`): when this screen started it and it
    /// exited failing within two seconds, its last lines are read for a "Terminal failed to
    /// start" notice (`app:src/window.rs` `notify_launch_failure`). Told once per terminal.
    fn launch_failed(
        &mut self,
        address: &str,
        reason: &CloseReason,
        created: Option<u64>,
        fx: &mut Effects,
    ) {
        let Some(born) = self.births.remove(address) else {
            return;
        };
        // From the core's times, else (no record left) from when this screen started it.
        let ran = closed::ran_for(reason, created)
            .or_else(|| Duration::try_from_secs_f64(self.now - born).ok());
        let Some(code) = closed::launch_failure(reason, ran) else {
            return;
        };
        let read = TerminalRead {
            terminal: address.to_owned(),
            lines: 5,
            ..Default::default()
        };
        self.call(
            fx,
            Request::Read(read),
            Some(Pending::LaunchFailed { code }),
        );
    }

    /// Shares what the person is looking at as `herdr.seen` (`app:src/window.rs` `share_seen`):
    /// the agents on screen while the window is in the foreground, once per turn while they stay
    /// there, and only to raise the label. Each shows as seen at once, not after the core's echo.
    /// One whose write just failed waits for the core's next news.
    fn share_seen(&mut self, fx: &mut Effects) {
        let looking: Vec<String> = if self.window_active {
            self.visible_terminals()
        } else {
            Vec::new()
        };
        let (world, failed) = (&self.world, &self.seen_failed);
        let writes = self.seen.look(
            looking
                .iter()
                .filter(|a| !failed.contains(*a))
                .filter_map(|a| world.terminal(a)),
        );
        for write in writes {
            if let Some(agent) = self.attention.get_mut(&write.terminal) {
                agent.set_turn(write.turn, Some(write.turn));
            }
            self.call(
                fx,
                Request::Label(TerminalLabel {
                    terminal: write.terminal.clone(),
                    labels: labels::for_seen(write.turn),
                    ..Default::default()
                }),
                Some(Pending::Seen(write)),
            );
        }
    }

    /// `nvim --remote-send` came back (`cfg/mods/navigator-status/navigation.js` `navigate`):
    /// Neovim took the key, or it couldn't run (an error toast), or the labeled Neovim is gone,
    /// so its label is cleared and the pane moves, if focus hasn't moved meanwhile.
    fn navigated(
        &mut self,
        address: String,
        pane: String,
        tab: Option<String>,
        direction: Direction,
        result: Result<ExecResult, RpcError>,
        fx: &mut Effects,
    ) {
        let failed = match result {
            Err(error) => Some(error.message),
            Ok(r) if r.timed_out => Some(format!(
                "nvim took longer than {NAVIGATE_TIMEOUT} s and was stopped"
            )),
            Ok(r) if r.code == Some(0) => return,
            Ok(_) => None,
        };
        if let Some(why) = failed {
            fx.push(Effect::Toast(Toast::new(
                Level::Error,
                "Pane navigation failed",
                format!("Could not run nvim: {why}"),
            )));
            return;
        }
        // Its Neovim died without releasing the terminal: forget it and move.
        self.call(
            fx,
            Request::Label(TerminalLabel {
                terminal: address.clone(),
                labels: [(EDITOR_LABEL.to_owned(), String::new())].into(),
                ..Default::default()
            }),
            None,
        );
        let same = self.focused_pane().as_deref() == Some(pane.as_str())
            && self.focused_terminal() == Some(address.as_str())
            && self.shown_tab().map(|t| &t.id) == tab.as_ref();
        if same {
            self.go(direction);
        }
    }

    /// Neovim asks for a pane move by writing a fresh [`NAVIGATOR_LABEL`] at a window edge. A
    /// request is acted on once, only while its terminal is focused; every one is noted as seen
    /// on arrival, so coming back to a terminal (or connecting again) never replays an old one
    /// (`cfg/mods/navigator-status/navigation.js` `onTerminalsChanged`).
    fn navigator_requests(&mut self, resynced: bool) {
        let focused = self.focused_terminal().map(str::to_owned);
        let world = &self.world;
        self.navigator.retain(|a, _| world.is_live(a));
        let mut go = None;
        for t in world.live() {
            let token = t
                .label(NAVIGATOR_LABEL)
                .filter(|v| !v.is_empty())
                .map(str::to_owned);
            let Some(previous) = self.navigator.insert(t.address.clone(), token.clone()) else {
                continue;
            };
            if resynced || token.is_none() || token == previous {
                continue;
            }
            if focused.as_deref() != Some(t.address.as_str()) {
                continue;
            }
            go = token.as_deref().and_then(navigator_direction).or(go);
        }
        if let Some(direction) = go {
            self.go(direction);
        }
    }

    fn launched(
        &mut self,
        machine: Option<String>,
        launch: Launch,
        record: Terminal,
        answer: Option<CallId>,
        fx: &mut Effects,
    ) {
        let address = match machine.as_deref() {
            Some(m) if machine_of(&record.address) != Some(m) => qualify(&record.address, Some(m)),
            _ => record.address.clone(),
        };
        self.births.insert(address.clone(), self.now);
        let content = Content::terminal(address.clone());
        match launch {
            Launch::Space {
                space,
                name,
                folder,
                tab,
                tab_name,
                pane,
                focus,
            } => {
                let intent = Intent::NewSpace {
                    space: space.clone(),
                    name: name.clone(),
                    folder,
                    tab: tab.clone(),
                    tab_name,
                    pane: pane.clone(),
                    content,
                };
                let placed = self.edit(intent, fx);
                if placed {
                    // Open again, it is no longer a recently closed space (`app:src/model.rs`
                    // `insert_space`).
                    self.view.recent.retain(|r| r.name != name);
                    // Unfocused, it waits behind the shown space (or shows, with none shown).
                    if focus {
                        self.view.select_tab(&self.world, &tab);
                    }
                    self.view.focus(&tab, &pane);
                }
                self.launched_space(&space, &tab, &address, placed, answer, fx);
            }
            Launch::Tab {
                space,
                tab,
                name,
                pane,
                after,
            } => {
                let intent = Intent::NewTab {
                    space,
                    tab: tab.clone(),
                    name,
                    pane: pane.clone(),
                    content,
                    after,
                };
                if self.edit(intent, fx) {
                    self.view.select_tab(&self.world, &tab);
                    self.view.focus(&tab, &pane);
                }
            }
            Launch::Split {
                target,
                axis,
                pane,
                container,
            } => {
                let intent = Intent::Split {
                    target,
                    axis,
                    pane: pane.clone(),
                    content,
                    container,
                };
                if self.edit(intent, fx)
                    && let Some(tab) = self.world.tab_of(&pane).map(|t| t.id.clone())
                {
                    self.view.unzoom(&tab);
                    self.view.focus(&tab, &pane);
                }
            }
        }
    }

    fn key(&mut self, press: Press, terminal: bool, fx: &mut Effects) {
        let reset = Chord::new(
            Key::Escape,
            Mods {
                cmd: false,
                ctrl: true,
                alt: true,
                shift: true,
            },
        );
        if press.matches(&reset) {
            self.close_mode(fx);
            self.run(Action::App(AppCommand::Reset), fx);
            return;
        }
        match std::mem::take(&mut self.mode) {
            Mode::Normal => match self.keymap.lookup_in(Context::normal(terminal), &[press]) {
                Lookup::Action(action) => {
                    let action = action.clone();
                    self.run_key(action, fx);
                }
                Lookup::Pending => self.wait_for_more(vec![press], fx),
                Lookup::Program | Lookup::None => fx.handled = false,
            },
            Mode::Prefix { mut keys, .. } => {
                keys.push(press);
                match self.keymap.lookup_in(Context::normal(terminal), &keys) {
                    Lookup::Action(action) => {
                        let action = action.clone();
                        self.run_key(action, fx);
                    }
                    Lookup::Pending => self.wait_for_more(keys, fx),
                    // An unbound sequence is dropped whole; none of it reaches the terminal (a
                    // key file can't send a sequence to it).
                    Lookup::Program | Lookup::None => {}
                }
            }
            Mode::Popup(state) => {
                let jump = matches!(state, mode::PopupState::Jump(_));
                // A list or a prompt passes other keys to its text field; the jump bar and
                // confirm dialogs hold every key.
                let typing = matches!(
                    state,
                    mode::PopupState::List(_) | mode::PopupState::Prompt(_)
                );
                // The key file's popup keys first, then the popup's own, then the window's.
                let action = self
                    .bound(Context::Popup, press)
                    .or_else(|| press.first(|chord| mode::popup_key(&state, chord)));
                self.mode = Mode::Popup(state);
                match action.or_else(|| (!jump).then(|| self.global_key(press)).flatten()) {
                    Some(action) => self.run(action, fx),
                    None => fx.handled = !typing,
                }
            }
            Mode::Copy(state) => {
                self.mode = Mode::Copy(state);
                // The key file's copy keys, then Cmd+C, then the Vim grammar, then the
                // window's one-key bindings. Copy mode holds the keyboard: no key reaches the
                // terminal.
                if let Some(action) = self
                    .bound(Context::Copy, press)
                    .or_else(|| press.first(mode::copy_key))
                {
                    self.run(action, fx);
                    return;
                }
                if let Some(key) = mode::vim_key(press)
                    && self.copy_key(key, fx)
                {
                    return;
                }
                if let Some(action) = self.global_key(press) {
                    self.run(action, fx);
                }
            }
        }
    }

    /// Runs a key's action. With the peek open, closing a tab or pane closes the peek instead,
    /// not the terminal behind it (`app:src/window.rs` `run_key`).
    fn run_key(&mut self, action: Action, fx: &mut Effects) {
        let action = match action {
            Action::TabClose | Action::PaneClose if self.peek.is_some() => Action::ClosePeek,
            action => action,
        };
        self.run(action, fx);
    }

    /// The action a single key runs in one context of the key file.
    fn bound(&self, context: Context, press: Press) -> Option<Action> {
        match self.keymap.lookup_in(&[context], &[press]) {
            Lookup::Action(action) => Some(action.clone()),
            _ => None,
        }
    }

    /// A one-key global binding, for a key an open overlay doesn't use: Herdr app runs the
    /// window's keys over a dialog too. Closing a tab or pane closes the overlay (or leaves copy
    /// mode) instead, not the terminal behind it (`app:src/window.rs` `run_key`).
    fn global_key(&self, press: Press) -> Option<Action> {
        match self.keymap.lookup_in(Context::normal(false), &[press]) {
            Lookup::Action(Action::TabClose | Action::PaneClose) => Some(match self.mode {
                Mode::Copy(_) => Action::Copy(crate::action::CopyAction::Quit),
                _ => Action::Popup(crate::action::PopupAction::Close),
            }),
            Lookup::Action(action) => Some(action.clone()),
            Lookup::Program | Lookup::Pending | Lookup::None => None,
        }
    }

    fn wait_for_more(&mut self, keys: Vec<Press>, fx: &mut Effects) {
        let due = self.now + PREFIX_TIMEOUT;
        self.mode = Mode::Prefix { keys, due };
        fx.push(Effect::WakeAt(due));
    }

    /// Leaves whatever mode holds the keyboard.
    pub(crate) fn close_mode(&mut self, fx: &mut Effects) {
        if let Mode::Copy(copy) = std::mem::take(&mut self.mode) {
            fx.push(Effect::Terminal {
                address: copy.address,
                command: TerminalCommand::CopyEnd,
            });
        }
    }

    fn settle(&mut self, before: &Before, fx: &mut Effects) {
        self.view.reconcile(&self.world, before);
        self.close_focus(before);
        // The peek closes when its terminal ends.
        if self.peek.as_ref().is_some_and(|a| !self.world.is_live(a)) {
            self.peek = None;
        }
        // Copy mode ends when its terminal goes or loses the keyboard.
        if let Mode::Copy(copy) = &self.mode
            && (!self.world.is_live(&copy.address)
                || self.keyboard_terminal() != Some(copy.address.as_str()))
        {
            self.close_mode(fx);
        }
        self.pin();
        if let Some(pane) = self.focused_pane() {
            geometry::remember(&mut self.pane_history, &pane);
        }
        let world = &self.world;
        self.pane_history
            .retain(|p| world.node(p).is_some() || !world.loaded_for(p));
        let shown = self.shown_tab().map(|t| t.id.clone());
        self.view.jumps.shown(shown.as_deref(), self.now);
        if self.window_active {
            let visible = self.visible_terminals();
            self.attention.mark_seen(visible.iter().map(String::as_str));
        }
        let focus = if self.mode.holds_keyboard() {
            Focus::Popup
        } else if self.peek.is_some() {
            Focus::Peek
        } else {
            self.focused_pane().map_or(Focus::None, Focus::Pane)
        };
        if self.last_focus.as_ref() != Some(&focus) {
            self.last_focus = Some(focus.clone());
            fx.push(Effect::Focus(focus));
        }
        if self.connected {
            if self.listed {
                self.share_seen(fx);
            }
            self.pump(fx);
            if self.listed && std::mem::take(&mut self.scan) {
                self.adopt_and_check(fx);
            }
        }
        let saved = self.saved_view();
        if saved != self.last_saved {
            self.last_saved = saved;
            fx.push(Effect::SaveView);
        }
    }

    /// Writes down the shown space, its selected tab and that tab's focused pane, so a layout
    /// change elsewhere (a swap, a tab moved first) can't move what is shown through a fallback.
    fn pin(&mut self) {
        let Some(space) = self.view.active_space(&self.world) else {
            return;
        };
        let space_id = space.id.clone();
        let Some(tab) = self.view.selected_tab(&self.world, space) else {
            return;
        };
        let tab_id = tab.id.clone();
        let pane = self.view.focused_in(tab);
        // A saved space on a machine not read yet stays chosen: it shows once its layout
        // arrives (`app:src/mirror.rs` `View::apply_machine`). Meanwhile the fallback shows
        // without being written down.
        let waiting = self
            .view
            .active
            .as_deref()
            .is_some_and(|a| a != space_id && !self.world.loaded_for(a));
        if !waiting {
            self.view.active = Some(space_id.clone());
        }
        self.view.spaces.insert(space_id, tab_id.clone());
        if let Some(pane) = pane {
            let v = self.view.tabs.entry(tab_id).or_default();
            if v.focused.is_none() {
                v.focused = Some(pane);
            }
        }
    }

    /// Closing the shown tab shows the nearest shown, unsettled tab to its left, else to its
    /// right; closing an agent's tab focuses the next agent in the Agents order (wrapping, across
    /// spaces), else a shown tab. Only for the shown space; a background close keeps the slot rule
    /// (`cfg/mods/navigator-status/tab-close-focus.js`).
    fn close_focus(&mut self, before: &Before) {
        let Some(space_id) = before.active.as_deref() else {
            return;
        };
        if self.active_space().map(|s| s.id.as_str()) != Some(space_id) {
            return;
        }
        let Some(closed) = before.selected.get(space_id) else {
            return;
        };
        let Some(space) = self.world.space(space_id) else {
            return;
        };
        if space.tab(closed).is_some() {
            return;
        }
        if before.agents.iter().any(|(_, tab)| tab == closed) {
            let order = &before.agents;
            let at = order
                .iter()
                .rposition(|(_, tab)| tab == closed)
                .unwrap_or(0);
            let next = (1..=order.len())
                .map(|i| &order[(at + i) % order.len()])
                .find(|(a, tab)| {
                    tab != closed && self.world.pane_of(a).is_some() && self.world.is_agent(a)
                })
                .map(|(a, _)| a.clone());
            if let Some(pane) = next.and_then(|a| self.world.pane_of(&a)) {
                self.view.focus_pane(&self.world, &pane);
                return;
            }
        }
        let Some((_, tabs)) = before.spaces.iter().find(|(id, _)| id == space_id) else {
            return;
        };
        let at = tabs.iter().position(|t| t == closed).unwrap_or(0);
        let target = tabs[..at]
            .iter()
            .rev()
            .chain(&tabs[at + 1..])
            .find(|id| {
                space
                    .tab(id)
                    .is_some_and(|t| !self.view.settled(id) && !self.is_hidden_tab(t))
            })
            .cloned();
        if let Some(target) = target {
            self.view.select_tab(&self.world, &target);
        }
    }

    /// Asks for each reachable machine's next layout round.
    fn pump(&mut self, fx: &mut Effects) {
        let now = self.now;
        for (key, m) in self.world.machines_mut() {
            if !m.reachable() || self.retry_at.get(key).is_some_and(|due| now < *due) {
                continue;
            }
            if let Some(round) = m.mirror.next_round() {
                fx.push(Effect::Doc {
                    machine: key.clone(),
                    round,
                });
            }
        }
    }

    /// Looks up labeled terminals no pane shows (to adopt them), and panes whose terminal isn't
    /// live (to learn whether it is gone).
    fn adopt_and_check(&mut self, fx: &mut Effects) {
        let world = &self.world;
        let ready = |machine: Option<&str>| {
            world.machine(machine).is_some_and(|m| {
                m.reachable() && *m.mirror.layout() == Layout::Ready && m.shown().is_some()
            })
        };
        // A terminal this screen is starting is placed by its launch, not adopted: its tab is
        // the launch's (the reply with its address may come after the core's `created`).
        let launching: BTreeSet<String> = self
            .pending
            .values()
            .filter_map(|p| match p {
                Pending::Launch {
                    launch: Launch::Space { tab, .. } | Launch::Tab { tab, .. },
                    ..
                } => Some(tab.clone()),
                Pending::Launch {
                    launch: Launch::Split { target, .. },
                    ..
                } => world.tab_of(target).map(|t| t.id.clone()),
                // Nor is one the peek is keeping: its tab comes with the keep.
                Pending::Keep(keep) => Some(keep.tab.clone()),
                _ => None,
            })
            .collect();
        let orphans: Vec<Orphan> = adoption::orphans(
            world.live(),
            |a| world.pane_of(a).is_some(),
            ready,
            |a| self.busy.contains(a),
        )
        .into_iter()
        .filter(|o| o.tab.as_ref().is_none_or(|t| !launching.contains(t)))
        .collect();
        let now = self.now;
        let mut missing = Vec::new();
        for (key, m) in world.machines() {
            if !ready(key.as_deref()) {
                continue;
            }
            let Some(doc) = m.shown() else { continue };
            let just_born = |a: &str| {
                self.births
                    .get(a)
                    .is_some_and(|at| now - at < closed::BIRTH_GRACE.as_secs_f64())
            };
            missing.extend(
                closed::missing(doc, |a| world.is_live(a), just_born)
                    .into_iter()
                    .filter(|(_, a)| !self.busy.contains(a) && !self.not_closed.contains(a)),
            );
        }
        for orphan in orphans {
            self.busy.insert(orphan.terminal.clone());
            let terminal = orphan.terminal.clone();
            self.call(
                fx,
                Request::Get(TerminalGet {
                    terminal,
                    ..Default::default()
                }),
                Some(Pending::Adopt(orphan)),
            );
        }
        for (pane, address) in missing {
            self.busy.insert(address.clone());
            self.call(
                fx,
                Request::Get(TerminalGet {
                    terminal: address.clone(),
                    ..Default::default()
                }),
                Some(Pending::CheckClosed { pane, address }),
            );
        }
        let memory = closed::BIRTH_MEMORY.as_secs_f64();
        self.births.retain(|_, at| now - *at < memory);
    }
}

/// The direction of a [`NAVIGATOR_LABEL`] request: `<digits>:<digits>:<direction>`.
fn navigator_direction(token: &str) -> Option<Direction> {
    let mut parts = token.split(':');
    let digits =
        |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !digits(parts.next()) || !digits(parts.next()) {
        return None;
    }
    let direction = Direction::parse(parts.next()?)?;
    parts.next().is_none().then_some(direction)
}

/// The toast for an edit or launch the layout can't take.
fn unavailable(layout: &Layout) -> Toast {
    let why = match layout {
        Layout::Unavailable(why) | Layout::Blocked(why) | Layout::Undecodable(why) => why.clone(),
        Layout::Loading => "not read yet".into(),
        Layout::Ready => String::new(),
    };
    Toast::new(Level::Warning, "Layout unavailable", why)
}

/// The machine an intent edits.
fn intent_machine(intent: &Intent) -> Option<&str> {
    let id = match intent {
        Intent::NewSpace { space, .. } | Intent::NewTab { space, .. } => space,
        Intent::Split { target, .. } => target,
        Intent::RemovePane { pane }
        | Intent::RenamePane { pane, .. }
        | Intent::SwapPanes { pane, .. }
        | Intent::MovePane { pane, .. } => pane,
        Intent::CloseTab { tab } | Intent::RenameTab { tab, .. } | Intent::MoveTab { tab, .. } => {
            tab
        }
        Intent::CloseSpace { space }
        | Intent::RenameSpace { space, .. }
        | Intent::MoveSpace { space, .. } => space,
        Intent::Resize { sizes } => &sizes.first()?.0,
        Intent::Adopt(adopt) => &adopt.space,
    };
    machine_of(id)
}
