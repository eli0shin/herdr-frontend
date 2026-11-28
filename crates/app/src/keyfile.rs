//! The key file: `keys.json` in herdr-frontend's config folder, in Eli's `keys.json` form
//! ([`Keymap::from_json`]), applied live.
//!
//! - With no file, the keys are Eli's ([`ELI_KEYS`], `feature-inventory.md` §22), and that file
//!   is written so there is one to edit.
//! - A [`Watcher`] thread reads the file every [`EVERY`] and, when its text changes, parses it
//!   there, off the UI thread, and hands the result over with a wake. A file that can't be used
//!   keeps the keys the app had, says why in a toast and in `keys.error` in the state folder (as
//!   Herdr app's `<name>.error` files do), which goes once the file is good again.
//! - The escape hatch (`ctrl-alt-shift-escape`) sets the file aside as `keys.json.reset-<time>`
//!   and writes Eli's keys in its place.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use model::{ELI_KEYS, KEY_FILE, Keymap};

/// Where a key file's problems are written.
pub const KEY_ERRORS: &str = "keys.error";

/// How often the watcher reads the file.
pub const EVERY: Duration = Duration::from_millis(500);

/// What the watcher found.
#[derive(Debug)]
pub enum Loaded {
    /// The file changed and reads: these are the keys now.
    Keys(Keymap),
    /// The file changed and can't be used, for this reason. The keys stay as they were.
    Bad(String),
    /// The escape hatch set the file aside under this name (or couldn't, and why).
    Reset(Result<String, String>),
}

/// The key file as the app read it at start.
#[derive(Debug)]
pub struct Start {
    pub keymap: Keymap,
    /// Why the keys aren't the file's, when it can't be used.
    pub problem: Option<String>,
    /// The text read, for the [`Watcher`] to compare with (`None`: there was no file).
    pub text: Option<String>,
}

/// Reads the key file in `dir`. A missing file is Eli's keys.
pub fn load(dir: &Path) -> Start {
    let text = std::fs::read_to_string(dir.join(KEY_FILE)).ok();
    let (keymap, problem) = match text.as_deref().map(Keymap::from_json) {
        Some(Ok(keys)) => (keys, None),
        Some(Err(e)) => (Keymap::eli(), Some(e.to_string())),
        None => (Keymap::eli(), None),
    };
    Start {
        keymap,
        problem,
        text,
    }
}

/// The thread that reads the key file, and the way to talk to it.
pub struct Watcher {
    news: Receiver<Loaded>,
    reset: Sender<()>,
}

impl Watcher {
    /// Starts watching `dir`'s key file, waking the UI with `wake` when there is news, and
    /// writing `keys.error` in `state`. `known` is the text the app has applied
    /// ([`Start::text`]): a file still the same is not news. A missing file is written with
    /// Eli's keys.
    pub fn start(
        dir: PathBuf,
        state: PathBuf,
        known: Option<String>,
        every: Duration,
        wake: rune::Waker,
    ) -> Watcher {
        let (news_tx, news) = mpsc::channel();
        let (reset, resets) = mpsc::channel();
        std::thread::Builder::new()
            .name("herdr-frontend-keys".into())
            .spawn(move || watch(&dir, &state, known, every, &resets, &news_tx, &wake))
            .expect("spawning the key file thread");
        Watcher { news, reset }
    }

    /// What the thread found, oldest first, if anything.
    pub fn next(&self) -> Option<Loaded> {
        self.news.try_recv().ok()
    }

    /// The escape hatch: set the file aside and write Eli's keys in its place.
    pub fn reset(&self) {
        let _ = self.reset.send(());
    }
}

fn watch(
    dir: &Path,
    state: &Path,
    known: Option<String>,
    every: Duration,
    resets: &Receiver<()>,
    news: &Sender<Loaded>,
    wake: &rune::Waker,
) {
    let path = dir.join(KEY_FILE);
    let read = || std::fs::read_to_string(&path).ok();
    let mut last = known;
    if last.is_none() && read().is_none() {
        // Eli's keys, which a missing file already meant: not news.
        write_defaults(&path);
        last = Some(ELI_KEYS.to_owned());
    }
    errors(state, last.as_deref());
    loop {
        match resets.recv_timeout(every) {
            Ok(()) => {
                let aside = set_aside(&path);
                write_defaults(&path);
                if news.send(Loaded::Reset(aside)).is_err() {
                    return;
                }
                wake();
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let mut text = read();
        if text == last {
            continue;
        }
        // An editor part-way through writing leaves a file that doesn't read: it is bad only
        // if it stays the same for another look.
        while let Some(t) = &text
            && Keymap::from_json(t).is_err()
        {
            std::thread::sleep(every);
            let again = read();
            if again == text {
                break;
            }
            text = again;
        }
        last = text;
        // A file removed while running is Eli's keys until one comes back.
        let loaded = match last.as_deref().map(Keymap::from_json) {
            None => Loaded::Keys(Keymap::eli()),
            Some(Ok(keys)) => Loaded::Keys(keys),
            Some(Err(e)) => Loaded::Bad(e.to_string()),
        };
        errors(state, last.as_deref());
        if news.send(loaded).is_err() {
            return;
        }
        wake();
    }
}

/// Writes `keys.error` for a file that can't be used, and removes it for one that can.
fn errors(dir: &Path, text: Option<&str>) {
    let path = dir.join(KEY_ERRORS);
    match text.map(Keymap::from_json) {
        Some(Err(e)) => {
            let written =
                std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&path, format!("{e}\n")));
            if let Err(error) = written {
                eprintln!("herdr-frontend: couldn't write {}: {error}", path.display());
            }
        }
        _ => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn write_defaults(path: &Path) {
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let mut tmp = path.as_os_str().to_owned();
            tmp.push(".tmp");
            std::fs::write(&tmp, ELI_KEYS)?;
            std::fs::rename(&tmp, path)
        });
    if let Err(error) = written {
        eprintln!("herdr-frontend: couldn't write {}: {error}", path.display());
    }
}

/// Renames the file to `keys.json.reset-<seconds since 1970>`; its new name.
fn set_aside(path: &Path) -> Result<String, String> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let name = format!("{KEY_FILE}.reset-{secs}");
    let aside = path.with_file_name(&name);
    match std::fs::rename(path, &aside) {
        Ok(()) => Ok(name),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("there was no key file".into()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("herdr-frontend-keys-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn next(w: &Watcher) -> Loaded {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(news) = w.next() {
                return news;
            }
            assert!(Instant::now() < deadline, "no news from the key file");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Writes a file whole, as editors save.
    fn save(path: &Path, text: &str) {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text).unwrap();
        std::fs::rename(&tmp, path).unwrap();
    }

    fn wait_for(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "{} never written",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_missing_file_is_elis_keys_and_is_written_for_editing() {
        let dir = scratch("missing");
        let start = load(&dir);
        assert_eq!(start.keymap, Keymap::eli());
        assert_eq!(start.problem, None);
        let w = Watcher::start(
            dir.clone(),
            dir.clone(),
            start.text,
            Duration::from_millis(10),
            Arc::new(|| {}),
        );
        wait_for(&dir.join(KEY_FILE));
        assert_eq!(
            std::fs::read_to_string(dir.join(KEY_FILE)).unwrap(),
            ELI_KEYS
        );
        std::thread::sleep(Duration::from_millis(50));
        assert!(w.next().is_none(), "the keys were Eli's already");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_apply_live_and_a_bad_file_keeps_the_keys_and_says_why() {
        let dir = scratch("live");
        let file = dir.join(KEY_FILE);
        std::fs::write(&file, r#"{"global": {"cmd-k": "tab.new"}}"#).unwrap();
        let start = load(&dir);
        assert_eq!(start.problem, None);
        assert_eq!(
            start.keymap.bindings().len(),
            2,
            "cmd-k and the escape hatch"
        );
        // `keys.error` goes in the state folder, never beside the key file.
        let state = dir.join("state");
        let w = Watcher::start(
            dir.clone(),
            state.clone(),
            start.text,
            Duration::from_millis(10),
            Arc::new(|| {}),
        );

        save(
            &file,
            r#"{"global": {"cmd-j": "tab.new", "cmd-k": "tab.close"}}"#,
        );
        let Loaded::Keys(keys) = next(&w) else {
            panic!("the new keys")
        };
        assert_eq!(keys.keys_for(&model::Action::TabClose).len(), 1);

        save(&file, r#"{"global": {"cmd-j": "tab.nope"}}"#);
        let Loaded::Bad(why) = next(&w) else {
            panic!("the file's problem")
        };
        assert_eq!(why, r#"global "cmd-j": unknown action "tab.nope""#);
        wait_for(&state.join(KEY_ERRORS));
        assert_eq!(
            std::fs::read_to_string(state.join(KEY_ERRORS)).unwrap(),
            format!("{why}\n")
        );
        assert!(!dir.join(KEY_ERRORS).exists());

        save(&file, r#"{"global": {"cmd-j": "tab.new"}}"#);
        assert!(matches!(next(&w), Loaded::Keys(_)));
        assert!(!state.join(KEY_ERRORS).exists(), "the problem is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_escape_hatch_sets_the_file_aside_and_writes_elis_keys() {
        let dir = scratch("reset");
        let file = dir.join(KEY_FILE);
        let mine = r#"{"global": {"cmd-k": "tab.new"}}"#;
        std::fs::write(&file, mine).unwrap();
        let w = Watcher::start(
            dir.clone(),
            dir.clone(),
            load(&dir).text,
            Duration::from_millis(10),
            Arc::new(|| {}),
        );
        w.reset();
        let Loaded::Reset(Ok(name)) = next(&w) else {
            panic!("set aside")
        };
        assert!(name.starts_with("keys.json.reset-"), "{name}");
        assert_eq!(std::fs::read_to_string(dir.join(&name)).unwrap(), mine);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), ELI_KEYS);
        let Loaded::Keys(keys) = next(&w) else {
            panic!("Eli's keys")
        };
        assert_eq!(keys, Keymap::eli());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
