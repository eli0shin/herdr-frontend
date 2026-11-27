//! The window, outside in, as Eli's `eli` layout has it (`feature-inventory.md` §1):
//!
//! ```text
//! root row
//! ├─ sidebar   366 pt: its 38 pt top row drags the window ("Agents"), the list below (later)
//! └─ work column
//!    ├─ tabs   38 pt: drags the window; the shown space's tabs (a placeholder strip), and at
//!    │         its right end the keys of a sequence part-way through (`ctrl-g …`)
//!    └─ main   the shown tab's panes, bare, with a 1 pt gap that shows the border color
//! ```
//!
//! No title bar, no status row: terminals reach the window's bottom edge. A pane is its
//! terminal's grid at the core's size, at the pane's top left; until its first snapshot it is
//! the terminal's background.

use rune::{Cx, El, TextStyle, col, el, row, text};

use crate::State;
use crate::state::{Status, pane_key, tab_key};
use crate::view::terminal::look::color;

/// The sidebar's top row and the tab strip: 38 pt, 6 above the 29 pt tabs and 3 below.
pub const BAR: f32 = 38.0;
const TAB: f32 = 29.0;

/// The chrome's text size, in points (`status`'s `ui_font_size`).
pub const UI_POINTS: f32 = 13.0;

/// The chrome's labels.
fn label() -> TextStyle {
    TextStyle::new(UI_POINTS, 400.0)
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let c = &s.chrome;
    let work = col()
        .key("work")
        .grow(1.0)
        .h_full()
        .child(tabs(s))
        .child(main(s, cx));
    row()
        .key("root")
        .w_full()
        .h_full()
        .fill(color(c.canvas))
        .children(s.sidebar.map(|w| sidebar(s, w)))
        .child(work)
}

fn sidebar(s: &State, width: f32) -> El {
    let c = &s.chrome;
    col()
        .key("sidebar")
        .w(width)
        .h_full()
        .fill(color(c.chrome))
        .child(
            row()
                .key("brand")
                .w_full()
                .h(BAR)
                .pt(6.0)
                .pb(3.0)
                .pl(14.0)
                .items_center()
                .window_drag()
                .child(
                    text("Agents")
                        .key("brand-title")
                        .font(label())
                        .color(color(c.muted)),
                ),
        )
        .child(col().key("agents").w_full().grow(1.0))
}

/// The tab strip: the shown space's tabs, the selected one raised. Clicking, closing and
/// reordering come with the tab strip ticket.
fn tabs(s: &State) -> El {
    let c = &s.chrome;
    let model = &s.model;
    let pills: Vec<El> = match model.active_space() {
        Some(space) => {
            let selected = model.shown_tab().map(|t| t.id.clone());
            model
                .shown_tabs(space)
                .into_iter()
                .map(|tab| {
                    let on = selected.as_deref() == Some(tab.id.as_str());
                    let name = model::tab_name(space, tab);
                    el().key(tab_key(&tab.id))
                        .h(TAB)
                        .px(12.0)
                        .items_center()
                        .radius(6.0)
                        .fill(color(if on { c.raised } else { c.chrome }))
                        .child(
                            text(if name.is_empty() { "Tab".into() } else { name })
                                .font(label())
                                .color(color(if on { c.text } else { c.muted })),
                        )
                })
                .collect()
        }
        None => Vec::new(),
    };
    row()
        .key("tabs")
        .w_full()
        .h(BAR)
        .pt(6.0)
        .pb(3.0)
        .pl(10.0)
        .pr(10.0)
        .gap(4.0)
        .fill(color(c.chrome))
        .window_drag()
        .children(pills)
        .child(el().grow(1.0))
        .children(prefix(s))
}

/// The prefix indicator: the keys of a sequence part-way through (`ctrl-g …`), at the tab
/// strip's right end until the next key or the prefix timeout.
fn prefix(s: &State) -> Option<El> {
    let c = &s.chrome;
    let keys = s.model.mode().pending_keys()?;
    let typed: Vec<String> = keys.iter().map(|p| p.to_string()).collect();
    Some(
        el().key("prefix")
            .h(TAB)
            .px(10.0)
            .items_center()
            .radius(6.0)
            .fill(color(c.raised))
            .border(1.0, color(c.border))
            .child(
                text(format!("{} \u{2026}", typed.join(" ")))
                    .key("prefix-keys")
                    .font(label())
                    .color(color(c.text)),
            ),
    )
}

/// The shown tab's panes where the model's geometry puts them, or what the core is doing.
fn main(s: &State, cx: &mut Cx<State>) -> El {
    let c = &s.chrome;
    let area = el()
        .key("main")
        .grow(1.0)
        .w_full()
        .clip_soft(0.0)
        .on_layout_tx(|s: &mut State, rect, tx| s.laid_out(rect, tx));
    let shown = s.shown_panes();
    if shown.is_empty() {
        let message = match &s.status {
            Status::Offline => "Not connected".to_owned(),
            Status::Connecting => "Connecting to herdr-core…".to_owned(),
            Status::Down(why) => format!("herdr-core unavailable: {why}"),
            Status::Connected if s.model.shown_tab().is_some() => String::new(),
            Status::Connected => "No spaces".to_owned(),
        };
        return area.fill(color(c.canvas)).center().child(
            text(message)
                .key("status")
                .font(label())
                .color(color(c.muted)),
        );
    }
    let face = s.face(cx.env().scale);
    let background = color(s.theme.background.unwrap_or(c.canvas));
    let panes = shown.into_iter().map(|(pane, rect, address)| {
        let viewer = address.as_deref().and_then(|a| s.viewers.get(a));
        let body = match viewer.and_then(|v| v.pane.as_ref()) {
            Some(terminal) => terminal.view(&face, &s.look),
            None => el().key(pane_key(&pane)).w_full().h_full().fill(background),
        };
        let (focus, typing) = (pane.clone(), pane.clone());
        el().absolute(rect.x, rect.y).size(rect.w, rect.h).child(
            body.focusable(true)
                .on_key(move |s: &mut State, ev, tx| s.typed(&typing, ev, tx))
                .on_focus_change(move |s: &mut State, gained, tx| s.pane_focus(&focus, gained, tx)),
        )
    });
    area.fill(color(c.border)).children(panes)
}
