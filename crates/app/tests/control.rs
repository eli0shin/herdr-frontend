//! `herdr-frontend ls | do | status` end to end: the real binary as the client, a [`Control`]
//! serving an isolated home, and a model on the model crate's fake core behind it, pumped the
//! way the window's frame loop will: drain the socket, carry out the effects, send the answers.

#[path = "../../model/tests/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use herdr_frontend::control::{Control, HOME_VAR, SOCKET};
use model::{Effect, Model};
use serde_json::{Map, Value, json};
use support::*;

/// macOS has no `SOCK_CLOEXEC`: a socket is made close-on-exec just after it is created, so a
/// child another test spawns in between keeps it open (and a closed socket listening). Sockets
/// are made and children spawned under this lock.
static FDS: Mutex<()> = Mutex::new(());

fn fds() -> MutexGuard<'static, ()> {
    FDS.lock().unwrap_or_else(|e| e.into_inner())
}

fn listen(path: &Path) -> std::io::Result<Control> {
    let _fds = fds();
    Control::listen(path, || {})
}

/// A scratch home under `/tmp` (socket paths must stay short), removed when dropped.
struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Home {
        let dir = PathBuf::from(format!("/tmp/hf-ctl-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Home(dir)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A window: the control socket and the model behind it.
struct Window {
    control: Control,
    model: Model,
    core: Core,
    facts: Map<String, Value>,
}

impl Window {
    fn open(home: &Home) -> Window {
        let mut core = Core::new(
            doc(vec![
                space("sp1", vec![tab("tb1", vec![pane("p1", "t1")])]),
                space(
                    "sp2",
                    vec![
                        tab("tb2", vec![pane("p2", "t2"), pane("p3", "t3")]),
                        tab("tb3", vec![pane("p4", "t4")]),
                    ],
                ),
            ]),
            vec![
                shell("t1", "/home/me/code/one"),
                agent("t2", herdr_proto::StatusState::Idle),
                shell("t3", "/home/me/code/x"),
                shell("t4", "/home/me/code/x"),
            ],
        );
        let model = connected(&mut core);
        let control = listen(&home.0.join(SOCKET)).unwrap();
        let facts = serde_json::from_value(json!({"zoom": 1.0, "font_size": 14.0})).unwrap();
        Window {
            control,
            model,
            core,
            facts,
        }
    }

    /// One frame: requests in, effects carried out, answers out.
    fn pump(&mut self) {
        let fx = self
            .control
            .drain(&mut self.model, self.core.now, &self.facts);
        for effect in run_effects(&mut self.model, &mut self.core, fx.list) {
            if let Effect::Answer { id, reply } = effect {
                self.control.answer(id, reply);
            }
        }
    }

    /// Runs `herdr-frontend <args>` against this window's home, pumping until it exits.
    fn run(&mut self, home: &Home, args: &[&str]) -> Output {
        run_in(home, args, || self.pump())
    }
}

fn run_in(home: &Home, args: &[&str], pump: impl FnMut()) -> Output {
    run_env(&[(HOME_VAR, Some(&home.0))], args, pump)
}

/// Runs `herdr-frontend <args>` with these variables set (`None`: unset), pumping until it
/// exits.
fn run_env(vars: &[(&str, Option<&Path>)], args: &[&str], mut pump: impl FnMut()) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-frontend"));
    for (name, value) in vars {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    let mut child = {
        let _fds = fds();
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "herdr-frontend {args:?} hangs");
        pump();
        std::thread::sleep(Duration::from_millis(2));
    }
    child.wait_with_output().unwrap()
}

fn json_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stdout)))
}

#[test]
fn ls_prints_the_models_listing() {
    let home = Home::new("ls");
    let mut window = Window::open(&home);
    let out = window.run(&home, &["ls"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(json_of(&out), window.model.ls());
    assert_eq!(
        json_of(&out)["spaces"][1]["tabs"][0]["terminals"][0]["status"],
        "idle"
    );
}

#[test]
fn space_new_without_focus_answers_its_ids_once_its_terminal_runs() {
    let home = Home::new("new");
    let mut window = Window::open(&home);
    let out = window.run(
        &home,
        &[
            "do",
            "space.new",
            "--name",
            "repo@branch",
            "--folder",
            "/home/me/code/repo",
            "--focus",
            "false",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let reply = json_of(&out);
    let space = reply["space"].as_str().unwrap();
    assert_eq!(
        window.core.doc(None).space(space).unwrap().name,
        "repo@branch"
    );
    assert_eq!(window.model.active_space().unwrap().id, "sp1");
    let listed = json_of(&window.run(&home, &["ls"]));
    let made = &listed["spaces"][2];
    assert_eq!(made["id"], space);
    assert_eq!(made["tabs"][0]["id"], reply["tab"]);
    assert_eq!(
        made["tabs"][0]["terminals"][0]["terminal"],
        reply["terminal"]
    );
}

#[test]
fn do_takes_one_json_object_and_errors_exit_1_on_stdout() {
    let home = Home::new("json");
    let mut window = Window::open(&home);
    let out = window.run(
        &home,
        &["do", "space.rename", r#"{"id":"sp2","name":"api"}"#],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(json_of(&out), json!({"ok": true}));
    assert_eq!(window.core.doc(None).space("sp2").unwrap().name, "api");

    let out = window.run(&home, &["do", "space.close", "--id", "sp2"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        json_of(&out),
        json!({"error": "space.close ends its terminals: pass confirmed: true"})
    );
    let out = window.run(
        &home,
        &["do", "space.close", "--id", "sp2", "--confirmed", "true"],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(window.core.doc(None).space("sp2").is_none());
    let mut kills = window.core.kills();
    kills.sort();
    assert_eq!(kills, ["t2", "t3", "t4"]);
}

#[test]
fn the_other_commands_tools_use_run_through_the_socket() {
    let home = Home::new("tools");
    let mut window = Window::open(&home);
    for args in [
        &["do", "agent.focus", "--terminal", "t2"][..],
        &["do", "tab.rename", "--id", "tb3", "--name", "Fix login"],
        &["do", "pane.close", "--pane", "p3"],
        &["do", "space.select", "--id", "sp1"],
        &["do", "layout.switch", "--name", "eli"],
    ] {
        let out = window.run(&home, args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {out:?}");
        assert_eq!(json_of(&out), json!({"ok": true}), "{args:?}");
    }
    assert_eq!(window.core.doc(None).tab("tb3").unwrap().name, "Fix login");
    assert_eq!(window.core.kills(), ["t3"]);
    assert_eq!(window.model.active_space().unwrap().id, "sp1");
    let out = window.run(&home, &["do", "space.select", "--index", "2"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        window.model.focused_pane().as_deref(),
        Some("p2"),
        "agent.focus"
    );
}

#[test]
fn status_has_the_models_view_and_the_windows_facts() {
    let home = Home::new("status");
    let mut window = Window::open(&home);
    let out = window.run(&home, &["status"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        json_of(&out),
        json!({
            "zoom": 1.0, "font_size": 14.0, "space": "sp1", "tab": "tb1", "focused_pane": "p1",
            "layout": "eli", "agent": "t2", "floats": {}, "overlays": [], "errors": [],
        })
    );
}

#[test]
fn without_a_window_the_cli_says_so_and_exits_2() {
    let home = Home::new("none");
    for args in [
        &["ls"][..],
        &["status"],
        &["do", "space.select", "--index", "1"],
    ] {
        let out = run_in(&home, args, || {});
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        let socket = home.0.join(SOCKET);
        assert!(
            err.starts_with(&format!(
                "herdr-frontend: no Herdr window at {}",
                socket.display()
            )),
            "{err}"
        );
        assert!(out.stdout.is_empty());
    }
    let out = run_in(&home, &["commands"], || {});
    assert_eq!(out.status.code(), Some(0), "commands need no window");
    assert_eq!(json_of(&out), model::control::listing());
}

#[test]
fn one_window_serves_a_home_and_leaves_no_socket_behind() {
    let home = Home::new("one");
    let socket = home.0.join(SOCKET);
    let first = listen(&socket).unwrap();
    let second = listen(&socket);
    assert_eq!(
        second.err().map(|e| e.kind()),
        Some(std::io::ErrorKind::AddrInUse)
    );
    drop(first);
    assert!(!socket.exists(), "the socket goes with its window");
    // A socket left by a window that died is taken over.
    let stale = {
        let _fds = fds();
        std::os::unix::net::UnixListener::bind(&socket).unwrap()
    };
    drop(stale);
    assert!(socket.exists());
    let again = listen(&socket);
    assert!(again.is_ok(), "{:?}", again.err());
}

#[test]
fn a_request_wakes_the_window_and_a_late_answer_times_out_politely() {
    let home = Home::new("wake");
    let woken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = woken.clone();
    let _control = {
        let _fds = fds();
        Control::listen(home.0.join(SOCKET), move || {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .unwrap()
    };
    // Nobody drains: the connection gives up after the answer timeout.
    let out = run_in(&home, &["ls"], || {});
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(json_of(&out), json!({"error": "the window did not answer"}));
    assert_eq!(woken.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn the_cli_finds_the_home_as_herdr_app_does() {
    let scratch = Home::new("xdg");
    let xdg = scratch.0.join("xdg");
    let user = scratch.0.join("user");
    // `$HERDR_FRONTEND_HOME` (empty: unset), then `$XDG_CONFIG_HOME/herdr-frontend`, then
    // `~/.config/herdr-frontend`.
    let empty = Path::new("");
    for (vars, home) in [
        (
            vec![
                (HOME_VAR, Some(empty)),
                ("XDG_CONFIG_HOME", Some(xdg.as_path())),
            ],
            xdg.join("herdr-frontend"),
        ),
        (
            vec![
                (HOME_VAR, None),
                ("XDG_CONFIG_HOME", Some(empty)),
                ("HOME", Some(user.as_path())),
            ],
            user.join(".config/herdr-frontend"),
        ),
    ] {
        // Inside the scratch home, so it goes with it; the window closes first.
        let home = Home(home);
        let mut window = Window::open(&home);
        let out = run_env(&vars, &["ls"], || window.pump());
        assert_eq!(out.status.code(), Some(0), "{vars:?}: {out:?}");
        assert_eq!(json_of(&out), window.model.ls());
    }
}
