//! The command palette (`cmd-shift-p`, Herdr app's `frame-palette`, `feature-inventory.md`
//! §10): 620 wide, at most 0.85 of the window, 60 from the top. Every command by name with its
//! key; empty, the recent ones first and then every group under its title; typed, the matches
//! best first, each with its group. Ten rows show at once; ↑/↓ stop at the ends, Page Up and
//! Page Down move ten.
//!
//! Rows and matching are [`model::popups::palette_rows`]; Enter runs the row through
//! [`model::Action::FromPalette`], so it is remembered as recent. A command that needs a name
//! asks for it in the prompt.

use model::mode::ListPopup;
use model::popups::{self, PaletteRow};
use rune::{El, col, row, text};

use crate::State;
use crate::view::icons::COMMAND;
use crate::view::list::{self, Item, More};
use crate::view::overlay::{self, Place};

pub const PLACE: Place = Place::Top {
    offset: 60.0,
    width: 620.0,
    height: 0.85,
};

/// Rows shown at once (`SHOWN`).
pub const SHOWN: usize = 10;

pub fn rows(s: &State, list: &ListPopup) -> Vec<PaletteRow> {
    popups::palette_rows(&s.model, &list.query)
}

/// Per item, whether it is a row and which: group titles go before a group's first row while
/// nothing is typed.
pub fn picks(rows: &[PaletteRow], typed: bool) -> Vec<Option<usize>> {
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if !typed && (i == 0 || r.head != rows[i - 1].head) {
            out.push(None);
        }
        out.push(Some(i));
    }
    out
}

pub fn view(s: &State, list: &ListPopup) -> El {
    let c = &s.chrome;
    let rows = rows(s, list);
    let typed = !list.query.trim().is_empty();
    let mut items = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if !typed && (i == 0 || r.head != rows[i - 1].head) {
            items.push(Item::head(
                row()
                    .key(format!("palette-head-{}", r.head.to_lowercase()))
                    .h(26.0)
                    .px(14.0)
                    .pb(4.0)
                    .items_end()
                    .child(text(r.head).font(s.ui_font(11.0, 600.0)).color(c.dim)),
            ));
        }
        let on = i == list.cursor;
        items.push(Item::row(
            row()
                .key(format!("palette-row-{i}"))
                .mx(6.0)
                .px(10.0)
                .h(32.0)
                .gap(10.0)
                .items_center()
                .radius(7.0)
                .fill(if on { c.selected } else { c.popover })
                .child(
                    text(r.title.clone())
                        .key(format!("palette-title-{i}"))
                        .font(s.ui_font(12.8, 400.0))
                        .color(if on { c.foreground } else { c.muted })
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .children(typed.then(|| overlay::note(s, r.group).font(s.ui_font(11.5, 400.0))))
                .children(r.key.as_deref().map(|k| overlay::keycaps(s, k))),
            i,
        ));
    }
    let body = if rows.is_empty() {
        text(format!(
            "Nothing matches \"{}\". Places are in the picker.",
            list.query
        ))
        .key("palette-empty")
        .font(s.ui_font(13.0, 400.0))
        .color(c.dim)
        .px(16.0)
        .py(14.0)
    } else {
        list::view(s, "palette-rows", items, SHOWN, More::None).mb(6.0)
    };
    col()
        .w_full()
        .min_h(0.0)
        .padding(18.0)
        .gap(10.0)
        .child(overlay::field(
            s,
            "Type a command…",
            Some(&COMMAND),
            Some(overlay::esc(s)),
        ))
        .child(body)
}
