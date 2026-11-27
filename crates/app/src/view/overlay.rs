//! The frame every popup and dialog draws in, and the pieces they share: the filter or name
//! field, buttons, titles, notes, key caps and icons. Herdr app's overlays
//! (`app/src/window.rs` `render_overlays`), in the theme's colors:
//!
//! - A box in the popover color with a 1 pt border, radius 8 and a shadow. Its height is the
//!   most it takes (a share of the window when at most 1); it is as tall as what's in it.
//! - **Dialogs** (prompt, confirm, keys help, settings) are centered over the window dimmed 35%.
//! - **Popups** (the pickers, the palette) hang centered from `offset` points below the top;
//!   **menus** open at the pointer, kept inside the window. Neither dims the window.
//! - A press outside closes it (a right-click too); a press inside stays inside.
//!
//! The model holds the keyboard while one is open ([`model::Mode::Popup`]); the window focuses
//! the popup's text field ([`FIELD`]), or the box itself when it has none.

use model::{Action, PopupAction};
use rune::{Color, El, Icon, col, el, icon, row, text, text_input};

use crate::State;

/// The open popup's text field: the filter or the prompt's name. One at a time.
pub const FIELD: &str = "popup-filter";

/// The scrim behind the open popup, which closes it on a press.
pub const SCRIM: &str = "popup-scrim";

/// Where a popup's box goes. Sizes are in points at zoom 1; a height up to 1 is a share of the
/// window's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// Centered over the dimmed window, `height` tall (or as tall as its content).
    Dialog { width: f32, height: Option<f32> },
    /// Centered across, `offset` from the top, at most `height` tall.
    Top {
        offset: f32,
        width: f32,
        height: f32,
    },
    /// At a point (the pointer), at most `height` tall, kept inside the window.
    At {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
}

/// `body` in the popup frame at `place`. `key` is the box's: the focus goes there when the
/// popup has no field.
pub fn frame(s: &State, key: &str, place: Place, body: El) -> El {
    frame_closing(s, key, place, body, Action::Popup(PopupAction::Close))
}

/// [`frame`], with a press outside running `close` (copy mode's bar leaves copy mode).
pub fn frame_closing(s: &State, key: &str, place: Place, body: El, close: Action) -> El {
    let c = &s.chrome;
    let (vw, vh) = s.view_size();
    let span = |v: f32, full: f32| if v <= 1.0 { full * v } else { v };
    let width = match place {
        Place::Dialog { width, .. } | Place::Top { width, .. } | Place::At { width, .. } => {
            width.min(vw)
        }
    };
    let boxed = col()
        .key(key)
        .w(width)
        .fill(c.popover)
        .border(1.0, c.border)
        .radius(8.0)
        .shadow(8.0, 28.0, Color::BLACK.alpha(0.45))
        .clip_soft(8.0)
        .focusable(true)
        // A press inside stays inside.
        .on_press(|_: &mut State| {})
        .child(body);
    let again = close.clone();
    let scrim = el()
        .key(SCRIM)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .on_press_tx(move |s: &mut State, tx| s.act(close.clone(), tx))
        .on_secondary_press(move |s: &mut State, _, tx| s.act(again.clone(), tx));
    match place {
        Place::Dialog { height, .. } => {
            let boxed = match height {
                Some(h) => boxed.h(span(h, vh)),
                None => boxed.max_h(vh * 0.85),
            };
            scrim.center().fill(Color::BLACK.alpha(0.35)).child(boxed)
        }
        Place::Top { offset, height, .. } => {
            let x = ((vw - width) / 2.0).max(0.0);
            scrim.child(
                boxed
                    .max_h(span(height, vh).min(vh - offset))
                    .absolute(x, offset),
            )
        }
        Place::At { x, y, height, .. } => {
            let height = span(height, vh).min(vh);
            let x = x.min(vw - width).max(0.0);
            let y = y.min(vh - height).max(0.0);
            scrim.child(boxed.max_h(height).absolute(x, y))
        }
    }
}

/// The popup's text field (`popup-filter`) in Herdr app's input box: 36 tall, an accent
/// border, the terminal background, an optional icon before it and a hint after it. Its edits
/// become the popup's query ([`PopupAction::Query`]).
pub fn field(s: &State, placeholder: &str, glyph: Option<&'static Icon>, after: Option<El>) -> El {
    let c = &s.chrome;
    let input = text_input(&s.popup_field, s.ui_font(12.8, 400.0))
        .single_line()
        .key(FIELD)
        .label(placeholder)
        .placeholder(placeholder, c.dim)
        .color(c.foreground)
        .grow(1.0)
        .min_w(0.0)
        .on_edit(|s: &mut State| &mut s.popup_field)
        .on_input_change(|s: &mut State, tx| {
            let query = s.popup_field.text.clone();
            s.act(Action::Popup(PopupAction::Query(query)), tx);
        });
    row()
        .key("popup-field")
        .w_full()
        .h(36.0)
        .px(10.0)
        .gap(10.0)
        .items_center()
        .radius(9.0)
        .border(1.0, c.accent)
        .fill(s.background())
        .children(glyph.map(|g| icon(g, 15.0).color(c.dim)))
        .child(input)
        .children(after)
}

/// A popup's title row: the title, and something small at its right end.
pub fn title(s: &State, key: &str, title: &str, right: Option<El>) -> El {
    let c = &s.chrome;
    row()
        .w_full()
        .items_center()
        .child(
            text(title.to_owned())
                .key(key)
                .font(s.ui_font(15.0, 600.0))
                .color(c.foreground)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .children(right)
}

/// Small dim text: a hint, a "more" note, an empty list's note.
pub fn note(s: &State, text_: impl Into<String>) -> El {
    text(text_.into())
        .font(s.ui_font(11.0, 400.0))
        .color(s.chrome.dim)
}

/// "Esc" at a title's right end: a click closes the popup.
pub fn esc(s: &State) -> El {
    let c = &s.chrome;
    el().key("popup-esc")
        .px(6.0)
        .py(2.0)
        .radius(5.0)
        .hover_fill(c.hover)
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(|s: &mut State, tx| s.act(Action::Popup(PopupAction::Close), tx))
        .child(note(s, "Esc"))
}

/// A dialog button: 32 tall, the primary one in the accent color.
pub fn button(s: &State, key: &str, label: &str, primary: bool, action: PopupAction) -> El {
    let c = &s.chrome;
    let (fill, ink, edge) = if primary {
        (c.accent, c.accent_foreground, c.accent)
    } else {
        (c.raised, c.foreground, c.input)
    };
    row()
        .key(key)
        .h(32.0)
        .px(14.0)
        .items_center()
        .radius(8.0)
        .border(1.0, edge)
        .fill(fill)
        .cursor(rune::Cursor::Pointer)
        .hover(Color::WHITE.alpha(0.06))
        .on_press_tx(move |s: &mut State, tx| s.act(Action::Popup(action.clone()), tx))
        .child(
            text(label.to_owned())
                .font(s.ui_font(12.5, 500.0))
                .color(ink),
        )
}

/// A key in Eli's syntax as caps, one per part (`cmd-shift-p`: cmd, shift, p), as the palette
/// shows keys.
pub fn keycaps(s: &State, keys: &str) -> El {
    let c = &s.chrome;
    let mut caps = Vec::new();
    for chord in keys.split(' ') {
        let parts: Vec<&str> = if let Some(rest) = chord.strip_suffix("--") {
            rest.split('-').chain(["-"]).collect()
        } else {
            chord.split('-').collect()
        };
        caps.extend(parts.into_iter().filter(|p| !p.is_empty()).map(|p| {
            el().px(5.0)
                .radius(4.0)
                .border(1.0, c.border)
                .fill(c.raised)
                .child(note(s, p.to_owned()))
        }));
    }
    row().gap(3.0).items_center().children(caps)
}

/// A status's color, as the theme names it.
pub fn status_color(s: &State, status: Option<herdr_shared::Displayed>) -> Color {
    use herdr_shared::Displayed;
    let c = &s.chrome;
    match status {
        Some(Displayed::Blocked) => c.blocked,
        Some(Displayed::Done) => c.done,
        Some(Displayed::Working) => c.working,
        Some(Displayed::Idle) => c.idle,
        None => c.dim,
    }
}
