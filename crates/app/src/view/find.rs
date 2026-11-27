//! A terminal's find bar (`cmd-f`, `feature-inventory.md` §17), as Herdr app draws it
//! (`app:src/terminal.rs` `find-bar`): at the pane's top right, at most 460 wide and 38 high, on
//! the popover color with a border. Its field searches the screen and the scrollback as it's
//! typed into; the label says how many hits there are and which one is selected; ↑ (or Enter)
//! goes up into history, ↓ (or Shift+Enter) back down, and × (or Escape in the field) closes it.
//!
//! Clicking the terminal gives typing back to the program, while the bar and its hits stay.
//! Each terminal keeps its own query, and `cmd-f` selects it again.

use rune::{El, Key, KeyEvent, Tx, el, row, text, text_input};
use terminal::Found;

use crate::State;

/// The bar's widest, its height and its gap from the pane's edges, in points.
pub const WIDTH: f32 = 460.0;
pub const HEIGHT: f32 = 38.0;
pub const MARGIN: f32 = 8.0;
const BUTTON: f32 = 26.0;

/// The bar's field: its key, which the keyboard goes to on `cmd-f`.
pub fn field_key(pane: &str) -> String {
    format!("find-{}", pane.replace('/', ":"))
}

/// What the bar says about the search, as Herdr app words it.
pub fn label(query: &str, found: Option<Found>) -> String {
    let Some(found) = found.filter(|_| !query.is_empty()) else {
        return String::new();
    };
    if !found.complete {
        return if found.total == 0 {
            "Searching\u{2026}".into()
        } else {
            format!("{}+", found.total)
        };
    }
    match (found.total, found.selected) {
        (0, _) => "No matches".into(),
        (total, Some(i)) => format!("{} / {total}", i + 1),
        (total, None) => format!("{total} matches"),
    }
}

/// The bar over pane `pane` (`pane_w` points wide) showing terminal `address`, while it's open.
pub fn view(s: &State, pane: &str, address: &str, pane_w: f32) -> Option<El> {
    let viewer = s.viewers.get(address)?;
    let bar = viewer.find.as_ref().filter(|b| b.open)?;
    let c = &s.chrome;
    let found = viewer.pane.as_ref().and_then(|p| p.snapshot.find);
    let width = WIDTH.min((pane_w - 2.0 * MARGIN).max(0.0));
    let a = || address.to_owned();
    let (edit, typed, submit, keys) = (a(), a(), a(), a());
    let field = text_input(&bar.edit, s.ui_font(13.0, 400.0))
        .single_line()
        .key(field_key(pane))
        .label("Find in terminal")
        .placeholder("Find in terminal", c.muted)
        .color(c.foreground)
        .w_full()
        .on_edit(rune::edit_access(move |s: &mut State| s.find_edit(&edit)))
        .on_input_change(move |s: &mut State, tx| s.find_typed(&typed, tx))
        .on_submit(move |s: &mut State, tx| s.find_next(&submit, false, tx));
    let button = |key: &str, glyph: &str, address: String, f: fn(&mut State, &str, &mut Tx)| {
        el().key(key.to_owned())
            .size(BUTTON, BUTTON)
            .center()
            .radius(4.0)
            .hover_fill(c.hover)
            .label(glyph.to_owned())
            .on_press_tx(move |s: &mut State, tx| f(s, &address, tx))
            .child(
                text(glyph.to_owned())
                    .font(s.ui_font(13.0, 400.0))
                    .color(c.foreground),
            )
    };
    Some(
        row()
            .key(format!("find-bar-{}", pane.replace('/', ":")))
            .pin(None, Some(MARGIN), Some(MARGIN), None)
            .w(width)
            .h(HEIGHT)
            .px(4.0)
            .gap(4.0)
            .items_center()
            .radius(6.0)
            .fill(c.popover)
            .border(1.0, c.border)
            // A press on the bar stays on it: the terminal under it keeps its selection.
            .on_press(|_: &mut State| {})
            .on_key_preview(move |s: &mut State, ev: &KeyEvent, tx: &mut Tx| {
                bar_key(s, &keys, ev, tx)
            })
            .child(
                el().grow(1.0)
                    .min_w(48.0)
                    .px(6.0)
                    .h_full()
                    .items_center()
                    .child(field),
            )
            .child(
                text(label(&bar.edit.text, found))
                    .key("find-count")
                    .font(s.ui_font(11.0, 400.0))
                    .color(c.muted)
                    .w(58.0)
                    .truncate(),
            )
            .child(button("find-up", "\u{2191}", a(), |s, a, tx| {
                s.find_next(a, false, tx)
            }))
            .child(button("find-down", "\u{2193}", a(), |s, a, tx| {
                s.find_next(a, true, tx)
            }))
            .child(button("find-close", "\u{00d7}", a(), State::close_find)),
    )
}

/// Keys in the bar's field: Escape closes it, Shift+Enter goes back down (Enter is the
/// field's submit), and `cmd-f` selects the query again.
fn bar_key(s: &mut State, address: &str, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match &ev.key {
        Key::Escape => {
            s.close_find(address, tx);
            true
        }
        Key::Enter if ev.mods.shift => {
            s.find_next(address, true, tx);
            true
        }
        Key::Command('f' | 'F') => {
            let edit = s.find_edit(address);
            edit.anchor = 0;
            edit.focus = edit.text.len();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_reads_as_herdr_apps() {
        let found = |total, selected, complete| {
            Some(Found {
                total,
                selected,
                complete,
            })
        };
        assert_eq!(label("", found(3, None, true)), "");
        assert_eq!(label("x", found(0, None, false)), "Searching\u{2026}");
        assert_eq!(label("x", found(4, None, false)), "4+");
        assert_eq!(label("x", found(0, None, true)), "No matches");
        assert_eq!(label("x", found(9, None, true)), "9 matches");
        assert_eq!(label("x", found(9, Some(2), true)), "3 / 9");
    }
}
