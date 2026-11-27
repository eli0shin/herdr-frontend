// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! The machine popups (`feature-inventory.md` §9, `docs/machines.md`):
//!
//! - **The machine filter** (`ctrl-g /`, the Agents header's monitor button; Eli's
//!   `agent-machines` mod): 340 wide, 40 from the top, at most half the window. "Agents from",
//!   then "All machines", this computer and each machine, with a checkbox, "offline", the
//!   agent count and "⏎ only" on the cursor's row. ↑/↓ or J/K move, Space or a click shows or
//!   hides a machine, Enter shows only it and closes, Esc or Q closes.
//! - **Machines** (the palette's "Machines", the settings dialog's Machines row; Herdr app's
//!   Settings → Machines): a 520-wide dialog listing this computer and every machine with its
//!   line, hc version and what it needs run, then "Add a machine". Enter or a click opens a
//!   machine's menu: Connect now, a copy of each command it needs, Rename, Remove.
//! - **Add a machine** (the shipped `machine-add` mod): a 560-wide dialog. The field takes how
//!   ssh reaches it (`user@host`) and filters the ssh config's hosts; Enter adds the row's
//!   machine. A row says when it is connecting, and what went wrong; when the machine needs
//!   answers only a terminal can take, Enter copies the `hc machine add` command instead.
//!
//! Rows and rules are [`model::machines`]; the calls are the window's ([`crate::machines`]).

use herdr_proto::types::MachineView;
use model::machines::{self, AddRow, FilterRow, Line};
use model::mode::ListPopup;
use rune::{Color, El, col, el, row, text};

use crate::State;
use crate::view::icons::{CHECK, MONITOR, PLUS, SERVER};
use crate::view::list::{self, Item, More};
use crate::view::overlay::{self, Place};

pub const FILTER_PLACE: Place = Place::Top {
    offset: 40.0,
    width: 340.0,
    height: 0.5,
};

pub const MACHINES_PLACE: Place = Place::Dialog {
    width: 520.0,
    height: None,
};

pub const ADD_PLACE: Place = Place::Dialog {
    width: 560.0,
    height: None,
};

/// Rows of the Machines dialog and the add dialog that show at once.
pub const SHOWN: usize = 8;

// ---- The machine filter -------------------------------------------------------------------------

pub fn filter_rows(s: &State) -> Vec<FilterRow> {
    machines::filter_rows(
        s.model.world(),
        &s.list.machines,
        &s.shared.hidden_machines(),
    )
}

fn check(s: &State, on: bool) -> El {
    let c = &s.chrome;
    el().w(16.0)
        .h(16.0)
        .radius(4.0)
        .border(1.5, if on { c.working } else { c.dim })
        .fill(if on {
            c.working.alpha(0.2)
        } else {
            Color::TRANSPARENT
        })
        .center()
        .children(on.then(|| rune::icon(&CHECK, 12.0).color(c.foreground)))
}

pub fn filter_view(s: &State, list: &ListPopup) -> El {
    let c = &s.chrome;
    let small = |t: String| text(t).font(s.ui_font(11.0, 400.0)).color(c.dim);
    let items: Vec<Item> = filter_rows(s)
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let here = i == list.cursor;
            let machine = r.key.is_some();
            let agents = format!("{} agent{}", r.agents, if r.agents == 1 { "" } else { "s" });
            Item::toggle(
                row()
                    .key(format!("am-{i}"))
                    .mx(4.0)
                    .px(10.0)
                    .h(32.0)
                    .gap(10.0)
                    .items_center()
                    .radius(7.0)
                    .fill(if here { c.selected } else { c.popover })
                    .child(check(s, r.shown).key(format!("am-check-{i}")))
                    .child(
                        text(r.label)
                            .key(format!("am-label-{i}"))
                            .font(s.ui_font(13.0, 400.0))
                            .color(if r.shown { c.foreground } else { c.muted })
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .children(
                        (machine && r.offline).then(|| small("offline".into()).key("am-offline")),
                    )
                    .children(machine.then(|| small(agents).key(format!("am-count-{i}"))))
                    .children((here && machine).then(|| small("⏎ only".into()).key("am-only"))),
                i,
            )
        })
        .collect();
    let n = items.len();
    col()
        .w_full()
        .min_h(0.0)
        .padding(14.0)
        .gap(8.0)
        .child(overlay::title(
            s,
            "am-title",
            "Agents from",
            Some(overlay::note(s, "↑/↓ · Space · Enter only · Esc")),
        ))
        .child(list::view(s, "am-rows", items, n, More::None))
}

// ---- Machines -----------------------------------------------------------------------------------

/// One Machines dialog row.
#[derive(Clone, Debug, PartialEq)]
pub enum MachinesRow {
    ThisComputer,
    Machine(Box<MachineView>),
    Add,
}

/// This computer, every machine `machine.list` has (temporary ones left out), then "Add a
/// machine".
pub fn machines_rows(s: &State) -> Vec<MachinesRow> {
    let mut out = vec![MachinesRow::ThisComputer];
    out.extend(
        s.model
            .world()
            .machines()
            .filter_map(|(_, m)| m.status.clone())
            .map(|v| MachinesRow::Machine(Box::new(v))),
    );
    out.push(MachinesRow::Add);
    out
}

/// Where the dialog's cursor starts: on the machine whose menu it last opened.
pub fn start(s: &State, rows: &[MachinesRow]) -> usize {
    let Some(picked) = &s.machines.picked else {
        return 0;
    };
    rows.iter()
        .position(|r| matches!(r, MachinesRow::Machine(v) if v.machine.name == *picked))
        .unwrap_or(0)
}

fn dot(color: Color) -> El {
    el().w(7.0).h(7.0).radius(3.5).fill(color)
}

pub fn machines_view(s: &State, list: &ListPopup) -> El {
    let c = &s.chrome;
    let rows = machines_rows(s);
    let others = rows.len() - 2;
    let name = |t: String, i: usize| {
        text(t)
            .key(format!("machine-name-{i}"))
            .font(s.ui_font(14.0, 400.0))
            .color(c.foreground)
            .truncate()
    };
    let line_text =
        |t: String, color: Color| text(t).font(s.ui_font(12.5, 400.0)).color(color).w_full();
    let items: Vec<Item> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let here = i == list.cursor;
            let frame = |body: El| {
                row()
                    .key(format!("machine-row-{i}"))
                    .mx(4.0)
                    .px(10.0)
                    .py(9.0)
                    .gap(10.0)
                    .items_center()
                    .radius(7.0)
                    .fill(if here { c.selected } else { c.popover })
                    .child(body)
            };
            let body = match r {
                MachinesRow::ThisComputer => {
                    let (color, state) = match (&s.status, &s.hello) {
                        (crate::state::Status::Connected, Some(hello)) => {
                            (c.done, format!("Connected · hc {}", hello.version))
                        }
                        (crate::state::Status::Connected, None) => {
                            (c.done, "Connected · hc version unknown".to_owned())
                        }
                        _ => (c.blocked, "Not connected".to_owned()),
                    };
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(10.0)
                        .items_center()
                        .child(dot(color))
                        .child(rune::icon(&MONITOR, 15.0).color(c.muted))
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(3.0)
                                .child(name(machines::THIS_COMPUTER.into(), i))
                                .child(line_text(state, c.dim).key(format!("machine-detail-{i}"))),
                        )
                }
                MachinesRow::Machine(v) => {
                    let color = match machines::line(&v.state) {
                        Line::Up => c.done,
                        Line::Connecting => c.working,
                        Line::Down => c.blocked,
                    };
                    let busy = s.machines.busy.get(&v.machine.name).map(|call| call.busy());
                    let needs = machines::needs(v).into_iter().enumerate().map(|(n, need)| {
                        col()
                            .w_full()
                            .gap(2.0)
                            .child(line_text(need.text, c.blocked).wrap())
                            .child(
                                line_text(need.command, c.foreground)
                                    .wrap()
                                    .key(format!("machine-need-{i}-{n}")),
                            )
                    });
                    let why = machines::why(v).map(|w| {
                        line_text(w.to_owned(), c.dim)
                            .wrap()
                            .key(format!("machine-why-{i}"))
                    });
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(10.0)
                        .items_center()
                        .child(dot(color).key(format!("machine-dot-{i}")))
                        .child(rune::icon(&SERVER, 15.0).color(c.muted))
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(3.0)
                                .child(name(v.machine.name.clone(), i))
                                .child(
                                    line_text(machines::detail(v), c.dim)
                                        .key(format!("machine-detail-{i}"))
                                        .truncate(),
                                )
                                .children(why)
                                .children(needs),
                        )
                        .children(
                            busy.map(|b| overlay::note(s, b).key(format!("machine-busy-{i}"))),
                        )
                }
                MachinesRow::Add => row()
                    .grow(1.0)
                    .gap(10.0)
                    .items_center()
                    .child(el().w(7.0))
                    .child(rune::icon(&PLUS, 15.0).color(c.muted))
                    .child(name("Add a machine".into(), i)),
            };
            Item::row(frame(body), i)
        })
        .collect();
    col()
        .w_full()
        .min_h(0.0)
        .padding(18.0)
        .gap(10.0)
        .child(overlay::title(
            s,
            "machines-title",
            "Machines",
            Some(overlay::note(s, "↑/↓ · Enter · Esc")),
        ))
        .child(
            overlay::note(
                s,
                "Your computers, reached over ssh. Their terminals keep running when you \
                 disconnect.",
            )
            .font(s.ui_font(12.5, 400.0))
            .w_full()
            .wrap(),
        )
        .child(list::view(s, "machines-rows", items, SHOWN, More::Both))
        .children((others == 0).then(|| {
            overlay::note(s, "No other machines yet.")
                .key("machines-empty")
                .px(14.0)
        }))
}

// ---- Add a machine ------------------------------------------------------------------------------

pub fn add_rows(s: &State, list: &ListPopup) -> Vec<AddRow> {
    let hosts = s.machines.hosts.as_deref().unwrap_or_default();
    machines::add_rows(s.model.world(), hosts, &list.query)
}

pub fn add_view(s: &State, list: &ListPopup) -> El {
    let c = &s.chrome;
    let rows = add_rows(s, list);
    let items: Vec<Item> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let here = i == list.cursor;
            let busy = s.machines.busy.get(&r.target).map(|call| call.busy());
            let failed = s.machines.failed.get(&r.target);
            let right = match (busy, failed) {
                (Some(b), _) => overlay::note(s, b),
                (None, Some(f)) if f.command.is_some() => overlay::note(s, "⏎ copy"),
                (None, _) => overlay::note(s, "⏎ add"),
            };
            let label = if r.typed {
                format!("Add {}", r.target)
            } else {
                r.target.clone()
            };
            let line = row()
                .w_full()
                .h(28.0)
                .gap(10.0)
                .items_center()
                .child(rune::icon(&SERVER, 15.0).color(c.muted))
                .child(
                    text(label)
                        .key(format!("add-name-{i}"))
                        .font(s.ui_font(13.0, 400.0))
                        .color(c.foreground),
                )
                .child(
                    overlay::note(s, r.sub.clone())
                        .font(s.ui_font(12.0, 400.0))
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(right.key(format!("add-state-{i}")));
            let message = failed.map(|f| {
                text(f.message.clone())
                    .key(format!("add-message-{i}"))
                    .font(s.ui_font(12.0, 400.0))
                    .color(c.blocked)
                    .w_full()
                    .wrap()
                    .pl(25.0)
                    .pb(4.0)
            });
            Item::row(
                col()
                    .key(format!("add-row-{i}"))
                    .mx(4.0)
                    .px(10.0)
                    .py(4.0)
                    .radius(7.0)
                    .fill(if here { c.selected } else { c.popover })
                    .child(line)
                    .children(message),
                i,
            )
        })
        .collect();
    let heading = match &s.machines.hosts {
        None => "Reading ~/.ssh/config…",
        Some(hosts) if hosts.is_empty() => "Your ssh config lists no hosts",
        Some(_) => "From your ssh config",
    };
    col()
        .w_full()
        .min_h(0.0)
        .padding(18.0)
        .gap(10.0)
        .child(overlay::title(
            s,
            "add-title",
            "Add a machine",
            Some(overlay::esc(s)),
        ))
        .child(
            overlay::note(
                s,
                "A VPS, a Mac mini, a box at home: herdr-core reaches it over ssh, and agents \
                 there keep running while your laptop sleeps.",
            )
            .font(s.ui_font(12.8, 400.0))
            .w_full()
            .wrap(),
        )
        .child(overlay::field(
            s,
            "user@host, or a Host from ~/.ssh/config",
            None,
            None,
        ))
        .child(overlay::note(s, heading).key("add-heading"))
        .child(list::view(s, "add-rows", items, SHOWN, More::Both))
}
