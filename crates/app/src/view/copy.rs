//! Copy mode's bar (`ctrl-g [`, `feature-inventory.md` §16): Eli's `copy-mode` mod, a popup at
//! the top (420 × 32, `offset` 6, never closer than 8 window points) in the shared frame
//! ([`crate::view::overlay`]), as the agent-jump bar hangs.
//!
//! ```text
//! COPY   hints, or the / prompt, or what the last command said   keys so far   row/total
//! ```
//!
//! The mode is COPY in the accent color, or VISUAL, VISUAL LINE or VISUAL BLOCK in the working
//! color. The row is the cursor's, counted from the top of the scrollback. The cursor and the
//! selection draw in the terminal itself (`terminal::Vt::copy_run`). The model holds the
//! keyboard ([`model::Mode::Copy`]); a press beside the bar leaves without copying.

use model::copy_mode::Visual;
use model::mode::CopyState;
use model::{Action, CopyAction};
use rune::{El, row, text};

use crate::State;
use crate::view::jump;
use crate::view::overlay::{self, Place};

/// The bar's width (the mod's `overlay.width`) and height.
pub const WIDTH: f32 = 420.0;
pub const HEIGHT: f32 = 32.0;

/// The box's key: the keyboard goes there while copy mode runs.
pub const KEY: &str = "copy-mode";

/// The mode's name, as the bar shows it.
pub fn mode_name(visual: Option<Visual>) -> &'static str {
    match visual {
        None => "COPY",
        Some(Visual::Chars) => "VISUAL",
        Some(Visual::Lines) => "VISUAL LINE",
        Some(Visual::Block) => "VISUAL BLOCK",
    }
}

/// The keys the bar reminds of.
pub fn hint(visual: Option<Visual>) -> &'static str {
    match visual {
        None => "v select · / search · y copy · q leave",
        Some(_) => "y copy · o other end · Esc drop",
    }
}

/// The bar while copy mode runs in `copy.address`.
pub fn view(s: &State) -> Option<El> {
    let model::Mode::Copy(copy) = s.model.mode() else {
        return None;
    };
    let place = Place::Top {
        offset: jump::top(s.zoom()),
        width: WIDTH,
        height: HEIGHT,
    };
    Some(overlay::frame_closing(
        s,
        KEY,
        place,
        body(s, copy),
        Action::Copy(CopyAction::Quit),
    ))
}

fn body(s: &State, copy: &CopyState) -> El {
    let c = &s.chrome;
    let visual = copy.visual();
    let shown = s
        .viewers
        .get(&copy.address)
        .and_then(|v| v.pane.as_ref())
        .and_then(|p| p.snapshot.copy.clone());
    let (middle, ink) = match (
        copy.vim.prompt(),
        shown.as_ref().and_then(|v| v.message.clone()),
    ) {
        (Some(prompt), _) => {
            let slash = if prompt.backward { '?' } else { '/' };
            (format!("{slash}{}\u{258f}", prompt.query), c.foreground)
        }
        (None, Some(message)) => (message, c.blocked),
        (None, None) => (hint(visual).to_owned(), c.dim),
    };
    let typed = (!copy.vim.typed().is_empty()).then(|| {
        text(copy.vim.typed().to_owned())
            .key("copy-typed")
            .font(s.ui_font(12.0, 600.0))
            .color(c.foreground)
    });
    let position = shown.map(|v| {
        text(format!("{}/{}", v.row, v.total))
            .key("copy-row")
            .font(s.ui_font(11.0, 400.0))
            .color(c.muted)
    });
    row()
        .key("copy-bar")
        .w_full()
        .h(HEIGHT)
        .px(12.0)
        .gap(12.0)
        .items_center()
        .child(
            text(mode_name(visual))
                .key("copy-mode-name")
                .font(s.ui_font(12.0, 600.0))
                .color(if visual.is_some() {
                    c.working
                } else {
                    c.accent
                }),
        )
        .child(
            text(middle)
                .key("copy-hint")
                .font(s.ui_font(12.0, 400.0))
                .color(ink)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .children(typed)
        .children(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_reads_as_the_mods() {
        assert_eq!(mode_name(None), "COPY");
        assert_eq!(mode_name(Some(Visual::Chars)), "VISUAL");
        assert_eq!(mode_name(Some(Visual::Lines)), "VISUAL LINE");
        assert_eq!(mode_name(Some(Visual::Block)), "VISUAL BLOCK");
    }
}
