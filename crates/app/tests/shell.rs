//! The app shell on a fake core, through Rune's real runtime with no window: the window's
//! regions, the shown tab's live panes, the viewer policy (`herdr-core-protocol.md` §4), plain
//! typing, and that the layout document is only read.

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{self, demo_core};
use herdr_frontend::link::{Fake, Out};
use herdr_frontend::state::{Attach, REASSERT_AFTER};
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
    // t2 and t3 took their panes' smaller grids: their screens' last lines are on screen.
    assert!(screen(&rt, "t2").text().contains("Underlines:"));
    assert!(screen(&rt, "t3").text().contains("Shapes:"));
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
        // JetBrains Mono 14 pt at 2x: 17 × 37 px at macOS's 72 DPI, 22 × 49 at 96.
        let cell = if cfg!(target_os = "macos") {
            [17, 37]
        } else {
            [22, 49]
        };
        assert_eq!(
            attach.display.as_ref().and_then(|d| d.cell),
            Some(cell),
            "the cell at 2x"
        );
    }
    drop(core);
    for address in ["t1", "t2", "t3"] {
        assert_eq!(rt.state.viewers[address].attach, Attach::Attached);
    }
}

#[test]
fn every_shown_pane_takes_its_size_with_facts_before_resize_and_the_focused_one_focus() {
    let (rt, fake) = runtime();
    let display = rt.state.display();
    let (cols, rows) = grid_of(&rt, "pn-1");
    assert_eq!(
        fake.sent_to("t1"),
        [
            Out::Facts(display.clone()),
            Out::Resize { cols, rows },
            Out::Focus(true)
        ]
    );
    for (pane, address) in [("pn-2", "t2"), ("pn-3", "t3")] {
        let (cols, rows) = grid_of(&rt, pane);
        assert_eq!(
            fake.sent_to(address),
            [Out::Facts(display.clone()), Out::Resize { cols, rows }],
            "{address} isn't focused, but it's shown"
        );
    }
    assert_eq!(
        fake.sent_to("t4"),
        [],
        "a hidden tab's terminal takes nothing"
    );
    assert_eq!(rt.state.in_use(), Some("t1"));

    // The core gave each its pane's grid, and the local terminals follow the core.
    for (pane, address) in [("pn-1", "t1"), ("pn-2", "t2"), ("pn-3", "t3")] {
        let s = screen(&rt, address);
        assert_eq!((s.cols, s.rows), grid_of(&rt, pane), "{address}");
    }
}

#[test]
fn a_size_from_another_viewer_resizes_the_local_terminal_without_taking_it_back() {
    let (mut rt, fake) = runtime();
    let before = fake.sent_to("t2").len();
    fake.resize_elsewhere("t2", 50, 10);
    frames(&mut rt, 2);
    let s = screen(&rt, "t2");
    assert_eq!((s.cols, s.rows), (50, 10));
    assert_eq!(
        fake.sent_to("t2")[before..],
        [],
        "its grid didn't change, so it isn't fought for"
    );
}

#[test]
fn every_shown_pane_whose_grid_changes_sends_resize_and_hidden_ones_dont() {
    let (mut rt, fake) = runtime();
    let before: Vec<usize> = ["t1", "t2", "t3"]
        .iter()
        .map(|a| resizes(&fake, a).len())
        .collect();
    // Hiding the sidebar widens every pane.
    rt.key(KeyEvent {
        key: Key::Text("b".into()),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    });
    frames(&mut rt, 4);
    for ((pane, address), before) in [("pn-1", "t1"), ("pn-2", "t2"), ("pn-3", "t3")]
        .into_iter()
        .zip(before)
    {
        let (cols, rows) = grid_of(&rt, pane);
        let sent = resizes(&fake, address);
        assert_eq!(sent.len(), before + 1, "{address}");
        assert_eq!(sent.last(), Some(&Out::Resize { cols, rows }), "{address}");
        let s = screen(&rt, address);
        assert_eq!((s.cols, s.rows), (cols, rows), "{address} redrawn at once");
    }

    // In another tab, the panes left behind take nothing when the window changes.
    rt.state.step(0.0, model::Action::ShowTab("tb-logs".into()));
    frames(&mut rt, 3);
    let left: Vec<usize> = ["t1", "t2", "t3"]
        .iter()
        .map(|a| resizes(&fake, a).len())
        .collect();
    rt.key(KeyEvent {
        key: Key::Text("b".into()),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    });
    frames(&mut rt, 4);
    for (address, left) in ["t1", "t2", "t3"].into_iter().zip(left) {
        assert_eq!(resizes(&fake, address).len(), left, "{address} is hidden");
    }
    let (cols, rows) = grid_of(&rt, "pn-4");
    assert_eq!(
        resizes(&fake, "t4").last(),
        Some(&Out::Resize { cols, rows })
    );
}

#[test]
fn split_panes_are_whole_cells_and_each_grid_sits_at_its_bottom_left() {
    let (rt, _) = runtime();
    let (cell_w, cell_h) = rt.state.face(2.0).cell_pt();
    let whole = |len: f32, cell: f32| ((len / cell).round() * cell - len).abs() < 1e-3;
    let rect = |key: &str| element(&rt, key).rect;
    let grid = |pane: &str| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("screen") && e.path.contains(&format!("{pane}/")))
            .unwrap_or_else(|| panic!("{pane}'s grid"))
            .rect
    };
    // pn-1 | (pn-2 / pn-3): pn-1 is whole columns, so its grid fills it to the divider; pn-3,
    // at the bottom of the column, is whole rows. pn-2, at the top, takes the leftover.
    let (a, b, c) = (rect("pane-pn-1"), rect("pane-pn-2"), rect("pane-pn-3"));
    assert!(whole(a[2], cell_w), "{a:?}");
    assert!(whole(c[3], cell_h), "{c:?}");
    for (pane, r) in [("pane-pn-1", a), ("pane-pn-2", b), ("pane-pn-3", c)] {
        let g = grid(pane);
        assert_eq!(g[0], r[0], "{pane}: at the left");
        assert!(
            (g[1] + g[3] - (r[1] + r[3])).abs() <= 0.5,
            "{pane}: the bottom row on the bottom edge, {g:?} in {r:?}"
        );
    }
    assert!(
        (grid("pane-pn-1")[2] - a[2]).abs() < 1e-3,
        "no gap at the divider"
    );
    // Only the gaps are the border color; the rest of the tab area is the terminal's background.
    let gaps: Vec<[f32; 4]> = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with("gap-")))
        .map(|e| e.rect)
        .collect();
    assert_eq!(
        gaps,
        [
            [a[0] + a[2], a[1], 1.0, a[3]],
            [b[0], b[1] + b[3], b[2], 1.0]
        ]
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

/// Herdr app takes the size back the moment the pane in use takes it: the core's two `size`
/// events, the window's own and then Herdr app's, reach the window together. The terminal ends at
/// the core's grid, Herdr app's, not at the one the window took (ticket 047).
#[test]
fn a_size_taken_back_at_once_is_followed() {
    let (mut rt, fake) = runtime();
    let (cols, rows) = grid_of(&rt, "pn-1");
    fake.resize_elsewhere("t1", 50, 10);
    frames(&mut rt, 3);
    // From here, the core's news waits, as on a slow link.
    fake.core().quiet = true;
    let (x, y) = rt.find("pane-pn-1").expect("pane 1");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    // The click's RESIZE goes out, and the terminal is at that grid at once.
    frames(&mut rt, 2);
    assert_eq!(
        resizes(&fake, "t1").last(),
        Some(&Out::Resize { cols, rows })
    );
    let s = screen(&rt, "t1");
    assert_eq!((s.cols, s.rows), (cols, rows));
    // Herdr app takes it back before the window hears of either.
    fake.resize_elsewhere("t1", 50, 10);
    fake.core().quiet = false;
    frames(&mut rt, 3);
    let s = screen(&rt, "t1");
    assert_eq!((s.cols, s.rows), (50, 10));
    assert_eq!(resizes(&fake, "t1").len(), 2, "taken back without use");
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
    assert_eq!(fake.sent_to("t2").last(), Some(&reply));
}

#[test]
fn without_automatic_input_only_panes_shown_in_front_answer_queries() {
    let fake = demo_core();
    fake.core().no_automatic_input = true;
    let (mut rt, fake) = runtime_on(fake);
    let before: Vec<usize> = ["t1", "t2"].iter().map(|a| fake.sent_to(a).len()).collect();
    fake.output("t1", b"\x1b[5n");
    fake.output("t2", b"\x1b[5n");
    frames(&mut rt, 3);
    // Each shown pane holds the grid it told the core: its DATA takes nothing from anyone.
    for (address, before) in ["t1", "t2"].into_iter().zip(before) {
        assert_eq!(
            fake.sent_to(address)[before..],
            [Out::Input(b"\x1b[0n".to_vec())],
            "{address}"
        );
    }
    // In a window in the background, as DATA, a reply would take the size.
    rt.window_focus(false);
    frames(&mut rt, 2);
    let before: Vec<usize> = ["t1", "t2"].iter().map(|a| fake.sent_to(a).len()).collect();
    fake.output("t1", b"\x1b[5n");
    fake.output("t2", b"\x1b[5n");
    frames(&mut rt, 3);
    for (address, before) in ["t1", "t2"].into_iter().zip(before) {
        assert_eq!(fake.sent_to(address)[before..], [], "{address}");
    }
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
fn turning_size_reports_on_in_the_pane_in_use_re_asserts_the_size_after_the_other_answers() {
    let (mut rt, fake) = runtime();
    assert_eq!(rt.state.in_use(), Some("t1"));
    let (cols, rows) = grid_of(&rt, "pn-1");
    let report = |fake: &Fake, address: &str| -> Vec<u8> {
        let Some(Out::Automatic(bytes)) = fake.sent_to(address).last().cloned() else {
            panic!("no report: {:?}", fake.sent_to(address));
        };
        assert!(bytes.starts_with(b"\x1b[48;"), "{bytes:?}");
        bytes
    };
    let since = |fake: &Fake, address: &str, from: usize| fake.sent_to(address)[from..].to_vec();

    // The program in the pane in use turns the mode on: the terminal answers at once, as every
    // viewer's does.
    let before = fake.sent_to("t1").len();
    fake.output("t1", b"\x1b[?2048h");
    frames(&mut rt, 2);
    let ours = report(&fake, "t1");
    assert_eq!(since(&fake, "t1", before), [Out::Automatic(ours.clone())]);

    // Once the other viewers have answered, it re-asserts: RESIZE, so the core takes this
    // window's grid, then this window's report again. Then again, for a slow viewer.
    // The test's frames are 1/60 s apart.
    let reassert = [Out::Resize { cols, rows }, Out::Automatic(ours.clone())];
    let mut at = 2;
    for after in REASSERT_AFTER {
        let due = (after * 60.0).round() as usize;
        let before = fake.sent_to("t1").len();
        frames(&mut rt, due - 2 - at);
        assert_eq!(since(&fake, "t1", before), [], "not before {after} s");
        frames(&mut rt, 5);
        assert_eq!(since(&fake, "t1", before), reassert, "at {after} s");
        at = due + 3;
    }
    let before = fake.sent_to("t1").len();
    frames(&mut rt, 120);
    assert_eq!(since(&fake, "t1", before), [], "that's all");

    // In a pane that isn't in use, the answers stand.
    let before = fake.sent_to("t2").len();
    fake.output("t2", b"\x1b[?2048h");
    frames(&mut rt, 2);
    let theirs = report(&fake, "t2");
    frames(&mut rt, 120);
    assert_eq!(since(&fake, "t2", before), [Out::Automatic(theirs)]);

    // Nor does the pane in use take the size back from another viewer that took it.
    fake.resize_elsewhere("t1", 50, 10);
    frames(&mut rt, 2);
    fake.output("t1", b"\x1b[?2048h");
    frames(&mut rt, 2);
    let before = fake.sent_to("t1").len();
    frames(&mut rt, 120);
    assert_eq!(since(&fake, "t1", before), []);
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
fn clicking_a_pane_moves_focus_to_it() {
    let (mut rt, fake) = runtime();
    let (x, y) = rt.find("pane-pn-2").expect("pane 2");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    assert_eq!(rt.state.model.focused_pane().as_deref(), Some("pn-2"));
    assert_eq!(rt.state.in_use(), Some("t2"));
    assert_eq!(fake.sent_to("t1").last(), Some(&Out::Focus(false)));
    // It held its size already. Focusing it re-asserts the size, RESIZE again at the same grid,
    // then FOCUS(1).
    let (cols, rows) = grid_of(&rt, "pn-2");
    assert_eq!(
        fake.sent_to("t2"),
        [
            Out::Facts(rt.state.display()),
            Out::Resize { cols, rows },
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
    assert!(
        !fake
            .sent_to("t2")
            .iter()
            .any(|o| matches!(o, Out::Input(_)))
    );

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

fn cmd(key: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Text(key.into()),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    }
}

/// The last `terminal.run` the app made.
fn last_run(fake: &Fake) -> herdr_proto::methods::TerminalRun {
    fake.core()
        .calls
        .iter()
        .rev()
        .find_map(|c| match c {
            model::Request::Run(r) => Some((**r).clone()),
            _ => None,
        })
        .expect("a terminal.run")
}

#[test]
fn a_terminal_the_app_makes_starts_at_its_panes_grid_and_display() {
    let (mut rt, fake) = runtime();
    // A split of the focused pane, and a new tab: each new pane's grid, as its RESIZE will
    // tell it, and the window's FACTS.
    for key in ["d", "t"] {
        rt.key(cmd(key));
        frames(&mut rt, 3);
        let run = last_run(&fake);
        let pane = rt.state.model.focused_pane().expect("the new pane");
        let address = rt.state.model.focused_terminal().expect("its terminal");
        assert_eq!(
            fake.core().terminals[address].labels.get("herdr.tab"),
            run.labels.get("herdr.tab"),
            "the new pane holds the terminal that run made"
        );
        let (cols, rows) = grid_of(&rt, &pane);
        assert_eq!(
            run.size,
            Some(herdr_proto::methods::InitialSize { cols, rows }),
            "cmd-{key}"
        );
        assert_eq!(run.display, Some(rt.state.display()), "cmd-{key}");
    }
}

#[test]
fn a_window_in_the_background_gives_up_focus_and_takes_no_size() {
    let (mut rt, fake) = runtime();
    rt.window_focus(false);
    frames(&mut rt, 2);
    assert_eq!(rt.state.in_use(), None);
    assert_eq!(fake.sent_to("t1").last(), Some(&Out::Focus(false)));
    let before = fake.sent_to("t1").len();
    let before_t2 = fake.sent_to("t2").len();

    // Nothing takes the size in the background, not even a grid change.
    rt.key(KeyEvent {
        key: Key::Text("b".into()),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    });
    frames(&mut rt, 3);
    assert_eq!(fake.sent_to("t1").len(), before);
    assert_eq!(fake.sent_to("t2").len(), before_t2);

    // Back in front: every shown pane takes the size again, and the focused one focus.
    rt.window_focus(true);
    frames(&mut rt, 2);
    let (cols, rows) = grid_of(&rt, "pn-1");
    assert_eq!(
        fake.sent_to("t1")[before..],
        [Out::Resize { cols, rows }, Out::Focus(true)]
    );
    let (cols, rows) = grid_of(&rt, "pn-2");
    assert_eq!(
        fake.sent_to("t2")[before_t2..],
        [Out::Resize { cols, rows }]
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
    // The shown panes take the size again on the new stream.
    for address in ["t1", "t2", "t3"] {
        assert_eq!(resizes(&fake, address).len(), 2, "{address}");
    }
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
