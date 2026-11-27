//! Closing spaces whose folder was deleted (`feature-inventory.md` §3, Eli's
//! `cfg/mods/navigator-status/dead-spaces.js`). `gh pr merge --delete-branch`, a plain
//! `git worktree remove` or `rm -rf` take a worktree away without telling anyone, so its space
//! lingers with terminals in a folder that no longer exists.
//!
//! Every [`SWEEP_EVERY`] seconds, a space on this computer that isn't the shown one qualifies when
//! it has live terminals, none of them reports a working agent, and its folder (home, for a
//! space made without one) and each live terminal's folder are all gone. It must qualify on two
//! sweeps in a row with the same folders before it closes, so a folder that is only briefly
//! missing keeps its space. [`crate::Model`] runs the sweeps: one `machine.exec` checks every
//! candidate's folders at once ([`check`]), and the model is read again when it answers.

use std::collections::{BTreeMap, BTreeSet};

use herdr_proto::methods::{ExecResult, MachineExec};
use herdr_proto::{RpcError, StatusState};
use herdr_shared::{labels, machine_of};

use crate::world::World;

/// How often the sweep runs, in seconds (the mod's `cx.timer.every(30000, …)`).
pub const SWEEP_EVERY: f64 = 30.0;
/// How long the folder check may take, in seconds; one that takes longer closes nothing.
pub const CHECK_TIMEOUT: f64 = 10.0;

/// Prints 1 for each folder that exists and 0 for each that doesn't, in order.
const CHECK: &str = r#"for d; do if [ -d "$d" ]; then echo 1; else echo 0; fi; done"#;

/// A space that qualifies now, with the folders it was found with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Candidate {
    /// Its own folder (home when it has none) first, then each terminal's, without repeats.
    pub folders: Vec<String>,
    /// The folders, sorted: a space that qualifies with other folders starts over.
    pub key: String,
}

/// The spaces that qualify now, by id. `shown`: the space on screen, which never does.
pub(crate) fn candidates(
    world: &World,
    shown: Option<&str>,
    home: &str,
) -> BTreeMap<String, Candidate> {
    let mut by_space: BTreeMap<&str, Vec<&herdr_proto::Terminal>> = BTreeMap::new();
    for t in world.live() {
        // Only this computer's terminals: another machine's labels name its own spaces.
        if machine_of(&t.address).is_some() {
            continue;
        }
        if let Some(space) = labels::space(&t.labels) {
            by_space.entry(space).or_default().push(t);
        }
    }
    let Some(doc) = world.machine(None).and_then(|m| m.shown()) else {
        return BTreeMap::new();
    };
    let mut found = BTreeMap::new();
    for space in &doc.spaces {
        if Some(space.id.as_str()) == shown {
            continue;
        }
        let Some(list) = by_space.get(space.id.as_str()) else {
            continue;
        };
        let own = if space.folder.is_empty() {
            home
        } else {
            &space.folder
        };
        let mut folders = vec![own.to_owned()];
        let qualifies = list.iter().all(|t| {
            let working = t
                .status
                .as_ref()
                .is_some_and(|s| s.state == StatusState::Working);
            // The live folder when the core knows what runs there, else the launch folder.
            let cwd = t.foreground.as_ref().map_or(&t.cwd, |f| &f.cwd);
            if working || cwd.is_empty() {
                return false;
            }
            if !folders.contains(cwd) {
                folders.push(cwd.clone());
            }
            true
        });
        if !qualifies || !folders.iter().all(|f| f.starts_with('/')) {
            continue;
        }
        let mut sorted = folders.clone();
        sorted.sort();
        let key = sorted.join("\n");
        found.insert(space.id.clone(), Candidate { folders, key });
    }
    found
}

/// The folders to check for `candidates`, each once.
pub(crate) fn paths(candidates: &BTreeMap<String, Candidate>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    candidates
        .values()
        .flat_map(|c| &c.folders)
        .filter(|f| seen.insert(f.as_str()))
        .cloned()
        .collect()
}

/// The `machine.exec` that checks `paths` on this computer.
pub(crate) fn check(paths: &[String]) -> MachineExec {
    let mut argv: Vec<String> = ["sh", "-c", CHECK, "sh"].map(str::to_owned).into();
    argv.extend(paths.iter().cloned());
    MachineExec {
        argv,
        timeout: Some(CHECK_TIMEOUT),
        ..Default::default()
    }
}

/// Which of `paths` exist, from the check's answer; `None` when it failed or answered for a
/// different number of folders.
pub(crate) fn existing(
    paths: &[String],
    result: &Result<ExecResult, RpcError>,
) -> Option<BTreeMap<String, bool>> {
    let result = result.as_ref().ok().filter(|r| r.succeeded())?;
    let stdout = result.stdout_text();
    let answers: Vec<&str> = stdout.trim().split('\n').collect();
    (answers.len() == paths.len()).then(|| {
        paths
            .iter()
            .zip(answers)
            .map(|(p, a)| (p.clone(), a == "1"))
            .collect()
    })
}

/// The sweep's memory between sweeps.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DeadSpaces {
    /// Each space found dead on the last sweep, with its [`Candidate::key`] then.
    suspects: BTreeMap<String, String>,
    /// When the next sweep is due; `None` until the first listing.
    pub due: Option<f64>,
}

impl DeadSpaces {
    /// Nothing qualified: no space is suspected any more.
    pub fn none(&mut self) {
        self.suspects.clear();
    }

    /// A check came back. `before`: the spaces it checked; `after`: the spaces that qualify
    /// now, since an agent may have started working, a terminal moved or the space been shown
    /// while it ran. Returns the spaces to close (dead on this sweep and the last, with the same
    /// folders), with their candidates; the spaces dead for the first time are remembered.
    pub fn judge(
        &mut self,
        before: BTreeMap<String, Candidate>,
        exists: &BTreeMap<String, bool>,
        after: &BTreeMap<String, Candidate>,
    ) -> Vec<(String, Candidate)> {
        let mut next = BTreeMap::new();
        let mut close = Vec::new();
        for (id, candidate) in before {
            if after.get(&id).map(|c| &c.key) != Some(&candidate.key) {
                continue;
            }
            if candidate
                .folders
                .iter()
                .any(|f| exists.get(f).copied().unwrap_or(true))
            {
                continue;
            }
            if self.suspects.get(&id) == Some(&candidate.key) {
                close.push((id, candidate));
            } else {
                next.insert(id, candidate.key.clone());
            }
        }
        self.suspects = next;
        close
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(stdout: &str) -> Result<ExecResult, RpcError> {
        use base64::Engine;
        Ok(ExecResult {
            code: Some(0),
            stdout: base64::engine::general_purpose::STANDARD.encode(stdout),
            ..Default::default()
        })
    }

    fn paths_of(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_check_passes_the_folders_as_arguments_not_as_shell_source() {
        let exec = check(&paths_of(&["/w/a b", "/w/$(rm -rf ~)"]));
        assert_eq!(exec.argv[..2], ["sh", "-c"]);
        assert_eq!(exec.argv[3..], ["sh", "/w/a b", "/w/$(rm -rf ~)"]);
        assert_eq!(exec.on, None);
        assert_eq!(exec.timeout, Some(CHECK_TIMEOUT));
    }

    #[test]
    fn the_check_runs_in_sh() {
        let dir = std::env::temp_dir();
        let here = dir.to_string_lossy().into_owned();
        let gone = dir
            .join("herdr-frontend-no-such-folder")
            .to_string_lossy()
            .into_owned();
        let exec = check(&[here, gone]);
        let out = std::process::Command::new(&exec.argv[0])
            .args(&exec.argv[1..])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "1\n0\n");
    }

    #[test]
    fn a_failed_or_short_answer_says_nothing() {
        let paths = paths_of(&["/a", "/b"]);
        assert_eq!(
            existing(&paths, &ok("1\n0\n")),
            Some([("/a".into(), true), ("/b".into(), false)].into())
        );
        assert_eq!(existing(&paths, &ok("1\n")), None);
        assert_eq!(existing(&paths, &ok("")), None);
        let failed = Ok(ExecResult {
            code: Some(1),
            ..Default::default()
        });
        assert_eq!(existing(&paths, &failed), None);
        let timed_out = ok("0\n0\n").map(|r| ExecResult {
            timed_out: true,
            ..r
        });
        assert_eq!(existing(&paths, &timed_out), None);
    }
}
