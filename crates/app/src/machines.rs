//! What the machine dialogs keep between frames (`docs/machines.md`): the machine calls under
//! way, what each failed add said, and the ssh config's hosts, read off the UI thread each time
//! the add dialog opens. The rules (rows, guidance, matching) are [`model::machines`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use model::machines::{self, AddFailure, Host, MachineCall};

/// The machine dialogs' state.
#[derive(Default)]
pub struct Machines {
    /// Calls under way, by the machine (or, for an add, the target) they are about.
    pub busy: BTreeMap<String, MachineCall>,
    /// What the last add of each target said when it failed.
    pub failed: BTreeMap<String, AddFailure>,
    /// The ssh config's hosts, once read.
    pub hosts: Option<Vec<Host>>,
    reading: Option<mpsc::Receiver<Vec<Host>>>,
    /// The ssh config to read instead of `~/.ssh/config` (tests and scenes).
    pub ssh_config: Option<PathBuf>,
    /// The machine whose menu the Machines dialog last opened: the dialog opens on it again,
    /// as it comes back after connecting or copying.
    pub picked: Option<String>,
}

impl Machines {
    /// Reads the ssh config again on its own thread; `wake` once it's read.
    pub fn read_hosts(&mut self, home: &Path, wake: Option<rune::Waker>) {
        let home = home.to_owned();
        let file = self
            .ssh_config
            .clone()
            .unwrap_or_else(|| home.join(".ssh/config"));
        let (tx, rx) = mpsc::channel();
        self.reading = Some(rx);
        std::thread::Builder::new()
            .name("ssh-config".into())
            .spawn(move || {
                let _ = tx.send(read_hosts(&file, &home));
                if let Some(wake) = wake {
                    wake();
                }
            })
            .expect("spawning the ssh config reader");
    }

    /// The hosts, when the reader has them. True when they arrived.
    pub fn take_hosts(&mut self) -> bool {
        let Some(hosts) = self.reading.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return false;
        };
        self.reading = None;
        self.hosts = Some(hosts);
        true
    }

    /// Waits for the reader (scenes and tests, which draw what is there at once).
    pub fn wait_hosts(&mut self) {
        if let Some(hosts) = self.reading.take().and_then(|rx| rx.recv().ok()) {
            self.hosts = Some(hosts);
        }
    }
}

/// The hosts `file` and the files it includes name. No file, no hosts.
pub fn read_hosts(file: &Path, home: &Path) -> Vec<Host> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    machines::ssh_hosts(&text, &mut |value| {
        include_files(value, home)
            .iter()
            .filter_map(|f| std::fs::read_to_string(f).ok())
            .collect()
    })
}

/// The files an `Include` names, as ssh finds them: `~` is home, a relative path is under
/// `~/.ssh`, and `*` and `?` in the file name match the files there, in order.
fn include_files(value: &str, home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for word in words(value) {
        let path = if let Some(rest) = word.strip_prefix("~/") {
            home.join(rest)
        } else if word.starts_with('/') {
            PathBuf::from(&word)
        } else {
            home.join(".ssh").join(&word)
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        match (name, path.parent()) {
            (Some(name), Some(dir)) if name.contains(['*', '?']) => {
                let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter(|e| wildcard(&name, &e.file_name().to_string_lossy()))
                    .map(|e| e.path())
                    .collect();
                found.sort();
                out.extend(found);
            }
            _ => out.push(path),
        }
    }
    out
}

/// An `Include` line's words: quoted ones keep their spaces.
fn words(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value.trim();
    while !rest.is_empty() {
        if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            out.push(quoted[..end].to_owned());
            rest = quoted.get(end + 1..).unwrap_or("").trim_start();
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push(rest[..end].to_owned());
            rest = rest[end..].trim_start();
        }
    }
    out
}

/// Whether `name` matches a pattern of `*` (any run) and `?` (any one character).
fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some((i, j));
            i += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_runs_and_single_characters() {
        assert!(wildcard("*", "work"));
        assert!(wildcard("*.conf", "a.conf"));
        assert!(!wildcard("*.conf", "a.conf~"));
        assert!(wildcard("h?st", "host"));
        assert!(!wildcard("h?st", "hst"));
        assert!(wildcard("a*b*c", "aXbYc"));
    }

    #[test]
    fn includes_read_home_relative_and_wildcard_files_in_order() {
        let home = std::env::temp_dir().join(format!("hf-ssh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".ssh/config.d")).unwrap();
        std::fs::create_dir_all(home.join("extra dir")).unwrap();
        std::fs::write(
            home.join(".ssh/config"),
            "Include config.d/*\nInclude \"~/extra dir/hosts\" missing\nHost devbox\n",
        )
        .unwrap();
        std::fs::write(home.join(".ssh/config.d/b"), "Host omarchy\n").unwrap();
        std::fs::write(home.join(".ssh/config.d/a"), "Host work-mac\n").unwrap();
        std::fs::write(home.join("extra dir/hosts"), "Host vps\n  User can\n").unwrap();
        let hosts = read_hosts(&home.join(".ssh/config"), &home);
        let names: Vec<_> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(names, ["work-mac", "omarchy", "vps", "devbox"]);
        assert!(read_hosts(&home.join("nothing"), &home).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}
