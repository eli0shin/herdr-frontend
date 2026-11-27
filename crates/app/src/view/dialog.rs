//! The prompt and confirm dialogs (Herdr app's `prompt` and `confirm` mods,
//! `feature-inventory.md` §10), 380 wide over the dimmed window.
//!
//! - **Prompt** (`tab.rename`, `space.rename`, `pane.rename`): its title, a "Name" field holding
//!   the current name, selected, a hint ("empty: its number" for a tab, "empty: named after its
//!   folder" for a space, "empty: unnamed" for a pane), Cancel and Save. Enter saves, Esc
//!   cancels.
//! - **Confirm** (`space.close`, closing a space's last tab): its title, the detail, Cancel and
//!   Confirm. Enter confirms, Esc cancels.
//!
//! Every [`model::mode::PopupState::Prompt`] and [`model::mode::PopupState::Confirm`] draws
//! here, whichever feature opened it.

use model::PopupAction;
use model::mode::{Confirm, Prompt, Rename};
use rune::{El, col, row, text};

use crate::State;
use crate::view::overlay::{self, Place};

pub const PLACE: Place = Place::Dialog {
    width: 380.0,
    height: None,
};

/// A prompt's title and hint (`app/src/window.rs` `ask_name`).
pub fn words(prompt: &Prompt) -> (&'static str, &'static str) {
    match prompt.rename {
        Rename::Tab(_) => ("Rename tab", "empty: its number"),
        Rename::Space(_) => ("Rename space", "empty: named after its folder"),
        Rename::Pane(_) => ("Rename pane", "empty: unnamed"),
        Rename::Machine(_) => (
            "Rename machine",
            "addresses using the old name stop working",
        ),
    }
}

fn buttons(s: &State, yes: &str) -> El {
    row()
        .w_full()
        .pt(20.0)
        .gap(8.0)
        .justify_end()
        .child(overlay::button(
            s,
            "dialog-cancel",
            "Cancel",
            false,
            PopupAction::Close,
        ))
        .child(overlay::button(
            s,
            "dialog-yes",
            yes,
            true,
            PopupAction::Choose,
        ))
}

pub fn prompt(s: &State, prompt: &Prompt) -> El {
    let (title, hint) = words(prompt);
    col()
        .w_full()
        .padding(18.0)
        .child(overlay::title(
            s,
            "prompt-title",
            title,
            Some(overlay::note(s, "Esc")),
        ))
        .child(
            text("Name")
                .font(s.ui_font(11.5, 400.0))
                .color(s.chrome.dim)
                .pt(16.0)
                .pb(7.0),
        )
        .child(overlay::field(s, "Name", None, None))
        .child(
            overlay::note(s, hint)
                .key("prompt-hint")
                .font(s.ui_font(11.5, 400.0))
                .pt(6.0),
        )
        .child(buttons(s, "Save"))
}

pub fn confirm(s: &State, confirm: &Confirm) -> El {
    col()
        .w_full()
        .padding(18.0)
        .gap(12.0)
        .child(overlay::title(
            s,
            "confirm-title",
            &confirm.title,
            Some(overlay::note(s, "Esc")),
        ))
        .child(
            text(confirm.detail.clone())
                .key("confirm-detail")
                .font(s.ui_font(13.0, 400.0))
                .color(s.chrome.muted)
                .w_full(),
        )
        .child(buttons(s, "Confirm").pt(8.0))
}
