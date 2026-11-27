//! The window, outside in, as Eli's `eli` layout has it (`feature-inventory.md` §1):
//!
//! ```text
//! root row
//! ├─ sidebar   366 pt by default (200 to 600): its 38 pt top row drags the window, the Agents
//! │            list below (`view::sidebar`)
//! └─ work column
//!    ├─ tabs   38 pt: drags the window; the shown space's tab pills ([`super::tabs`])
//!    └─ main   the shown tab's panes, bare, with a 1 pt gap that shows the border color;
//!              over them each divider's 6 pt handle, each pane's top-edge grip while the
//!              pointer is on that edge, and a dragged pane's drop preview
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

use model::geometry::{self, GRIP_ROW};
use rune::{Color, Cursor, Cx, Drag, El, PointerButton, TextStyle, col, el, row, text};

use crate::State;
use crate::config::programs::Program;
use crate::config::settings::UI_FONT_SIZE;
use crate::config::theme::Rgba;
use crate::state::{Status, pane_key};
use crate::view::terminal::look::color;

/// A divider's handle: wider than the 1 pt gap it sits on, to be easy to grab
/// (`app:src/window.rs` `HANDLE`).
pub const HANDLE: f32 = 6.0;
/// A pane's grip: two rows of three dots in a rounded box, centered on its top edge.
const GRIP: (f32, f32) = (28.0, 18.0);

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
        .children(
            s.sidebar
                .shown()
                .map(|w| crate::view::sidebar::view(s, w, cx)),
        )
        .child(work)
}

/// A program's icon at `size` points: a one-color glyph in the program's color (else `ink`), or
/// a picture in its own colors, as the tab bar draws it (`eli-tabs`). `None` when it has none,
/// or before the images can be made.
pub fn program_icon(
    s: &State,
    program: &Program,
    size: f32,
    ink: rune::Color,
    cx: &Cx<State>,
) -> Option<El> {
    let tint = program
        .color
        .as_deref()
        .and_then(Rgba::parse)
        .map_or(ink, Rgba::color);
    icon_in(s, program, size, tint, cx)
}

/// [`program_icon`] with a one-color glyph always in `ink`, as the Agents list, the PR finder
/// and the peek draw it (their mods' `icon(p, size, t.foreground)`).
pub fn icon_in(
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
        icon.pixels_tint(ink)
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
        let bars = address.as_deref().map(|a| {
            let scrollbar = viewer.and_then(|v| scrollbar(s, v, a, rect.w, &face));
            // A peeked terminal's find bar is over the peek.
            let find = (s.model.peeked() != Some(a))
                .then(|| crate::view::find::view(s, &pane, a, rect.w))
                .flatten();
            (scrollbar, find)
        });
        let (scrollbar, find) = bars.unwrap_or((None, None));
        el().absolute(rect.x, rect.y)
            .size(rect.w, rect.h)
            .child(pane_input(body, &pane))
            .children(scrollbar)
            .children(find)
    });
    let rects = s.model.pane_rects();
    let grips = rects
        .iter()
        .filter(|(pane, _)| s.model.pane_movable(pane))
        .filter_map(|(pane, rect)| grip(s, pane, *rect));
    let preview = s
        .grip_drag
        .as_ref()
        .and_then(|d| d.target.as_ref())
        .and_then(|(target, edge)| {
            let rect = rects.iter().find(|(id, _)| id == target)?.1;
            let r = geometry::drop_rect(rect, *edge);
            Some(
                el().key("drop-preview")
                    .absolute(r.x, r.y)
                    .size(r.w, r.h)
                    .fill(Color {
                        a: 0.18,
                        ..c.accent
                    })
                    .border(2.0, c.accent),
            )
        });
    // Pointer input comes in view points, the panes' own (Rune's `zoom.rs`): drags need no
    // conversion at any zoom.
    // The tab area follows the pointer for the grips: as the panes' parent it covers none of
    // them. Only a grip, drawn while the pointer is on its pane's top edge, covers a terminal's
    // top row; presses anywhere else on that row reach the terminal.
    area.fill(c.border)
        .on_pointer_move_local(|s: &mut State, p, tx| s.tab_area_pointer(Some((p.x, p.y)), tx))
        .on_hover_change(|s: &mut State, inside, tx| {
            if !inside {
                s.tab_area_pointer(None, tx);
            }
        })
        .children(panes)
        .children(s.model.dividers().into_iter().map(divider))
        .children(grips)
        .children(preview)
}

/// A divider's handle, centered on its gap: dragging it resizes the panes beside it.
fn divider(d: geometry::Divider) -> El {
    let row = d.axis == herdr_shared::Axis::Row;
    let r = d.rect;
    let (x, y, w, h) = if row {
        let w = HANDLE.max(r.w);
        (r.x + (r.w - w) / 2.0, r.y, w, r.h)
    } else {
        let h = HANDLE.max(r.h);
        (r.x, r.y + (r.h - h) / 2.0, r.w, h)
    };
    // The divider's place along the axis when pressed; the pointer's travel moves it from there,
    // so it doesn't jump to where in the handle the press was.
    let start = if row { r.x } else { r.y };
    let container = d.container.clone();
    let index = d.index;
    el().key(format!("divider-{}-{index}", container.replace('/', ":")))
        .absolute(x, y)
        .size(w, h)
        .cursor(if row {
            Cursor::ResizeColumn
        } else {
            Cursor::ResizeRow
        })
        .on_drag(move |s: &mut State, drag: &mut Drag, tx| {
            if drag.phase == rune::Phase::Down {
                drag.data = start;
            }
            let at = drag.data + if row { drag.dx } else { drag.dy };
            s.divider_dragged(&container, index, drag.phase, at, drag.cancelled(), tx);
        })
}

/// A movable pane's grip, at the center of its top edge, while the pointer is on that edge (its
/// top [`GRIP_ROW`] points) or the grip is held: dragging it rearranges the panes
/// (`app:src/window/pane_drag.rs`).
fn grip(s: &State, pane: &str, rect: model::Rect) -> Option<El> {
    let c = &s.chrome;
    let held = s.grip_drag.as_ref().is_some_and(|d| d.pane == pane);
    if !held && s.grip_hover.as_deref() != Some(pane) {
        return None;
    }
    let (gx, gy) = (
        rect.x + (rect.w - GRIP.0) / 2.0,
        rect.y + (GRIP_ROW - GRIP.1) / 2.0,
    );
    let origin = (gx, gy);
    let dragged = pane.to_owned();
    let dot = if held { c.foreground } else { c.muted };
    let dots = |key: &str| {
        row()
            .key(key.to_owned())
            .gap(3.0)
            .children((0..3).map(|_| el().size(2.0, 2.0).radius(1.0).fill(dot)))
    };
    let grip = col()
        .key(format!("grip-{}", pane.replace('/', ":")))
        .absolute(gx, gy)
        .size(GRIP.0, GRIP.1)
        .radius(4.0)
        .fill(Color {
            a: 0.85,
            ..s.theme.background.map_or(c.canvas, color)
        })
        .hover(c.hover)
        .center()
        .gap(3.0)
        .cursor(if held { Cursor::Grabbing } else { Cursor::Grab })
        .child(dots("grip-top"))
        .child(dots("grip-bottom"))
        .on_drag(move |s: &mut State, drag: &mut Drag, tx| {
            let at = (
                origin.0 + drag.start_x + drag.dx,
                origin.1 + drag.start_y + drag.dy,
            );
            s.grip_dragged(&dragged, drag.phase, at, drag.cancelled(), tx);
        });
    Some(grip)
}

/// The pane's scrollbar, with `pane_scrollbars` on (`view::terminal::scrollbar`): over the
/// lane right of the grid, the height of the grid. Pressing or dragging scrolls there.
fn scrollbar(
    s: &State,
    viewer: &crate::state::Viewer,
    address: &str,
    pane_w: f32,
    face: &crate::view::terminal::font::Face,
) -> Option<El> {
    use crate::view::terminal::scrollbar::{INSET, LANE, THUMB, thumb};
    if !s.config.settings.pane_scrollbars {
        return None;
    }
    let pane = viewer.pane.as_ref()?;
    let snap = &pane.snapshot;
    if snap.active_screen == terminal::ActiveScreen::Alternate || snap.cols <= 4 {
        return None;
    }
    let (cell_w, cell_h) = face.cell_pt();
    let scale = face.scale;
    let track = f32::from(snap.rows) * cell_h;
    let sb = snap.scrollbar;
    let (top, len) = thumb(sb.total, sb.offset, sb.len, track, cell_h)?;
    let grid_right = pane.padding.left as f32 / scale + f32::from(snap.cols) * cell_w;
    let left = grid_right.min(pane_w - LANE).max(0.0);
    let lane_w = pane_w - left;
    let c = &s.chrome;
    let x = lane_w - THUMB - INSET;
    let dragging = address.to_owned();
    Some(
        el().key("scrollbar")
            .absolute(left, pane.padding.top as f32 / scale)
            .size(lane_w, track)
            .on_drag(move |s: &mut State, d, tx| s.scrollbar_drag(&dragging, d, track, tx))
            .child(
                el().absolute(x + THUMB / 2.0 - 0.5, 0.0)
                    .size(1.0, track)
                    .fill(c.muted.alpha(0.35))
                    .inert(),
            )
            .child(
                el().key("scrollbar-thumb")
                    .absolute(x, top)
                    .size(THUMB, len)
                    .radius(THUMB / 2.0)
                    .fill(c.muted)
                    .inert(),
            ),
    )
}

/// A pane's body with the handlers that give its terminal the keyboard and the mouse. `pane`
/// is [`crate::state::PEEK`] for the peek's terminal.
pub fn pane_input(body: El, pane: &str) -> El {
    let p = || pane.to_owned();
    let (focus, typing, releasing, dragging, hovering, wheeling, menu, dropping) =
        (p(), p(), p(), p(), p(), p(), p(), p());
    let leaving = p();
    body.focusable(true)
        .pointer_buttons([PointerButton::Middle])
        .on_key(move |s: &mut State, ev, tx| s.typed(&typing, ev, tx))
        .on_key_up(move |s: &mut State, ev, tx| s.released(&releasing, ev, tx))
        .on_focus_change(move |s: &mut State, gained, tx| s.pane_focus(&focus, gained, tx))
        .on_drag(move |s: &mut State, d, tx| s.pane_drag(&dragging, d, tx))
        .on_pointer_move_local(move |s: &mut State, p, tx| s.pane_hover(&hovering, p, tx))
        .on_hover_change(move |s: &mut State, inside, tx| {
            if !inside {
                s.pane_left(&leaving, tx)
            }
        })
        .on_wheel(move |s: &mut State, ev, tx| s.pane_wheel(&wheeling, &ev, tx))
        .on_secondary_press_local(move |s: &mut State, p, tx| s.pane_right_click(&menu, p, tx))
        .on_drop_files(move |s: &mut State, paths, tx| s.pane_dropped(&dropping, paths, tx))
}
