// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! What git says about each terminal's folder: the checkout it is in (a worktree's own top) and
//! its branch (`app:src/window.rs` `track_folders`, `folder_info` and `recheck`). An unnamed space
//! is called after its first terminal's checkout ([`crate::world::World::space_name`]), and the
//! space picker matches the branch of the terminal a space shows first.
//!
//! A live terminal's folder is looked up with `git -C <folder> rev-parse --show-toplevel
//! --abbrev-ref HEAD` through `machine.exec` on the terminal's machine, once per machine and
//! folder however many terminals are there. It is looked up again only when the answer could have
//! changed: the folder moved, a command or an agent's turn just ended there (a checkout, a
//! commit), or it is a repo, the window is in front and [`FALLBACK`] went by (a commit made in
//! another app).

use std::fmt;
use std::sync::Arc;

use herdr_proto::methods::ExecResult;
use herdr_proto::{RpcError, StatusState, Terminal};

/// How long a repo's answer stands while the window is in front, in seconds
/// (`FOLDER_FALLBACK`).
pub const FALLBACK: f64 = 60.0;
/// How long git may take before the lookup counts as failed, in seconds (the app's `EXEC_LIMIT`).
pub const TIMEOUT: f64 = 30.0;

/// The checkout a terminal's folder is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkout {
    /// The checkout's top folder (a worktree's own).
    pub top: String,
    /// `None` on a detached HEAD.
    pub branch: Option<String>,
}

/// The last lookup asked for a terminal.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Check {
    /// When it was asked (the model's clock).
    pub at: f64,
    /// The folder it asked about.
    pub cwd: String,
    /// Whether the terminal was [`busy`] when last seen.
    pub busy: bool,
    /// A repo, or git couldn't say (the machine unreachable): worth a look again.
    pub repo: bool,
}

/// Whether the shell or an agent is in front of a terminal: the app's program table decides
/// (its `shell` and `agent` judgements, as Herdr app's `program` has them).
#[derive(Clone)]
pub struct Front(Arc<dyn Fn(&Terminal) -> bool + Send + Sync>);

impl Front {
    pub fn new(quiet: impl Fn(&Terminal) -> bool + Send + Sync + 'static) -> Front {
        Front(Arc::new(quiet))
    }

    fn quiet(&self, t: &Terminal) -> bool {
        (self.0)(t)
    }
}

/// Until the app says, every terminal counts as quiet: only a move or the fallback looks again.
impl Default for Front {
    fn default() -> Front {
        Front::new(|_| true)
    }
}

impl fmt::Debug for Front {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Front(..)")
    }
}

/// An agent working, or a program other than the shell in front (an agent between turns isn't,
/// even started from a shell).
pub(crate) fn busy(t: &Terminal, front: &Front) -> bool {
    t.status
        .as_ref()
        .is_some_and(|s| s.state == StatusState::Working)
        || !front.quiet(t)
}

/// Whether to look a terminal's folder up again: only when the answer could have changed.
pub(crate) fn recheck(last: Option<&Check>, cwd: &str, busy: bool, active: bool, now: f64) -> bool {
    let Some(last) = last else { return true };
    last.cwd != cwd || (last.busy && !busy) || (active && last.repo && now - last.at >= FALLBACK)
}

/// When a check is due again by the fallback alone; `None` when it isn't a repo.
pub(crate) fn due(check: &Check) -> Option<f64> {
    check.repo.then_some(check.at + FALLBACK)
}

/// The lookup for a folder.
pub(crate) fn argv(cwd: &str) -> Vec<String> {
    [
        "git",
        "-C",
        cwd,
        "rev-parse",
        "--show-toplevel",
        "--abbrev-ref",
        "HEAD",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// What a lookup's answer says: the checkout (`None` outside git, or when git couldn't say), and
/// whether it is a repo or git couldn't say. Only git's own "not a git repository" means no
/// repo; a failed run, a repo without commits or one git won't read are asked again.
pub(crate) fn read(result: &Result<ExecResult, RpcError>) -> (Option<Checkout>, bool) {
    let Ok(r) = result else {
        return (None, true);
    };
    if !r.succeeded() {
        let no = !r.timed_out
            && r.stderr_text()
                .to_lowercase()
                .contains("not a git repository");
        return (None, !no);
    }
    let out = r.stdout_text();
    let lines: Vec<&str> = out.lines().collect();
    let [top, head] = lines.as_slice() else {
        return (None, true);
    };
    let branch = (*head != "HEAD").then(|| (*head).to_owned());
    (
        Some(Checkout {
            top: (*top).to_owned(),
            branch,
        }),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use herdr_proto::Status;

    fn ran(code: i32, stdout: &str, stderr: &str) -> Result<ExecResult, RpcError> {
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        Ok(ExecResult {
            code: Some(code),
            stdout: b64(stdout),
            stderr: b64(stderr),
            ..Default::default()
        })
    }

    fn check(at: f64, busy: bool, repo: bool) -> Check {
        Check {
            at,
            cwd: "/src".into(),
            busy,
            repo,
        }
    }

    #[test]
    fn an_answer_reads_as_the_checkout_and_its_branch() {
        let (checkout, repo) = read(&ran(0, "/src/herdr\nmain\n", ""));
        assert_eq!(
            checkout,
            Some(Checkout {
                top: "/src/herdr".into(),
                branch: Some("main".into())
            })
        );
        assert!(repo);
        let (detached, _) = read(&ran(0, "/src/herdr\nHEAD\n", ""));
        assert_eq!(
            detached.unwrap().branch,
            None,
            "a detached HEAD has no branch"
        );
    }

    #[test]
    fn only_gits_own_no_means_no_repo() {
        let no = "fatal: not a git repository (or any of the parent directories): .git\n";
        assert_eq!(read(&ran(128, "", no)), (None, false));
        // A repo without commits: `--abbrev-ref HEAD` fails.
        assert_eq!(
            read(&ran(
                128,
                "/src/new\nHEAD\n",
                "fatal: ambiguous argument 'HEAD'"
            )),
            (None, true)
        );
        assert_eq!(read(&ran(0, "garbled", "")), (None, true));
        let timed_out = Ok(ExecResult {
            timed_out: true,
            ..Default::default()
        });
        assert_eq!(read(&timed_out), (None, true));
        let failed = Err(RpcError {
            code: -32000,
            message: "machine unreachable".into(),
            kind: herdr_proto::ErrorKind::Unavailable,
            hint: None,
            data: Default::default(),
        });
        assert_eq!(read(&failed), (None, true));
    }

    #[test]
    fn a_folder_is_looked_up_again_only_when_its_answer_could_have_changed() {
        assert!(recheck(None, "/src", false, false, 0.0), "never asked");
        let quiet = check(0.0, false, true);
        assert!(!recheck(Some(&quiet), "/src", false, true, 10.0));
        assert!(
            recheck(Some(&quiet), "/src/crates", false, false, 10.0),
            "it moved"
        );
        let busy = check(0.0, true, true);
        assert!(
            !recheck(Some(&busy), "/src", true, false, 10.0),
            "still running"
        );
        assert!(
            recheck(Some(&busy), "/src", false, false, 10.0),
            "a command ended"
        );
        assert!(
            recheck(Some(&quiet), "/src", false, true, FALLBACK),
            "a repo's fallback"
        );
        assert!(
            !recheck(Some(&quiet), "/src", false, false, FALLBACK),
            "window behind"
        );
        let plain = check(0.0, false, false);
        assert!(
            !recheck(Some(&plain), "/src", false, true, FALLBACK * 2.0),
            "not a repo"
        );
        assert_eq!(due(&quiet), Some(FALLBACK));
        assert_eq!(due(&plain), None);
    }

    #[test]
    fn an_agent_working_or_a_program_in_front_is_busy() {
        let shell_only = Front::new(|t: &Terminal| t.argv.is_empty());
        let shell = Terminal::default();
        assert!(!busy(&shell, &shell_only));
        let program = Terminal {
            argv: vec!["make".into()],
            ..Default::default()
        };
        assert!(busy(&program, &shell_only));
        let working = Terminal {
            status: Some(Status {
                state: StatusState::Working,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(busy(&working, &shell_only));
        assert!(!busy(&program, &Front::default()));
    }

    #[test]
    fn the_lookup_runs_git_in_the_folder() {
        assert_eq!(
            argv("/src/x"),
            [
                "git",
                "-C",
                "/src/x",
                "rev-parse",
                "--show-toplevel",
                "--abbrev-ref",
                "HEAD"
            ]
        );
    }
}
