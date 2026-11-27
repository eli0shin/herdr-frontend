//! Plain typing into the focused pane: printable text, Return, Backspace, Tab, Escape, the
//! arrows and the editing keys, and Ctrl with a letter. The terminal's worker encodes them for
//! the program's modes (application cursor keys, the kitty keyboard protocol), so an arrow
//! sends what Neovim asked for.
//!
//! Everything else is left for the app: chords with Cmd, and Option or Ctrl with anything but
//! a letter. Full input (the physical key, consumed modifiers, releases, IME, the mouse, paste)
//! is ticket 015's. Keys reach here only after the keymap passed on them (`docs/keys.md`).

use ghostty_vt_sys::*;
use rune::{Key, KeyEvent};
use terminal::{KeyEvent as TermKey, Mods};

/// What `ev` types into a terminal, or `None` for a key the terminal doesn't take here.
pub fn encode(ev: &KeyEvent) -> Option<TermKey> {
    let m = ev.mods;
    if m.cmd || m.logo || m.alt {
        return None;
    }
    let mut mods = Mods::NONE;
    if m.shift {
        mods = mods | Mods::SHIFT;
    }
    let named = |key| Some(TermKey::press(key, mods));
    match &ev.key {
        Key::Text(text) if m.ctrl => {
            let letter = single_letter(text)?;
            Some(TermKey {
                unshifted: Some(letter),
                ..TermKey::press(
                    GHOSTTY_KEY_A + (letter as u32 - 'a' as u32),
                    mods | Mods::CTRL,
                )
            })
        }
        Key::Text(text) if !text.is_empty() => Some(TermKey {
            mods,
            consumed_mods: mods,
            ..TermKey::text(text_key(text), text)
        }),
        _ if m.ctrl => None,
        Key::Enter => named(GHOSTTY_KEY_ENTER),
        Key::Backspace => named(GHOSTTY_KEY_BACKSPACE),
        Key::Tab => named(GHOSTTY_KEY_TAB),
        Key::Escape => named(GHOSTTY_KEY_ESCAPE),
        Key::Delete => named(GHOSTTY_KEY_DELETE),
        Key::Left => named(GHOSTTY_KEY_ARROW_LEFT),
        Key::Right => named(GHOSTTY_KEY_ARROW_RIGHT),
        Key::Up => named(GHOSTTY_KEY_ARROW_UP),
        Key::Down => named(GHOSTTY_KEY_ARROW_DOWN),
        Key::Home => named(GHOSTTY_KEY_HOME),
        Key::End => named(GHOSTTY_KEY_END),
        Key::PageUp => named(GHOSTTY_KEY_PAGE_UP),
        Key::PageDown => named(GHOSTTY_KEY_PAGE_DOWN),
        _ => None,
    }
}

/// The lowercase ASCII letter `text` is, if it is one.
fn single_letter(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let c = chars.next()?.to_ascii_lowercase();
    (chars.next().is_none() && c.is_ascii_lowercase()).then_some(c)
}

/// The key a typed character sits on, where it's a letter, a digit or the space bar; the
/// encoder needs it only under the kitty keyboard protocol, and text it can't place still
/// types.
fn text_key(text: &str) -> terminal::Key {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return GHOSTTY_KEY_UNIDENTIFIED;
    };
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => GHOSTTY_KEY_A + (c as u32 - 'a' as u32),
        '0' => GHOSTTY_KEY_DIGIT_0,
        c @ '1'..='9' => GHOSTTY_KEY_DIGIT_1 + (c as u32 - '1' as u32),
        ' ' => GHOSTTY_KEY_SPACE,
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

    fn ctrl(s: &str) -> KeyEvent {
        key(
            Key::Text(s.into()),
            RuneMods {
                ctrl: true,
                ..RuneMods::default()
            },
        )
    }

    /// What a key sends to a program in the terminal's default modes.
    fn bytes(ev: &KeyEvent) -> Option<Vec<u8>> {
        let mut vt = Vt::new(&Config::default());
        encode(ev).map(|k| vt.encode_key(&k))
    }

    #[test]
    fn text_and_the_editing_keys_type() {
        assert_eq!(bytes(&text("a")), Some(b"a".to_vec()));
        assert_eq!(bytes(&text("A")), Some(b"A".to_vec()));
        assert_eq!(bytes(&text("é")), Some("é".as_bytes().to_vec()));
        assert_eq!(bytes(&text(" ")), Some(b" ".to_vec()));
        let plain = RuneMods::default();
        assert_eq!(bytes(&key(Key::Enter, plain)), Some(b"\r".to_vec()));
        assert_eq!(bytes(&key(Key::Backspace, plain)), Some(b"\x7f".to_vec()));
        assert_eq!(bytes(&key(Key::Tab, plain)), Some(b"\t".to_vec()));
        assert_eq!(bytes(&key(Key::Escape, plain)), Some(b"\x1b".to_vec()));
        assert_eq!(bytes(&key(Key::Up, plain)), Some(b"\x1b[A".to_vec()));
    }

    #[test]
    fn ctrl_with_a_letter_sends_its_control_character() {
        assert_eq!(bytes(&ctrl("c")), Some(vec![0x03]));
        assert_eq!(bytes(&ctrl("d")), Some(vec![0x04]));
    }

    #[test]
    fn app_chords_are_left_alone() {
        let cmd = RuneMods {
            cmd: true,
            ..RuneMods::default()
        };
        assert_eq!(encode(&key(Key::Command('t'), cmd)), None);
        assert_eq!(encode(&key(Key::Text("t".into()), cmd)), None);
        let alt = RuneMods {
            alt: true,
            ..RuneMods::default()
        };
        assert_eq!(encode(&key(Key::Left, alt)), None);
        assert_eq!(encode(&ctrl(";")), None);
        let ctrl_only = RuneMods {
            ctrl: true,
            ..RuneMods::default()
        };
        assert_eq!(encode(&key(Key::Left, ctrl_only)), None);
    }
}
