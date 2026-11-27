//! The space picker (`ctrl-g s`, Eli's `space-picker` mod, `feature-inventory.md` §3): 520 wide,
//! at most 0.6 of the window, 40 from the top, titled "Spaces" with a "Go to space…" field.
//! Rows: the space's number (1-9), its status glyph, its name (semibold when shown), its machine
//! when remote and "N tabs". Twelve show at once, with "↑ N more" / "↓ N more".
//!
//! Rows and matching are [`model::popups::space_rows`]; Enter goes to the space
//! ([`model::Action::ShowSpace`]).

use model::mode::ListPopup;
use model::popups::{self, SpaceRow};
use rune::{El, col, el, row, text};

use crate::State;
use crate::view::list::{self, Item, More};
use crate::view::overlay::{self, Place};

pub const PLACE: Place = Place::Top {
    offset: 40.0,
    width: 520.0,
    height: 0.6,
};

/// Rows shown at once.
pub const SHOWN: usize = 12;

pub fn rows(s: &State, list: &ListPopup) -> Vec<SpaceRow> {
    popups::space_rows(&s.model, &list.query)
}

pub fn view(s: &State, list: &ListPopup) -> El {
    let c = &s.chrome;
    let rows = rows(s, list);
    let label = |t: String| text(t).font(s.ui_font(13.0, 400.0));
    let items: Vec<Item> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let on = i == list.cursor;
            let number = r.number.map(|n| n.to_string()).unwrap_or_default();
            let name = text(r.name.clone())
                .key(format!("space-name-{i}"))
                .font(s.ui_font(13.0, if r.shown { 600.0 } else { 400.0 }))
                .color(c.foreground)
                .grow(1.0)
                .min_w(0.0)
                .truncate();
            let tabs = format!("{} tab{}", r.tabs, if r.tabs == 1 { "" } else { "s" });
            Item::row(
                row()
                    .key(format!("space-row-{i}"))
                    .mx(4.0)
                    .px(10.0)
                    .h(32.0)
                    .gap(10.0)
                    .items_center()
                    .radius(7.0)
                    .fill(if on { c.selected } else { c.popover })
                    .child(el().w(14.0).child(label(number).color(c.dim)))
                    .child(
                        el().w(12.0).child(
                            label(popups::glyph(r.status).into())
                                .color(overlay::status_color(s, r.status)),
                        ),
                    )
                    .child(name)
                    .children(r.machine.clone().map(|m| overlay::note(s, m)))
                    .child(overlay::note(s, tabs)),
                i,
            )
        })
        .collect();
    let empty = rows.is_empty().then(|| {
        overlay::note(s, "No space matches")
            .key("space-empty")
            .px(14.0)
            .h(20.0)
    });
    col()
        .w_full()
        .min_h(0.0)
        .padding(14.0)
        .gap(8.0)
        .child(overlay::title(
            s,
            "space-picker-title",
            "Spaces",
            Some(overlay::note(s, "↑/↓ · Enter · Esc")),
        ))
        .child(overlay::field(s, "Go to space…", None, None))
        .child(list::view(s, "space-rows", items, SHOWN, More::Both))
        .children(empty)
}
