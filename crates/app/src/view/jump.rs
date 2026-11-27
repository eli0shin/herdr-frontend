//! The agent-jump bar (`ctrl-;`, `feature-inventory.md` §7): Eli's `agent-jump` mod, a popup
//! at the top (`mod.json`: `"at": "top", "offset": 6, "width": 360`) in the shared frame
//! ([`crate::view::overlay`]). Herdr app hangs it `offset` (zoomed) below the window's top edge,
//! but never closer than 8 window points (`position_popup`).
//!
//! ```text
//! agent-jump   360 × 32 pt: "JUMP", the labels it takes and Esc, the label typed so far
//! ```
//!
//! The model holds the keyboard while it's open ([`model::mode::PopupState::Jump`]): a whole
//! label focuses its row, the first of two letters waits, Esc or Ctrl+; leave, and so does a
//! press beside it. The sidebar brightens its labels meanwhile, and is shown for the jump if it
//! was hidden (`State::mode_changed`).

use model::mode::{self, JumpPopup};
use rune::{El, row, text};

use crate::State;
use crate::view::overlay::Place;

/// The bar's width (the mod's `overlay.width`).
pub const WIDTH: f32 = 360.0;
/// Its height (the mod's `.h(32)`).
pub const HEIGHT: f32 = 32.0;
/// How far below the window's top it hangs (the mod's `overlay.offset`)...
pub const OFFSET: f32 = 6.0;
/// ...but never closer than this many window points, whatever the zoom.
pub const MARGIN: f32 = 8.0;

/// Its top in view points at `zoom` (the window draws the view zoomed).
pub fn top(zoom: f32) -> f32 {
    OFFSET.max(MARGIN / zoom)
}

/// Where the frame goes at the settings' zoom.
pub fn place(s: &State) -> Place {
    Place::Top {
        offset: top(s.zoom()),
        width: WIDTH,
        height: HEIGHT,
    }
}

pub fn view(s: &State, jump: &JumpPopup) -> El {
    let c = &s.chrome;
    let count = s.model.agent_rows().len();
    let typed = (!jump.typed.is_empty()).then(|| {
        text(jump.typed.clone())
            .key("jump-typed")
            .font(s.ui_font(12.0, 600.0))
            .color(c.foreground)
    });
    row()
        .key("jump-bar")
        .w_full()
        .h(HEIGHT)
        .px(12.0)
        .gap(12.0)
        .items_center()
        .child(
            text("JUMP")
                .key("jump-title")
                .font(s.ui_font(12.0, 600.0))
                .color(c.working),
        )
        .child(
            text(mode::jump_hint(count))
                .key("jump-hint")
                .font(s.ui_font(12.0, 400.0))
                .color(c.dim)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .children(typed)
}
