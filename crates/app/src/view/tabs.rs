//! The tab strip, as Eli's `eli-tabs` mod draws it (`feature-inventory.md` §4,
//! `cfg/mods/eli-tabs/main.js`):
//!
//! ```text
//! tabs  38 pt, drags the window: 10 pt in, 6 above the 29 pt pills and 3 below
//! ├─ tab-scroll   the shown space's shown tabs, scrolling sideways when they overflow
//! │  └─ tab-<id>  icon · name (at most 170 pt, cut short, its whole name as a tooltip) · " Z"
//! │               when zoomed · a spinner while working, a dot when blocked or done · × on
//! │               the selected one
//! ├─ prefix       the keys of a sequence part-way through (`ctrl-g …`)
//! └─ tab-machine  where the shown space runs: "This computer", or the machine's name
//! ```
//!
//! The mouse, on a pill: a click selects on release, a double-click renames, a press released
//! on another pill moves the tab to that one's place among all the space's tabs (by id, as the
//! shown set can change mid-drag), a right-click opens the tab menu and × closes it. The wheel
//! over the strip steps through the shown tabs as `tab.next` / `tab.prev` do, over hidden ones.
//! There is no new-tab button (Cmd+T, Ctrl+G T) and no Files or Changes buttons: Eli's
//! `workspace.panels` is empty.

use herdr_shared::{Displayed, Space, Tab, machine_of};
use model::{Action, SpaceExt, TabPill};
use rune::{Cx, Drag, El, LoopSpec, Phase, Tx, WheelEvent, el, icon, row, text};

use crate::State;
use crate::config::programs::Program;
use crate::state::tab_key;
use crate::view::icons::{CLOSE, MONITOR, SERVER, SPINNER};
use crate::view::shell::{BAR, label, program_icon};

/// A pill's height, corner radius and widest name.
pub const PILL: f32 = 29.0;
const RADIUS: f32 = 7.0;
const NAME_MAX: f32 = 170.0;
/// A pill's text, and the machine label's.
const PILL_TEXT: f32 = 12.8;
const MACHINE_TEXT: f32 = 11.8;
/// A program icon on a pill (eli-tabs' `size(14)`).
const ICON: f32 = 14.0;
/// The spinner and the ×.
const MARK: f32 = 12.0;
const DOT: f32 = 6.0;

/// Which tab a pill stands for, read back from a hit when a press is released over it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PillOf(String);

/// Whether a terminal runs Pi, as the program table says: its tab may go by its session name.
fn is_pi(s: &State) -> impl Fn(&herdr_proto::Terminal) -> bool + '_ {
    |t| s.config.programs.of_terminal(t).key == "pi"
}

/// The program a tab shows: its first agent's, else its first pane's (eli-tabs).
pub fn tab_program(s: &State, tab: &Tab) -> Option<Program> {
    let world = s.model.world();
    let programs: Vec<Program> = tab
        .root
        .panes()
        .into_iter()
        .filter_map(|pane| world.address_of(&pane.id))
        .filter_map(|address| world.terminal(address))
        .map(|record| s.config.programs.of_terminal(record))
        .collect();
    let first = programs.first().cloned();
    programs.into_iter().find(|p| p.agent).or(first)
}

pub fn view(s: &State, cx: &Cx<State>) -> El {
    let c = &s.chrome;
    let space = s.model.active_space();
    let pills: Vec<El> = space
        .map(|space| {
            s.model
                .tab_pills(space, is_pi(s))
                .into_iter()
                .filter_map(|p| Some(pill(s, space.tab(&p.tab)?, p, cx)))
                .collect()
        })
        .unwrap_or_default();
    row()
        .key("tabs")
        .w_full()
        .h(BAR)
        .pt(6.0)
        .pb(3.0)
        .px(10.0)
        .gap(3.0)
        .items_center()
        .fill(c.chrome)
        .window_drag()
        .on_wheel(wheel)
        .child(
            row()
                .key("tab-scroll")
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .gap(3.0)
                .items_center()
                .scroll_x()
                .children(pills),
        )
        .children(prefix(s))
        .children(space.map(|space| machine(s, space)))
}

fn pill(s: &State, tab: &Tab, p: TabPill, cx: &Cx<State>) -> El {
    let c = &s.chrome;
    let icon_el =
        tab_program(s, tab).and_then(|program| program_icon(s, &program, ICON, c.accent, cx));
    let shown = if p.zoomed {
        format!("{} Z", p.label)
    } else {
        p.label.clone()
    };
    let name = text(shown)
        .key("tab-name")
        .font(s.ui_font(PILL_TEXT, 400.0))
        .color(if p.active { c.foreground } else { c.muted })
        .max_w(NAME_MAX)
        .truncate()
        .help(p.label.clone());
    let status = match p.status {
        Some(Displayed::Working) => Some(
            icon(&SPINNER, MARK)
                .key("tab-working")
                .color(c.working)
                .looping(LoopSpec::rotation(0.9)),
        ),
        Some(state @ (Displayed::Blocked | Displayed::Done)) => Some(
            el().key(if state == Displayed::Blocked {
                "tab-blocked"
            } else {
                "tab-done"
            })
            .size(DOT, DOT)
            .radius(DOT / 2.0)
            .fill(if state == Displayed::Blocked {
                c.blocked
            } else {
                c.done
            }),
        ),
        Some(Displayed::Idle) | None => None,
    };
    let close = p.active.then(|| {
        let id = p.tab.clone();
        el().key("tab-close")
            .size(MARK, MARK)
            .focusable(false)
            .label("Close tab")
            .on_press_tx(move |s: &mut State, tx| s.act(Action::CloseTabOf(id.clone()), tx))
            .child(icon(&CLOSE, MARK).color(c.dim))
    });
    let (drag, menu) = (p.tab.clone(), p.tab.clone());
    row()
        .key(tab_key(&p.tab))
        .data(PillOf(p.tab.clone()))
        .h(PILL)
        .px(10.0)
        .gap(7.0)
        .items_center()
        .radius(RADIUS)
        .border(1.0, if p.active { c.border } else { c.chrome })
        .fill(if p.active { c.raised } else { c.chrome })
        .hover(c.hover)
        .focusable(false)
        .on_drag(move |s: &mut State, d: &mut Drag, tx| pressed(s, &drag, d, tx))
        .on_secondary_press(move |s: &mut State, _, tx| s.act(Action::TabMenu(menu.clone()), tx))
        .children(icon_el)
        .child(name)
        .children(status)
        .children(close)
}

/// A press on the pill of `tab`: the second of a double-click renames it; let go over a pill,
/// it shows that tab (its own) or moves there (another's); let go anywhere else, nothing.
fn pressed(s: &mut State, tab: &str, d: &mut Drag, tx: &mut Tx) {
    match d.phase {
        Phase::Down if d.clicks == 2 => s.act(Action::RenameTabOf(tab.to_owned()), tx),
        Phase::Up if !d.cancelled() => {
            let [x, y] = d.window_point();
            let from = tab.to_owned();
            tx.element_at_in("tabs", x, y, move |s: &mut State, picked, tx| {
                let Some(onto) = picked.and_then(|p| p.data::<PillOf>().map(|o| o.0.clone()))
                else {
                    return;
                };
                if onto == from {
                    s.act(Action::ShowTab(from), tx);
                    return;
                }
                let to = s
                    .model
                    .world()
                    .space_of(&onto)
                    .and_then(|space| space.tabs.iter().position(|t| t.id == onto));
                if let Some(to) = to {
                    s.act(Action::MoveTab { tab: from, to }, tx);
                }
            });
        }
        _ => {}
    }
}

/// The wheel over the strip: a step down or up of a point or more shows the next or previous
/// shown tab (eli-tabs' `on_scroll_wheel`); sideways scrolls go to the pills.
fn wheel(s: &mut State, e: WheelEvent, tx: &mut Tx) -> bool {
    if e.dy.abs() < 1.0 {
        return false;
    }
    s.act(
        if e.dy > 0.0 {
            Action::TabPrev
        } else {
            Action::TabNext
        },
        tx,
    );
    true
}

/// The prefix indicator: the keys of a sequence part-way through (`ctrl-g …`), until the next
/// key or the prefix timeout.
fn prefix(s: &State) -> Option<El> {
    let c = &s.chrome;
    let keys = s.model.mode().pending_keys()?;
    let typed: Vec<String> = keys.iter().map(|p| p.to_string()).collect();
    Some(
        el().key("prefix")
            .h(PILL)
            .px(10.0)
            .items_center()
            .radius(6.0)
            .fill(c.raised)
            .border(1.0, c.border)
            .child(
                text(format!("{} \u{2026}", typed.join(" ")))
                    .key("prefix-keys")
                    .font(label(s))
                    .color(c.foreground),
            ),
    )
}

/// Where the shown space runs, at the strip's end.
fn machine(s: &State, space: &Space) -> El {
    let c = &s.chrome;
    let (glyph, name) = match machine_of(&space.id) {
        Some(machine) => (&SERVER, machine.to_owned()),
        None => (&MONITOR, "This computer".to_owned()),
    };
    row()
        .key("tab-machine")
        .px(6.0)
        .gap(6.0)
        .items_center()
        .child(icon(glyph, 13.0).color(c.muted))
        .child(
            text(name)
                .key("tab-machine-name")
                .font(s.ui_font(MACHINE_TEXT, 400.0))
                .color(c.muted),
        )
}
