//! herdr-frontend's home: the folder its own files live in, never shared with Herdr app
//! (`docs/plan.md` "Decisions"). `$HERDR_FRONTEND_HOME`, else `$XDG_CONFIG_HOME/herdr-frontend`,
//! else `~/.config/herdr-frontend`, as Herdr app finds `~/.config/herdr-app`.
//!
//! - `keys.json`: the key file, read and reloaded live by [`crate::keyfile`].
//! - `view.json`: this screen's view ([`model::View::to_json`]): the space shown, each space's
//!   tab, each tab's focused pane, recents and seen attention.
//! - `window.json`: where the window was ([`crate::window::Placement`]).
//! - `shared.json` and `agents.json`: the app-local shared state and the Agents list's times
//!   ([`crate::shared`], `docs/sidebar.md`), imported from Herdr app's home on the first run.
//! - `settings.json`, `themes/`, `programs.json`, `icons/`, `program-icons/` and `fonts/`: the
//!   look and preferences ([`crate::config`], `docs/settings.md`). Only the zoom keys write here,
//!   and only `settings.json`'s `zoom`.
//!
//! Files are written whole to a temporary name and renamed into place, so a crash never leaves
//! half a file. A file that is a symlink (a dotfiles-managed `settings.json`) is written through
//! to its target, so the link stays. Writes during a run go to a [`Saver`] thread, off the UI thread; the last writes
//! on close and quit are made at once, as the process may end right after.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};

pub const VIEW: &str = "view.json";
pub const WINDOW: &str = "window.json";

/// The home folder, and the thread that writes into it.
#[derive(Clone)]
pub struct Home {
    dir: PathBuf,
    saver: Saver,
}

impl Home {
    /// The home the environment names ([`crate::control::home`], where the control socket
    /// lives too).
    pub fn from_env() -> Home {
        Home::at(crate::control::home())
    }

    pub fn at(dir: impl Into<PathBuf>) -> Home {
        Home {
            dir: dir.into(),
            saver: Saver::start(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A file's text, or `None` when there is none or it can't be read.
    pub fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(name)).ok()
    }

    /// Writes a file on the saver thread.
    pub fn save(&self, name: &str, text: String) {
        self.saver.save(self.dir.join(name), text);
    }

    /// Writes a file now, on this thread: on close and quit.
    pub fn save_now(&self, name: &str, text: String) {
        self.saver.save_now(self.dir.join(name), text);
    }
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
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        std::fs::write(dir.join("dotfiles/settings.json"), "{}").unwrap();
        std::os::unix::fs::symlink(
            dir.join("dotfiles/settings.json"),
            dir.join("settings.json"),
        )
        .unwrap();
        let home = Home::at(&dir);
        home.save_now("settings.json", "{\"zoom\": 1.2}".into());
        assert!(
            std::fs::symlink_metadata(dir.join("settings.json"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("dotfiles/settings.json")).unwrap(),
            "{\"zoom\": 1.2}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
