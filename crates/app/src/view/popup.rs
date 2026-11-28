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
//! | New-space folder picker (`cmd-n`) | [`new_space`] | 640 wide, 40 from the top |
//! | PR finder (`ctrl-g f`) | [`pr_finder`] | 620 wide, 40 from the top |
//! | Settings (`cmd-,`) | [`settings`] | dialog 520 wide |
//! | Prompt, confirm | [`dialog`] | dialog 380 wide |
//! | Menus | [`menu`] | at the pointer |
//! | Sort menu | [`crate::view::sidebar`] | under the sidebar's sort button |
//! | Agent-jump bar (`ctrl-;`) | [`jump`] | 360 × 32, 6 from the top (8 window points at least) |
//! | Copy mode's bar (`ctrl-g [`) | [`crate::view::copy`] | 420 × 32, as the jump bar hangs |
//! | Machine filter (`ctrl-g /`) | [`machines`] | 340 wide, 40 from the top |
//! | Machines, Add a machine | [`machines`] | dialogs 520 and 560 wide |

use model::mode::{ListPopup, PopupState};
use model::{Mode, Popup};
use rune::{Cx, El};

use crate::State;
use crate::view::overlay;
use crate::view::{
    dialog, help, jump, list, machines, menu, new_space, palette, picker, pr_finder, settings,
    space_picker,
};

/// What a list popup lists now: per item whether (and as which row) the cursor can reach it,
/// where the cursor starts for its query, and where its rows are: the scroll viewport's key and
/// the key of a row's place in it, which the window reveals when the cursor moves.
pub struct Listing {
    pub picks: Vec<Option<usize>>,
    pub start: usize,
    pub key: &'static str,
    pub row_key: fn(usize) -> String,
}

impl Listing {
    pub fn rows(&self) -> usize {
        self.picks.iter().filter(|p| p.is_some()).count()
    }

    /// Rows only, no titles, in [`crate::view::list`] under `key`.
    fn flat(rows: usize, start: usize, key: &'static str) -> Listing {
        Listing {
            picks: (0..rows).map(Some).collect(),
            start,
            key,
            row_key: list::pick_key,
        }
    }
}

/// The list popups the window draws.
pub fn drawn(popup: Popup) -> bool {
    matches!(
        popup,
        Popup::Help
            | Popup::Palette
            | Popup::Picker
            | Popup::SpacePicker
            | Popup::NewSpace
            | Popup::PrFinder
            | Popup::Settings
            | Popup::AgentMachines
            | Popup::Machines
            | Popup::MachineAdd
    )
}

/// What a drawn list popup lists; `None` for one the window doesn't draw.
pub fn listing(s: &State, list: &ListPopup) -> Option<Listing> {
    Some(match list.popup {
        Popup::Help => Listing {
            row_key: help::row_key,
            ..Listing::flat(help::shown(s, list).len(), 0, help::ROWS)
        },
        Popup::Palette => {
            let rows = palette::rows(s, list);
            Listing {
                picks: palette::picks(&rows, !list.query.trim().is_empty()),
                start: 0,
                key: palette::ROWS,
                row_key: list::pick_key,
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
                key: picker::ROWS,
                row_key: list::pick_key,
            }
        }
        Popup::SpacePicker => {
            let rows = space_picker::rows(s, list);
            let start = model::popups::space_cursor(&rows, &list.query);
            Listing::flat(rows.len(), start, space_picker::ROWS)
        }
        Popup::NewSpace => Listing::flat(new_space::rows(s, list).len(), 0, new_space::ROWS),
        Popup::PrFinder => Listing::flat(pr_finder::rows(s, list).len(), 0, pr_finder::ROWS),
        Popup::Settings => Listing::flat(settings::rows(s).len(), 0, settings::ROWS),
        Popup::AgentMachines => {
            Listing::flat(machines::filter_rows(s).len(), 0, machines::FILTER_ROWS)
        }
        Popup::Machines => {
            let rows = machines::machines_rows(s);
            let start = machines::start(s, &rows);
            Listing::flat(rows.len(), start, machines::MACHINES_ROWS)
        }
        Popup::MachineAdd => {
            Listing::flat(machines::add_rows(s, list).len(), 0, machines::ADD_ROWS)
        }
        _ => return None,
    })
}

/// Whether the popup types into [`overlay::FIELD`].
pub fn has_field(state: &PopupState) -> bool {
    match state {
        PopupState::List(list) => !matches!(
            list.popup,
            Popup::Settings | Popup::SortMenu | Popup::AgentMachines | Popup::Machines
        ),
        PopupState::Prompt(_) => true,
        PopupState::Jump(_) | PopupState::Confirm(_) | PopupState::Menu(_) => false,
    }
}

/// The element the open popup's keyboard goes to: its field, else its box.
pub fn focus_key(mode: &Mode) -> &'static str {
    match mode {
        Mode::Popup(state) if has_field(state) => overlay::FIELD,
        Mode::Popup(state) => box_key(state),
        Mode::Copy(_) => crate::view::copy::KEY,
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
            Popup::NewSpace => "new-space",
            Popup::PrFinder => "pr-finder",
            Popup::Settings => "settings",
            Popup::SortMenu => "sort-menu",
            Popup::AgentMachines => "agent-machines",
            Popup::Machines => "machines",
            Popup::MachineAdd => "machine-add",
            _ => "popup",
        },
        PopupState::Prompt(_) => "prompt",
        PopupState::Confirm(_) => "confirm",
        PopupState::Menu(_) => "menu",
        PopupState::Jump(_) => "agent-jump",
    }
}

/// The open popup in its frame, when the window draws it.
pub fn view(s: &State, cx: &Cx<State>) -> Option<El> {
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
            Popup::Help => (help::PLACE, help::view(s, list)),
            Popup::Palette => (palette::PLACE, palette::view(s, list)),
            Popup::Picker => (picker::PLACE, picker::view(s, list)),
            Popup::SpacePicker => (space_picker::PLACE, space_picker::view(s, list)),
            Popup::NewSpace => (new_space::PLACE, new_space::view(s, list)),
            Popup::PrFinder => (pr_finder::PLACE, pr_finder::view(s, list, cx)),
            Popup::Settings => (settings::PLACE, settings::view(s, list)),
            Popup::AgentMachines => (machines::FILTER_PLACE, machines::filter_view(s, list)),
            Popup::Machines => (machines::MACHINES_PLACE, machines::machines_view(s, list)),
            Popup::MachineAdd => (machines::ADD_PLACE, machines::add_view(s, list)),
            _ => return None,
        },
        PopupState::Prompt(prompt) => (dialog::PLACE, dialog::prompt(s, prompt)),
        PopupState::Confirm(confirm) => (dialog::PLACE, dialog::confirm(s, confirm)),
        PopupState::Menu(m) => (menu::place(s, m), menu::view(s, m)),
        PopupState::Jump(j) => (jump::place(s), jump::view(s, j)),
    };
    Some(overlay::frame(s, key, place, body))
}
