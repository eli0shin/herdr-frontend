// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! The sidebar (`feature-inventory.md` §1 and §6): Eli's `eli-chrome` top row over his
//! `eli-agents` list, drawn from the model's [`model::agents::List`].
//!
//! ```text
//! sidebar   200 to 600 pt (366 by default), chrome; its right edge drags its width
//! ├─ brand  38 pt, drags the window: "Agents", the machine filter button, the sort button
//! └─ agents the list: section headers (not clickable), two-line agent rows, sub-agent rows
//! ```
//!
//! A row's click focuses its agent's pane; a sub-agent's click peeks it; an agent's icon or its
//! chevron folds its sub-agents. The sort button opens the sort menu under it. Sizes are the
//! mods' at zoom 1, in the settings' `ui_font`.

use model::agents::{AgentRow, Branch, Fold, Header, List, Row, Sort, SubRow, Tree};
use model::mode::{ListPopup, PopupState};
use model::{Action, Mode, Popup, PopupAction};
use rune::{
    Align, Color, Cursor, Cx, El, Icon, LoopSpec, Side, Spring, Transition, col, el, icon, row,
    text,
};

use crate::State;
use crate::config::theme::{Chrome, Rgba};
use crate::view::icons::{
    CHECK, CHEVRON_DOWN, CHEVRON_RIGHT, CLOCK, GIT_PULL_REQUEST, LAYERS, MONITOR, SPINNER,
};
use crate::view::shell::{BAR, icon_in};
use herdr_shared::Displayed;

/// The list's live titles: a touch under white (`LIVE_TITLE`).
pub(crate) const LIVE_TITLE: &str = "#F2F2F2";
/// A chip's fill: white at 5%.
const CHIP: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0x0d as f32 / 255.0,
};
/// The sort menu's size (`eli-chrome`'s `openOverlay` popup).
pub const SORT_MENU: (f32, f32) = (200.0, 140.0);
/// How wide the sidebar's draggable edge is, inside its right side.
const EDGE: f32 = 6.0;

/// A color at `a` of 255 (the mods' `color + "B3"`).
pub(crate) fn a(c: Color, a: u8) -> Color {
    c.alpha(f32::from(a) / 255.0)
}

pub(crate) fn hex(value: &str) -> Color {
    Rgba::parse(value).map_or(Color::WHITE, Rgba::color)
}

/// A status's color: working, blocked, done; anything else is dim.
pub(crate) fn tone(c: &Chrome, status: Displayed) -> Color {
    match status {
        Displayed::Working => c.working,
        Displayed::Blocked => c.blocked,
        Displayed::Done => c.done,
        Displayed::Idle => c.idle,
    }
}

/// An element key for an address or id: the `/` of a machine's (which separates key paths) as
/// `:`.
pub(crate) fn key(prefix: &str, id: &str) -> String {
    format!("{prefix}-{}", id.replace('/', ":"))
}

/// The sidebar, `width` wide.
pub fn view(s: &State, width: f32, cx: &Cx<State>) -> El {
    let c = &s.chrome;
    let list = &s.list;
    col()
        .key("sidebar")
        .w(width)
        .h_full()
        .fill(c.chrome)
        .child(brand(s))
        .child(agents(s, list, cx))
        .child(edge())
}

/// The top row: "Agents", the machine filter and the sort button (`eli-chrome`). Same 6 pt over
/// and 3 pt under as the tab strip, so the label lines up with the tabs.
fn brand(s: &State) -> El {
    let c = &s.chrome;
    let hidden = s.shared.hidden_machines().len();
    let sort = s.shared.sort();
    let machines = row()
        .key("ag-machines")
        .gap(4.0)
        .items_center()
        .help(if hidden > 0 {
            format!(
                "Agents hidden on {hidden} machine{} (Ctrl+G, /)",
                if hidden == 1 { "" } else { "s" }
            )
        } else {
            "Filter agents by machine (Ctrl+G, /)".into()
        })
        .on_press_tx(|s: &mut State, tx| s.act(Action::Open(Popup::AgentMachines), tx))
        .child(glyph(
            &MONITOR,
            14.0,
            if hidden > 0 { c.working } else { c.dim },
        ))
        .children((hidden > 0).then(|| {
            text(format!("{hidden} hidden"))
                .key("ag-machines-hidden")
                .font(s.ui_font(11.0, 400.0))
                .color(c.working)
        }));
    let sorter = row()
        .key("ag-sort")
        .gap(4.0)
        .items_center()
        .help(format!("Sort agents: {}", sort.label()))
        .on_press_tx(|s: &mut State, tx| s.act(Action::Open(Popup::SortMenu), tx))
        .child(glyph(sort_icon(sort), 14.0, c.dim));
    row()
        .key("brand")
        .w_full()
        .h(BAR)
        .pt(6.0)
        .pb(3.0)
        .pl(16.0)
        .pr(14.0)
        .gap(10.0)
        .items_center()
        .window_drag()
        .child(
            text("Agents")
                .key("brand-title")
                .font(s.ui_font(11.5, 500.0))
                .color(c.dim)
                .grow(1.0),
        )
        .child(machines)
        .child(sorter)
}

fn sort_icon(sort: Sort) -> &'static Icon {
    match sort {
        Sort::Priority => &CLOCK,
        Sort::Grouped => &LAYERS,
    }
}

/// A one-color glyph, `size` points square, in `color` (`view::icons`).
pub(crate) fn glyph(glyph: &'static Icon, size: f32, color: Color) -> El {
    icon(glyph, size).color(color)
}

/// The list, or why it's empty.
fn agents(s: &State, list: &List, cx: &Cx<State>) -> El {
    let c = &s.chrome;
    let jumping = matches!(s.model.mode(), Mode::Popup(PopupState::Jump(_)));
    let rows: Vec<El> = list
        .rows
        .iter()
        .map(|r| match r {
            Row::Header(h) => header(s, h),
            Row::Agent(a) => agent(s, list, a, jumping, cx),
            Row::Sub(k) => sub(s, k, cx),
        })
        .collect();
    let empty = list.empty.map(|why| {
        text(why.message())
            .key("agents-empty")
            .font(s.ui_font(12.0, 400.0))
            .color(c.dim)
            .w_full()
            .px(12.0)
    });
    col()
        .key("agents")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .children(empty)
        .child(
            col()
                .key("ag-list")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .px(8.0)
                .scroll(rune::ScrollSpec::default())
                .children(rows),
        )
}

/// A section header: its label in its status color and, in the priority view, its count. It
/// doesn't fold (`docs/plan.md`). The first sits right under the top row.
fn header(s: &State, h: &Header) -> El {
    let c = &s.chrome;
    let font = s.ui_font(11.0, 500.0);
    row()
        .key(key("hdr", &h.id))
        .w_full()
        .px(8.0)
        .pt(if h.first { 0.0 } else { 8.0 })
        .h(if h.first { 20.0 } else { 30.0 })
        .gap(6.0)
        .items_center()
        .child(
            text(h.label.clone())
                .key("label")
                .font(font)
                .color(h.status.map_or(c.dim, |st| tone(c, st)))
                .truncate(),
        )
        .children(
            h.count
                .map(|n| text(n.to_string()).key("count").font(font).color(c.dim)),
        )
}

/// A status mark: a spinner, "Needs you" (or a compact "!"), a done dot, an idle ring.
fn mark(s: &State, status: Option<Displayed>, compact: bool) -> El {
    let c = &s.chrome;
    match status {
        Some(Displayed::Working) => glyph(&SPINNER, 12.0, c.working)
            .key("spinner")
            .mx(1.0)
            .looping(LoopSpec::rotation(0.9)),
        Some(Displayed::Blocked) if compact => el()
            .key("needs-you")
            .size(14.0, 14.0)
            .radius(4.0)
            .fill(a(c.blocked, 0x26))
            .center()
            .child(text("!").font(s.ui_font(10.5, 700.0)).color(c.blocked)),
        Some(Displayed::Blocked) => row()
            .key("needs-you")
            .px(6.0)
            .h(18.0)
            .radius(5.0)
            .fill(a(c.blocked, 0x26))
            .items_center()
            .child(
                text("Needs you")
                    .font(s.ui_font(10.5, 600.0))
                    .color(c.blocked),
            ),
        Some(Displayed::Idle) => el()
            .key("idle")
            .size(7.0, 7.0)
            .mx(3.0)
            .radius(4.0)
            .border(1.5, c.idle),
        Some(Displayed::Done) => el()
            .key("done")
            .size(7.0, 7.0)
            .mx(3.0)
            .radius(4.0)
            .fill(c.done),
        None => el().key("none").size(7.0, 7.0).mx(3.0),
    }
}

/// An agent's program icon, `size` points: a one-color glyph in the foreground (eli-agents'
/// `icon(p, size, t.foreground)`; only the tab bar uses the program's color).
pub(crate) fn program_pic(s: &State, address: &str, size: f32, cx: &Cx<State>) -> Option<El> {
    let record = s.model.world().terminal(address)?;
    let program = s.config.programs.of_terminal(record);
    icon_in(s, &program, size, s.chrome.foreground, cx)
}

/// A worker's tree line: an elbow to its first line, going on for all but the last.
fn tree_line(c: &Chrome, branch: Branch) -> El {
    let elbow = el()
        .mt(-4.0)
        .h(14.0)
        .w_full()
        .border_sides([0.0, 0.0, 1.5, 1.5], c.dim)
        .corners([0.0, 0.0, 0.0, 6.0]);
    let on = (branch == Branch::Middle).then(|| {
        el().w_full()
            .grow(1.0)
            .mb(-4.0)
            .border_sides([0.0, 0.0, 0.0, 1.5], c.dim)
    });
    col()
        .key("tree")
        .w(9.0)
        .ml(8.0)
        .mr(4.0)
        .self_stretch()
        .child(elbow)
        .children(on)
}

/// The fold chevron; folded, the count too, in its most urgent sub-agent's color.
fn fold_control(s: &State, address: &str, fold: &Fold) -> El {
    let c = &s.chrome;
    let color = if fold.open {
        c.dim
    } else {
        fold.top.map_or(c.dim, |t| tone(c, t))
    };
    let target = address.to_owned();
    row()
        .key("fold")
        .h(18.0)
        .px(3.0)
        .gap(3.0)
        .items_center()
        .radius(4.0)
        .cursor(Cursor::Pointer)
        .hover_fill(c.hover)
        .on_press_tx(move |s: &mut State, tx| s.fold(&target, tx))
        .child(glyph(
            if fold.open {
                &CHEVRON_DOWN
            } else {
                &CHEVRON_RIGHT
            },
            11.0,
            color,
        ))
        .children((!fold.open).then(|| {
            text(fold.count.to_string())
                .font(s.ui_font(11.0, 400.0))
                .color(color)
        }))
}

/// A small filled tag on line 2 (workers, PRs).
pub(crate) fn chip(key: &str) -> El {
    row()
        .key(key)
        .h(17.0)
        .px(5.0)
        .gap(4.0)
        .items_center()
        .radius(4.0)
        .fill(CHIP)
}

/// An agent's two lines: icon, title, fold, status and label; then the space (in its color),
/// the ticket, the worker and PR chips, the machine and the time.
fn agent(s: &State, list: &List, r: &AgentRow, jumping: bool, cx: &Cx<State>) -> El {
    let c = &s.chrome;
    let live = r.status != Displayed::Idle;
    let edge = live.then(|| tone(c, r.status));
    let quiet = a(c.dim, 0xD9);
    let small = |size: f32| s.ui_font(size, 400.0);
    let pic = program_pic(s, &r.address, 16.0, cx).map(|pic| {
        let wrap = el().key("icon").child(pic);
        if r.fold.is_some() {
            let target = r.address.clone();
            wrap.on_press_tx(move |s: &mut State, tx| s.fold(&target, tx))
        } else {
            wrap
        }
    });
    let label = text(r.label.clone())
        .key("jump")
        .w(if list.wide_labels { 18.0 } else { 11.0 })
        .align(Align::End)
        .font(s.ui_font(12.0, if jumping { 600.0 } else { 400.0 }))
        .color(if jumping { c.working } else { a(c.dim, 0xB3) });
    let line1 = row()
        .key("line1")
        .w_full()
        .h(22.0)
        .gap(9.0)
        .items_center()
        .children(pic)
        .child(
            text(r.title.clone())
                .key("title")
                .font(s.ui_font(14.5, 400.0))
                // An idle agent's title brightens while it's selected, at once as the
                // selection moves.
                .motion(Spring::NONE)
                .color(if live || r.focused {
                    hex(LIVE_TITLE)
                } else {
                    a(c.foreground, 0xE6)
                })
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .children(r.fold.as_ref().map(|f| fold_control(s, &r.address, f)))
        .child(mark(s, Some(r.status), r.compact))
        .child(label);
    let space = r.space.as_ref().map(|name| {
        let color = r
            .space_color
            .as_deref()
            .and_then(Rgba::parse)
            .map_or(c.dim, Rgba::color);
        text(name.clone())
            .key("space")
            .font(small(12.5))
            .color(color)
            .min_w(0.0)
            .truncate()
    });
    let ticket = r.ticket.as_ref().map(|t| {
        let lead = if r.space.is_some() { " · " } else { "" };
        text(format!("{lead}{t}"))
            .key("ticket")
            .font(small(12.5))
            .color(c.dim)
    });
    let workers = (r.workers > 0).then(|| {
        chip("workers").child(
            text(format!(
                "{} worker{}",
                r.workers,
                if r.workers == 1 { "" } else { "s" }
            ))
            .font(small(11.5))
            .color(a(c.foreground, 0xC0)),
        )
    });
    let prs = r.prs.as_ref().map(|p| {
        let font = if p.paused {
            small(11.5).italic()
        } else {
            small(11.5)
        };
        chip("prs")
            .max_w(150.0)
            .help(p.tooltip.clone())
            .child(glyph(&GIT_PULL_REQUEST, 11.0, a(c.foreground, 0xB3)))
            .child(
                text(p.first.clone())
                    .font(font)
                    .color(a(c.foreground, 0xB3))
                    .min_w(0.0)
                    .truncate(),
            )
            .children((p.more > 0).then(|| text(format!("+{}", p.more)).font(font).color(c.dim)))
    });
    let machine = r.machine.as_ref().map(|m| {
        text(m.clone())
            .key("machine")
            .font(small(11.5))
            .color(quiet)
    });
    let ago = r.ago.as_ref().map(|t| {
        text(t.clone())
            .key("ago")
            .font(small(12.5))
            .color(edge.unwrap_or(quiet))
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
                .children(space)
                .children(ticket),
        )
        .children(workers)
        .children(prs)
        .children(machine)
        .children(ago);
    let body = col()
        .key("body")
        .grow(1.0)
        .min_w(0.0)
        .child(line1)
        .child(line2);
    let edge = edge.map(|t| {
        el().key("edge")
            .pin(Some(0.0), Some(8.0), None, Some(8.0))
            .w(2.5)
            .radius(2.0)
            .fill(t)
    });
    // The selected row: a faint wash and a thin outline under what it shows. They are their
    // own element, on the selected row only, so they move to another row at once when the
    // focused agent changes (a click, a space or tab switch) rather than fading out on one row
    // and in on the other, and the row's own motion (folds, re-sorts) stays as it is. The wash
    // gives way to the hover's fill, as the row's own fill would.
    let selected = r.focused.then(|| {
        el().key("selected")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(8.0)
            .border(1.0, a(c.working, 0x73))
            .transition(Transition::none())
            .child(
                el().key("wash")
                    .w_full()
                    .h_full()
                    .radius(8.0)
                    .fill(a(c.working, 0x12))
                    .group_opacity(1.0, 0.0)
                    .transition(Transition::none()),
            )
    });
    let target = r.address.clone();
    row()
        .key(key("ag", &r.address))
        .w_full()
        .px(8.0)
        .py(4.0)
        .radius(8.0)
        // Every row has the border so none shifts.
        .border(1.0, Color::TRANSPARENT)
        .hover_fill(c.hover)
        .hover_group()
        .cursor(Cursor::Pointer)
        .on_press_tx(move |s: &mut State, tx| s.act(Action::FocusAgent(target.clone()), tx))
        .children(selected)
        .children(edge)
        .children(r.line.map(|b| tree_line(c, b)))
        .child(body)
}

/// A sub-agent's row: tree lines, icon, name and status. Its click peeks it.
fn sub(s: &State, r: &SubRow, cx: &Cx<State>) -> El {
    let c = &s.chrome;
    let lead = r.lead.map(|goes_on| {
        el().key("lead")
            .w(9.0)
            .ml(8.0)
            .mr(35.0)
            .self_stretch()
            .border_sides([0.0, 0.0, 0.0, if goes_on { 1.5 } else { 0.0 }], c.dim)
    });
    let lines = r.lines.iter().map(|line| {
        let piece = el().self_start().mt(-4.0).w(9.0).h(17.0).mr(4.0);
        match line {
            Tree::Blank => piece,
            Tree::Pipe => piece.border_sides([0.0, 0.0, 0.0, 1.5], c.input),
            Tree::Middle | Tree::Last => piece
                .border_sides([0.0, 0.0, 1.5, 1.5], c.input)
                .corners([0.0, 0.0, 0.0, 6.0]),
        }
    });
    let pic = program_pic(s, &r.address, 14.0, cx).map(|p| el().mr(8.0).child(p));
    let target = r.address.clone();
    row()
        .key(key("sub", &r.address))
        .w_full()
        .h(26.0)
        .pl(if r.lead.is_some() { 8.0 } else { 39.0 })
        .pr(8.0)
        .radius(8.0)
        .items_center()
        .fill(if r.on { c.raised } else { Color::TRANSPARENT })
        .hover_fill(c.hover)
        .cursor(Cursor::Pointer)
        .on_press_tx(move |s: &mut State, tx| s.act(Action::Peek(target.clone()), tx))
        .children(lead)
        .children(lines)
        .children(pic)
        .child(
            text(r.name.clone())
                .key("name")
                .font(s.ui_font(12.5, 400.0))
                .color(if r.on { c.foreground } else { c.muted })
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(mark(s, r.status, r.compact))
}

/// The sidebar's right edge: dragging it sets the width, 200 to 600 pt.
fn edge() -> El {
    el().key("sidebar-edge")
        .pin(None, Some(0.0), Some(0.0), Some(0.0))
        .w(EDGE)
        .cursor(Cursor::ResizeColumn)
        .on_drag(|s: &mut State, drag, _tx| s.drag_sidebar(drag))
}

/// The sort menu, under the sort button: "Priority" and "Grouped by space", a check on the
/// current one. A click picks; ↑/↓, Enter and Esc work too.
pub fn sort_menu(s: &State, list: &ListPopup) -> El {
    let c = &s.chrome;
    let on = s.shared.sort();
    let rows = Sort::ALL.iter().enumerate().map(|(i, sort)| {
        let sort = *sort;
        row()
            .key(format!("sort-{}", sort.as_str()))
            .w_full()
            .h(31.0)
            .px(8.0)
            .gap(9.0)
            .radius(7.0)
            .items_center()
            .fill(if i == list.cursor {
                c.hover
            } else {
                Color::TRANSPARENT
            })
            .hover_fill(c.hover)
            .on_press_tx(move |s: &mut State, tx| {
                s.set_sort(sort);
                s.act(Action::Popup(PopupAction::Close), tx);
            })
            .child(glyph(sort_icon(sort), 14.0, c.muted))
            .child(
                text(sort.label())
                    .font(s.ui_font(12.8, 400.0))
                    .color(c.foreground)
                    .grow(1.0),
            )
            .children((sort == on).then(|| glyph(&CHECK, 14.0, c.foreground)))
    });
    let menu = col()
        .key("sort-menu")
        .w(SORT_MENU.0)
        .padding(5.0)
        .radius(8.0)
        .fill(c.popover)
        .border(1.0, c.border)
        .anchored("ag-sort", Side::Bottom, Align::End, 6.0)
        // A press inside stays inside.
        .on_press(|_: &mut State| {})
        .child(
            text("Sort agents")
                .key("sort-title")
                .font(s.ui_font(11.0, 400.0))
                .color(c.dim)
                .px(8.0)
                .pt(4.0)
                .pb(6.0),
        )
        .children(rows);
    el().key("sort-scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .on_press_tx(|s: &mut State, tx| s.act(Action::Popup(PopupAction::Close), tx))
        .child(menu)
}
