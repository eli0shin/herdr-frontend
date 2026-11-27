//! The keys help (`cmd-/`, `feature-inventory.md` §10): a 640 × 520 dialog over the window
//! listing every binding from the keymap ([`model::help::rows`]), grouped spaces, tabs, panes,
//! agents, terminal, window (then popups and copy mode), with keys outside `global` and
//! `layout:eli` labelled by their section. Typing filters it; Esc, `cmd-/` again or a click
//! outside closes it.
//!
//! The dialog draws the model's list popup: its query is the filter field's text, its cursor
//! the highlighted row. Keys reach the model through the window's keymap first, so the field
//! only types what the popup doesn't use.

use model::help::{self, HelpRow};
use model::mode::ListPopup;
use model::{Action, PopupAction};
use rune::{El, ScrollSpec, TextStyle, col, el, row, text, text_input};

use crate::State;
use crate::view::terminal::look::color;

pub const WIDTH: f32 = 640.0;
pub const HEIGHT: f32 = 520.0;

/// The filter field's key: the popup's focus goes there.
pub const FILTER: &str = "popup-filter";

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
    let label = |t: String| text(t).font(TextStyle::new(13.0, 400.0));
    let mut body: Vec<El> = Vec::new();
    let mut group = "";
    for (i, r) in rows.iter().enumerate() {
        if r.group != group {
            group = r.group;
            body.push(
                text(group.to_uppercase())
                    .key(format!("help-group-{group}"))
                    .font(TextStyle::new(11.0, 600.0))
                    .color(color(c.muted))
                    .pt(if i == 0 { 2.0 } else { 12.0 })
                    .pb(4.0)
                    .px(10.0),
            );
        }
        let on = i == list.cursor;
        let keys = r.keys.join(", ");
        let section = r.section.map(|s| {
            text(s.to_owned())
                .font(TextStyle::new(11.0, 400.0))
                .color(color(c.muted))
                .px(6.0)
                .py(1.0)
                .radius(4.0)
                .border(1.0, color(c.border))
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
                .fill(color(if on { c.raised } else { c.chrome }))
                .child(
                    label(r.title.clone())
                        .color(color(c.text))
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .children(section)
                .child(label(keys).color(color(c.muted))),
        );
    }
    if rows.is_empty() {
        body.push(
            text("No keys match")
                .key("help-empty")
                .font(TextStyle::new(13.0, 400.0))
                .color(color(c.muted))
                .px(10.0)
                .py(8.0),
        );
    }
    let filter = text_input(&s.help_filter, TextStyle::new(14.0, 400.0))
        .single_line()
        .key(FILTER)
        .label("Filter keys")
        .placeholder("Filter keys", color(c.muted))
        .color(color(c.text))
        .w_full()
        .on_edit(|s: &mut State| &mut s.help_filter)
        .on_input_change(|s: &mut State, tx| {
            let query = s.help_filter.text.clone();
            s.act(Action::Popup(PopupAction::Query(query)), tx);
        });
    let dialog = col()
        .key("help")
        .w(WIDTH)
        .h(HEIGHT)
        .radius(10.0)
        .fill(color(c.chrome))
        .border(1.0, color(c.border))
        .padding(12.0)
        .gap(10.0)
        // A press inside stays inside.
        .on_press(|_: &mut State| {})
        .child(
            row()
                .w_full()
                .items_center()
                .gap(10.0)
                .child(
                    text("Keys")
                        .key("help-title")
                        .font(TextStyle::new(14.0, 600.0))
                        .color(color(c.text)),
                )
                .child(
                    el().grow(1.0)
                        .px(8.0)
                        .py(5.0)
                        .radius(6.0)
                        .fill(color(c.raised))
                        .child(filter),
                ),
        )
        .child(
            col()
                .key("help-rows")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec::default())
                .children(body),
        );
    el().key("help-scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .fill(rune::Color::BLACK.alpha(0.35))
        .on_press_tx(|s: &mut State, tx| s.act(Action::Popup(PopupAction::Close), tx))
        .child(dialog)
}
