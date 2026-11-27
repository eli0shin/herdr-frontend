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
use model::mode::{PopupState, Rename};
use model::{
    Action, AppCommand, CopyAction, Effect, Event, Focus, Input, Level, Model, Popup, PopupAction,
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

/// The sidebar's width: what Eli's saved layout shows (`feature-inventory.md` §1).
pub const SIDEBAR: f32 = 366.0;

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
    focus_sent: bool,
    /// What the input method is composing in this terminal.
    pub preedit: Option<String>,
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
pub struct PaneDrag {
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
    /// The panels' font family, when the font folders have it; Rune's sans otherwise.
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
    /// The sidebar's width, or `None` when it's hidden.
    pub sidebar: Option<f32>,
    /// The font at the display scale the last frame drew at.
    face: RefCell<Option<Face>>,
    scale: f32,
    area: Option<Area>,
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
    /// The open popup's filter field.
    pub help_filter: TextEdit,
    /// Something only the view shows changed (a toast, the prefix indicator): draw a frame.
    redraw: bool,
    /// Which Option keys act as Alt in terminals: both, as Eli's Ghostty has it
    /// (`macos-option-as-alt = true`).
    pub option_as_alt: OptionAsAlt,
    /// Keys whose press went to a terminal, so only their releases follow.
    keys_down: Vec<(String, terminal::Key)>,
    /// The mouse button held over a terminal.
    press: Option<Press>,
    /// Trackpad scrolling not yet a whole line, in points.
    wheel: f32,
    /// The pane whose top strip the pointer is over: its grip shows.
    pub grip_hover: Option<String>,
    pub pane_drag: Option<PaneDrag>,
    divider_drag: Option<DividerDrag>,
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
    /// A state on `link`, restoring the view saved in `home` and reading its settings. With no
    /// home it has the built-in defaults: Eli's settings, arctic, no font files.
    pub fn new(link: Box<dyn Link>, home: Option<Home>, window: Keeper) -> State {
        let view = home
            .as_ref()
            .and_then(|h| h.read(VIEW))
            .and_then(|text| View::from_json(&text).ok())
            .unwrap_or_default();
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
            sidebar: Some(SIDEBAR),
            face: RefCell::new(None),
            scale: 2.0,
            area: None,
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
            help_filter: TextEdit::default(),
            redraw: false,
            option_as_alt: OptionAsAlt::Both,
            keys_down: Vec::new(),
            press: None,
            wheel: 0.0,
            grip_hover: None,
            pane_drag: None,
            divider_drag: None,
        };
        s.configure_all(config);
        s.window_sized(crate::SIZE.0, crate::SIZE.1);
        s
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
                Some(files) => Font::from_files(files, s.font_size).unwrap_or_else(|e| {
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
                Some(files) => Font::from_files(files, UI_FONT_SIZE)
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
            (w / k - self.sidebar.unwrap_or(0.0)).max(0.0),
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
                    PopupState::Menu(menu) => match &menu.target {
                        model::MenuTarget::Tab(tab) => Action::TabMenu(tab.clone()),
                    },
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
            Effect::Terminal { address, command } => self.terminal_command(&address, command),
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

    /// An app command: the window's own are Rune's, the sidebar and the zoom are this state's,
    /// the escape hatch resets the key file; the rest aren't built yet.
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
            AppCommand::ZoomIn => self.zoom_by(1),
            AppCommand::ZoomOut => self.zoom_by(-1),
            AppCommand::ZoomReset => self.zoom_by(0),
            // macOS's menu hides the app on ⌘H and ⌥⌘H before the window sees them.
            AppCommand::Hide | AppCommand::HideOthers | AppCommand::LayoutSwitch => {
                self.not_yet(&Action::App(command))
            }
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

    /// A `terminal` key binding's action, or another command for a terminal's view.
    fn terminal_command(&mut self, address: &str, command: TerminalCommand) {
        // Find and select all are ticket 019's; until then they say so.
        if let TerminalCommand::Action(
            action @ (TerminalAction::Search | TerminalAction::SelectAll),
        ) = command
        {
            self.not_yet(&Action::Terminal(action));
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
            TerminalCommand::Action(TerminalAction::Search | TerminalAction::SelectAll) => {}
            // Copy mode says "not yet" once, as it closes (`State::mode_changed`); it's ticket
            // 020's.
            TerminalCommand::CopyStart
            | TerminalCommand::CopyMove(_)
            | TerminalCommand::CopySelect(_)
            | TerminalCommand::CopyYank
            | TerminalCommand::CopyEnd => {}
        }
    }

    /// Hands the focus, the window commands, the clipboard and the wakes asked for to Rune.
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
        changed | std::mem::take(&mut self.redraw)
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
        for effect in fx.list {
            self.carry_out(effect);
        }
        // A `do` can open a popup or copy mode as a key does.
        if *self.model.mode() != before {
            self.mode_changed(&before);
        }
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
            focus_sent: false,
            preedit: None,
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
                    if let Output::Selection(text) = output {
                        self.ui.copy = Some(text);
                        continue;
                    }
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
                        Output::Selection(_) => {}
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

    /// A pane took or lost the keyboard: the model follows a click into another pane.
    pub fn pane_focus(&mut self, pane: &str, gained: bool, tx: &mut Tx) {
        if !gained {
            // Rune discards a composition when focus moves; the pane left behind must not keep
            // drawing it, or treat its keys as part of it.
            if let Some(address) = self.model.world().address_of(pane).map(str::to_owned)
                && let Some(viewer) = self.viewers.get_mut(&address)
            {
                viewer.set_preedit(None);
            }
            return;
        }
        if self.model.focused_pane().as_deref() != Some(pane) {
            self.step(tx.clock(), Action::FocusPane(pane.to_owned()));
            self.pump(tx.clock());
            self.apply_ui(tx);
        }
    }

    /// Sends a terminal that isn't in use its pane's grid, before input it must send anyway.
    fn resize_for_input(&mut self, address: &str) {
        let Some(rect) = self
            .shown_panes()
            .into_iter()
            .find(|(_, _, a)| a.as_deref() == Some(address))
            .map(|(_, rect, _)| rect)
        else {
            return;
        };
        let grid = self.grid_for(rect);
        if let Some(viewer) = self.viewers.get_mut(address)
            && viewer.attach == Attach::Attached
            && viewer.told_grid != Some(grid)
        {
            self.link.send(
                address,
                Out::Resize {
                    cols: grid.columns,
                    rows: grid.rows,
                },
            );
            viewer.told_grid = Some(grid);
        }
    }

    /// Makes pane `pane` the one in use before input it didn't get through the keyboard (a
    /// click, a drop): focused, so the window's in-use terminal takes its size first. True when
    /// its terminal is the one in use: only then may it send the program DATA, which would
    /// otherwise apply this viewer's stale size (`herdr-core-protocol.md` §4).
    fn use_pane(&mut self, pane: &str, address: &str, tx: &mut Tx) -> bool {
        if self.model.focused_pane().as_deref() != Some(pane) {
            self.step(tx.clock(), Action::FocusPane(pane.to_owned()));
        }
        self.pump(tx.clock());
        self.in_use() == Some(address)
    }

    /// A key pressed in pane `pane`: it goes to its terminal. False for keys it leaves (chords
    /// with Cmd).
    pub fn typed(&mut self, pane: &str, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let address = self.model.world().address_of(pane).map(str::to_owned);
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
        if self.model.focused_pane().as_deref() != Some(pane) {
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
        let Some(address) = self.model.world().address_of(pane).map(str::to_owned) else {
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

    /// The terminal the keyboard is in: the focused pane's.
    fn typing_into(&self) -> Option<String> {
        let address = self.model.focused_terminal()?;
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
                self.pane_drag = match shown {
                    Some(tab) if self.model.pane_movable(pane) => Some(PaneDrag {
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
                let Some(drag) = self.pane_drag.as_mut() else {
                    return;
                };
                if shown.as_ref() != Some(&drag.tab) {
                    self.pane_drag = None;
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
                let Some(drag) = self.pane_drag.take() else {
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
    /// selection copies it (`copy_on_select`).
    pub fn pane_drag(&mut self, pane: &str, d: &Drag, tx: &mut Tx) {
        let Some(address) = self.model.world().address_of(pane).map(str::to_owned) else {
            return;
        };
        let in_use = d.phase == Phase::Down && self.use_pane(pane, &address, tx);
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
                    viewer.terminal.copy(None);
                }
                _ => {}
            },
        }
        self.after_input(tx);
    }

    /// The pointer moving over pane `pane` with no button held: a program tracking every
    /// motion (mode 1003) hears it.
    pub fn pane_hover(&mut self, pane: &str, p: PointerEvent, tx: &mut Tx) {
        tx.live_only();
        if self.press.is_some() {
            return;
        }
        let Some(address) = self.model.world().address_of(pane).map(str::to_owned) else {
            return;
        };
        let mods = tx.mods();
        let (x, y) = self.grid_px(&address, p.x, p.y);
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

    /// The wheel over pane `pane`, as Herdr app and Ghostty scroll: a notch is three lines,
    /// trackpad scrolling a line per cell height. A program tracking the mouse hears wheel
    /// buttons (Shift keeps the wheel the terminal's); on the alternate screen, which has no
    /// scrollback, the arrows (xterm's alternate scroll); otherwise the scrollback moves. Only
    /// the terminal in use sends the program anything: over another pane the wheel moves the
    /// scrollback alone.
    pub fn pane_wheel(&mut self, pane: &str, ev: &WheelEvent, tx: &mut Tx) -> bool {
        let Some(address) = self.model.world().address_of(pane).map(str::to_owned) else {
            return false;
        };
        // macOS turns Shift with a notched wheel into horizontal scrolling.
        let dy = if ev.mods.shift && ev.dy == 0.0 {
            ev.dx
        } else {
            ev.dy
        };
        let lines = if ev.notch || !ev.precise {
            // Rune hands the delta over in the zoomed view's points; a notch is 40 window points.
            let ticks = dy * self.zoom() / WHEEL_NOTCH;
            if ticks == 0.0 {
                0
            } else {
                (ticks.signum() * ticks.abs().max(1.0) * LINES_PER_NOTCH).trunc() as i32
            }
        } else {
            let line = self.face(self.scale).cell_pt().1;
            self.wheel += dy;
            let lines = (self.wheel / line).trunc();
            self.wheel -= lines * line;
            lines as i32
        };
        let in_use = self.in_use() == Some(address.as_str());
        let Some(viewer) = self.viewers.get(&address) else {
            return true;
        };
        if lines == 0 {
            return true;
        }
        let (x, y) = self.grid_px(&address, ev.x, ev.y);
        let up = lines > 0;
        let n = lines.unsigned_abs();
        let alternate = viewer
            .shown()
            .is_some_and(|s| s.active_screen == ActiveScreen::Alternate);
        if !in_use && (alternate || (viewer.tracks_mouse() && !ev.mods.shift)) {
            return true;
        }
        if viewer.tracks_mouse() && !ev.mods.shift {
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

    /// A right-click on pane `pane`: the program's, when its terminal is labelled to send
    /// right-clicks there (`herdr.right-click=program`) and it tracks the mouse, unless Shift is
    /// held; otherwise the terminal's menu (ticket 026).
    pub fn pane_right_click(&mut self, pane: &str, p: PointerEvent, tx: &mut Tx) {
        let Some(address) = self.model.world().address_of(pane).map(str::to_owned) else {
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
            _ => {
                self.step(tx.clock(), Action::TerminalMenu(pane.to_owned()));
            }
        }
        self.after_input(tx);
    }

    /// Files dropped on pane `pane`: their paths typed in, quoted for the shell, as a paste.
    /// The pane takes focus first, as a click would. In a window in the background no pane is
    /// in use, and the paste goes anyway, as in Herdr app: the person asked for it. It takes
    /// the pane's grid with it, so the core doesn't apply a stale one.
    pub fn pane_dropped(&mut self, pane: &str, paths: &[PathBuf], tx: &mut Tx) {
        if let Some(address) = self.model.world().address_of(pane).map(str::to_owned) {
            if !self.use_pane(pane, &address, tx) {
                self.resize_for_input(&address);
            }
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
