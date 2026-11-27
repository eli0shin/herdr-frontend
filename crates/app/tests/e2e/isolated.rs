//! An isolated herdr-core for one app run (`docs/plan.md` "Testing rules"): its own
//! `HC_SESSION=test-*` and scratch XDG directories under `/tmp`, with every inherited `HC_*`
//! variable removed. The apps under test get the same environment, so they reach this core and
//! never the person's. Dropping it runs `hc server stop` and removes the scratch folder.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use herdr_client::{ConnectOptions, Connection, HcCommand, Transport};

static NEXT: AtomicU32 = AtomicU32::new(0);

pub struct Core {
    hc: PathBuf,
    /// The scratch folder: XDG dirs, the apps' homes, the seeded folders.
    pub scratch: PathBuf,
    /// What every process on this core gets: `HC_SESSION`, the XDG dirs and `SHELL`.
    env: Vec<(String, String)>,
    pub conn: Connection,
}

impl Core {
    /// Starts a fresh core (the first `hc connect` starts it).
    pub fn start(hc: &Path) -> Result<Core, String> {
        let id = format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        // Short: socket paths under it must fit in 104 bytes.
        let scratch = PathBuf::from(format!("/tmp/hfe-{id}"));
        let _ = std::fs::remove_dir_all(&scratch);
        let session = format!("test-hfe{id}");
        assert!(session.starts_with("test-"), "never the person's core");
        let mut env = vec![
            ("HC_SESSION".to_owned(), session),
            ("SHELL".to_owned(), "/bin/sh".to_owned()),
        ];
        for (var, dir) in [
            ("XDG_STATE_HOME", "state"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_RUNTIME_DIR", "run"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CACHE_HOME", "cache"),
        ] {
            let path = scratch.join(dir);
            std::fs::create_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            env.push((var.to_owned(), path.display().to_string()));
        }
        let mut command = HcCommand::new(hc);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HC_") {
                command = command.env_remove(key);
            }
        }
        for (key, value) in &env {
            command = command.env(key, value);
        }
        let conn = Connection::open(&Transport::Hc(command), &ConnectOptions::default())
            .map_err(|e| format!("connecting to the isolated core: {e}"))?;
        Ok(Core {
            hc: hc.to_owned(),
            scratch,
            env,
            conn,
        })
    }

    /// The core's session name.
    pub fn session(&self) -> &str {
        &self.env[0].1
    }

    /// Points `cmd` at this core: no inherited `HC_*`, this core's session and XDG dirs.
    pub fn apply(&self, cmd: &mut Command) {
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HC_") {
                cmd.env_remove(key);
            }
        }
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
    }

    /// Runs `hc <args>` on this core, giving up after 30 s.
    pub fn hc(&self, args: &[&str]) -> Result<Output, String> {
        let mut cmd = Command::new(&self.hc);
        self.apply(&mut cmd);
        let mut child = cmd
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("hc {args:?}: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().map_err(|e| e.to_string())?.is_none() {
            if Instant::now() > deadline {
                let _ = child.kill();
                return Err(format!("hc {args:?} hangs"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        child.wait_with_output().map_err(|e| e.to_string())
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.conn.close();
        let _ = self.hc(&["server", "stop"]);
        // Programs the apps started (review's update check, say) can still be writing while
        // the core stops them: remove what they leave a moment later too.
        let _ = std::fs::remove_dir_all(&self.scratch);
        std::thread::sleep(Duration::from_millis(500));
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}
