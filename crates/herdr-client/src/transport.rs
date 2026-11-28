// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! How a connection reaches a core (`herdr-core-protocol.md` §1 "Transports").
//!
//! - [`HcCommand`]: an `hc connect` child, whose stdin and stdout carry the protocol. This is
//!   herdr-app's choice and the default: it starts the core when none runs, applies the session,
//!   `HC_SOCKET` and config `socket` rules, and on macOS repairs a core left from an earlier login.
//! - [`Transport::Socket`]: the core's Unix socket directly. There is no autostart and no login
//!   repair; [`socket_path`] reimplements the path rules.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use herdr_proto::Version;

use crate::error::{Error, StartupError};

/// Stderr kept from `hc connect`: enough for its startup error and any warnings.
const STDERR_KEEP: usize = 64 * 1024;

/// What a terminal that resumes programs (OSC 7577) gives its first process to sign reports with.
const RESUME_TOKEN: &str = "OSC7577_TOKEN";

/// Where a connection goes.
#[derive(Clone, Debug)]
pub enum Transport {
    /// An `hc connect` child process.
    Hc(HcCommand),
    /// The core's Unix socket.
    #[cfg(unix)]
    Socket(PathBuf),
}

/// How to run `hc connect`: which binary, and the environment changes it gets (an isolated
/// session for tests, say).
#[derive(Clone, Debug)]
pub struct HcCommand {
    program: PathBuf,
    session: Option<String>,
    env: Vec<(OsString, Option<OsString>)>,
}

impl HcCommand {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        HcCommand {
            program: program.into(),
            session: None,
            env: Vec::new(),
        }
    }

    /// The `hc` [`find_hc`] finds, else plain `hc` on `PATH`.
    pub fn found() -> Self {
        HcCommand::new(find_hc(None).unwrap_or_else(|| PathBuf::from("hc")))
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// `--session <name>`, like `HC_SESSION`.
    pub fn session(mut self, name: impl Into<String>) -> Self {
        self.session = Some(name.into());
        self
    }

    /// Sets a variable for the child.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), Some(value.into())));
        self
    }

    /// Removes a variable the child would inherit.
    pub fn env_remove(mut self, key: impl Into<OsString>) -> Self {
        self.env.push((key.into(), None));
        self
    }

    /// The command for `hc <args>` under this environment, such as `hc server stop` for an
    /// isolated test core.
    pub fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(&self.program);
        for (key, value) in &self.env {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        // The resume token of the terminal herdr-frontend was started from, if any, belongs to
        // that terminal alone (OSC 7577, `docs/research/osc-7577.md`). A core that `hc connect`
        // starts would hand it to every terminal it runs.
        command.env_remove(RESUME_TOKEN);
        command.args(args);
        command
    }

    fn connect(&self) -> Result<Opened, Error> {
        let mut command = self.command(["connect"]);
        if let Some(session) = &self.session {
            command.args(["--session", session]);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                Error::from(io::Error::new(
                    e.kind(),
                    format!(
                        "cannot run `{} connect` (is herdr-core installed?): {e}",
                        self.program.display()
                    ),
                ))
            })?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Protocol("hc connect has no pipes".into()));
        };
        let stderr = StderrLog::capture(stderr)?;
        Ok(Opened {
            read: Box::new(stdout),
            write: Box::new(stdin),
            closer: Box::new(ChildCloser(Some(child))),
            stderr: Some(stderr),
        })
    }
}

impl Transport {
    pub(crate) fn open(&self) -> Result<Opened, Error> {
        match self {
            Transport::Hc(hc) => hc.connect(),
            #[cfg(unix)]
            Transport::Socket(path) => {
                let stream = std::os::unix::net::UnixStream::connect(path).map_err(|e| {
                    Error::from(io::Error::new(
                        e.kind(),
                        format!("cannot reach the core at {}: {e}", path.display()),
                    ))
                })?;
                let read = stream.try_clone()?;
                let write = stream.try_clone()?;
                Ok(Opened {
                    read: Box::new(read),
                    write: Box::new(write),
                    closer: Box::new(SocketCloser(stream)),
                    stderr: None,
                })
            }
        }
    }
}

/// An open transport, before the handshake.
pub(crate) struct Opened {
    pub read: Box<dyn Read + Send>,
    pub write: Box<dyn Write + Send>,
    pub closer: Box<dyn Closer>,
    pub stderr: Option<StderrLog>,
}

/// Ends a transport so its reader sees the end of the stream.
pub(crate) trait Closer: Send {
    fn close(&mut self);
}

struct ChildCloser(Option<Child>);

impl Closer for ChildCloser {
    fn close(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ChildCloser {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(unix)]
struct SocketCloser(std::os::unix::net::UnixStream);

#[cfg(unix)]
impl Closer for SocketCloser {
    fn close(&mut self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

/// What `hc connect` writes on stderr, kept by its own thread (the startup error, warnings).
#[derive(Clone)]
pub(crate) struct StderrLog {
    text: Arc<Mutex<Vec<u8>>>,
    done: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
}

impl StderrLog {
    pub(crate) fn capture(mut stderr: impl Read + Send + 'static) -> io::Result<StderrLog> {
        let text = Arc::new(Mutex::new(Vec::new()));
        let (done_tx, done_rx) = mpsc::channel();
        let kept = text.clone();
        std::thread::Builder::new()
            .name("hc-stderr".into())
            .spawn(move || {
                let _done = done_tx;
                let mut buf = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut text = kept.lock().unwrap_or_else(PoisonError::into_inner);
                    let room = STDERR_KEEP.saturating_sub(text.len());
                    text.extend_from_slice(&buf[..n.min(room)]);
                }
            })?;
        Ok(StderrLog {
            text,
            done: Arc::new(Mutex::new(Some(done_rx))),
        })
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.text.lock().unwrap_or_else(PoisonError::into_inner))
            .into_owned()
    }

    /// The startup error, once stderr has closed (or `timeout` passed).
    pub fn startup_error(&self, timeout: Duration) -> Option<StartupError> {
        if let Some(done) = self
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = done.recv_timeout(timeout);
        }
        StartupError::parse(&self.text())
    }
}

/// `hc X.Y.Z` from `bin --version`, if `bin` is herdr-core and runs. Some window managers ship an
/// unrelated `hc`, which this rejects.
pub fn hc_version(bin: &Path) -> Option<Version> {
    let mut child = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let status = wait_with_deadline(&mut child, Duration::from_secs(5))?;
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let text = out.trim();
    (status.success() && text.starts_with("hc "))
        .then(|| Version::parse(text))
        .flatten()
}

/// Finds herdr-core the way herdr-app does (`app:src/core_bin.rs`): `configured` (a setting),
/// then this process's `PATH`, then the usual install places, then the login shell's `PATH`,
/// since an app opened from the Dock doesn't inherit the shell's. A candidate counts only if
/// [`hc_version`] accepts it.
pub fn find_hc(configured: Option<&Path>) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let pick = |candidates: Vec<PathBuf>| {
        candidates
            .into_iter()
            .filter(|c| c.is_file())
            .find(|c| hc_version(c).is_some())
    };
    if let Some(configured) = configured {
        let configured = match (configured.strip_prefix("~"), &home) {
            (Ok(rest), Some(home)) => home.join(rest),
            _ => configured.to_path_buf(),
        };
        if let Some(found) = pick(vec![configured]) {
            return Some(found);
        }
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let quick: Vec<PathBuf> = std::env::split_paths(&path)
        .chain(home.as_deref().map(usual_dirs).unwrap_or_default())
        .flat_map(|dir| names(&dir))
        .collect();
    pick(quick).or_else(|| {
        let login = login_shell_path()?;
        pick(
            std::env::split_paths(&login)
                .flat_map(|dir| names(&dir))
                .collect(),
        )
    })
}

fn names(dir: &Path) -> [PathBuf; 2] {
    [dir.join("herdr-core"), dir.join("hc")]
}

/// herdr-core's own installer (`~/.local/bin`), Homebrew, mise, Nix, cargo, and the macOS bundle.
fn usual_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".local/share/mise/shims"),
        home.join(".nix-profile/bin"),
        PathBuf::from("/run/current-system/sw/bin"),
        home.join(".cargo/bin"),
    ];
    if cfg!(target_os = "macos") {
        dirs.push(home.join(".local/share/herdr-core/current/Contents/MacOS"));
        dirs.push(home.join(".local/share/herdr-core/Herdr Core.app/Contents/MacOS"));
    }
    dirs
}

/// The `PATH` the login shell sets up, or `None` if it doesn't answer within 3 s.
fn login_shell_path() -> Option<OsString> {
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    let mut command = Command::new(shell);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Shells can open /dev/tty even with stdin redirected: in a session of its own, the shell
        // can't stop this process's group when it runs as a background job (as herdr-app's
        // `spawn::detached_command` does).
        // SAFETY: setsid is async-signal-safe and runs only in the new child.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command
        .args(["-lc", "printf 'herdr-path[%s]herdr-path' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    wait_with_deadline(&mut child, Duration::from_secs(3))?;
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    login_path_answer(&out)
}

/// The `PATH` inside the probe's frame (`app:src/core_bin.rs` `login_path_answer`): a chatty
/// profile prints around it, and a shell that doesn't speak sh (nushell) answers an error,
/// nothing, or `$PATH` literally.
fn login_path_answer(out: &str) -> Option<OsString> {
    let (_, rest) = out.split_once("herdr-path[")?;
    let (path, _) = rest.rsplit_once("]herdr-path")?;
    (!path.is_empty() && path != "$PATH").then(|| path.into())
}

/// Waits for a short-lived child, killing it at the deadline.
fn wait_with_deadline(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().ok()? {
            return Some(status);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The core's client socket for this environment (`core:crates/herdr-core/src/paths.rs`,
/// `core:docs/security.md` "State and endpoints"): `HC_SOCKET`, else `socket` in the core's
/// config, else `<runtime>/herdr-core/<session>.sock`, where the runtime dir is
/// `XDG_RUNTIME_DIR`, else `/run/user/<uid>` if it exists, else the session's state directory.
#[cfg(unix)]
pub fn socket_path() -> Result<PathBuf, Error> {
    socket_path_in(&|key| std::env::var_os(key).filter(|v| !v.is_empty()))
}

#[cfg(unix)]
fn socket_path_in(var: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    let session = var("HC_SESSION")
        .map(|s| s.to_string_lossy().trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".into());
    if session == "."
        || session == ".."
        || session
            .chars()
            .any(|c| c.is_control() || r#"/\:*?"<>|"#.contains(c))
    {
        return Err(Error::Protocol(format!(
            "HC_SESSION {session:?} can't be a folder name"
        )));
    }
    let home = var("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    let xdg = |key: &str, fallback: PathBuf| var(key).map_or(fallback, PathBuf::from);
    let state = xdg("XDG_STATE_HOME", home.join(".local/state"))
        .join("herdr-core")
        .join(&session);
    let config = xdg("XDG_CONFIG_HOME", home.join(".config"))
        .join("herdr-core")
        .join("config.toml");
    if let Some(socket) = var("HC_SOCKET") {
        return Ok(PathBuf::from(socket));
    }
    if let Some(socket) = config_socket(&config) {
        return Ok(socket);
    }
    let run = var("XDG_RUNTIME_DIR").map(PathBuf::from).or_else(|| {
        // SAFETY: getuid has no preconditions and can't fail.
        let uid = unsafe { libc::getuid() };
        Some(PathBuf::from(format!("/run/user/{uid}"))).filter(|p| p.is_dir())
    });
    let dir = run.map_or(state, |run| run.join("herdr-core"));
    Ok(dir.join(format!("{session}.sock")))
}

/// `socket` in the core's config file, if it is there and readable.
#[cfg(unix)]
fn config_socket(config: &Path) -> Option<PathBuf> {
    #[derive(serde::Deserialize)]
    struct Config {
        socket: Option<String>,
    }
    let text = std::fs::read_to_string(config).ok()?;
    let config: Config = toml::from_str(&text).ok()?;
    config.socket.filter(|s| !s.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn path_with(vars: &[(&str, &str)]) -> PathBuf {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        socket_path_in(&|key| vars.get(key).cloned()).unwrap()
    }

    #[test]
    fn the_socket_follows_the_cores_rules() {
        assert_eq!(
            path_with(&[("HOME", "/h"), ("XDG_RUNTIME_DIR", "/run/me")]),
            PathBuf::from("/run/me/herdr-core/default.sock")
        );
        assert_eq!(
            path_with(&[
                ("HOME", "/h"),
                ("HC_SESSION", " test-x "),
                ("XDG_RUNTIME_DIR", "/r")
            ]),
            PathBuf::from("/r/herdr-core/test-x.sock")
        );
        assert_eq!(
            path_with(&[
                ("HOME", "/h"),
                ("HC_SOCKET", "/s.sock"),
                ("XDG_RUNTIME_DIR", "/r")
            ]),
            PathBuf::from("/s.sock")
        );
        if !Path::new(&format!("/run/user/{}", unsafe { libc::getuid() })).is_dir() {
            assert_eq!(
                path_with(&[("HOME", "/h"), ("XDG_STATE_HOME", "/st")]),
                PathBuf::from("/st/herdr-core/default/default.sock")
            );
        }
        let vars = |key: &str| (key == "HC_SESSION").then(|| OsString::from("a/b"));
        assert!(socket_path_in(&vars).is_err());
    }

    #[test]
    fn the_config_socket_comes_before_the_runtime_dir() {
        let dir = std::env::temp_dir().join(format!("hfc-config-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("herdr-core")).unwrap();
        std::fs::write(
            dir.join("herdr-core/config.toml"),
            "headless = \"80x24\"\nsocket = \"/tmp/x.sock\"\n",
        )
        .unwrap();
        let config = dir.to_str().unwrap();
        assert_eq!(
            path_with(&[
                ("HOME", "/h"),
                ("XDG_CONFIG_HOME", config),
                ("XDG_RUNTIME_DIR", "/r")
            ]),
            PathBuf::from("/tmp/x.sock")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_login_shell_answer_is_the_path_inside_the_frame() {
        for path in [
            "/opt/homebrew/bin:/usr/bin::/bin",
            ".:/custom/agents/bin:/usr/bin",
            "~/bin:/opt/$x/bin:bin",
            ":/usr/bin:",
            ":",
        ] {
            let framed = format!("Welcome!\nherdr-path[{path}]herdr-path\nbye\n");
            assert_eq!(
                login_path_answer(&framed).as_deref(),
                Some(OsStr::new(path)),
                "{path:?}"
            );
        }
        // What nushell answers, a shell that printed nothing, and an unexpanded PATH.
        for garbage in [
            "Error: nu::parser::missing_flag_param",
            "",
            "/usr/bin:/bin",
            "herdr-path[]herdr-path",
            "herdr-path[$PATH]herdr-path",
        ] {
            assert_eq!(login_path_answer(garbage), None, "{garbage:?}");
        }
    }

    #[test]
    fn a_binary_that_is_not_hc_is_rejected() {
        assert_eq!(hc_version(Path::new("/bin/echo")), None);
        assert_eq!(hc_version(Path::new("/nonexistent/hc")), None);
    }

    #[test]
    fn hc_commands_carry_their_environment() {
        let hc = HcCommand::new("/x/hc")
            .env("HC_SESSION", "test-a")
            .env_remove("HC_SOCKET");
        let command = hc.command(["server", "stop"]);
        let envs: Vec<_> = command.get_envs().collect();
        assert!(envs.contains(&(OsStr::new("HC_SESSION"), Some(OsStr::new("test-a")))));
        assert!(envs.contains(&(OsStr::new("HC_SOCKET"), None)));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("server"), OsStr::new("stop")]
        );
    }

    #[test]
    fn hc_never_gets_a_resume_token() {
        let hc = HcCommand::new("/x/hc").env("OSC7577_TOKEN", "from-the-outer-terminal");
        let command = hc.command(["connect"]);
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(
            envs.iter().find(|(key, _)| *key == "OSC7577_TOKEN"),
            Some(&(OsStr::new("OSC7577_TOKEN"), None))
        );
    }
}
