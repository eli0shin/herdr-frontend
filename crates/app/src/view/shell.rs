//! The window, outside in, as Eli's `eli` layout has it (`feature-inventory.md` §1):
//!
//! ```text
//! root row
//! ├─ sidebar   366 pt: its 38 pt top row drags the window ("Agents"), the list below (later)
//! └─ work column
//!    ├─ tabs   38 pt: drags the window; the shown space's tab pills ([`super::tabs`])
//!    └─ main   the shown tab's panes, bare, with a 1 pt gap that shows the border color
//! ```
//!
//! No title bar, no status row: terminals reach the window's bottom edge. A pane is its
//! terminal's grid at the core's size, at the pane's top left; until its first snapshot it is
//! the terminal's background.
//!
//! Colors are the theme's ([`crate::config::theme::Chrome`]), labels are in the settings'
//! `ui_font`, and every size here is at zoom 1: Rune's zoom scales the whole view.
//!
//! A pane takes the terminal's input: keys and their releases, the mouse (buttons, drags, hover,
//! the wheel, right-clicks) and dropped files ([`State`]'s pane handlers). The window's root
//! (`crate::view`) takes pastes Rune delivers and releases with nothing focused, for the focused
//! pane.

use rune::{Cx, El, PointerButton, TextStyle, col, el, row, text};

use crate::State;
use crate::config::programs::Program;
use crate::config::settings::UI_FONT_SIZE;
use crate::config::theme::Rgba;
use crate::state::{Status, pane_key};
use crate::view::terminal::look::color;

/// The sidebar's top row and the tab strip: 38 pt, 6 above the 29 pt tabs and 3 below.
pub const BAR: f32 = 38.0;

/// The chrome's labels.
pub fn label(s: &State) -> TextStyle {
    s.ui_font(UI_FONT_SIZE, 400.0)
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let c = &s.chrome;
    let work = col()
        .key("work")
        .grow(1.0)
        .h_full()
        .child(super::tabs::view(s, cx))
        .child(main(s, cx));
    row()
        .key("root")
        .w_full()
        .h_full()
        .fill(c.canvas)
        .children(s.sidebar.map(|w| sidebar(s, w)))
        .child(work)
}

fn sidebar(s: &State, width: f32) -> El {
    let c = &s.chrome;
    col()
        .key("sidebar")
        .w(width)
        .h_full()
        .fill(c.chrome)
        .child(
            row()
                .key("brand")
                .w_full()
                .h(BAR)
                .pt(6.0)
                .pb(3.0)
                .pl(14.0)
                .items_center()
                .window_drag()
                .child(
                    text("Agents")
                        .key("brand-title")
                        .font(label(s))
                        .color(c.muted),
                ),
        )
        .child(col().key("agents").w_full().grow(1.0))
}

/// A program's icon at `size` points: a one-color glyph in the program's color (else `ink`), or
/// a picture in its own colors. `None` when it has none, or before the images can be made.
pub fn program_icon(
    s: &State,
    program: &Program,
    size: f32,
    ink: rune::Color,
    cx: &Cx<State>,
) -> Option<El> {
    let path = program.icon.as_deref()?;
    let image = s.icon_image(path, size, cx.env().scale)?;
    let icon = el()
        .key("icon")
        .size(size, size)
        .pixels(image.pixels())
        .a11y_hidden();
    Some(if program.tint {
        let tint = program
            .color
            .as_deref()
            .and_then(Rgba::parse)
            .map_or(ink, Rgba::color);
        icon.pixels_tint(tint)
    } else {
        icon
    })
}

/// The shown tab's panes where the model's geometry puts them, or what the core is doing.
fn main(s: &State, cx: &mut Cx<State>) -> El {
    let c = &s.chrome;
    let area = el()
        .key("main")
        .grow(1.0)
        .w_full()
        .clip_soft(0.0)
        .on_layout_tx(|s: &mut State, rect, tx| s.laid_out(rect, tx));
    let shown = s.shown_panes();
    if shown.is_empty() {
        let message = match &s.status {
            Status::Offline => "Not connected".to_owned(),
            Status::Connecting => "Connecting to herdr-core…".to_owned(),
            Status::Down(why) => format!("herdr-core unavailable: {why}"),
            Status::Connected if s.model.shown_tab().is_some() => String::new(),
            Status::Connected => "No spaces".to_owned(),
        };
        return area
            .fill(c.canvas)
            .center()
            .child(text(message).key("status").font(label(s)).color(c.muted));
    }
    let face = s.face(cx.env().scale);
    let background = s.theme.background.map_or(c.canvas, color);
    let panes = shown.into_iter().map(|(pane, rect, address)| {
        let viewer = address.as_deref().and_then(|a| s.viewers.get(a));
        let body = match viewer.and_then(|v| v.pane.as_ref()) {
            Some(terminal) => terminal.view(&face, &s.look),
            None => el().key(pane_key(&pane)).w_full().h_full().fill(background),
        };
        el().absolute(rect.x, rect.y)
            .size(rect.w, rect.h)
            .child(pane_input(body, &pane))
    });
    area.fill(c.border).children(panes)
}

/// A pane's body with the handlers that give its terminal the keyboard and the mouse.
fn pane_input(body: El, pane: &str) -> El {
    let p = || pane.to_owned();
    let (focus, typing, releasing, dragging, hovering, wheeling, menu, dropping) =
        (p(), p(), p(), p(), p(), p(), p(), p());
    body.focusable(true)
        .pointer_buttons([PointerButton::Middle])
        .on_key(move |s: &mut State, ev, tx| s.typed(&typing, ev, tx))
        .on_key_up(move |s: &mut State, ev, tx| s.released(&releasing, ev, tx))
        .on_focus_change(move |s: &mut State, gained, tx| s.pane_focus(&focus, gained, tx))
        .on_drag(move |s: &mut State, d, tx| s.pane_drag(&dragging, d, tx))
        .on_pointer_move_local(move |s: &mut State, p, tx| s.pane_hover(&hovering, p, tx))
        .on_wheel(move |s: &mut State, ev, tx| s.pane_wheel(&wheeling, &ev, tx))
        .on_secondary_press_local(move |s: &mut State, p, tx| s.pane_right_click(&menu, p, tx))
        .on_drop_files(move |s: &mut State, paths, tx| s.pane_dropped(&dropping, paths, tx))
}
