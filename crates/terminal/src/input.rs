//! Input to a terminal's program: key, mouse, paste and focus events, and what they encode to.
//!
//! Keys, mouse events and pastes encode through libghostty-vt with the terminal's modes as they
//! are at that moment ([`crate::Vt::encode_key`], [`crate::Vt::encode_mouse`],
//! [`crate::Vt::paste`], or the same requests sent to a [`crate::Terminal`]'s worker). Focus has
//! no modes to follow, so [`encode_focus`] is a plain function.

use ghostty_vt_sys::*;

/// A physical key, as libghostty-vt names it (`GHOSTTY_KEY_A`, `GHOSTTY_KEY_ENTER`, ...). It's
/// the key's position on a US layout, whatever the user's layout prints on it.
pub type Key = GhosttyKey;

/// Modifier keys held during an event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods(pub GhosttyMods);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(GHOSTTY_MODS_SHIFT as GhosttyMods);
    pub const CTRL: Mods = Mods(GHOSTTY_MODS_CTRL as GhosttyMods);
    pub const ALT: Mods = Mods(GHOSTTY_MODS_ALT as GhosttyMods);
    /// Cmd on macOS.
    pub const SUPER: Mods = Mods(GHOSTTY_MODS_SUPER as GhosttyMods);
    pub const CAPS_LOCK: Mods = Mods(GHOSTTY_MODS_CAPS_LOCK as GhosttyMods);
    pub const NUM_LOCK: Mods = Mods(GHOSTTY_MODS_NUM_LOCK as GhosttyMods);

    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;

    fn bitor(self, rhs: Mods) -> Mods {
        Mods(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyAction {
    #[default]
    Press,
    Repeat,
    /// Programs only hear releases under the kitty keyboard protocol's event types flag.
    Release,
}

/// One key event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyEvent {
    pub action: KeyAction,
    pub key: Key,
    pub mods: Mods,
    /// The modifiers the layout used up to produce `text`: Shift for `:`, Option for `ß`.
    /// Without them the kitty keyboard protocol reports shift+; and Neovim never sees `:`.
    pub consumed_mods: Mods,
    /// The text the key produced, if any.
    pub text: Option<String>,
    /// The key's character with no modifiers held (`;` for the `:` key on a US layout).
    pub unshifted: Option<char>,
    /// The key is part of an IME composition.
    pub composing: bool,
}

impl KeyEvent {
    /// A press of `key` that typed `text`, with no modifiers.
    pub fn text(key: Key, text: &str) -> KeyEvent {
        KeyEvent {
            key,
            text: Some(text.to_owned()),
            unshifted: text.chars().next(),
            ..KeyEvent::default()
        }
    }

    /// A press of `key` with `mods` that typed no text.
    pub fn press(key: Key, mods: Mods) -> KeyEvent {
        KeyEvent {
            key,
            mods,
            ..KeyEvent::default()
        }
    }
}

/// Which Option keys act as Alt on macOS. The others type their layout's characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OptionAsAlt {
    /// Both Option keys type characters.
    No,
    /// Both Option keys are Alt, as Eli's Ghostty sets (`macos-option-as-alt = true`).
    #[default]
    Both,
    Left,
    Right,
}

impl OptionAsAlt {
    pub(crate) fn raw(self) -> GhosttyOptionAsAlt {
        match self {
            OptionAsAlt::No => GHOSTTY_OPTION_AS_ALT_FALSE,
            OptionAsAlt::Both => GHOSTTY_OPTION_AS_ALT_TRUE,
            OptionAsAlt::Left => GHOSTTY_OPTION_AS_ALT_LEFT,
            OptionAsAlt::Right => GHOSTTY_OPTION_AS_ALT_RIGHT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    Back,
    Forward,
}

impl MouseButton {
    pub(crate) fn raw(self) -> GhosttyMouseButton {
        match self {
            MouseButton::Left => GHOSTTY_MOUSE_BUTTON_LEFT,
            MouseButton::Middle => GHOSTTY_MOUSE_BUTTON_MIDDLE,
            MouseButton::Right => GHOSTTY_MOUSE_BUTTON_RIGHT,
            MouseButton::WheelUp => GHOSTTY_MOUSE_BUTTON_FOUR,
            MouseButton::WheelDown => GHOSTTY_MOUSE_BUTTON_FIVE,
            MouseButton::WheelLeft => GHOSTTY_MOUSE_BUTTON_SIX,
            MouseButton::WheelRight => GHOSTTY_MOUSE_BUTTON_SEVEN,
            MouseButton::Back => GHOSTTY_MOUSE_BUTTON_EIGHT,
            MouseButton::Forward => GHOSTTY_MOUSE_BUTTON_NINE,
        }
    }
}

/// One mouse event over the terminal's grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseEvent {
    pub action: MouseAction,
    /// The button pressed or released; `None` for plain motion.
    pub button: Option<MouseButton>,
    pub mods: Mods,
    /// The pointer in device pixels from the grid's top-left corner, in the cell size the
    /// terminal was last resized with.
    pub x: f32,
    pub y: f32,
    /// Some button is held (motion while dragging).
    pub any_button_pressed: bool,
}

/// A paste was refused because the text could run commands (a newline outside bracketed paste,
/// or the bracketed paste terminator). Ask the user, then paste again with `allow_unsafe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteRejected {
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Gained,
    Lost,
}

/// `CSI I` or `CSI O`, for a program that turned on focus reporting (mode 1004,
/// [`crate::Modes::focus_reporting`]).
///
/// Never write these for a herdr-core terminal: send the core a FOCUS frame instead. The core
/// writes the sequence itself when the program asked for it, so encoding it here too would
/// report every focus change twice (`docs/research/terminal-rendering.md` §3).
pub fn encode_focus(focus: Focus) -> Vec<u8> {
    let event = match focus {
        Focus::Gained => GHOSTTY_FOCUS_GAINED,
        Focus::Lost => GHOSTTY_FOCUS_LOST,
    };
    let mut buf = [0u8; 8];
    let mut written = 0usize;
    // SAFETY: `buf` is writable for its length and `written` is a valid out pointer.
    let result = unsafe { ghostty_focus_encode(event, buf.as_mut_ptr(), buf.len(), &mut written) };
    assert_eq!(result, GHOSTTY_SUCCESS, "ghostty_focus_encode");
    buf[..written].to_vec()
}
