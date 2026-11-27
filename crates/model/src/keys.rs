//! Keys as plain data, with no Rune types: a [`Chord`] (one key plus modifiers), written in Eli's
//! `keys.json` syntax (`cmd-shift-]`, `ctrl-g s` for a sequence), and the [`Keymap`] that maps
//! sequences to [`Action`]s in a [`Context`].
//!
//! A key file ([`Keymap::from_json`]) is Eli's `keys.json` form, and [`Keymap::eli`] reads Eli's
//! own ([`ELI_KEYS`]): the table of `docs/research/feature-inventory.md` §22. The keys popups
//! and copy mode read themselves are not here; their machines have them (`crate::mode`), after
//! the file's `popup` and `copy` sections. The app turns a Rune key event into a [`Press`],
//! reads the file from herdr-frontend's home and reloads it live.

use std::fmt;

use crate::action::{Action, AppCommand};

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

/// One key press as the keymap reads it: the chord by the key's unshifted character
/// (`shift-1`, `cmd-shift-]`), and, when Shift made the key type another character, that
/// spelling too (`!`, `cmd-}`). A binding written either way matches, as Herdr app's shifted
/// punctuation does (`cfg/AGENTS.md` "Keys").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Press {
    pub chord: Chord,
    pub typed: Option<Chord>,
}

impl Press {
    /// A press with one spelling.
    pub const fn new(chord: Chord) -> Press {
        Press { chord, typed: None }
    }

    /// Whether `chord` is one of this press's spellings.
    pub fn matches(&self, chord: &Chord) -> bool {
        self.chord == *chord || self.typed.as_ref() == Some(chord)
    }

    /// The first spelling `f` answers for.
    pub fn first<T>(&self, mut f: impl FnMut(Chord) -> Option<T>) -> Option<T> {
        f(self.chord).or_else(|| self.typed.and_then(f))
    }
}

impl From<Chord> for Press {
    fn from(chord: Chord) -> Press {
        Press::new(chord)
    }
}

impl fmt::Display for Press {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.chord)
    }
}

/// Where a binding applies, as the key file's sections name them:
///
/// - `Global` (`global`) and `Layout` (`layout:eli`), which wins over it;
/// - `Terminal` (`terminal`): while a terminal pane has the keyboard, over both;
/// - `Popup` (`popup`): while a popup or dialog is open, before its own keys;
/// - `Copy` (`copy`, or Eli's `mod:copy-mode`): in copy mode, before its own keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Context {
    Global,
    Layout,
    Terminal,
    Popup,
    Copy,
}

impl Context {
    /// The contexts a key outside popups and copy mode looks in, most specific first.
    pub fn normal(terminal: bool) -> &'static [Context] {
        if terminal {
            &[Context::Terminal, Context::Layout, Context::Global]
        } else {
            &[Context::Layout, Context::Global]
        }
    }

    /// The key file section this context is written as.
    pub fn section(self) -> &'static str {
        match self {
            Context::Global => "global",
            Context::Layout => LAYOUT_SECTION,
            Context::Terminal => "terminal",
            Context::Popup => "popup",
            Context::Copy => "copy",
        }
    }
}

/// The one layout herdr-frontend has: Eli's.
pub const LAYOUT_SECTION: &str = "layout:eli";

/// The escape hatch: built in, answered in every mode, and never rebound
/// (`feature-inventory.md` §1).
pub const RESET: &str = "ctrl-alt-shift-escape";

/// The key file's name in herdr-frontend's home.
pub const KEY_FILE: &str = "keys.json";

/// Eli's `keys.json` (`feature-inventory.md` §22): the default key file, written to the home
/// when it has none.
pub const ELI_KEYS: &str = include_str!("keys.json");

/// A key sequence bound to an action.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub context: Context,
    pub keys: Vec<Chord>,
    /// `None`: the key goes to the terminal's program (the key file's `"terminal"`), over a
    /// less specific binding.
    pub action: Option<Action>,
}

/// What a sequence typed so far means.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup<'a> {
    /// A whole binding.
    Action(&'a Action),
    /// Bound to `"terminal"`: the program gets the key.
    Program,
    /// The start of one or more longer bindings: wait for the next key.
    Pending,
    None,
}

/// Why a key file can't be used. The app keeps the keys it had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyFileError(pub Vec<String>);

impl fmt::Display for KeyFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("; "))
    }
}

impl std::error::Error for KeyFileError {}

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
        self.put(context, keys, Some(action));
    }

    /// Leaves a key to the terminal's program in `context` (the key file's `"terminal"`).
    pub fn pass(&mut self, context: Context, keys: Vec<Chord>) {
        self.put(context, keys, None);
    }

    fn put(&mut self, context: Context, keys: Vec<Chord>, action: Option<Action>) {
        self.bindings
            .retain(|b| !(b.context == context && b.keys == keys));
        self.bindings.push(Binding {
            context,
            keys,
            action,
        });
    }

    /// What `keys` mean outside popups and copy mode, with the terminal's bindings first while
    /// it has the keyboard. Each key has one spelling here; [`Keymap::lookup_in`] takes both.
    pub fn lookup(&self, terminal: bool, keys: &[Chord]) -> Lookup<'_> {
        let presses: Vec<Press> = keys.iter().copied().map(Press::new).collect();
        self.lookup_in(Context::normal(terminal), &presses)
    }

    /// What `keys` mean in `contexts`, most specific first. A whole binding wins over a longer
    /// one it starts, in any of them.
    pub fn lookup_in(&self, contexts: &[Context], keys: &[Press]) -> Lookup<'_> {
        let matches = |chords: &[Chord]| chords.iter().zip(keys).all(|(c, p)| p.matches(c));
        for context in contexts {
            if let Some(b) = self
                .bindings
                .iter()
                .find(|b| b.context == *context && b.keys.len() == keys.len() && matches(&b.keys))
            {
                return match &b.action {
                    Some(action) => Lookup::Action(action),
                    None => Lookup::Program,
                };
            }
        }
        let starts = self.bindings.iter().any(|b| {
            contexts.contains(&b.context)
                && b.keys.len() > keys.len()
                && matches(&b.keys[..keys.len()])
        });
        if starts {
            Lookup::Pending
        } else {
            Lookup::None
        }
    }

    /// The bindings that can run: all but a `global` one whose keys `layout:eli` binds too.
    pub fn effective(&self) -> impl Iterator<Item = &Binding> {
        self.bindings.iter().filter(|b| {
            b.context != Context::Global
                || !self
                    .bindings
                    .iter()
                    .any(|l| l.context == Context::Layout && l.keys == b.keys)
        })
    }

    /// The sequences that run an action, once each, in binding order (for help and menu hints).
    pub fn keys_for(&self, action: &Action) -> Vec<&[Chord]> {
        let mut out: Vec<&[Chord]> = Vec::new();
        for b in self.effective() {
            if b.action.as_ref() == Some(action) && !out.contains(&b.keys.as_slice()) {
                out.push(b.keys.as_slice());
            }
        }
        out
    }

    /// Reads a key file in Eli's `keys.json` form: sections of `"<keys>": "<action>"`.
    ///
    /// - Sections: `global`, `layout:eli`, `terminal`, `popup`, and `copy` (also Eli's
    ///   `mod:copy-mode`). Other `layout:<name>` and `mod:<name>` sections are for layouts and
    ///   mods herdr-frontend doesn't have, and are skipped.
    /// - Keys in Eli's syntax ([`Chord::parse_sequence`]): `cmd-shift-]`, `ctrl-g s`. Popups and
    ///   copy mode take single keys.
    /// - Actions are command names ([`Action::parse`], Herdr app's names included), `send:<text>`,
    ///   or `"terminal"`: the program gets the key (single keys in `global`, `layout:eli` and
    ///   `terminal`).
    /// - `ctrl-alt-shift-escape` is built in and can't be bound.
    ///
    /// The file is the whole keymap (as Herdr app's is): keys it leaves out are unbound. Every
    /// problem is reported; one is enough to refuse the file.
    pub fn from_json(text: &str) -> Result<Keymap, KeyFileError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| KeyFileError(vec![format!("not JSON: {e}")]))?;
        let serde_json::Value::Object(sections) = value else {
            return Err(KeyFileError(vec![
                "the file must be an object of sections".into(),
            ]));
        };
        let reset = Chord::parse_sequence(RESET).expect("built-in key");
        let mut k = Keymap::new();
        let mut errors = Vec::new();
        for (section, keys) in &sections {
            let context = match section.as_str() {
                "global" => Context::Global,
                LAYOUT_SECTION => Context::Layout,
                "terminal" => Context::Terminal,
                "popup" => Context::Popup,
                "copy" | "mod:copy-mode" => Context::Copy,
                s if s.starts_with("layout:") || s.starts_with("mod:") => continue,
                s => {
                    errors.push(format!("unknown section {s:?}"));
                    continue;
                }
            };
            let serde_json::Value::Object(keys) = keys else {
                errors.push(format!("{section}: must be an object of keys"));
                continue;
            };
            for (text, action) in keys {
                let at = format!("{section} {text:?}");
                let chords = match Chord::parse_sequence(text) {
                    Ok(chords) => chords,
                    Err(e) => {
                        errors.push(format!("{at}: {e}"));
                        continue;
                    }
                };
                if chords == reset {
                    errors.push(format!("{at}: {RESET} is built in and can't be bound"));
                    continue;
                }
                let one_key = chords.len() == 1;
                if matches!(context, Context::Popup | Context::Copy) && !one_key {
                    errors.push(format!("{at}: popups and copy mode take single keys"));
                    continue;
                }
                let Some(name) = action.as_str() else {
                    errors.push(format!("{at}: the action must be a command name"));
                    continue;
                };
                if name == "terminal" {
                    if one_key && !matches!(context, Context::Popup | Context::Copy) {
                        k.pass(context, chords);
                    } else {
                        errors.push(format!(
                            "{at}: only a single key outside popups and copy mode can go to the terminal"
                        ));
                    }
                    continue;
                }
                match Action::parse(name) {
                    Some(action) => k.bind(context, chords, action),
                    None => errors.push(format!("{at}: unknown action {name:?}")),
                }
            }
        }
        if !errors.is_empty() {
            return Err(KeyFileError(errors));
        }
        // Listed for help; the model answers it before any binding, in every mode.
        k.bind(Context::Global, reset, Action::App(AppCommand::Reset));
        Ok(k)
    }

    /// Eli's keys ([`ELI_KEYS`]): §22's table, `layout:eli` over `global`, `terminal` over both,
    /// and copy mode's Esc.
    pub fn eli() -> Keymap {
        Keymap::from_json(ELI_KEYS).expect("Eli's key file reads")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Popup, TerminalAction};

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

    fn press(chord: &str, typed: Option<&str>) -> Press {
        Press {
            chord: Chord::parse(chord).unwrap(),
            typed: typed.map(|t| Chord::parse(t).unwrap()),
        }
    }

    #[test]
    fn a_press_matches_a_binding_written_either_way() {
        let k = Keymap::eli();
        let g = press("ctrl-g", None);
        let normal = Context::normal(false);
        // Shift+1 types `!`: `ctrl-g shift-1` and `ctrl-g !` are both Eli's.
        let bang = press("shift-1", Some("!"));
        assert_eq!(
            k.lookup_in(normal, &[g, bang]),
            Lookup::Action(&Action::AgentSelect(1))
        );
        // `cmd-+` is written by the character Shift types on `=`.
        assert_eq!(
            k.lookup_in(normal, &[press("cmd-shift-=", Some("cmd-+"))]),
            Lookup::Action(&Action::App(AppCommand::ZoomIn))
        );
        assert_eq!(
            k.lookup_in(normal, &[press("cmd-shift-]", Some("cmd-}"))]),
            Lookup::Action(&Action::TabNext)
        );
    }

    #[test]
    fn keymap_eli_reads_elis_key_file_section_by_section() {
        let k = Keymap::eli();
        let at = |context: Context, keys: &str| {
            k.bindings()
                .iter()
                .find(|b| b.context == context && b.keys == seq(keys))
                .and_then(|b| b.action.clone())
        };
        // `global` keeps its own binding; `layout:eli` overrides it in the eli layout.
        assert_eq!(
            at(Context::Global, "ctrl-h"),
            Some(Action::PaneGo(crate::Direction::Left))
        );
        assert_eq!(
            at(Context::Layout, "ctrl-h"),
            Some(Action::PaneNavigate(crate::Direction::Left))
        );
        assert_eq!(
            k.lookup(false, &seq("ctrl-h")),
            Lookup::Action(&Action::PaneNavigate(crate::Direction::Left))
        );
        assert_eq!(
            at(Context::Copy, "escape"),
            Some(Action::Copy(crate::CopyAction::Escape))
        );
        assert_eq!(
            at(Context::Global, RESET),
            Some(Action::App(AppCommand::Reset))
        );
    }

    #[test]
    fn a_key_file_is_the_whole_keymap() {
        let k = Keymap::from_json(
            r#"{
                "global": {"cmd-k": "tab.new", "ctrl-a s": "mod.open:space-picker"},
                "terminal": {"ctrl-h": "terminal", "cmd-k": "send:\f"},
                "popup": {"ctrl-j": "popup.down"},
                "copy": {"x": "copy_mode.yank"},
                "layout:terminals": {"cmd-q": "nonsense"},
                "mod:editor": {"ctrl-s": "editor.save"}
            }"#,
        )
        .unwrap();
        assert_eq!(k.lookup(false, &seq("cmd-t")), Lookup::None);
        assert_eq!(
            k.lookup(false, &seq("cmd-k")),
            Lookup::Action(&Action::TabNew)
        );
        assert_eq!(k.lookup(false, &seq("ctrl-a")), Lookup::Pending);
        assert_eq!(
            k.lookup(true, &seq("ctrl-a s")),
            Lookup::Action(&Action::Open(Popup::SpacePicker))
        );
        assert_eq!(k.lookup(true, &seq("ctrl-h")), Lookup::Program);
        assert_eq!(
            k.lookup(true, &seq("cmd-k")),
            Lookup::Action(&Action::Terminal(TerminalAction::Send("\u{c}".into())))
        );
        assert_eq!(
            k.lookup_in(&[Context::Popup], &[press("ctrl-j", None)]),
            Lookup::Action(&Action::Popup(crate::PopupAction::Down))
        );
        assert_eq!(
            k.lookup_in(&[Context::Copy], &[press("x", None)]),
            Lookup::Action(&Action::Copy(crate::CopyAction::Yank))
        );
        // Popup and copy keys don't apply outside them.
        assert_eq!(k.lookup(true, &seq("ctrl-j")), Lookup::None);
    }

    #[test]
    fn a_bad_key_file_says_what_is_wrong_with_each_key() {
        let err = |text: &str| Keymap::from_json(text).unwrap_err().to_string();
        assert!(err("{").starts_with("not JSON"));
        assert_eq!(err("[]"), "the file must be an object of sections");
        assert_eq!(err(r#"{"gloabl": {}}"#), r#"unknown section "gloabl""#);
        assert_eq!(
            err(r#"{"global": []}"#),
            "global: must be an object of keys"
        );
        assert_eq!(
            err(r#"{"global": {"hyper-k": "tab.new"}}"#),
            r#"global "hyper-k": can't read the key "hyper-k""#
        );
        assert_eq!(
            err(r#"{"global": {"cmd-k": "tab.nope"}}"#),
            r#"global "cmd-k": unknown action "tab.nope""#
        );
        assert_eq!(
            err(r#"{"global": {"cmd-k": {"command": "tab.new"}}}"#),
            r#"global "cmd-k": the action must be a command name"#
        );
        assert_eq!(
            err(r#"{"global": {"ctrl-alt-shift-escape": "tab.new"}}"#),
            r#"global "ctrl-alt-shift-escape": ctrl-alt-shift-escape is built in and can't be bound"#
        );
        assert_eq!(
            err(r#"{"popup": {"ctrl-g j": "popup.down"}}"#),
            r#"popup "ctrl-g j": popups and copy mode take single keys"#
        );
        assert_eq!(
            err(r#"{"global": {"ctrl-g x": "terminal"}}"#),
            r#"global "ctrl-g x": only a single key outside popups and copy mode can go to the terminal"#
        );
        // Every problem at once.
        assert_eq!(
            err(r#"{"global": {"cmd-k": "x", "cmd-j": "y"}}"#),
            r#"global "cmd-j": unknown action "y"; global "cmd-k": unknown action "x""#
        );
    }

    #[test]
    fn effective_bindings_drop_global_keys_the_layout_rebinds() {
        let k = Keymap::eli();
        assert!(
            k.effective()
                .all(|b| !(b.context == Context::Global && b.keys == seq("ctrl-h")))
        );
        assert!(
            k.effective()
                .any(|b| b.context == Context::Global && b.keys == seq("cmd-q"))
        );
        // ctrl-tab runs tab.next from both sections: listed once.
        let next: Vec<String> = k
            .keys_for(&Action::TabNext)
            .iter()
            .map(|s| s.iter().map(Chord::to_string).collect::<Vec<_>>().join(" "))
            .collect();
        assert_eq!(next.iter().filter(|k| *k == "ctrl-tab").count(), 1);
    }
}
