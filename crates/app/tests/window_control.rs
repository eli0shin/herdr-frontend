//! The whole app serving `herdr-frontend ls | do | status` (`docs/cli.md`): the app as the window
//! runs it ([`live`], its home and core link), on a real, isolated herdr-core and an isolated
//! home, driven by the real binary as the CLI. Every command the doc lists goes through the
//! window's frame loop: the socket's request into the model, the effects out to the core, and
//! `space.new`'s answer once its terminal runs.
//!
//! The core is a `test-*` session with scratch dirs under `/tmp` (herdr-client's test support),
//! never the person's. Without an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;
mod window;

use herdr_client::{ConnectOptions, Connection};
use herdr_frontend::control::SOCKET;
use herdr_frontend::fixtures::{pane, split, tab};
use serde_json::json;
use support::IsolatedCore;
use window::{
    Scratch, cli, cli_without_window, fails, is_live, json_of, layout_until, ok, open, run,
    set_layout, space_in, status, until,
};

#[test]
fn the_window_serves_every_cli_command_on_a_live_core() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let (a, b, c, d) = (
        run(&conn, &core),
        run(&conn, &core),
        run(&conn, &core),
        run(&conn, &core),
    );
    // Another client (Herdr app, say) wrote the layout: `work` with two tabs, and `api`.
    let folder = core.folder().display().to_string();
    let doc = json!({"spaces": [
        {"id": "sp-1", "name": "work", "folder": folder, "tabs": [
            tab("tb-1", "1", split("tb-1", "row", vec![pane("pn-a", &a), pane("pn-b", &b)])),
            tab("tb-2", "2", split("tb-2", "row", vec![pane("pn-c", &c)])),
        ]},
        {"id": "sp-2", "name": "api", "folder": folder, "tabs": [
            tab("tb-3", "1", split("tb-3", "row", vec![pane("pn-d", &d)])),
        ]},
    ]});
    set_layout(&conn, doc);

    let scratch = Scratch::new("win");
    let home = scratch.0.as_path();
    let socket = home.join(SOCKET);
    let mut rt = open(&core, home);
    until(&mut rt, "the layout and its terminals", |rt| {
        let world = rt.state.model.world();
        [&a, &b, &c, &d].iter().all(|t| world.is_live(t))
            && rt.state.model.ls()["spaces"][1]["id"] == "sp-2"
    });

    // `ls`: the window's listing.
    let out = cli(&mut rt, home, &["ls"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let listed = json_of(&out);
    assert_eq!(listed, rt.state.model.ls());
    let spaces = listed["spaces"].as_array().unwrap();
    assert_eq!(spaces.len(), 2);
    assert_eq!(spaces[0]["name"], "work");
    assert_eq!(spaces[0]["tabs"][0]["terminals"][1]["terminal"], b.as_str());

    // `status`: the model's view and the window's own facts.
    let shown = status(&mut rt, home);
    assert_eq!(shown["space"], "sp-1");
    assert_eq!(shown["tab"], "tb-1");
    assert_eq!(shown["focused_pane"], "pn-a");
    assert_eq!(shown["layout"], "eli");
    assert_eq!(shown["errors"], json!([]));
    assert_eq!(
        (&shown["zoom"], &shown["font_size"], &shown["ui_font_size"]),
        (&json!(1.1), &json!(17.0), &json!(13.0))
    );

    // `space.new {focus: false}` answers the new ids once its terminal runs, and keeps the
    // shown space.
    let out = cli(
        &mut rt,
        home,
        &[
            "do",
            "space.new",
            "--name",
            "repo@branch",
            "--folder",
            &folder,
            "--focus",
            "false",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let made = json_of(&out);
    let (space, made_tab, terminal) = (
        made["space"].as_str().expect("the new space").to_owned(),
        made["tab"].as_str().expect("the new tab").to_owned(),
        made["terminal"]
            .as_str()
            .expect("the new terminal")
            .to_owned(),
    );
    assert!(is_live(&conn, &terminal), "space.new's terminal runs");
    layout_until(
        &mut rt,
        &conn,
        "the new space in the core's layout",
        |doc| space_in(doc, &space).is_some_and(|s| s["name"] == "repo@branch"),
    );
    let listed = json_of(&cli(&mut rt, home, &["ls"]));
    let new = &listed["spaces"][2];
    assert_eq!(new["id"], space.as_str());
    assert_eq!(new["tabs"][0]["id"], made_tab.as_str());
    assert_eq!(
        new["tabs"][0]["terminals"][0]["terminal"],
        terminal.as_str()
    );
    assert_eq!(status(&mut rt, home)["space"], "sp-1", "focus: false");

    // Renames write the shared layout.
    ok(
        &mut rt,
        home,
        &["do", "space.rename", "--id", &space, "--name", " renamed "],
    );
    ok(
        &mut rt,
        home,
        &["do", "tab.rename", r#"{"id": "tb-2", "name": "Fix login"}"#],
    );
    layout_until(&mut rt, &conn, "the renames in the core's layout", |doc| {
        space_in(doc, &space).is_some_and(|s| s["name"] == "renamed")
            && space_in(doc, "sp-1").is_some_and(|s| s["tabs"][1]["name"] == "Fix login")
    });

    // Selecting and focusing move the view.
    ok(&mut rt, home, &["do", "space.select", "--index", "2"]);
    assert_eq!(status(&mut rt, home)["space"], "sp-2");
    ok(&mut rt, home, &["do", "agent.focus", "--terminal", &b]);
    let shown = status(&mut rt, home);
    assert_eq!(
        (&shown["space"], &shown["focused_pane"]),
        (&json!("sp-1"), &json!("pn-b"))
    );
    fails(
        &mut rt,
        home,
        &["do", "agent.focus", "--terminal", "nope"],
        "no running terminal \"nope\"",
    );

    // `pane.close` ends the pane's terminal and takes it out of the layout.
    ok(&mut rt, home, &["do", "pane.close", "--pane", "pn-b"]);
    layout_until(&mut rt, &conn, "pn-b to leave the core's layout", |doc| {
        !doc.to_string().contains("pn-b")
    });
    until(&mut rt, "pn-b's terminal to end", |_| !is_live(&conn, &b));

    // `space.close` needs `confirmed`, then ends the space's terminals.
    fails(
        &mut rt,
        home,
        &["do", "space.close", "--id", &space],
        "space.close ends its terminals: pass confirmed: true",
    );
    ok(
        &mut rt,
        home,
        &["do", "space.close", "--id", &space, "--confirmed", "true"],
    );
    layout_until(
        &mut rt,
        &conn,
        "the space to leave the core's layout",
        |doc| space_in(doc, &space).is_none(),
    );
    until(&mut rt, "the closed space's terminal to end", |_| {
        !is_live(&conn, &terminal)
    });

    // `layout.switch` knows only `eli`; unknown commands say where the list is.
    ok(&mut rt, home, &["do", "layout.switch", "--name", "eli"]);
    fails(
        &mut rt,
        home,
        &["do", "layout.switch", "--name", "wide"],
        "no layout \"wide\": herdr-frontend has only the \"eli\" layout",
    );
    fails(
        &mut rt,
        home,
        &["do", "tab.new"],
        "unknown command \"tab.new\": see `herdr-frontend commands` for the ones it serves",
    );
    assert!(is_live(&conn, &a) && is_live(&conn, &c) && is_live(&conn, &d));

    // The socket goes with the window.
    drop(rt);
    assert!(!socket.exists(), "the window left its socket");
    let out = cli_without_window(home, &["ls"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with(&format!(
            "herdr-frontend: no Herdr window at {}",
            socket.display()
        )),
        "{out:?}"
    );
}
