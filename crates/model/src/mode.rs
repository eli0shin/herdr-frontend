//! Who has the keyboard, as one explicit state machine (`docs/research/rune.md` §2 "Modes are
//! explicit state machines"): normal keys, a prefix sequence part-way through (`ctrl-g …`), an
//! open popup or dialog, or copy mode. Each variant carries only what is valid in it.
//!
//! This module holds the states and the keys each popup and copy mode read
//! (`feature-inventory.md` §22 "Keys inside Eli's popups"); [`crate::Model::step`] runs the
//! transitions and the actions the keys map to.

use crate::action::{Action, CopyAction, Popup, PopupAction};
use crate::keys::{Chord, Key, Mods, Press};

/// How long a prefix sequence waits for its next key, in seconds (GPUI's pending-keystroke
/// timeout, which Herdr app's `ctrl-g` sequences use).
pub const PREFIX_TIMEOUT: f64 = 1.0;

/// Who has the keyboard.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Mode {
    /// Keys go through the keymap, and unbound ones to the focused terminal.
    #[default]
    Normal,
    /// Part of a sequence: `keys` so far. At `due` (app clock, seconds) it is dropped.
    Prefix {
        keys: Vec<Press>,
        due: f64,
    },
    Popup(PopupState),
    /// Copy mode in one terminal.
    Copy(CopyState),
}

/// An open popup or dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum PopupState {
    /// A filtered list: the pickers, palette, help, the machine filter and the sort menu.
    List(ListPopup),
    /// The agent-jump bar.
    Jump(JumpPopup),
    /// A rename prompt.
    Prompt(Prompt),
    /// A confirm dialog.
    Confirm(Confirm),
}

impl PopupState {
    /// The popup this is, when it is one of [`Popup`]'s.
    pub fn popup(&self) -> Option<Popup> {
        match self {
            PopupState::List(list) => Some(list.popup),
            PopupState::Jump(_) => Some(Popup::AgentJump),
            PopupState::Prompt(_) | PopupState::Confirm(_) => None,
        }
    }
}

/// A list popup's query and cursor. The popup's feature filters the rows and reports how many
/// there are ([`crate::Event::PopupRows`]); the cursor moves over them, wrapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListPopup {
    pub popup: Popup,
    pub query: String,
    pub cursor: usize,
    pub rows: usize,
}

impl ListPopup {
    pub fn new(popup: Popup) -> ListPopup {
        ListPopup {
            popup,
            query: String::new(),
            cursor: 0,
            rows: 0,
        }
    }

    pub(crate) fn step(&mut self, by: isize) {
        if self.rows == 0 {
            self.cursor = 0;
            return;
        }
        let n = self.rows as isize;
        self.cursor = (self.cursor as isize + by).rem_euclid(n) as usize;
    }

    pub(crate) fn set_rows(&mut self, rows: usize) {
        self.rows = rows;
        self.cursor = self.cursor.min(rows.saturating_sub(1));
    }
}

/// The agent-jump bar: the label typed so far (the first of two letters waits).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JumpPopup {
    pub typed: String,
}

/// What a prompt renames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rename {
    Tab(String),
    Space(String),
}

/// A rename prompt: "Name", the current value selected, "empty: its number".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub rename: Rename,
    pub value: String,
}

/// What a confirm dialog does once confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmed {
    /// Close a space and end its terminals (also closing a space's last tab).
    CloseSpace(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub title: String,
    pub detail: String,
    pub then: Confirmed,
}

/// What copy mode has selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visual {
    Chars,
    Lines,
}

/// Copy mode: the terminal it runs in and its selection kind. The cursor and selection live in
/// the terminal's own buffer (ticket 020).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyState {
    pub address: String,
    pub visual: Option<Visual>,
}

impl Mode {
    pub fn popup(&self) -> Option<&PopupState> {
        match self {
            Mode::Popup(p) => Some(p),
            _ => None,
        }
    }

    /// The keys of a sequence part-way through, for the prefix indicator.
    pub fn pending_keys(&self) -> Option<&[Press]> {
        match self {
            Mode::Prefix { keys, .. } => Some(keys),
            _ => None,
        }
    }

    /// Whether an overlay holds the keyboard (a popup, a dialog or copy mode).
    pub fn holds_keyboard(&self) -> bool {
        matches!(self, Mode::Popup(_) | Mode::Copy(_))
    }
}

const fn mods(cmd: bool, ctrl: bool, alt: bool, shift: bool) -> Mods {
    Mods {
        cmd,
        ctrl,
        alt,
        shift,
    }
}

const SHIFT: Mods = mods(false, false, false, true);
const CTRL: Mods = mods(false, true, false, false);
const CMD: Mods = mods(true, false, false, false);

/// What a key does in an open popup. `None`: the popup's text field (if it has one) takes it.
pub fn popup_key(state: &PopupState, chord: Chord) -> Option<Action> {
    use PopupAction as P;
    let Chord { key, mods } = chord;
    if let PopupState::Jump(_) = state {
        return jump_key(chord);
    }
    let list = |a: P| Some(Action::Popup(a));
    match (key, mods) {
        (Key::Escape, Mods::NONE) => return list(P::Close),
        (Key::Enter, Mods::NONE) => return list(P::Choose),
        _ => {}
    }
    let PopupState::List(popup) = state else {
        return None;
    };
    match (key, mods) {
        (Key::Up, Mods::NONE) => list(P::Up),
        (Key::Down, Mods::NONE) => list(P::Down),
        (Key::Char('p'), CTRL) => list(P::Up),
        (Key::Char('n'), CTRL) => list(P::Down),
        (Key::Char('d'), CTRL) if popup.popup == Popup::Picker => list(P::CloseItem),
        _ if popup.popup == Popup::AgentMachines => match (key, mods) {
            (Key::Char('k'), Mods::NONE) => list(P::Up),
            (Key::Char('j'), Mods::NONE) => list(P::Down),
            (Key::Space, Mods::NONE) => list(P::Toggle),
            (Key::Char('q'), Mods::NONE) => list(P::Close),
            _ => None,
        },
        _ => None,
    }
}

/// The agent-jump bar: a label character, or leaving (Esc, Ctrl+;). Keys with Cmd, Ctrl or Alt
/// are ignored, as the mod ignores them.
fn jump_key(chord: Chord) -> Option<Action> {
    match (chord.key, chord.mods) {
        (Key::Escape, _) => Some(Action::Popup(PopupAction::Close)),
        (Key::Char(';'), CTRL) => Some(Action::Popup(PopupAction::Close)),
        (_, m) if m.command_like() => None,
        (Key::Char(c), _) => Some(Action::AgentJumpKey(c.to_ascii_lowercase())),
        _ => None,
    }
}

/// What a key does in copy mode (`feature-inventory.md` §16). `None`: nothing; copy mode holds
/// the keyboard, so the key goes nowhere else either.
pub fn copy_key(chord: Chord) -> Option<Action> {
    use CopyAction as C;
    let Chord { key, mods } = chord;
    let action = match (key, mods) {
        (Key::Char('h') | Key::Left, Mods::NONE) => C::Left,
        (Key::Char('j') | Key::Down, Mods::NONE) => C::Down,
        (Key::Char('k') | Key::Up, Mods::NONE) => C::Up,
        (Key::Char('l') | Key::Right, Mods::NONE) => C::Right,
        (Key::Char('0') | Key::Home, Mods::NONE) => C::LineStart,
        (Key::Char('^'), Mods::NONE | SHIFT) | (Key::Char('6'), SHIFT) => C::FirstText,
        (Key::Char('$'), Mods::NONE | SHIFT) | (Key::Char('4'), SHIFT) | (Key::End, Mods::NONE) => {
            C::LineEnd
        }
        (Key::Char('g'), Mods::NONE) => C::Top,
        (Key::Char('g'), SHIFT) => C::Bottom,
        (Key::Char('h'), SHIFT) => C::ViewTop,
        (Key::Char('m'), SHIFT) => C::ViewMiddle,
        (Key::Char('l'), SHIFT) => C::ViewBottom,
        (Key::Char('u'), CTRL) => C::HalfPageUp,
        (Key::Char('d'), CTRL) => C::HalfPageDown,
        (Key::Char('b'), CTRL) | (Key::PageUp, Mods::NONE) => C::PageUp,
        (Key::Char('f'), CTRL) | (Key::PageDown, Mods::NONE) => C::PageDown,
        (Key::Char('v') | Key::Space, Mods::NONE) => C::SelectChars,
        (Key::Char('v'), SHIFT) => C::SelectLines,
        (Key::Char('y') | Key::Enter, Mods::NONE) | (Key::Char('c'), CMD) => C::Yank,
        (Key::Char('q'), Mods::NONE) | (Key::Char('c'), CTRL) => C::Quit,
        (Key::Escape, Mods::NONE) => C::Escape,
        _ => return None,
    };
    Some(Action::Copy(action))
}

/// Agent-jump labels for `count` rows: 1-9 and 0, then home-row letters; with more rows than
/// single keys, every row past the tenth gets two letters, so no label begins another
/// (`cfg/mods/eli-agents/jump.js` `jumpLabels`).
pub fn jump_labels(count: usize) -> Vec<String> {
    const DIGITS: &str = "1234567890";
    const LETTERS: &str = "asdfghjklqwertyuiopzxcvbnm";
    let digits = DIGITS.chars().map(String::from);
    let rest: Vec<String> = if count.saturating_sub(DIGITS.len()) > LETTERS.len() {
        LETTERS
            .chars()
            .flat_map(|a| LETTERS.chars().map(move |b| format!("{a}{b}")))
            .collect()
    } else {
        LETTERS.chars().map(String::from).collect()
    };
    digits.chain(rest).take(count).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).unwrap()
    }

    fn list(popup: Popup) -> PopupState {
        PopupState::List(ListPopup::new(popup))
    }

    #[test]
    fn list_popups_move_choose_and_close() {
        for popup in [
            Popup::SpacePicker,
            Popup::Picker,
            Popup::NewSpace,
            Popup::PrFinder,
        ] {
            let s = list(popup);
            let key = |k| popup_key(&s, chord(k));
            assert_eq!(key("up"), Some(Action::Popup(PopupAction::Up)));
            assert_eq!(key("ctrl-p"), Some(Action::Popup(PopupAction::Up)));
            assert_eq!(key("down"), Some(Action::Popup(PopupAction::Down)));
            assert_eq!(key("ctrl-n"), Some(Action::Popup(PopupAction::Down)));
            assert_eq!(key("enter"), Some(Action::Popup(PopupAction::Choose)));
            assert_eq!(key("escape"), Some(Action::Popup(PopupAction::Close)));
            assert_eq!(key("a"), None, "typing goes to the filter");
            assert_eq!(key("j"), None);
        }
    }

    #[test]
    fn only_the_picker_closes_a_tab_with_ctrl_d() {
        assert_eq!(
            popup_key(&list(Popup::Picker), chord("ctrl-d")),
            Some(Action::Popup(PopupAction::CloseItem))
        );
        assert_eq!(popup_key(&list(Popup::SpacePicker), chord("ctrl-d")), None);
    }

    #[test]
    fn the_machine_filter_also_reads_j_k_space_and_q() {
        let s = list(Popup::AgentMachines);
        for (k, a) in [
            ("j", PopupAction::Down),
            ("k", PopupAction::Up),
            ("space", PopupAction::Toggle),
            ("q", PopupAction::Close),
            ("enter", PopupAction::Choose),
            ("escape", PopupAction::Close),
        ] {
            assert_eq!(popup_key(&s, chord(k)), Some(Action::Popup(a)), "{k}");
        }
    }

    #[test]
    fn prompts_and_confirms_read_enter_and_escape_only() {
        let prompt = PopupState::Prompt(Prompt {
            rename: Rename::Tab("t".into()),
            value: String::new(),
        });
        let confirm = PopupState::Confirm(Confirm {
            title: String::new(),
            detail: String::new(),
            then: Confirmed::CloseSpace("s".into()),
        });
        for s in [prompt, confirm] {
            assert_eq!(
                popup_key(&s, chord("enter")),
                Some(Action::Popup(PopupAction::Choose))
            );
            assert_eq!(
                popup_key(&s, chord("escape")),
                Some(Action::Popup(PopupAction::Close))
            );
            assert_eq!(popup_key(&s, chord("up")), None);
            assert_eq!(popup_key(&s, chord("x")), None);
        }
    }

    #[test]
    fn the_jump_bar_takes_label_characters_and_ignores_modified_keys() {
        let s = PopupState::Jump(JumpPopup::default());
        assert_eq!(popup_key(&s, chord("a")), Some(Action::AgentJumpKey('a')));
        assert_eq!(
            popup_key(&s, chord("shift-a")),
            Some(Action::AgentJumpKey('a'))
        );
        assert_eq!(popup_key(&s, chord("1")), Some(Action::AgentJumpKey('1')));
        assert_eq!(popup_key(&s, chord("cmd-a")), None);
        assert_eq!(popup_key(&s, chord("alt-a")), None);
        assert_eq!(popup_key(&s, chord("ctrl-a")), None);
        assert_eq!(
            popup_key(&s, chord("escape")),
            Some(Action::Popup(PopupAction::Close))
        );
        assert_eq!(
            popup_key(&s, chord("ctrl-;")),
            Some(Action::Popup(PopupAction::Close))
        );
        assert_eq!(popup_key(&s, chord("enter")), None);
    }

    #[test]
    fn copy_mode_reads_its_table() {
        use CopyAction as C;
        for (k, a) in [
            ("h", C::Left),
            ("left", C::Left),
            ("j", C::Down),
            ("down", C::Down),
            ("k", C::Up),
            ("up", C::Up),
            ("l", C::Right),
            ("right", C::Right),
            ("0", C::LineStart),
            ("home", C::LineStart),
            ("^", C::FirstText),
            ("shift-6", C::FirstText),
            ("$", C::LineEnd),
            ("shift-4", C::LineEnd),
            ("end", C::LineEnd),
            ("g", C::Top),
            ("shift-g", C::Bottom),
            ("shift-h", C::ViewTop),
            ("shift-m", C::ViewMiddle),
            ("shift-l", C::ViewBottom),
            ("ctrl-u", C::HalfPageUp),
            ("ctrl-d", C::HalfPageDown),
            ("ctrl-b", C::PageUp),
            ("pageup", C::PageUp),
            ("ctrl-f", C::PageDown),
            ("pagedown", C::PageDown),
            ("v", C::SelectChars),
            ("space", C::SelectChars),
            ("shift-v", C::SelectLines),
            ("y", C::Yank),
            ("enter", C::Yank),
            ("cmd-c", C::Yank),
            ("q", C::Quit),
            ("ctrl-c", C::Quit),
            ("escape", C::Escape),
        ] {
            assert_eq!(copy_key(chord(k)), Some(Action::Copy(a)), "{k}");
        }
        assert_eq!(copy_key(chord("x")), None);
        assert_eq!(copy_key(chord("cmd-t")), None);
    }

    #[test]
    fn list_cursors_wrap_and_clamp_to_the_rows() {
        let mut l = ListPopup::new(Popup::SpacePicker);
        l.step(1);
        assert_eq!(l.cursor, 0, "no rows");
        l.set_rows(3);
        l.step(-1);
        assert_eq!(l.cursor, 2);
        l.step(1);
        assert_eq!(l.cursor, 0);
        l.cursor = 2;
        l.set_rows(2);
        assert_eq!(l.cursor, 1);
        l.set_rows(0);
        assert_eq!(l.cursor, 0);
    }

    #[test]
    fn jump_labels_are_digits_then_letters_then_pairs_past_36_rows() {
        assert!(jump_labels(0).is_empty());
        assert_eq!(jump_labels(3), ["1", "2", "3"]);
        let ten = jump_labels(12);
        assert_eq!(ten[9], "0");
        assert_eq!(&ten[10..], ["a", "s"]);
        assert_eq!(jump_labels(36).last().map(String::as_str), Some("m"));
        let many = jump_labels(37);
        assert_eq!(many.len(), 37);
        assert_eq!(&many[10..13], ["aa", "as", "ad"]);
        for (i, a) in many.iter().enumerate() {
            for (j, b) in many.iter().enumerate() {
                assert!(i == j || !b.starts_with(a.as_str()), "{a} begins {b}");
            }
        }
    }

    #[test]
    fn only_popups_and_copy_mode_hold_the_keyboard() {
        assert!(!Mode::Normal.holds_keyboard());
        assert!(
            !Mode::Prefix {
                keys: vec![],
                due: 0.0
            }
            .holds_keyboard()
        );
        assert!(Mode::Popup(list(Popup::Help)).holds_keyboard());
        assert!(
            Mode::Copy(CopyState {
                address: "t".into(),
                visual: None
            })
            .holds_keyboard()
        );
        assert_eq!(list(Popup::Help).popup(), Some(Popup::Help));
        assert_eq!(
            PopupState::Jump(JumpPopup::default()).popup(),
            Some(Popup::AgentJump)
        );
    }
}
