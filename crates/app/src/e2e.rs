//! What the end-to-end harness (`docs/e2e.md`) asks of a real window, beyond `ls | do | status`:
//! keys sent to the app itself, the element tree it last drew, and its window number for
//! `screencapture -l`.
//!
//! A window serves these only when it starts with `HERDR_FRONTEND_E2E=1` ([`enabled`]); any other
//! window answers them as unknown requests. They come in on the control socket
//! (`crate::control`), one JSON line each way:
//!
//! - `{"keys": "ctrl-g s"}`: chords in `keys.json` syntax, pressed in order.
//! - `{"type": "echo hi\n"}`: text typed a character at a time (`\n` is Return).
//! - `{"tree": true}`: the element tree of the last frame the window drew ([`Tap`]).
//! - `{"window": true}`: `{"window": <CGWindowID or null>, "pid": <pid>}`.
//!
//! Keys go through `Tx::send_key`, so they reach the window as typed keys do: the focused
//! element's handlers, then `AppSpec::on_key`. Nothing is sent to the OS.

use std::cell::RefCell;
use std::rc::Rc;

use model::{Chord, Key as ModelKey};
use rune::{ElementInfo, Key, KeyEvent, Mods, Runtime};
use serde_json::{Map, Value, json};

/// Turns the harness requests on.
pub const VAR: &str = "HERDR_FRONTEND_E2E";

/// The requests only a harness window serves.
pub const REQUESTS: [&str; 4] = ["keys", "type", "tree", "window"];

/// Whether this process was started by the harness: `HERDR_FRONTEND_E2E=1`.
pub fn enabled() -> bool {
    std::env::var(VAR).is_ok_and(|v| v == "1")
}

/// Whether `cmd` is one of the harness requests.
pub fn is_request(cmd: &Value) -> bool {
    REQUESTS.iter().any(|name| cmd.get(name).is_some())
}

/// The key events a `keys` or `type` request asks for.
pub fn key_events(cmd: &Value) -> Result<Vec<KeyEvent>, String> {
    if let Some(keys) = cmd.get("keys") {
        let keys = keys
            .as_str()
            .ok_or("keys: expected a string like \"ctrl-g s\"")?;
        let chords = Chord::parse_sequence(keys).map_err(|e| format!("keys: {e}"))?;
        return Ok(chords.iter().map(key_event).collect());
    }
    if let Some(text) = cmd.get("type") {
        let text = text.as_str().ok_or("type: expected a string")?;
        return Ok(text.chars().map(typed).collect());
    }
    Err("expected keys or type".into())
}

/// One chord as the window would get it on macOS: Cmd chords as `Key::Command` with Cmd and the
/// logo key held, Ctrl and Alt with the character, Shift with the shifted letter.
pub fn key_event(chord: &Chord) -> KeyEvent {
    let m = chord.mods;
    let mods = Mods {
        shift: m.shift,
        cmd: m.cmd,
        alt: m.alt,
        ctrl: m.ctrl,
        logo: m.cmd,
    };
    let key = match chord.key {
        ModelKey::Char(c) if m.cmd => Key::Command(c),
        ModelKey::Char(c) if m.shift => Key::Text(c.to_uppercase().collect()),
        ModelKey::Char(c) => Key::Text(c.to_string()),
        ModelKey::Space if m.cmd => Key::Command(' '),
        ModelKey::Space => Key::Text(" ".into()),
        ModelKey::Enter => Key::Enter,
        ModelKey::Escape => Key::Escape,
        ModelKey::Tab => Key::Tab,
        ModelKey::Backspace => Key::Backspace,
        ModelKey::Delete => Key::Delete,
        ModelKey::Left => Key::Left,
        ModelKey::Right => Key::Right,
        ModelKey::Up => Key::Up,
        ModelKey::Down => Key::Down,
        ModelKey::Home => Key::Home,
        ModelKey::End => Key::End,
        ModelKey::PageUp => Key::PageUp,
        ModelKey::PageDown => Key::PageDown,
        ModelKey::F(n) => Key::Function(n),
    };
    KeyEvent { key, mods }
}

/// One typed character: Return for a newline, Tab for a tab, else the text itself.
fn typed(c: char) -> KeyEvent {
    let key = match c {
        '\n' | '\r' => Key::Enter,
        '\t' => Key::Tab,
        c => Key::Text(c.to_string()),
    };
    KeyEvent {
        key,
        mods: Mods::default(),
    }
}

/// The element tree of the last frame the window drew, kept by `App::on_frame_drawn`. Only
/// windows draw frames, so a headless run never has one.
#[derive(Clone, Default)]
pub struct Tap(Rc<RefCell<Option<Value>>>);

impl Tap {
    /// Keeps the tree of the frame `rt` just presented.
    pub fn record<S>(&self, rt: &Runtime<S>) {
        let elements: Vec<Value> = rt.elements().iter().map(element).collect();
        let mut tap = self.0.borrow_mut();
        let frame = tap
            .as_ref()
            .and_then(|t| t["frame"].as_u64())
            .map_or(1, |n| n + 1);
        *tap = Some(json!({
            "frame": frame,
            "viewport": rt.window_viewport(),
            "scale": rt.window_scale(),
            "focused": rt.presented_focus_key(),
            "elements": elements,
        }));
    }

    /// The last drawn frame's tree, or `None` before the first frame.
    pub fn tree(&self) -> Option<Value> {
        self.0.borrow().clone()
    }
}

/// One element as `--tree` writes it: its path, key, role, text, place and colors.
fn element(e: &ElementInfo) -> Value {
    let r2 = |v: f32| (f64::from(v) * 100.0).round() / 100.0;
    let mut v = Map::new();
    v.insert("path".into(), e.path.clone().into());
    if let Some(key) = &e.key {
        v.insert("key".into(), key.clone().into());
    }
    v.insert("role".into(), e.role.into());
    if let Some(label) = &e.label {
        v.insert("label".into(), label.clone().into());
    }
    if let Some(text) = &e.text {
        v.insert("text".into(), text.clone().into());
    }
    v.insert("rect".into(), json!(e.rect.map(r2)));
    if e.opacity < 1.0 {
        v.insert("opacity".into(), json!(r2(e.opacity)));
    }
    v.insert("fill".into(), e.fill.hex().into());
    if let Some(c) = e.text_color {
        v.insert("color".into(), c.hex().into());
    }
    if let Some(c) = e.border_color {
        v.insert("borderColor".into(), c.hex().into());
    }
    if let Some(style) = &e.text_style {
        v.insert(
            "textStyle".into(),
            json!({"family": style.family, "size": r2(style.size), "weight": r2(style.weight)}),
        );
    }
    if e.focusable {
        v.insert("focusable".into(), true.into());
    }
    if e.focused {
        v.insert("focused".into(), true.into());
    }
    Value::Object(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(text: &str) -> Vec<KeyEvent> {
        key_events(&json!({ "keys": text })).unwrap()
    }

    #[test]
    fn chords_become_the_keys_macos_sends() {
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        assert_eq!(
            keys("ctrl-g s"),
            vec![
                KeyEvent {
                    key: Key::Text("g".into()),
                    mods: ctrl
                },
                KeyEvent {
                    key: Key::Text("s".into()),
                    mods: Mods::default()
                },
            ]
        );
        let cmd_shift = Mods {
            cmd: true,
            logo: true,
            shift: true,
            ..Mods::default()
        };
        assert_eq!(
            keys("cmd-shift-]"),
            vec![KeyEvent {
                key: Key::Command(']'),
                mods: cmd_shift
            }]
        );
        assert_eq!(keys("shift-q")[0].key, Key::Text("Q".into()));
        assert_eq!(keys("f2")[0].key, Key::Function(2));
        assert_eq!(keys("enter escape")[1].key, Key::Escape);
    }

    #[test]
    fn typed_text_is_one_key_a_character() {
        let typed = key_events(&json!({"type": "hi\n"})).unwrap();
        let names: Vec<Key> = typed.iter().map(|k| k.key.clone()).collect();
        assert_eq!(
            names,
            vec![Key::Text("h".into()), Key::Text("i".into()), Key::Enter]
        );
    }

    #[test]
    fn bad_requests_say_why() {
        assert!(
            key_events(&json!({"keys": "hyper-x"}))
                .unwrap_err()
                .starts_with("keys: ")
        );
        assert!(key_events(&json!({"keys": 3})).is_err());
        assert!(key_events(&json!({"tree": true})).is_err());
        assert!(is_request(&json!({"tree": true})));
        assert!(!is_request(&json!({"ls": true})));
    }
}
