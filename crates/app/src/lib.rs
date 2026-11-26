//! The herdr-frontend app: its `AppSpec`, state, views and scenes.
//!
//! `main.rs` only runs [`app`], so headless tests drive exactly what the window shows.

pub mod fixtures;
mod scenes;
pub mod view;

use std::cell::RefCell;

use rune::tok;
use rune::{App, AppSpec, Cx, El, FontSmoothing, col, el, row, text};
use terminal::Theme;

use view::terminal::TerminalPane;
use view::terminal::font::{Face, Font};
use view::terminal::look::{self, Look};

/// The window title.
pub const TITLE: &str = "herdr-frontend";

/// The window's first size, in points.
pub const SIZE: (f32, f32) = (1200.0, 800.0);

/// The terminal font's size until settings land: Ghostty's and Herdr app's 14 pt default.
pub const TERMINAL_POINTS: f32 = 14.0;

/// The app's state: the terminal panes on screen, and how terminals look.
pub struct State {
    pub panes: Vec<TerminalPane>,
    pub font: Font,
    /// The colors terminals start with.
    pub theme: Theme,
    pub look: Look,
    /// The font at the display scale the last frame drew at.
    face: RefCell<Option<Face>>,
}

impl Default for State {
    fn default() -> State {
        let (theme, look) = look::arctic();
        State {
            panes: Vec::new(),
            font: Font::jetbrains_mono(TERMINAL_POINTS),
            theme,
            look,
            face: RefCell::new(None),
        }
    }
}

impl State {
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
}

/// The app as Rune runs it, in a window or headless: [`spec`] with glyphs rasterized as Ghostty's
/// are by default (`font-thicken = false`), which is CoreText with font smoothing off. Rune sets
/// this per window, so the app's own text draws the same way.
pub fn app() -> App<State> {
    App::new(spec()).font_smoothing(FontSmoothing::Off)
}

/// The app's spec: its title, size, state, view and scenes.
pub fn spec() -> AppSpec<State> {
    AppSpec {
        scenes: scenes::SCENES,
        ..AppSpec::new(TITLE, SIZE, State::default, view)
    }
}

/// The whole window: the terminal panes in a square-ish grid (two, four and nine panes are 2 × 1,
/// 2 × 2 and 3 × 3) with a 1 px gap, or the app's name in the middle while there are none. The
/// app shell replaces this with the tab's pane tree.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let t = tok::theme(true);
    if s.panes.is_empty() {
        return col()
            .key("root")
            .w_full()
            .h_full()
            .center()
            .fill(t.bg)
            .child(text(TITLE).key("title").color(t.text_secondary));
    }
    let face = s.face(cx.env().scale);
    let gap = 1.0 / face.scale;
    let pane = |p: &TerminalPane| el().grow(1.0).h_full().child(p.view(&face, &s.look));
    let per_row = (s.panes.len() as f32).sqrt().ceil() as usize;
    let rows = s.panes.chunks(per_row).map(|panes| {
        row()
            .w_full()
            .grow(1.0)
            .gap(gap)
            .children(panes.iter().map(pane))
    });
    col()
        .key("root")
        .w_full()
        .h_full()
        .gap(gap)
        .fill(t.bg)
        .children(rows)
}
