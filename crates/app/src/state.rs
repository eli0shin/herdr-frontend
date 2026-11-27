//! The app's state, and how core news, layout and input move it (`docs/research/rune.md` §8
//! "State and event flow").
//!
//! Each frame, [`State::pump`] runs the control socket's requests (`docs/cli.md`) and drains the
//! [`Link`] into [`Model::step`], and carries out the effects (calls and document rounds on the
//! link, focus, saving the view, answers to `do`). Then it keeps one
//! [`Viewer`] per terminal the shown tab draws, and applies the viewer policy of
//! `herdr-core-protocol.md` §4 ("Recommended viewer policy"):
//!
//! - Every terminal attaches `resumed: true`, so attaching takes no size: hidden and unfocused
//!   panes never do, and a viewer that comes back after a dropped connection doesn't either.
//! - Only the pane in use, the focused pane while the window is focused, takes the size: FACTS
//!   with the cell, then RESIZE with the pane's grid, then FOCUS(1) (§4 rule 4: the cell before
//!   the size, so a program woken by it asks for the new cell). Each change of its grid sends
//!   RESIZE again, at most every 25 ms and always the last one. When it stops being in use it
//!   sends FOCUS(0), and takes the size again only when it is used again.
//! - Each terminal is drawn at the core's grid (its record's `cols` × `rows`, which `size` events
//!   keep current), never the pane's: a pane bigger than it shows the background around it, a
//!   smaller one clips it.
//!
//! Viewers stay attached while their pane is in the layout, shown or not, so switching back is
//! instant and the program isn't sent to the headless size and back.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;

use herdr_proto::Display;
use herdr_proto::methods::TerminalAttach;
use model::mode::{PopupState, Rename};
use model::{
    Action, AppCommand, CopyAction, Effect, Event, Focus, Input, Level, Model, Popup, PopupAction,
    Toast, View,
};
use rune::{KeyEvent, TextEdit, Tx};
use serde_json::{Map, Value, json};
use terminal::{GridSize, Output, Padding, PaddingBalance, ScreenSize, Size, Terminal, Theme};

use crate::control::{self, Control};
use crate::home::{Home, VIEW};
use crate::keyfile::{self, Loaded, Watcher};
use crate::link::{Link, News, Offline, Out};
use crate::toast::{self, Toasts};
use crate::view::terminal::TerminalPane;
use crate::view::terminal::font::{Face, Font};
use crate::view::terminal::look::{self, ChromeColors, Look};
use crate::window::Keeper;
use crate::{chord, keys};

/// The terminal font's size until settings land: Ghostty's and Herdr app's 14 pt default.
pub const TERMINAL_POINTS: f32 = 14.0;

/// Lines of scrollback a terminal's first paint brings, as Herdr app asks.
pub const HISTORY: u64 = 2000;

/// The sidebar's width: what Eli's saved layout shows (`feature-inventory.md` §1).
pub const SIDEBAR: f32 = 366.0;

/// Ghostty's resize coalescing: RESIZE goes at most this often while a pane changes size.
pub const RESIZE_EVERY: f64 = 0.025;

/// The gap between split panes, in points: Eli's layout's 1 px.
pub const SPLIT_GAP: f32 = 1.0;

/// How long a scene or test waits for a terminal to settle.
const SETTLE: Duration = Duration::from_secs(5);

/// Where the core connection stands, as the window shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// No core to connect to: scenes and tests until they bring one.
    Offline,
    Connecting,
    Connected,
    /// The connection dropped or can't be made; the supervisor keeps trying.
    Down(String),
}

/// How a viewer's attach stream stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attach {
    /// Attach when connected. `fresh`: the screen is replaced by the repaint.
    Want {
        fresh: bool,
    },
    Attaching,
    Attached,
    /// The core ended the stream: the program is gone, or lost until it restarts.
    Ended,
    /// The attach failed; tried again on the next connection or restart.
    Failed,
}

/// One terminal as this window shows it: its worker, the pane drawing its snapshots, and what
/// its stream told the core.
pub struct Viewer {
    pub address: String,
    pub terminal: Terminal,
    /// The pane drawing it, from its first snapshot on.
    pub pane: Option<TerminalPane>,
    /// The pane's key and whether it shows focus.
    key: String,
    focused: bool,
    pub attach: Attach,
    /// The local terminal's grid and cell, in cells and device pixels.
    vt: (u16, u16, u32, u32),
    /// The FACTS, RESIZE and FOCUS this viewer last sent.
    told_facts: Option<Display>,
    told_grid: Option<GridSize>,
    focus_sent: bool,
}

impl Viewer {
    fn show(&mut self, snapshot: terminal::Snapshot) {
        match &mut self.pane {
            Some(pane) => pane.show(snapshot),
            None => {
                let mut pane = TerminalPane::new(self.key.clone(), snapshot);
                pane.focused = self.focused;
                self.pane = Some(pane);
            }
        }
    }
}

/// What the next handler with a [`Tx`] carries out.
#[derive(Default)]
struct Ui {
    focus: Option<Focus>,
    /// The next frame to ask Rune for.
    wake: Option<f64>,
    /// Window commands for Rune: quit, close, maximize.
    window: Vec<AppCommand>,
}

/// The window's tab area, as laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Area {
    w: f32,
    h: f32,
}

/// The app's state.
pub struct State {
    pub model: Model,
    link: Box<dyn Link>,
    started: bool,
    waker: Option<rune::Waker>,
    pub status: Status,
    /// Terminals with a viewer, by address.
    pub viewers: BTreeMap<String, Viewer>,
    /// Fixture panes, drawn bare in a grid with no shell around them: the renderer's scenes and
    /// the Ghostty pixel diff.
    pub panes: Vec<TerminalPane>,
    pub font: Font,
    /// The colors terminals start with.
    pub theme: Theme,
    pub look: Look,
    pub chrome: ChromeColors,
    /// The sidebar's width, or `None` when it's hidden.
    pub sidebar: Option<f32>,
    /// The font at the display scale the last frame drew at.
    face: RefCell<Option<Face>>,
    scale: f32,
    area: Option<Area>,
    window_active: bool,
    /// The terminal that takes the size.
    in_use: Option<String>,
    resized_at: f64,
    /// Every pump settles each terminal before it returns ([`Terminal::snapshot_now`]): scenes
    /// and fake-core tests, so a frame never races a worker.
    synchronous: bool,
    home: Option<Home>,
    /// The control socket in the home, served from setup on ([`State::serve`]).
    control: Option<Control>,
    pub window: Keeper,
    ui: Ui,
    /// When the model asked to be stepped with [`Event::Tick`] ([`Effect::WakeAt`]).
    tick_at: Option<f64>,
    now: f64,
    /// The in-app toasts on screen.
    pub toasts: Toasts,
    /// The key file's thread, once the app is set up with a home.
    keys: Option<Watcher>,
    /// The key file's text as read at start, for that thread.
    keys_read: Option<String>,
    /// How often that thread reads the file.
    pub keys_every: Duration,
    /// The open popup's filter field.
    pub help_filter: TextEdit,
    /// Something only the view shows changed (a toast, the prefix indicator): draw a frame.
    redraw: bool,
}

impl Default for State {
    fn default() -> State {
        State::new(Box::new(Offline), None, Keeper::default())
    }
}

/// A pane's element key: `pane-<id>`, with the `/` of a machine's ids (which separates key
/// paths) as `:`.
pub fn pane_key(id: &str) -> String {
    format!("pane-{}", id.replace('/', ":"))
}

/// A tab pill's element key, as [`pane_key`].
pub fn tab_key(id: &str) -> String {
    format!("tab-{}", id.replace('/', ":"))
}

/// The person's home folder, for the model's names and new spaces.
fn user_home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

impl State {
    /// A state on `link`, restoring the view saved in `home`.
    pub fn new(link: Box<dyn Link>, home: Option<Home>, window: Keeper) -> State {
        let view = home
            .as_ref()
            .and_then(|h| h.read(VIEW))
            .and_then(|text| View::from_json(&text).ok())
            .unwrap_or_default();
        let (theme, look) = look::arctic();
        let mut model = Model::with_view(user_home(), view);
        let mut toasts = Toasts::default();
        let mut keys_read = None;
        if let Some(home) = &home {
            let start = keyfile::load(home.dir());
            model.set_keymap(start.keymap);
            if let Some(why) = start.problem {
                toasts.push(
                    Toast::new(Level::Error, "keys.json can't be used", why),
                    0.0,
                );
            }
            keys_read = start.text;
        }
        let mut s = State {
            model,
            link,
            started: false,
            waker: None,
            status: Status::Offline,
            viewers: BTreeMap::new(),
            panes: Vec::new(),
            font: Font::jetbrains_mono(TERMINAL_POINTS),
            theme,
            look,
            chrome: ChromeColors::arctic(),
            sidebar: Some(SIDEBAR),
            face: RefCell::new(None),
            scale: 2.0,
            area: None,
            window_active: true,
            in_use: None,
            resized_at: f64::NEG_INFINITY,
            synchronous: false,
            home,
            control: None,
            window,
            ui: Ui::default(),
            tick_at: None,
            now: 0.0,
            toasts,
            keys: None,
            keys_read,
            keys_every: keyfile::EVERY,
            help_filter: TextEdit::default(),
            redraw: false,
        };
        s.window_sized(crate::SIZE.0, crate::SIZE.1);
        s
    }

    /// The window's size, in points, before its first layout: the panes' first frame is laid
    /// out for the tab area it leaves (Rune reports where elements are only after a frame).
    pub fn window_sized(&mut self, w: f32, h: f32) {
        if self.area.is_some() {
            return;
        }
        let area = model::Rect::new(
            0.0,
            0.0,
            (w - self.sidebar.unwrap_or(0.0)).max(0.0),
            (h - crate::view::shell::BAR).max(0.0),
        );
        self.step(
            self.now,
            Event::Geometry(model::Geometry {
                area,
                gap: SPLIT_GAP,
                zoom: 1.0,
            }),
        );
    }

    /// Puts a core in: the link starts at once if the app is set up, else at setup.
    pub fn connect(&mut self, link: Box<dyn Link>) {
        self.link = link;
        self.started = false;
        self.status = Status::Connecting;
        if let Some(waker) = self.waker.clone() {
            self.link.start(waker);
            self.started = true;
        }
    }

    /// The terminal font at display scale `scale`, measured once per scale.
    pub fn face(&self, scale: f32) -> Face {
        let mut face = self.face.borrow_mut();
        match face.as_ref() {
            Some(f) if f.scale == scale => f.clone(),
            _ => {
                let made = self
                    .font
                    .at(scale)
                    .expect("the terminal font measures at every scale");
                *face = Some(made.clone());
                made
            }
        }
    }

    pub fn window_active(&self) -> bool {
        self.window_active
    }

    /// The terminal that holds the size.
    pub fn in_use(&self) -> Option<&str> {
        self.in_use.as_deref()
    }

    /// Steps the model and carries out what it asks. True when it used the key (always, for
    /// anything but a key).
    pub fn step(&mut self, now: f64, input: impl Into<Input>) -> bool {
        self.now = self.now.max(now);
        let before = self.model.mode().clone();
        let effects = self.model.step(self.now, input);
        let handled = effects.handled;
        for effect in effects {
            self.carry_out(effect);
        }
        if *self.model.mode() != before {
            self.redraw = true;
            self.mode_changed(&before);
        }
        handled
    }

    /// The keyboard went to a new mode. A popup or copy mode the window doesn't draw yet would
    /// hold every key unseen, so it closes at once with a "not yet" toast; the help opens with
    /// an empty filter and says how many rows it has.
    fn mode_changed(&mut self, before: &model::Mode) {
        let mode = self.model.mode().clone();
        match &mode {
            model::Mode::Popup(PopupState::List(list)) if list.popup == Popup::Help => {
                let opened = before
                    .popup()
                    .and_then(PopupState::popup)
                    .is_none_or(|p| p != Popup::Help);
                if opened {
                    self.help_filter = TextEdit::default();
                }
                let rows = crate::view::help::shown(self, list).len();
                if rows != list.rows {
                    self.step(self.now, Event::PopupRows(rows));
                }
            }
            model::Mode::Popup(state) => {
                let action = match state {
                    PopupState::List(list) => Action::Open(list.popup),
                    PopupState::Jump(_) => Action::Open(Popup::AgentJump),
                    PopupState::Prompt(p) => match p.rename {
                        Rename::Tab(_) => Action::TabRename,
                        Rename::Space(_) => Action::SpaceRename,
                    },
                    PopupState::Confirm(_) => Action::SpaceClose,
                };
                self.not_yet(&action);
                self.step(self.now, Action::Popup(PopupAction::Close));
            }
            model::Mode::Copy(_) => {
                self.not_yet(&Action::CopyMode);
                self.step(self.now, Action::Copy(CopyAction::Quit));
            }
            model::Mode::Normal | model::Mode::Prefix { .. } => {}
        }
    }

    /// Shows a toast.
    pub fn toast(&mut self, toast: Toast) {
        let due = self.toasts.push(toast, self.now);
        self.wake_at(due);
        self.redraw = true;
    }

    fn not_yet(&mut self, action: &Action) {
        self.toast(toast::not_yet(action));
    }

    fn carry_out(&mut self, effect: Effect) {
        match effect {
            Effect::Call { token, request } => self.link.call(token, request),
            Effect::Doc { machine, round } => self.link.doc(machine, round),
            Effect::Focus(focus) => self.ui.focus = Some(focus),
            Effect::WakeAt(at) => {
                self.tick_at = Some(self.tick_at.map_or(at, |t| t.min(at)));
                self.wake_at(at);
            }
            Effect::SaveView => {
                if let Some(home) = &self.home {
                    home.save(VIEW, self.model.saved_view().to_json());
                }
            }
            Effect::App(command) => self.app_command(command),
            Effect::Toast(toast) => self.toast(toast),
            Effect::NotYet(action) => self.not_yet(&action),
            // The terminal's own commands (sends, paste, copy, search) are ticket 015's and
            // 019's; until then they say so.
            Effect::Terminal { command, .. } => match command {
                model::TerminalCommand::Action(action) => self.not_yet(&Action::Terminal(action)),
                // Copy mode says "not yet" once, as it closes (`State::mode_changed`).
                model::TerminalCommand::CopyStart
                | model::TerminalCommand::CopyMove(_)
                | model::TerminalCommand::CopySelect(_)
                | model::TerminalCommand::CopyYank
                | model::TerminalCommand::CopyEnd => {}
            },
            // A `do` call's answer that came in a later step: `space.new`'s, once its terminal
            // runs.
            Effect::Answer { id, reply } => {
                if let Some(control) = &mut self.control {
                    control.answer(id, reply);
                }
            }
            // The help has nothing to pick; the other list popups come with their tickets (and
            // close before a pick, `State::mode_changed`). Alerts are ticket 025's.
            Effect::Pick { .. } | Effect::Alert(_) => {}
        }
    }

    /// An app command: the window's own are Rune's, the sidebar is this state's, the escape
    /// hatch resets the key file; the rest aren't built yet.
    fn app_command(&mut self, command: AppCommand) {
        match command {
            AppCommand::Quit | AppCommand::CloseWindow | AppCommand::Maximize => {
                self.ui.window.push(command)
            }
            AppCommand::ToggleSidebar => {
                self.sidebar = match self.sidebar {
                    Some(_) => None,
                    None => Some(SIDEBAR),
                };
                self.redraw = true;
            }
            AppCommand::Reset => {
                self.model.set_keymap(model::Keymap::eli());
                match &self.keys {
                    // The file goes aside on the key thread; it says where.
                    Some(keys) => keys.reset(),
                    None => {
                        self.toast(Toast::new(Level::Info, "Keys reset", "Eli's keys are back"))
                    }
                }
            }
            // macOS's menu hides the app on ⌘H and ⌥⌘H before the window sees them.
            AppCommand::Hide
            | AppCommand::HideOthers
            | AppCommand::ZoomIn
            | AppCommand::ZoomOut
            | AppCommand::ZoomReset
            | AppCommand::LayoutSwitch => self.not_yet(&Action::App(command)),
        }
    }

    /// What the key file's thread found.
    fn hear_keys(&mut self) {
        let Some(keys) = &self.keys else { return };
        let news: Vec<Loaded> = std::iter::from_fn(|| keys.next()).collect();
        for loaded in news {
            match loaded {
                Loaded::Keys(keymap) => {
                    self.model.set_keymap(keymap);
                    self.redraw = true;
                }
                Loaded::Bad(why) => {
                    self.toast(Toast::new(Level::Error, "keys.json can't be used", why))
                }
                Loaded::Reset(Ok(aside)) => self.toast(Toast::new(
                    Level::Info,
                    "Keys reset",
                    format!("Eli's keys are back; yours are in {aside}"),
                )),
                Loaded::Reset(Err(why)) => self.toast(Toast::new(
                    Level::Info,
                    "Keys reset",
                    format!("Eli's keys are back ({why})"),
                )),
            }
        }
    }

    fn wake_at(&mut self, at: f64) {
        self.ui.wake = Some(self.ui.wake.map_or(at, |w| w.min(at)));
    }

    /// Hands the focus, the window commands and the wakes asked for to Rune.
    fn apply_ui(&mut self, tx: &mut Tx) {
        match self.ui.focus.take() {
            Some(Focus::Pane(pane)) => tx.focus_when_shown(pane_key(&pane), false),
            Some(Focus::Popup) => tx.focus_when_shown(crate::view::help::FILTER, false),
            Some(Focus::None) | None => {}
        }
        for command in std::mem::take(&mut self.ui.window) {
            match command {
                AppCommand::Quit => tx.quit(),
                AppCommand::CloseWindow => tx.close_window(),
                AppCommand::Maximize => tx.toggle_maximized(),
                _ => {}
            }
        }
        if let Some(at) = self.ui.wake.take() {
            tx.wake_at(at);
        }
    }

    /// Drains the core and the terminals and keeps the viewers and their sizes right. True when
    /// something on screen changed.
    pub fn pump(&mut self, now: f64) -> bool {
        self.now = self.now.max(now);
        let now = self.now;
        if let Some(due) = self.tick_at
            && now >= due
        {
            self.tick_at = None;
            self.step(now, Event::Tick);
        }
        self.hear_keys();
        if self.toasts.expire(now) {
            self.redraw = true;
        }
        if let Some(due) = self.toasts.next_due() {
            self.wake_at(due);
        }
        // Before the link, so a fake core answers the calls a `do` made in this same pump.
        let mut changed = self.drain_control(now);
        // A fake core answers on the next poll, so keep going until it is quiet.
        for _ in 0..64 {
            let mut news = Vec::new();
            self.link.poll(&mut news);
            if news.is_empty() {
                break;
            }
            changed = true;
            for item in news {
                self.hear(now, item);
            }
        }
        changed |= self.keep_viewers();
        changed |= self.fit(now);
        changed |= self.drain_terminals();
        changed | std::mem::take(&mut self.redraw)
    }

    /// Runs the requests the control socket took (`ls`, `status`, `do`) through the model, and
    /// carries out their effects. True when any came in.
    fn drain_control(&mut self, now: f64) -> bool {
        let Some(control) = &mut self.control else {
            return false;
        };
        let facts = window_facts(&self.font);
        let before = self.model.mode().clone();
        let Some(fx) = control.drain(&mut self.model, now, &facts) else {
            return false;
        };
        for effect in fx.list {
            self.carry_out(effect);
        }
        // A `do` can open a popup or copy mode as a key does.
        if *self.model.mode() != before {
            self.mode_changed(&before);
        }
        true
    }

    /// Serves the control socket in the home, if there is one: the CLI's `ls | do | status`
    /// (`docs/cli.md`). Another window already serving it keeps it; this one goes on without.
    fn serve(&mut self, waker: rune::Waker) {
        let Some(home) = &self.home else { return };
        if self.control.is_some() {
            return;
        }
        match Control::listen(home.dir().join(control::SOCKET), move || waker()) {
            Ok(control) => self.control = Some(control),
            Err(e) => eprintln!("herdr-frontend: not serving ls, do and status: {e}"),
        }
    }

    /// The control socket's path, while the window serves it.
    pub fn control_socket(&self) -> Option<&std::path::Path> {
        self.control.as_ref().map(Control::path)
    }

    fn hear(&mut self, now: f64, news: News) {
        match news {
            News::Connected => {
                self.status = Status::Connected;
                for viewer in self.viewers.values_mut() {
                    viewer.attach = Attach::Want { fresh: true };
                }
                self.step(now, Event::Connected);
            }
            News::Lost(why) => {
                self.status = Status::Down(why);
                for viewer in self.viewers.values_mut() {
                    viewer.attach = Attach::Want { fresh: true };
                    viewer.told_grid = None;
                    viewer.told_facts = None;
                    viewer.focus_sent = false;
                }
                self.in_use = None;
                self.step(now, Event::Lost);
            }
            News::Unavailable(why) => self.status = Status::Down(why),
            News::Event(event) => {
                self.step(now, event);
            }
            News::Restarted(addresses) => {
                for address in addresses {
                    if let Some(viewer) = self.viewers.get_mut(&address) {
                        viewer.attach = Attach::Want { fresh: true };
                    }
                }
            }
            News::Attached { address, result } => {
                let Some(viewer) = self.viewers.get_mut(&address) else {
                    return;
                };
                if viewer.attach != Attach::Attaching {
                    return;
                }
                viewer.attach = if result.is_ok() {
                    Attach::Attached
                } else {
                    Attach::Failed
                };
                // A new stream has told the core nothing yet.
                viewer.told_facts = None;
                viewer.told_grid = None;
                viewer.focus_sent = false;
                if self.in_use.as_deref() == Some(address.as_str()) {
                    self.in_use = None;
                }
            }
            News::Ended { address, reason } => {
                if let Some(viewer) = self.viewers.get_mut(&address) {
                    viewer.attach = match reason {
                        Some(_) => Attach::Ended,
                        None => Attach::Want { fresh: true },
                    };
                    viewer.focus_sent = false;
                    viewer.told_grid = None;
                }
                if self.in_use.as_deref() == Some(address.as_str()) {
                    self.in_use = None;
                }
                if let Some(reason) = reason {
                    self.step(now, Event::StreamClosed { address, reason });
                }
            }
        }
    }

    /// The panes on screen, where they sit in the tab area, and their terminals.
    pub fn shown_panes(&self) -> Vec<(String, model::Rect, Option<String>)> {
        let world = self.model.world();
        self.model
            .pane_rects()
            .into_iter()
            .map(|(pane, rect)| {
                let address = world.address_of(&pane).map(str::to_owned);
                (pane, rect, address)
            })
            .collect()
    }

    /// Makes viewers for the terminals the shown tab draws, attaches those waiting, and drops
    /// viewers whose terminal left the layout or ended.
    fn keep_viewers(&mut self) -> bool {
        let mut changed = false;
        let focused = self.model.focused_pane();
        for (pane, _, address) in self.shown_panes() {
            let Some(address) = address else { continue };
            let Some(record) = self.model.world().terminal(&address) else {
                continue;
            };
            if !self.model.world().is_live(&address) {
                continue;
            }
            let (cols, rows) = (record.cols.max(1), record.rows.max(1));
            if !self.viewers.contains_key(&address) {
                let viewer = self.viewer(&address, cols, rows);
                self.viewers.insert(address.clone(), viewer);
                changed = true;
            }
            let active = self.window_active;
            let viewer = self.viewers.get_mut(&address).expect("made above");
            let key = pane_key(&pane);
            let has_focus = active && focused.as_deref() == Some(pane.as_str());
            if viewer.key != key || viewer.focused != has_focus {
                viewer.key = key;
                viewer.focused = has_focus;
                if let Some(p) = &mut viewer.pane {
                    p.key = viewer.key.clone();
                    p.focused = has_focus;
                }
                changed = true;
            }
        }
        let world = self.model.world();
        let gone: Vec<String> = self
            .viewers
            .keys()
            .filter(|a| world.pane_of(a).is_none() || !world.is_live(a))
            .cloned()
            .collect();
        for address in gone {
            if let Some(viewer) = self.viewers.remove(&address)
                && matches!(viewer.attach, Attach::Attaching | Attach::Attached)
            {
                self.link.detach(&address);
            }
            if self.in_use.as_deref() == Some(address.as_str()) {
                self.in_use = None;
            }
            changed = true;
        }
        if self.status == Status::Connected {
            let display = self.display();
            for viewer in self.viewers.values_mut() {
                let Attach::Want { fresh } = viewer.attach else {
                    continue;
                };
                let (cols, rows, ..) = viewer.vt;
                viewer.attach = Attach::Attaching;
                self.link.attach(
                    TerminalAttach {
                        terminal: viewer.address.clone(),
                        cols,
                        rows,
                        history: HISTORY,
                        display: Some(display.clone()),
                        resumed: true,
                        ..Default::default()
                    },
                    viewer.terminal.writer(),
                    fresh,
                );
            }
        }
        changed
    }

    fn viewer(&self, address: &str, cols: u16, rows: u16) -> Viewer {
        let cell = self.face(self.scale).metrics.cell_size();
        let waker = self
            .waker
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(|| {}));
        let terminal = Terminal::spawn(
            terminal::Config {
                cols,
                rows,
                cell_width: cell.width,
                cell_height: cell.height,
                theme: self.theme.clone(),
                ..terminal::Config::default()
            },
            waker,
        );
        Viewer {
            address: address.to_owned(),
            terminal,
            pane: None,
            key: String::new(),
            focused: false,
            attach: Attach::Want { fresh: false },
            vt: (cols, rows, cell.width, cell.height),
            told_facts: None,
            told_grid: None,
            focus_sent: false,
        }
    }

    /// The colors and cell this window reports (FACTS, and the attach's `display`).
    pub fn display(&self) -> Display {
        let cell = self.face(self.scale).metrics.cell_size();
        let hex =
            |c: Option<terminal::Rgb>| c.map(|c| format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b));
        Display {
            fg: hex(self.theme.foreground),
            bg: hex(self.theme.background),
            cursor: hex(self
                .look
                .cursor
                .or(self.theme.cursor)
                .or(self.theme.foreground)),
            cell: Some([cell.width, cell.height]),
            ..Default::default()
        }
    }

    /// The grid a pane of `rect` points holds at the window's scale.
    pub fn grid_for(&self, rect: model::Rect) -> GridSize {
        let cell = self.face(self.scale).metrics.cell_size();
        let px = |points: f32| (points * self.scale).floor().max(0.0) as u32;
        Size::new(
            ScreenSize {
                width: px(rect.w),
                height: px(rect.h),
            },
            cell,
            Padding::default(),
            PaddingBalance::False,
        )
        .report()
        .grid
    }

    /// Keeps each local terminal at the core's grid, and lets the pane in use take the size.
    fn fit(&mut self, now: f64) -> bool {
        let cell = self.face(self.scale).metrics.cell_size();
        for viewer in self.viewers.values_mut() {
            let Some(record) = self.model.world().terminal(&viewer.address) else {
                continue;
            };
            let vt = (
                record.cols.max(1),
                record.rows.max(1),
                cell.width,
                cell.height,
            );
            if vt != viewer.vt {
                viewer.vt = vt;
                viewer.terminal.resize(vt.0, vt.1, vt.2, vt.3);
            }
        }

        let in_use = self
            .window_active
            .then(|| self.model.focused_terminal().map(str::to_owned))
            .flatten()
            .filter(|a| {
                self.viewers
                    .get(a)
                    .is_some_and(|v| v.attach == Attach::Attached)
            });
        if self.in_use != in_use {
            if let Some(old) = self.in_use.take()
                && let Some(viewer) = self.viewers.get_mut(&old)
            {
                if viewer.focus_sent && viewer.attach == Attach::Attached {
                    self.link.send(&old, Out::Focus(false));
                }
                viewer.focus_sent = false;
                // It takes the size again when it's used again.
                viewer.told_grid = None;
            }
            self.in_use = in_use;
        }
        let Some(address) = self.in_use.clone() else {
            return false;
        };
        if self.area.is_none() {
            return false;
        }
        let Some(rect) = self
            .shown_panes()
            .into_iter()
            .find(|(_, _, a)| a.as_deref() == Some(address.as_str()))
            .map(|(_, rect, _)| rect)
        else {
            return false;
        };
        let grid = self.grid_for(rect);
        let display = self.display();
        let viewer = self.viewers.get_mut(&address).expect("in use");
        if viewer.told_facts.as_ref() != Some(&display) {
            self.link.send(&address, Out::Facts(display.clone()));
            viewer.told_facts = Some(display);
        }
        if viewer.told_grid != Some(grid) {
            let due = self.resized_at + RESIZE_EVERY;
            if viewer.told_grid.is_some() && now < due {
                self.ui.wake = Some(self.ui.wake.map_or(due, |w| w.min(due)));
            } else {
                self.link.send(
                    &address,
                    Out::Resize {
                        cols: grid.columns,
                        rows: grid.rows,
                    },
                );
                viewer.told_grid = Some(grid);
                self.resized_at = now;
            }
        }
        if !viewer.focus_sent && viewer.told_grid.is_some() {
            self.link.send(&address, Out::Focus(true));
            viewer.focus_sent = true;
        }
        false
    }

    /// Sends what the terminals encoded (replies as AUTOMATIC_INPUT, typing as DATA) and shows
    /// the shown terminals' new snapshots.
    fn drain_terminals(&mut self) -> bool {
        let shown: Vec<String> = self.model.visible_terminals();
        let mut changed = false;
        let rounds = if self.synchronous { 4 } else { 1 };
        for _ in 0..rounds {
            if self.synchronous {
                for address in &shown {
                    if let Some(viewer) = self.viewers.get_mut(address)
                        && let Some(snapshot) = viewer.terminal.snapshot_now(SETTLE)
                    {
                        viewer.show(snapshot);
                        changed = true;
                    }
                }
            }
            let mut sent = false;
            for viewer in self.viewers.values() {
                while let Ok(output) = viewer.terminal.output().try_recv() {
                    if viewer.attach != Attach::Attached {
                        continue;
                    }
                    match output {
                        Output::Reply(bytes) => {
                            self.link.send(&viewer.address, Out::Automatic(bytes))
                        }
                        Output::Input(bytes) => {
                            self.link.send(&viewer.address, Out::Input(bytes));
                            sent = true;
                        }
                        // Titles, bells and the clipboard come with their tickets.
                        Output::Event(_) => {}
                    }
                }
            }
            if !sent {
                break;
            }
        }
        if !self.synchronous {
            for address in &shown {
                if let Some(viewer) = self.viewers.get_mut(address)
                    && let Some(snapshot) = viewer.terminal.take_snapshot()
                {
                    viewer.show(snapshot);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Settles everything at once, waiting on the terminals: for scenes and tests on a fake core,
    /// whose frames then never race a worker.
    pub fn settle(&mut self) {
        self.synchronous = true;
        if !self.started {
            let waker = self
                .waker
                .clone()
                .unwrap_or_else(|| std::sync::Arc::new(|| {}));
            self.link.start(waker);
            self.started = true;
        }
        for _ in 0..8 {
            if !self.pump(self.now) {
                break;
            }
        }
    }

    // ---- Rune hooks -------------------------------------------------------------------------

    /// `AppSpec::setup`: the waker other threads wake the UI with; the control socket, the link
    /// and the key file's thread start here. The socket goes first: macOS has no
    /// `SOCK_CLOEXEC`, so an `hc` the link spawned while the socket was being made would keep it
    /// open.
    pub fn setup(s: &mut State, waker: rune::Waker) {
        s.waker = Some(waker.clone());
        s.serve(waker.clone());
        if !s.started {
            s.link.start(waker.clone());
            s.started = true;
        }
        if s.keys.is_none()
            && let Some(home) = &s.home
        {
            let known = s.keys_read.take();
            s.keys = Some(Watcher::start(
                home.dir().to_owned(),
                known,
                s.keys_every,
                waker,
            ));
        }
    }

    /// `AppSpec::poll`.
    pub fn poll(s: &mut State, now: f64, tx: &mut Tx) -> bool {
        let changed = s.pump(now);
        s.apply_ui(tx);
        changed
    }

    /// `App::on_focus`: the window became the foreground window, or stopped being it.
    pub fn on_focus(s: &mut State, active: bool, tx: &mut Tx) {
        s.window_active = active;
        s.step(tx.clock(), Event::WindowActive(active));
        if !active {
            s.window.save(false);
        }
        s.pump(tx.clock());
        s.apply_ui(tx);
    }

    /// The tab area was laid out: the model's geometry, and the pane in use's grid.
    pub fn laid_out(&mut self, rect: [f32; 4], tx: &mut Tx) {
        let area = Area {
            w: rect[2],
            h: rect[3],
        };
        let scale = tx.scale();
        if self.area == Some(area) && self.scale == scale {
            tx.live_only();
            return;
        }
        self.area = Some(area);
        self.scale = scale;
        self.step(
            tx.clock(),
            Event::Geometry(model::Geometry {
                area: model::Rect::new(0.0, 0.0, area.w, area.h),
                gap: SPLIT_GAP,
                zoom: 1.0,
            }),
        );
        self.pump(tx.clock());
        self.apply_ui(tx);
    }

    /// A pane took or lost the keyboard: the model follows a click into another pane.
    pub fn pane_focus(&mut self, pane: &str, gained: bool, tx: &mut Tx) {
        if gained && self.model.focused_pane().as_deref() != Some(pane) {
            self.step(tx.clock(), Action::FocusPane(pane.to_owned()));
            self.pump(tx.clock());
            self.apply_ui(tx);
        }
    }

    /// A key in pane `pane`: plain typing goes to its terminal. False for keys it leaves.
    pub fn typed(&mut self, pane: &str, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let Some(key) = keys::encode(ev) else {
            return false;
        };
        if self.model.focused_pane().as_deref() != Some(pane) {
            self.step(tx.clock(), Action::FocusPane(pane.to_owned()));
        }
        let Some(address) = self.model.world().address_of(pane).map(str::to_owned) else {
            return true;
        };
        // Typing is using it: it takes the size first, if it hadn't.
        self.pump(tx.clock());
        if let Some(viewer) = self.viewers.get(&address) {
            viewer.terminal.key(key);
        }
        if self.synchronous {
            self.pump(tx.clock());
        }
        self.apply_ui(tx);
        true
    }

    /// An action from the window (a click, the popup's field), stepped and carried out.
    pub fn act(&mut self, action: Action, tx: &mut Tx) {
        self.step(tx.clock(), action);
        self.pump(tx.clock());
        self.apply_ui(tx);
    }

    /// A key through the one keymap and mode machine. `terminal`: a terminal pane has the
    /// keyboard. True when the model used it, so it goes no further.
    pub fn keymap_key(&mut self, ev: &KeyEvent, terminal: bool, tx: &mut Tx) -> bool {
        let Some(press) = chord::press(ev, tx.key_base()) else {
            return false;
        };
        let handled = self.step(tx.clock(), Event::Key { press, terminal });
        self.pump(tx.clock());
        self.apply_ui(tx);
        handled
    }

    /// The root's `El::on_key_preview`: every key while something has focus, before the
    /// focused element sees it (`docs/research/rune.md` §4). With nothing focused previews
    /// don't run; [`State::on_key`] reads those keys instead, so none is read twice.
    pub fn preview_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let Some(focused) = tx.focused() else {
            return false;
        };
        let terminal = focused.starts_with("pane-");
        let in_field = focused == crate::view::help::FILTER;
        let handled = s.keymap_key(ev, terminal, tx);
        if handled || !s.model.mode().holds_keyboard() {
            return handled;
        }
        // An open popup keeps the keyboard: its field types what the popup leaves, and nothing
        // else gets a key, nor does Tab move focus out of it to a pane behind.
        if in_field && ev.key != rune::Key::Tab {
            return false;
        }
        tx.focus(crate::view::help::FILTER, false);
        true
    }

    /// `AppSpec::on_key`: with nothing focused, the keymap, then typing into the model's focused
    /// pane, which takes the keyboard back. Keys a focused element passed on (`tx.focused()` is
    /// set) already went through the keymap in [`State::preview_key`], and go nowhere.
    pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if tx.focused().is_some() {
            return false;
        }
        let pane = s.model.focused_pane();
        if s.keymap_key(ev, pane.is_some(), tx) {
            return true;
        }
        // An open popup's typing goes to its field, never the terminal behind it.
        if s.model.mode().holds_keyboard() {
            return false;
        }
        let Some(pane) = pane.or_else(|| s.model.focused_pane()) else {
            return false;
        };
        let typed = s.typed(&pane, ev, tx);
        if typed {
            tx.focus(pane_key(&pane), false);
        }
        typed
    }

    /// `App::on_close` and `App::on_quit`: the last save of the window's place and the view.
    pub fn on_close(s: &mut State, _tx: &mut Tx) {
        s.window.save(true);
        if let Some(home) = &s.home {
            home.save_now(VIEW, s.model.saved_view().to_json());
        }
        // The socket goes with the window, even when the process ends before the state drops.
        s.control = None;
    }
}

/// What only the window knows for `status`: its zoom and font sizes (`docs/cli.md`).
fn window_facts(font: &Font) -> Map<String, Value> {
    let mut facts = Map::new();
    facts.insert("zoom".into(), json!(1.0));
    facts.insert("font_size".into(), json!(font.points));
    facts.insert("ui_font_size".into(), json!(crate::view::shell::UI_POINTS));
    facts
}
