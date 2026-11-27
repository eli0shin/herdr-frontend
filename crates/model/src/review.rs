//! The review view (`feature-inventory.md` §13, `cfg/mods/navigator-status/review-view.js`):
//! Eli's review TUI (`review`) as a view of the window instead of a tab to hunt for.
//!
//! - There is one review terminal per machine: a live terminal labeled [`REVIEW_LABEL`], found
//!   on the shown space's machine.
//! - `cmd-r` focuses it. On the review, `cmd-r` goes back to the terminal it came from, if that
//!   still runs.
//! - With none running, `cmd-r` starts one in a new tab of the shown space, named "review",
//!   running [`REVIEW_ARGV`]: the terminal labels itself, runs `review`, and ends when `review`
//!   quits, so the next press starts a fresh one and no shell is left behind. It lives in a
//!   space because `review` opens its Review and Diff tabs in the space and folder it runs in.
//! - Its tab stays out of the tab bar and the tab keys ([`Model::is_hidden_tab`]). A review
//!   started by hand isn't labeled and stays an ordinary tab.

use herdr_proto::Terminal;
use herdr_shared::machine_of;

use crate::effect::Effects;
use crate::model::{Model, REVIEW_LABEL};

/// The review tab's name.
pub const REVIEW_NAME: &str = "review";

/// What the review terminal runs: it labels itself, runs `review`, then ends itself.
pub const REVIEW_ARGV: [&str; 3] = [
    "/bin/sh",
    "-c",
    "hc label \"$HC_TERMINAL_ID\" dotfiles.review=1 >/dev/null 2>&1; review; hc kill \"$HC_TERMINAL_ID\"",
];

/// Whether a terminal is a running review: live, with a non-empty [`REVIEW_LABEL`].
pub fn is_review(t: &Terminal) -> bool {
    t.is_live() && t.label(REVIEW_LABEL).is_some_and(|v| !v.is_empty())
}

impl Model {
    /// `cmd-r` (`navigator-status.review`): the shown machine's review, or back from it, or a
    /// new one.
    pub(crate) fn review(&mut self, fx: &mut Effects) {
        let space = self.active_space().map(|s| s.id.clone());
        let machine = space.as_deref().and_then(machine_of).map(str::to_owned);
        let here = self.focused_terminal().map(str::to_owned);
        let review = self
            .world
            .live()
            .find(|t| is_review(t) && machine_of(&t.address) == machine.as_deref())
            .map(|t| t.address.clone());
        if review.is_some() && review == here {
            if let Some(back) = self.review_back.clone()
                && self.world.is_live(&back)
            {
                self.focus_terminal(&back);
            }
            return;
        }
        self.review_back = here;
        match review {
            Some(review) => self.focus_terminal(&review),
            None => {
                let argv = REVIEW_ARGV.iter().map(|a| (*a).to_owned()).collect();
                self.new_tab_running(space.as_deref(), REVIEW_NAME, argv, fx);
            }
        }
    }

    /// Focuses a terminal's pane, showing its tab and space (`pane.focus {terminal}`); nothing
    /// for one without a pane.
    fn focus_terminal(&mut self, address: &str) {
        if let Some(pane) = self.world.pane_of(address) {
            self.view.focus_pane(&self.world, &pane);
        }
    }
}
