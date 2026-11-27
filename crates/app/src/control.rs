//! The control socket and `herdr-frontend ls | do | status | commands` (`docs/cli.md`): Eli's
//! tools drive the running window the way they drive `herdr-app`, with the same arguments, JSON
//! and exit codes.
//!
//! The window serves `$HERDR_FRONTEND_HOME/herdr-frontend.sock` ([`Control`]), one JSON line
//! each way, as herdr-app serves `herdr-app.sock` (`app:src/control.rs`). A thread per connection
//! reads the request and hands it to the UI thread; [`Control::drain`] runs it against the model
//! there, so the model stays the one place state changes. [`client`] is the command line side.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use model::{Call, CallId, Effect, Effects, Model};
use serde_json::{Map, Value, json};

/// Picks another home, as `HERDR_APP_HOME` does for herdr-app.
pub const HOME_VAR: &str = "HERDR_FRONTEND_HOME";
/// The socket's file name in the home.
pub const SOCKET: &str = "herdr-frontend.sock";
/// How long a connection waits for the window's answer (herdr-app's 5 s).
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the client waits for the socket's reply: the window's timeout and some.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a connection may take to send its request line.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The subcommands [`client`] runs instead of opening the window.
pub const SUBCOMMANDS: [&str; 4] = ["ls", "do", "status", "commands"];

const USAGE: &str = "usage: herdr-frontend ls | herdr-frontend commands | herdr-frontend status | \
                     herdr-frontend do <command> [--arg value ... | json]";

/// herdr-frontend's home: `$HERDR_FRONTEND_HOME`, else `$XDG_CONFIG_HOME/herdr-frontend`, else
/// `~/.config/herdr-frontend`, as herdr-app finds its own (`app:src/mods.rs` `app_dir`). Empty
/// variables count as unset.
pub fn home() -> PathBuf {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(home) = var(HOME_VAR) {
        return PathBuf::from(home);
    }
    var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(var("HOME").unwrap_or_default()).join(".config"))
        .join("herdr-frontend")
}

/// The window's control socket.
pub fn socket_path() -> PathBuf {
    home().join(SOCKET)
}

/// One request from a connection, and where its reply goes.
struct Request {
    cmd: Value,
    reply: mpsc::Sender<Value>,
}

/// A harness request (`crate::e2e`) for the window to answer itself: keys, its tree or its
/// window number.
pub struct Probe {
    pub cmd: Value,
    reply: mpsc::Sender<Value>,
}

impl Probe {
    /// Sends the answer to the connection waiting for it, if it still is.
    pub fn answer(self, reply: Value) {
        let _ = self.reply.send(reply);
    }
}

/// The window's end of the control socket: it listens while alive and removes the socket when
/// dropped.
pub struct Control {
    path: PathBuf,
    requests: mpsc::Receiver<Request>,
    /// `do` calls waiting for their [`Effect::Answer`].
    waiting: BTreeMap<CallId, mpsc::Sender<Value>>,
    next: u64,
    stop: Arc<AtomicBool>,
    /// Whether harness requests are served ([`Control::with_probes`]), and those waiting.
    probes: Option<Vec<Probe>>,
}

impl Control {
    /// Serves `path`. `wake` is called from a connection's thread when a request arrives, so the
    /// UI thread drains it on its next frame (Rune's waker). Fails when another window answers
    /// on `path`: one window per home.
    pub fn listen(
        path: impl Into<PathBuf>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> io::Result<Control> {
        let path = path.into();
        if UnixStream::connect(&path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("another herdr-frontend window serves {}", path.display()),
            ));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Nobody answers on it: a socket left by a window that didn't close cleanly.
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let listener = UnixListener::bind(&path)?;
        let (tx, requests) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let stopping = stop.clone();
        std::thread::Builder::new()
            .name("herdr-frontend-control".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let (tx, wake) = (tx.clone(), wake.clone());
                    std::thread::spawn(move || serve(stream, &tx, &*wake));
                }
            })?;
        Ok(Control {
            path,
            requests,
            waiting: BTreeMap::new(),
            next: 0,
            stop,
            probes: None,
        })
    }

    /// Serves the harness requests too (`crate::e2e`): [`Control::drain`] queues them for
    /// [`Control::take_probes`] instead of refusing them.
    pub fn with_probes(mut self) -> Control {
        self.probes = Some(Vec::new());
        self
    }

    /// The harness requests that came in since the last call, for the window to answer.
    pub fn take_probes(&mut self) -> Vec<Probe> {
        self.probes.as_mut().map(std::mem::take).unwrap_or_default()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Runs the requests that arrived, on the UI thread. `ls` and `status` read `model`; a `do`
    /// steps it with a [`Call`] at app clock `now`. `window` is what only the window knows for
    /// `status` (`zoom`, `font_size`, `ui_font_size`, …): its keys go in as given, except
    /// `errors`, which go before the model's.
    ///
    /// Returns the steps' effects for the app to carry out, or `None` when no request came in
    /// (so nothing changed). Answers among them are sent here and left out; one that comes in a
    /// later step (a `space.new` waiting for its terminal) goes to [`Control::answer`]. Harness
    /// requests wait in [`Control::take_probes`] when served.
    pub fn drain(
        &mut self,
        model: &mut Model,
        now: f64,
        window: &Map<String, Value>,
    ) -> Option<Effects> {
        let mut fx = Effects {
            list: Vec::new(),
            handled: true,
        };
        let mut any = false;
        while let Ok(Request { cmd, reply }) = self.requests.try_recv() {
            any = true;
            if let Some(name) = cmd.get("do").and_then(Value::as_str) {
                self.next += 1;
                let id = CallId(self.next);
                self.waiting.insert(id, reply);
                let call = Call {
                    id,
                    name: name.to_owned(),
                    args: cmd.get("args").cloned().unwrap_or(Value::Null),
                };
                for effect in model.step(now, call) {
                    match effect {
                        Effect::Answer { id, reply } => self.answer(id, reply),
                        other => fx.push(other),
                    }
                }
                continue;
            }
            if let Some(probes) = &mut self.probes
                && crate::e2e::is_request(&cmd)
            {
                probes.push(Probe { cmd, reply });
                continue;
            }
            let answer = if cmd.get("ls").is_some() {
                model.ls()
            } else if cmd.get("status").is_some() {
                Value::Object(status(model, window))
            } else if cmd.get("commands").is_some() {
                model::control::listing()
            } else {
                json!({ "error": "expected { do }, { ls }, { commands } or { status }" })
            };
            let _ = reply.send(answer);
        }
        any.then_some(fx)
    }

    /// Sends a call's answer ([`Effect::Answer`]) to the connection waiting for it. A connection
    /// that gave up is gone already; its answer is dropped.
    pub fn answer(&mut self, id: CallId, reply: Value) {
        if let Some(waiting) = self.waiting.remove(&id) {
            let _ = waiting.send(reply);
        }
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        // Wake the accept loop so its thread ends, then take the socket away.
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `status`: the model's, with the window's own facts.
fn status(model: &Model, window: &Map<String, Value>) -> Map<String, Value> {
    let mut status = model.status();
    for (key, value) in window {
        if key == "errors"
            && let (Some(Value::Array(model_errors)), Value::Array(errors)) =
                (status.get("errors"), value)
        {
            let all: Vec<Value> = errors.iter().chain(model_errors).cloned().collect();
            status.insert(key.clone(), Value::Array(all));
        } else {
            status.insert(key.clone(), value.clone());
        }
    }
    status
}

/// One connection: a request line in, the window's answer line out.
fn serve(stream: UnixStream, tx: &mpsc::Sender<Request>, wake: &(dyn Fn() + Send + Sync)) {
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let answer = match serde_json::from_str::<Value>(&line) {
        Ok(cmd) => {
            let (reply, answer) = mpsc::channel();
            if tx.send(Request { cmd, reply }).is_err() {
                return;
            }
            wake();
            answer
                .recv_timeout(ANSWER_TIMEOUT)
                .unwrap_or_else(|_| json!({ "error": "the window did not answer" }))
        }
        Err(e) => json!({ "error": format!("bad request: {e}") }),
    };
    let _ = (&stream).write_all(format!("{answer}\n").as_bytes());
}

/// Whether these arguments (after the program name) are a control subcommand, not a window.
pub fn is_subcommand(args: &[String]) -> bool {
    args.first()
        .is_some_and(|a| SUBCOMMANDS.contains(&a.as_str()))
}

/// `herdr-frontend ls | do | status | commands`, against the window serving [`socket_path`].
/// Prints the reply as pretty JSON and returns the exit code: 0, 1 for an `{error}` reply, and 2
/// for a usage error or no window (herdr-app's codes; its "no Herdr window at" message, which
/// tools match).
pub fn client(args: &[String]) -> i32 {
    client_at(&socket_path(), args, &mut io::stdout(), &mut io::stderr())
}

fn client_at(socket: &Path, args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let mut fail = |message: &str| {
        let _ = writeln!(err, "herdr-frontend: {message}");
        2
    };
    if args == ["commands"] {
        let listing = model::control::listing();
        let _ = writeln!(out, "{}", pretty(&listing));
        return 0;
    }
    let cmd = match args {
        [verb] if verb == "ls" => json!({ "ls": true }),
        [verb] if verb == "status" => json!({ "status": true }),
        [verb, command, rest @ ..] if verb == "do" => match parse_args(rest) {
            Ok(args) => json!({ "do": command, "args": args }),
            Err(e) => return fail(&e),
        },
        _ => return fail(USAGE),
    };
    let mut stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(e) => return fail(&format!("no Herdr window at {}: {e}", socket.display())),
    };
    let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
    let mut line = String::new();
    if stream.write_all(format!("{cmd}\n").as_bytes()).is_err()
        || BufReader::new(&stream).read_line(&mut line).is_err()
        || line.is_empty()
    {
        return fail("the window hung up");
    }
    let reply: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    let _ = writeln!(out, "{}", pretty(&reply));
    if reply.get("error").is_some() { 1 } else { 0 }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// `--name api --argv '["pi"]'` (a value that parses as JSON other than a string is that JSON),
/// or one JSON object (`app:src/control.rs` `parse_args`).
fn parse_args(rest: &[String]) -> Result<Value, String> {
    match rest {
        [] => Ok(Value::Null),
        [raw] if raw.trim_start().starts_with('{') => {
            serde_json::from_str(raw).map_err(|e| format!("args are not JSON: {e}"))
        }
        _ => {
            let mut out = Map::new();
            let mut it = rest.iter();
            while let Some(flag) = it.next() {
                let key = flag
                    .strip_prefix("--")
                    .ok_or_else(|| format!("expected --<arg>, got {flag:?}"))?;
                let value = it.next().ok_or_else(|| format!("--{key} needs a value"))?;
                let v = serde_json::from_str::<Value>(value)
                    .ok()
                    .filter(|v| !v.is_string())
                    .unwrap_or_else(|| Value::String(value.clone()));
                out.insert(key.to_owned(), v);
            }
            Ok(Value::Object(out))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flag_values_that_parse_as_json_are_json() {
        let parsed = parse_args(&args(&[
            "--name",
            "eng-123",
            "--focus",
            "false",
            "--argv",
            r#"["claude"]"#,
            "--index",
            "3",
            "--id",
            "\"7\"",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            json!({"name": "eng-123", "focus": false, "argv": ["claude"], "index": 3, "id": "\"7\""})
        );
        assert_eq!(
            parse_args(&args(&[r#"{"id": "sp1", "confirmed": true}"#])).unwrap(),
            json!({"id": "sp1", "confirmed": true})
        );
        assert_eq!(parse_args(&[]).unwrap(), Value::Null);
    }

    #[test]
    fn malformed_flags_are_usage_errors() {
        assert_eq!(
            parse_args(&args(&["name", "x"])),
            Err("expected --<arg>, got \"name\"".into())
        );
        assert_eq!(
            parse_args(&args(&["--name"])),
            Err("--name needs a value".into())
        );
        assert!(
            parse_args(&args(&["{nope"]))
                .unwrap_err()
                .starts_with("args are not JSON")
        );
    }

    #[test]
    fn the_window_facts_go_into_status_and_their_errors_first() {
        let window: Map<String, Value> = serde_json::from_value(
            json!({"zoom": 1.1, "font_size": 14.0, "errors": ["keys: bad chord"]}),
        )
        .unwrap();
        let model = Model::new("/home/me");
        let status = status(&model, &window);
        assert_eq!(status["zoom"], 1.1);
        assert_eq!(status["layout"], "eli");
        assert_eq!(status["errors"], json!(["keys: bad chord"]));
    }

    #[test]
    fn only_the_four_subcommands_skip_the_window() {
        for verb in SUBCOMMANDS {
            assert!(is_subcommand(&args(&[verb])));
        }
        for other in [&["--snap", "x.png"][..], &["shot"], &[]] {
            assert!(!is_subcommand(&args(other)), "{other:?}");
        }
    }

    #[test]
    fn usage_errors_and_a_missing_window_exit_2_on_stderr() {
        let socket = Path::new("/tmp/herdr-frontend-no-such.sock");
        for (list, want) in [
            (&["ls", "extra"][..], USAGE.to_owned()),
            (&["do"], USAGE.to_owned()),
            (
                &["do", "space.new", "name"],
                "expected --<arg>, got \"name\"".into(),
            ),
            (
                &["ls"],
                format!("no Herdr window at {}: ", socket.display()),
            ),
        ] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(client_at(socket, &args(list), &mut out, &mut err), 2);
            let err = String::from_utf8(err).unwrap();
            assert!(err.starts_with(&format!("herdr-frontend: {want}")), "{err}");
            assert!(out.is_empty());
        }
    }

    #[test]
    fn commands_print_without_a_window() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let socket = Path::new("/tmp/herdr-frontend-no-such.sock");
        assert_eq!(
            client_at(socket, &args(&["commands"]), &mut out, &mut err),
            0
        );
        let listed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(listed, model::control::listing());
    }
}
