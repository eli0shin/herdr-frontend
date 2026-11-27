//! A Rune key event as a terminal key event: the physical key, every modifier, the modifiers
//! the layout used up making the text, the unshifted character, and press, repeat or release
//! (`docs/research/terminal-rendering.md` §3, "Keys"). The terminal's worker encodes it for the
//! program's modes (legacy, application cursor keys, the kitty keyboard protocol), so a key sends
//! what the program asked for.
//!
//! Chords with Cmd are the app's, and are left alone. Every other key goes to the terminal;
//! keys reach here only after the keymap passed on them (`docs/keys.md`).

use ghostty_vt_sys::*;
use rune::{Key, KeyEvent, PhysicalKeyCode, Tx};
use terminal::{KeyAction, KeyEvent as TermKey, Mods, OptionAsAlt};

/// What a key handler knows about a key beyond its [`KeyEvent`]: Rune's reads of it, and how
/// the terminal treats Option.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyInfo {
    /// Where the key sits ([`Tx::physical_key`]); `None` for a key with no physical origin.
    pub physical: Option<PhysicalKeyCode>,
    /// The character the key types with no modifiers ([`Tx::key_base`]).
    pub base: Option<char>,
    pub action: KeyAction,
    /// An input method is composing: the key is part of it.
    pub composing: bool,
    /// Which Option keys are Alt. Rune hands those over as the key they're on, so their text
    /// is never the layout's.
    pub option_as_alt: OptionAsAlt,
}

impl KeyInfo {
    /// The key a handler is seeing: a press or a repeat in `on_key`.
    pub fn pressed(tx: &Tx, option_as_alt: OptionAsAlt) -> KeyInfo {
        KeyInfo {
            physical: tx.physical_key(),
            base: tx.key_base(),
            action: if tx.key_repeat() {
                KeyAction::Repeat
            } else {
                KeyAction::Press
            },
            composing: false,
            option_as_alt,
        }
    }

    /// The key an `on_key_up` handler is seeing.
    pub fn released(tx: &Tx, option_as_alt: OptionAsAlt) -> KeyInfo {
        KeyInfo {
            action: KeyAction::Release,
            ..KeyInfo::pressed(tx, option_as_alt)
        }
    }
}

/// What `ev` is to a terminal, or `None` for a key the terminal doesn't take: chords with Cmd,
/// and keys with nothing to encode.
pub fn encode(ev: &KeyEvent, info: KeyInfo) -> Option<TermKey> {
    let m = ev.mods;
    if m.cmd || m.logo {
        return None;
    }
    let mut mods = Mods::NONE;
    for (held, bit) in [
        (m.shift, Mods::SHIFT),
        (m.ctrl, Mods::CTRL),
        (m.alt, Mods::ALT),
    ] {
        if held {
            mods = mods | bit;
        }
    }
    let (logical, text) = match &ev.key {
        Key::Text(text) if !text.is_empty() => (char_key(text), Some(text.clone())),
        // A Ctrl chord off macOS: the key, with no text of its own.
        Key::Command(c) => (char_key(&c.to_string()), None),
        key => (named_key(key)?, None),
    };
    let key = info
        .physical
        .map(physical_key)
        .filter(|&k| k != GHOSTTY_KEY_UNIDENTIFIED)
        .unwrap_or(logical);
    let unshifted = info.base.or_else(|| match &ev.key {
        Key::Text(text) => unshifted(text),
        Key::Command(c) => unshifted(&c.to_string()),
        _ => None,
    });
    Some(TermKey {
        action: info.action,
        key,
        mods,
        consumed_mods: text
            .as_deref()
            .map_or(Mods::NONE, |text| consumed(text, mods, unshifted, info)),
        text,
        unshifted,
        composing: info.composing,
    })
}

/// The modifiers the layout spent making `text`, as Ghostty reports them: Shift whenever it's
/// held, and Option when it typed the layout's character rather than acting as Alt.
fn consumed(text: &str, mods: Mods, unshifted: Option<char>, info: KeyInfo) -> Mods {
    let mut used = Mods::NONE;
    if mods.contains(Mods::SHIFT) {
        used = used | Mods::SHIFT;
    }
    if mods.contains(Mods::ALT) {
        let layouts = match info.option_as_alt {
            OptionAsAlt::Both => false,
            OptionAsAlt::No => true,
            // Rune hands an Alt Option key over as the key it's on: text that isn't that key's
            // came from the other Option key, which types the layout's characters.
            OptionAsAlt::Left | OptionAsAlt::Right => {
                let mut chars = text.chars();
                match (chars.next(), chars.next(), unshifted) {
                    (Some(c), None, Some(base)) => !c.to_lowercase().eq(base.to_lowercase()),
                    _ => true,
                }
            }
        };
        if layouts {
            used = used | Mods::ALT;
        }
    }
    used
}

/// A key that types no text, by its logical name.
fn named_key(key: &Key) -> Option<terminal::Key> {
    Some(match key {
        Key::Enter => GHOSTTY_KEY_ENTER,
        Key::Backspace => GHOSTTY_KEY_BACKSPACE,
        Key::Tab => GHOSTTY_KEY_TAB,
        Key::Escape => GHOSTTY_KEY_ESCAPE,
        Key::Delete => GHOSTTY_KEY_DELETE,
        Key::Insert => GHOSTTY_KEY_INSERT,
        Key::Left => GHOSTTY_KEY_ARROW_LEFT,
        Key::Right => GHOSTTY_KEY_ARROW_RIGHT,
        Key::Up => GHOSTTY_KEY_ARROW_UP,
        Key::Down => GHOSTTY_KEY_ARROW_DOWN,
        Key::Home => GHOSTTY_KEY_HOME,
        Key::End => GHOSTTY_KEY_END,
        Key::PageUp => GHOSTTY_KEY_PAGE_UP,
        Key::PageDown => GHOSTTY_KEY_PAGE_DOWN,
        Key::PrintScreen => GHOSTTY_KEY_PRINT_SCREEN,
        Key::ScrollLock => GHOSTTY_KEY_SCROLL_LOCK,
        Key::Pause => GHOSTTY_KEY_PAUSE,
        Key::ContextMenu => GHOSTTY_KEY_CONTEXT_MENU,
        Key::Function(n @ 1..=25) => GHOSTTY_KEY_F1 + terminal::Key::from(*n - 1),
        _ => return None,
    })
}

/// US-layout punctuation typed with Shift, and the key it's on.
const SHIFTED: &[(char, char)] = &[
    ('~', '`'),
    ('!', '1'),
    ('@', '2'),
    ('#', '3'),
    ('$', '4'),
    ('%', '5'),
    ('^', '6'),
    ('&', '7'),
    ('*', '8'),
    ('(', '9'),
    (')', '0'),
    ('_', '-'),
    ('+', '='),
    ('{', '['),
    ('}', ']'),
    ('|', '\\'),
    (':', ';'),
    ('"', '\''),
    ('<', ','),
    ('>', '.'),
    ('?', '/'),
];

/// The character a key's text comes from with no modifiers, on a US layout: `;` for `:`, `a`
/// for `A`. Only for a key with no physical origin, whose layout Rune didn't read.
fn unshifted(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let base = SHIFTED.iter().find(|(s, _)| *s == c).map_or(c, |(_, b)| *b);
    base.to_lowercase().next()
}

/// The key a typed character sits on, on a US layout, for a key with no physical origin (a
/// script's key, a test's). The encoder needs it only under the kitty keyboard protocol, and
/// text it can't place still types.
fn char_key(text: &str) -> terminal::Key {
    match unshifted(text) {
        Some(c @ 'a'..='z') => GHOSTTY_KEY_A + (c as u32 - 'a' as u32),
        Some(c @ '0'..='9') => GHOSTTY_KEY_DIGIT_0 + (c as u32 - '0' as u32),
        Some(' ') => GHOSTTY_KEY_SPACE,
        Some('`') => GHOSTTY_KEY_BACKQUOTE,
        Some('\\') => GHOSTTY_KEY_BACKSLASH,
        Some('[') => GHOSTTY_KEY_BRACKET_LEFT,
        Some(']') => GHOSTTY_KEY_BRACKET_RIGHT,
        Some(',') => GHOSTTY_KEY_COMMA,
        Some('=') => GHOSTTY_KEY_EQUAL,
        Some('-') => GHOSTTY_KEY_MINUS,
        Some('.') => GHOSTTY_KEY_PERIOD,
        Some('\'') => GHOSTTY_KEY_QUOTE,
        Some(';') => GHOSTTY_KEY_SEMICOLON,
        Some('/') => GHOSTTY_KEY_SLASH,
        _ => GHOSTTY_KEY_UNIDENTIFIED,
    }
}

/// Rune's physical key (W3C `code` names) as libghostty-vt's, which uses the same names.
fn physical_key(code: PhysicalKeyCode) -> terminal::Key {
    use PhysicalKeyCode as P;
    let letter = |c: u8| GHOSTTY_KEY_A + terminal::Key::from(c - b'a');
    let digit = |n: u8| GHOSTTY_KEY_DIGIT_0 + terminal::Key::from(n);
    let numpad = |n: u8| GHOSTTY_KEY_NUMPAD_0 + terminal::Key::from(n);
    match code {
        P::KeyA => letter(b'a'),
        P::KeyB => letter(b'b'),
        P::KeyC => letter(b'c'),
        P::KeyD => letter(b'd'),
        P::KeyE => letter(b'e'),
        P::KeyF => letter(b'f'),
        P::KeyG => letter(b'g'),
        P::KeyH => letter(b'h'),
        P::KeyI => letter(b'i'),
        P::KeyJ => letter(b'j'),
        P::KeyK => letter(b'k'),
        P::KeyL => letter(b'l'),
        P::KeyM => letter(b'm'),
        P::KeyN => letter(b'n'),
        P::KeyO => letter(b'o'),
        P::KeyP => letter(b'p'),
        P::KeyQ => letter(b'q'),
        P::KeyR => letter(b'r'),
        P::KeyS => letter(b's'),
        P::KeyT => letter(b't'),
        P::KeyU => letter(b'u'),
        P::KeyV => letter(b'v'),
        P::KeyW => letter(b'w'),
        P::KeyX => letter(b'x'),
        P::KeyY => letter(b'y'),
        P::KeyZ => letter(b'z'),
        P::Digit0 => digit(0),
        P::Digit1 => digit(1),
        P::Digit2 => digit(2),
        P::Digit3 => digit(3),
        P::Digit4 => digit(4),
        P::Digit5 => digit(5),
        P::Digit6 => digit(6),
        P::Digit7 => digit(7),
        P::Digit8 => digit(8),
        P::Digit9 => digit(9),
        P::Backquote => GHOSTTY_KEY_BACKQUOTE,
        P::Backslash => GHOSTTY_KEY_BACKSLASH,
        P::BracketLeft => GHOSTTY_KEY_BRACKET_LEFT,
        P::BracketRight => GHOSTTY_KEY_BRACKET_RIGHT,
        P::Comma => GHOSTTY_KEY_COMMA,
        P::Equal => GHOSTTY_KEY_EQUAL,
        P::IntlBackslash => GHOSTTY_KEY_INTL_BACKSLASH,
        P::IntlRo => GHOSTTY_KEY_INTL_RO,
        P::IntlYen => GHOSTTY_KEY_INTL_YEN,
        P::Minus => GHOSTTY_KEY_MINUS,
        P::Period => GHOSTTY_KEY_PERIOD,
        P::Quote => GHOSTTY_KEY_QUOTE,
        P::Semicolon => GHOSTTY_KEY_SEMICOLON,
        P::Slash => GHOSTTY_KEY_SLASH,
        P::AltLeft => GHOSTTY_KEY_ALT_LEFT,
        P::AltRight => GHOSTTY_KEY_ALT_RIGHT,
        P::Backspace => GHOSTTY_KEY_BACKSPACE,
        P::CapsLock => GHOSTTY_KEY_CAPS_LOCK,
        P::ContextMenu => GHOSTTY_KEY_CONTEXT_MENU,
        P::ControlLeft => GHOSTTY_KEY_CONTROL_LEFT,
        P::ControlRight => GHOSTTY_KEY_CONTROL_RIGHT,
        P::Enter => GHOSTTY_KEY_ENTER,
        P::SuperLeft => GHOSTTY_KEY_META_LEFT,
        P::SuperRight => GHOSTTY_KEY_META_RIGHT,
        P::ShiftLeft => GHOSTTY_KEY_SHIFT_LEFT,
        P::ShiftRight => GHOSTTY_KEY_SHIFT_RIGHT,
        P::Space => GHOSTTY_KEY_SPACE,
        P::Tab => GHOSTTY_KEY_TAB,
        P::Convert => GHOSTTY_KEY_CONVERT,
        P::KanaMode => GHOSTTY_KEY_KANA_MODE,
        P::NonConvert => GHOSTTY_KEY_NON_CONVERT,
        P::Delete => GHOSTTY_KEY_DELETE,
        P::End => GHOSTTY_KEY_END,
        P::Help => GHOSTTY_KEY_HELP,
        P::Home => GHOSTTY_KEY_HOME,
        P::Insert => GHOSTTY_KEY_INSERT,
        P::PageDown => GHOSTTY_KEY_PAGE_DOWN,
        P::PageUp => GHOSTTY_KEY_PAGE_UP,
        P::ArrowDown => GHOSTTY_KEY_ARROW_DOWN,
        P::ArrowLeft => GHOSTTY_KEY_ARROW_LEFT,
        P::ArrowRight => GHOSTTY_KEY_ARROW_RIGHT,
        P::ArrowUp => GHOSTTY_KEY_ARROW_UP,
        P::NumLock => GHOSTTY_KEY_NUM_LOCK,
        P::Numpad0 => numpad(0),
        P::Numpad1 => numpad(1),
        P::Numpad2 => numpad(2),
        P::Numpad3 => numpad(3),
        P::Numpad4 => numpad(4),
        P::Numpad5 => numpad(5),
        P::Numpad6 => numpad(6),
        P::Numpad7 => numpad(7),
        P::Numpad8 => numpad(8),
        P::Numpad9 => numpad(9),
        P::NumpadAdd => GHOSTTY_KEY_NUMPAD_ADD,
        P::NumpadBackspace => GHOSTTY_KEY_NUMPAD_BACKSPACE,
        P::NumpadClear => GHOSTTY_KEY_NUMPAD_CLEAR,
        P::NumpadClearEntry => GHOSTTY_KEY_NUMPAD_CLEAR_ENTRY,
        P::NumpadComma => GHOSTTY_KEY_NUMPAD_COMMA,
        P::NumpadDecimal => GHOSTTY_KEY_NUMPAD_DECIMAL,
        P::NumpadDivide => GHOSTTY_KEY_NUMPAD_DIVIDE,
        P::NumpadEnter => GHOSTTY_KEY_NUMPAD_ENTER,
        P::NumpadEqual => GHOSTTY_KEY_NUMPAD_EQUAL,
        P::NumpadMultiply | P::NumpadStar => GHOSTTY_KEY_NUMPAD_MULTIPLY,
        P::NumpadParenLeft => GHOSTTY_KEY_NUMPAD_PAREN_LEFT,
        P::NumpadParenRight => GHOSTTY_KEY_NUMPAD_PAREN_RIGHT,
        P::NumpadSubtract => GHOSTTY_KEY_NUMPAD_SUBTRACT,
        P::Escape => GHOSTTY_KEY_ESCAPE,
        P::Fn => GHOSTTY_KEY_FN,
        P::FnLock => GHOSTTY_KEY_FN_LOCK,
        P::PrintScreen => GHOSTTY_KEY_PRINT_SCREEN,
        P::ScrollLock => GHOSTTY_KEY_SCROLL_LOCK,
        P::Pause => GHOSTTY_KEY_PAUSE,
        P::F1 => GHOSTTY_KEY_F1,
        P::F2 => GHOSTTY_KEY_F2,
        P::F3 => GHOSTTY_KEY_F3,
        P::F4 => GHOSTTY_KEY_F4,
        P::F5 => GHOSTTY_KEY_F5,
        P::F6 => GHOSTTY_KEY_F6,
        P::F7 => GHOSTTY_KEY_F7,
        P::F8 => GHOSTTY_KEY_F8,
        P::F9 => GHOSTTY_KEY_F9,
        P::F10 => GHOSTTY_KEY_F10,
        P::F11 => GHOSTTY_KEY_F11,
        P::F12 => GHOSTTY_KEY_F12,
        P::F13 => GHOSTTY_KEY_F13,
        P::F14 => GHOSTTY_KEY_F14,
        P::F15 => GHOSTTY_KEY_F15,
        P::F16 => GHOSTTY_KEY_F16,
        P::F17 => GHOSTTY_KEY_F17,
        P::F18 => GHOSTTY_KEY_F18,
        P::F19 => GHOSTTY_KEY_F19,
        P::F20 => GHOSTTY_KEY_F20,
        P::F21 => GHOSTTY_KEY_F21,
        P::F22 => GHOSTTY_KEY_F22,
        P::F23 => GHOSTTY_KEY_F23,
        P::F24 => GHOSTTY_KEY_F24,
        P::F25 => GHOSTTY_KEY_F25,
        _ => GHOSTTY_KEY_UNIDENTIFIED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::Mods as RuneMods;
    use terminal::{Config, Vt};

    fn key(key: Key, mods: RuneMods) -> KeyEvent {
        KeyEvent { key, mods }
    }

    fn text(s: &str) -> KeyEvent {
        key(Key::Text(s.into()), RuneMods::default())
    }

    fn with(s: &str, mods: RuneMods) -> KeyEvent {
        key(Key::Text(s.into()), mods)
    }

    const NONE: RuneMods = RuneMods {
        shift: false,
        ctrl: false,
        alt: false,
        cmd: false,
        logo: false,
    };
    const CTRL: RuneMods = RuneMods { ctrl: true, ..NONE };
    const ALT: RuneMods = RuneMods { alt: true, ..NONE };
    const SHIFT: RuneMods = RuneMods {
        shift: true,
        ..NONE
    };

    /// What a key sends to a program, after `modes` (the program's output) on a new terminal.
    fn sent(modes: &[u8], ev: &KeyEvent, info: KeyInfo) -> Option<Vec<u8>> {
        let mut vt = Vt::new(&Config::default());
        vt.write(modes);
        encode(ev, info).map(|k| vt.encode_key(&k))
    }

    fn bytes(ev: &KeyEvent) -> Option<Vec<u8>> {
        sent(b"", ev, KeyInfo::default())
    }

    #[test]
    fn text_and_the_editing_keys_type() {
        assert_eq!(bytes(&text("a")), Some(b"a".to_vec()));
        assert_eq!(bytes(&with("A", SHIFT)), Some(b"A".to_vec()));
        assert_eq!(bytes(&text("é")), Some("é".as_bytes().to_vec()));
        assert_eq!(bytes(&text(" ")), Some(b" ".to_vec()));
        let plain = RuneMods::default();
        assert_eq!(bytes(&key(Key::Enter, plain)), Some(b"\r".to_vec()));
        assert_eq!(bytes(&key(Key::Backspace, plain)), Some(b"\x7f".to_vec()));
        assert_eq!(bytes(&key(Key::Tab, plain)), Some(b"\t".to_vec()));
        assert_eq!(bytes(&key(Key::Escape, plain)), Some(b"\x1b".to_vec()));
        assert_eq!(bytes(&key(Key::Up, plain)), Some(b"\x1b[A".to_vec()));
        assert_eq!(bytes(&key(Key::Delete, plain)), Some(b"\x1b[3~".to_vec()));
        assert_eq!(
            bytes(&key(Key::Function(5), plain)),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(
            bytes(&key(Key::Function(1), plain)),
            Some(b"\x1bOP".to_vec())
        );
    }

    #[test]
    fn ctrl_chords_send_their_control_characters() {
        assert_eq!(bytes(&with("c", CTRL)), Some(vec![0x03]));
        assert_eq!(bytes(&with("d", CTRL)), Some(vec![0x04]));
        // xterm's legacy aliases, which libghostty-vt leaves to fixterms.
        assert_eq!(bytes(&with("[", CTRL)), Some(vec![0x1b]));
        assert_eq!(bytes(&with("i", CTRL)), Some(b"\t".to_vec()));
        assert_eq!(bytes(&with("m", CTRL)), Some(b"\r".to_vec()));
        assert_eq!(bytes(&with("-", CTRL)), Some(vec![0x1f]));
        assert_eq!(bytes(&with("]", CTRL)), Some(vec![0x1d]));
        assert_eq!(bytes(&with("/", CTRL)), Some(vec![0x1f]));
        assert_eq!(bytes(&with(" ", CTRL)), Some(vec![0x00]));
        assert_eq!(bytes(&with("\\", CTRL)), Some(vec![0x1c]));
        // Modified named keys get xterm's modifier parameter.
        assert_eq!(bytes(&key(Key::Left, CTRL)), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(bytes(&key(Key::Up, SHIFT)), Some(b"\x1b[1;2A".to_vec()));
    }

    #[test]
    fn option_as_alt_sends_escape_and_the_key() {
        // Rune hands an Alt Option key over as the key it's on.
        let info = KeyInfo {
            base: Some('b'),
            ..KeyInfo::default()
        };
        assert_eq!(sent(b"", &with("b", ALT), info), Some(b"\x1bb".to_vec()));
        assert_eq!(bytes(&key(Key::Backspace, ALT)), Some(b"\x1b\x7f".to_vec()));
        assert_eq!(bytes(&key(Key::Right, ALT)), Some(b"\x1b[1;3C".to_vec()));
        // With Option typing the layout's characters, the character is the text.
        let layout = KeyInfo {
            base: Some('b'),
            option_as_alt: OptionAsAlt::No,
            ..KeyInfo::default()
        };
        let mut vt = Vt::new(&Config {
            option_as_alt: OptionAsAlt::No,
            ..Config::default()
        });
        let k = encode(&with("∫", ALT), layout).unwrap();
        assert_eq!(k.consumed_mods, Mods::ALT);
        assert_eq!(vt.encode_key(&k), "∫".as_bytes());
    }

    #[test]
    fn the_kitty_protocol_sees_the_physical_key_consumed_mods_and_releases() {
        // Shift used up making `:`: Neovim sees `:`, not shift+;.
        let colon = KeyInfo {
            physical: Some(PhysicalKeyCode::Semicolon),
            base: Some(';'),
            ..KeyInfo::default()
        };
        let k = encode(&with(":", SHIFT), colon).unwrap();
        assert_eq!(
            (k.key, k.consumed_mods),
            (GHOSTTY_KEY_SEMICOLON, Mods::SHIFT)
        );
        assert_eq!(
            sent(b"\x1b[>1u", &with(":", SHIFT), colon),
            Some(b":".to_vec())
        );
        // Every key as an escape code (flags 1|8): the key's code, with shift.
        assert_eq!(
            sent(b"\x1b[>11u", &with(":", SHIFT), colon),
            Some(b"\x1b[59;2u".to_vec())
        );
        // Ctrl+letter is unambiguous under flag 1.
        assert_eq!(
            sent(b"\x1b[>1u", &with("c", CTRL), KeyInfo::default()),
            Some(b"\x1b[99;5u".to_vec())
        );
        // Releases are heard only with the event types flag (2), and a text key's only when
        // every key is an escape code (8).
        let up = KeyInfo {
            physical: Some(PhysicalKeyCode::KeyA),
            base: Some('a'),
            action: KeyAction::Release,
            ..KeyInfo::default()
        };
        assert_eq!(sent(b"\x1b[>1u", &text("a"), up), Some(Vec::new()));
        assert_eq!(
            sent(b"\x1b[>11u", &text("a"), up),
            Some(b"\x1b[97;1:3u".to_vec())
        );
        let arrow_up = KeyInfo {
            physical: Some(PhysicalKeyCode::ArrowUp),
            ..up
        };
        assert_eq!(
            sent(b"\x1b[>3u", &key(Key::Up, RuneMods::default()), arrow_up),
            Some(b"\x1b[1;1:3A".to_vec())
        );
        let repeat = KeyInfo {
            action: KeyAction::Repeat,
            ..up
        };
        assert_eq!(
            sent(b"\x1b[>11u", &text("a"), repeat),
            Some(b"\x1b[97;1:2u".to_vec())
        );
        // Under the kitty protocol the aliases are the chord again.
        assert_eq!(
            sent(b"\x1b[>1u", &with("[", CTRL), KeyInfo::default()),
            Some(b"\x1b[91;5u".to_vec())
        );
        // A layout that types another letter on the key: the physical key is what's reported
        // with the text.
        let azerty = KeyInfo {
            physical: Some(PhysicalKeyCode::KeyQ),
            base: Some('a'),
            ..KeyInfo::default()
        };
        assert_eq!(encode(&text("a"), azerty).unwrap().key, GHOSTTY_KEY_Q);
    }

    #[test]
    fn a_composing_key_sends_nothing() {
        let info = KeyInfo {
            composing: true,
            ..KeyInfo::default()
        };
        assert_eq!(sent(b"", &text("a"), info), Some(Vec::new()));
    }

    #[test]
    fn cmd_chords_are_left_for_the_app() {
        let cmd = RuneMods {
            cmd: true,
            ..RuneMods::default()
        };
        assert_eq!(
            encode(&key(Key::Command('t'), cmd), KeyInfo::default()),
            None
        );
        assert_eq!(encode(&with("t", cmd), KeyInfo::default()), None);
        assert_eq!(encode(&key(Key::Backspace, cmd), KeyInfo::default()), None);
    }
}
