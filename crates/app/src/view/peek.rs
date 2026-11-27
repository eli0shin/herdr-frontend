//! The peek (`feature-inventory.md` §15, [`model::peek`]): a live terminal in an 80% × 80%
//! dialog over the window, under a 34 pt bar, as Herdr app's `terminal.peek` draws it with its
//! `peek` mod (`app:src/window.rs` `peek_terminal`, `cfg/mods/peek/main.js`).
//!
//! ```text
//! scrim        the whole window, dimmed; a press on it closes the peek
//! └─ peek      80% × 80%, centered
//!    ├─ bar    status glyph, icon, name, status word, "from <parent>", ↗ go to / ↗ keep, ✕
//!    └─ area   the terminal, with a pane's handlers: keys (Esc too), mouse, wheel, drops
//! ```
//!
//! The terminal takes the keyboard while no popup does, and takes the size as the pane in use
//! does: the area's grid. The glyph spins through braille frames every 90 ms while it works.

use std::ops::Range;

use herdr_shared::Displayed;
use model::Action;
use model::peek::Go;
use rune::{Color, Cursor, Cx, Cycle, El, col, el, row, text};
use terminal::{Snapshot, Width};

use crate::State;
use crate::state::{PEEK, PEEK_KEY};
use crate::view::overlay::status_color;
use crate::view::shell::{pane_input, program_icon};
use crate::view::terminal::look::color;

/// The bar's height (`peek_terminal`'s `{ "px": 34 }` block).
pub const BAR: f32 = 34.0;

/// The dialog's share of the window, each way.
pub const SHARE: f32 = 0.8;

/// The working glyph's frames, one every 90 ms.
const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A frame shows for a tenth of the 0.9 s loop around its phase.
const FRAME: &[(f32, f32)] = &[
    (0.0, 1.0),
    (0.05, 1.0),
    (0.0501, 0.0),
    (0.9499, 0.0),
    (0.95, 1.0),
    (1.0, 1.0),
];

pub fn view(s: &State, cx: &Cx<State>) -> Option<El> {
    let bar = s.model.peek_bar()?;
    let c = &s.chrome;
    let face = s.face(cx.env().scale);
    let viewer = s.viewers.get(&bar.address);
    let focused = s.window_active() && !s.model.mode().holds_keyboard();
    let background = s.theme.background.map_or(c.canvas, color);
    let body = match viewer.and_then(|v| v.pane.as_ref()) {
        Some(pane) => pane.view_as(PEEK_KEY, focused, &face, &s.look),
        None => el().key(PEEK_KEY).w_full().h_full().fill(background),
    };
    let area = el()
        .key("peek-area")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .fill(background)
        .clip_soft(0.0)
        .on_layout_tx(|s: &mut State, rect, tx| s.peek_laid_out(rect, tx))
        .child(pane_input(body, PEEK));
    // Herdr app's dialog frame, as the popups' (`view::overlay::frame`), but the terminal in
    // it takes the keyboard and its own close.
    let (vw, vh) = s.view_size();
    let dialog = col()
        .key("peek")
        .w(vw * SHARE)
        .h(vh * SHARE)
        .radius(8.0)
        .fill(c.popover)
        .border(1.0, c.border)
        .shadow(8.0, 28.0, Color::BLACK.alpha(0.45))
        // A hard edge: a soft one would fade the terminal's first column.
        .clip_soft(0.0)
        // Inside the border, so the terminal doesn't draw over it.
        .padding(1.0)
        // A press inside stays inside.
        .on_press(|_: &mut State| {})
        .child(bar_view(s, &bar, cx))
        .child(area);
    Some(
        el().key("peek-scrim")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .center()
            .fill(Color::BLACK.alpha(0.35))
            .on_press_tx(|s: &mut State, tx| s.act(Action::ClosePeek, tx))
            .child(dialog),
    )
}

/// Who it is and how it's doing, and what can be done with it.
fn bar_view(s: &State, bar: &model::peek::PeekBar, cx: &Cx<State>) -> El {
    let c = &s.chrome;
    let font = |weight| s.ui_font(12.0, weight);
    let record = s.model.world().terminal(&bar.address);
    let program = record.map(|t| s.config.programs.of_terminal(t));
    let name = record
        .and_then(|t| {
            [t.name.as_deref(), t.template.as_deref()]
                .into_iter()
                .flatten()
                .find(|n| !n.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| program.as_ref().map(|p| p.name.clone()))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| bar.address.clone());
    let tint = status_color(s, bar.status);
    let glyph = match bar.status {
        Some(Displayed::Working) => spinner(s, tint),
        status => {
            let mark = match status {
                Some(Displayed::Idle) => "○",
                Some(_) => "●",
                None => "·",
            };
            text(mark).font(font(400.0)).color(tint)
        }
    };
    let word = bar.status.map(|status| {
        let word = match status {
            Displayed::Working => "working",
            Displayed::Blocked => "needs you",
            Displayed::Done => "finished",
            Displayed::Idle => "idle",
        };
        text(word).key("peek-status").font(font(400.0)).color(tint)
    });
    let from = bar.from.as_ref().map(|from| {
        text(format!("from {from}"))
            .key("peek-from")
            .font(font(400.0))
            .color(c.dim)
            .min_w(0.0)
            .truncate()
    });
    let icon = program
        .as_ref()
        .and_then(|p| program_icon(s, p, 14.0, c.foreground, cx));
    let go = bar.go.as_ref().map(|go| {
        let label = match go {
            Go::Pane(_) => "↗ go to",
            Go::Keep(_) => "↗ keep",
        };
        button(s, "peek-keep", label, Action::KeepPeek)
    });
    row()
        .key("peek-bar")
        .w_full()
        .h(BAR)
        .px(12.0)
        .gap(8.0)
        .items_center()
        .fill(c.surface)
        .border_sides([0.0, 0.0, 1.0, 0.0], c.border)
        .child(el().key("peek-glyph").w(12.0).center().child(glyph))
        .children(icon)
        .child(
            text(name)
                .key("peek-name")
                .font(font(600.0))
                .color(c.foreground)
                .min_w(0.0)
                .truncate(),
        )
        .children(word)
        .children(from)
        .child(el().grow(1.0))
        .children(go)
        .child(button(s, "peek-close", "✕", Action::ClosePeek))
}

/// The braille spinner: its frames stacked, each shown for its tenth of the loop.
fn spinner(s: &State, tint: Color) -> El {
    let frames = SPIN.iter().enumerate().map(|(i, frame)| {
        text(*frame)
            .font(s.ui_font(12.0, 400.0))
            .color(tint)
            .absolute(0.0, 0.0)
            .cycle(Cycle {
                period: 0.9,
                phase: i as f32 / SPIN.len() as f32,
                stops: FRAME,
                still: if i == 0 { 1.0 } else { 0.0 },
                fps: SPIN.len() as f32 / 0.9,
            })
    });
    el().key("peek-spinner").size(12.0, 16.0).children(frames)
}

/// A bar button: muted, brighter under the pointer.
fn button(s: &State, key: &str, label: &str, action: Action) -> El {
    let c = &s.chrome;
    text(label.to_owned())
        .key(key.to_owned())
        .font(s.ui_font(12.0, 400.0))
        .color(c.muted)
        .px(8.0)
        .py(2.0)
        .radius(5.0)
        .hover_fill(c.hover)
        .cursor(Cursor::Pointer)
        .on_press_tx(move |s: &mut State, tx| s.act(action.clone(), tx))
}

/// The word under a point of a terminal's grid (device pixels from its top left), in the
/// soft-wrapped line it's on as the snapshot shows it ([`model::peek::word`]), and the word's
/// bytes in that line.
pub fn word_at(snapshot: &Snapshot, x: f32, y: f32) -> Option<(String, Range<usize>)> {
    if x < 0.0 || y < 0.0 || snapshot.cell_width == 0 || snapshot.cell_height == 0 {
        return None;
    }
    let col = (x / snapshot.cell_width as f32) as usize;
    let row = (y / snapshot.cell_height as f32) as usize;
    let rows = &snapshot.screen;
    if row >= rows.len() || col >= rows[row].cells.len() {
        return None;
    }
    let mut first = row;
    while first > 0 && rows[first - 1].wrapped {
        first -= 1;
    }
    let mut last = row;
    while last + 1 < rows.len() && rows[last].wrapped {
        last += 1;
    }
    let mut line = String::new();
    let mut at = None;
    for (y, r) in rows.iter().enumerate().take(last + 1).skip(first) {
        // The spacer half of a wide character is its first half's.
        let mut start = line.len();
        for (x, cell) in r.cells.iter().enumerate() {
            match cell.width {
                Width::SpacerTail | Width::SpacerHead => {}
                Width::Narrow | Width::Wide => {
                    start = line.len();
                    match cell.text.as_str() {
                        "" => line.push(' '),
                        t => line.push_str(t),
                    }
                }
            }
            if y == row && x == col {
                at = Some(start);
            }
        }
    }
    let range = model::peek::word(&line, at?)?;
    Some((line[range.clone()].to_owned(), range))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::replay;

    #[test]
    fn the_word_under_a_cell_comes_from_its_soft_wrapped_line() {
        // 15 columns: "devbox/t3" wraps after "dev".
        let mut s = replay(
            b"ran t12 and devbox/t3. done\r\ndevbox/t4",
            15,
            4,
            &Default::default(),
        );
        (s.cell_width, s.cell_height) = (10, 20);
        let (w, h) = (s.cell_width as f32, s.cell_height as f32);
        let word = |col: f32, row: f32| word_at(&s, (col + 0.5) * w, (row + 0.5) * h).map(|w| w.0);
        assert_eq!(word(5.0, 0.0).as_deref(), Some("t12"));
        assert_eq!(word(3.0, 0.0), None, "a space");
        assert_eq!(
            word(13.0, 0.0).as_deref(),
            Some("devbox/t3"),
            "across the wrap"
        );
        assert_eq!(word(1.0, 1.0).as_deref(), Some("devbox/t3"));
        assert_eq!(word(6.0, 1.0), None, "the dot after it");
        assert_eq!(
            word(2.0, 2.0).as_deref(),
            Some("devbox/t4"),
            "a row of its own"
        );
        assert_eq!(word(2.0, 9.0), None, "below the screen");
        assert_eq!(word_at(&s, -1.0, 0.0), None);
    }
}
