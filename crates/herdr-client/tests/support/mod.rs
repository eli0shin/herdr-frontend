//! An isolated herdr-core for integration tests (`docs/plan.md` "Testing rules").
//!
//! Each core gets its own `HC_SESSION=test-*` and scratch XDG directories under `/tmp` (short, as
//! socket paths must be), with every inherited `HC_*` variable removed, so it never touches the
//! person's core. Dropping it runs `hc server stop` under the same environment.
//!
//! `hc` comes from `HERDR_TEST_HC`, else wherever `find_hc` finds it. Without one the tests skip,
//! except under CI (`CI` set), where that is a failure.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use herdr_client::{HcCommand, Transport, find_hc, hc_version};

static NEXT: AtomicU32 = AtomicU32::new(0);

pub struct IsolatedCore {
    hc: HcCommand,
    scratch: PathBuf,
    session: String,
}

/// The scratch directories each core gets, by variable.
const XDG: [(&str, &str); 5] = [
    ("XDG_STATE_HOME", "state"),
    ("XDG_CONFIG_HOME", "config"),
    ("XDG_RUNTIME_DIR", "run"),
    ("XDG_DATA_HOME", "data"),
    ("XDG_CACHE_HOME", "cache"),
];

/// The `hc` to test with, or `None` to skip.
fn test_hc() -> Option<PathBuf> {
    if let Some(hc) = std::env::var_os("HERDR_TEST_HC") {
        let hc = PathBuf::from(hc);
        assert!(
            hc_version(&hc).is_some(),
            "HERDR_TEST_HC={} is not a working hc",
            hc.display()
        );
        return Some(hc);
    }
    let found = find_hc(None);
    assert!(
        found.is_some() || std::env::var_os("CI").is_none(),
        "no hc to test against: set HERDR_TEST_HC"
    );
    found
}

impl IsolatedCore {
    /// A new isolated core's environment. The core itself starts on the first `hc connect`.
    pub fn new() -> Option<IsolatedCore> {
        let Some(program) = test_hc() else {
            eprintln!("skipped: no hc found (set HERDR_TEST_HC to run against an isolated core)");
            return None;
        };
        let id = format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let scratch = PathBuf::from(format!("/tmp/hfc-{id}"));
        let _ = std::fs::remove_dir_all(&scratch);
        let mut hc = HcCommand::new(program);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HC_") {
                hc = hc.env_remove(key);
            }
        }
        let session = format!("test-hfc{id}");
        assert!(session.starts_with("test-"));
        hc = hc.env("HC_SESSION", &session).env("SHELL", "/bin/sh");
        for (var, dir) in XDG {
            let path = scratch.join(dir);
            std::fs::create_dir_all(&path).unwrap();
            hc = hc.env(var, path);
        }
        Some(IsolatedCore {
            hc,
            scratch,
            session,
        })
    }

    /// A command that reaches this core as another machine, for
    /// `hc machine add --via <it> --name <name>` on a second isolated core: like ssh, it runs
    /// the shell line it is given, here under this core's session and scratch directories, with
    /// this `hc` first on `PATH`. A fake second machine, on this computer.
    pub fn via(&self) -> String {
        let quote = |v: &str| format!("'{}'", v.replace('\'', r"'\''"));
        let bin = self.hc.program().parent().unwrap_or(Path::new("/"));
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut words = vec![
            "env".to_owned(),
            format!("PATH={}", quote(&path)),
            format!("HC_SESSION={}", quote(&self.session)),
            "SHELL=/bin/sh".to_owned(),
        ];
        for (var, dir) in XDG {
            let dir = self.scratch.join(dir);
            words.push(format!("{var}={}", quote(&dir.to_string_lossy())));
        }
        words.push("sh -c".to_owned());
        words.join(" ")
    }

    pub fn transport(&self) -> Transport {
        Transport::Hc(self.hc.clone())
    }

    /// A scratch folder for terminals to start in.
    pub fn folder(&self) -> &Path {
        &self.scratch
    }

    /// Runs `hc <args>` against this core, giving up after 60 s.
    pub fn hc(&self, args: &[&str]) -> Output {
        let mut child = self
            .hc
            .command(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        child.wait_with_output().unwrap()
    }

    /// A planned restart of this core: programs keep running and the next core adopts them.
    pub fn restart(&self) {
        let out = self.hc(&["server", "restart"]);
        assert!(
            out.status.success(),
            "hc server restart: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

impl Drop for IsolatedCore {
    fn drop(&mut self) {
        self.hc(&["server", "stop"]);
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

/// Polls `f` until it returns `Some`, for up to `timeout`.
pub fn eventually<T>(timeout: Duration, what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = f() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
