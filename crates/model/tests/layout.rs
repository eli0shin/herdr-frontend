//! Tabs, spaces and panes through `Model::step`, against a fake core: what is written to the
//! shared document, which calls go to the core, and what this screen shows after.

mod support;

use herdr_proto::{CloseReason, ErrorKind, Reason, StatusState};
use herdr_shared::{Axis, Kind, Layout, Node};
use model::mode::{Confirmed, PopupState, Rename};
use model::{Action, Effect, Event, Focus, Geometry, Mode, PopupAction, Rect, Split};
use serde_json::json;
use support::*;

/// One space, tabs tb1 (p1: t1), tb2 (p2: t2), tb3 (p3: t3).
fn three_tabs() -> Core {
    Core::new(
        doc(vec![space(
            "sp1",
            vec![
                tab("tb1", vec![pane("p1", "t1")]),
                tab("tb2", vec![pane("p2", "t2")]),
                tab("tb3", vec![pane("p3", "t3")]),
            ],
        )]),
        vec![
            shell("t1", "/home/me/code/one"),
            shell("t2", "/home/me/code/two"),
            shell("t3", "/home/me/code/three"),
        ],
    )
}

fn shown(m: &model::Model) -> Option<String> {
    m.shown_tab().map(|t| t.id.clone())
}

#[test]
fn connecting_reads_the_layout_and_focuses_the_first_pane() {
    let mut core = three_tabs();
    let mut m = model::Model::new(HOME);
    // Connected comes once subscribed, so the layout is read at once.
    let fx = drive(&mut m, &mut core, Event::Connected);
    let focus: Vec<&Effect> = fx
        .iter()
        .filter(|e| matches!(e, Effect::Focus(_)))
        .collect();
    assert_eq!(
        focus,
        [
            &Effect::Focus(Focus::None),
            &Effect::Focus(Focus::Pane("p1".into()))
        ]
    );
    let snapshot = core.snapshot();
    drive(&mut m, &mut core, snapshot);
    assert_eq!(
        *m.world().machine(None).unwrap().mirror.layout(),
        Layout::Ready
    );
    assert_eq!(m.active_space().map(|s| s.id.as_str()), Some("sp1"));
    assert_eq!(shown(&m).as_deref(), Some("tb1"));
    assert_eq!(m.focused_pane().as_deref(), Some("p1"));
    assert_eq!(m.focused_terminal(), Some("t1"));
    assert!(
        core.calls.is_empty(),
        "every terminal is placed and live: no lookups"
    );
}

#[test]
fn nothing_is_read_or_written_while_disconnected() {
    let mut core = three_tabs();
    let mut m = model::Model::new(HOME);
    let fx = m.step(1.0, Event::Tick);
    assert!(!fx.iter().any(|e| matches!(e, Effect::Doc { .. })));
    let fx = m.step(m.now(), Action::TabNew);
    // The layout isn't read, so no terminal starts: it would be left without a pane.
    assert!(!fx.iter().any(|e| matches!(e, Effect::Call { .. })));
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Event::Lost);
    let fx = m.step(m.now(), Action::PaneSplit(Split::Right));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Doc { .. })));
}

#[test]
fn a_new_tab_starts_a_labeled_terminal_then_goes_after_the_selected_tab() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::ShowTab("tb2".into()));
    drive(&mut m, &mut core, Action::TabNew);
    let runs = core.runs();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(
        run.cwd.as_deref(),
        Some("/home/me/code/two"),
        "the focused terminal's folder"
    );
    assert!(run.keep);
    assert_eq!(run.on, None);
    assert_eq!(run.env["TERM"], "xterm-256color");
    assert_eq!(run.labels["herdr.space"], "sp1");
    assert_eq!(run.labels["herdr.seen"], "0");
    let new_tab = run.labels["herdr.tab"].clone();
    assert!(
        new_tab.starts_with("tb") && new_tab.len() == 34,
        "{new_tab}"
    );
    let doc = core.doc(None);
    assert_eq!(
        tab_ids(&doc, "sp1"),
        ["tb1", "tb2", new_tab.as_str(), "tb3"]
    );
    assert_eq!(shown(&m), Some(new_tab.clone()));
    let pane = m.focused_pane().unwrap();
    assert_eq!(doc.tab(&new_tab).unwrap().root.panes()[0].id, pane);
    assert_eq!(m.focused_terminal(), Some("t101"));
}

#[test]
fn a_new_tab_with_no_space_starts_a_space_in_home() {
    let mut core = Core::new(doc(vec![]), vec![]);
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::TabNew);
    let doc = core.doc(None);
    assert_eq!(doc.spaces.len(), 1);
    assert_eq!(doc.spaces[0].folder, HOME);
    assert_eq!(core.runs()[0].cwd.as_deref(), Some(HOME));
    assert_eq!(
        m.active_space().map(|s| s.id.clone()),
        Some(doc.spaces[0].id.clone())
    );
    assert_eq!(m.focused_terminal(), Some("t101"));
}

#[test]
fn a_failed_launch_says_so_and_changes_nothing() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    let fx = m.step(m.now(), Action::TabNew);
    let Some(Effect::Call { token, .. }) = fx.iter().find(|e| matches!(e, Effect::Call { .. }))
    else {
        panic!("{fx:?}")
    };
    let fx = m.step(
        m.now(),
        Event::Reply {
            token: *token,
            result: Err(error(ErrorKind::Other("spawn".into()), "no shell")),
        },
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Toast(t) if t.body == "no shell"))
    );
    assert_eq!(m.shown_tab().unwrap().id, "tb1");
}

#[test]
fn splitting_puts_the_new_pane_beside_the_focused_one_and_focuses_it() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::PaneSplit(Split::Right));
    let doc = core.doc(None);
    let root = &doc.tab("tb1").unwrap().root;
    let panes: Vec<&str> = root.panes().iter().map(|n| n.id.as_str()).collect();
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0], "p1");
    assert_eq!(m.focused_pane().as_deref(), Some(panes[1]));
    assert_eq!(core.runs()[0].labels["herdr.tab"], "tb1");
    // Down in the new pane wraps it in a column.
    drive(&mut m, &mut core, Action::PaneSplit(Split::Down));
    let doc = core.doc(None);
    let root = &doc.tab("tb1").unwrap().root;
    assert_eq!(root.panes().len(), 3);
    assert!(
        root.children()
            .iter()
            .any(|c| c.axis() == Some(Axis::Column))
    );
}

#[test]
fn closing_a_pane_kills_its_terminal_and_focus_goes_back() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::PaneSplit(Split::Right));
    let new = m.focused_pane().unwrap();
    let new_terminal = m.focused_terminal().unwrap().to_owned();
    drive(&mut m, &mut core, Action::PaneClose);
    assert_eq!(core.kills(), [new_terminal]);
    let doc = core.doc(None);
    assert!(doc.tab("tb1").unwrap().root.find(&new).is_none());
    assert_eq!(m.focused_pane().as_deref(), Some("p1"));
}

#[test]
fn closing_a_tabs_last_pane_closes_the_tab_and_eli_goes_left() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::ShowTab("tb2".into()));
    drive(&mut m, &mut core, Action::PaneClose);
    assert_eq!(tab_ids(&core.doc(None), "sp1"), ["tb1", "tb3"]);
    assert_eq!(core.kills(), ["t2"]);
    assert_eq!(
        shown(&m).as_deref(),
        Some("tb1"),
        "left of the closed tab, not right"
    );
}

#[test]
fn closing_the_first_tab_shows_the_new_first_and_settled_and_agent_tabs_are_skipped() {
    let mut core = Core::new(
        doc(vec![space(
            "sp1",
            vec![
                tab("tb1", vec![pane("p1", "t1")]),
                tab("tb2", vec![pane("p2", "a2")]),
                tab("tb3", vec![pane("p3", "t3")]),
                tab("tb4", vec![pane("p4", "t4")]),
            ],
        )]),
        vec![
            shell("t1", "/"),
            agent("a2", StatusState::Working),
            shell("t3", "/"),
            shell("t4", "/"),
        ],
    );
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::TabClose);
    // tb2 is an agent's tab (hidden), so the nearest shown one to the right.
    assert_eq!(shown(&m).as_deref(), Some("tb3"));
    drive(&mut m, &mut core, Action::ShowTab("tb4".into()));
    let mut m2 = m.clone();
    // tb3 settled: closing tb4 skips it and tb2 (an agent's), and there's nothing to the right.
    drive(&mut m2, &mut core, Action::FocusPane("p4".into()));
    let mut view = m2.view().clone();
    view.settle("tb3", 1);
    let mut m3 = model::Model::with_view(HOME, view);
    let mut core3 = Core::new(
        core.docs[&None].0.clone(),
        core.terminals.values().cloned().collect(),
    );
    drive(&mut m3, &mut core3, Event::Connected);
    let snapshot = core3.snapshot();
    drive(&mut m3, &mut core3, snapshot);
    assert_eq!(shown(&m3).as_deref(), Some("tb4"));
    drive(&mut m3, &mut core3, Action::TabClose);
    // No shown, unsettled tab either side: the slot rule's pick stands.
    assert_eq!(shown(&m3).as_deref(), Some("tb3"));
}

#[test]
fn closing_an_agents_tab_focuses_the_next_agent_across_spaces() {
    let mut core = Core::new(
        doc(vec![
            space(
                "sp1",
                vec![
                    tab("tb1", vec![pane("p1", "t1")]),
                    tab("tb2", vec![pane("p2", "a2")]),
                ],
            ),
            space("sp2", vec![tab("tb3", vec![pane("p3", "a3")])]),
        ]),
        vec![
            shell("t1", "/"),
            agent("a2", StatusState::Working),
            agent("a3", StatusState::Idle),
        ],
    );
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::FocusAgent("a2".into()));
    assert_eq!(shown(&m).as_deref(), Some("tb2"));
    drive(&mut m, &mut core, Action::TabClose);
    assert_eq!(m.active_space().map(|s| s.id.as_str()), Some("sp2"));
    assert_eq!(m.focused_terminal(), Some("a3"));
}

#[test]
fn closing_a_spaces_last_tab_asks_and_confirming_closes_the_space() {
    let mut core = Core::new(
        doc(vec![
            space(
                "sp1",
                vec![tab("tb1", vec![pane("p1", "t1"), pane("p9", "t9")])],
            ),
            space("sp2", vec![tab("tb2", vec![pane("p2", "t2")])]),
        ]),
        vec![
            shell("t1", "/home/me/code/proj"),
            shell("t9", "/"),
            shell("t2", "/"),
        ],
    );
    let mut m = connected(&mut core);
    let fx = drive(&mut m, &mut core, Action::TabClose);
    let Mode::Popup(PopupState::Confirm(confirm)) = m.mode() else {
        panic!("{:?}", m.mode())
    };
    assert_eq!(confirm.title, "Close space?");
    assert_eq!(confirm.detail, "proj \u{2014} 1 tabs, 2 terminals");
    assert_eq!(confirm.then, Confirmed::CloseSpace("sp1".into()));
    assert!(fx.contains(&Effect::Focus(Focus::Popup)));
    assert!(core.kills().is_empty(), "nothing closes before confirming");
    // Esc cancels.
    key(&mut m, &mut core, "escape", false);
    assert_eq!(*m.mode(), Mode::Normal);
    assert_eq!(core.doc(None).spaces.len(), 2);
    // Enter confirms.
    drive(&mut m, &mut core, Action::SpaceClose);
    key(&mut m, &mut core, "enter", false);
    let doc = core.doc(None);
    assert_eq!(doc.spaces.len(), 1);
    assert_eq!(core.kills(), ["t1", "t9"]);
    assert_eq!(m.active_space().map(|s| s.id.as_str()), Some("sp2"));
    assert_eq!(m.view().recent[0].folder, "/home/me/code/x");
}

#[test]
fn closing_a_space_shows_the_one_that_slides_into_its_place() {
    let mut core = Core::new(
        doc(vec![
            space("sp1", vec![tab("tb1", vec![pane("p1", "t1")])]),
            space("sp2", vec![tab("tb2", vec![pane("p2", "t2")])]),
            space("sp3", vec![tab("tb3", vec![pane("p3", "t3")])]),
        ]),
        vec![shell("t1", "/"), shell("t2", "/"), shell("t3", "/")],
    );
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::SpaceSelect(2));
    drive(&mut m, &mut core, Action::SpaceClose);
    key(&mut m, &mut core, "enter", false);
    assert_eq!(m.active_space().map(|s| s.id.as_str()), Some("sp3"));
    drive(&mut m, &mut core, Action::SpaceClose);
    key(&mut m, &mut core, "enter", false);
    assert_eq!(
        m.active_space().map(|s| s.id.as_str()),
        Some("sp1"),
        "no next: the last"
    );
}

#[test]
fn a_saved_space_gone_at_startup_falls_back_to_the_first_space() {
    let mut core = Core::new(
        doc(vec![
            space("sp1", vec![tab("tb1", vec![pane("p1", "t1")])]),
            space("sp2", vec![tab("tb2", vec![pane("p2", "t2")])]),
            space("sp3", vec![tab("tb3", vec![pane("p3", "t3")])]),
        ]),
        vec![shell("t1", "/"), shell("t2", "/"), shell("t3", "/")],
    );
    let view = model::View {
        active: Some("spGONE".into()),
        ..Default::default()
    };
    let mut m = model::Model::with_view(HOME, view);
    drive(&mut m, &mut core, Event::Connected);
    assert_eq!(m.active_space().map(|s| s.id.as_str()), Some("sp1"));
    assert_eq!(m.view().active.as_deref(), Some("sp1"));
}

#[test]
fn spaces_step_and_select_by_number_in_order() {
    let mut core = Core::new(
        doc(vec![
            space("sp1", vec![tab("tb1", vec![pane("p1", "t1")])]),
            space("sp2", vec![tab("tb2", vec![pane("p2", "t2")])]),
        ]),
        vec![shell("t1", "/"), shell("t2", "/")],
    );
    let mut m = connected(&mut core);
    let space = |m: &model::Model| m.active_space().map(|s| s.id.clone()).unwrap();
    drive(&mut m, &mut core, Action::SpaceNext);
    assert_eq!(space(&m), "sp2");
    drive(&mut m, &mut core, Action::SpaceNext);
    assert_eq!(space(&m), "sp1");
    drive(&mut m, &mut core, Action::SpacePrev);
    assert_eq!(space(&m), "sp2");
    drive(&mut m, &mut core, Action::SpaceSelect(1));
    assert_eq!(space(&m), "sp1");
    drive(&mut m, &mut core, Action::SpaceSelect(9));
    assert_eq!(space(&m), "sp1", "no ninth space");
    key(&mut m, &mut core, "ctrl-2", true);
    assert_eq!(space(&m), "sp2");
}

#[test]
fn tab_keys_count_and_step_over_only_shown_unsettled_tabs() {
    let mut core = Core::new(
        doc(vec![space(
            "sp1",
            vec![
                tab("tb1", vec![pane("p1", "t1")]),
                tab("tb2", vec![pane("p2", "a2")]),
                tab("tb3", vec![pane("p3", "t3")]),
                tab("tb4", vec![pane("p4", "r4")]),
                tab("tb5", vec![pane("p5", "t5")]),
            ],
        )]),
        vec![
            shell("t1", "/"),
            agent("a2", StatusState::Done),
            shell("t3", "/"),
            {
                let mut r = shell("r4", "/");
                r.labels.insert(model::REVIEW_LABEL.into(), "1".into());
                r
            },
            shell("t5", "/"),
        ],
    );
    let mut m = connected(&mut core);
    let shown_ids: Vec<String> = m
        .shown_tabs(m.active_space().unwrap())
        .iter()
        .map(|t| t.id.clone())
        .collect();
    assert_eq!(
        shown_ids,
        ["tb1", "tb3", "tb5"],
        "agents' and the review's tabs are hidden"
    );
    key(&mut m, &mut core, "cmd-2", false);
    assert_eq!(shown(&m).as_deref(), Some("tb3"), "the second shown tab");
    key(&mut m, &mut core, "ctrl-g 3", false);
    assert_eq!(shown(&m).as_deref(), Some("tb5"));
    key(&mut m, &mut core, "cmd-4", false);
    assert_eq!(shown(&m).as_deref(), Some("tb5"), "no fourth shown tab");
    key(&mut m, &mut core, "ctrl-tab", false);
    assert_eq!(shown(&m).as_deref(), Some("tb1"), "wraps");
    key(&mut m, &mut core, "ctrl-g n", false);
    assert_eq!(shown(&m).as_deref(), Some("tb3"), "skips the agent's tab");
    key(&mut m, &mut core, "ctrl-shift-tab", false);
    assert_eq!(shown(&m).as_deref(), Some("tb1"));
    key(&mut m, &mut core, "cmd-shift-[", false);
    assert_eq!(shown(&m).as_deref(), Some("tb5"));
}

#[test]
fn tab_keys_on_an_agents_tab_move_through_the_agents() {
    let mut core = Core::new(
        doc(vec![
            space(
                "sp1",
                vec![
                    tab("tb1", vec![pane("p1", "t1")]),
                    tab("tb2", vec![pane("p2", "a2")]),
                ],
            ),
            space("sp2", vec![tab("tb3", vec![pane("p3", "a3")])]),
        ]),
        vec![
            shell("t1", "/"),
            agent("a2", StatusState::Working),
            agent("a3", StatusState::Blocked),
        ],
    );
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::FocusAgent("a2".into()));
    drive(&mut m, &mut core, Action::TabNext);
    assert_eq!(
        m.focused_terminal(),
        Some("a3"),
        "the next agent, in another space"
    );
    drive(&mut m, &mut core, Action::TabNext);
    assert_eq!(m.focused_terminal(), Some("a2"), "wraps");
    // The published order decides.
    drive(
        &mut m,
        &mut core,
        Event::AgentOrder {
            rows: vec!["a3".into(), "a2".into()],
            order: vec!["a3".into(), "a2".into()],
        },
    );
    drive(&mut m, &mut core, Action::TabPrev);
    assert_eq!(m.focused_terminal(), Some("a3"));
}

#[test]
fn renaming_a_tab_prompts_with_its_name_and_writes_it_trimmed() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    key(&mut m, &mut core, "f2", true);
    let Mode::Popup(PopupState::Prompt(prompt)) = m.mode() else {
        panic!("{:?}", m.mode())
    };
    assert_eq!(prompt.rename, Rename::Tab("tb1".into()));
    assert_eq!(prompt.value, "");
    // Typing goes to the field, not the terminal or the keymap.
    let fx = m.step(m.now(), Event::key(model::Chord::char('w'), true));
    assert!(!fx.handled);
    drive(
        &mut m,
        &mut core,
        Action::Popup(PopupAction::Query("  work ".into())),
    );
    key(&mut m, &mut core, "enter", false);
    assert_eq!(core.doc(None).tab("tb1").unwrap().name, "work");
    assert_eq!(*m.mode(), Mode::Normal);
    // A second rename press closes the prompt instead of stacking.
    key(&mut m, &mut core, "ctrl-g r", true);
    assert!(matches!(m.mode(), Mode::Popup(PopupState::Prompt(p)) if p.value == "work"));
    key(&mut m, &mut core, "f2", false);
    assert_eq!(*m.mode(), Mode::Normal);
}

#[test]
fn renaming_a_space_writes_its_name_and_empty_goes_back_to_automatic() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    key(&mut m, &mut core, "shift-f2", true);
    drive(
        &mut m,
        &mut core,
        Action::Popup(PopupAction::Query("herdr".into())),
    );
    drive(&mut m, &mut core, Action::Popup(PopupAction::Choose));
    assert_eq!(core.doc(None).space("sp1").unwrap().name, "herdr");
    drive(&mut m, &mut core, Action::SpaceRename);
    drive(
        &mut m,
        &mut core,
        Action::Popup(PopupAction::Query(" ".into())),
    );
    drive(&mut m, &mut core, Action::Popup(PopupAction::Choose));
    assert_eq!(core.doc(None).space("sp1").unwrap().name, "");
}

#[test]
fn moving_a_tab_and_dragging_a_divider_write_the_document() {
    let mut core = Core::new(
        doc(vec![space(
            "sp1",
            vec![
                tab("tb1", vec![pane("p1", "t1"), pane("p2", "t2")]),
                tab("tb2", vec![pane("p3", "t3")]),
            ],
        )]),
        vec![shell("t1", "/"), shell("t2", "/"), shell("t3", "/")],
    );
    let mut m = connected(&mut core);
    drive(
        &mut m,
        &mut core,
        Action::MoveTab {
            tab: "tb2".into(),
            to: 0,
        },
    );
    assert_eq!(tab_ids(&core.doc(None), "sp1"), ["tb2", "tb1"]);
    drive(
        &mut m,
        &mut core,
        Action::ResizeSplit {
            sizes: vec![
                ("p1".into(), herdr_shared::Size::Weight(3.0)),
                ("p2".into(), herdr_shared::Size::Weight(1.0)),
            ],
        },
    );
    let doc = core.doc(None);
    let root = &doc.tab("tb1").unwrap().root;
    assert_eq!(
        root.find("p1").unwrap().size(),
        Some(herdr_shared::Size::Weight(3.0))
    );
}

/// a | b
///   | c
fn three_panes() -> Core {
    Core::new(
        doc(vec![space(
            "sp1",
            vec![
                json!({"id": "tb1", "name": "", "root": {"id": "tb1", "type": "row", "children": [
                    pane("a", "ta"),
                    {"id": "col", "type": "column", "children": [pane("b", "tb"), pane("c", "tc")]}
                ]}}),
            ],
        )]),
        vec![shell("ta", "/"), shell("tb", "/"), shell("tc", "/")],
    )
}

#[test]
fn focus_by_direction_prefers_the_pane_last_focused_along_the_edge() {
    let mut core = three_panes();
    let mut m = connected(&mut core);
    drive(
        &mut m,
        &mut core,
        Event::Geometry(Geometry {
            area: Rect::new(0.0, 0.0, 201.0, 201.0),
            gap: 1.0,
            zoom: 1.0,
        }),
    );
    assert_eq!(m.focused_pane().as_deref(), Some("a"));
    key(&mut m, &mut core, "cmd-alt-right", true);
    assert_eq!(
        m.focused_pane().as_deref(),
        Some("b"),
        "unvisited: layout order"
    );
    key(&mut m, &mut core, "cmd-alt-down", true);
    assert_eq!(m.focused_pane().as_deref(), Some("c"));
    key(&mut m, &mut core, "cmd-alt-left", true);
    assert_eq!(m.focused_pane().as_deref(), Some("a"));
    key(&mut m, &mut core, "cmd-alt-right", true);
    assert_eq!(
        m.focused_pane().as_deref(),
        Some("c"),
        "the most recent along the edge"
    );
    key(&mut m, &mut core, "cmd-alt-right", true);
    assert_eq!(
        m.focused_pane().as_deref(),
        Some("c"),
        "nothing further right"
    );
    // Ctrl+L without the Neovim label is the same move.
    key(&mut m, &mut core, "ctrl-h", true);
    assert_eq!(m.focused_pane().as_deref(), Some("a"));
}

/// `three_panes`, laid out 201 × 201 with a 1 pt gap: a is 100 wide, b and c 100 × 100.
fn three_panes_laid_out() -> (Core, model::Model) {
    let mut core = three_panes();
    let mut m = connected(&mut core);
    drive(
        &mut m,
        &mut core,
        Event::Geometry(Geometry {
            area: Rect::new(0.0, 0.0, 201.0, 201.0),
            gap: 1.0,
            zoom: 1.0,
        }),
    );
    (core, m)
}

fn with_editor(mut core: Core) -> Core {
    core.terminals
        .get_mut("ta")
        .unwrap()
        .labels
        .insert(model::EDITOR_LABEL.into(), "/tmp/nvim.sock".into());
    core
}

fn weight(core: &Core, id: &str) -> Option<herdr_shared::Size> {
    core.doc(None)
        .tab("tb1")
        .unwrap()
        .root
        .find(id)
        .unwrap()
        .size()
}

#[test]
fn neovim_panes_hand_ctrl_hjkl_to_neovim_over_its_server() {
    let mut core = with_editor(three_panes());
    let mut m = connected(&mut core);
    let fx = key(&mut m, &mut core, "ctrl-l", true);
    assert_eq!(
        core.execs(),
        [(
            [
                "nvim",
                "--server",
                "/tmp/nvim.sock",
                "--remote-send",
                "<C-l>"
            ]
            .map(String::from)
            .to_vec(),
            None
        )]
    );
    // Neovim took the key (it exited 0): the pane doesn't move, nothing else is said.
    assert_eq!(m.focused_pane().as_deref(), Some("a"));
    assert!(
        fx.iter()
            .all(|e| !matches!(e, Effect::Toast(_) | Effect::NotYet(_)))
    );
    // Never through `hc keys`, nor any other call.
    assert_eq!(core.calls.len(), 1);
    key(&mut m, &mut core, "ctrl-j", true);
    assert_eq!(core.execs()[1].0[4], "<C-j>");
}

#[test]
fn neovim_at_its_edge_asks_for_the_pane_move_once_and_only_while_focused() {
    let (mut core, mut m) = {
        let mut core = with_editor(three_panes());
        let m = connected(&mut core);
        (core, m)
    };
    let label = model::NAVIGATOR_LABEL;
    let edge = core.label_elsewhere("ta", label, "4242:1000:right");
    drive(&mut m, &mut core, edge);
    assert_eq!(
        m.focused_pane().as_deref(),
        Some("b"),
        "the edge request moves"
    );
    // The same request again, as part of another change, is not a new one.
    drive(&mut m, &mut core, Action::FocusPane("a".into()));
    let again = core.label_elsewhere("ta", "other", "x");
    drive(&mut m, &mut core, again);
    assert_eq!(m.focused_pane().as_deref(), Some("a"));
    // A request from a terminal that isn't focused is noted, never acted on, even once its
    // pane is focused again.
    let elsewhere = core.label_elsewhere("tc", label, "4242:2000:left");
    drive(&mut m, &mut core, elsewhere);
    assert_eq!(m.focused_pane().as_deref(), Some("a"));
    drive(&mut m, &mut core, Action::FocusPane("c".into()));
    let unrelated = core.label_elsewhere("tc", "other", "y");
    drive(&mut m, &mut core, unrelated);
    assert_eq!(m.focused_pane().as_deref(), Some("c"));
    // Requests that aren't the form Neovim writes do nothing.
    for bad in ["4242:3000:sideways", "right", "a:b:up", "1:2:up:3"] {
        let event = core.label_elsewhere("tc", label, bad);
        drive(&mut m, &mut core, event);
        assert_eq!(m.focused_pane().as_deref(), Some("c"), "{bad:?}");
    }
    // One already set when the terminal first shows up is old: not replayed on connecting.
    let mut core = with_editor(three_panes());
    core.terminals
        .get_mut("ta")
        .unwrap()
        .labels
        .insert(label.into(), "1:1:right".into());
    let m = connected(&mut core);
    assert_eq!(m.focused_pane().as_deref(), Some("a"));
}

#[test]
fn a_labeled_neovim_that_is_gone_loses_its_label_and_the_pane_moves() {
    let mut core = with_editor(three_panes());
    core.exec = exited(1);
    let mut m = connected(&mut core);
    key(&mut m, &mut core, "ctrl-l", true);
    assert_eq!(m.focused_pane().as_deref(), Some("b"));
    assert_eq!(core.terminals["ta"].label(model::EDITOR_LABEL), None);
    // Without the label, the next Ctrl+H is a plain move.
    key(&mut m, &mut core, "ctrl-h", true);
    assert_eq!(m.focused_pane().as_deref(), Some("a"));
    assert_eq!(core.execs().len(), 1);
}

#[test]
fn nvim_that_cant_run_says_so_and_nothing_moves() {
    for result in [
        Err(error(
            ErrorKind::Other("spawn_failed".into()),
            "no such file",
        )),
        Ok(herdr_proto::methods::ExecResult {
            timed_out: true,
            ..Default::default()
        }),
    ] {
        let mut core = with_editor(three_panes());
        core.exec = result;
        let mut m = connected(&mut core);
        let fx = key(&mut m, &mut core, "ctrl-l", true);
        let toast = fx.iter().find_map(|e| match e {
            Effect::Toast(t) => Some(t.clone()),
            _ => None,
        });
        let toast = toast.expect("an error toast");
        assert_eq!(toast.level, model::Level::Error);
        assert_eq!(toast.title, "Pane navigation failed");
        assert!(
            toast.body.starts_with("Could not run nvim: "),
            "{}",
            toast.body
        );
        assert_eq!(m.focused_pane().as_deref(), Some("a"));
        assert!(core.terminals["ta"].label(model::EDITOR_LABEL).is_some());
    }
}

#[test]
fn a_remote_neovim_gets_its_keys_on_its_own_machine() {
    let mut core = Core::new(doc(vec![]), vec![]);
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![space("sp9", vec![tab("tb9", vec![pane("p9", "t9")])])]),
            1,
        ),
    );
    let mut t = shell("devbox/t9", "/");
    t.labels
        .insert(model::EDITOR_LABEL.into(), "/run/nvim.sock".into());
    core.terminals.insert(t.address.clone(), t);
    let mut m = connected(&mut core);
    let mut machine = herdr_proto::types::MachineView {
        state: herdr_proto::types::MachineState::Connected,
        ..Default::default()
    };
    machine.machine.name = "devbox".into();
    drive(&mut m, &mut core, Event::Machines(vec![machine]));
    assert_eq!(m.focused_pane().as_deref(), Some("devbox/p9"));
    key(&mut m, &mut core, "ctrl-k", true);
    assert_eq!(core.execs()[0].1.as_deref(), Some("devbox"));
}

#[test]
fn a_keyboard_resize_moves_the_nearest_divider_forty_points() {
    let (mut core, mut m) = three_panes_laid_out();
    // a | col: the divider after a goes 40 right: a 140, the column 60.
    key(&mut m, &mut core, "alt-shift-l", true);
    assert_eq!(weight(&core, "a"), Some(herdr_shared::Size::Weight(1.4)));
    assert_eq!(weight(&core, "col"), Some(herdr_shared::Size::Weight(0.6)));
    // From the column's side, left moves the same divider left.
    drive(&mut m, &mut core, Action::FocusPane("b".into()));
    key(&mut m, &mut core, "alt-shift-h", true);
    let a = match weight(&core, "a") {
        Some(herdr_shared::Size::Weight(w)) => w,
        other => panic!("{other:?}"),
    };
    assert!((a - 1.0).abs() < 1e-4, "{a}");
    // b / c: down moves their divider down.
    key(&mut m, &mut core, "alt-shift-j", true);
    assert_eq!(weight(&core, "b"), Some(herdr_shared::Size::Weight(1.4)));
    // a has nothing split above or below it: no write.
    drive(&mut m, &mut core, Action::FocusPane("a".into()));
    let version = core.version(None);
    key(&mut m, &mut core, "alt-shift-j", true);
    assert_eq!(core.version(None), version);
    // Nor while zoomed.
    drive(&mut m, &mut core, Action::PaneZoom);
    key(&mut m, &mut core, "alt-shift-l", true);
    assert_eq!(core.version(None), version);
    assert!(m.dividers().is_empty(), "a zoomed tab draws no dividers");
}

#[test]
fn dragging_a_divider_follows_the_pointer_and_writes_the_sizes() {
    let (mut core, mut m) = three_panes_laid_out();
    let dividers = m.dividers();
    assert_eq!(dividers.len(), 2);
    assert_eq!(
        (dividers[0].container.as_str(), dividers[0].index),
        ("tb1", 0)
    );
    // Move it to 50: a gets a quarter of the 200 it shares with the column.
    drive(
        &mut m,
        &mut core,
        Action::DragDivider {
            container: "tb1".into(),
            index: 0,
            at: 50.0,
        },
    );
    assert_eq!(weight(&core, "a"), Some(herdr_shared::Size::Weight(0.5)));
    assert_eq!(weight(&core, "col"), Some(herdr_shared::Size::Weight(1.5)));
    let rects = m.pane_rects();
    assert_eq!(rects[0], ("a".into(), Rect::new(0.0, 0.0, 50.0, 201.0)));
    // Where it already is: nothing to write.
    let version = core.version(None);
    drive(
        &mut m,
        &mut core,
        Action::DragDivider {
            container: "tb1".into(),
            index: 0,
            at: 50.0,
        },
    );
    assert_eq!(core.version(None), version);
}

#[test]
fn dropping_a_panes_grip_swaps_in_the_middle_and_places_beside_at_an_edge() {
    let (mut core, mut m) = three_panes_laid_out();
    let panes = |core: &Core| -> Vec<String> {
        core.doc(None)
            .tab("tb1")
            .unwrap()
            .root
            .panes()
            .iter()
            .map(|n| n.id.clone())
            .collect()
    };
    assert!(m.pane_movable("a"));
    drive(
        &mut m,
        &mut core,
        Action::RearrangePane {
            pane: "a".into(),
            target: "c".into(),
            edge: model::Edge::Center,
        },
    );
    assert_eq!(panes(&core), ["c", "b", "a"]);
    assert_eq!(
        m.focused_pane().as_deref(),
        Some("a"),
        "the moved pane is focused"
    );
    // b dropped on the bottom edge of c: below it, in a column with it.
    drive(
        &mut m,
        &mut core,
        Action::RearrangePane {
            pane: "b".into(),
            target: "c".into(),
            edge: model::Edge::Bottom,
        },
    );
    let doc = core.doc(None);
    let root = &doc.tab("tb1").unwrap().root;
    let parent = root.parent_of("b").unwrap();
    assert_eq!(parent.axis(), Some(Axis::Column));
    let ids: Vec<&str> = parent.children().iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["c", "b"]);
    assert_eq!(m.focused_pane().as_deref(), Some("b"));
    for pane in ["a", "b", "c"] {
        assert_eq!(doc.address_of(pane), Some(format!("t{pane}").as_str()));
    }
    // Not while zoomed.
    drive(&mut m, &mut core, Action::PaneZoom);
    assert!(!m.pane_movable("b"));
    let version = core.version(None);
    drive(
        &mut m,
        &mut core,
        Action::RearrangePane {
            pane: "b".into(),
            target: "a".into(),
            edge: model::Edge::Center,
        },
    );
    assert_eq!(core.version(None), version);
    // Nor in a one-pane tab.
    let mut core = three_tabs();
    let m = connected(&mut core);
    assert!(!m.pane_movable("p1"));
}

#[test]
fn zoom_shows_one_pane_moves_with_focus_and_ends_with_one_pane_left() {
    let mut core = three_panes();
    let mut m = connected(&mut core);
    key(&mut m, &mut core, "cmd-shift-enter", true);
    let tab = m.shown_tab().unwrap().clone();
    assert_eq!(m.view().zoomed_in(&tab).as_deref(), Some("a"));
    assert_eq!(m.visible_terminals(), ["ta"]);
    assert_eq!(m.pane_rects().len(), 1);
    // Focus by direction looks at the whole tab, and the zoom follows.
    key(&mut m, &mut core, "cmd-alt-right", true);
    let tab = m.shown_tab().unwrap().clone();
    assert_eq!(m.view().zoomed_in(&tab).as_deref(), Some("b"));
    // Again: unzoom.
    key(&mut m, &mut core, "cmd-shift-enter", true);
    let tab = m.shown_tab().unwrap().clone();
    assert_eq!(m.view().zoomed_in(&tab), None);
    // A one-pane tab doesn't zoom.
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    key(&mut m, &mut core, "cmd-shift-enter", true);
    let tab = m.shown_tab().unwrap().clone();
    assert_eq!(m.view().zoomed_in(&tab), None);
    // A zoom ends when the tab is back to one pane.
    let mut core = Core::new(
        doc(vec![space("sp1", vec![tab_two()])]),
        vec![shell("t1", "/"), shell("t2", "/")],
    );
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::PaneZoom);
    drive(&mut m, &mut core, Action::PaneClose);
    let tab = m.shown_tab().unwrap().clone();
    assert_eq!(tab.root.panes().len(), 1);
    assert_eq!(m.view().zoomed_in(&tab), None);
    assert!(m.view().tabs.get("tb1").is_none_or(|v| v.zoomed.is_none()));
}

fn tab_two() -> serde_json::Value {
    tab("tb1", vec![pane("p1", "t1"), pane("p2", "t2")])
}

#[test]
fn swapping_moves_the_pane_not_the_focus() {
    let mut core = three_panes();
    let mut m = connected(&mut core);
    key(&mut m, &mut core, "cmd-alt-shift-right", true);
    let doc = core.doc(None);
    let order: Vec<String> = doc
        .tab("tb1")
        .unwrap()
        .root
        .panes()
        .iter()
        .map(|n| n.id.clone())
        .collect();
    assert_eq!(order, ["b", "a", "c"]);
    assert_eq!(m.focused_pane().as_deref(), Some("a"));
    assert_eq!(core.version(None), 2);
    // a is in b's old place now, at the top of the column: nothing above it, so no write.
    key(&mut m, &mut core, "cmd-alt-shift-up", true);
    assert_eq!(core.version(None), 2);
}

#[test]
fn another_clients_edit_shows_and_view_choices_survive_where_their_ids_do() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::ShowTab("tb3".into()));
    // Herdr app closes tb2 and renames tb3.
    let mut value = core.docs[&None].0.clone();
    let tabs = value["spaces"][0]["tabs"].as_array_mut().unwrap();
    tabs.remove(1);
    tabs[1]["name"] = json!("renamed");
    let event = core.write_elsewhere(None, value);
    drive(&mut m, &mut core, event);
    assert_eq!(shown(&m).as_deref(), Some("tb3"));
    assert_eq!(m.shown_tab().unwrap().name, "renamed");
    // Then closes the tab this screen shows: the one to its left.
    let mut value = core.docs[&None].0.clone();
    value["spaces"][0]["tabs"].as_array_mut().unwrap().remove(1);
    let event = core.write_elsewhere(None, value);
    drive(&mut m, &mut core, event);
    assert_eq!(shown(&m).as_deref(), Some("tb1"));
}

#[test]
fn an_edit_racing_another_clients_write_is_replayed_on_its_copy() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    // Herdr app renames tb1, but this screen hasn't heard yet.
    let mut value = core.docs[&None].0.clone();
    value["spaces"][0]["tabs"][0]["name"] = json!("theirs");
    core.docs.insert(None, (value, 2));
    drive(
        &mut m,
        &mut core,
        Action::MoveTab {
            tab: "tb3".into(),
            to: 0,
        },
    );
    let doc = core.doc(None);
    assert_eq!(tab_ids(&doc, "sp1"), ["tb3", "tb1", "tb2"]);
    assert_eq!(doc.tab("tb1").unwrap().name, "theirs", "their edit stays");
}

#[test]
fn edits_to_an_unreadable_layout_are_refused_with_a_toast() {
    let mut core = three_tabs();
    let mut m = model::Model::new(HOME);
    let fx = m.step(
        m.now(),
        Action::MoveTab {
            tab: "tb1".into(),
            to: 1,
        },
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::Toast(_))),
        "no such tab yet: {fx:?}"
    );
    let mut m = connected(&mut core);
    // A newer version is announced, but reading it fails: the last copy stays shown.
    core.down.push(None);
    let value = core.docs[&None].0.clone();
    let event = core.write_elsewhere(None, value);
    let fx = drive(&mut m, &mut core, event);
    assert!(fx.iter().any(|e| matches!(e, Effect::WakeAt(_))), "{fx:?}");
    assert!(matches!(
        m.world().machine(None).unwrap().mirror.layout(),
        Layout::Unavailable(_)
    ));
    assert_eq!(shown(&m).as_deref(), Some("tb1"));
    let fx = drive(
        &mut m,
        &mut core,
        Action::MoveTab {
            tab: "tb1".into(),
            to: 1,
        },
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Toast(t) if t.title == "Layout unavailable"))
    );
    assert_eq!(tab_ids(&core.doc(None), "sp1"), ["tb1", "tb2", "tb3"]);
    // Launches are refused before starting a terminal the layout couldn't take.
    for action in [
        Action::TabNew,
        Action::PaneSplit(Split::Right),
        Action::PaneSplit(Split::Down),
    ] {
        let fx = drive(&mut m, &mut core, action.clone());
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Toast(t) if t.title == "Layout unavailable")),
            "{action:?}: {fx:?}"
        );
    }
    assert!(core.runs().is_empty(), "no terminal started");
    assert_eq!(core.doc(None).tab("tb1").unwrap().root.panes().len(), 1);
}

#[test]
fn a_machine_whose_reads_fail_waits_before_reading_again() {
    let mut core = three_tabs();
    core.down.push(None);
    let mut m = model::Model::new(HOME);
    drive(&mut m, &mut core, Event::Connected);
    let fx = m.step(0.5, Event::Tick);
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::Doc { .. })),
        "too soon: {fx:?}"
    );
    core.down.clear();
    core.now = model::RETRY + 0.1;
    drive(&mut m, &mut core, Event::Tick);
    assert_eq!(
        *m.world().machine(None).unwrap().mirror.layout(),
        Layout::Ready
    );
}

#[test]
fn a_pane_whose_stream_ends_goes() {
    let mut core = Core::new(
        doc(vec![space("sp1", vec![tab_two()])]),
        vec![shell("t1", "/"), shell("t2", "/")],
    );
    let mut m = connected(&mut core);
    let closed = |reason| CloseReason {
        reason,
        ..Default::default()
    };
    drive(
        &mut m,
        &mut core,
        Event::StreamClosed {
            address: "t2".into(),
            reason: closed(None),
        },
    );
    assert_eq!(
        core.doc(None).tab("tb1").unwrap().root.panes().len(),
        2,
        "a broken stream isn't an end"
    );
    drive(
        &mut m,
        &mut core,
        Event::StreamClosed {
            address: "t2".into(),
            reason: closed(Some(Reason::Exited)),
        },
    );
    assert_eq!(core.doc(None).tab("tb1").unwrap().root.panes().len(), 1);
}

#[test]
fn a_pane_whose_terminal_is_missing_goes_once_the_core_says_so() {
    let mut core = Core::new(
        doc(vec![space("sp1", vec![tab_two()])]),
        vec![shell("t1", "/"), shell("t2", "/")],
    );
    let mut m = connected(&mut core);
    // t2 vanishes: still known to the core (not closed), so its pane stays and isn't asked about
    // again until it's live again.
    let t2 = core.terminals.get_mut("t2").unwrap();
    t2.state = herdr_proto::TerminalState::Finished;
    t2.reason = Some(Reason::HostShutdown);
    let update = Event::Terminals(model::TerminalUpdate {
        terminals: vec![t2.clone()],
        ..Default::default()
    });
    drive(&mut m, &mut core, update.clone());
    let gets = |core: &Core| {
        core.calls
            .iter()
            .filter(|c| matches!(c, model::Request::Get(_)))
            .count()
    };
    assert_eq!(gets(&core), 1);
    assert_eq!(core.doc(None).tab("tb1").unwrap().root.panes().len(), 2);
    drive(&mut m, &mut core, update);
    assert_eq!(gets(&core), 1, "not asked again");
    // Gone from the core: the pane goes.
    core.terminals.remove("t2");
    let update = Event::Terminals(model::TerminalUpdate {
        resynced: true,
        terminals: core.terminals.values().cloned().collect(),
        ..Default::default()
    });
    drive(&mut m, &mut core, update);
    assert_eq!(gets(&core), 2);
    assert_eq!(core.doc(None).tab("tb1").unwrap().root.panes().len(), 1);
}

#[test]
fn a_labeled_terminal_no_pane_shows_is_adopted_without_taking_focus() {
    let mut core = three_tabs();
    let mut m = connected(&mut core);
    let mut worker = shell("w1", "/home/me/code/worker");
    worker.keep = false;
    worker.labels.insert("herdr.space".into(), "spNEW".into());
    worker.labels.insert("herdr.tab".into(), "tbNEW".into());
    core.terminals.insert("w1".into(), worker.clone());
    drive(
        &mut m,
        &mut core,
        Event::Terminals(model::TerminalUpdate {
            terminals: vec![worker],
            ..Default::default()
        }),
    );
    let doc = core.doc(None);
    let space = doc
        .space("spNEW")
        .expect("a new space under the labeled id");
    assert_eq!(space.folder, "/home/me/code/worker");
    assert_eq!(space.tabs[0].id, "tbNEW");
    assert!(core.terminals["w1"].keep, "adopted terminals are kept");
    assert_eq!(
        m.active_space().map(|s| s.id.as_str()),
        Some("sp1"),
        "focus stays"
    );
    assert_eq!(m.focused_pane().as_deref(), Some("p1"));
}

#[test]
fn remote_spaces_qualify_ids_and_launch_on_their_machine() {
    let mut core = three_tabs();
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
    let ids: Vec<String> = m.world().spaces().map(|s| s.id.clone()).collect();
    assert_eq!(ids, ["sp1", "devbox/sp9"]);
    drive(&mut m, &mut core, Action::SpaceNext);
    assert_eq!(m.focused_terminal(), Some("devbox/t9"));
    drive(&mut m, &mut core, Action::TabNew);
    let run = core.runs().pop().unwrap();
    assert_eq!(run.on.as_deref(), Some("devbox"));
    assert_eq!(run.cwd.as_deref(), Some("/srv"));
    assert_eq!(
        run.labels["herdr.space"], "sp9",
        "labels on that machine use bare ids"
    );
    assert!(!run.labels["herdr.tab"].contains('/'));
    let doc = core.doc(Some("devbox"));
    assert_eq!(
        doc.space("sp9").unwrap().tabs.len(),
        2,
        "written unqualified to devbox"
    );
    assert!(doc.tab(&run.labels["herdr.tab"]).is_some());
    assert_eq!(m.focused_terminal(), Some("devbox/t101"));
    assert_eq!(
        core.version(None),
        1,
        "this machine's document is untouched"
    );
}

#[test]
fn unknown_node_kinds_and_mod_blocks_survive_this_screens_writes() {
    let mut core = Core::new(
        doc(vec![space(
            "sp1",
            vec![
                json!({"id": "tb1", "name": "", "future": 1, "root": {"id": "tb1", "type": "row", "children": [
                    pane("p1", "t1"),
                    {"id": "m1", "type": "block", "content": {"mod": "files", "props": {"x": 1}}},
                    {"id": "w1", "type": "wormhole", "children": []}
                ]}}),
            ],
        )]),
        vec![shell("t1", "/")],
    );
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::PaneSplit(Split::Down));
    let value = &core.docs[&None].0;
    let tab = &value["spaces"][0]["tabs"][0];
    assert_eq!(tab["future"], 1);
    let doc = core.doc(None);
    let root = &doc.tab("tb1").unwrap().root;
    assert!(root.find("m1").is_some_and(Node::is_pane));
    assert!(matches!(root.find("w1").unwrap().kind, Kind::Other { .. }));
}
