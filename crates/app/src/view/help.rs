//! The keys help (`cmd-/`, `feature-inventory.md` §10): a 640 × 520 dialog over the window
//! listing every binding from the keymap ([`model::help::rows`]), grouped spaces, tabs, panes,
//! agents, terminal, window (then popups and copy mode), with keys outside `global` and
//! `layout:eli` labelled by their section. Typing filters it; Esc, `cmd-/` again or a click
//! outside closes it.
//!
//! The dialog draws the model's list popup in the shared frame ([`crate::view::overlay`]): its
//! query is the filter field's text, its cursor the highlighted row. Keys reach the model through
//! the window's keymap first, so the field only types what the popup doesn't use. Unlike the
//! pickers, the whole list scrolls (the wheel moves it; the cursor doesn't).

use model::help::{self, HelpRow};
use model::mode::ListPopup;
use rune::{Cx, El, ScrollSpec, col, row, text};

use crate::State;
use crate::view::overlay::{self, Place};

pub const WIDTH: f32 = 640.0;
pub const HEIGHT: f32 = 520.0;

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

pub fn view(s: &State, list: &ListPopup, cx: &Cx<State>) -> El {
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
                .key(format!("help-row-{i}"))
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
    col()
        .w_full()
        .h_full()
        .padding(12.0)
        .gap(10.0)
        .child(
            row()
                .w_full()
                .items_center()
                .gap(10.0)
                .child(
                    text("Keys")
                        .key("help-title")
                        .font(s.ui_font(14.0, 600.0))
                        .color(c.foreground),
                )
                .child(rune::el().grow(1.0).child(overlay::field(
                    s,
                    "Filter keys",
                    None,
                    None,
                    cx,
                ))),
        )
        .child(
            col()
                .key("help-rows")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec::default())
                .children(body),
        )
}
