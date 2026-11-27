//! The herdr-frontend app: its `AppSpec`, state, views and scenes.
//!
//! `main.rs` only runs [`app`] or [`live`], so headless tests drive exactly what the window
//! shows. [`State`] holds the model and the core [`link::Link`]; [`view`] draws the shell
//! (`view::shell`) around the shown tab's live terminals.

pub mod chord;
pub mod config;
pub mod control;
pub mod e2e;
pub mod fixtures;
pub mod home;
pub mod keyfile;
pub mod keys;
pub mod link;
pub mod machines;
mod scenes;
pub mod shared;
pub mod state;
pub mod toast;
pub mod view;
pub mod window;

use rune::{App, AppSpec, Cx, El, FontSmoothing, col, el, row, text};

pub use state::{State, TERMINAL_POINTS};

use home::Home;
use link::Live;
use view::terminal::TerminalPane;

/// The window title.
pub const TITLE: &str = "herdr-frontend";

/// The window's first size, in points.
pub const SIZE: (f32, f32) = (1200.0, 800.0);

/// The app as Rune runs it, in a window or headless, with no core: [`spec`] with glyphs
/// rasterized as Ghostty's are by default (`font-thicken = false`), which is CoreText with font
/// smoothing off. Rune sets this per window, so the app's own text draws the same way.
pub fn app() -> App<State> {
    App::new(spec())
        .font_smoothing(FontSmoothing::Off)
        .on_focus(State::on_focus)
        .on_modifiers(State::on_modifiers)
        .on_ime(State::on_ime)
}

/// The app on a live core, with its home: the window opens where it was, borderless (Eli's
/// Ghostty frame, [`window`]), and saves its place and view as it goes. Started by the
/// end-to-end harness (`HERDR_FRONTEND_E2E=1`), it also serves the harness's requests.
pub fn live(link: Live, home: Option<Home>) -> App<State> {
    live_with(link, home, e2e::enabled())
}

/// [`live`], serving the end-to-end harness's requests (`e2e`) when `harness`: keys sent to the
/// app, the last drawn frame's tree and the window number.
pub fn live_with(link: Live, home: Option<Home>, harness: bool) -> App<State> {
    let tap = harness.then(e2e::Tap::default);
    let keeper = window::Keeper::new(home.clone());
    let saved = keeper.saved();
    let placed = saved.clone();
    let on_window = keeper.clone();
    let mut app = app()
        .window_options(window::options(saved.as_ref()))
        .chrome(window::CHROME)
        .on_window(move |handle| on_window.created(handle, placed.as_ref()))
        .on_window_geometry(|s, geometry, _| {
            let scale = geometry.scale_factor as f32;
            let (w, h) = geometry.physical_size;
            if scale > 0.0 {
                s.window_sized(w as f32 / scale, h as f32 / scale);
            }
            s.window.save(false);
            true
        })
        .on_close(State::on_close)
        .on_quit(State::on_close)
        .init({
            let tap = tap.clone();
            move || {
                let mut s = State::new(Box::new(link::Offline), home, keeper);
                s.reveal = Some(reveal);
                if let Some(tap) = tap {
                    s.serve_harness(tap);
                }
                s.connect(Box::new(link));
                s
            }
        });
    if let Some(tap) = tap {
        app = app.on_frame_drawn(move |rt| tap.record(rt));
    }
    if let Some(saved) = &saved {
        app = app
            .maximized(saved.maximized)
            .full_screen(saved.full_screen);
    }
    app
}

/// Shows a folder in Finder, without waiting for it.
fn reveal(dir: &std::path::Path) {
    if let Err(e) = std::process::Command::new("open").arg(dir).spawn() {
        eprintln!("herdr-frontend: open {}: {e}", dir.display());
    }
}

/// The app's spec: its title, size, state, view, scenes and core hooks.
pub fn spec() -> AppSpec<State> {
    AppSpec {
        scenes: scenes::SCENES,
        setup: Some(State::setup),
        poll: Some(State::poll),
        on_key: Some(State::on_key),
        ..AppSpec::new(TITLE, SIZE, State::default, view)
    }
}

/// The whole window, at the settings' zoom: the shell around the shown tab's terminals
/// ([`view::shell`]), or, in the renderer's fixture scenes, their panes alone; the open popup
/// and the toasts over it. Its root previews every key for the keymap ([`State::preview_key`]),
/// and takes pastes Rune delivers and key releases with nothing focused, for the focused pane.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    cx.zoom(s.zoom());
    let content = if !s.panes.is_empty() {
        fixture_panes(s, cx)
    } else if s.status == state::Status::Offline && s.model.shown_tab().is_none() {
        hello(s, cx)
    } else {
        view::shell::view(s, cx)
    };
    let popup = view::popup::view(s, cx).or_else(|| view::copy::view(s));
    let peek = view::peek::view(s, cx);
    let toasts = toast::view(s);
    let zoom = s.zoom();
    el().key("window")
        .w_full()
        .h_full()
        .on_key_preview(State::preview_key)
        .on_paste_content(|s: &mut State, content, tx| s.pasted(content, tx))
        .on_key_up(|s: &mut State, ev, tx| {
            let pane = s.keyboard_surface();
            tx.focused().is_none() && pane.is_some_and(|pane| s.released(&pane, ev, tx))
        })
        // Where a right-click was: a menu it opens opens there.
        .on_any_press(move |s: &mut State, button, p, _| {
            if button == rune::PointerButton::Secondary {
                s.menu_at = Some((p.window_x / zoom, p.window_y / zoom));
            }
        })
        .child(content)
        .children(peek)
        .children(popup)
        .children(toasts)
}

/// Rune's tokens in the theme's colors.
fn tokens(s: &State, cx: &Cx<State>) -> rune::tok::Theme {
    s.config
        .theme
        .tokens(s.config.settings.dark(cx.system_dark()))
}

/// The app's name in the middle, before any core: the hello window.
fn hello(s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens(s, cx);
    col()
        .key("root")
        .w_full()
        .h_full()
        .center()
        .fill(t.bg)
        .child(
            text(TITLE)
                .key("title")
                .font(s.ui_font(13.0, 400.0))
                .color(t.text_secondary),
        )
}

/// Fixture panes in a square-ish grid (two, four and nine panes are 2 × 1, 2 × 2 and 3 × 3)
/// with a 1 px gap, nothing around them: what the Ghostty pixel diff compares.
fn fixture_panes(s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens(s, cx);
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
