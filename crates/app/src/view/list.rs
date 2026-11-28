// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! The list every picker, the palette, the settings and machine dialogs share: one flat column
//! of every row and group title, always laid out, in a scroll viewport at most `max_h` tall.
//! The wheel and the trackpad scroll the viewport and leave the cursor alone; a list that fits
//! doesn't scroll. When the model's cursor moves, the window reveals its row just far enough,
//! with its group's title when it opens one (as Herdr app's lists scroll "only as far as it
//! takes to keep the selected row in view"). A press on a row picks it.
//!
//! A popup builds its [`Item`]s; the state asks Rune to reveal the cursor's row ([`pick_key`]
//! inside the list's key) whenever the cursor moves, the list opens or its query changes.

use rune::{Color, El, ScrollSpec, col};

use crate::State;
use crate::view::wheel::{self, Axis};

/// One item: a row the cursor can reach (`pick`: its place among those), or a title.
pub struct Item {
    pub el: El,
    pub pick: Option<usize>,
    /// A press shows or hides the row (Space) instead of choosing it (the machine filter).
    pub toggles: bool,
}

impl Item {
    pub fn row(el: El, pick: usize) -> Item {
        Item {
            el,
            pick: Some(pick),
            toggles: false,
        }
    }

    /// A row a press toggles.
    pub fn toggle(el: El, pick: usize) -> Item {
        Item {
            toggles: true,
            ..Item::row(el, pick)
        }
    }

    pub fn head(el: El) -> Item {
        Item {
            el,
            pick: None,
            toggles: false,
        }
    }
}

/// The key of row `pick`'s place in its list, with the title before it when it opens a group:
/// what revealing the cursor's row scrolls into view.
pub fn pick_key(pick: usize) -> String {
    format!("pick-{pick}")
}

/// Every item, in order, in a scroll viewport keyed `key` at most `max_h` tall (as tall as its
/// rows when they fit); each row picks itself on a press, and a wheel's notch scrolls it at once
/// ([`wheel`]).
pub fn view(s: &State, key: &'static str, items: Vec<Item>, max_h: Option<f32>) -> El {
    let mut places: Vec<El> = Vec::new();
    let mut title: Option<El> = None;
    for item in items {
        let Some(i) = item.pick else {
            places.extend(title.replace(item.el));
            continue;
        };
        let toggles = item.toggles;
        let row = item
            .el
            .cursor(rune::Cursor::Pointer)
            .hover(Color::WHITE.alpha(0.04))
            .on_press_tx(move |s: &mut State, tx| {
                if toggles {
                    s.toggle_row(i, tx)
                } else {
                    s.pick_row(i, tx)
                }
            });
        places.push(
            col()
                .key(pick_key(i))
                .w_full()
                .children(title.take())
                .child(row),
        );
    }
    places.extend(title);
    let list = col()
        .key(key)
        .w_full()
        .min_h(0.0)
        .scroll(ScrollSpec::default())
        .children(places);
    let list = match max_h {
        Some(h) => list.max_h(h),
        None => list,
    };
    wheel::notches_at_once(list, key, Axis::Y, s)
}
