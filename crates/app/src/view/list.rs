//! The list every picker, the palette and the settings dialog share: rows the model's cursor
//! moves over, group titles it skips, a window of rows that follows the cursor (as Herdr app's
//! lists scroll "only as far as it takes to keep the selected row in view"), clicks that pick a
//! row and the wheel that moves the window.
//!
//! A popup builds its [`Item`]s; the state keeps the window's first item ([`State::list_top`])
//! with [`follow`] whenever the cursor or the rows change.

use rune::{Color, El, col};

use crate::State;
use crate::view::overlay;

/// One item: a row the cursor can reach (`pick`: its place among those), or a title.
pub struct Item {
    pub el: El,
    pub pick: Option<usize>,
}

impl Item {
    pub fn row(el: El, pick: usize) -> Item {
        Item {
            el,
            pick: Some(pick),
        }
    }

    pub fn head(el: El) -> Item {
        Item { el, pick: None }
    }
}

/// The first item to show so the cursor's row is in view, moving the window as little as it
/// can from `top`. A row that opens a group shows with its title. `picks` says, per item,
/// whether (and as which row) the cursor can reach it; `shown` items fit.
pub fn follow(picks: &[Option<usize>], cursor: usize, top: usize, shown: usize) -> usize {
    let total = picks.len();
    if total <= shown {
        return 0;
    }
    let mut top = top.min(total - shown);
    let Some(at) = picks.iter().position(|p| *p == Some(cursor)) else {
        return top;
    };
    // Its group's title, right before it, comes into view with it.
    let from = if at > 0 && picks[at - 1].is_none() {
        at - 1
    } else {
        at
    };
    if from < top {
        top = from;
    }
    if at >= top + shown {
        top = at + 1 - shown;
    }
    top
}

/// Which notes say what is out of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum More {
    None,
    /// "↑ N more" and "↓ N more" (the space picker).
    Both,
    /// "↓ N more" only (the new-space picker, the PR finder).
    Below,
}

/// The items from the window's first, `shown` of them, each row picking itself on a press,
/// with `more`'s notes.
pub fn view(s: &State, key: &str, items: Vec<Item>, shown: usize, more: More) -> El {
    let total = items.len();
    let top = s.list_top.min(total.saturating_sub(shown));
    let above = if more == More::Both { top } else { 0 };
    let below = if more == More::None {
        0
    } else {
        total.saturating_sub(top + shown)
    };
    let note = |text: String| overlay::note(s, text).px(14.0).h(20.0).items_center();
    let rows = items
        .into_iter()
        .skip(top)
        .take(shown)
        .map(|item| match item.pick {
            Some(i) => item
                .el
                .cursor(rune::Cursor::Pointer)
                .hover(Color::WHITE.alpha(0.04))
                .on_press_tx(move |s: &mut State, tx| s.pick_row(i, tx)),
            None => item.el,
        });
    col()
        .key(key)
        .w_full()
        .min_h(0.0)
        .children((above > 0).then(|| note(format!("↑ {above} more"))))
        .children(rows)
        .children((below > 0).then(|| note(format!("↓ {below} more"))))
        .on_wheel(|s: &mut State, e, tx| s.list_wheel(e.dy, tx))
}

#[cfg(test)]
mod tests {
    use super::follow;

    #[test]
    fn the_window_moves_only_as_far_as_the_cursor_needs() {
        let rows: Vec<Option<usize>> = (0..20).map(Some).collect();
        assert_eq!(follow(&rows, 0, 0, 5), 0);
        assert_eq!(follow(&rows, 4, 0, 5), 0);
        assert_eq!(follow(&rows, 5, 0, 5), 1);
        assert_eq!(follow(&rows, 19, 1, 5), 15);
        assert_eq!(follow(&rows, 16, 15, 5), 15, "still in view");
        assert_eq!(follow(&rows, 3, 15, 5), 3);
        assert_eq!(follow(&rows[..4], 3, 2, 5), 0, "everything fits");
        assert_eq!(follow(&rows, 0, 40, 5), 0, "a stale top comes back");
    }

    #[test]
    fn a_row_opening_a_group_shows_with_its_title() {
        // title, 0, 1, title, 2, 3
        let items = [None, Some(0), Some(1), None, Some(2), Some(3)];
        assert_eq!(follow(&items, 0, 1, 3), 0, "the first title");
        assert_eq!(
            follow(&items, 2, 0, 3),
            2,
            "down to row 2: the window ends on it"
        );
        assert_eq!(follow(&items, 2, 4, 3), 3, "up to row 2: its title too");
    }
}
