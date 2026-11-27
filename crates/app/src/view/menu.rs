// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! Context menus (Herdr app's `menu` mod, `feature-inventory.md` §10), at the pointer: 31-point
//! items, each with its icon, its label and its command's `global` key, a divider above closing
//! items. ↑/↓ move, Enter runs, Esc or a press outside closes. The tab menu is 200 wide, the
//! terminal menu 220; both are at most 260 tall.
//!
//! Every [`model::mode::PopupState::Menu`] draws here, whichever feature opened it.

use model::Menu;
use rune::{El, col, el, row, text};

use crate::State;
use crate::view::icons::menu_icon;
use crate::view::overlay::{self, Place};

/// Where a menu opens: the last right-click, else the window's middle third.
pub fn place(s: &State, menu: &Menu) -> Place {
    let width = menu.width;
    let (vw, vh) = s.view_size();
    let (x, y) = s.menu_at.unwrap_or((vw / 3.0, vh / 3.0));
    Place::At {
        x,
        y,
        width,
        height: 260.0,
    }
}

pub fn view(s: &State, menu: &Menu) -> El {
    let c = &s.chrome;
    let items = menu.items.iter().enumerate().map(|(i, item)| {
        let on = i == menu.cursor;
        let hint = item
            .hint
            .as_ref()
            .and_then(|a| model::help::hint(s.model.keymap(), a))
            .unwrap_or_default();
        let line = row()
            .key(format!("menu-item-{i}"))
            .h(31.0)
            .px(8.0)
            .gap(9.0)
            .items_center()
            .radius(7.0)
            .fill(if on { c.selected } else { c.popover })
            .cursor(rune::Cursor::Pointer)
            .hover(rune::Color::WHITE.alpha(0.04))
            .on_press_tx(move |s: &mut State, tx| s.pick_row(i, tx))
            .child(rune::icon(menu_icon(item.icon), 14.0).color(c.muted))
            .child(
                text(item.label.clone())
                    .key(format!("menu-label-{i}"))
                    .font(s.ui_font(12.8, 400.0))
                    .color(c.foreground)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
            )
            .child(overlay::note(s, hint));
        let divider =
            (i > 0 && item.divided).then(|| el().mx(7.0).my(4.0).h(1.0).w_full().fill(c.border));
        col().w_full().children(divider).child(line)
    });
    col().w_full().padding(5.0).children(items)
}
