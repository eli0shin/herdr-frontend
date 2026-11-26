//! The herdr-frontend app: its `AppSpec`, state, views and scenes.
//!
//! `main.rs` only runs [`spec`], so headless tests drive exactly what the window shows.

mod scenes;

use rune::tok;
use rune::{AppSpec, Cx, El, col, text};

/// The window title.
pub const TITLE: &str = "herdr-frontend";

/// The window's first size, in points.
pub const SIZE: (f32, f32) = (1200.0, 800.0);

/// The app's state. Empty until the shell lands.
#[derive(Default)]
pub struct State {}

/// The app as Rune runs it, in a window or headless.
pub fn spec() -> AppSpec<State> {
    AppSpec {
        scenes: scenes::SCENES,
        ..AppSpec::new(TITLE, SIZE, State::default, view)
    }
}

/// The whole window: a dark background with the app's name in the middle.
pub fn view(_: &State, _: &mut Cx<State>) -> El {
    let t = tok::theme(true);
    col()
        .key("root")
        .w_full()
        .h_full()
        .center()
        .fill(t.bg)
        .child(text(TITLE).key("title").color(t.text_secondary))
}
