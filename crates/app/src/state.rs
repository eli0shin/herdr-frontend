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
//! - A click in the pane in use takes the size back when the core holds it at another grid
//!   (Herdr app attached, refit or reattached it). Nothing takes it back without real use.
//! - DATA from any other viewer (a paste into a window in the background) first reports the grid
//!   it should hold, so the core never applies a stale one.
//! - The engine's replies to queries go as AUTOMATIC_INPUT, which takes nothing, where the stream
//!   carries it; where it doesn't, only the terminal in use answers, as DATA, and only while no
//!   other viewer has moved its grid.
//! - Each terminal is drawn at the core's grid (its record's `cols` × `rows`, which `size` events
//!   keep current), never the pane's: a pane bigger than it shows the background around it, a
//!   smaller one clips it.
//!
//! Viewers stay attached while their pane is in the layout, shown or not, so switching back is
//! instant and the program isn't sent to the headless size and back.
//!
//! Input goes to the terminals' workers, which encode it for the program's modes
//! (`docs/research/terminal-rendering.md` §3): keys with their physical key, consumed modifiers
//! and releases ([`keys`]), what an input method commits, mouse reports while the program tracks
//! the mouse and selections while it doesn't, the wheel, pastes and dropped files. Focus is never
//! encoded here: the FOCUS frames above are the core's to turn into `CSI I` and `CSI O`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use ghostty_vt_sys::{GHOSTTY_KEY_C, GHOSTTY_KEY_V};
use herdr_proto::Display;
use herdr_proto::methods::TerminalAttach;
use model::agents::{self, Sort, Tracker};
use model::mode::PopupState;
use model::{
    Action, AppCommand, Effect, Event, Focus, Input, Level, Model, Pick, Popup, PopupAction,
    TerminalAction, TerminalCommand, Toast, View,
};
use rune::{
    ClipboardContent, Drag, Family, ImageCache, Ime, KeyEvent, Phase, PointerButton, PointerEvent,
    TextEdit, TextStyle, Tx, WheelEvent,
};
use serde_json::{Map, Value, json};
use terminal::{
    ActiveScreen, GridSize, MouseAction, MouseButton, MouseEvent, OptionAsAlt, Output, Padding,
    PaddingBalance, ScreenSize, Scroll, SelectAction, SelectEvent, Size, Terminal, Theme,
};

use crate::config::settings::{self, Settings, UI_FONT_SIZE};
use crate::config::theme::Chrome;
use crate::config::{Config, SETTINGS, Sources, Stamp, Watcher as ConfigWatcher};
use crate::control::{self, Control, Probe};
use crate::e2e::{self, Tap};
use crate::home::{Home, VIEW};
use crate::keyfile::{self, Loaded, Watcher};
use crate::keys::KeyInfo;
use crate::link::{Link, News, Offline, Out, Upload};
use crate::shared::{self, Shared};
use crate::toast::{self, Toasts};
use crate::view::terminal::TerminalPane;
use crate::view::terminal::font::{Face, Font};
use crate::view::terminal::look::Look;
use crate::window::Keeper;
use crate::{chord, keys};

/// Ghostty's and Herdr app's default terminal font size, 14 pt: the renderer's fixtures and the
/// Ghostty pixel diff draw at it ([`Settings::fixture`]).
pub const TERMINAL_POINTS: f32 = 14.0;

/// Lines of scrollback a terminal's first paint brings, as Herdr app asks.
pub const HISTORY: u64 = 2000;

/// Ghostty's resize coalescing: RESIZE goes at most this often while a pane changes size.
pub const RESIZE_EVERY: f64 = 0.025;

/// The gap between split panes, in points: Eli's layout's 1 px.
pub const SPLIT_GAP: f32 = 1.0;

/// How long a scene or test waits for a terminal to settle.
const SETTLE: Duration = Duration::from_secs(5);

/// The window points Rune gives a notched mouse wheel's step on macOS (its `LINE_POINTS`).
const WHEEL_NOTCH: f32 = 40.0;

/// Lines a notch scrolls: Ghostty's default wheel multiplier, which Herdr app keeps.
const LINES_PER_NOTCH: f32 = 3.0;

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
    /// The core's grid changed away from `told_grid` since it was told: another viewer took the
    /// size.
    moved: bool,
    focus_sent: bool,
    /// Its stream carries the engine's replies as AUTOMATIC_INPUT (rule 9).
    automatic_input: bool,
    /// What the input method is composing in this terminal.
    pub preedit: Option<String>,
    /// The find bar: open or not, and its query, which stays while it's closed.
    pub find: Option<FindBar>,
    /// Trackpad travel not yet a whole line, in points.
    wheel: f32,
    /// Cmd is held over the grid, so the link under the pointer is underlined.
    hovering_link: bool,
}

/// A terminal's find bar (`cmd-f`): its field's text is the query.
#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub edit: TextEdit,
}

impl Viewer {
    fn show(&mut self, snapshot: terminal::Snapshot) {
        match &mut self.pane {
            Some(pane) => pane.show(snapshot),
            None => {
                let mut pane = TerminalPane::new(self.key.clone(), snapshot);
                pane.focused = self.focused;
                pane.preedit = self.preedit.clone();
                self.pane = Some(pane);
            }
        }
    }

    fn set_preedit(&mut self, preedit: Option<String>) {
        if let Some(pane) = &mut self.pane {
            pane.preedit = preedit.clone();
        }
        self.preedit = preedit;
    }

    /// The program's modes and screen as the last snapshot drew them.
    fn shown(&self) -> Option<&terminal::Snapshot> {
        self.pane.as_ref().map(|p| &p.snapshot)
    }

    fn tracks_mouse(&self) -> bool {
        self.shown().is_some_and(|s| s.modes.mouse_tracking)
    }

    /// Whether the pixel at `(x, y)` from the grid's corner is on the link underlined while Cmd
    /// is held.
    fn link_at(&self, x: f32, y: f32) -> bool {
        let Some(s) = self.shown() else {
            return false;
        };
        if s.cell_width == 0 || s.cell_height == 0 || x < 0.0 || y < 0.0 {
            return false;
        }
        let col = (x / s.cell_width as f32) as u16;
        let row = (y / s.cell_height as f32) as u16;
        s.link
            .iter()
            .any(|span| span.row == row && (span.start..=span.end).contains(&col))
    }
}

/// What a mouse button held over a terminal is doing.
#[derive(Clone, Debug, PartialEq)]
enum Gesture {
    /// The program tracks the mouse: the press, motion and release are its.
    Report(MouseButton),
    /// Selecting text.
    Select,
}

#[derive(Clone, Debug, PartialEq)]
struct Press {
    address: String,
    gesture: Gesture,
}

/// What the next handler with a [`Tx`] carries out.
#[derive(Default)]
struct Ui {
    focus: Option<Focus>,
    /// The next frame to ask Rune for.
    wake: Option<f64>,
    /// Window commands for Rune: quit, close, maximize.
    window: Vec<AppCommand>,
    /// Keys a harness sent (`crate::e2e`), to deliver as typed.
    keys: Vec<KeyEvent>,
    /// Text for the clipboard: a selection copied.
    copy: Option<String>,
    /// Terminals to paste the clipboard into, once it's read.
    paste: Vec<String>,
    /// Terminals to paste the clipboard's image into, once it's read.
    paste_image: Vec<String>,
    /// An element to give the keyboard, by key: a find bar's field.
    focus_key: Option<String>,
    /// URLs and paths for the system's opener.
    open: Vec<String>,
}

/// A divider being dragged: the sizes beside it before, put back if the drag is cancelled after
/// it moved them.
struct DividerDrag {
    container: String,
    index: usize,
    before: Vec<(String, herdr_shared::Size)>,
    moved: bool,
}

/// A pane's grip being dragged (`app:src/window/pane_drag.rs` `PaneDrag`).
#[derive(Clone, Debug, PartialEq)]
pub struct GripDrag {
    pub pane: String,
    /// The tab it started in: a drag ends if another tab shows.
    tab: String,
    /// Where the press was, in the tab area.
    from: (f32, f32),
    /// Past the 4 pt that make a press a drag.
    moving: bool,
    /// The pane under the pointer and where on it the pane would land.
    pub target: Option<(String, model::Edge)>,
}

/// A grip press moves this far before it is a drag, in view points.
const GRIP_SLOP: f32 = 4.0;

/// The sidebar: its width (200 to 600 pt) and whether `cmd-b` hid it. Kept in the shared state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sidebar {
    pub width: f32,
    pub hidden: bool,
    /// The width a divider drag started from.
    dragged_from: Option<f32>,
    /// Shown only for the agent-jump bar, to hide again when it goes: never saved.
    for_jump: bool,
}

impl Sidebar {
    /// Its width on screen, `None` while hidden.
    pub fn shown(&self) -> Option<f32> {
        (!self.hidden).then_some(self.width)
    }
}

/// The window's tab area, as laid out, in view points.
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
    /// The panels' font family, when it's installed; Rune's sans otherwise.
    ui_family: Option<&'static str>,
    /// The colors terminals start with.
    pub theme: Theme,
    pub look: Look,
    /// The panels' colors.
    pub chrome: Chrome,
    /// What the files say (`docs/settings.md`).
    pub config: Config,
    sources: Sources,
    watcher: Option<ConfigWatcher>,
    /// The files as they were just before the first load, for the watcher to start from.
    stamp: Option<Stamp>,
    /// `settings.json` texts the zoom keys wrote, oldest first, until the watcher reads the
    /// newest back: one it reads before that is stale and is skipped.
    own_writes: Vec<String>,
    /// Program icons, rasterized per path and size in device pixels.
    images: Option<ImageCache>,
    icons: RefCell<BTreeMap<(String, u32), rune::Image>>,
    pub sidebar: Sidebar,
    /// The app-local shared state (`shared.json` in the home).
    pub shared: Shared,
    /// What the Agents list keeps between builds: times, folds, the held done agent.
    pub agents: Tracker,
    /// The Agents list as last built.
    pub list: agents::List,
    /// Something the list reads changed: build it again at the end of the pump.
    agents_stale: bool,
    /// When the list's times next refresh (app clock).
    agents_refresh_at: f64,
    /// The font at the display scale the last frame drew at.
    face: RefCell<Option<Face>>,
    scale: f32,
    area: Option<Area>,
    /// The peek's terminal area, as laid out, in view points.
    peek_area: Option<Area>,
    /// The window's size in window points, as last reported.
    window_points: (f32, f32),
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
    /// The last drawn frame's tree, in a window the end-to-end harness started: it then serves
    /// the harness requests too (`crate::e2e`).
    tap: Option<Tap>,
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
    /// The open popup's text field: its filter, or the prompt's name.
    pub popup_field: TextEdit,
    /// The first item the open list shows ([`crate::view::list::follow`]).
    pub list_top: usize,
    /// The picker's quiet spaces are unfolded ("earlier").
    pub picker_earlier: bool,
    /// Where the last right-click was, in view points: where a menu opens.
    pub menu_at: Option<(f32, f32)>,
    /// Wheel travel over the open list not yet a whole row, in points.
    list_wheel_travel: f32,
    /// What is wrong with `keys.json` now, for the settings dialog.
    keys_problem: Option<String>,
    /// Shows a folder in Finder: set for a live window only, so tests and scenes never open
    /// one ([`State::revealed`] records what was asked).
    pub reveal: Option<fn(&std::path::Path)>,
    /// The folders the settings dialog asked to show.
    pub revealed: Vec<std::path::PathBuf>,
    /// Something only the view shows changed (a toast, the prefix indicator): draw a frame.
    redraw: bool,
    /// Which Option keys act as Alt in terminals: both, as Eli's Ghostty has it
    /// (`macos-option-as-alt = true`).
    pub option_as_alt: OptionAsAlt,
    /// Keys whose press went to a terminal, so only their releases follow.
    keys_down: Vec<(String, terminal::Key)>,
    /// The mouse button held over a terminal.
    press: Option<Press>,
    /// The pane whose top strip the pointer is over: its grip shows.
    pub grip_hover: Option<String>,
    pub grip_drag: Option<GripDrag>,
    divider_drag: Option<DividerDrag>,
    /// What the connected core said in its `hello`: its version, for the Machines dialog.
    pub hello: Option<Box<herdr_proto::Hello>>,
    /// The machine dialogs' calls, failures and ssh hosts.
    pub machines: crate::machines::Machines,
    /// What a find field edits when its terminal is gone (Rune asks for a field's text by
    /// key).
    spare_edit: TextEdit,
    /// The pointer over a terminal, in device pixels from its grid's corner: where Cmd
    /// pressed with no motion underlines a link.
    pointer: Option<(String, f32, f32)>,
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

/// What the pane handlers take in place of a pane id for the peek's terminal (no pane id has an
/// `@`).
pub const PEEK: &str = "@peek";

/// The peek's terminal element key: what has the keyboard while the peek does.
pub const PEEK_KEY: &str = "peek-terminal";

/// The element key of a pane, or of the peek's terminal for [`PEEK`].
pub fn surface_key(pane: &str) -> String {
    if pane == PEEK {
        PEEK_KEY.to_owned()
    } else {
        pane_key(pane)
    }
}

/// A tab pill's element key, as [`pane_key`].
pub fn tab_key(id: &str) -> String {
    format!("tab-{}", id.replace('/', ":"))
}

/// The wall clock, in ms since the epoch: the Agents list's times.
pub fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Whether one kind of popup replaced another (both with a field: the help gave way to the
/// prompt, say), whose field is drawn anew.
fn box_changed(before: &model::Mode, now: &model::Mode) -> bool {
    use std::mem::discriminant;
    match (before, now) {
        (model::Mode::Popup(a), model::Mode::Popup(b)) => match (a, b) {
            (PopupState::List(a), PopupState::List(b)) => a.popup != b.popup,
            (a, b) => discriminant(a) != discriminant(b),
        },
        _ => false,
    }
}

/// The person's home folder, for the model's names and new spaces.
fn user_home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

impl State {
    /// A state on `link`, restoring the view saved in `home` and reading its settings. With no
    /// home it has the built-in defaults: Eli's settings, arctic, no font files.
    pub fn new(link: Box<dyn Link>, home: Option<Home>, window: Keeper) -> State {
        let view = home
            .as_ref()
            .and_then(|h| h.read(VIEW))
            .and_then(|text| View::from_json(&text).ok())
            .unwrap_or_default();
        let mut model = Model::with_view(user_home(), view);
        model.set_shell(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()));
        let mut toasts = Toasts::default();
        let mut keys_read = None;
        let mut keys_problem = None;
        if let Some(home) = &home {
            let start = keyfile::load(home.dir());
            model.set_keymap(start.keymap);
            keys_problem = start.problem.clone();
            if let Some(why) = start.problem {
                toasts.push(
                    Toast::new(Level::Error, "keys.json can't be used", why),
                    0.0,
                );
            }
            keys_read = start.text;
        }
        let sources = match &home {
            Some(home) => Sources::new(Some(home.dir().to_owned())),
            None => Sources::none(),
        };
        // Stamped before the load, so the watcher loads what changes during it.
        let stamp = home.as_ref().map(|_| sources.stamp());
        let config = match &home {
            Some(_) => Config::load(&sources, None),
            None => Config::default(),
        };
        // The shared state and the list's times: the home's, else the defaults.
        let shared = Shared::load(home.as_ref().map(|h| h.dir()));
        let times = shared::load_times(home.as_ref().map(|h| h.dir()));
        let sidebar = Sidebar {
            width: shared.sidebar_px(),
            hidden: shared.sidebar_hidden(),
            dragged_from: None,
            for_jump: false,
        };
        let mut s = State {
            model,
            link,
            started: false,
            waker: None,
            status: Status::Offline,
            viewers: BTreeMap::new(),
            panes: Vec::new(),
            font: Font::jetbrains_mono(TERMINAL_POINTS),
            ui_family: None,
            theme: Theme::default(),
            look: Look::default(),
            chrome: config.theme.chrome(),
            config: Config::default(),
            sources,
            watcher: None,
            stamp,
            own_writes: Vec::new(),
            images: None,
            icons: RefCell::new(BTreeMap::new()),
            sidebar,
            shared,
            agents: Tracker::with_times(times),
            list: agents::List::default(),
            agents_stale: true,
            agents_refresh_at: 0.0,
            face: RefCell::new(None),
            scale: 2.0,
            area: None,
            peek_area: None,
            window_points: crate::SIZE,
            window_active: true,
            in_use: None,
            resized_at: f64::NEG_INFINITY,
            synchronous: false,
            home,
            control: None,
            tap: None,
            window,
            ui: Ui::default(),
            tick_at: None,
            now: 0.0,
            toasts,
            keys: None,
            keys_read,
            keys_every: keyfile::EVERY,
            popup_field: TextEdit::default(),
            list_top: 0,
            picker_earlier: false,
            menu_at: None,
            hello: None,
            machines: Default::default(),
            list_wheel_travel: 0.0,
            keys_problem: None,
            reveal: None,
            revealed: Vec::new(),
            redraw: false,
            option_as_alt: OptionAsAlt::Both,
            keys_down: Vec::new(),
            press: None,
            grip_hover: None,
            grip_drag: None,
            divider_drag: None,
            spare_edit: TextEdit::default(),
            pointer: None,
        };
        s.keys_problem = keys_problem;
        s.configure_all(config);
        s.window_sized(crate::SIZE.0, crate::SIZE.1);
        s
    }

    /// The window's size in view points (window points over the zoom).
    pub fn view_size(&self) -> (f32, f32) {
        let (w, h) = self.window_points;
        let k = self.zoom();
        (w / k, h / k)
    }

    /// The terminals' background: what text fields sit on, as Herdr app's do.
    pub fn background(&self) -> rune::Color {
        self.theme
            .background
            .map_or(self.chrome.canvas, crate::view::terminal::look::color)
    }

    /// herdr-frontend's home: where its files are, or would be with none chosen.
    pub fn home_dir(&self) -> std::path::PathBuf {
        match &self.home {
            Some(home) => home.dir().to_owned(),
            None => crate::control::home(),
        }
    }

    /// What is wrong with the home's files now: the look's and the key file's.
    pub fn file_problems(&self) -> Vec<String> {
        let mut out = self.config.problems.clone();
        out.extend(self.keys_problem.iter().map(|p| format!("keys.json: {p}")));
        out
    }

    /// Runs with `settings` and the built-in theme and programs, as if a file said so: scenes and
    /// tests ([`Settings::fixture`] for the renderer's fixtures and geometry tests).
    pub fn with_settings(&mut self, settings: Settings) {
        let config = Config::from_settings(settings);
        self.configure(config);
    }

    /// The zoom the window draws at.
    pub fn zoom(&self) -> f32 {
        self.config.settings.zoom
    }

    /// Takes a new config: the font, theme and zoom it changes apply at once, to terminals
    /// already on screen too.
    pub fn configure(&mut self, config: Config) {
        let old = &self.config;
        let font_changed = old.font != config.font
            || old.settings.font != config.settings.font
            || old.settings.font_size != config.settings.font_size
            || old.settings.terminal_line_height != config.settings.terminal_line_height
            || old.settings.terminal_cell_width != config.settings.terminal_cell_width;
        let ui_changed =
            old.ui_font != config.ui_font || old.settings.ui_font != config.settings.ui_font;
        let look_changed = old.theme != config.theme
            || old.settings.minimum_contrast != config.settings.minimum_contrast;
        let zoom_changed = old.settings.zoom != config.settings.zoom;
        self.apply_config(config, font_changed, ui_changed, look_changed);
        if zoom_changed {
            self.zoomed();
        }
    }

    /// Takes everything from `config`, whatever was there before.
    fn configure_all(&mut self, config: Config) {
        self.apply_config(config, true, true, true);
    }

    fn apply_config(&mut self, config: Config, font: bool, ui: bool, look: bool) {
        let s = &config.settings;
        if font {
            let base = match &config.font {
                Some(family) => Font::from_family(family, s.font_size).unwrap_or_else(|e| {
                    eprintln!("herdr-frontend: font {:?}: {e}", s.font);
                    Font::jetbrains_mono(s.font_size)
                }),
                None => Font::jetbrains_mono(s.font_size),
            };
            self.font = base.adjusted(s.terminal_line_height, s.terminal_cell_width);
            *self.face.borrow_mut() = None;
        }
        if ui {
            self.ui_family = match &config.ui_font {
                Some(family) => Font::from_family(family, UI_FONT_SIZE)
                    .ok()
                    .map(|f| f.family()),
                None if s
                    .ui_font
                    .eq_ignore_ascii_case(crate::view::terminal::font::JETBRAINS_MONO_FAMILY) =>
                {
                    Some(Font::jetbrains_mono(UI_FONT_SIZE).family())
                }
                None => None,
            };
        }
        // Icon files may have new bytes under the same paths.
        self.icons.borrow_mut().clear();
        if look {
            self.theme = config.theme.terminal();
            self.look = config.theme.look(s.minimum_contrast);
            self.chrome = config.theme.chrome();
            for viewer in self.viewers.values() {
                viewer.terminal.set_theme(self.theme.clone());
            }
        }
        self.config = config;
    }

    /// The panels' text at `size` points (before zoom).
    pub fn ui_font(&self, size: f32, weight: f32) -> TextStyle {
        let style = TextStyle::new(size, weight);
        match self.ui_family {
            Some(family) => style.family(Family::Named(family)),
            None => style,
        }
    }

    /// The zoom keys: `steps` tenths in or out, or back to 1 with 0. The new zoom is written to
    /// `settings.json`, keeping the file's other keys.
    pub fn zoom_by(&mut self, steps: i32) {
        let zoom = settings::next_zoom(self.zoom(), steps);
        if zoom == self.zoom() {
            return;
        }
        self.config.settings.zoom = zoom;
        self.zoomed();
        let Some(home) = &self.home else { return };
        match settings::with_zoom(self.config.settings_text.as_deref(), zoom) {
            Some(text) => {
                home.save(SETTINGS, text.clone());
                self.config.settings_text = Some(text.clone());
                self.own_writes.push(text);
            }
            None => {
                let why = "settings.json isn't a JSON object, so the zoom isn't saved".to_owned();
                if !self.config.problems.contains(&why) {
                    self.config.problems.push(why);
                }
            }
        }
    }

    /// The zoom changed. Once laid out, the tab area's next layout brings its new size in the
    /// new points; before that, the first frame's area comes from the window's size.
    fn zoomed(&mut self) {
        if self.area.is_none() {
            let (w, h) = self.window_points;
            self.window_sized_points(w, h, self.zoom());
        }
    }

    /// A config the watcher read. One holding a zoom write of ours that a newer one follows is
    /// stale: the newer one comes next.
    fn reloaded(&mut self, config: Config) {
        if let Some(text) = config.settings_text.as_ref()
            && let Some(at) = self.own_writes.iter().position(|w| w == text)
        {
            if at + 1 < self.own_writes.len() {
                return;
            }
            self.own_writes.clear();
        } else {
            self.own_writes.clear();
        }
        self.configure(config);
    }

    /// What only the window knows, for `status` ([`crate::control::Control::drain`]): the zoom,
    /// the font sizes and what's wrong with the files.
    pub fn window_status(&self) -> Map<String, Value> {
        let mut status = Map::new();
        // As the file has them (1.1, not the f32's 1.100000023841858).
        let short = |v: f32| (f64::from(v) * 1e4).round() / 1e4;
        status.insert("zoom".into(), json!(short(self.zoom())));
        status.insert(
            "font_size".into(),
            json!(short(self.config.settings.font_size)),
        );
        status.insert("ui_font_size".into(), json!(UI_FONT_SIZE));
        status.insert("errors".into(), json!(self.config.problems));
        status
    }

    /// The image a program icon draws, rasterized for `size` points at `scale`. `None` before
    /// setup, which makes the image cache.
    pub fn icon_image(&self, path: &str, size: f32, scale: f32) -> Option<rune::Image> {
        let images = self.images.as_ref()?;
        let bytes = self.config.programs.icon(path)?;
        let px = (size * scale).ceil() as u32;
        let image = self
            .icons
            .borrow_mut()
            .entry((path.to_owned(), px))
            .or_insert_with(|| images.svg(bytes.clone(), size, scale))
            .clone();
        if self.synchronous {
            let _ = image.wait();
        }
        Some(image)
    }

    /// The window's size, in window points, before its first layout: the panes' first frame is
    /// laid out for the tab area it leaves (Rune reports where elements are only after a frame).
    pub fn window_sized(&mut self, w: f32, h: f32) {
        self.window_points = (w, h);
        if self.area.is_some() {
            return;
        }
        self.window_sized_points(w, h, self.zoom());
    }

    /// The tab area a `w` × `h` window leaves at zoom `k`: the view lays out in `1 / k` as many
    /// points.
    fn window_sized_points(&mut self, w: f32, h: f32, k: f32) {
        let area = model::Rect::new(
            0.0,
            0.0,
            (w / k - self.sidebar.shown().unwrap_or(0.0)).max(0.0),
            (h / k - crate::view::shell::BAR).max(0.0),
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
        self.agents_stale = true;
        let handled = effects.handled;
        // A pick runs what it leads to as steps of its own, after this step's other effects:
        // otherwise this step's focus move (back to the pane, as the list closes) would land
        // after, and undo, the focus of a popup the pick opens.
        let mut picks = Vec::new();
        for effect in effects {
            match effect {
                Effect::Pick { .. } => picks.push(effect),
                other => self.carry_out(other),
            }
        }
        if *self.model.mode() != before {
            self.redraw = true;
            self.mode_changed(&before);
        }
        for pick in picks {
            self.carry_out(pick);
        }
        // The world may have changed under an open list (a tab made or closed elsewhere): its
        // rows are counted again, so the cursor and clicks reach exactly what it shows.
        self.relist(false);
        handled
    }

    /// The keyboard went to a new mode. A popup or copy mode the window doesn't draw yet would
    /// hold every key unseen, so it closes at once with a "not yet" toast. A list popup that
    /// opens starts with an empty field; each change tells the model its rows, and on opening
    /// or a new query where its cursor starts; the window follows the cursor. A prompt opens
    /// with its name in the field, selected. A hidden sidebar shows while the agent-jump bar is
    /// open, for its labels, and hides again when it closes (the mod's `layout.show` and
    /// `layout.hide`); that isn't saved.
    fn mode_changed(&mut self, before: &model::Mode) {
        let mode = self.model.mode().clone();
        // One popup replacing another in a step (a menu's "Rename" opening the prompt) is no
        // focus change to the model, but the keyboard must move to the new one's field or box.
        if let (model::Mode::Popup(_), model::Mode::Popup(_)) = (before, &mode) {
            let key = crate::view::popup::focus_key(&mode);
            if crate::view::popup::focus_key(before) != key || box_changed(before, &mode) {
                self.ui.focus = Some(Focus::Popup);
            }
        }
        let jumping = |m: &model::Mode| matches!(m, model::Mode::Popup(PopupState::Jump(_)));
        if jumping(before) && !jumping(&mode) && self.sidebar.for_jump {
            self.sidebar.for_jump = false;
            self.sidebar.hidden = true;
        }
        match &mode {
            model::Mode::Popup(PopupState::List(list)) if crate::view::popup::drawn(list.popup) => {
                let was = match before {
                    model::Mode::Popup(PopupState::List(b)) if b.popup == list.popup => Some(b),
                    _ => None,
                };
                if was.is_none() {
                    self.popup_field = TextEdit::default();
                    self.list_top = 0;
                    self.list_wheel_travel = 0.0;
                    self.picker_earlier = false;
                    if list.popup == Popup::MachineAdd {
                        // Each opening starts fresh, as the mod does: a target that needed a
                        // terminal can be tried again once that's done.
                        self.machines.failed.clear();
                        let home = std::path::PathBuf::from(self.model.home());
                        self.machines.read_hosts(&home, self.waker.clone());
                        if self.synchronous {
                            self.machines.wait_hosts();
                        }
                    }
                }
                let new_query = was.is_none_or(|b| b.query != list.query);
                self.relist(new_query);
            }
            model::Mode::Popup(PopupState::Prompt(prompt)) => {
                if !matches!(before, model::Mode::Popup(PopupState::Prompt(_))) {
                    let mut field = TextEdit::new(prompt.value.clone());
                    field.set_selection(0, prompt.value.len());
                    self.popup_field = field;
                }
            }
            // The sort menu hangs under the sidebar's sort button: with the sidebar hidden there
            // is nothing to open it under. It opens on the current sort.
            model::Mode::Popup(PopupState::List(list)) if list.popup == Popup::SortMenu => {
                if self.sidebar.hidden {
                    self.step(self.now, Action::Popup(PopupAction::Close));
                } else if list.rows != Sort::ALL.len() {
                    self.step(self.now, Event::PopupRows(Sort::ALL.len()));
                    let at = Sort::ALL.iter().position(|s| *s == self.shared.sort());
                    for _ in 0..at.unwrap_or(0) {
                        self.step(self.now, Action::Popup(PopupAction::Down));
                    }
                }
            }
            model::Mode::Popup(PopupState::Confirm(_) | PopupState::Menu(_)) => {}
            model::Mode::Popup(PopupState::Jump(_)) => {
                if !jumping(before) && self.sidebar.hidden {
                    self.sidebar.hidden = false;
                    self.sidebar.for_jump = true;
                }
            }
            model::Mode::Popup(PopupState::List(list)) => {
                self.not_yet(&Action::Open(list.popup));
                self.step(self.now, Action::Popup(PopupAction::Close));
            }
            // Its bar draws (`view::copy`); the terminal's worker runs it.
            model::Mode::Copy(_) => {}
            model::Mode::Normal | model::Mode::Prefix { .. } => {}
        }
    }

    /// Tells the model what the open list shows now, and moves the window to its cursor.
    /// `start`: the query is new (or the list just opened), so the cursor goes where it starts.
    /// With no drawn list open, nothing.
    fn relist(&mut self, start: bool) {
        let model::Mode::Popup(PopupState::List(list)) = self.model.mode().clone() else {
            return;
        };
        let Some(listing) = crate::view::popup::listing(self, &list) else {
            return;
        };
        let rows = listing.rows();
        if rows != list.rows {
            self.step(self.now, Event::PopupRows(rows));
        }
        if start {
            let cursor = match self.model.mode() {
                model::Mode::Popup(PopupState::List(l)) => l.cursor,
                _ => return,
            };
            if listing.start != cursor {
                self.step(self.now, Event::PopupCursor(listing.start));
            }
        }
        if let model::Mode::Popup(PopupState::List(l)) = self.model.mode() {
            self.list_top =
                crate::view::list::follow(&listing.picks, l.cursor, self.list_top, listing.shown);
        }
    }

    /// A list row was picked (`Effect::Pick`): run what it stands for. Enter already closed the
    /// list; Ctrl+D closes it here first.
    fn picked(&mut self, popup: Popup, how: model::Pick, cursor: usize, query: String) {
        use crate::view::{machines, palette, picker, pr_finder, settings, space_picker};
        let mut list = model::mode::ListPopup::new(popup);
        list.query = query.clone();
        let now = self.now;
        match (popup, how) {
            (Popup::Palette, model::Pick::Choose) => {
                if let Some(row) = palette::rows(self, &list).into_iter().nth(cursor) {
                    self.step(now, Action::FromPalette(Box::new(row.action)));
                }
            }
            (Popup::SpacePicker, model::Pick::Choose) => {
                if let Some(row) = space_picker::rows(self, &list).into_iter().nth(cursor) {
                    self.step(now, Action::ShowSpace(row.id));
                }
            }
            (Popup::NewSpace, model::Pick::Choose) => {
                if let Some(action) = model::popups::new_space_pick(&self.model, &query, cursor) {
                    self.step(now, action);
                }
            }
            (Popup::PrFinder, model::Pick::Choose) => {
                if let Some(row) = pr_finder::rows(self, &list).into_iter().nth(cursor) {
                    self.step(now, Action::FocusAgent(row.card.address));
                }
            }
            (Popup::Picker, how) => {
                let rows = picker::rows(self, &list);
                let Some(row) = model::popups::pickable(&rows)
                    .get(cursor)
                    .map(|r| (*r).clone())
                else {
                    return;
                };
                match how {
                    model::Pick::Choose => match row.action() {
                        Some(action) => {
                            self.step(now, action);
                        }
                        // "earlier": unfold or fold, and stay on it.
                        None => self.toggle_earlier(cursor, query),
                    },
                    model::Pick::CloseItem => {
                        if let Some(action) = row.close_item() {
                            self.step(now, Action::Popup(PopupAction::Close));
                            self.step(now, action);
                        }
                    }
                    model::Pick::Toggle => {}
                }
            }
            (Popup::AgentMachines, how) => {
                let rows = machines::filter_rows(self);
                let hidden = model::machines::hidden_after(
                    &rows,
                    &self.shared.hidden_machines(),
                    cursor,
                    how == model::Pick::Choose,
                );
                self.set_hidden_machines(hidden);
            }
            (Popup::Machines, model::Pick::Choose) => {
                match machines::machines_rows(self).into_iter().nth(cursor) {
                    Some(machines::MachinesRow::Machine(view)) => {
                        // Its menu opens over the dialog's middle, where the row was.
                        let (vw, vh) = self.view_size();
                        self.menu_at = Some((vw / 2.0 - 150.0, vh / 3.0));
                        let name = view.machine.name.clone();
                        self.machines.picked = Some(name.clone());
                        self.step(
                            now,
                            Action::Machine(model::machines::MachineAction::Menu(name)),
                        );
                    }
                    Some(machines::MachinesRow::Add) => {
                        self.step(now, Action::Open(Popup::MachineAdd));
                    }
                    Some(machines::MachinesRow::ThisComputer) | None => {}
                }
            }
            (Popup::MachineAdd, model::Pick::Choose) => {
                let Some(row) = machines::add_rows(self, &list).into_iter().nth(cursor) else {
                    return;
                };
                if self.machines.busy.contains_key(&row.target) {
                    return;
                }
                let copy = self
                    .machines
                    .failed
                    .get(&row.target)
                    .and_then(|f| f.command.clone());
                let action = match copy {
                    Some(command) => Action::CopyText(command),
                    None => Action::Machine(model::machines::MachineAction::Add(row.target)),
                };
                self.step(now, action);
            }
            (Popup::Settings, model::Pick::Choose) => {
                match settings::rows(self).into_iter().nth(cursor) {
                    Some(settings::Row::File { path, .. }) => {
                        self.step(now, Action::EditFile(path.to_string_lossy().into_owned()));
                    }
                    Some(settings::Row::Machines) => {
                        self.step(now, Action::Open(Popup::Machines));
                    }
                    Some(settings::Row::Reveal(dir)) => {
                        if let Some(reveal) = self.reveal {
                            reveal(&dir);
                        }
                        self.revealed.push(dir);
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// The picker's "earlier" row: it opens again with its quiet spaces unfolded (or folded),
    /// its query kept and its cursor still on that row.
    fn toggle_earlier(&mut self, cursor: usize, query: String) {
        let earlier = !self.picker_earlier;
        let field = self.popup_field.clone();
        self.step(self.now, Action::Open(Popup::Picker));
        self.picker_earlier = earlier;
        self.popup_field = field;
        if !query.is_empty() {
            self.step(self.now, Action::Popup(PopupAction::Query(query)));
        }
        self.relist(false);
        self.step(self.now, Event::PopupCursor(cursor));
        self.relist(false);
    }

    /// A press on row `i` of the open list or menu: the cursor goes there, then it is chosen.
    pub fn pick_row(&mut self, i: usize, tx: &mut Tx) {
        // The row drawn is the row picked, even if the rows changed since the model last heard.
        self.relist(false);
        self.step(tx.clock(), Event::PopupCursor(i));
        self.act(Action::Popup(PopupAction::Choose), tx);
    }

    /// A press on row `i` of the machine filter: the cursor goes there, and its machine shows or
    /// hides; the filter stays open.
    pub fn toggle_row(&mut self, i: usize, tx: &mut Tx) {
        self.relist(false);
        self.step(tx.clock(), Event::PopupCursor(i));
        self.act(Action::Popup(PopupAction::Toggle), tx);
    }

    /// The machine filter's choice: shared `dotfiles.agents.hidden-machines`, `null` when
    /// nothing is hidden, as the mod stores it.
    pub fn set_hidden_machines(&mut self, hidden: std::collections::BTreeSet<String>) {
        let value = if hidden.is_empty() {
            Value::Null
        } else {
            json!(hidden)
        };
        self.share(agents::HIDDEN_MACHINES_KEY, value);
        self.redraw = true;
    }

    /// A machine call answered: a toast says how it went; a failed add explains itself in the
    /// add dialog instead, and a working one empties its field.
    fn machine_answered(
        &mut self,
        call: model::machines::MachineCall,
        result: Result<(), herdr_proto::RpcError>,
    ) {
        use model::machines::MachineCall;
        self.machines.busy.remove(call.machine());
        self.redraw = true;
        match (&call, result) {
            (MachineCall::Add(target), Err(error)) => {
                let failure = model::machines::add_failure(target, &error);
                self.machines.failed.insert(target.clone(), failure);
            }
            (_, Err(error)) => {
                let verb = match &call {
                    MachineCall::Add(_) => "add",
                    MachineCall::Connect(_) => "connect to",
                    MachineCall::Rename { .. } => "rename",
                    MachineCall::Remove(_) => "remove",
                };
                self.toast(Toast::new(
                    Level::Error,
                    format!("Couldn't {verb} {}", call.machine()),
                    error.message,
                ));
            }
            (MachineCall::Add(target), Ok(())) => {
                self.toast(Toast::new(Level::Success, call.done(), ""));
                let adding = matches!(self.model.mode(),
                    model::Mode::Popup(PopupState::List(l)) if l.popup == Popup::MachineAdd);
                if adding && self.popup_field.text.trim() == target {
                    self.popup_field = TextEdit::default();
                    self.step(self.now, Action::Popup(PopupAction::Query(String::new())));
                }
            }
            (MachineCall::Connect(_), Ok(())) => {}
            (MachineCall::Rename { machine, to }, Ok(())) => {
                if self.machines.picked.as_ref() == Some(machine) {
                    self.machines.picked = Some(to.clone());
                }
                self.toast(Toast::new(Level::Success, call.done(), ""));
            }
            (_, Ok(())) => self.toast(Toast::new(Level::Success, call.done(), "")),
        }
        self.relist(false);
    }

    /// The wheel over the open list: each 32 points move the cursor a row, so the window follows.
    pub fn list_wheel(&mut self, dy: f32, tx: &mut Tx) -> bool {
        const ROW: f32 = 32.0;
        self.list_wheel_travel += dy;
        let rows = (self.list_wheel_travel / ROW).trunc();
        if rows == 0.0 {
            return true;
        }
        self.list_wheel_travel -= rows * ROW;
        // Content moving down (fingers down) shows rows above.
        let (up, n) = (rows > 0.0, rows.abs() as usize);
        let step = if up {
            PopupAction::Up
        } else {
            PopupAction::Down
        };
        for _ in 0..n {
            self.step(tx.clock(), Action::Popup(step.clone()));
        }
        self.pump(tx.clock());
        self.apply_ui(tx);
        true
    }

    /// Shows a toast. One about an agent on a machine the Agents list hides closes at once, so
    /// that agent stays out of view (`cfg/mods/eli-toast/filter.js`).
    pub fn toast(&mut self, toast: Toast) {
        if toast::about_hidden_machine(&toast, &self.shared.hidden_machines()) {
            return;
        }
        let due = self.toasts.push(toast, self.now);
        self.wake_at(due);
        self.redraw = true;
    }

    /// A click on a toast: it goes, then its action runs.
    pub fn click_toast(&mut self, id: u64, tx: &mut Tx) {
        let Some(toast) = self.toasts.dismiss(id) else {
            return;
        };
        self.redraw = true;
        match toast.action {
            Some(action) => self.act(action, tx),
            None => {
                self.pump(tx.clock());
                self.apply_ui(tx);
            }
        }
    }

    /// A toast's Copy button: its command goes to the clipboard and the toast stays.
    pub fn copy_from_toast(&mut self, id: u64, tx: &mut Tx) {
        if let Some(text) = self.toasts.copied(id) {
            self.ui.copy = Some(text);
            self.redraw = true;
            self.apply_ui(tx);
        }
    }

    /// An agent's alert: its toast, unless the person is looking at it.
    fn alert(&mut self, raise: &herdr_shared::Raise) {
        let programs = &self.config.programs;
        let toast = self
            .model
            .alert_toast(raise, |app| programs.of(Some(app), &[]).name);
        if let Some(toast) = toast {
            self.toast(toast);
        }
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
            Effect::Machine(call) => {
                let key = call.machine().to_owned();
                if self.machines.busy.contains_key(&key) {
                    return;
                }
                if let model::machines::MachineCall::Add(target) = &call {
                    self.machines.failed.remove(target);
                }
                self.machines.busy.insert(key, call.clone());
                self.link.machine(call);
                self.redraw = true;
            }
            Effect::Clipboard(text) => {
                self.ui.copy = Some(text.clone());
                self.toast(Toast::new(Level::Success, "Copied", text));
            }
            Effect::Open(target) => {
                self.ui.open.push(target);
                self.redraw = true;
            }
            Effect::Terminal { address, command } => self.terminal_command(&address, command),
            // A `do` call's answer that came in a later step: `space.new`'s, once its terminal
            // runs.
            Effect::Answer { id, reply } => {
                if let Some(control) = &mut self.control {
                    control.answer(id, reply);
                }
            }
            Effect::Pick {
                popup: Popup::SortMenu,
                how: Pick::Choose,
                cursor,
                ..
            } => {
                if let Some(sort) = Sort::ALL.get(cursor) {
                    self.set_sort(*sort);
                }
            }
            Effect::Pick {
                popup,
                how,
                cursor,
                query,
            } => self.picked(popup, how, cursor, query),
            Effect::Alert(raise) => self.alert(&raise),
        }
    }

    /// An app command: the window's own are Rune's, the sidebar and the zoom are this state's,
    /// the escape hatch resets the key file, and switching layouts stays on `eli`; hiding is
    /// the macOS menu's.
    fn app_command(&mut self, command: AppCommand) {
        match command {
            AppCommand::Quit | AppCommand::CloseWindow | AppCommand::Maximize => {
                self.ui.window.push(command)
            }
            AppCommand::ToggleSidebar => {
                self.sidebar.hidden = !self.sidebar.hidden;
                // Toggled by hand while the jump bar showed it: it stays as toggled.
                self.sidebar.for_jump = false;
                let sort_menu = self.model.mode().popup().and_then(PopupState::popup);
                if self.sidebar.hidden && sort_menu == Some(Popup::SortMenu) {
                    self.step(self.now, Action::Popup(PopupAction::Close));
                }
                self.share(shared::SIDEBAR_HIDDEN_KEY, json!(self.sidebar.hidden));
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
            AppCommand::ZoomIn => self.zoom_by(1),
            AppCommand::ZoomOut => self.zoom_by(-1),
            AppCommand::ZoomReset => self.zoom_by(0),
            // macOS's menu hides the app on ⌘H and ⌥⌘H before the window sees them.
            AppCommand::Hide | AppCommand::HideOthers => self.not_yet(&Action::App(command)),
            // `eli` is the only layout, so the next one is `eli` again (as `do layout.switch`).
            AppCommand::LayoutSwitch => {}
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
                    self.keys_problem = None;
                    self.redraw = true;
                }
                Loaded::Bad(why) => {
                    self.keys_problem = Some(why.clone());
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

    /// A `terminal` key binding's action, or another command for a terminal's view.
    fn terminal_command(&mut self, address: &str, command: TerminalCommand) {
        if command == TerminalCommand::Action(TerminalAction::Search) {
            self.open_find(address);
            return;
        }
        let Some(viewer) = self.viewers.get(address) else {
            return;
        };
        match command {
            // `send:` bindings: the text as it is (`alt-left` is `ESC b`).
            TerminalCommand::Action(TerminalAction::Send(text)) => {
                viewer.terminal.input(text.into_bytes())
            }
            TerminalCommand::Action(TerminalAction::Paste) => self.ui.paste.push(address.into()),
            // The selection, or with none, Cmd+C itself for the program.
            TerminalCommand::Action(TerminalAction::Copy) => {
                viewer.terminal.copy(Some(terminal::KeyEvent {
                    unshifted: Some('c'),
                    ..terminal::KeyEvent::press(GHOSTTY_KEY_C, terminal::Mods::SUPER)
                }))
            }
            // The clipboard's image, once it's read.
            TerminalCommand::Action(TerminalAction::PasteImage) => {
                self.ui.paste_image.push(address.into())
            }
            TerminalCommand::Action(TerminalAction::SelectAll) => viewer.terminal.select_all(),
            TerminalCommand::Action(TerminalAction::Search) => {}
            // Copy mode runs on the terminal's own buffer, on its worker; a yank comes back as
            // `Output::Selection`, for the clipboard.
            TerminalCommand::CopyStart => viewer.terminal.copy_mode_start(),
            TerminalCommand::Copy(command) => viewer.terminal.copy_mode(command),
            TerminalCommand::CopyEnd => viewer.terminal.copy_mode_end(),
        }
    }

    /// The surface terminal `address` is in front on, as the pane handlers name it: [`PEEK`]
    /// while it's peeked, else its pane.
    fn surface_of(&self, address: &str) -> Option<String> {
        if self.model.peeked() == Some(address) {
            Some(PEEK.to_owned())
        } else {
            self.model.world().pane_of(address)
        }
    }

    /// `cmd-f`: the terminal's find bar opens with the keyboard in its field, over the peek
    /// while it's peeked, else over its pane. Its query is the one it had, selected on reopening
    /// so typing replaces it, and searched again.
    fn open_find(&mut self, address: &str) {
        let Some(pane) = self.surface_of(address) else {
            return;
        };
        let Some(viewer) = self.viewers.get_mut(address) else {
            return;
        };
        let bar = viewer.find.get_or_insert_with(FindBar::default);
        if !bar.open {
            bar.open = true;
            bar.edit.anchor = 0;
            bar.edit.focus = bar.edit.text.len();
        }
        viewer.terminal.find(bar.edit.text.clone());
        self.ui.focus_key = Some(crate::view::find::field_key(&pane));
        self.redraw = true;
    }

    /// The find field's text changed: the terminal searches for it.
    pub fn find_typed(&mut self, address: &str, tx: &mut Tx) {
        if let Some(viewer) = self.viewers.get(address)
            && let Some(bar) = &viewer.find
        {
            viewer.terminal.find(bar.edit.text.clone());
        }
        self.after_input(tx);
    }

    /// Puts `query` in a terminal's find bar, as if typed, and searches for it: scenes.
    pub fn search(&mut self, address: &str, query: &str) {
        if let Some(viewer) = self.viewers.get_mut(address) {
            let bar = viewer.find.get_or_insert_with(FindBar::default);
            bar.edit = TextEdit::new(query);
            viewer.terminal.find(query);
        }
        self.redraw = true;
    }

    /// Enter (up into history) or Shift+Enter (back down) in the find bar, or its arrows. The
    /// keyboard stays in the field.
    pub fn find_next(&mut self, address: &str, back: bool, tx: &mut Tx) {
        if let Some(viewer) = self.viewers.get(address)
            && viewer
                .find
                .as_ref()
                .is_some_and(|b| b.open && !b.edit.text.is_empty())
        {
            viewer.terminal.find_next(back);
        }
        self.after_input(tx);
    }

    /// Escape in the find field, or its ×: the bar and its hits go, and the keyboard goes back
    /// to the terminal. The query stays for the next `cmd-f`.
    pub fn close_find(&mut self, address: &str, tx: &mut Tx) {
        let Some(viewer) = self.viewers.get_mut(address) else {
            return;
        };
        if let Some(bar) = &mut viewer.find {
            bar.open = false;
        }
        viewer.terminal.find("");
        if let Some(pane) = self.surface_of(address) {
            self.ui.focus_key = Some(surface_key(&pane));
        }
        self.redraw = true;
        self.after_input(tx);
    }

    /// The find field's text, for Rune to edit.
    pub fn find_edit(&mut self, address: &str) -> &mut TextEdit {
        match self.viewers.get_mut(address).and_then(|v| v.find.as_mut()) {
            Some(bar) => &mut bar.edit,
            None => &mut self.spare_edit,
        }
    }

    /// A URL or local path for the system's opener: Rune opens `http:`, `https:` and `mailto:`
    /// links and local paths; other schemes it refuses, and a toast says so. Headless runs and
    /// tests open nothing (Rune keeps the requests).
    fn open_externally(&mut self, target: &str, tx: &mut Tx) {
        let refused = if model::links::has_scheme(target) {
            tx.open_url(target).map_err(|e| e.to_string())
        } else {
            tx.open_path(target).map_err(|e| e.to_string())
        };
        if let Err(why) = refused {
            self.toast(Toast::new(Level::Warning, "Can't open the link", why));
        }
    }

    /// Hands the focus, the window commands, the clipboard and the wakes asked for to Rune.
    fn apply_ui(&mut self, tx: &mut Tx) {
        match self.ui.focus.take() {
            Some(Focus::Pane(pane)) => tx.focus_when_shown(pane_key(&pane), false),
            Some(Focus::Popup) => {
                tx.focus_when_shown(crate::view::popup::focus_key(self.model.mode()), false)
            }
            Some(Focus::Peek) => tx.focus_when_shown(PEEK_KEY, false),
            Some(Focus::None) | None => {}
        }
        if let Some(key) = self.ui.focus_key.take() {
            tx.focus_when_shown(key, false);
        }
        for target in std::mem::take(&mut self.ui.open) {
            self.open_externally(&target, tx);
        }
        for command in std::mem::take(&mut self.ui.window) {
            match command {
                AppCommand::Quit => tx.quit(),
                AppCommand::CloseWindow => tx.close_window(),
                AppCommand::Maximize => tx.toggle_maximized(),
                _ => {}
            }
        }
        if let Some(text) = self.ui.copy.take() {
            tx.copy(text);
        }
        for address in std::mem::take(&mut self.ui.paste_image) {
            tx.read_clipboard(move |s: &mut State, content, tx| {
                s.paste_image(&address, content.image());
                s.after_input(tx);
            });
        }
        for address in std::mem::take(&mut self.ui.paste) {
            tx.read_clipboard(move |s: &mut State, content, tx| {
                s.paste_content(&address, content);
                s.after_input(tx);
            });
        }
        if let Some(at) = self.ui.wake.take() {
            tx.wake_at(at);
        }
        for key in self.ui.keys.drain(..) {
            tx.send_key(key);
        }
    }

    /// Drains the core and the terminals and keeps the viewers and their sizes right. True when
    /// something on screen changed.
    pub fn pump(&mut self, now: f64) -> bool {
        self.now = self.now.max(now);
        let now = self.now;
        let mut changed = false;
        if let Some(config) = self.watcher.as_ref().and_then(ConfigWatcher::latest) {
            self.reloaded(config);
            changed = true;
        }
        if let Some(due) = self.tick_at
            && now >= due
        {
            self.tick_at = None;
            self.step(now, Event::Tick);
        }
        self.hear_keys();
        if self.machines.take_hosts() {
            self.redraw = true;
            self.relist(false);
        }
        if self.toasts.expire(now) {
            self.redraw = true;
        }
        if let Some(due) = self.toasts.next_due() {
            self.wake_at(due);
        }
        // Before the link, so a fake core answers the calls a `do` made in this same pump.
        changed |= self.drain_control(now);
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
        changed |= self.refresh_agents(now);
        changed | std::mem::take(&mut self.redraw)
    }

    /// Builds the Agents list again when something it reads changed or its times are due, and
    /// publishes what changed: the order to the model, the order and space colors to the shared
    /// state, the times to the home. True when the list on screen changed.
    fn refresh_agents(&mut self, now: f64) -> bool {
        if now >= self.agents_refresh_at {
            self.agents_stale = true;
        }
        if !self.agents_stale {
            return false;
        }
        self.agents_stale = false;
        let hidden = self.shared.hidden_machines();
        let colors = self.shared.space_colors();
        let sort_value = self.shared.str(agents::SORT_KEY).map(str::to_owned);
        let programs = &self.config.programs;
        let program = |t: &herdr_proto::Terminal| {
            let p = programs.of_terminal(t);
            agents::Program {
                key: p.key,
                name: p.name,
                agent: p.agent,
            }
        };
        let input = agents::Input {
            sort: self.shared.sort(),
            sort_value: sort_value.as_deref(),
            hidden_machines: &hidden,
            space_colors: &colors,
            now_ms: wall_ms(),
            program: &program,
            peeked: self.model.peeked(),
        };
        let list = self.agents.build(&self.model, &input);
        if let Some(kept) = &list.space_colors {
            self.share(agents::SPACE_COLORS_KEY, json!(kept));
        }
        if self.model.listed() && self.shared.get(agents::ORDER_KEY) != Some(&list.published) {
            self.share(agents::ORDER_KEY, list.published.clone());
        }
        if std::mem::take(&mut self.agents.dirty)
            && let Some(home) = &self.home
        {
            home.save(shared::TIMES, shared::times_json(&self.agents.changed));
        }
        let publish = list.rows_order != self.list.rows_order || list.nav != self.list.nav;
        let changed = list != self.list;
        self.list = list;
        if publish {
            let (rows, order) = (self.list.rows_order.clone(), self.list.nav.clone());
            self.step(self.now, Event::AgentOrder { rows, order });
            // Publishing changes nothing the list reads.
            self.agents_stale = false;
        }
        // The PR finder lists from it: its rows are counted again.
        if changed && self.model.mode().popup().and_then(PopupState::popup) == Some(Popup::PrFinder)
        {
            self.relist(false);
            self.agents_stale = false;
        }
        // The times say minutes: build again in 30 s while the sidebar shows them.
        if !self.sidebar.hidden {
            self.agents_refresh_at = now + agents::REFRESH_EVERY;
            self.wake_at(self.agents_refresh_at);
        } else {
            self.agents_refresh_at = f64::INFINITY;
        }
        changed
    }

    /// Sets a shared key and saves the file when it changed.
    pub fn share(&mut self, key: &str, value: Value) {
        if self.shared.set(key, value) {
            shared::save(self.home.as_ref(), &self.shared);
            self.agents_stale = true;
        }
    }

    /// The sort menu's pick: shared `agents.sort`.
    pub fn set_sort(&mut self, sort: Sort) {
        self.share(agents::SORT_KEY, json!(sort.as_str()));
        self.redraw = true;
    }

    /// Folds or unfolds an agent's sub-agents.
    pub fn fold(&mut self, address: &str, tx: &mut Tx) {
        self.agents.toggle_fold(address);
        self.agents_stale = true;
        self.pump(tx.clock());
        self.apply_ui(tx);
    }

    /// A drag of the sidebar's edge: its width follows the pointer between 200 and 600 pt, and
    /// is kept when the drag ends (put back when it's cancelled).
    pub fn drag_sidebar(&mut self, drag: &rune::Drag) {
        let from = *self.sidebar.dragged_from.get_or_insert(self.sidebar.width);
        match drag.phase {
            rune::Phase::Up if drag.cancelled() => self.sidebar.width = from,
            _ => self.sidebar.width = shared::clamp_sidebar(from + drag.dx),
        }
        if drag.phase == rune::Phase::Up {
            self.sidebar.dragged_from = None;
            let width = self.sidebar.width;
            self.share(shared::SIDEBAR_PX_KEY, json!(width));
        }
        self.redraw = true;
    }

    /// Runs the requests the control socket took (`ls`, `status`, `do`) through the model, and
    /// carries out their effects. True when any came in.
    fn drain_control(&mut self, now: f64) -> bool {
        if self.control.is_none() {
            return false;
        }
        let facts = self.window_status();
        let Some(control) = &mut self.control else {
            return false;
        };
        let before = self.model.mode().clone();
        let Some(fx) = control.drain(&mut self.model, now, &facts) else {
            return false;
        };
        let probes = control.take_probes();
        self.agents_stale = true;
        for effect in fx.list {
            self.carry_out(effect);
        }
        // A `do` can open a popup or copy mode as a key does, and change what a list shows.
        if *self.model.mode() != before {
            self.mode_changed(&before);
        }
        self.relist(false);
        for probe in probes {
            self.probe(probe);
        }
        true
    }

    /// Answers a harness request (`crate::e2e`): keys go out with the next handler's `Tx`.
    fn probe(&mut self, probe: Probe) {
        let cmd = &probe.cmd;
        let reply = if cmd.get("tree").is_some() {
            self.tap
                .as_ref()
                .and_then(Tap::tree)
                .unwrap_or_else(|| json!({ "error": "the window has drawn no frame yet" }))
        } else if cmd.get("window").is_some() {
            json!({ "window": self.window.window_number(), "pid": std::process::id() })
        } else {
            match e2e::key_events(cmd) {
                Ok(keys) => {
                    self.ui.keys.extend(keys);
                    // Delivered after the frame's handler; a waker makes sure one runs.
                    if let Some(waker) = &self.waker {
                        waker();
                    }
                    json!({ "ok": true })
                }
                Err(e) => json!({ "error": e }),
            }
        };
        probe.answer(reply);
    }

    /// Serves the end-to-end harness's requests (`crate::e2e`), with `tap` keeping the drawn
    /// frames' trees. Call before setup, where the socket opens.
    pub fn serve_harness(&mut self, tap: Tap) {
        self.tap = Some(tap);
    }

    /// Serves the control socket in the home, if there is one: the CLI's `ls | do | status`
    /// (`docs/cli.md`). Another window already serving it keeps it; this one goes on without.
    fn serve(&mut self, waker: rune::Waker) {
        let Some(home) = &self.home else { return };
        if self.control.is_some() {
            return;
        }
        match Control::listen(home.dir().join(control::SOCKET), move || waker()) {
            Ok(control) if self.tap.is_some() => self.control = Some(control.with_probes()),
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
            News::Connected(hello) => {
                self.status = Status::Connected;
                for viewer in self.viewers.values_mut() {
                    viewer.attach = Attach::Want { fresh: true };
                }
                self.step(now, Event::Connected);
                if let Some(hello) = &hello {
                    self.step(now, Event::Core(hello.core.clone()));
                }
                self.hello = hello;
            }
            News::Machine { call, result } => self.machine_answered(call, result),
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
            News::Put { address, paths } => self.paste_text(&address, &quote_paths(&paths)),
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
                (viewer.attach, viewer.automatic_input) = match result {
                    Ok(attachment) => (Attach::Attached, attachment.caps.automatic_input),
                    Err(_) => (Attach::Failed, false),
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
        let peeked = self.model.peeked().map(str::to_owned);
        if let Some(address) = &peeked
            && !self.viewers.contains_key(address)
            && let Some(record) = self.model.world().terminal(address)
        {
            let (cols, rows) = (record.cols.max(1), record.rows.max(1));
            let mut viewer = self.viewer(address, cols, rows);
            viewer.key = PEEK_KEY.to_owned();
            self.viewers.insert(address.clone(), viewer);
            changed = true;
        }
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
            // Behind the peek, or while a popup or copy mode holds the keyboard, the focused pane
            // doesn't show focus: its cursor goes hollow, as Herdr app's does under an overlay
            // (copy mode draws its own cursor solid).
            let has_focus = active
                && peeked.is_none()
                && !self.model.mode().holds_keyboard()
                && focused.as_deref() == Some(pane.as_str());
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
            .filter(|a| {
                (world.pane_of(a).is_none() && peeked.as_ref() != Some(*a)) || !world.is_live(a)
            })
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
                option_as_alt: self.option_as_alt,
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
            moved: false,
            focus_sent: false,
            automatic_input: false,
            preedit: None,
            find: None,
            wheel: 0.0,
            hovering_link: false,
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
                // Only a change counts: a size event for a RESIZE just sent may not be in yet.
                // Each one decides afresh, so a late event for an older RESIZE is undone by the
                // event for the grid told last.
                if (vt.0, vt.1) != (viewer.vt.0, viewer.vt.1) {
                    viewer.moved = viewer
                        .told_grid
                        .is_some_and(|g| (g.columns, g.rows) != (vt.0, vt.1));
                }
                viewer.vt = vt;
                viewer.terminal.resize(vt.0, vt.1, vt.2, vt.3);
            }
        }

        let in_use = self
            .window_active
            .then(|| self.model.keyboard_terminal().map(str::to_owned))
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
        let Some(rect) = self.rect_of(&address) else {
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
                viewer.moved = false;
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
        let mut links = Vec::new();
        let mut yanked = Vec::new();
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
            let mut outputs = Vec::new();
            for viewer in self.viewers.values() {
                while let Ok(output) = viewer.terminal.output().try_recv() {
                    match output {
                        Output::Selection(text) => self.ui.copy = Some(text),
                        // Copy mode copied text: to the clipboard, and copy mode ends.
                        Output::Yanked(text) => {
                            self.ui.copy = Some(text);
                            yanked.push(viewer.address.clone());
                        }
                        Output::Link(link) => links.push((viewer.address.clone(), link.target)),
                        Output::Reply(_) | Output::Input(_)
                            if viewer.attach == Attach::Attached =>
                        {
                            outputs.push((viewer.address.clone(), output))
                        }
                        // Titles, bells and the clipboard come with their tickets.
                        _ => {}
                    }
                }
            }
            let sent = outputs.iter().any(|(_, o)| matches!(o, Output::Input(_)));
            for (address, output) in outputs {
                match output {
                    Output::Reply(bytes) => self.send_reply(&address, bytes),
                    Output::Input(bytes) => self.send_data(&address, bytes),
                    _ => {}
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
        for address in yanked {
            self.step(self.now, Event::CopyYanked(address));
            changed = true;
        }
        // A Cmd-click found a link: the model decides what opens it.
        for (address, link) in links {
            if let Some(pane) = self.model.world().pane_of(&address) {
                self.step(self.now, Action::OpenLink { pane, link });
                changed = true;
            }
        }
        changed
    }

    /// A reply the terminal engine made to the program's query (rule 9): AUTOMATIC_INPUT, which
    /// never takes the size, on a stream that carries it. On one that doesn't (a machine whose
    /// core predates it), the reply would go as DATA, which takes the size. So only the terminal
    /// in use answers, and only while the core still holds it at the grid this window told:
    /// once another viewer moved it, a reply (the mode-2048 report its resize brings) would take
    /// the size back with no real use, and contradict the leader's own reply. Such replies are
    /// dropped.
    fn send_reply(&mut self, address: &str, bytes: Vec<u8>) {
        let Some(viewer) = self.viewers.get(address) else {
            return;
        };
        if viewer.automatic_input {
            self.link.send(address, Out::Automatic(bytes));
        } else if self.in_use.as_deref() == Some(address) && !viewer.moved {
            self.send_data(address, bytes);
        }
    }

    /// Input for the program, as DATA. DATA takes the size, and the core then applies the grid
    /// this viewer last reported (`herdr-core-protocol.md` §4), which may be stale. So a viewer
    /// that isn't in use (a paste into a window in the background, a file landing after a
    /// `terminal.put`) reports first the grid it should hold: its pane's, or the core's own for
    /// a terminal no shown pane draws, which leaves the size where it is. The terminal in use
    /// told its grid already ([`State::fit`]). Either way the core then holds the grid told
    /// last, so the viewer is no longer `moved`.
    fn send_data(&mut self, address: &str, bytes: Vec<u8>) {
        let in_use = self.in_use.as_deref() == Some(address);
        let told = self.viewers.get(address).and_then(|v| v.told_grid);
        if !(in_use && told.is_some())
            && let Some(grid) = self.grid_to_hold(address)
            && told != Some(grid)
        {
            self.link.send(
                address,
                Out::Resize {
                    cols: grid.columns,
                    rows: grid.rows,
                },
            );
            if let Some(viewer) = self.viewers.get_mut(address) {
                viewer.told_grid = Some(grid);
            }
        }
        self.link.send(address, Out::Input(bytes));
        if let Some(viewer) = self.viewers.get_mut(address) {
            viewer.moved = false;
        }
    }

    /// The grid a terminal should have when this window takes its size: the peek's or its shown
    /// pane's, or the core's when nothing laid out draws it.
    fn grid_to_hold(&self, address: &str) -> Option<GridSize> {
        match self.rect_of(address) {
            Some(rect) => Some(self.grid_for(rect)),
            None => self.model.world().terminal(address).map(|t| GridSize {
                columns: t.cols.max(1),
                rows: t.rows.max(1),
            }),
        }
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
        s.images = Some(ImageCache::new(waker.clone()));
        if s.sources.home.is_some() && s.watcher.is_none() {
            let wake = waker.clone();
            let seen = s.stamp.take().unwrap_or_else(|| s.sources.stamp());
            s.watcher = Some(ConfigWatcher::start(
                s.sources.clone(),
                seen,
                s.config.settings.clone(),
                move || wake(),
            ));
        }
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
            // The window's composition ends with it; Rune's commit of it is dropped (`on_ime`).
            for viewer in s.viewers.values_mut() {
                viewer.set_preedit(None);
            }
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

    /// The peek's terminal area was laid out: the peeked terminal's grid, while it is in use.
    pub fn peek_laid_out(&mut self, rect: [f32; 4], tx: &mut Tx) {
        let area = Area {
            w: rect[2],
            h: rect[3],
        };
        if self.peek_area == Some(area) && self.scale == tx.scale() {
            tx.live_only();
            return;
        }
        self.peek_area = Some(area);
        self.scale = tx.scale();
        self.pump(tx.clock());
        self.apply_ui(tx);
    }

    /// The peek's terminal area's width, in view points, once it's laid out.
    pub fn peek_area_w(&self) -> Option<f32> {
        self.peek_area.map(|a| a.w)
    }

    /// The terminal a pane handler's `pane` means: the pane's, or the peeked one for [`PEEK`].
    fn surface_address(&self, pane: &str) -> Option<String> {
        if pane == PEEK {
            self.model.peeked().map(str::to_owned)
        } else {
            self.model.world().address_of(pane).map(str::to_owned)
        }
    }

    /// A pane took or lost the keyboard: the model follows a click into another pane.
    pub fn pane_focus(&mut self, pane: &str, gained: bool, tx: &mut Tx) {
        if !gained {
            // Rune discards a composition when focus moves; the pane left behind must not keep
            // drawing it, or treat its keys as part of it.
            if let Some(address) = self.surface_address(pane)
                && let Some(viewer) = self.viewers.get_mut(&address)
            {
                viewer.set_preedit(None);
            }
            return;
        }
        if pane == PEEK {
            return;
        }
        if self.model.focused_pane().as_deref() != Some(pane) {
            self.step(tx.clock(), Action::FocusPane(pane.to_owned()));
            self.pump(tx.clock());
            self.apply_ui(tx);
        }
    }

    /// Where a terminal is drawn, laid out, in view points: the peek's area for the peeked one,
    /// else its pane's place in the tab area.
    fn rect_of(&self, address: &str) -> Option<model::Rect> {
        if self.model.peeked() == Some(address) {
            let area = self.peek_area?;
            return Some(model::Rect::new(0.0, 0.0, area.w, area.h));
        }
        self.area?;
        self.shown_panes()
            .into_iter()
            .find(|(_, _, a)| a.as_deref() == Some(address))
            .map(|(_, rect, _)| rect)
    }

    /// Makes pane `pane` the one in use before input it didn't get through the keyboard (a
    /// click, a drop): focused, so the window's in-use terminal takes its size first. True when
    /// its terminal is the one in use: only then may it send the program DATA, which would
    /// otherwise apply this viewer's stale size (`herdr-core-protocol.md` §4).
    fn use_pane(&mut self, pane: &str, address: &str, tx: &mut Tx) -> bool {
        if pane != PEEK && self.model.focused_pane().as_deref() != Some(pane) {
            self.step(tx.clock(), Action::FocusPane(pane.to_owned()));
        }
        self.reclaim(address);
        self.pump(tx.clock());
        self.in_use() == Some(address)
    }

    /// A click in the terminal in use, after the core moved it off the grid this window told
    /// (Herdr app attached, refit or reattached it): RESIZE again, so the click takes the size
    /// back, as a mouse-down in Herdr app does. Typing needs no such step: its DATA applies the
    /// grid told last. Nothing takes it back without real use.
    fn reclaim(&mut self, address: &str) {
        if self.in_use.as_deref() != Some(address) {
            return;
        }
        if let Some(viewer) = self.viewers.get_mut(address)
            && viewer.moved
        {
            viewer.told_grid = None;
        }
    }

    /// A key pressed in pane `pane`: it goes to its terminal. False for keys it leaves (chords
    /// with Cmd).
    pub fn typed(&mut self, pane: &str, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let address = self.surface_address(pane);
        let composing = address
            .as_ref()
            .and_then(|a| self.viewers.get(a))
            .is_some_and(|v| v.preedit.is_some());
        let info = KeyInfo {
            composing,
            ..KeyInfo::pressed(tx, self.option_as_alt)
        };
        let Some(key) = keys::encode(ev, info) else {
            return false;
        };
        if pane != PEEK && self.model.focused_pane().as_deref() != Some(pane) {
            self.step(tx.clock(), Action::FocusPane(pane.to_owned()));
        }
        let Some(address) = address else {
            return true;
        };
        // Typing is using it: it takes the size first, if it hadn't.
        self.pump(tx.clock());
        if let Some(viewer) = self.viewers.get(&address) {
            let down = (address.clone(), key.key);
            if !self.keys_down.contains(&down) {
                self.keys_down.push(down);
            }
            viewer.terminal.key(key);
        }
        self.after_input(tx);
        true
    }

    /// A key let go in pane `pane`: its release goes to the terminal its press went to, which
    /// sends it only to a program that asked for releases (kitty's event types flag).
    pub fn released(&mut self, pane: &str, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let info = KeyInfo::released(tx, self.option_as_alt);
        let Some(key) = keys::encode(ev, info) else {
            return false;
        };
        let Some(address) = self.surface_address(pane) else {
            return false;
        };
        let down = (address.clone(), key.key);
        let Some(i) = self.keys_down.iter().position(|k| *k == down) else {
            return false;
        };
        self.keys_down.remove(i);
        if let Some(viewer) = self.viewers.get(&address) {
            viewer.terminal.key(key);
        }
        self.after_input(tx);
        true
    }

    /// After input reached a worker: in scenes and tests its answer is waited for, and what it
    /// asked of the UI (the clipboard, a frame) goes to Rune.
    fn after_input(&mut self, tx: &mut Tx) {
        if self.synchronous {
            self.pump(tx.clock());
        }
        self.apply_ui(tx);
    }

    /// The terminal the keyboard is in: the peeked one, else the focused pane's.
    fn typing_into(&self) -> Option<String> {
        let address = self.model.keyboard_terminal()?;
        self.viewers
            .contains_key(address)
            .then(|| address.to_owned())
    }

    /// `App::on_ime`: the input method composing for the focused terminal, whose caret the pane
    /// marks. A composition shows at the cursor; committed text goes to the program as it is,
    /// only from the terminal in use. Rune commits an open composition when the window goes to
    /// the background, after FOCUS(0): sent, that DATA would take the size back from the window
    /// in front (`herdr-core-protocol.md` §4), so it's dropped.
    pub fn on_ime(s: &mut State, ime: &Ime, tx: &mut Tx) -> bool {
        // The jump bar holds the keyboard: nothing composes into the terminal behind it.
        if matches!(s.model.mode(), model::Mode::Popup(PopupState::Jump(_))) {
            return false;
        }
        let Some(address) = s.typing_into() else {
            return false;
        };
        let in_use = s.in_use() == Some(address.as_str());
        let viewer = s.viewers.get_mut(&address).expect("typing into a viewer");
        match ime {
            Ime::Preedit(text, ..) => {
                viewer.set_preedit((!text.is_empty()).then(|| text.clone()));
                true
            }
            Ime::Commit(text) => {
                viewer.set_preedit(None);
                if in_use {
                    viewer.terminal.input(text.as_bytes());
                }
                s.after_input(tx);
                true
            }
            Ime::Disabled => {
                viewer.set_preedit(None);
                false
            }
            Ime::Select(_) | Ime::Enabled => false,
        }
    }

    /// The pointer over the tab area (in its points), or gone from it: a movable pane whose top
    /// [`model::geometry::GRIP_ROW`] points it is on shows its grip.
    pub fn tab_area_pointer(&mut self, at: Option<(f32, f32)>, tx: &mut Tx) {
        let row = model::geometry::GRIP_ROW;
        let over = at.and_then(|(x, y)| {
            self.model
                .pane_rects()
                .into_iter()
                .find(|(pane, r)| {
                    r.contains(x, y) && y < r.y + row.min(r.h) && self.model.pane_movable(pane)
                })
                .map(|(pane, _)| pane)
        });
        if over == self.grip_hover {
            // Nothing to draw: most moves land here.
            tx.live_only();
            return;
        }
        self.grip_hover = over;
    }

    /// A divider of the shown tab dragged: it follows the pointer, writing the sizes as it goes
    /// (each write waits for the one before it). `grab` is where along the split's axis, in the
    /// tab area, the press was; a cancelled drag puts the sizes back.
    pub fn divider_dragged(
        &mut self,
        container: &str,
        index: usize,
        phase: rune::Phase,
        at: f32,
        cancelled: bool,
        tx: &mut Tx,
    ) {
        let ours = |d: &DividerDrag| d.container == container && d.index == index;
        match phase {
            rune::Phase::Down => {
                self.divider_drag = None;
                // A layout that can't take the sizes says so once, here, not on every move; the
                // drag then does nothing.
                if let Some(refused) = self.model.edit_refused(container) {
                    self.toast(refused);
                    return;
                }
                let before = self
                    .model
                    .world()
                    .node(container)
                    .map(|n| {
                        n.children()
                            .iter()
                            .filter(|c| !c.hidden())
                            .skip(index)
                            .take(2)
                            .map(|c| {
                                let size = c.size().unwrap_or(herdr_shared::Size::Weight(1.0));
                                (c.id.clone(), size)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.divider_drag = Some(DividerDrag {
                    container: container.to_owned(),
                    index,
                    before,
                    moved: false,
                });
            }
            rune::Phase::Move => {
                let Some(drag) = self.divider_drag.as_mut().filter(|d| ours(d)) else {
                    return;
                };
                drag.moved = true;
                self.act(
                    Action::DragDivider {
                        container: container.to_owned(),
                        index,
                        at,
                    },
                    tx,
                );
            }
            rune::Phase::Up => {
                let drag = self.divider_drag.take();
                // A press that never moved wrote nothing: a click leaves the sizes alone, and a
                // cancel has nothing to put back.
                if let Some(drag) = drag.filter(|d| d.moved && ours(d)) {
                    if cancelled {
                        self.act(Action::ResizeSplit { sizes: drag.before }, tx);
                    } else {
                        self.act(
                            Action::DragDivider {
                                container: container.to_owned(),
                                index,
                                at,
                            },
                            tx,
                        );
                    }
                }
            }
        }
    }

    /// A pane's grip dragged: past a few points it picks the pane under the pointer and where on
    /// it the pane would land; the release moves it there (`app:src/window/pane_drag.rs`).
    /// `at` is the pointer in the tab area.
    pub fn grip_dragged(
        &mut self,
        pane: &str,
        phase: rune::Phase,
        at: (f32, f32),
        cancelled: bool,
        tx: &mut Tx,
    ) {
        let shown = self.model.shown_tab().map(|t| t.id.clone());
        match phase {
            rune::Phase::Down => {
                self.grip_drag = match shown {
                    Some(tab) if self.model.pane_movable(pane) => Some(GripDrag {
                        pane: pane.to_owned(),
                        tab,
                        from: at,
                        moving: false,
                        target: None,
                    }),
                    _ => None,
                };
            }
            rune::Phase::Move => {
                let rects = self.model.pane_rects();
                let Some(drag) = self.grip_drag.as_mut() else {
                    return;
                };
                if shown.as_ref() != Some(&drag.tab) {
                    self.grip_drag = None;
                    return;
                }
                let travel = (at.0 - drag.from.0).abs().max((at.1 - drag.from.1).abs());
                if !drag.moving && travel < GRIP_SLOP {
                    return;
                }
                drag.moving = true;
                drag.target =
                    rects
                        .iter()
                        .filter(|(id, _)| *id != drag.pane)
                        .find_map(|(id, r)| {
                            Some((id.clone(), model::geometry::drop_zone(*r, at.0, at.1)?))
                        });
            }
            rune::Phase::Up => {
                let Some(drag) = self.grip_drag.take() else {
                    return;
                };
                if cancelled || !drag.moving || shown.as_ref() != Some(&drag.tab) {
                    return;
                }
                let rects = self.model.pane_rects();
                let target = rects
                    .iter()
                    .filter(|(id, _)| *id != drag.pane)
                    .find_map(|(id, r)| {
                        Some((id.clone(), model::geometry::drop_zone(*r, at.0, at.1)?))
                    });
                if let Some((target, edge)) = target {
                    self.act(
                        Action::RearrangePane {
                            pane: drag.pane,
                            target,
                            edge,
                        },
                        tx,
                    );
                }
            }
        }
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
        let terminal = focused.starts_with("pane-") || focused == PEEK_KEY;
        let target = crate::view::popup::focus_key(s.model.mode());
        let in_field = focused == crate::view::overlay::FIELD;
        let handled = s.keymap_key(ev, terminal, tx);
        if handled || !s.model.mode().holds_keyboard() {
            return handled;
        }
        // An open popup keeps the keyboard: its field types what the popup leaves, and nothing
        // else gets a key, nor does Tab move focus out of it to a pane behind.
        if in_field && ev.key != rune::Key::Tab {
            return false;
        }
        tx.focus(target, false);
        true
    }

    /// `AppSpec::on_key`: with nothing focused, the keymap, then typing into the model's focused
    /// pane, which takes the keyboard back. Keys a focused element passed on (`tx.focused()` is
    /// set) already went through the keymap in [`State::preview_key`], and go nowhere.
    pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if tx.focused().is_some() {
            return false;
        }
        let pane = s.keyboard_surface();
        if s.keymap_key(ev, pane.is_some(), tx) {
            return true;
        }
        // An open popup's typing goes to its field, never the terminal behind it.
        if s.model.mode().holds_keyboard() {
            return false;
        }
        let Some(pane) = s.keyboard_surface() else {
            return false;
        };
        let typed = s.typed(&pane, ev, tx);
        if typed {
            tx.focus(surface_key(&pane), false);
        }
        typed
    }

    /// What the keyboard types into, as a pane handler names it: [`PEEK`] while the peek is
    /// open, else the focused pane.
    pub fn keyboard_surface(&self) -> Option<String> {
        if self.model.peeked().is_some() {
            Some(PEEK.to_owned())
        } else {
            self.model.focused_pane()
        }
    }

    /// The pointer in device pixels from the grid's top left, from points in the pane.
    fn grid_px(&self, address: &str, x: f32, y: f32) -> (f32, f32) {
        let padding = self
            .viewers
            .get(address)
            .and_then(|v| v.pane.as_ref())
            .map_or(Padding::default(), |p| p.padding);
        (
            x * self.scale - padding.left as f32,
            y * self.scale - padding.top as f32,
        )
    }

    /// A mouse button pressed, dragged or let go over pane `pane` (`El::on_drag`). A program
    /// that tracks the mouse gets it, unless Shift is held; otherwise the primary button
    /// selects (Shift extends, Alt makes a block) and the middle one pastes. Letting go of a
    /// selection copies it (`copy_on_select`). Cmd-click on a live terminal's address in the
    /// text peeks it, and goes no further.
    pub fn pane_drag(&mut self, pane: &str, d: &Drag, tx: &mut Tx) {
        let Some(address) = self.surface_address(pane) else {
            return;
        };
        let in_use = d.phase == Phase::Down && self.use_pane(pane, &address, tx);
        if d.phase == Phase::Down && tx.button() == PointerButton::Primary && tx.mods().cmd {
            let (x, y) = self.grid_px(&address, d.x, d.y);
            if let Some((target, _)) = self.address_at(&address, x, y) {
                self.press = None;
                self.step(tx.clock(), Action::Peek(target));
                self.pump(tx.clock());
                self.apply_ui(tx);
                return;
            }
        }
        let Some(viewer) = self.viewers.get(&address) else {
            return;
        };
        let mods = tx.mods();
        let (x, y) = self.grid_px(&address, d.x, d.y);
        let button = match tx.button() {
            PointerButton::Middle => MouseButton::Middle,
            PointerButton::Secondary => MouseButton::Right,
            _ => MouseButton::Left,
        };
        let report = |action, button, any_button_pressed| MouseEvent {
            action,
            button,
            mods: term_mods(mods),
            x,
            y,
            any_button_pressed,
        };
        let select = |action| SelectEvent {
            clicks: d.clicks,
            extend: mods.shift,
            rectangle: mods.alt,
            ..SelectEvent::new(action, x, y)
        };
        match d.phase {
            // Cmd-click on the underlined link opens it, whatever the program does with the
            // mouse; anywhere else Cmd-click is a click (Ghostty's `mouseButtonCallback`).
            Phase::Down if mods.cmd && button == MouseButton::Left && viewer.link_at(x, y) => {
                viewer.terminal.open_link(x, y);
                self.press = None;
            }
            Phase::Down => {
                let gesture = if in_use && viewer.tracks_mouse() && !mods.shift {
                    // The program took the click: an old selection can't be copied again.
                    viewer.terminal.clear_selection();
                    viewer
                        .terminal
                        .mouse(report(MouseAction::Press, Some(button), false));
                    Some(Gesture::Report(button))
                } else if button == MouseButton::Left {
                    viewer.terminal.select(select(SelectAction::Press));
                    Some(Gesture::Select)
                } else {
                    if button == MouseButton::Middle && in_use {
                        self.ui.paste.push(address.clone());
                    }
                    None
                };
                self.press = gesture.map(|gesture| Press {
                    address: address.clone(),
                    gesture,
                });
            }
            Phase::Move => {
                match self.press.as_ref().map(|p| (&p.address, &p.gesture)) {
                    Some((a, Gesture::Report(held))) if *a == address => viewer
                        .terminal
                        .mouse(report(MouseAction::Motion, Some(*held), true)),
                    Some((a, Gesture::Select)) if *a == address => {
                        viewer.terminal.select(select(SelectAction::Drag))
                    }
                    _ => {}
                }
            }
            Phase::Up => match self.press.take() {
                Some(Press {
                    address: a,
                    gesture: Gesture::Report(held),
                }) if a == address => {
                    viewer
                        .terminal
                        .mouse(report(MouseAction::Release, Some(held), false))
                }
                Some(Press {
                    address: a,
                    gesture: Gesture::Select,
                }) if a == address => {
                    viewer.terminal.select(select(SelectAction::Release));
                    if self.config.settings.copy_on_select {
                        viewer.terminal.copy(None);
                    }
                }
                _ => {}
            },
        }
        self.after_input(tx);
    }

    /// The live terminal whose address is under the pointer (device pixels from the grid's top
    /// left) in terminal `source`'s text, as its last snapshot shows it, and the word naming it.
    fn address_at(&self, source: &str, x: f32, y: f32) -> Option<(String, String)> {
        let snapshot = self.viewers.get(source)?.shown()?;
        let (word, _) = crate::view::peek::word_at(snapshot, x, y)?;
        let live = self.model.world().live().map(|t| t.address.as_str());
        let address = model::peek::terminal_address(&word, source, live)?;
        Some((address, word))
    }

    /// Cmd held with the pointer at `(x, y)` over terminal `address`'s grid, or not: the link
    /// or terminal address there is underlined, the address first, as Cmd-click peeks it before
    /// opening a link. Without Cmd the underline goes.
    fn hover_link(&mut self, address: &str, at: Option<(f32, f32)>) {
        let word = at
            .and_then(|(x, y)| self.address_at(address, x, y))
            .map(|(_, word)| word);
        if let Some(viewer) = self.viewers.get_mut(address) {
            viewer.terminal.hover(at, word);
            viewer.hovering_link = at.is_some();
        }
    }

    /// The pointer moving over pane `pane` with no button held: a program tracking every
    /// motion (mode 1003) hears it.
    pub fn pane_hover(&mut self, pane: &str, p: PointerEvent, tx: &mut Tx) {
        tx.live_only();
        if self.press.is_some() {
            return;
        }
        let Some(address) = self.surface_address(pane) else {
            return;
        };
        let mods = tx.mods();
        let (x, y) = self.grid_px(&address, p.x, p.y);
        if let Some((was, ..)) = self.pointer.clone()
            && was != address
            && self.viewers.get(&was).is_some_and(|v| v.hovering_link)
        {
            self.hover_link(&was, None);
        }
        self.pointer = Some((address.clone(), x, y));
        // With Cmd held the link under the pointer is underlined, until Cmd or the pointer goes.
        if mods.cmd || self.viewers.get(&address).is_some_and(|v| v.hovering_link) {
            self.hover_link(&address, mods.cmd.then_some((x, y)));
        }
        if self.in_use() == Some(address.as_str())
            && let Some(viewer) = self.viewers.get(&address)
            && viewer.tracks_mouse()
            && !mods.shift
        {
            viewer.terminal.mouse(MouseEvent {
                action: MouseAction::Motion,
                button: None,
                mods: term_mods(mods),
                x,
                y,
                any_button_pressed: false,
            });
        }
    }

    /// The pointer left pane `pane`: no link of its is under the pointer any more.
    pub fn pane_left(&mut self, pane: &str, tx: &mut Tx) {
        let Some(address) = self.surface_address(pane) else {
            return;
        };
        if self.pointer.as_ref().is_some_and(|(a, ..)| *a == address) {
            self.pointer = None;
        }
        if self.viewers.get(&address).is_some_and(|v| v.hovering_link) {
            self.hover_link(&address, None);
            self.after_input(tx);
        }
    }

    /// `App::on_modifiers`: Cmd pressed underlines the link under the pointer, as moving with it
    /// held does; let go, the underline goes.
    pub fn on_modifiers(s: &mut State, mods: rune::Mods, tx: &mut Tx) {
        let over = s.pointer.clone().filter(|_| mods.cmd);
        let addresses: Vec<String> = s.viewers.keys().cloned().collect();
        for address in addresses {
            match &over {
                Some((a, x, y)) if *a == address => s.hover_link(&address, Some((*x, *y))),
                _ if s.viewers[&address].hovering_link => s.hover_link(&address, None),
                _ => {}
            }
        }
        s.after_input(tx);
    }

    /// The wheel over pane `pane`, as Ghostty scrolls: a notch is three lines, and trackpad
    /// travel adds up to a line per cell height, both times `terminal_scroll_speed`; whole lines
    /// only, never part of one. A program tracking the mouse hears wheel buttons (Shift keeps
    /// the wheel the terminal's); on the alternate screen, which has no scrollback, the arrows
    /// (xterm's alternate scroll); otherwise the scrollback moves. Only the terminal in use sends
    /// the program anything: over another pane the wheel moves the scrollback alone.
    pub fn pane_wheel(&mut self, pane: &str, ev: &WheelEvent, tx: &mut Tx) -> bool {
        let Some(address) = self.surface_address(pane) else {
            return false;
        };
        // macOS turns Shift with a notched wheel into horizontal scrolling.
        let dy = if ev.mods.shift && ev.dy == 0.0 {
            ev.dx
        } else {
            ev.dy
        };
        let speed = self.config.settings.terminal_scroll_speed;
        let in_use = self.in_use() == Some(address.as_str());
        let line = self.face(self.scale).cell_pt().1;
        let zoom = self.zoom();
        let Some(viewer) = self.viewers.get_mut(&address) else {
            return true;
        };
        let alternate = viewer
            .shown()
            .is_some_and(|s| s.active_screen == ActiveScreen::Alternate);
        let to_program = viewer.tracks_mouse() && !ev.mods.shift;
        if !in_use && (alternate || to_program) {
            return true;
        }
        let lines = if ev.notch || !ev.precise {
            // Rune hands the delta over in the zoomed view's points; a notch is 40 window points.
            let ticks = dy * zoom / WHEEL_NOTCH;
            if ticks == 0.0 {
                0
            } else {
                // A slow notch still counts as one (Ghostty's `yoff_max = max(yoff, 1)`).
                let n = whole(ticks.abs().max(1.0) * LINES_PER_NOTCH * speed).max(1.0);
                (ticks.signum() * n) as i32
            }
        } else {
            viewer.wheel += dy * speed;
            let lines = whole(viewer.wheel / line);
            viewer.wheel -= lines * line;
            lines as i32
        };
        if lines == 0 {
            return true;
        }
        let viewer = self.viewers.get(&address).expect("found above");
        let (x, y) = self.grid_px(&address, ev.x, ev.y);
        let up = lines > 0;
        let n = lines.unsigned_abs();
        if to_program {
            let button = if up {
                MouseButton::WheelUp
            } else {
                MouseButton::WheelDown
            };
            for _ in 0..n {
                viewer.terminal.mouse(MouseEvent {
                    action: MouseAction::Press,
                    button: Some(button),
                    mods: term_mods(ev.mods),
                    x,
                    y,
                    any_button_pressed: false,
                });
            }
        } else if alternate {
            let arrow = if up {
                ghostty_vt_sys::GHOSTTY_KEY_ARROW_UP
            } else {
                ghostty_vt_sys::GHOSTTY_KEY_ARROW_DOWN
            };
            for _ in 0..n {
                viewer
                    .terminal
                    .key(terminal::KeyEvent::press(arrow, terminal::Mods::NONE));
            }
        } else {
            let delta = n as isize;
            viewer
                .terminal
                .scroll(Scroll::Delta(if up { -delta } else { delta }));
        }
        self.after_input(tx);
        true
    }

    /// A press or drag on a pane's scrollbar, a `track`-point lane: the thumb follows the
    /// pointer (held where it was pressed, or by its middle when the press was beside it) and
    /// the viewport with it. All the way down follows the program's output again.
    pub fn scrollbar_drag(&mut self, address: &str, d: &mut Drag, track: f32, tx: &mut Tx) {
        use crate::view::terminal::scrollbar::{grab_at, row_at, thumb};
        let line = self.face(self.scale).cell_pt().1;
        let Some(viewer) = self.viewers.get(address) else {
            return;
        };
        let Some(sb) = viewer.shown().map(|s| s.scrollbar) else {
            return;
        };
        if d.phase == Phase::Down {
            let Some((top, len)) = thumb(sb.total, sb.offset, sb.len, track, line) else {
                return;
            };
            d.data = grab_at(top, len, d.y);
        }
        let row = row_at(sb.total, sb.len, track, line, d.y - d.data);
        let bottom = sb.total.saturating_sub(sb.len);
        viewer.terminal.scroll(if row >= bottom {
            Scroll::Bottom
        } else {
            Scroll::Row(row as usize)
        });
        self.after_input(tx);
    }

    /// A right-click on pane `pane`: the program's, when its terminal is labelled to send
    /// right-clicks there (`herdr.right-click=program`) and it tracks the mouse, unless Shift is
    /// held; otherwise the terminal's menu (ticket 026), without the pane items in the peek.
    pub fn pane_right_click(&mut self, pane: &str, p: PointerEvent, tx: &mut Tx) {
        let Some(address) = self.surface_address(pane) else {
            return;
        };
        let to_program = self
            .model
            .world()
            .terminal(&address)
            .is_some_and(|t| herdr_shared::labels::right_click_to_program(&t.labels));
        let mods = tx.mods();
        let (x, y) = self.grid_px(&address, p.x, p.y);
        let for_program = to_program
            && !mods.shift
            && self.viewers.get(&address).is_some_and(Viewer::tracks_mouse);
        // The label decides, not the focus: a right-click for the program takes its pane, as
        // a click does.
        let in_use = for_program && self.use_pane(pane, &address, tx);
        match self.viewers.get(&address) {
            Some(viewer) if in_use => {
                for action in [MouseAction::Press, MouseAction::Release] {
                    viewer.terminal.mouse(MouseEvent {
                        action,
                        button: Some(MouseButton::Right),
                        mods: term_mods(mods),
                        x,
                        y,
                        any_button_pressed: false,
                    });
                }
            }
            // The peek's terminal: its menu, with no pane items.
            _ if pane == PEEK => {
                self.step(tx.clock(), Action::PeekMenu);
            }
            _ => {
                self.step(tx.clock(), Action::TerminalMenu(pane.to_owned()));
            }
        }
        self.after_input(tx);
    }

    /// Files dropped on pane `pane`: their paths typed in, quoted for the shell, as a paste.
    /// The pane takes focus first, as a click would. In a window in the background no pane is
    /// in use, and the paste goes anyway, as in Herdr app: the person asked for it. It takes
    /// the pane's grid with it ([`State::send_data`]), so the core doesn't apply a stale one.
    pub fn pane_dropped(&mut self, pane: &str, paths: &[PathBuf], tx: &mut Tx) {
        if let Some(address) = self.surface_address(pane) {
            self.use_pane(pane, &address, tx);
            self.drop_files(&address, paths.to_vec());
        }
        self.after_input(tx);
    }

    /// The root's `on_paste_content`: a paste Rune delivers (its Edit menu, a script) goes to
    /// the terminal the keyboard is in.
    pub fn pasted(&mut self, content: &ClipboardContent, tx: &mut Tx) {
        if let Some(address) = self.typing_into() {
            self.paste_content(&address, content);
        }
        self.after_input(tx);
    }

    /// What the clipboard held, pasted into a terminal as Herdr app pastes it: copied files as
    /// if dropped, else the text, else (an image alone) Ctrl+V for the program to read it.
    pub fn paste_content(&mut self, address: &str, content: &ClipboardContent) {
        if !content.files().is_empty() {
            self.drop_files(address, content.files().to_vec());
        } else if !content.text().is_empty() {
            self.paste_text(address, content.text());
        } else if let Some(image) = content.image() {
            self.paste_image(address, Some(image));
        }
    }

    /// An image for a terminal's program. One on another machine reads that machine's
    /// clipboard, so the image is sent beside it as a PNG and its path pasted; a local program
    /// reads the clipboard itself when it hears Ctrl+V (Herdr app's `paste_image`).
    fn paste_image(&mut self, address: &str, image: Option<&rune::ClipboardImage>) {
        if address.contains('/') {
            if let Some(image) = image {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos());
                self.link.put(
                    address,
                    vec![Upload::Bytes {
                        name: format!("herdr-paste-{stamp}.png"),
                        data: image.png().into(),
                    }],
                );
            }
        } else if let Some(viewer) = self.viewers.get(address) {
            viewer.terminal.key(ctrl_v());
        }
    }

    /// Text pasted into a terminal, framed for its modes (bracketed paste). Like Herdr app it
    /// never asks before pasting text that could run commands; unsafe control bytes are still
    /// replaced.
    fn paste_text(&mut self, address: &str, text: &str) {
        if let Some(viewer) = self.viewers.get(address) {
            viewer.terminal.paste(text, true);
        }
    }

    /// Files for a terminal: their paths, quoted, as a paste. A terminal on another machine
    /// gets each file copied beside it first (`terminal.put`), and its path there.
    fn drop_files(&mut self, address: &str, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if address.contains('/') {
            self.link
                .put(address, paths.into_iter().map(Upload::File).collect());
        } else {
            let paths: Vec<String> = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            self.paste_text(address, &quote_paths(&paths));
        }
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

/// The whole lines in `lines`, toward zero. Rune divides the wheel's points by the zoom and
/// [`State::pane_wheel`] multiplies them back, so 27 lines can come back as 26.99997: lines are
/// counted to a thousandth first.
pub fn whole(lines: f32) -> f32 {
    ((lines * 1000.0).round() / 1000.0).trunc()
}

/// Rune's modifiers as a terminal's (Cmd is Super).
fn term_mods(m: rune::Mods) -> terminal::Mods {
    let mut mods = terminal::Mods::NONE;
    for (held, bit) in [
        (m.shift, terminal::Mods::SHIFT),
        (m.ctrl, terminal::Mods::CTRL),
        (m.alt, terminal::Mods::ALT),
        (m.cmd || m.logo, terminal::Mods::SUPER),
    ] {
        if held {
            mods = mods | bit;
        }
    }
    mods
}

/// Ctrl+V, for a program to read the clipboard's image itself.
fn ctrl_v() -> terminal::KeyEvent {
    terminal::KeyEvent {
        unshifted: Some('v'),
        ..terminal::KeyEvent::press(GHOSTTY_KEY_V, terminal::Mods::CTRL)
    }
}

/// Paths as a shell reads them, each followed by a space: as they are when plain, else
/// single-quoted, as Herdr app types a drop (`quote_paths`). Agents read a dropped image that
/// way.
pub fn quote_paths(paths: &[String]) -> String {
    let plain = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/._-+=,:@%~".contains(c))
    };
    let mut out = String::new();
    for p in paths {
        if plain(p) {
            out.push_str(p);
        } else {
            out.push('\'');
            out.push_str(&p.replace('\'', "'\\''"));
            out.push('\'');
        }
        out.push(' ');
    }
    out
}
