//! The open popup, whichever it is: what draws it, how many rows it lists and where its cursor
//! starts, where the keyboard goes, and what a pick runs. Each popup's own module draws its
//! body in the shared frame ([`crate::view::overlay`]) and list ([`crate::view::list`]).
//!
//! | Popup | Module | Frame |
//! | --- | --- | --- |
//! | Keys help (`cmd-/`) | [`help`] | dialog 640 × 520 |
//! | Command palette (`cmd-shift-p`) | [`palette`] | 620 wide, 60 from the top |
//! | Tab and agent picker (`cmd-p`) | [`picker`] | 640 wide, 40 from the top |
//! | Space picker (`ctrl-g s`) | [`space_picker`] | 520 wide, 40 from the top |
//! | Settings (`cmd-,`) | [`settings`] | dialog 520 wide |
//! | Prompt, confirm | [`dialog`] | dialog 380 wide |
//! | Menus | [`menu`] | at the pointer |
//! | Sort menu | [`crate::view::sidebar`] | under the sidebar's sort button |
//!
//! The new-space folder picker, the PR finder, the machine filter and the agent-jump bar come with their tickets; until then they close at once with a "not yet".

use model::mode::{ListPopup, PopupState};
use model::{Mode, Popup};
use rune::{Cx, El};

use crate::State;
use crate::view::overlay;
use crate::view::{dialog, help, menu, palette, picker, settings, space_picker};

/// What a list popup lists now: per item whether (and as which row) the cursor can reach it,
/// where the cursor starts for its query, and how many items show at once.
pub struct Listing {
    pub picks: Vec<Option<usize>>,
    pub start: usize,
    pub shown: usize,
}

impl Listing {
    pub fn rows(&self) -> usize {
        self.picks.iter().filter(|p| p.is_some()).count()
    }

    fn flat(rows: usize, start: usize, shown: usize) -> Listing {
        Listing {
            picks: (0..rows).map(Some).collect(),
            start,
            shown,
        }
    }
}

/// The list popups the window draws.
pub fn drawn(popup: Popup) -> bool {
    matches!(
        popup,
        Popup::Help | Popup::Palette | Popup::Picker | Popup::SpacePicker | Popup::Settings
    )
}

/// What a drawn list popup lists; `None` for one the window doesn't draw.
pub fn listing(s: &State, list: &ListPopup) -> Option<Listing> {
    Some(match list.popup {
        Popup::Help => Listing::flat(help::shown(s, list).len(), 0, usize::MAX),
        Popup::Palette => {
            let rows = palette::rows(s, list);
            Listing {
                picks: palette::picks(&rows, !list.query.trim().is_empty()),
                start: 0,
                shown: palette::SHOWN,
            }
        }
        Popup::Picker => {
            let rows = picker::rows(s, list);
            let mut n = 0;
            let picks = rows
                .iter()
                .map(|r| {
                    r.pickable().then(|| {
                        n += 1;
                        n - 1
                    })
                })
                .collect();
            Listing {
                picks,
                start: model::popups::picker_cursor(&rows, &list.query),
                shown: picker::SHOWN,
            }
        }
        Popup::SpacePicker => {
            let rows = space_picker::rows(s, list);
            let start = model::popups::space_cursor(&rows, &list.query);
            Listing::flat(rows.len(), start, space_picker::SHOWN)
        }
        Popup::Settings => {
            let n = settings::rows(s).len();
            Listing::flat(n, 0, n)
        }
        _ => return None,
    })
}

/// Whether the popup types into [`overlay::FIELD`].
pub fn has_field(state: &PopupState) -> bool {
    match state {
        PopupState::List(list) => !matches!(list.popup, Popup::Settings | Popup::SortMenu),
        PopupState::Prompt(_) => true,
        PopupState::Jump(_) | PopupState::Confirm(_) | PopupState::Menu(_) => false,
    }
}

/// The element the open popup's keyboard goes to: its field, else its box.
pub fn focus_key(mode: &Mode) -> &'static str {
    match mode {
        Mode::Popup(state) if has_field(state) => overlay::FIELD,
        Mode::Popup(state) => box_key(state),
        _ => overlay::FIELD,
    }
}

fn box_key(state: &PopupState) -> &'static str {
    match state {
        PopupState::List(list) => match list.popup {
            Popup::Help => "help",
            Popup::Palette => "palette",
            Popup::Picker => "picker",
            Popup::SpacePicker => "space-picker",
            Popup::Settings => "settings",
            Popup::SortMenu => "sort-menu",
            _ => "popup",
        },
        PopupState::Prompt(_) => "prompt",
        PopupState::Confirm(_) => "confirm",
        PopupState::Menu(_) => "menu",
        PopupState::Jump(_) => "agent-jump",
    }
}

/// The open popup in its frame, when the window draws it.
pub fn view(s: &State, cx: &mut Cx<State>) -> Option<El> {
    let Mode::Popup(state) = s.model.mode() else {
        return None;
    };
    // The sort menu hangs under the sidebar's sort button, in its own frame (`view::sidebar`).
    if let PopupState::List(list) = state
        && list.popup == Popup::SortMenu
    {
        return s
            .sidebar
            .shown()
            .is_some()
            .then(|| crate::view::sidebar::sort_menu(s, list));
    }
    let key = box_key(state);
    let (place, body) = match state {
        PopupState::List(list) => match list.popup {
            Popup::Help => (help::PLACE, help::view(s, list, cx)),
            Popup::Palette => (palette::PLACE, palette::view(s, list, cx)),
            Popup::Picker => (picker::PLACE, picker::view(s, list, cx)),
            Popup::SpacePicker => (space_picker::PLACE, space_picker::view(s, list, cx)),
            Popup::Settings => (settings::PLACE, settings::view(s, list, cx)),
            _ => return None,
        },
        PopupState::Prompt(prompt) => (dialog::PLACE, dialog::prompt(s, prompt, cx)),
        PopupState::Confirm(confirm) => (dialog::PLACE, dialog::confirm(s, confirm)),
        PopupState::Menu(m) => (menu::place(s, m), menu::view(s, m, cx)),
        PopupState::Jump(_) => return None,
    };
    Some(overlay::frame(s, key, place, body))
}
