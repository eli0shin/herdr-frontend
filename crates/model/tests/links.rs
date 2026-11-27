//! Cmd-clicking a link (`feature-inventory.md` §17, Eli's `nvim-links`): URLs go to the system's
//! opener, paths are looked up on the pane's machine and open in Neovim beside the pane, or in
//! the opener.

mod support;

use base64::Engine as _;
use herdr_proto::ErrorKind;
use herdr_proto::methods::ExecResult;
use model::{Action, Effect, Event, Level, links};
use support::*;

fn one_pane() -> Core {
    Core::new(
        doc(vec![space("sp1", vec![tab("tb1", vec![pane("p1", "t1")])])]),
        vec![shell("t1", "/home/me/code/one")],
    )
}

/// `machine.exec` printing `stdout`, exiting 0.
fn printed(stdout: &str) -> Result<ExecResult, herdr_proto::RpcError> {
    Ok(ExecResult {
        code: Some(0),
        stdout: base64::engine::general_purpose::STANDARD.encode(stdout),
        ..Default::default()
    })
}

fn click(m: &mut model::Model, core: &mut Core, pane: &str, link: &str) -> Vec<Effect> {
    drive(
        m,
        core,
        Action::OpenLink {
            pane: pane.into(),
            link: link.into(),
        },
    )
}

#[test]
fn a_url_goes_to_the_opener() {
    let mut core = one_pane();
    let mut m = connected(&mut core);
    let fx = click(&mut m, &mut core, "p1", "https://example.com/x?a=1");
    assert!(fx.contains(&Effect::Open("https://example.com/x?a=1".into())));
    assert!(core.execs().is_empty());
}

/// Pi prints every path it reads or edits as an OSC 8 `file:` link: they open as paths do.
#[test]
fn a_file_uri_is_a_path_on_the_panes_machine() {
    let mut core = one_pane();
    let mut m = connected(&mut core);
    core.exec = printed("0\tf\t/home/me/code/one/src/main.rs\n");
    let fx = click(
        &mut m,
        &mut core,
        "p1",
        "file:///home/me/code/one/src/main%20x.rs",
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::Open(_))));
    let (argv, on) = core.execs().pop().unwrap();
    assert_eq!(on, None);
    assert_eq!(argv[5..], ["/home/me/code/one/src/main x.rs"]);
    let run = core.runs().pop().expect("Neovim in a split");
    assert_eq!(run.argv, ["nvim", "--", "/home/me/code/one/src/main.rs"]);

    // One on another host names nothing here.
    let calls = core.execs().len();
    let fx = click(&mut m, &mut core, "p1", "file://elsewhere/tmp/a.rs");
    assert!(!fx.iter().any(|e| matches!(e, Effect::Open(_))));
    assert_eq!(core.execs().len(), calls);
}

#[test]
fn a_source_file_opens_in_neovim_to_the_right_at_its_place() {
    let mut core = one_pane();
    let mut m = connected(&mut core);
    core.exec = printed("1\tf\t/home/me/code/one/src/main.rs\n");
    click(&mut m, &mut core, "p1", "src/main.rs:42:7");
    // Looked up here, in the pane's folder, as written and then bare.
    let (argv, on) = core.execs().pop().unwrap();
    assert_eq!(on, None);
    assert_eq!(argv[..3], ["sh", "-c", links::FIND]);
    assert_eq!(
        argv[3..],
        ["sh", "/home/me/code/one", "src/main.rs:42:7", "src/main.rs"]
    );
    let run = core.runs().pop().expect("a terminal for Neovim");
    assert_eq!(
        run.argv,
        [
            "nvim",
            "+call cursor(42, 7)",
            "--",
            "/home/me/code/one/src/main.rs"
        ]
    );
    assert_eq!(run.cwd.as_deref(), Some("/home/me/code/one"));
    let doc = core.doc(None);
    let panes: Vec<String> = doc
        .tab("tb1")
        .unwrap()
        .root
        .panes()
        .iter()
        .map(|n| n.id.clone())
        .collect();
    assert_eq!(panes.len(), 2);
    assert_eq!(
        panes[0], "p1",
        "the new pane is to the right of the clicked one"
    );
}

#[test]
fn folders_and_other_files_here_go_to_the_opener() {
    let mut core = one_pane();
    let mut m = connected(&mut core);
    core.exec = printed("0\td\t/home/me/code/one/src\n");
    let fx = click(&mut m, &mut core, "p1", "src/");
    assert!(fx.contains(&Effect::Open("/home/me/code/one/src".into())));
    core.exec = printed("0\tf\t/home/me/shot.png\n");
    let fx = click(&mut m, &mut core, "p1", "~/shot.png");
    assert!(fx.contains(&Effect::Open("/home/me/shot.png".into())));
    assert!(core.runs().is_empty());
}

#[test]
fn a_path_found_nowhere_opens_nothing_and_a_failed_lookup_says_so() {
    let mut core = one_pane();
    let mut m = connected(&mut core);
    core.exec = exited(1);
    let fx = click(&mut m, &mut core, "p1", "src/gone.rs");
    assert!(!fx.iter().any(|e| matches!(e, Effect::Open(_))));
    assert!(core.runs().is_empty());

    core.exec = Err(error(ErrorKind::Unavailable, "machine is gone"));
    let fx = click(&mut m, &mut core, "p1", "src/main.rs");
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Toast(t) if t.level == Level::Error && t.body == "machine is gone"
    )));
}

#[test]
fn a_remote_pane_looks_up_and_opens_there_with_no_opener() {
    let mut core = one_pane();
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![space("sp9", vec![tab("tb9", vec![pane("p9", "t9")])])]),
            4,
        ),
    );
    core.terminals
        .insert("devbox/t9".into(), shell("devbox/t9", "/srv"));
    let mut m = connected(&mut core);
    let mut devbox = herdr_proto::types::MachineView {
        state: herdr_proto::types::MachineState::Connected,
        ..Default::default()
    };
    devbox.machine.name = "devbox".into();
    drive(&mut m, &mut core, Event::Machines(vec![devbox]));

    core.exec = printed("0\tf\t/srv/app.py\n");
    click(&mut m, &mut core, "devbox/p9", "app.py");
    let (argv, on) = core.execs().pop().unwrap();
    assert_eq!(on.as_deref(), Some("devbox"));
    assert_eq!(argv[4], "/srv");
    let run = core.runs().pop().unwrap();
    assert_eq!(run.on.as_deref(), Some("devbox"));
    assert_eq!(run.argv, ["nvim", "--", "/srv/app.py"]);

    // A `file:` URI there names a file there.
    core.exec = printed("0\tf\t/srv/notes.md\n");
    click(&mut m, &mut core, "devbox/p9", "file:///srv/notes.md");
    let (argv, _) = core.execs().pop().unwrap();
    assert_eq!(argv[5..], ["/srv/notes.md"]);

    // Anything else there has no opener.
    core.exec = printed("0\tf\t/srv/plot.png\n");
    let runs = core.runs().len();
    let fx = click(&mut m, &mut core, "devbox/p9", "plot.png");
    assert!(!fx.iter().any(|e| matches!(e, Effect::Open(_))));
    assert_eq!(core.runs().len(), runs);
}
