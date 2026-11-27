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
    /// A context menu: the tab menu, the terminal menu.
    Menu(Menu),
}

impl PopupState {
    /// The popup this is, when it is one of [`Popup`]'s.
    pub fn popup(&self) -> Option<Popup> {
        match self {
            PopupState::List(list) => Some(list.popup),
            PopupState::Jump(_) => Some(Popup::AgentJump),
            PopupState::Prompt(_) | PopupState::Confirm(_) | PopupState::Menu(_) => None,
        }
    }
}

/// A list popup's query and cursor. The popup's feature filters the rows and reports how many
/// there are ([`crate::Event::PopupRows`]) and, when it opens or the query changes, where the
/// cursor starts ([`crate::Event::PopupCursor`]); the cursor moves over them, wrapping (the
/// palette stops at its ends instead).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListPopup {
    pub popup: Popup,
    pub query: String,
    pub cursor: usize,
    pub rows: usize,
    /// Up from the first row goes to the last, and down from the last to the first.
    pub wrap: bool,
}

/// How far Page Up and Page Down move a list's cursor: the palette's ten shown rows.
pub const PAGE: isize = 10;

impl ListPopup {
    pub fn new(popup: Popup) -> ListPopup {
        ListPopup {
            popup,
            query: String::new(),
            cursor: 0,
            rows: 0,
            // "Stops at the ends: wrapping jumps out of sight" (`cfg/frame/palette/main.js`).
            wrap: popup != Popup::Palette,
        }
    }

    pub(crate) fn step(&mut self, by: isize) {
        if self.rows == 0 {
            self.cursor = 0;
            return;
        }
        let n = self.rows as isize;
        let to = self.cursor as isize + by;
        self.cursor = if self.wrap && by.abs() == 1 {
            to.rem_euclid(n)
        } else {
            to.clamp(0, n - 1)
        } as usize;
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
    /// The terminal menu's "Rename": a pane's name, its block's `title`.
    Pane(String),
    /// A machine this core added (`machine.rename`).
    Machine(String),
}

/// A rename prompt: "Name", the current value selected, and a hint ("empty: its number").
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
    /// Forget a machine (`machine.rm`); its terminals keep running there.
    RemoveMachine(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub title: String,
    pub detail: String,
    pub then: Confirmed,
}

/// What a context menu is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuTarget {
    /// A tab pill, right-clicked.
    Tab(String),
    /// A pane's terminal, right-clicked.
    Pane(String),
    /// A machine in the Machines dialog.
    Machine(String),
}

/// The icon a menu item shows (`cfg/mods/menu/main.js` `menuIcon`): Lucide glyphs, drawn by the
/// window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuIcon {
    /// `x`: closing.
    Close,
    /// `square-pen`: renaming.
    Rename,
    /// `plus`: something new.
    New,
    /// `panel-right`.
    SplitRight,
    /// `panel-bottom`.
    SplitDown,
    /// `square`: zoom.
    Zoom,
    /// `terminal`: anything else.
    Terminal,
    /// `server`: a machine.
    Machine,
    /// `copy`: a command to the clipboard.
    Copy,
}

/// One row of a context menu: what it says, its icon, what it runs (in order), and the named
/// action whose key it shows on its right (its first `global` binding, [`crate::help::hint`]).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub icon: MenuIcon,
    pub actions: Vec<Action>,
    pub hint: Option<Action>,
    /// A divider above it (before closing items, as the menu mod draws).
    pub divided: bool,
}

impl MenuItem {
    pub fn new(label: impl Into<String>, icon: MenuIcon, actions: Vec<Action>) -> MenuItem {
        MenuItem {
            label: label.into(),
            icon,
            divided: icon == MenuIcon::Close,
            actions,
            hint: None,
        }
    }

    /// Shows `action`'s key on the right.
    pub fn hint(mut self, action: Action) -> MenuItem {
        self.hint = Some(action);
        self
    }
}

/// A context menu: its items and the one under the cursor. ↑/↓ move (wrapping), Enter runs the
/// item, Esc closes (`cfg/mods/menu/main.js`). Built by [`crate::popups`]; any feature can open
/// one with its own items, and the window draws them all alike.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub target: MenuTarget,
    pub items: Vec<MenuItem>,
    pub cursor: usize,
    /// How wide it draws, in points: the opener's choice, as `openOverlay({width})` is.
    pub width: f32,
}

impl Menu {
    pub fn new(target: MenuTarget, items: Vec<MenuItem>, width: f32) -> Menu {
        Menu {
            target,
            items,
            cursor: 0,
            width,
        }
    }

    /// The tab menu: Rename, New tab (in the tab's space), Close, 200 wide as eli-tabs opens it
    /// (`menu/main.js` `items`, case `"tab"`).
    pub fn tab(tab: &str, space: &str) -> Menu {
        Menu::new(
            MenuTarget::Tab(tab.to_owned()),
            vec![
                MenuItem::new(
                    "Rename",
                    MenuIcon::Rename,
                    vec![Action::RenameTabOf(tab.into())],
                )
                .hint(Action::TabRename),
                MenuItem::new(
                    "New tab",
                    MenuIcon::New,
                    vec![Action::NewTabIn(space.into())],
                )
                .hint(Action::TabNew),
                MenuItem::new(
                    "Close",
                    MenuIcon::Close,
                    vec![Action::CloseTabOf(tab.into())],
                )
                .hint(Action::TabClose),
            ],
            200.0,
        )
    }

    pub(crate) fn step(&mut self, by: isize) {
        let n = self.items.len() as isize;
        if n > 0 {
            self.cursor = (self.cursor as isize + by).rem_euclid(n) as usize;
        }
    }
}

pub use copy_mode::Visual;

/// Copy mode: the terminal it runs in and the Vim grammar reading its keys
/// ([`copy_mode::Vim`]: counts, pending keys, the visual mode, the `/` prompt). The cursor,
/// selection, marks and search live with the terminal's own buffer, on its worker
/// (`terminal::Vt::copy_run`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyState {
    pub address: String,
    pub vim: copy_mode::Vim,
}

impl CopyState {
    pub fn new(address: impl Into<String>) -> CopyState {
        CopyState {
            address: address.into(),
            vim: copy_mode::Vim::default(),
        }
    }

    /// What's selected, if anything.
    pub fn visual(&self) -> Option<Visual> {
        self.vim.visual()
    }
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
    if let PopupState::Menu(_) = state {
        return match (key, mods) {
            (Key::Up, Mods::NONE) => list(P::Up),
            (Key::Down, Mods::NONE) => list(P::Down),
            _ => None,
        };
    }
    let PopupState::List(popup) = state else {
        return None;
    };
    match (key, mods) {
        (Key::Up, Mods::NONE) => list(P::Up),
        (Key::Down, Mods::NONE) => list(P::Down),
        (Key::Char('p'), CTRL) => list(P::Up),
        (Key::Char('n'), CTRL) => list(P::Down),
        (Key::PageUp, Mods::NONE) => list(P::PageUp),
        (Key::PageDown, Mods::NONE) => list(P::PageDown),
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
/// are ignored, as the mod ignores them. The mod reads the typed character, so Shift+1 is `!`
/// (its other spelling), never row 1; a shifted letter is the letter.
fn jump_key(chord: Chord) -> Option<Action> {
    match (chord.key, chord.mods) {
        (Key::Escape, _) => Some(Action::Popup(PopupAction::Close)),
        (Key::Char(';'), CTRL) => Some(Action::Popup(PopupAction::Close)),
        (_, m) if m.command_like() => None,
        (Key::Char(c), SHIFT) if !c.is_ascii_alphabetic() => None,
        (Key::Char(c), _) => Some(Action::AgentJumpKey(c.to_ascii_lowercase())),
        _ => None,
    }
}

/// What a key does in copy mode before the Vim grammar reads it: Cmd+C copies and leaves, as
/// `y` does (`feature-inventory.md` §22).
pub fn copy_key(chord: Chord) -> Option<Action> {
    match (chord.key, chord.mods) {
        (Key::Char('c'), CMD) => Some(Action::Copy(CopyAction::Yank)),
        _ => None,
    }
}

/// A press as copy mode's grammar reads it: the character it typed (`G` for Shift+G, `$` for
/// Shift+4), Ctrl with a letter, or a named key. `None` for a chord with Cmd or Alt, and other
/// keys copy mode doesn't read; those go to the window's own bindings.
pub fn vim_key(press: Press) -> Option<copy_mode::Key> {
    use copy_mode::Key as V;
    if let Some(Chord {
        key: Key::Char(c),
        mods,
    }) = press.typed
        && !mods.command_like()
    {
        return Some(V::Char(c));
    }
    let Chord { key, mods } = press.chord;
    if mods.cmd || mods.alt {
        return None;
    }
    if mods.ctrl {
        return match key {
            Key::Char(c) if c.is_ascii_lowercase() && !mods.shift => Some(V::Ctrl(c)),
            _ => None,
        };
    }
    Some(match key {
        Key::Char(c) if mods.shift => V::Char(shifted(c)),
        Key::Char(c) => V::Char(c),
        Key::Space => V::Char(' '),
        Key::Escape => V::Escape,
        Key::Enter => V::Enter,
        Key::Backspace => V::Backspace,
        Key::Left => V::Left,
        Key::Right => V::Right,
        Key::Up => V::Up,
        Key::Down => V::Down,
        Key::Home => V::Home,
        Key::End => V::End,
        Key::PageUp => V::PageUp,
        Key::PageDown => V::PageDown,
        Key::Tab | Key::Delete | Key::F(_) => return None,
    })
}

/// The character Shift makes of a key on a US layout, for a press that came without its typed
/// spelling (a letter's capital always does).
fn shifted(c: char) -> char {
    const PLAIN: &str = "`1234567890-=[]\\;',./";
    const SHIFTED: &str = "~!@#$%^&*()_+{}|:\"<>?";
    match PLAIN.chars().position(|p| p == c) {
        Some(i) => SHIFTED.chars().nth(i).unwrap_or(c),
        None => c.to_ascii_uppercase(),
    }
}

/// The keys a named copy-mode command (the key file's `copy_mode.<name>`) stands for.
pub fn copy_keys(action: CopyAction) -> &'static [copy_mode::Key] {
    use CopyAction as C;
    use copy_mode::Key as V;
    match action {
        C::Left => &[V::Char('h')],
        C::Down => &[V::Char('j')],
        C::Up => &[V::Char('k')],
        C::Right => &[V::Char('l')],
        // Home rather than `0`, which a count would take as a digit.
        C::LineStart => &[V::Home],
        C::FirstText => &[V::Char('^')],
        C::LineEnd => &[V::Char('$')],
        C::Top => &[V::Char('g'), V::Char('g')],
        C::Bottom => &[V::Char('G')],
        C::ViewTop => &[V::Char('H')],
        C::ViewMiddle => &[V::Char('M')],
        C::ViewBottom => &[V::Char('L')],
        C::HalfPageUp => &[V::Ctrl('u')],
        C::HalfPageDown => &[V::Ctrl('d')],
        C::PageUp => &[V::Ctrl('b')],
        C::PageDown => &[V::Ctrl('f')],
        C::SelectChars => &[V::Char('v')],
        C::SelectLines => &[V::Char('V')],
        C::Yank => &[V::Enter],
        C::Quit => &[V::Char('q')],
        C::Escape => &[V::Escape],
    }
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

/// What the agent-jump bar says after "JUMP" for `count` rows: the labels it takes, then Esc.
/// Letters go a, s, d, f…, so a range like a–d would mislead: up to six are named, else it
/// says "letters" or "two letters" (`cfg/mods/agent-jump/main.js` `render`).
pub fn jump_hint(count: usize) -> String {
    if count == 0 {
        return "No agents to jump to · Esc".into();
    }
    let labels = jump_labels(count);
    let letters = &labels[labels.len().min(10)..];
    let digits = if count <= 9 {
        format!("1–{count}")
    } else {
        "1–9 · 0".into()
    };
    let range = match letters {
        [] => digits,
        [first, ..] => {
            let letters = if letters.len() <= 6 {
                letters.join(" ")
            } else if first.len() > 1 {
                "two letters".into()
            } else {
                "letters".into()
            };
            format!("{digits} · {letters}")
        }
    };
    format!("{range} · Esc")
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
    fn menus_read_arrows_enter_and_escape_and_nothing_types() {
        let s = PopupState::Menu(Menu::tab("tb1", "sp1"));
        for (k, a) in [
            ("up", PopupAction::Up),
            ("down", PopupAction::Down),
            ("enter", PopupAction::Choose),
            ("escape", PopupAction::Close),
        ] {
            assert_eq!(popup_key(&s, chord(k)), Some(Action::Popup(a)), "{k}");
        }
        for k in ["ctrl-n", "ctrl-p", "j", "x"] {
            assert_eq!(popup_key(&s, chord(k)), None, "{k}");
        }
        assert_eq!(s.popup(), None);
        let mut menu = Menu::tab("tb1", "sp1");
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Rename", "New tab", "Close"]);
        menu.step(-1);
        assert_eq!(menu.cursor, 2, "wraps");
        menu.step(1);
        assert_eq!(menu.cursor, 0);
    }

    #[test]
    fn menus_read_arrows_enter_and_escape_and_wrap() {
        let item = |l: &str| MenuItem::new(l, MenuIcon::Terminal, vec![]);
        let mut menu = Menu::new(
            MenuTarget::Pane("p1".into()),
            vec![item("a"), item("b"), item("c")],
            200.0,
        );
        let s = PopupState::Menu(menu.clone());
        assert_eq!(
            popup_key(&s, chord("up")),
            Some(Action::Popup(PopupAction::Up))
        );
        assert_eq!(
            popup_key(&s, chord("down")),
            Some(Action::Popup(PopupAction::Down))
        );
        assert_eq!(
            popup_key(&s, chord("enter")),
            Some(Action::Popup(PopupAction::Choose))
        );
        assert_eq!(
            popup_key(&s, chord("escape")),
            Some(Action::Popup(PopupAction::Close))
        );
        assert_eq!(popup_key(&s, chord("j")), None);
        menu.step(-1);
        assert_eq!(menu.cursor, 2);
        menu.step(1);
        assert_eq!(menu.cursor, 0);
        assert!(MenuItem::new("Close", MenuIcon::Close, vec![]).divided);
        assert!(!item("x").divided);
    }

    #[test]
    fn lists_page_by_ten_and_the_palette_stops_at_its_ends() {
        assert_eq!(
            popup_key(&list(Popup::Palette), chord("pagedown")),
            Some(Action::Popup(PopupAction::PageDown))
        );
        let mut palette = ListPopup::new(Popup::Palette);
        palette.set_rows(15);
        palette.step(-1);
        assert_eq!(palette.cursor, 0, "no wrap");
        palette.step(PAGE);
        palette.step(PAGE);
        assert_eq!(palette.cursor, 14);
        palette.step(1);
        assert_eq!(palette.cursor, 14);
        let mut spaces = ListPopup::new(Popup::SpacePicker);
        spaces.set_rows(15);
        spaces.step(-1);
        assert_eq!(spaces.cursor, 14, "wraps one at a time");
        spaces.step(PAGE);
        assert_eq!(spaces.cursor, 14, "a page stops at the end");
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
        assert_eq!(popup_key(&s, chord("shift-1")), None, "Shift+1 types !");
        assert_eq!(popup_key(&s, chord("!")), Some(Action::AgentJumpKey('!')));
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
    fn copy_mode_reads_keys_as_vim_does() {
        use copy_mode::Key as V;
        let typed = |chord: &str, typed: Option<&str>| Press {
            chord: Chord::parse(chord).unwrap(),
            typed: typed.map(|t| Chord::parse(t).unwrap()),
        };
        for (press, key) in [
            (typed("h", None), Some(V::Char('h'))),
            (typed("shift-g", None), Some(V::Char('G'))),
            (typed("shift-4", Some("$")), Some(V::Char('$'))),
            (typed("shift-4", None), Some(V::Char('$'))),
            (typed("shift-[", None), Some(V::Char('{'))),
            (typed("shift-'", None), Some(V::Char('"'))),
            (typed("space", None), Some(V::Char(' '))),
            (typed("ctrl-d", None), Some(V::Ctrl('d'))),
            (typed("ctrl-v", None), Some(V::Ctrl('v'))),
            (typed("escape", None), Some(V::Escape)),
            (typed("enter", None), Some(V::Enter)),
            (typed("pageup", None), Some(V::PageUp)),
            (typed("cmd-t", None), None),
            (typed("alt-h", None), None),
            (typed("ctrl-shift-d", None), None),
            (typed("tab", None), None),
        ] {
            assert_eq!(vim_key(press), key, "{press}");
        }
        assert_eq!(
            copy_key(chord("cmd-c")),
            Some(Action::Copy(CopyAction::Yank))
        );
        assert_eq!(copy_key(chord("y")), None, "the grammar reads y");
        // Every named command stands for keys.
        for action in CopyAction::ALL {
            assert!(!copy_keys(action).is_empty(), "{action:?}");
        }
        assert_eq!(copy_keys(CopyAction::Top), [V::Char('g'), V::Char('g')]);
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
    fn the_jump_hint_names_the_labels_it_takes() {
        assert_eq!(jump_hint(0), "No agents to jump to · Esc");
        assert_eq!(jump_hint(1), "1–1 · Esc");
        assert_eq!(jump_hint(9), "1–9 · Esc");
        assert_eq!(jump_hint(10), "1–9 · 0 · Esc");
        assert_eq!(jump_hint(13), "1–9 · 0 · a s d · Esc");
        assert_eq!(jump_hint(16), "1–9 · 0 · a s d f g h · Esc");
        assert_eq!(jump_hint(17), "1–9 · 0 · letters · Esc");
        assert_eq!(jump_hint(37), "1–9 · 0 · two letters · Esc");
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
        assert!(Mode::Copy(CopyState::new("t")).holds_keyboard());
        assert_eq!(list(Popup::Help).popup(), Some(Popup::Help));
        assert_eq!(
            PopupState::Jump(JumpPopup::default()).popup(),
            Some(Popup::AgentJump)
        );
    }
}
