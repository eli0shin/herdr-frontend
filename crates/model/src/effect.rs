//! What [`crate::Model::step`] asks the app to do. The model does no I/O: the app carries each
//! effect out (a call on the core connection's worker, a Rune focus move, a toast) and feeds the
//! answers back as [`crate::Event`]s, as `rune::machine` transitions do.

use herdr_proto::methods::{
    MachineExec, TerminalGet, TerminalKeep, TerminalKill, TerminalLabel, TerminalRun,
};
use herdr_shared::{Raise, Round};

use crate::action::{Action, AppCommand, CopyAction, Popup, TerminalAction};
use crate::control::CallId;
use crate::mode::Visual;

/// Matches a call's reply to the request that made it ([`crate::Event::Reply`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token(pub u64);

/// A herdr-core call the model needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Start a pane's program; its reply carries the new terminal.
    Run(Box<TerminalRun>),
    /// Look a terminal up: before adopting it, or to learn whether a pane's terminal is gone.
    Get(TerminalGet),
    Kill(TerminalKill),
    Keep(TerminalKeep),
    Label(TerminalLabel),
    /// A command on a machine, without a terminal; answered with [`crate::Event::Executed`].
    Exec(MachineExec),
}

/// A command for one terminal's view (its worker, selection and find bar).
#[derive(Clone, Debug, PartialEq)]
pub enum TerminalCommand {
    /// A `terminal` key-context action.
    Action(TerminalAction),
    /// Copy mode starts at the program's cursor.
    CopyStart,
    /// A copy-mode motion, to run on the terminal's own buffer.
    CopyMove(CopyAction),
    /// Start selecting from the cursor (characters or lines), or drop the selection.
    CopySelect(Option<Visual>),
    /// Copy the selection to the clipboard.
    CopyYank,
    /// Copy mode ends: clear the selection and scroll to the bottom.
    CopyEnd,
}

/// What the keyboard should go to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Focus {
    /// A pane's terminal grid (`pane-<id>/screen`).
    Pane(String),
    /// The open popup or dialog.
    Popup,
    /// Nothing in particular (no pane to focus).
    None,
}

/// How a toast looks (`feature-inventory.md` §8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

/// An in-app toast. Never a native notification and never a sound.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub level: Level,
    pub title: String,
    pub body: String,
    /// What a click runs before the toast goes (an agent's alert: [`Action::FocusAgent`]).
    pub action: Option<Action>,
}

impl Toast {
    pub fn new(level: Level, title: impl Into<String>, body: impl Into<String>) -> Toast {
        Toast {
            level,
            title: title.into(),
            body: body.into(),
            action: None,
        }
    }

    /// The toast, running `action` when clicked.
    pub fn on_click(mut self, action: Action) -> Toast {
        self.action = Some(action);
        self
    }

    /// The terminal its click goes to, when it is about one.
    pub fn terminal(&self) -> Option<&str> {
        match self.action.as_ref()? {
            Action::FocusAgent(address) | Action::Peek(address) => Some(address),
            _ => None,
        }
    }
}

/// How a list popup's row was picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Enter or a click.
    Choose,
    /// Space in the machine filter.
    Toggle,
    /// Ctrl+D in the picker.
    CloseItem,
}

/// One thing for the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Make a call; answer with [`crate::Event::Reply`] carrying `token` (an
    /// [`Request::Exec`] with [`crate::Event::Executed`]).
    Call {
        token: Token,
        request: Request,
    },
    /// Make one machine's document round (`doc.get` or `doc.set`); answer with
    /// [`crate::Event::DocRead`] or [`crate::Event::DocWritten`].
    Doc {
        machine: Option<String>,
        round: Round,
    },
    /// Run a command in a terminal's view.
    Terminal {
        address: String,
        command: TerminalCommand,
    },
    App(AppCommand),
    /// Move the keyboard.
    Focus(Focus),
    Toast(Toast),
    /// An agent wants the person; the model already brought its tab back. With `toast`, the app
    /// shows [`crate::Model::alert_toast`].
    Alert(Raise),
    /// A list popup's row was picked; the popup's feature resolves it.
    Pick {
        popup: Popup,
        how: Pick,
        cursor: usize,
        query: String,
    },
    /// Not built yet: show a "not yet" toast.
    NotYet(Action),
    /// Step [`crate::Event::Tick`] at this clock time (seconds) or later.
    WakeAt(f64),
    /// The per-screen view changed: write [`crate::Model::saved_view`] to herdr-frontend's home.
    SaveView,
    /// The answer to a control call ([`crate::Call`]): its result, or `{"error": …}`. Every
    /// call gets exactly one.
    Answer {
        id: CallId,
        reply: serde_json::Value,
    },
}

/// The effects of one step, in order, and whether its key was used.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effects {
    pub list: Vec<Effect>,
    /// For a key: the model used it, so it must not reach the terminal. Always true for an
    /// action.
    pub handled: bool,
}

impl Effects {
    pub fn push(&mut self, effect: Effect) {
        self.list.push(effect);
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Effect> {
        self.list.iter()
    }
}

impl IntoIterator for Effects {
    type Item = Effect;
    type IntoIter = std::vec::IntoIter<Effect>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.into_iter()
    }
}

impl Extend<Effect> for Effects {
    fn extend<I: IntoIterator<Item = Effect>>(&mut self, iter: I) {
        self.list.extend(iter);
    }
}
