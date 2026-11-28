//! herdr-frontend's home: two folders of its own, never shared with Herdr app (`docs/plan.md`
//! "Decisions"). Config is what the person writes, and state is what the app writes, so a
//! dotfiles-managed config folder never changes under git (`docs/settings.md` "Config and state").
//!
//! The config folder ([`crate::control::config_home`]: `$XDG_CONFIG_HOME/herdr-frontend`, else
//! `~/.config/herdr-frontend`) is read live and never written, except that a missing `keys.json`
//! is created with Eli's keys:
//!
//! - `keys.json`: the key file ([`crate::keyfile`]).
//! - `settings.json`, `themes/`, `programs.json`, `icons/` and `program-icons/`: the look and
//!   preferences ([`crate::config`]).
//!
//! The state folder ([`crate::control::state_home`]: `$XDG_STATE_HOME/herdr-frontend`, else
//! `~/.local/state/herdr-frontend`) holds everything the app writes:
//!
//! - `view.json`: this screen's view ([`model::View::to_json`]): the space shown, each space's
//!   tab, each tab's focused pane, recents and seen attention.
//! - `window.json`: where the window was ([`crate::window::Placement`]).
//! - `zoom.json`: the zoom the zoom keys set ([`SavedZoom`]).
//! - `shared.json` and `agents.json`: the app-local shared state and the Agents list's times
//!   ([`crate::shared`], `docs/sidebar.md`), from the defaults on the first run.
//! - `keys.error`: why `keys.json` can't be used, while it can't.
//! - `herdr-frontend.sock`: the control socket ([`crate::control`]).
//!
//! `$HERDR_FRONTEND_HOME` names one folder that is both (tests, the e2e harness). Older versions
//! kept the state in the config folder: [`Home::from_env`] moves those files over once.
//!
//! Files are written whole to a temporary name and renamed into place, so a crash never leaves
//! half a file. A file that is a symlink is written through to its target, so the link stays.
//! Writes during a run go to a [`Saver`] thread, off the UI thread; the last writes on close and
//! quit are made at once, as the process may end right after.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::config::settings::{ZOOM_MAX, ZOOM_MIN};

pub const VIEW: &str = "view.json";
pub const WINDOW: &str = "window.json";
pub const ZOOM: &str = "zoom.json";

/// The state files older versions wrote into the config folder, which [`migrate`] moves.
pub const MOVED: [&str; 5] = [
    VIEW,
    WINDOW,
    crate::shared::SHARED,
    crate::shared::TIMES,
    crate::keyfile::KEY_ERRORS,
];

/// The config and state folders, and the thread that writes state.
#[derive(Clone)]
pub struct Home {
    config: PathBuf,
    state: PathBuf,
    saver: Saver,
}

impl Home {
    /// The folders the environment names ([`crate::control::config_home`] and
    /// [`crate::control::state_home`]), with any state an older version left in the config
    /// folder moved over first.
    pub fn from_env() -> Home {
        let home = Home::split(crate::control::config_home(), crate::control::state_home());
        migrate(&home.config, &home.state);
        home
    }

    /// One folder for config and state both: `$HERDR_FRONTEND_HOME`, tests.
    pub fn at(dir: impl Into<PathBuf>) -> Home {
        let dir = dir.into();
        Home::split(dir.clone(), dir)
    }

    pub fn split(config: impl Into<PathBuf>, state: impl Into<PathBuf>) -> Home {
        Home {
            config: config.into(),
            state: state.into(),
            saver: Saver::start(),
        }
    }

    /// The config folder: read, never written.
    pub fn config(&self) -> &Path {
        &self.config
    }

    /// The state folder: everything the app writes.
    pub fn state(&self) -> &Path {
        &self.state
    }

    /// A state file's text, or `None` when there is none or it can't be read.
    pub fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.state.join(name)).ok()
    }

    /// Writes a state file on the saver thread.
    pub fn save(&self, name: &str, text: String) {
        self.saver.save(self.state.join(name), text);
    }

    /// Writes a state file now, on this thread: on close and quit.
    pub fn save_now(&self, name: &str, text: String) {
        self.saver.save_now(self.state.join(name), text);
    }
}

/// `zoom.json`: the zoom the zoom keys set, and `settings.json`'s zoom when they set it. It holds
/// while `settings.json`'s zoom stays that value; an edited `settings.json` zoom wins, as the
/// starting value it is (`docs/settings.md` "Zoom keys").
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedZoom {
    pub zoom: f32,
    pub settings: f32,
}

impl SavedZoom {
    /// The saved zoom, kept in the zoom keys' range.
    pub fn from_json(text: &str) -> Option<SavedZoom> {
        let saved: SavedZoom = serde_json::from_str(text).ok()?;
        (saved.zoom.is_finite() && saved.settings.is_finite()).then(|| SavedZoom {
            zoom: saved.zoom.clamp(ZOOM_MIN, ZOOM_MAX),
            ..saved
        })
    }

    pub fn to_json(&self) -> String {
        // As the files say it: 1.1, not the f32's 1.100000023841858.
        let short = |v: f32| (f64::from(v) * 1e4).round() / 1e4;
        let value = serde_json::json!({"zoom": short(self.zoom), "settings": short(self.settings)});
        serde_json::to_string_pretty(&value).expect("numbers always serialize")
    }

    /// The zoom to draw at with `settings.json`'s zoom `settings`. The two settings zooms are
    /// compared to the four places [`SavedZoom::to_json`] keeps.
    pub fn applied(saved: Option<SavedZoom>, settings: f32) -> f32 {
        match saved {
            Some(saved) if (saved.settings - settings).abs() < 1e-4 => saved.zoom,
            _ => settings,
        }
    }
}

/// Moves the state files an older version wrote into `config` to `state`, and removes a control
/// socket left there that nobody answers on. A file `state` already has is newer, and the
/// config folder's copy is removed. Nothing happens when the two are one folder.
pub fn migrate(config: &Path, state: &Path) {
    if same_folder(config, state) {
        return;
    }
    for name in MOVED {
        let from = config.join(name);
        if std::fs::symlink_metadata(&from).is_err() {
            continue;
        }
        let to = state.join(name);
        let moved = if std::fs::symlink_metadata(&to).is_ok() {
            std::fs::remove_file(&from)
        } else {
            move_file(&from, &to)
        };
        if let Err(error) = moved {
            eprintln!(
                "herdr-frontend: couldn't move {} to {}: {error}",
                from.display(),
                state.display()
            );
        }
    }
    let socket = config.join(crate::control::SOCKET);
    if std::fs::symlink_metadata(&socket).is_ok()
        && std::os::unix::net::UnixStream::connect(&socket).is_err()
    {
        let _ = std::fs::remove_file(&socket);
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

/// Renames `from` to `to`, or copies and removes it when they're on different volumes.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

/// Writes files in the background, each one's newest text last.
#[derive(Clone)]
struct Saver {
    queue: Sender<(PathBuf, u64, String)>,
    /// Numbers each write in the order it was asked for. Taking one never waits on a write.
    next: Arc<AtomicU64>,
    /// The newest version written, per file: an older write that comes late is skipped.
    written: Arc<Mutex<HashMap<PathBuf, u64>>>,
}

impl Saver {
    fn start() -> Saver {
        let (queue, inbox) = mpsc::channel::<(PathBuf, u64, String)>();
        let written = Arc::new(Mutex::new(HashMap::new()));
        let shared = written.clone();
        std::thread::Builder::new()
            .name("herdr-frontend-saver".into())
            .spawn(move || {
                for (path, version, text) in inbox {
                    write(&shared, &path, version, &text);
                }
            })
            .expect("spawning the saver thread");
        Saver {
            queue,
            next: Arc::new(AtomicU64::new(0)),
            written,
        }
    }

    fn version(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn save(&self, path: PathBuf, text: String) {
        let version = self.version();
        let _ = self.queue.send((path, version, text));
    }

    fn save_now(&self, path: PathBuf, text: String) {
        let version = self.version();
        write(&self.written, &path, version, &text);
    }
}

/// Writes `text` to `path` unless a newer version is already there. The lock is held for the
/// write, so two writes of one file never interleave.
fn write(written: &Mutex<HashMap<PathBuf, u64>>, path: &Path, version: u64, text: &str) {
    let mut written = written.lock().unwrap_or_else(PoisonError::into_inner);
    if written.get(path).is_some_and(|&w| w >= version) {
        return;
    }
    if let Err(error) = write_whole(path, text) {
        eprintln!("herdr-frontend: couldn't save {}: {error}", path.display());
        return;
    }
    written.insert(path.to_owned(), version);
}

fn write_whole(path: &Path, text: &str) -> std::io::Result<()> {
    let target = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(path)?,
        _ => path.to_owned(),
    };
    let path = target.as_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("herdr-frontend-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_saved_file_reads_back_and_the_newest_write_wins() {
        let dir = scratch("saves");
        let home = Home::at(&dir);
        assert_eq!(home.read(VIEW), None);
        home.save_now(VIEW, "one".into());
        assert_eq!(home.read(VIEW).as_deref(), Some("one"));

        // A write queued before a later one made at once never lands after it.
        let stale = home.saver.version();
        home.save_now(VIEW, "two".into());
        write(&home.saver.written, &dir.join(VIEW), stale, "stale");
        assert_eq!(home.read(VIEW).as_deref(), Some("two"));

        // The thread writes in the background.
        home.save(WINDOW, "{}".into());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while home.read(WINDOW).is_none() {
            assert!(std::time::Instant::now() < deadline, "never written");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!dir.join("window.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlinked_file_is_written_through_to_its_target() {
        let dir = scratch("symlink");
        std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
        std::fs::write(dir.join("elsewhere/window.json"), "{}").unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere/window.json"), dir.join(WINDOW)).unwrap();
        let home = Home::at(&dir);
        home.save_now(WINDOW, "{\"size\": [1, 1]}".into());
        assert!(
            std::fs::symlink_metadata(dir.join(WINDOW))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("elsewhere/window.json")).unwrap(),
            "{\"size\": [1, 1]}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_is_read_and_written_in_the_state_folder_only() {
        let dir = scratch("split");
        let home = Home::split(dir.join("config"), dir.join("state"));
        home.save_now(VIEW, "{}".into());
        assert_eq!(home.read(VIEW).as_deref(), Some("{}"));
        assert!(dir.join("state").join(VIEW).exists());
        assert!(!dir.join("config").exists(), "the config folder is never written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_versions_state_moves_out_of_the_config_folder() {
        let dir = scratch("migrate");
        let (config, state) = (dir.join("config"), dir.join("state"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        for name in MOVED {
            std::fs::write(config.join(name), format!("old {name}")).unwrap();
        }
        // The state folder's own copy is newer: it stays, and the old one goes.
        std::fs::write(state.join(WINDOW), "newer").unwrap();
        for name in ["settings.json", "keys.json"] {
            std::fs::write(config.join(name), "{}").unwrap();
        }
        // A socket a window that died left behind.
        drop(std::os::unix::net::UnixListener::bind(config.join(crate::control::SOCKET)).unwrap());

        migrate(&config, &state);
        for name in MOVED {
            assert!(!config.join(name).exists(), "{name} left in config");
            let want = if name == WINDOW { "newer".to_owned() } else { format!("old {name}") };
            assert_eq!(std::fs::read_to_string(state.join(name)).unwrap(), want);
        }
        assert!(!config.join(crate::control::SOCKET).exists());
        for name in ["settings.json", "keys.json"] {
            assert_eq!(std::fs::read_to_string(config.join(name)).unwrap(), "{}");
            assert!(!state.join(name).exists());
        }

        // One folder for both moves nothing.
        std::fs::write(state.join(VIEW), "kept").unwrap();
        migrate(&state, &state);
        assert_eq!(std::fs::read_to_string(state.join(VIEW)).unwrap(), "kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_zoom_holds_until_the_settings_zoom_changes() {
        let saved = SavedZoom {
            zoom: 1.3,
            settings: 1.1,
        };
        assert_eq!(SavedZoom::from_json(&saved.to_json()), Some(saved));
        assert!(saved.to_json().contains("1.3"), "{}", saved.to_json());
        assert_eq!(SavedZoom::from_json("{}"), None);
        assert_eq!(SavedZoom::applied(Some(saved), 1.1), 1.3);
        assert_eq!(SavedZoom::applied(Some(saved), 0.9), 0.9);
        assert_eq!(SavedZoom::applied(None, 0.9), 0.9);
        // Four places survive a save, and a saved zoom out of range comes back in it.
        let odd = SavedZoom {
            zoom: 1.3,
            settings: 1.23456,
        };
        let back = SavedZoom::from_json(&odd.to_json());
        assert_eq!(SavedZoom::applied(back, 1.23456), 1.3);
        let big = SavedZoom::from_json(r#"{"zoom": 9, "settings": 1.1}"#).unwrap();
        assert_eq!(big.zoom, ZOOM_MAX);
    }
}
