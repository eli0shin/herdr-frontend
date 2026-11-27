// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! In-app toasts (`feature-inventory.md` §8, Eli's `eli-toast`): 340 × 72 cards at the window's
//! bottom right, 16 pt in, stacked upward with 8 pt gaps, the oldest at the bottom as Herdr app
//! stacks them. Each shows a level dot, a 13 pt semibold title and a 12 pt muted body, one line
//! each, and goes after 5 s (8 s for errors and warnings). A click runs its action (an agent's
//! alert goes to the agent) and dismisses it. Never a native notification, never a sound.

use std::collections::BTreeSet;

use model::{Action, Level, Toast};
use rune::{Cursor, El, col, el, row, text};

use crate::config::theme::Chrome;

pub const WIDTH: f32 = 340.0;
pub const HEIGHT: f32 = 72.0;
pub const INSET: f32 = 16.0;
pub const GAP: f32 = 8.0;

/// How many show at once; older ones go first.
pub const MOST: usize = 4;

/// How long a toast shows, in seconds.
pub fn timeout(level: Level) -> f64 {
    match level {
        Level::Error | Level::Warning => 8.0,
        Level::Info | Level::Success => 5.0,
    }
}

/// The toast for an action that isn't built yet.
pub fn not_yet(action: &Action) -> Toast {
    let what = action
        .name()
        .unwrap_or_else(|| model::help::title(action).to_lowercase());
    Toast::new(Level::Info, "Not yet", format!("{what} isn't built yet"))
}

/// A toast on screen, until `due` (app clock, seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub id: u64,
    pub toast: Toast,
    pub due: f64,
    /// Its Copy button was pressed: it reads "Copied".
    pub copied: bool,
}

/// The toasts on screen, oldest first.
#[derive(Debug, Default)]
pub struct Toasts {
    pub shown: Vec<Shown>,
    next: u64,
}

impl Toasts {
    /// Shows a toast from `now`; when it goes.
    pub fn push(&mut self, toast: Toast, now: f64) -> f64 {
        let due = now + timeout(toast.level);
        self.next += 1;
        self.shown.push(Shown {
            id: self.next,
            toast,
            due,
            copied: false,
        });
        if self.shown.len() > MOST {
            self.shown.remove(0);
        }
        due
    }

    /// Takes away the toasts due by `now`; true when one went.
    pub fn expire(&mut self, now: f64) -> bool {
        let before = self.shown.len();
        self.shown.retain(|t| t.due > now);
        self.shown.len() != before
    }

    /// When the next one goes.
    pub fn next_due(&self) -> Option<f64> {
        self.shown.iter().map(|t| t.due).reduce(f64::min)
    }

    /// Its Copy button was pressed: the command it copies, which the button now says it did.
    pub fn copied(&mut self, id: u64) -> Option<String> {
        let shown = self.shown.iter_mut().find(|t| t.id == id)?;
        shown.copied = true;
        shown.toast.copy.clone()
    }

    /// Takes one away now (it was clicked); what it was.
    pub fn dismiss(&mut self, id: u64) -> Option<Toast> {
        let at = self.shown.iter().position(|t| t.id == id)?;
        Some(self.shown.remove(at).toast)
    }
}

/// Whether a toast is about an agent on a machine the Agents list hides: its click goes to a
/// terminal there (`""` is this computer).
pub fn about_hidden_machine(toast: &Toast, hidden: &BTreeSet<String>) -> bool {
    toast
        .terminal()
        .is_some_and(|t| hidden.contains(herdr_shared::machine_of(t).unwrap_or("")))
}

/// A level's color, from the theme: error is `blocked`, warning `working`, success `done`,
/// info `accent`.
fn dot(level: Level, chrome: &Chrome) -> rune::Color {
    match level {
        Level::Error => chrome.blocked,
        Level::Warning => chrome.working,
        Level::Success => chrome.done,
        Level::Info => chrome.accent,
    }
}

/// The stack over the window's bottom right, the oldest at the bottom; `None` with no toasts.
pub fn view(s: &crate::State) -> Option<El> {
    let (toasts, chrome) = (&s.toasts, &s.chrome);
    if toasts.shown.is_empty() {
        return None;
    }
    let cards = toasts.shown.iter().rev().map(|t| {
        let id = t.id;
        let body = (!t.toast.body.is_empty()).then(|| {
            text(t.toast.body.clone())
                .key(format!("toast-{id}-body"))
                .font(s.ui_font(12.0, 400.0))
                .color(chrome.muted)
                .grow(1.0)
                .min_w(0.0)
                .truncate()
        });
        // A command to run, with Copy beside it (eli-toast's login card).
        let copy = t.toast.copy.as_ref().map(|_| {
            el().key(format!("toast-{id}-copy"))
                .px(6.0)
                .py(3.0)
                .radius(4.0)
                .fill(chrome.raised)
                .hover_fill(chrome.hover)
                .cursor(Cursor::Pointer)
                .on_press_tx(move |s: &mut crate::State, tx| s.copy_from_toast(id, tx))
                .child(
                    text(if t.copied { "Copied" } else { "Copy" })
                        .font(s.ui_font(12.0, 400.0))
                        .color(chrome.foreground),
                )
        });
        let body = body.map(|body| row().gap(8.0).items_center().child(body).children(copy));
        row()
            .key(format!("toast-{id}"))
            .w(WIDTH)
            .h(HEIGHT)
            .px(18.0)
            .gap(12.0)
            .items_center()
            .radius(8.0)
            .fill(chrome.popover)
            .hover_fill(chrome.hover)
            .border(1.0, chrome.border)
            .cursor(Cursor::Pointer)
            .on_press_tx(move |s: &mut crate::State, tx| s.click_toast(id, tx))
            .child(
                el().size(8.0, 8.0)
                    .radius(4.0)
                    .fill(dot(t.toast.level, chrome)),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(3.0)
                    .child(
                        text(t.toast.title.clone())
                            .key(format!("toast-{id}-title"))
                            .font(s.ui_font(13.0, 600.0))
                            .color(chrome.foreground)
                            .truncate(),
                    )
                    .children(body),
            )
    });
    Some(
        col()
            .key("toasts")
            .pin(None, None, Some(INSET), Some(INSET))
            .w(WIDTH)
            .gap(GAP)
            .children(cards),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toasts_go_after_their_level_s_timeout_and_only_four_show() {
        let mut t = Toasts::default();
        assert_eq!(t.push(Toast::new(Level::Info, "a", ""), 10.0), 15.0);
        assert_eq!(t.push(Toast::new(Level::Error, "b", ""), 11.0), 19.0);
        assert_eq!(t.next_due(), Some(15.0));
        assert!(!t.expire(14.9));
        assert!(t.expire(15.0));
        assert_eq!(t.shown.len(), 1);
        for i in 0..5 {
            t.push(Toast::new(Level::Info, format!("{i}"), ""), 12.0);
        }
        assert_eq!(t.shown.len(), MOST);
        assert_eq!(t.shown[0].toast.title, "1", "the oldest went first");
    }

    #[test]
    fn a_dismissed_toast_goes_at_once() {
        let mut t = Toasts::default();
        t.push(Toast::new(Level::Info, "a", ""), 0.0);
        t.push(Toast::new(Level::Info, "b", ""), 0.0);
        let id = t.shown[0].id;
        assert_eq!(t.dismiss(id).map(|t| t.title), Some("a".into()));
        assert_eq!(t.dismiss(id), None);
        assert_eq!(t.shown.len(), 1);
    }

    #[test]
    fn notes_about_agents_on_hidden_machines_close() {
        let hidden: BTreeSet<String> = ["devbox".to_owned()].into();
        let note = |address: &str| {
            Toast::new(Level::Info, "x finished", "").on_click(Action::FocusAgent(address.into()))
        };
        assert!(about_hidden_machine(&note("devbox/t3"), &hidden));
        assert!(!about_hidden_machine(&note("omarchy/t3"), &hidden));
        assert!(!about_hidden_machine(&note("t3"), &hidden));
        assert!(about_hidden_machine(&note("t3"), &["".to_owned()].into()));
        let plain = Toast::new(Level::Info, "Keys reset", "");
        assert!(!about_hidden_machine(&plain, &["".to_owned()].into()));
    }

    #[test]
    fn not_yet_names_the_command() {
        let toast = not_yet(&Action::Review);
        assert_eq!(toast.title, "Not yet");
        assert_eq!(toast.body, "review isn't built yet");
        let toast = not_yet(&Action::Peek("a2".into()));
        assert!(toast.body.ends_with("isn't built yet"));
    }
}
