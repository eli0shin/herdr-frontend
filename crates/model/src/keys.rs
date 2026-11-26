//! Keys as plain data, with no Rune types: a [`Chord`] (one key plus modifiers), written in Eli's
//! `keys.json` syntax (`cmd-shift-]`, `ctrl-g s` for a sequence), and the [`Keymap`] that maps
//! sequences to [`Action`]s in a [`Context`].
//!
//! [`Keymap::eli`] is the effective table of `docs/research/feature-inventory.md` §22: `layout:eli`
//! folded over `global`, plus the `terminal` section. The keys inside popups and copy mode are not
//! here; their machines read them (`crate::mode`). The app turns a Rune key event into a
//! [`Chord`]; the key file and live reload are ticket 016's.

use std::fmt;

use crate::action::{Action, AppCommand, Direction, Popup, Split, TerminalAction};

/// Held modifiers. `cmd` is ⌘ on macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Mods {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        cmd: false,
        ctrl: false,
        alt: false,
        shift: false,
    };

    pub fn is_empty(self) -> bool {
        self == Mods::NONE
    }

    /// Cmd, Ctrl or Alt: keys a popup's letter handling ignores.
    pub fn command_like(self) -> bool {
        self.cmd || self.ctrl || self.alt
    }
}

/// A key without its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// A character key, lowercase for letters: `g`, `]`, `1`, `!`.
    Char(char),
    Enter,
    Escape,
    Tab,
    Space,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// F1-F24.
    F(u8),
}

const NAMED: &[(&str, Key)] = &[
    ("enter", Key::Enter),
    ("escape", Key::Escape),
    ("tab", Key::Tab),
    ("space", Key::Space),
    ("backspace", Key::Backspace),
    ("delete", Key::Delete),
    ("left", Key::Left),
    ("right", Key::Right),
    ("up", Key::Up),
    ("down", Key::Down),
    ("home", Key::Home),
    ("end", Key::End),
    ("pageup", Key::PageUp),
    ("pagedown", Key::PageDown),
];

impl Key {
    fn parse(text: &str) -> Option<Key> {
        if let Some((_, key)) = NAMED.iter().find(|(name, _)| *name == text) {
            return Some(*key);
        }
        if let Some(n) = text.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            return (1..=24).contains(&n).then_some(Key::F(n));
        }
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(Key::Char(c.to_ascii_lowercase())),
            _ => None,
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "f{n}"),
            named => {
                let name = NAMED.iter().find(|(_, k)| k == named).map(|(n, _)| *n);
                f.write_str(name.unwrap_or("?"))
            }
        }
    }
}

/// One key press: a key and the modifiers held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub key: Key,
    pub mods: Mods,
}

/// Why a key could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadKey(pub String);

impl fmt::Display for BadKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "can't read the key {:?}", self.0)
    }
}

impl std::error::Error for BadKey {}

impl Chord {
    pub const fn new(key: Key, mods: Mods) -> Chord {
        Chord { key, mods }
    }

    /// A key with no modifiers.
    pub const fn plain(key: Key) -> Chord {
        Chord::new(key, Mods::NONE)
    }

    pub const fn char(c: char) -> Chord {
        Chord::plain(Key::Char(c))
    }

    pub fn ctrl(c: char) -> Chord {
        Chord::new(
            Key::Char(c),
            Mods {
                ctrl: true,
                ..Mods::NONE
            },
        )
    }

    /// Reads one chord: modifiers (`cmd`, `ctrl`, `alt`, `shift`) joined to the key by `-`.
    /// The key may itself be `-` or `+` (`cmd--`, `cmd-+`).
    pub fn parse(text: &str) -> Result<Chord, BadKey> {
        let bad = || BadKey(text.to_owned());
        let mut rest = text.trim();
        let mut mods = Mods::NONE;
        while let Some((head, tail)) = rest.split_once('-') {
            if tail.is_empty() {
                break;
            }
            let flag = match head {
                "cmd" => &mut mods.cmd,
                "ctrl" => &mut mods.ctrl,
                "alt" => &mut mods.alt,
                "shift" => &mut mods.shift,
                _ => break,
            };
            if *flag {
                return Err(bad());
            }
            *flag = true;
            rest = tail;
        }
        let key = Key::parse(rest).ok_or_else(bad)?;
        Ok(Chord { key, mods })
    }

    /// Reads a sequence: chords separated by spaces (`ctrl-g shift-q`).
    pub fn parse_sequence(text: &str) -> Result<Vec<Chord>, BadKey> {
        let chords: Vec<Chord> = text
            .split_whitespace()
            .map(Chord::parse)
            .collect::<Result<_, _>>()?;
        if chords.is_empty() {
            return Err(BadKey(text.to_owned()));
        }
        Ok(chords)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Mods {
            cmd,
            ctrl,
            alt,
            shift,
        } = self.mods;
        for (held, name) in [
            (cmd, "cmd-"),
            (ctrl, "ctrl-"),
            (alt, "alt-"),
            (shift, "shift-"),
        ] {
            if held {
                f.write_str(name)?;
            }
        }
        write!(f, "{}", self.key)
    }
}

/// Which bindings apply: `Terminal` ones while a terminal pane has the keyboard, and they win
/// over `Global` ones (§22: `terminal` overrides `layout:eli`, which overrides `global`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Context {
    Global,
    Terminal,
}

/// A key sequence bound to an action.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub context: Context,
    pub keys: Vec<Chord>,
    pub action: Action,
}

/// What a sequence typed so far means.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup<'a> {
    /// A whole binding.
    Action(&'a Action),
    /// The start of one or more longer bindings: wait for the next key.
    Pending,
    None,
}

/// Key sequences and their actions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    pub fn new() -> Keymap {
        Keymap::default()
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Binds a sequence, replacing an earlier binding of the same keys in that context.
    pub fn bind(&mut self, context: Context, keys: Vec<Chord>, action: Action) {
        self.bindings
            .retain(|b| !(b.context == context && b.keys == keys));
        self.bindings.push(Binding {
            context,
            keys,
            action,
        });
    }

    /// Binds every sequence in `keys` (Eli's syntax) to `action`. Panics on a bad key: for
    /// built-in tables only.
    fn bind_all(&mut self, context: Context, keys: &[&str], action: Action) {
        for text in keys {
            let keys = Chord::parse_sequence(text).expect("built-in key");
            self.bind(context, keys, action.clone());
        }
    }

    /// What `keys` mean, with the terminal's bindings first while it has the keyboard. A whole
    /// binding wins over a longer one it starts.
    pub fn lookup(&self, terminal: bool, keys: &[Chord]) -> Lookup<'_> {
        let contexts: &[Context] = if terminal {
            &[Context::Terminal, Context::Global]
        } else {
            &[Context::Global]
        };
        for context in contexts {
            if let Some(b) = self
                .bindings
                .iter()
                .find(|b| b.context == *context && b.keys == keys)
            {
                return Lookup::Action(&b.action);
            }
        }
        let starts = self.bindings.iter().any(|b| {
            contexts.contains(&b.context) && b.keys.len() > keys.len() && b.keys.starts_with(keys)
        });
        if starts {
            Lookup::Pending
        } else {
            Lookup::None
        }
    }

    /// The sequences bound to an action, in binding order (for help and menu hints).
    pub fn keys_for(&self, action: &Action) -> Vec<&[Chord]> {
        self.bindings
            .iter()
            .filter(|b| &b.action == action)
            .map(|b| b.keys.as_slice())
            .collect()
    }

    /// Eli's effective keys (§22).
    pub fn eli() -> Keymap {
        use Action as A;
        use Context::{Global as G, Terminal as T};
        let mut k = Keymap::new();
        let app = |c| A::App(c);
        k.bind_all(G, &["cmd-q"], app(AppCommand::Quit));
        k.bind_all(G, &["cmd-h"], app(AppCommand::Hide));
        k.bind_all(G, &["cmd-alt-h"], app(AppCommand::HideOthers));
        k.bind_all(G, &["cmd-t", "ctrl-g t"], A::TabNew);
        k.bind_all(G, &["cmd-w", "ctrl-g shift-q"], A::TabClose);
        k.bind_all(G, &["ctrl-tab", "cmd-shift-]", "ctrl-g n"], A::TabNext);
        k.bind_all(
            G,
            &["ctrl-shift-tab", "cmd-shift-[", "ctrl-g p"],
            A::TabPrev,
        );
        for n in 1..=9u8 {
            k.bind_all(
                G,
                &[&format!("cmd-{n}"), &format!("ctrl-g {n}")],
                A::TabSelect(n),
            );
            k.bind_all(G, &[&format!("ctrl-{n}")], A::SpaceSelect(n));
        }
        k.bind_all(G, &["cmd-alt-]"], A::SpaceNext);
        k.bind_all(G, &["cmd-alt-["], A::SpacePrev);
        k.bind_all(G, &["cmd-]", "ctrl-g j"], A::AgentNext);
        k.bind_all(G, &["cmd-[", "ctrl-g k"], A::AgentPrev);
        for (n, shifted) in "!@#$%^&*()".chars().enumerate() {
            let n = n as u8 + 1;
            let digit = n % 10;
            k.bind_all(
                G,
                &[
                    &format!("ctrl-g shift-{digit}"),
                    &format!("ctrl-g {shifted}"),
                ],
                A::AgentSelect(n),
            );
        }
        k.bind_all(G, &["ctrl-;"], A::Open(Popup::AgentJump));
        k.bind_all(G, &["ctrl-g o"], A::JumpBack);
        k.bind_all(G, &["ctrl-g i"], A::JumpForward);
        k.bind_all(G, &["cmd-r"], A::Review);
        k.bind_all(G, &["cmd-shift-l"], app(AppCommand::LayoutSwitch));
        k.bind_all(G, &["cmd-n", "ctrl-g ctrl-s"], A::Open(Popup::NewSpace));
        k.bind_all(G, &["cmd-p"], A::Open(Popup::Picker));
        k.bind_all(G, &["ctrl-g s"], A::Open(Popup::SpacePicker));
        k.bind_all(G, &["ctrl-g /"], A::Open(Popup::AgentMachines));
        k.bind_all(G, &["ctrl-g f"], A::Open(Popup::PrFinder));
        k.bind_all(G, &["ctrl-g ["], A::CopyMode);
        k.bind_all(G, &["cmd-shift-p"], A::Open(Popup::Palette));
        k.bind_all(G, &["cmd-,", "ctrl-g c"], A::Open(Popup::Settings));
        k.bind_all(G, &["cmd-/", "cmd-shift-/"], A::Open(Popup::Help));
        k.bind_all(G, &["cmd-b", "ctrl-g b"], app(AppCommand::ToggleSidebar));
        k.bind_all(G, &["cmd-=", "cmd-+"], app(AppCommand::ZoomIn));
        k.bind_all(G, &["cmd--"], app(AppCommand::ZoomOut));
        k.bind_all(G, &["cmd-0"], app(AppCommand::ZoomReset));
        k.bind_all(G, &["cmd-d", "ctrl-g v"], A::PaneSplit(Split::Right));
        k.bind_all(G, &["cmd-shift-d", "ctrl-g h"], A::PaneSplit(Split::Down));
        k.bind_all(G, &["cmd-shift-w", "ctrl-g q"], A::PaneClose);
        k.bind_all(G, &["cmd-shift-enter"], A::PaneZoom);
        for (d, arrow, letter) in [
            (Direction::Left, "left", 'h'),
            (Direction::Down, "down", 'j'),
            (Direction::Up, "up", 'k'),
            (Direction::Right, "right", 'l'),
        ] {
            k.bind_all(G, &[&format!("cmd-alt-{arrow}")], A::PaneGo(d));
            k.bind_all(G, &[&format!("ctrl-{letter}")], A::PaneNavigate(d));
            k.bind_all(G, &[&format!("cmd-alt-shift-{arrow}")], A::PaneSwap(d));
            k.bind_all(G, &[&format!("alt-shift-{letter}")], A::PaneResize(d));
        }
        k.bind_all(G, &["f2", "ctrl-g r"], A::TabRename);
        k.bind_all(G, &["shift-f2"], A::SpaceRename);
        k.bind_all(G, &["ctrl-g x"], A::SpaceClose);
        k.bind_all(G, &["ctrl-g d"], app(AppCommand::CloseWindow));
        // Built in and unbindable in Herdr app; the mode machine also answers it in every mode.
        k.bind_all(G, &["ctrl-alt-shift-escape"], app(AppCommand::Reset));

        let send = |text: &str| A::Terminal(TerminalAction::Send(text.to_owned()));
        k.bind_all(T, &["cmd-f"], A::Terminal(TerminalAction::Search));
        k.bind_all(T, &["cmd-c"], A::Terminal(TerminalAction::Copy));
        k.bind_all(T, &["cmd-v", "ctrl-v"], A::Terminal(TerminalAction::Paste));
        k.bind_all(
            T,
            &["ctrl-shift-v"],
            A::Terminal(TerminalAction::PasteImage),
        );
        k.bind_all(T, &["alt-left"], send("\u{1b}b"));
        k.bind_all(T, &["alt-right"], send("\u{1b}f"));
        k.bind_all(T, &["alt-backspace"], send("\u{17}"));
        k.bind_all(T, &["cmd-left"], send("\u{1}"));
        k.bind_all(T, &["cmd-right"], send("\u{5}"));
        k.bind_all(T, &["cmd-backspace"], send("\u{15}"));
        k.bind_all(T, &["ctrl-g ctrl-l"], send("\u{c}"));
        k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(text: &str) -> Vec<Chord> {
        Chord::parse_sequence(text).unwrap()
    }

    #[test]
    fn chords_read_and_print_in_elis_syntax() {
        for text in [
            "cmd-q",
            "ctrl-g",
            "cmd-shift-]",
            "cmd--",
            "cmd-+",
            "cmd-alt-shift-left",
            "shift-f2",
            "ctrl-;",
            "!",
            "f2",
            "alt-backspace",
            "ctrl-shift-tab",
            "cmd-shift-enter",
            "ctrl-alt-shift-escape",
            "pageup",
            "space",
        ] {
            let chord = Chord::parse(text).unwrap();
            assert_eq!(chord.to_string(), text);
        }
        assert_eq!(
            Chord::parse("cmd--").unwrap(),
            Chord::new(
                Key::Char('-'),
                Mods {
                    cmd: true,
                    ..Mods::NONE
                }
            )
        );
        assert_eq!(Chord::parse("-").unwrap(), Chord::char('-'));
        assert_eq!(Chord::parse("G").unwrap(), Chord::char('g'));
        assert_eq!(Chord::parse("f24").unwrap().key, Key::F(24));
    }

    #[test]
    fn modifiers_print_in_one_order_whatever_order_they_were_written() {
        assert_eq!(
            Chord::parse("shift-alt-cmd-left").unwrap().to_string(),
            "cmd-alt-shift-left"
        );
    }

    #[test]
    fn bad_keys_are_errors() {
        for text in [
            "",
            "cmd-",
            "cmd-cmd-q",
            "hyper-q",
            "f0",
            "f25",
            "ab",
            "cmd-ab",
        ] {
            assert!(Chord::parse(text).is_err(), "{text:?}");
        }
        assert!(Chord::parse_sequence("  ").is_err());
        assert_eq!(seq("ctrl-g  shift-q").len(), 2);
    }

    #[test]
    fn whole_bindings_win_and_their_starts_wait() {
        let k = Keymap::eli();
        assert_eq!(
            k.lookup(false, &seq("ctrl-g")),
            Lookup::Pending,
            "ctrl-g starts the prefix sequences"
        );
        assert_eq!(
            k.lookup(false, &seq("ctrl-g s")),
            Lookup::Action(&Action::Open(Popup::SpacePicker))
        );
        assert_eq!(k.lookup(false, &seq("ctrl-g z")), Lookup::None);
        assert_eq!(
            k.lookup(false, &seq("cmd-t")),
            Lookup::Action(&Action::TabNew)
        );
        assert_eq!(k.lookup(false, &seq("x")), Lookup::None);
    }

    #[test]
    fn terminal_bindings_apply_only_while_a_terminal_has_the_keyboard() {
        let k = Keymap::eli();
        let clear = Action::Terminal(TerminalAction::Send("\u{c}".into()));
        assert_eq!(
            k.lookup(true, &seq("ctrl-g ctrl-l")),
            Lookup::Action(&clear)
        );
        assert_eq!(k.lookup(false, &seq("ctrl-g ctrl-l")), Lookup::None);
        assert_eq!(k.lookup(false, &seq("cmd-c")), Lookup::None);
        // Global keys still work in a terminal.
        assert_eq!(
            k.lookup(true, &seq("cmd-t")),
            Lookup::Action(&Action::TabNew)
        );
    }

    #[test]
    fn a_terminal_binding_overrides_a_global_one_with_the_same_keys() {
        let mut k = Keymap::new();
        k.bind(Context::Global, seq("cmd-f"), Action::TabNew);
        k.bind(
            Context::Terminal,
            seq("cmd-f"),
            Action::Terminal(TerminalAction::Search),
        );
        assert_eq!(
            k.lookup(true, &seq("cmd-f")),
            Lookup::Action(&Action::Terminal(TerminalAction::Search))
        );
        assert_eq!(
            k.lookup(false, &seq("cmd-f")),
            Lookup::Action(&Action::TabNew)
        );
    }

    #[test]
    fn binding_again_replaces_the_earlier_binding() {
        let mut k = Keymap::new();
        k.bind(Context::Global, seq("cmd-t"), Action::TabNew);
        k.bind(Context::Global, seq("cmd-t"), Action::TabClose);
        assert_eq!(k.bindings().len(), 1);
        assert_eq!(
            k.lookup(false, &seq("cmd-t")),
            Lookup::Action(&Action::TabClose)
        );
    }

    /// The §22 table, row by row: every key there runs the action named there.
    #[test]
    fn elis_table_is_bound_as_written() {
        let global: &[(&str, &str)] = &[
            ("cmd-q", "app.quit"),
            ("cmd-h", "app.hide"),
            ("cmd-alt-h", "app.hide_others"),
            ("cmd-t", "tab.new"),
            ("ctrl-g t", "tab.new"),
            ("cmd-w", "tab.close"),
            ("ctrl-g shift-q", "tab.close"),
            ("ctrl-tab", "navigator-status.tab-next"),
            ("cmd-shift-]", "navigator-status.tab-next"),
            ("ctrl-g n", "navigator-status.tab-next"),
            ("ctrl-shift-tab", "navigator-status.tab-prev"),
            ("cmd-shift-[", "navigator-status.tab-prev"),
            ("ctrl-g p", "navigator-status.tab-prev"),
            ("cmd-alt-]", "space.next"),
            ("cmd-alt-[", "space.prev"),
            ("cmd-]", "agent.next"),
            ("ctrl-g j", "agent.next"),
            ("cmd-[", "agent.prev"),
            ("ctrl-g k", "agent.prev"),
            ("ctrl-g shift-0", "navigator-status.agent-10"),
            ("ctrl-g )", "navigator-status.agent-10"),
            ("ctrl-;", "mod.open:agent-jump"),
            ("ctrl-g o", "navigator-status.jump-list-back"),
            ("ctrl-g i", "navigator-status.jump-list-forward"),
            ("cmd-r", "navigator-status.review"),
            ("cmd-shift-l", "layout.switch"),
            ("cmd-n", "mod.open:new-space"),
            ("ctrl-g ctrl-s", "mod.open:new-space"),
            ("cmd-p", "mod.open:picker"),
            ("ctrl-g s", "mod.open:space-picker"),
            ("ctrl-g /", "mod.open:agent-machines"),
            ("ctrl-g f", "mod.open:pr-finder"),
            ("ctrl-g [", "mod.open:copy-mode"),
            ("cmd-shift-p", "app.palette"),
            ("cmd-,", "app.settings"),
            ("ctrl-g c", "app.settings"),
            ("cmd-/", "app.help"),
            ("cmd-shift-/", "app.help"),
            ("cmd-b", "layout.toggle:sidebar"),
            ("ctrl-g b", "layout.toggle:sidebar"),
            ("cmd-=", "app.zoom_in"),
            ("cmd-+", "app.zoom_in"),
            ("cmd--", "app.zoom_out"),
            ("cmd-0", "app.zoom_reset"),
            ("cmd-d", "pane.split:right"),
            ("ctrl-g v", "pane.split:right"),
            ("cmd-shift-d", "pane.split:down"),
            ("ctrl-g h", "pane.split:down"),
            ("cmd-shift-w", "pane.close"),
            ("ctrl-g q", "pane.close"),
            ("cmd-shift-enter", "pane.zoom"),
            ("ctrl-h", "navigator-status.left"),
            ("ctrl-j", "navigator-status.down"),
            ("ctrl-k", "navigator-status.up"),
            ("ctrl-l", "navigator-status.right"),
            ("alt-shift-h", "pane.resize:left"),
            ("alt-shift-j", "pane.resize:down"),
            ("alt-shift-k", "pane.resize:up"),
            ("alt-shift-l", "pane.resize:right"),
            ("f2", "tab.rename"),
            ("ctrl-g r", "tab.rename"),
            ("shift-f2", "space.rename"),
            ("ctrl-g x", "space.close"),
            ("ctrl-g d", "window.close"),
            ("ctrl-alt-shift-escape", "app.reset"),
        ];
        let k = Keymap::eli();
        let check = |terminal: bool, keys: &str, name: &str| {
            let want = Action::parse(name).unwrap_or_else(|| panic!("{name} parses"));
            assert_eq!(
                k.lookup(terminal, &seq(keys)),
                Lookup::Action(&want),
                "{keys}"
            );
        };
        for (keys, name) in global {
            check(false, keys, name);
        }
        for n in 1..=9 {
            check(
                false,
                &format!("cmd-{n}"),
                &format!("navigator-status.tab-{n}"),
            );
            check(
                false,
                &format!("ctrl-g {n}"),
                &format!("navigator-status.tab-{n}"),
            );
            check(false, &format!("ctrl-{n}"), &format!("space.select:{n}"));
            check(
                false,
                &format!("ctrl-g shift-{n}"),
                &format!("navigator-status.agent-{n}"),
            );
        }
        for (shifted, n) in "!@#$%^&*(".chars().zip(1..) {
            check(
                false,
                &format!("ctrl-g {shifted}"),
                &format!("navigator-status.agent-{n}"),
            );
        }
        for d in ["left", "right", "up", "down"] {
            check(false, &format!("cmd-alt-{d}"), &format!("pane.go:{d}"));
            check(
                false,
                &format!("cmd-alt-shift-{d}"),
                &format!("pane.swap:{d}"),
            );
        }
        for (keys, name) in [
            ("cmd-f", "terminal.search"),
            ("cmd-c", "terminal.copy"),
            ("cmd-v", "terminal.paste"),
            ("ctrl-v", "terminal.paste"),
            ("ctrl-shift-v", "terminal.paste_image"),
            ("alt-left", "send:\u{1b}b"),
            ("alt-right", "send:\u{1b}f"),
            ("alt-backspace", "send:\u{17}"),
            ("cmd-left", "send:\u{1}"),
            ("cmd-right", "send:\u{5}"),
            ("cmd-backspace", "send:\u{15}"),
            ("ctrl-g ctrl-l", "send:\u{c}"),
        ] {
            check(true, keys, name);
        }
    }

    #[test]
    fn keys_for_lists_every_sequence_of_an_action() {
        let k = Keymap::eli();
        let keys: Vec<String> = k
            .keys_for(&Action::TabNew)
            .iter()
            .map(|s| s.iter().map(Chord::to_string).collect::<Vec<_>>().join(" "))
            .collect();
        assert_eq!(keys, ["cmd-t", "ctrl-g t"]);
    }
}
