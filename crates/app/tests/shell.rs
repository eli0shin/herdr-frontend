//! The app shell on a fake core, through Rune's real runtime with no window: the window's
//! regions, the shown tab's live panes, the viewer policy (`herdr-core-protocol.md` §4), plain
//! typing, and that the layout document is only read.

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{self, demo_core};
use herdr_frontend::link::{Fake, Out};
use herdr_frontend::state::Attach;
use herdr_frontend::{SIZE, State, app};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};

/// A runtime on the demo core, settled and laid out.
fn runtime() -> (Runtime<State>, Fake) {
    runtime_on(demo_core())
}

fn runtime_on(fake: Fake) -> (Runtime<State>, Fake) {
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(fake.clone()));
    rt.state.settle();
    frames(&mut rt, 3);
    (rt, fake)
}

fn frames(rt: &mut Runtime<State>, n: usize) {
    for _ in 0..n {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }
}

fn element(rt: &Runtime<State>, key: &str) -> ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements().iter().any(|e| e.key.as_deref() == Some(key))
}

/// A terminal's screen as the app last drew it.
fn screen(rt: &Runtime<State>, address: &str) -> terminal::Snapshot {
    rt.state.viewers[address]
        .pane
        .as_ref()
        .expect("a snapshot")
        .snapshot
        .clone()
}

/// The pane `pane`'s rect in the tab area, and the grid it holds.
fn grid_of(rt: &Runtime<State>, pane: &str) -> (u16, u16) {
    let (_, rect, _) = rt
        .state
        .shown_panes()
        .into_iter()
        .find(|(p, ..)| p == pane)
        .expect("a shown pane");
    let grid = rt.state.grid_for(rect);
    (grid.columns, grid.rows)
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        let key = match c {
            '\r' => Key::Enter,
            c => Key::Text(c.to_string()),
        };
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        rt.frame(1.0 / 60.0);
    }
}

#[test]
fn the_window_has_a_sidebar_a_tab_strip_and_the_shown_tabs_panes() {
    let (rt, _) = runtime();
    let root = element(&rt, "root");
    assert_eq!(root.rect, [0.0, 0.0, SIZE.0, SIZE.1]);

    // The 38 pt rows that drag the window, and no status row: the panes reach the bottom.
    let brand = element(&rt, "brand");
    assert_eq!(brand.rect, [0.0, 0.0, 366.0, 38.0]);
    let tabs = element(&rt, "tabs");
    assert_eq!(tabs.rect, [366.0, 0.0, SIZE.0 - 366.0, 38.0]);
    let main = element(&rt, "main");
    assert_eq!(main.rect, [366.0, 38.0, SIZE.0 - 366.0, SIZE.1 - 38.0]);

    // Both tabs in the strip; only the selected one's panes are drawn.
    assert!(has(&rt, "tab-tb-work") && has(&rt, "tab-tb-logs"));
    for pane in ["pane-pn-1", "pane-pn-2", "pane-pn-3"] {
        assert!(has(&rt, pane), "{pane}");
    }
    assert!(!has(&rt, "pane-pn-4"), "the hidden tab's pane isn't drawn");

    // pn-1 | (pn-2 / pn-3), bare, with a 1 pt gap between them.
    let (a, b, c) = (
        element(&rt, "pane-pn-1").rect,
        element(&rt, "pane-pn-2").rect,
        element(&rt, "pane-pn-3").rect,
    );
    let width = SIZE.0 - 366.0;
    assert_eq!(a[0], 366.0);
    assert_eq!(b[0], a[0] + a[2] + 1.0);
    assert_eq!(a[2] + 1.0 + b[2], width);
    assert_eq!(c[1], b[1] + b[3] + 1.0);
    assert_eq!(a[1] + a[3], SIZE.1);
}

#[test]
fn panes_draw_their_terminals_live() {
    let (rt, _) = runtime();
    assert!(
        screen(&rt, "t1")
            .line(0)
            .contains("line 1 of the pager fixture")
    );
    assert!(screen(&rt, "t2").text().contains("16 colors"));
    assert!(screen(&rt, "t3").text().contains("Light, heavy and double"));
    let grid = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("screen") && e.path.contains("pane-pn-2/"))
        .expect("pane 2's grid");
    assert_eq!(grid.role, "terminal");
}

#[test]
fn every_pane_attaches_resumed_and_hidden_tabs_wait() {
    let (rt, fake) = runtime();
    let core = fake.core();
    let attached: Vec<&str> = core.attaches.iter().map(|a| a.terminal.as_str()).collect();
    assert_eq!(attached, ["t1", "t2", "t3"]);
    for attach in &core.attaches {
        assert!(
            attach.resumed,
            "{} attached taking the size",
            attach.terminal
        );
        assert!(!attach.observe);
        assert_eq!(attach.history, 2000);
        assert_eq!((attach.cols, attach.rows), (fixtures::COLS, fixtures::ROWS));
        assert_eq!(
            attach.display.as_ref().and_then(|d| d.cell),
            Some([17, 37]),
            "the cell at 2x"
        );
    }
    drop(core);
    for address in ["t1", "t2", "t3"] {
        assert_eq!(rt.state.viewers[address].attach, Attach::Attached);
    }
}

#[test]
fn only_the_focused_pane_takes_the_size_with_facts_before_resize() {
    let (rt, fake) = runtime();
    let (cols, rows) = grid_of(&rt, "pn-1");
    let display = rt.state.display();
    assert_eq!(
        fake.sent_to("t1"),
        [
            Out::Facts(display),
            Out::Resize { cols, rows },
            Out::Focus(true)
        ]
    );
    assert_eq!(fake.sent_to("t2"), []);
    assert_eq!(fake.sent_to("t3"), []);
    assert_eq!(rt.state.in_use(), Some("t1"));

    // The core gave t1 the pane's grid, and the local terminal follows the core.
    let s = screen(&rt, "t1");
    assert_eq!((s.cols, s.rows), (cols, rows));
    // The others stay at the size the core has for them.
    let s = screen(&rt, "t2");
    assert_eq!((s.cols, s.rows), (fixtures::COLS, fixtures::ROWS));
}

#[test]
fn a_size_from_another_viewer_resizes_the_local_terminal_without_taking_it_back() {
    let (mut rt, fake) = runtime();
    fake.resize_elsewhere("t2", 50, 10);
    frames(&mut rt, 2);
    let s = screen(&rt, "t2");
    assert_eq!((s.cols, s.rows), (50, 10));
    assert_eq!(
        fake.sent_to("t2"),
        [],
        "an unfocused pane never sends RESIZE"
    );
}

fn resizes(fake: &Fake, address: &str) -> Vec<Out> {
    fake.sent_to(address)
        .into_iter()
        .filter(|o| matches!(o, Out::Resize { .. }))
        .collect()
}

#[test]
fn a_click_takes_the_size_back_from_another_viewer() {
    let (mut rt, fake) = runtime();
    let (cols, rows) = grid_of(&rt, "pn-1");
    assert_eq!(resizes(&fake, "t1"), [Out::Resize { cols, rows }]);
    // Herdr app took it (attached, refit or reattached); the window doesn't fight it.
    fake.resize_elsewhere("t1", 50, 10);
    frames(&mut rt, 3);
    assert_eq!(resizes(&fake, "t1").len(), 1, "taken back without use");
    let s = screen(&rt, "t1");
    assert_eq!((s.cols, s.rows), (50, 10));

    // A click in it is use: it takes the size back.
    let (x, y) = rt.find("pane-pn-1").expect("pane 1");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    assert_eq!(resizes(&fake, "t1").len(), 2);
    assert_eq!(
        resizes(&fake, "t1").last(),
        Some(&Out::Resize { cols, rows })
    );
    let s = screen(&rt, "t1");
    assert_eq!((s.cols, s.rows), (cols, rows));

    // While it holds the size, a click sends none.
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    assert_eq!(resizes(&fake, "t1").len(), 2);
}

#[test]
fn every_pane_answers_queries_as_automatic_input() {
    let (mut rt, fake) = runtime();
    let before = fake.sent_to("t1").len();
    // Device status: the engine answers `CSI 0 n`.
    fake.output("t1", b"\x1b[5n");
    fake.output("t2", b"\x1b[5n");
    frames(&mut rt, 3);
    let reply = Out::Automatic(b"\x1b[0n".to_vec());
    assert_eq!(&fake.sent_to("t1")[before..], std::slice::from_ref(&reply));
    assert_eq!(fake.sent_to("t2"), [reply]);
}

#[test]
fn without_automatic_input_only_the_pane_in_use_answers_queries() {
    let fake = demo_core();
    fake.core().no_automatic_input = true;
    let (mut rt, fake) = runtime_on(fake);
    let before = fake.sent_to("t1").len();
    fake.output("t1", b"\x1b[5n");
    fake.output("t2", b"\x1b[5n");
    frames(&mut rt, 3);
    // The pane in use, at the grid it told the core: its DATA takes nothing from anyone.
    assert_eq!(
        fake.sent_to("t1")[before..],
        [Out::Input(b"\x1b[0n".to_vec())]
    );
    assert_eq!(
        fake.sent_to("t2"),
        [],
        "as DATA, the reply would take the size"
    );
}

#[test]
fn without_automatic_input_the_pane_in_use_stops_answering_once_another_viewer_moves_it() {
    let fake = demo_core();
    fake.core().no_automatic_input = true;
    let (mut rt, fake) = runtime_on(fake);
    // The program asks for in-band size reports: one comes at once, as DATA.
    fake.output("t1", b"\x1b[?2048h");
    frames(&mut rt, 3);
    let before = fake.sent_to("t1").len();
    let data = |fake: &Fake, from: usize| -> Vec<Out> {
        fake.sent_to("t1")[from..]
            .iter()
            .filter(|o| matches!(o, Out::Input(_) | Out::Automatic(_)))
            .cloned()
            .collect()
    };

    // Herdr app takes the size. The resize brings a mode-2048 report, and a query follows:
    // as DATA, either would take the size back with no real use.
    fake.resize_elsewhere("t1", 50, 10);
    frames(&mut rt, 3);
    fake.output("t1", b"\x1b[5n");
    frames(&mut rt, 3);
    assert_eq!(data(&fake, before), [], "a reply took the size back");
    assert_eq!(resizes(&fake, "t1").len(), 1);

    // Typing takes it back; replies go again.
    type_text(&mut rt, "x");
    let typed = fake.sent_to("t1").len();
    fake.output("t1", b"\x1b[5n");
    frames(&mut rt, 3);
    assert_eq!(data(&fake, typed), [Out::Input(b"\x1b[0n".to_vec())]);
}

#[test]
fn a_late_size_event_for_an_older_resize_doesnt_silence_the_pane_in_use() {
    let fake = demo_core();
    fake.core().no_automatic_input = true;
    let (mut rt, fake) = runtime_on(fake);
    let (cols, rows) = grid_of(&rt, "pn-1");
    // A divider drag over a slow link: the event for an older RESIZE lands after the last one
    // went out, then the event for the last one.
    fake.resize_elsewhere("t1", cols - 3, rows);
    frames(&mut rt, 2);
    fake.resize_elsewhere("t1", cols, rows);
    frames(&mut rt, 2);
    let before = fake.sent_to("t1").len();
    fake.output("t1", b"\x1b[5n");
    frames(&mut rt, 3);
    assert_eq!(
        fake.sent_to("t1")[before..],
        [Out::Input(b"\x1b[0n".to_vec())]
    );
}

#[test]
fn clicking_a_pane_moves_focus_and_the_size_with_it() {
    let (mut rt, fake) = runtime();
    let (x, y) = rt.find("pane-pn-2").expect("pane 2");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    assert_eq!(rt.state.model.focused_pane().as_deref(), Some("pn-2"));
    assert_eq!(rt.state.in_use(), Some("t2"));
    assert_eq!(fake.sent_to("t1").last(), Some(&Out::Focus(false)));
    let (cols, rows) = grid_of(&rt, "pn-2");
    assert_eq!(
        fake.sent_to("t2"),
        [
            Out::Facts(rt.state.display()),
            Out::Resize { cols, rows },
            Out::Focus(true)
        ]
    );
}

#[test]
fn typing_goes_to_the_focused_pane_as_data() {
    let (mut rt, fake) = runtime();
    fake.core().echo = true;
    type_text(&mut rt, "ls\r");
    let typed: Vec<Out> = fake
        .sent_to("t1")
        .into_iter()
        .filter(|o| matches!(o, Out::Input(_)))
        .collect();
    assert_eq!(
        typed,
        [
            Out::Input(b"l".to_vec()),
            Out::Input(b"s".to_vec()),
            Out::Input(b"\r".to_vec())
        ]
    );
    assert!(fake.sent_to("t2").is_empty());

    // Ctrl+C reaches the program in the form it asked for: Neovim's modifyOtherKeys makes it
    // `CSI 27;5;99~`, as Ghostty sends it; a plain shell gets its control character.
    let ctrl_c = KeyEvent {
        key: Key::Text("c".into()),
        mods: Mods {
            ctrl: true,
            ..Mods::default()
        },
    };
    rt.key(ctrl_c.clone());
    rt.frame(1.0 / 60.0);
    assert_eq!(
        fake.sent_to("t1").last(),
        Some(&Out::Input(b"\x1b[27;5;99~".to_vec()))
    );
    fake.output("t1", b"\x1bc");
    frames(&mut rt, 3);
    rt.key(ctrl_c);
    rt.frame(1.0 / 60.0);
    assert_eq!(fake.sent_to("t1").last(), Some(&Out::Input(vec![0x03])));
}

#[test]
fn a_window_in_the_background_gives_up_focus_and_takes_no_size() {
    let (mut rt, fake) = runtime();
    rt.window_focus(false);
    frames(&mut rt, 2);
    assert_eq!(rt.state.in_use(), None);
    assert_eq!(fake.sent_to("t1").last(), Some(&Out::Focus(false)));
    let before = fake.sent_to("t1").len();

    // Back in front: the focused pane takes the size again.
    rt.window_focus(true);
    frames(&mut rt, 2);
    let after = fake.sent_to("t1");
    let (cols, rows) = grid_of(&rt, "pn-1");
    assert_eq!(
        &after[before..],
        [Out::Resize { cols, rows }, Out::Focus(true)]
    );
}

#[test]
fn after_a_dropped_connection_every_pane_reattaches_resumed() {
    let (mut rt, fake) = runtime();
    fake.drop_connection();
    frames(&mut rt, 3);
    let core = fake.core();
    let again: Vec<&str> = core.attaches[3..]
        .iter()
        .map(|a| a.terminal.as_str())
        .collect();
    assert_eq!(again, ["t1", "t2", "t3"]);
    assert!(core.attaches.iter().all(|a| a.resumed));
    drop(core);
    // The focused pane takes the size again on the new stream.
    let resizes = fake
        .sent_to("t1")
        .into_iter()
        .filter(|o| matches!(o, Out::Resize { .. }))
        .count();
    assert_eq!(resizes, 2);
}

#[test]
fn the_layout_document_is_only_read() {
    let (mut rt, fake) = runtime();
    let (_, version) = fake.core().docs[&None].clone();
    let (x, y) = rt.find("pane-pn-3").expect("pane 3");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    type_text(&mut rt, "x");
    frames(&mut rt, 2);
    let core = fake.core();
    assert_eq!(core.writes, 0, "nothing was written");
    assert_eq!(core.docs[&None].1, version);
}

#[test]
fn the_shell_scenes_draw() {
    for (scene, status) in [
        ("shell-no-spaces", "No spaces"),
        ("shell-connecting", "Connecting to herdr-core…"),
        (
            "shell-core-down",
            "herdr-core unavailable: hc connect: no core is running",
        ),
    ] {
        let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
        let (_, apply) = herdr_frontend::spec()
            .scenes
            .iter()
            .find(|(name, _)| *name == scene)
            .copied()
            .unwrap();
        apply(&mut rt.state);
        rt.frame(0.0);
        assert_eq!(
            element(&rt, "status").text.as_deref(),
            Some(status),
            "{scene}"
        );
        assert!(has(&rt, "brand") && has(&rt, "tabs"), "{scene}");
    }
}

#[test]
fn a_failed_layout_read_is_tried_again_when_the_model_asks() {
    let fake = demo_core();
    fake.core().docs_down = true;
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(fake.clone()));
    frames(&mut rt, 2);
    assert!(rt.state.model.shown_tab().is_none(), "nothing read yet");

    // The machine comes back; nothing else happens on the core. The model's retry (2 s)
    // reads the layout again on its own.
    fake.core().docs_down = false;
    for _ in 0..8 {
        rt.invalidate();
        rt.frame(0.5);
    }
    assert_eq!(
        rt.state.model.shown_tab().map(|t| t.id.as_str()),
        Some("tb-work")
    );
}
