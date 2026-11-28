// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! The keys help (`cmd-/`, `feature-inventory.md` §10): a 640 × 520 dialog over the window
//! listing every binding from the keymap ([`model::help::rows`]), grouped spaces, tabs, panes,
//! agents, terminal, window (then popups and copy mode), with keys outside `global` and
//! `layout:eli` labelled by their section. Typing filters it; Esc, `cmd-/` again or a click
//! outside closes it.
//!
//! The dialog draws the model's list popup in the shared frame ([`crate::view::overlay`]): its
//! query is the filter field's text, its cursor the highlighted row. Keys reach the model through
//! the window's keymap first, so the field only types what the popup doesn't use. The wheel
//! scrolls the list and leaves the cursor; a cursor moved out of view brings its row back.

use model::help::{self, HelpRow};
use model::mode::ListPopup;
use rune::{El, ScrollSpec, col, row, text};

use crate::State;
use crate::view::overlay::{self, Place};
use crate::view::wheel::{self, Axis};

pub const WIDTH: f32 = 640.0;
pub const HEIGHT: f32 = 520.0;

/// The list's scroll viewport.
pub const ROWS: &str = "help-rows";

/// Row `i`'s key: what the window reveals when the cursor moves to it.
pub fn row_key(i: usize) -> String {
    format!("help-row-{i}")
}

pub const PLACE: Place = Place::Dialog {
    width: WIDTH,
    height: Some(HEIGHT),
};

/// The rows the help shows for its query.
pub fn shown(s: &State, list: &ListPopup) -> Vec<HelpRow> {
    let rows = help::rows(s.model.keymap());
    help::filtered(&rows, &list.query)
        .into_iter()
        .cloned()
        .collect()
}

pub fn view(s: &State, list: &ListPopup) -> El {
    let c = &s.chrome;
    let rows = shown(s, list);
    let label = |t: String| text(t).font(s.ui_font(13.0, 400.0));
    let mut body: Vec<El> = Vec::new();
    let mut group = "";
    for (i, r) in rows.iter().enumerate() {
        if r.group != group {
            group = r.group;
            body.push(
                text(group.to_uppercase())
                    .key(format!("help-group-{group}"))
                    .font(s.ui_font(11.0, 600.0))
                    .color(c.muted)
                    .pt(if i == 0 { 2.0 } else { 12.0 })
                    .pb(4.0)
                    .px(10.0),
            );
        }
        let on = i == list.cursor;
        let keys = r.keys.join(", ");
        let section = r.section.map(|name| {
            text(name.to_owned())
                .font(s.ui_font(11.0, 400.0))
                .color(c.muted)
                .px(6.0)
                .py(1.0)
                .radius(4.0)
                .border(1.0, c.border)
        });
        body.push(
            row()
                .key(row_key(i))
                .w_full()
                .h(26.0)
                .px(10.0)
                .gap(8.0)
                .items_center()
                .radius(5.0)
                .fill(if on { c.raised } else { c.popover })
                .child(
                    label(r.title.clone())
                        .color(c.foreground)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .children(section)
                .child(label(keys).color(c.muted)),
        );
    }
    if rows.is_empty() {
        body.push(
            text("No keys match")
                .key("help-empty")
                .font(s.ui_font(13.0, 400.0))
                .color(c.muted)
                .px(10.0)
                .py(8.0),
        );
    }
    // The help mod's header: "Keybinds" and Esc, then the filter field.
    col()
        .w_full()
        .h_full()
        .padding(18.0)
        .gap(14.0)
        .child(overlay::title(
            s,
            "help-title",
            "Keybinds",
            Some(overlay::esc(s)),
        ))
        .child(overlay::field(s, "Type to filter\u{2026}", None, None))
        .child(wheel::notches_at_once(
            col()
                .key(ROWS)
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec::default())
                .children(body),
            "help-rows",
            Axis::Y,
            s,
        ))
}
