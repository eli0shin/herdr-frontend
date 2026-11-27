//! A Rune key event as the keymap reads it: a [`model::Press`] in Eli's key syntax.
//!
//! A character key is named by what it types with no modifiers (`Tx::key_base`), so ⇧⌘] is
//! `cmd-shift-]` on every layout and ⌥H (which types `˙`) is `alt-h`. When Shift made the key
//! type another character, that spelling comes too (`cmd-}`, `!`), as Herdr app accepts both.
//! With no base (a headless key, a script without `"code"`), a shifted US symbol stands for its
//! key (`!` is `shift-1`).
//!
//! macOS only: `cmd` is ⌘ and `ctrl` the Control key, as Rune reports them there.

use model::{Chord, Key as Named, Mods, Press};
use rune::{Key, KeyEvent};

/// The press `ev` is, with `base` the character its key types unmodified. `None` for keys no
/// binding can name (modifiers alone, media keys).
pub fn press(ev: &KeyEvent, base: Option<char>) -> Option<Press> {
    let m = ev.mods;
    let mods = Mods {
        cmd: m.cmd,
        ctrl: m.ctrl,
        alt: m.alt,
        shift: m.shift,
    };
    let named = |key| Some(Press::new(Chord::new(key, mods)));
    match &ev.key {
        Key::Text(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => character(c, base, mods),
                // More than one character (a composed sequence) is text, not a key.
                _ => None,
            }
        }
        Key::Command(c) => character(*c, base, mods),
        Key::Enter => named(Named::Enter),
        Key::Escape => named(Named::Escape),
        Key::Tab => named(Named::Tab),
        Key::Backspace => named(Named::Backspace),
        Key::Delete => named(Named::Delete),
        Key::Left => named(Named::Left),
        Key::Right => named(Named::Right),
        Key::Up => named(Named::Up),
        Key::Down => named(Named::Down),
        Key::Home => named(Named::Home),
        Key::End => named(Named::End),
        Key::PageUp => named(Named::PageUp),
        Key::PageDown => named(Named::PageDown),
        Key::Function(n) if (1..=24).contains(n) => named(Named::F(*n)),
        _ => None,
    }
}

fn character(c: char, base: Option<char>, mods: Mods) -> Option<Press> {
    let typed = control_letter(c).unwrap_or_else(|| lower(c));
    let base = base
        .filter(|b| !b.is_control())
        .map(lower)
        .or_else(|| mods.shift.then(|| us_unshifted(typed)).flatten())
        .unwrap_or(typed);
    if base.is_control() {
        return None;
    }
    let key = |c: char| {
        if c == ' ' {
            Named::Space
        } else {
            Named::Char(c)
        }
    };
    let chord = Chord::new(key(base), mods);
    // The shifted spelling: `!` for Shift+1. Letters only change case, and Option's characters
    // are the layout's, not a spelling of the key.
    let spelled = mods.shift && !mods.alt && typed != base && !typed.is_control();
    let typed = spelled.then(|| {
        Chord::new(
            key(typed),
            Mods {
                shift: false,
                ..mods
            },
        )
    });
    Some(Press { chord, typed })
}

/// The letter a control character (Ctrl+A is `\x01`) was typed with.
fn control_letter(c: char) -> Option<char> {
    let code = c as u32;
    (1..=26)
        .contains(&code)
        .then(|| char::from(b'a' + code as u8 - 1))
}

/// `c` lowercased, when that is one character.
fn lower(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// The key that types `c` with Shift on a US layout.
fn us_unshifted(c: char) -> Option<char> {
    let (shifted, plain) = ("~!@#$%^&*()_+{}|:\"<>?", "`1234567890-=[]\\;',./");
    shifted
        .chars()
        .position(|s| s == c)
        .and_then(|i| plain.chars().nth(i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::Mods as RuneMods;

    fn ev(key: Key, cmd: bool, ctrl: bool, alt: bool, shift: bool) -> KeyEvent {
        KeyEvent {
            key,
            mods: RuneMods {
                cmd,
                ctrl,
                alt,
                shift,
                logo: cmd,
            },
        }
    }

    fn spell(p: Option<Press>) -> (String, Option<String>) {
        let p = p.expect("a press");
        (p.chord.to_string(), p.typed.map(|t| t.to_string()))
    }

    fn one(text: &str) -> (String, Option<String>) {
        (text.to_owned(), None)
    }

    fn two(chord: &str, typed: &str) -> (String, Option<String>) {
        (chord.to_owned(), Some(typed.to_owned()))
    }

    #[test]
    fn command_and_control_chords_read_in_elis_syntax() {
        let cmd = |c| ev(Key::Command(c), true, false, false, false);
        assert_eq!(spell(press(&cmd('t'), Some('t'))), one("cmd-t"));
        assert_eq!(spell(press(&cmd('/'), None)), one("cmd-/"));
        let ctrl_g = ev(Key::Text("g".into()), false, true, false, false);
        assert_eq!(spell(press(&ctrl_g, Some('g'))), one("ctrl-g"));
        // A control character still names its letter.
        let ctrl_s = ev(Key::Text("\u{13}".into()), false, true, false, false);
        assert_eq!(spell(press(&ctrl_s, None)), one("ctrl-s"));
        let ctrl_semi = ev(Key::Text(";".into()), false, true, false, false);
        assert_eq!(spell(press(&ctrl_semi, Some(';'))), one("ctrl-;"));
    }

    #[test]
    fn shifted_keys_come_with_both_spellings() {
        // ⇧⌘] arrives as Command('}'), its key's base `]`.
        let tab_next = ev(Key::Command('}'), true, false, false, true);
        assert_eq!(
            spell(press(&tab_next, Some(']'))),
            two("cmd-shift-]", "cmd-}")
        );
        // Without a base, the US layout's key stands in.
        assert_eq!(spell(press(&tab_next, None)), two("cmd-shift-]", "cmd-}"));
        let bang = ev(Key::Text("!".into()), false, false, false, true);
        assert_eq!(spell(press(&bang, Some('1'))), two("shift-1", "!"));
        assert_eq!(spell(press(&bang, None)), two("shift-1", "!"));
        // ⌘+ is ⇧⌘= on a US layout.
        let plus = ev(Key::Command('+'), true, false, false, true);
        assert_eq!(spell(press(&plus, Some('='))), two("cmd-shift-=", "cmd-+"));
        // Letters only change case.
        let q = ev(Key::Text("Q".into()), false, false, false, true);
        assert_eq!(spell(press(&q, Some('q'))), one("shift-q"));
        assert_eq!(spell(press(&q, None)), one("shift-q"));
    }

    #[test]
    fn option_keys_are_named_by_their_key_not_the_character_they_type() {
        // ⌥⇧H types `Ó` on a US Mac layout.
        let resize = ev(Key::Text("Ó".into()), false, false, true, true);
        assert_eq!(spell(press(&resize, Some('h'))), one("alt-shift-h"));
        let word = ev(Key::Left, false, false, true, false);
        assert_eq!(spell(press(&word, None)), one("alt-left"));
    }

    #[test]
    fn named_keys_and_space() {
        let plain = |key| ev(key, false, false, false, false);
        assert_eq!(spell(press(&plain(Key::Escape), None)), one("escape"));
        assert_eq!(spell(press(&plain(Key::Function(2)), None)), one("f2"));
        assert_eq!(
            spell(press(
                &ev(Key::Function(2), false, false, false, true),
                None
            )),
            one("shift-f2")
        );
        assert_eq!(
            spell(press(&plain(Key::Text(" ".into())), None)),
            one("space")
        );
        assert_eq!(
            spell(press(&ev(Key::Tab, false, true, false, true), None)),
            one("ctrl-shift-tab")
        );
        assert_eq!(
            spell(press(&ev(Key::Escape, false, true, true, true), None)),
            one("ctrl-alt-shift-escape")
        );
        assert_eq!(press(&plain(Key::Text("ab".into())), None), None);
        assert_eq!(press(&plain(Key::Function(30)), None), None);
    }
}
