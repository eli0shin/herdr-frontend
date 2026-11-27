//! The PR finder (`ctrl-g f`; Eli's `pr-finder` mod, `feature-inventory.md` §12): which agent
//! watches a PR. 620 wide, at most 0.6 of the window, 40 from the top, "Find a PR" with a "PR
//! number or repo…" field. Nine rows show at once, with "↓ N more".
//!
//! Each row reads like an Agents row (`view::sidebar`) without its status mark, jump label and
//! time: the program icon and title; the space in its color, the ticket and "N workers" for an
//! orchestrator, up to three matched PR chips and "+N", the machine. A live agent keeps its
//! status-colored edge. Rows and matching are [`model::prs`]; Enter or a click focuses the agent
//! ([`model::Action::FocusAgent`]).

use model::mode::ListPopup;
use model::prs::{self, PrRow};
use rune::{Color, Cx, El, col, el, row, text};

use crate::State;
use crate::config::theme::Rgba;
use crate::view::icons::GIT_PULL_REQUEST;
use crate::view::list::{self, Item, More};
use crate::view::overlay::{self, Place};
use crate::view::sidebar::{LIVE_TITLE, a, chip, glyph, hex, key, program_pic, tone};
use herdr_shared::Displayed;

pub const PLACE: Place = Place::Top {
    offset: 40.0,
    width: 620.0,
    height: 0.6,
};

/// Rows shown at once.
pub const SHOWN: usize = 9;
/// PR chips a row shows before "+N".
const CHIPS: usize = 3;

pub fn rows(s: &State, list: &ListPopup) -> Vec<PrRow> {
    prs::rows(&s.list, &list.query)
}

pub fn view(s: &State, list: &ListPopup, cx: &Cx<State>) -> El {
    let rows = rows(s, list);
    let items: Vec<Item> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| Item::row(card(s, r, i == list.cursor, cx), i))
        .collect();
    let empty = rows.is_empty().then(|| {
        let why = if list.query.trim().is_empty() {
            "No agent is watching a PR"
        } else {
            "No agent watches a PR like that"
        };
        overlay::note(s, why)
            .key("pr-empty")
            .px(14.0)
            .h(22.0)
            .items_center()
    });
    col()
        .w_full()
        .min_h(0.0)
        .padding(14.0)
        .gap(8.0)
        .child(overlay::title(
            s,
            "pr-finder-title",
            "Find a PR",
            Some(overlay::note(s, "↑/↓ · Enter · Esc")),
        ))
        .child(overlay::field(s, "PR number or repo…", None, None))
        .child(list::view(s, "pr-rows", items, SHOWN, More::Below).gap(2.0))
        .children(empty)
}

/// One agent's two lines; `on`: the cursor's row.
fn card(s: &State, r: &PrRow, on: bool, cx: &Cx<State>) -> El {
    let c = &s.chrome;
    let card = &r.card;
    let live = card.status != Displayed::Idle;
    let quiet = a(c.dim, 0xD9);
    let small = |size: f32| s.ui_font(size, 400.0);
    let pic = program_pic(s, &card.address, 16.0, cx);
    let line1 = row()
        .key("line1")
        .w_full()
        .h(22.0)
        .gap(9.0)
        .items_center()
        .children(pic)
        .child(
            text(card.title.clone())
                .key("title")
                .font(s.ui_font(14.5, 400.0))
                .color(if live || card.focused {
                    hex(LIVE_TITLE)
                } else {
                    a(c.foreground, 0xE6)
                })
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    let space_color = card
        .space_color
        .as_deref()
        .and_then(Rgba::parse)
        .map_or(c.dim, Rgba::color);
    let ticket = card
        .watch
        .as_ref()
        .and_then(|w| w.ticket.clone())
        .filter(|_| card.workers > 0)
        .map(|t| {
            text(format!(" · {t}"))
                .key("ticket")
                .font(small(12.5))
                .color(c.dim)
        });
    let workers = (card.workers > 0).then(|| {
        chip("workers").child(
            text(format!(
                "{} worker{}",
                card.workers,
                if card.workers == 1 { "" } else { "s" }
            ))
            .font(small(11.5))
            .color(a(c.foreground, 0xC0)),
        )
    });
    let paused = card.watch.as_ref().is_some_and(|w| w.paused);
    let font = if paused {
        small(11.5).italic()
    } else {
        small(11.5)
    };
    let chips = r.matched.iter().take(CHIPS).enumerate().map(|(j, pr)| {
        chip(&format!("pr-{j}"))
            .child(glyph(&GIT_PULL_REQUEST, 11.0, a(c.foreground, 0xB3)))
            .child(
                text(pr.label.clone())
                    .font(font)
                    .color(a(c.foreground, 0xB3)),
            )
    });
    let more = (r.matched.len() > CHIPS).then(|| {
        text(format!("+{}", r.matched.len() - CHIPS))
            .key("more")
            .font(small(12.5))
            .color(c.dim)
    });
    let machine = card.machine.as_ref().map(|m| {
        text(m.clone())
            .key("machine")
            .font(small(11.5))
            .color(quiet)
    });
    let line2 = row()
        .key("line2")
        .w_full()
        .h(19.0)
        .gap(6.0)
        .items_center()
        .child(
            row()
                .key("where")
                .grow(1.0)
                .min_w(0.0)
                .items_center()
                .child(
                    text(card.space_name.clone())
                        .key("space")
                        .font(small(12.5))
                        .color(space_color)
                        .min_w(0.0)
                        .truncate(),
                )
                .children(ticket),
        )
        .children(workers)
        .children(chips)
        .children(more)
        .children(machine);
    let edge = live.then(|| {
        el().key("edge")
            .pin(Some(0.0), Some(8.0), None, Some(8.0))
            .w(2.5)
            .radius(2.0)
            .fill(tone(c, card.status))
    });
    row()
        .key(key("pf", &card.address))
        .mx(4.0)
        .px(8.0)
        .py(4.0)
        .radius(8.0)
        .border(
            1.0,
            if on {
                a(c.working, 0x73)
            } else {
                Color::TRANSPARENT
            },
        )
        .fill(if on {
            a(c.working, 0x12)
        } else {
            Color::TRANSPARENT
        })
        .children(edge)
        .child(
            col()
                .key("body")
                .grow(1.0)
                .min_w(0.0)
                .child(line1)
                .child(line2),
        )
}
