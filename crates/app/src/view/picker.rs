//! The tab and agent picker (`cmd-p`, Herdr app's shipped `picker`, `feature-inventory.md` §10):
//! 640 wide, at most 0.8 of the window, 40 from the top, "Go to…". Spaces with an agent first,
//! each under its avatar and name, then "earlier: N quiet spaces ▾", then "new tab", "new space"
//! and "new space on <machine>". Tab rows: status glyph, name, "needs you" / "finished", the
//! agents' programs and "+N" sub-agents. Ctrl+D closes the selected tab.
//!
//! Rows and matching are [`model::popups::picker_rows`]; the programs' names come from
//! `programs.json`.

use herdr_shared::Displayed;
use model::mode::ListPopup;
use model::popups::{self, PickerRow};
use rune::{Color, El, col, el, row, text};

use crate::State;
use crate::config::theme::Rgba;
use crate::view::icons::SEARCH;
use crate::view::list::{self, Item};
use crate::view::overlay::{self, Place};

pub const PLACE: Place = Place::Top {
    offset: 40.0,
    width: 640.0,
    height: 0.8,
};

/// Rows shown at once: the list's 320 points of 32-point rows.
pub const SHOWN: usize = 10;

/// The picker's space colors (`cfg/mods/picker/main.js` `MARK`), in the order a new space gets
/// them (`PROJECT_COLORS`).
const COLORS: [&str; 6] = [
    "#22a99a", "#e3814f", "#5b8def", "#8b7cf6", "#3fae6a", "#d9649a",
];

pub fn rows(s: &State, list: &ListPopup) -> Vec<PickerRow> {
    let programs = &s.config.programs;
    popups::picker_rows(&s.model, &list.query, s.picker_earlier, &|t| {
        programs.of_terminal(t).name
    })
}

/// Each space's avatar color: the one the sidebar keeps for it in herdr-frontend's home, so both
/// agree. A space not given one yet gets the color fewest spaces before it have, in space order
/// (`spaceColors`); the home space none.
fn space_color(s: &State, space: &str, name: &str) -> Color {
    if name == "~" {
        return s.chrome.dim;
    }
    if let Some(kept) = s.shared.space_colors().get(space)
        && let Some(hex) = model::agents::space_mark(kept)
    {
        return Rgba::parse(hex).map_or(s.chrome.accent, Rgba::color);
    }
    let mut uses = [0usize; COLORS.len()];
    for sp in s.model.world().spaces() {
        if s.model.world().space_name(sp, s.model.home()) == "~" {
            continue;
        }
        let pick = (0..COLORS.len()).min_by_key(|&i| uses[i]).unwrap_or(0);
        if sp.id == space {
            return Rgba::parse(COLORS[pick]).map_or(s.chrome.accent, Rgba::color);
        }
        uses[pick] += 1;
    }
    s.chrome.accent
}

pub fn view(s: &State, list: &ListPopup) -> El {
    let c = &s.chrome;
    let rows = rows(s, list);
    let small = |t: String, color: Color| text(t).font(s.ui_font(11.0, 400.0)).color(color);
    let mut pick = 0;
    let items: Vec<Item> = rows
        .iter()
        .map(|r| {
            if let PickerRow::Head { space, name } = r {
                let avatar = el()
                    .size(18.0, 18.0)
                    .radius(5.0)
                    .center()
                    .fill(space_color(s, space, name))
                    .child(
                        text(popups::avatar(name))
                            .font(s.ui_font(10.5, 700.0))
                            .color(Color::WHITE),
                    );
                return Item::head(
                    row()
                        .key(format!("picker-head-{}", space.replace('/', ":")))
                        .h(29.0)
                        .px(10.0)
                        .pt(10.0)
                        .pb(3.0)
                        .gap(8.0)
                        .items_center()
                        .child(avatar)
                        .child(text(name.clone()).font(s.ui_font(11.0, 600.0)).color(c.dim)),
                );
            }
            let i = pick;
            pick += 1;
            let on = i == list.cursor;
            let base = row()
                .key(format!("picker-row-{i}"))
                .mx(4.0)
                .px(10.0)
                .h(32.0)
                .gap(10.0)
                .items_center()
                .radius(7.0)
                .fill(if on { c.hover } else { c.popover });
            let el = match r {
                PickerRow::Tab {
                    name,
                    status,
                    here,
                    who,
                    team,
                    ..
                } => {
                    let glyph = match status {
                        Some(Displayed::Idle) => "○",
                        Some(Displayed::Working) => "◐",
                        Some(_) => "●",
                        None => "·",
                    };
                    let flag = match status {
                        Some(Displayed::Blocked) => Some(small("needs you".into(), c.blocked)),
                        Some(Displayed::Done) => Some(small("finished".into(), c.done)),
                        _ => None,
                    };
                    let who = if *team > 0 {
                        format!("{who}  +{team}")
                    } else {
                        who.clone()
                    };
                    base.child(
                        el().w(12.0).child(
                            text(glyph)
                                .font(s.ui_font(13.0, 400.0))
                                .color(overlay::status_color(s, *status)),
                        ),
                    )
                    .child(
                        text(name.clone())
                            .key(format!("picker-name-{i}"))
                            .font(s.ui_font(13.0, if *here { 600.0 } else { 400.0 }))
                            .color(c.foreground)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .children(flag)
                    .child(small(who, c.dim))
                }
                PickerRow::Earlier { count, open } => {
                    let label = format!(
                        "earlier: {count} quiet space{} {}",
                        if *count > 1 { "s" } else { "" },
                        if *open { "▴" } else { "▾" }
                    );
                    base.mx(0.0)
                        .radius(0.0)
                        .border_sides([1.0, 0.0, 0.0, 0.0], c.border)
                        .child(
                            small(label, if on { c.foreground } else { c.dim })
                                .font(s.ui_font(12.0, 400.0)),
                        )
                }
                PickerRow::NewTab | PickerRow::NewSpace(_) => {
                    let label = match r {
                        PickerRow::NewTab => "new tab".to_owned(),
                        PickerRow::NewSpace(Some(m)) => format!("new space on {m}"),
                        _ => "new space".to_owned(),
                    };
                    base.gap(8.0)
                        .child(
                            el().w(12.0)
                                .child(text("+").font(s.ui_font(13.0, 400.0)).color(c.muted)),
                        )
                        .child(text(label).font(s.ui_font(13.0, 400.0)).color(c.muted))
                }
                PickerRow::Head { .. } => unreachable!("titles are handled above"),
            };
            Item::row(el, i)
        })
        .collect();
    col()
        .w_full()
        .min_h(0.0)
        .padding(18.0)
        .gap(10.0)
        .child(overlay::title(
            s,
            "picker-title",
            "Go to…",
            Some(overlay::esc(s)),
        ))
        .child(overlay::field(
            s,
            "Go to a tab, space or agent…",
            Some(&SEARCH),
            None,
        ))
        .child(list::view(s, "picker-rows", items, SHOWN, false).mb(6.0))
}
