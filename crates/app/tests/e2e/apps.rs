//! The two apps the harness runs, each in a real window on an isolated core with an isolated
//! home: herdr-frontend (the debug build, `HERDR_FRONTEND_E2E=1`) and Herdr app (`herdr-app`, its
//! home a copy of Eli's config). Both are driven through their own CLI (`ls`, `status`, `do`);
//! herdr-frontend also takes keys, and reports its tree and window number, over its control
//! socket (`herdr_frontend::e2e`). Nothing here sends OS-level input.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::isolated::Core;

/// How long an app has to open its window and list the seeded spaces.
const READY: Duration = Duration::from_secs(45);
/// How long a CLI call may take.
const CLI: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Frontend,
    HerdrApp,
}

impl Which {
    /// The name in file names and the report.
    pub fn slug(self) -> &'static str {
        match self {
            Which::Frontend => "frontend",
            Which::HerdrApp => "herdr-app",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Which::Frontend => "herdr-frontend",
            Which::HerdrApp => "Herdr app",
        }
    }
}

/// Where each app comes from.
#[derive(Clone)]
pub struct Programs {
    pub frontend: PathBuf,
    /// `herdr-app` and the config folder its home copies, when both are there.
    pub herdr_app: Option<(PathBuf, PathBuf)>,
}

/// One app's window, running on a core.
pub struct App {
    pub which: Which,
    program: PathBuf,
    home: PathBuf,
    log: PathBuf,
    /// The window's process, while it runs.
    child: Option<Child>,
}

impl App {
    /// Makes the app's home under the core's scratch folder and opens its window.
    pub fn launch(
        which: Which,
        programs: &Programs,
        core: &Core,
        log: &Path,
    ) -> Result<App, String> {
        let (program, home) = match which {
            Which::Frontend => {
                let home = core.scratch.join("frontend");
                std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
                (programs.frontend.clone(), home)
            }
            Which::HerdrApp => {
                let (program, config) = programs
                    .herdr_app
                    .clone()
                    .ok_or("no herdr-app or no config to copy")?;
                let home = core.scratch.join("herdr-app");
                copy_config(&config, &home)?;
                (program, home)
            }
        };
        let mut app = App {
            which,
            program,
            home,
            log: log.to_owned(),
            child: None,
        };
        app.child = Some(app.spawn(core)?);
        Ok(app)
    }

    fn spawn(&self, core: &Core) -> Result<Child, String> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .map_err(|e| format!("{}: {e}", self.log.display()))?;
        let mut cmd = self.command(core);
        cmd.stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        cmd.spawn()
            .map_err(|e| format!("starting {}: {e}", self.program.display()))
    }

    /// The app's program with its home and the core's environment.
    fn command(&self, core: &Core) -> Command {
        let mut cmd = Command::new(&self.program);
        core.apply(&mut cmd);
        match self.which {
            Which::Frontend => {
                cmd.env(herdr_frontend::control::HOME_VAR, &self.home)
                    .env(herdr_frontend::e2e::VAR, "1");
            }
            Which::HerdrApp => {
                // No update checks or telemetry from the copy: both go to a closed port.
                cmd.env("HERDR_APP_HOME", &self.home)
                    .env("HERDR_RELEASES_URL", "http://127.0.0.1:9")
                    .env("HERDR_TELEMETRY_URL", "http://127.0.0.1:9")
                    .env("HERDR_APP_NO_SOUND", "1");
            }
        }
        cmd
    }

    /// Runs `<app> <args>` against this window: its exit code and the JSON it printed.
    pub fn cli(&self, core: &Core, args: &[&str]) -> Result<(i32, Value), String> {
        let mut child = self
            .command(core)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + CLI;
        while child.try_wait().map_err(|e| e.to_string())?.is_none() {
            if Instant::now() > deadline {
                let _ = child.kill();
                return Err(format!("{} {args:?} hangs", self.which.title()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let code = out.status.code().unwrap_or(-1);
        let json = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            json!({ "stdout": String::from_utf8_lossy(&out.stdout),
                    "stderr": String::from_utf8_lossy(&out.stderr).trim() })
        });
        Ok((code, json))
    }

    /// Waits for the window to list exactly the seeded `spaces`. Anything else means it isn't on
    /// the isolated core, and the app is stopped before it does anything. With no `spaces` (a
    /// restart mid-scenario, after steps made or closed some), any listing will do.
    pub fn wait_ready(&mut self, core: &Core, spaces: Option<&[String]>) -> Result<(), String> {
        let deadline = Instant::now() + READY;
        let title = self.which.title();
        loop {
            if let Some(status) = self.exited()? {
                return Err(format!(
                    "{title} exited ({status}); see {}",
                    self.log.display()
                ));
            }
            if let Ok((0, listed)) = self.cli(core, &["ls"]) {
                let ids: Vec<&str> = listed["spaces"]
                    .as_array()
                    .map(|s| s.iter().filter_map(|s| s["id"].as_str()).collect())
                    .unwrap_or_default();
                let listed_all = match spaces {
                    Some(spaces) => {
                        if ids.iter().any(|id| !spaces.iter().any(|s| s == id)) {
                            self.kill();
                            return Err(format!(
                                "{title} lists spaces the seed didn't make ({ids:?}): \
                                 not the isolated core; stopped"
                            ));
                        }
                        ids.len() == spaces.len()
                    }
                    None => true,
                };
                if listed_all && self.window_ready() {
                    return Ok(());
                }
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "{title} never showed the seeded spaces; see {}",
                    self.log.display()
                ));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// herdr-frontend: its window is open and has drawn a frame. Herdr app answering `ls` is
    /// enough.
    fn window_ready(&self) -> bool {
        if self.which == Which::HerdrApp {
            return true;
        }
        let window = self.probe(&json!({"window": true}));
        window.is_ok_and(|w| w["window"].as_u64().is_some()) && self.tree().is_ok()
    }

    /// One harness request on herdr-frontend's control socket (`herdr_frontend::e2e`).
    pub fn probe(&self, request: &Value) -> Result<Value, String> {
        let socket = self.home.join(herdr_frontend::control::SOCKET);
        let mut stream =
            UnixStream::connect(&socket).map_err(|e| format!("{}: {e}", socket.display()))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| e.to_string())?;
        stream
            .write_all(format!("{request}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        let mut line = String::new();
        BufReader::new(&stream)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let reply: Value = serde_json::from_str(&line).map_err(|e| format!("{e}: {line:?}"))?;
        match reply.get("error") {
            Some(error) => Err(error.as_str().unwrap_or("error").to_owned()),
            None => Ok(reply),
        }
    }

    /// herdr-frontend's tree of its last drawn frame.
    pub fn tree(&self) -> Result<Value, String> {
        self.probe(&json!({"tree": true}))
    }

    /// Waits for the window to come to rest. herdr-frontend is at rest when the tree of its
    /// drawn frames stays the same for half a second (up to `max`); Herdr app gets `max`. True
    /// when it came to rest.
    pub fn settle(&self, max: Duration) -> bool {
        if self.which == Which::HerdrApp {
            std::thread::sleep(max);
            return true;
        }
        let start = Instant::now();
        let mut last: Option<Value> = None;
        let mut since = Instant::now();
        std::thread::sleep(Duration::from_millis(250));
        while start.elapsed() < max.max(Duration::from_secs(1)) {
            let elements = self.tree().ok().map(|t| t["elements"].clone());
            if elements.is_some() && elements == last {
                if since.elapsed() >= Duration::from_millis(500) {
                    return true;
                }
            } else {
                last = elements;
                since = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// Saves the whole window as a PNG: `screencapture -l <window>` for herdr-frontend, the
    /// app's own `herdr-app shot` for Herdr app.
    pub fn capture(&self, core: &Core, png: &Path) -> Result<(), String> {
        match self.which {
            Which::Frontend => {
                let window = self.probe(&json!({"window": true}))?["window"]
                    .as_u64()
                    .ok_or("the window has no number")?;
                let out = Command::new("/usr/sbin/screencapture")
                    .args(["-x", "-o", "-l", &window.to_string()])
                    .arg(png)
                    .output()
                    .map_err(|e| format!("screencapture: {e}"))?;
                if !out.status.success() || !png.exists() {
                    return Err(format!(
                        "screencapture failed: {}",
                        String::from_utf8_lossy(&out.stderr).trim()
                    ));
                }
            }
            Which::HerdrApp => {
                let png = png.display().to_string();
                let (code, reply) = self.cli(core, &["shot", &png])?;
                if code != 0 || !Path::new(&png).exists() {
                    return Err(format!("herdr-app shot failed: {reply}"));
                }
            }
        }
        Ok(())
    }

    /// Quits the window. herdr-frontend is sent SIGTERM (it saves as it goes); Herdr app is
    /// asked to quit. Either is killed if it hasn't gone in 5 s.
    pub fn quit(&mut self, core: &Core) {
        match self.which {
            Which::Frontend => {
                if let Some(child) = &self.child {
                    let _ = Command::new("/bin/kill")
                        .arg(child.id().to_string())
                        .status();
                }
            }
            Which::HerdrApp => {
                let _ = self.cli(core, &["do", "app.quit"]);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(self.exited(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        self.kill();
    }

    /// Quits and opens the window again on the same home and core.
    pub fn restart(&mut self, core: &Core) -> Result<(), String> {
        self.quit(core);
        self.child = Some(self.spawn(core)?);
        Ok(())
    }

    /// Whether the window is still open: an error saying how it ended, once it has.
    pub fn running(&mut self) -> Result<(), String> {
        match self.exited()? {
            None => Ok(()),
            Some(status) => Err(format!(
                "{} exited ({status}); see {}",
                self.which.title(),
                self.log.display()
            )),
        }
    }

    /// How the window's process ended, once it has; `None` while it runs.
    fn exited(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        match &mut self.child {
            Some(child) => child.try_wait().map_err(|e| e.to_string()),
            None => Err("not running".into()),
        }
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Copies Eli's Herdr app config into `home`, minus what belongs to his running window (its
/// socket, its view, its window place and its seen marks) and what his use of it left behind:
/// the mods' stores (`data/`: the Agents list's times, the palette's recent commands) and the
/// shared state, which starts from his tracked `shared.defaults.json` as a fresh `dot stow`
/// leaves it (so his hidden machines and space colors stay out). Both apps then start from
/// their defaults. The window opens at 1200 × 800, as herdr-frontend's does. The source is only
/// read.
fn copy_config(config: &Path, home: &Path) -> Result<(), String> {
    const SKIP: [&str; 7] = [
        "herdr-app.sock",
        "view.json",
        "window.json",
        "seen.json",
        "data",
        "shared.json",
        "shared.defaults.json",
    ];
    copy_dir(config, home, &SKIP)?;
    let defaults = config.join("shared.defaults.json");
    if defaults.is_file() {
        std::fs::copy(&defaults, home.join("shared.json"))
            .map_err(|e| format!("{}: {e}", defaults.display()))?;
        std::fs::copy(&defaults, home.join("shared.defaults.json"))
            .map_err(|e| format!("{}: {e}", defaults.display()))?;
    }
    let window = json!({"state": "windowed", "bounds": [120.0, 60.0, 1200.0, 800.0]});
    std::fs::write(home.join("window.json"), window.to_string()).map_err(|e| e.to_string())
}

fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if skip.iter().any(|s| name == *s) {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let (src, dst) = (entry.path(), to.join(&name));
        if kind.is_dir() {
            copy_dir(&src, &dst, &[])?;
        } else if kind.is_file() || kind.is_symlink() {
            // Follows links, so the copy never points back into the source.
            if std::fs::metadata(&src).is_ok_and(|m| m.is_file()) {
                std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
            }
        }
    }
    Ok(())
}
