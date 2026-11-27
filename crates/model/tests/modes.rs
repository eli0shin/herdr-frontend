//! Keys, modes, popups, copy mode, agents, attention, the jump list and the saved view through
//! `Model::step`.

mod support;

use herdr_proto::StatusState;
use herdr_shared::Displayed;
use model::mode::{CopyState, PopupState, Visual};
use model::{
    Action, AppCommand, Chord, CopyAction, Effect, Event, Focus, Mode, Pick, Popup, PopupAction,
    TerminalAction, TerminalCommand, View,
};
use support::*;

/// sp1: tb1 (t1), tb2 (agent a2); sp2: tb3 (agent a3), tb4 (agent a4).
fn agents() -> Core {
    Core::new(
        doc(vec![
            space(
                "sp1",
                vec![
                    tab("tb1", vec![pane("p1", "t1")]),
                    tab("tb2", vec![pane("p2", "a2")]),
                ],
            ),
            space(
                "sp2",
                vec![
                    tab("tb3", vec![pane("p3", "a3")]),
                    tab("tb4", vec![pane("p4", "a4")]),
                ],
            ),
        ]),
        vec![
            shell("t1", "/home/me/code/one"),
            agent("a2", StatusState::Working),
            agent("a3", StatusState::Idle),
            agent("a4", StatusState::Blocked),
        ],
    )
}

/// Presses keys at the model's current time; returns the last key's effects.
fn press(m: &mut model::Model, keys: &str, terminal: bool) -> model::Effects {
    let now = m.now();
    press_at(m, now, keys, terminal)
}

fn press_at(m: &mut model::Model, now: f64, keys: &str, terminal: bool) -> model::Effects {
    let mut last = model::Effects::default();
    for chord in Chord::parse_sequence(keys).unwrap() {
        last = m.step(now, Event::key(chord, terminal));
    }
    last
}

#[test]
fn a_prefix_waits_for_its_next_key_then_runs_the_sequence() {
    let mut core = agents();
    let mut m = connected(&mut core);
    let fx = press(&mut m, "ctrl-g", true);
    assert!(fx.handled);
    assert!(
        fx.list
            .contains(&Effect::WakeAt(m.now() + model::mode::PREFIX_TIMEOUT))
    );
    assert!(matches!(m.mode(), Mode::Prefix { keys, .. } if keys.len() == 1));
    key(&mut m, &mut core, "s", true);
    assert!(matches!(m.mode(), Mode::Popup(PopupState::List(l)) if l.popup == Popup::SpacePicker));
}

#[test]
fn a_prefix_times_out_and_the_next_key_is_plain_again() {
    let mut core = agents();
    let mut m = connected(&mut core);
    // The app's clock comes with every input; ticks come only when a wake is due.
    let fx = press_at(&mut m, 50.0, "ctrl-g", true);
    assert!(fx.list.contains(&Effect::WakeAt(51.0)));
    let fx = press_at(&mut m, 50.5, "t", true);
    assert!(
        fx.list.iter().any(|e| matches!(e, Effect::Call { .. })),
        "in time: tab.new"
    );
    press_at(&mut m, 60.0, "ctrl-g", true);
    m.step(60.9, Event::Tick);
    assert!(matches!(m.mode(), Mode::Prefix { .. }), "not yet");
    m.step(61.0, Event::Tick);
    assert_eq!(*m.mode(), Mode::Normal);
    // A key that arrives after the timeout with no tick in between is plain too.
    press_at(&mut m, 70.0, "ctrl-g", true);
    let fx = press_at(&mut m, 75.0, "t", true);
    assert!(!fx.handled, "a plain t goes to the terminal");
    assert!(fx.list.iter().all(|e| !matches!(e, Effect::Call { .. })));
    // The clock never runs backwards.
    m.step(1.0, Event::Tick);
    assert_eq!(m.now(), 75.0);
}

#[test]
fn an_unbound_sequence_is_dropped_whole_and_unbound_keys_go_to_the_terminal() {
    let mut core = agents();
    let mut m = connected(&mut core);
    press(&mut m, "ctrl-g", true);
    let fx = press(&mut m, "z", true);
    assert!(fx.handled, "neither ctrl-g nor z reaches the program");
    assert_eq!(*m.mode(), Mode::Normal);
    assert!(!press(&mut m, "x", true).handled);
    assert!(!press(&mut m, "ctrl-c", true).handled);
    assert!(!press(&mut m, "enter", true).handled);
}

#[test]
fn terminal_keys_apply_only_while_a_terminal_has_the_keyboard() {
    let mut core = agents();
    let mut m = connected(&mut core);
    let fx = press(&mut m, "ctrl-g ctrl-l", true);
    assert!(fx.list.contains(&Effect::Terminal {
        address: "t1".into(),
        command: TerminalCommand::Action(TerminalAction::Send("\u{c}".into())),
    }));
    let fx = press(&mut m, "alt-left", true);
    assert!(fx.list.contains(&Effect::Terminal {
        address: "t1".into(),
        command: TerminalCommand::Action(TerminalAction::Send("\u{1b}b".into())),
    }));
    let fx = press(&mut m, "cmd-f", true);
    assert!(fx.list.contains(&Effect::Terminal {
        address: "t1".into(),
        command: TerminalCommand::Action(TerminalAction::Search),
    }));
    assert!(
        !press(&mut m, "cmd-f", false).handled,
        "no terminal has the keyboard"
    );
}

#[test]
fn app_keys_become_app_commands() {
    let mut core = agents();
    let mut m = connected(&mut core);
    for (keys, command) in [
        ("cmd-q", AppCommand::Quit),
        ("cmd-h", AppCommand::Hide),
        ("cmd-alt-h", AppCommand::HideOthers),
        ("cmd-=", AppCommand::ZoomIn),
        ("cmd--", AppCommand::ZoomOut),
        ("cmd-0", AppCommand::ZoomReset),
        ("cmd-b", AppCommand::ToggleSidebar),
        ("ctrl-g b", AppCommand::ToggleSidebar),
        ("cmd-shift-l", AppCommand::LayoutSwitch),
        ("ctrl-g d", AppCommand::CloseWindow),
    ] {
        let fx = press(&mut m, keys, true);
        assert!(fx.list.contains(&Effect::App(command)), "{keys}: {fx:?}");
    }
}

#[test]
fn the_escape_hatch_works_in_every_mode() {
    let mut core = agents();
    let mut m = connected(&mut core);
    for setup in ["cmd-p", "ctrl-g", "ctrl-g [", "ctrl-;", "f2"] {
        press(&mut m, setup, true);
        let fx = press(&mut m, "ctrl-alt-shift-escape", true);
        assert!(fx.list.contains(&Effect::App(AppCommand::Reset)), "{setup}");
        assert_eq!(*m.mode(), Mode::Normal, "{setup}");
    }
}

#[test]
fn list_popups_move_wrap_pick_and_close() {
    let mut core = agents();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::SpaceSelect(2));
    let fx = drive(&mut m, &mut core, Action::Open(Popup::SpacePicker));
    assert!(fx.contains(&Effect::Focus(Focus::Popup)));
    let list = |m: &model::Model| match m.mode() {
        Mode::Popup(PopupState::List(l)) => l.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(list(&m).cursor, 1, "opens on the shown space");
    assert_eq!(list(&m).rows, 2);
    press(&mut m, "down", false);
    assert_eq!(list(&m).cursor, 0, "wraps");
    press(&mut m, "ctrl-p", false);
    assert_eq!(list(&m).cursor, 1);
    // Typing filters: the field takes the key, the app reports the query and the rows.
    assert!(!press(&mut m, "a", false).handled);
    m.step(m.now(), Action::Popup(PopupAction::Query("one".into())));
    m.step(m.now(), Event::PopupRows(1));
    assert_eq!(
        (list(&m).query.as_str(), list(&m).cursor, list(&m).rows),
        ("one", 0, 1)
    );
    let fx = press(&mut m, "enter", false);
    assert!(fx.list.contains(&Effect::Pick {
        popup: Popup::SpacePicker,
        how: Pick::Choose,
        cursor: 0,
        query: "one".into(),
    }));
    assert_eq!(*m.mode(), Mode::Normal);
    assert!(
        fx.list
            .iter()
            .any(|e| matches!(e, Effect::Focus(Focus::Pane(_))))
    );
    // Esc closes; opening it again while open closes it too.
    press(&mut m, "cmd-p", false);
    press(&mut m, "escape", false);
    assert_eq!(*m.mode(), Mode::Normal);
    press(&mut m, "cmd-p", false);
    press(&mut m, "cmd-p", false);
    assert_eq!(*m.mode(), Mode::Normal);
}

#[test]
fn the_picker_closes_a_tab_and_the_machine_filter_toggles_without_closing() {
    let mut core = agents();
    let mut m = connected(&mut core);
    press(&mut m, "cmd-p", false);
    m.step(m.now(), Event::PopupRows(4));
    press(&mut m, "down", false);
    let fx = press(&mut m, "ctrl-d", false);
    assert!(fx.list.iter().any(|e| matches!(
        e,
        Effect::Pick {
            how: Pick::CloseItem,
            cursor: 1,
            ..
        }
    )));
    assert!(matches!(m.mode(), Mode::Popup(_)), "still open");
    press(&mut m, "escape", false);
    press(&mut m, "ctrl-g /", false);
    m.step(m.now(), Event::PopupRows(3));
    press(&mut m, "j", false);
    let fx = press(&mut m, "space", false);
    assert!(fx.list.iter().any(|e| matches!(
        e,
        Effect::Pick {
            popup: Popup::AgentMachines,
            how: Pick::Toggle,
            cursor: 1,
            ..
        }
    )));
    press(&mut m, "q", false);
    assert_eq!(*m.mode(), Mode::Normal);
}

#[test]
fn closing_a_tab_or_pane_with_a_popup_open_closes_the_popup_instead() {
    let mut core = agents();
    let mut m = connected(&mut core);
    press(&mut m, "cmd-p", false);
    key(&mut m, &mut core, "cmd-w", false);
    assert_eq!(*m.mode(), Mode::Normal);
    assert!(core.kills().is_empty());
    assert_eq!(core.doc(None).space("sp1").unwrap().tabs.len(), 2);
    // Other window keys still run over a popup.
    press(&mut m, "cmd-p", false);
    let fx = press(&mut m, "cmd-q", false);
    assert!(fx.list.contains(&Effect::App(AppCommand::Quit)));
}

#[test]
fn agent_jump_focuses_a_row_by_its_label_and_ignores_modified_keys() {
    let mut core = agents();
    let mut m = connected(&mut core);
    assert_eq!(
        m.agent_rows(),
        ["a2", "a3", "a4"],
        "layout order before the list publishes"
    );
    press(&mut m, "ctrl-;", true);
    assert!(matches!(m.mode(), Mode::Popup(PopupState::Jump(_))));
    assert!(
        press(&mut m, "cmd-t", true).handled,
        "the bar holds the keyboard"
    );
    assert!(
        matches!(m.mode(), Mode::Popup(PopupState::Jump(_))),
        "and ignores it"
    );
    press(&mut m, "3", true);
    assert_eq!(*m.mode(), Mode::Normal);
    assert_eq!(m.focused_terminal(), Some("a4"));
    // Esc and Ctrl+; leave.
    press(&mut m, "ctrl-;", true);
    press(&mut m, "ctrl-;", true);
    assert_eq!(*m.mode(), Mode::Normal);
    press(&mut m, "ctrl-;", true);
    press(&mut m, "escape", true);
    assert_eq!(*m.mode(), Mode::Normal);
    // A key no label has is dropped.
    press(&mut m, "ctrl-;", true);
    press(&mut m, "9", true);
    assert!(matches!(m.mode(), Mode::Popup(PopupState::Jump(j)) if j.typed.is_empty()));
}

#[test]
fn past_36_rows_the_first_letter_waits_for_the_second() {
    let mut core = agents();
    // Agents with no pane: a jump to one is a peek.
    let many: Vec<String> = (0..40).map(|i| format!("z{i}")).collect();
    for address in &many {
        core.terminals
            .insert(address.clone(), agent(address, StatusState::Idle));
    }
    let mut m = connected(&mut core);
    let publish = |m: &mut model::Model, rows: &[String]| {
        m.step(
            m.now(),
            Event::AgentOrder {
                rows: rows.to_vec(),
                order: rows.to_vec(),
            },
        );
    };
    publish(&mut m, &many[..11]);
    press(&mut m, "ctrl-;", true);
    let fx = press(&mut m, "a", true);
    assert!(
        fx.list
            .contains(&Effect::NotYet(Action::Peek("z10".into()))),
        "11 rows: one letter"
    );
    publish(&mut m, &many);
    press(&mut m, "ctrl-;", true);
    press(&mut m, "a", true);
    assert!(matches!(m.mode(), Mode::Popup(PopupState::Jump(j)) if j.typed == "a"));
    let fx = press(&mut m, "s", true);
    assert!(
        fx.list
            .contains(&Effect::NotYet(Action::Peek("z11".into()))),
        "as: the twelfth row"
    );
    assert_eq!(*m.mode(), Mode::Normal);
}

#[test]
fn copy_mode_selects_copies_and_leaves() {
    let mut core = agents();
    let mut m = connected(&mut core);
    let t = |command| Effect::Terminal {
        address: "t1".into(),
        command,
    };
    let fx = press(&mut m, "ctrl-g [", true);
    assert!(fx.list.contains(&t(TerminalCommand::CopyStart)));
    assert!(fx.list.contains(&Effect::Focus(Focus::Popup)));
    let copy = |m: &model::Model| match m.mode() {
        Mode::Copy(c) => c.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        copy(&m),
        CopyState {
            address: "t1".into(),
            visual: None
        }
    );
    let fx = press(&mut m, "k", true);
    assert!(
        fx.list
            .contains(&t(TerminalCommand::CopyMove(CopyAction::Up)))
    );
    assert!(
        press(&mut m, "x", true).handled,
        "copy mode holds the keyboard"
    );
    // v selects, v again drops; V selects lines; Esc drops, Esc again leaves.
    let fx = press(&mut m, "v", true);
    assert!(
        fx.list
            .contains(&t(TerminalCommand::CopySelect(Some(Visual::Chars))))
    );
    let fx = press(&mut m, "v", true);
    assert!(fx.list.contains(&t(TerminalCommand::CopySelect(None))));
    press(&mut m, "shift-v", true);
    assert_eq!(copy(&m).visual, Some(Visual::Lines));
    press(&mut m, "escape", true);
    assert_eq!(copy(&m).visual, None);
    let fx = press(&mut m, "escape", true);
    assert!(fx.list.contains(&t(TerminalCommand::CopyEnd)));
    assert_eq!(*m.mode(), Mode::Normal);
    // y with a selection copies, then leaves; without, just leaves.
    press(&mut m, "ctrl-g [", true);
    press(&mut m, "space", true);
    let fx = press(&mut m, "y", true);
    assert_eq!(
        fx.list
            .iter()
            .filter(|e| matches!(e, Effect::Terminal { .. }))
            .cloned()
            .collect::<Vec<_>>(),
        [t(TerminalCommand::CopyYank), t(TerminalCommand::CopyEnd)]
    );
    press(&mut m, "ctrl-g [", true);
    let fx = press(&mut m, "enter", true);
    assert!(!fx.list.contains(&t(TerminalCommand::CopyYank)));
    assert!(fx.list.contains(&t(TerminalCommand::CopyEnd)));
    press(&mut m, "ctrl-g [", true);
    press(&mut m, "ctrl-c", true);
    assert_eq!(*m.mode(), Mode::Normal);
}

#[test]
fn copy_mode_needs_a_focused_terminal_and_ends_when_focus_leaves_it() {
    let mut core = Core::new(doc(vec![]), vec![]);
    let mut m = connected(&mut core);
    let fx = press(&mut m, "ctrl-g [", true);
    assert!(
        fx.list
            .iter()
            .any(|e| matches!(e, Effect::Toast(t) if t.title == "Copy mode"))
    );
    assert_eq!(*m.mode(), Mode::Normal);
    let mut core = agents();
    let mut m = connected(&mut core);
    press(&mut m, "ctrl-g [", true);
    let fx = press(&mut m, "cmd-]", true);
    assert!(fx.list.contains(&Effect::Terminal {
        address: "t1".into(),
        command: TerminalCommand::CopyEnd
    }));
    assert_eq!(*m.mode(), Mode::Normal);
}

#[test]
fn closing_a_tab_or_pane_in_copy_mode_leaves_copy_mode_instead() {
    let mut core = agents();
    let mut m = connected(&mut core);
    for keys in ["cmd-w", "cmd-shift-w"] {
        press(&mut m, "ctrl-g [", true);
        let fx = key(&mut m, &mut core, keys, true);
        assert!(
            fx.contains(&Effect::Terminal {
                address: "t1".into(),
                command: TerminalCommand::CopyEnd
            }),
            "{keys}: {fx:?}"
        );
        assert_eq!(*m.mode(), Mode::Normal, "{keys}");
    }
    assert!(core.kills().is_empty(), "nothing closed");
    assert_eq!(core.doc(None).space("sp1").unwrap().tabs.len(), 2);
}

#[test]
fn agents_step_in_list_order_and_rows_select_by_number() {
    let mut core = agents();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::AgentNext);
    assert_eq!(
        m.focused_terminal(),
        Some("a2"),
        "from a shell: the first agent"
    );
    key(&mut m, &mut core, "cmd-]", true);
    assert_eq!(m.focused_terminal(), Some("a3"));
    key(&mut m, &mut core, "ctrl-g j", true);
    assert_eq!(m.focused_terminal(), Some("a4"));
    key(&mut m, &mut core, "ctrl-g j", true);
    assert_eq!(m.focused_terminal(), Some("a2"), "wraps");
    key(&mut m, &mut core, "cmd-[", true);
    assert_eq!(m.focused_terminal(), Some("a4"));
    key(&mut m, &mut core, "ctrl-g shift-2", true);
    assert_eq!(m.focused_terminal(), Some("a3"));
    key(&mut m, &mut core, "ctrl-g !", true);
    assert_eq!(m.focused_terminal(), Some("a2"));
    key(&mut m, &mut core, "ctrl-g shift-9", true);
    assert_eq!(m.focused_terminal(), Some("a2"), "no ninth row");
    // The published order wins, and agents it leaves out come after it.
    drive(
        &mut m,
        &mut core,
        Event::AgentOrder {
            rows: vec!["a4".into(), "a3".into()],
            order: vec!["a4".into(), "a3".into()],
        },
    );
    assert_eq!(m.agent_order(), ["a4", "a3", "a2"]);
    key(&mut m, &mut core, "ctrl-g shift-1", true);
    assert_eq!(m.focused_terminal(), Some("a4"));
}

#[test]
fn the_jump_list_goes_back_and_forward_over_tabs_seen_for_a_second() {
    let mut core = agents();
    let mut m = connected(&mut core);
    // Tabs shown two seconds apart, with no ticks in between.
    let show = |m: &mut model::Model, core: &mut Core, tab: &str| {
        core.now += 2.0;
        drive(m, core, Action::ShowTab(tab.into()));
    };
    show(&mut m, &mut core, "tb2");
    show(&mut m, &mut core, "tb3");
    show(&mut m, &mut core, "tb4");
    key(&mut m, &mut core, "ctrl-g o", true);
    assert_eq!(m.shown_tab().unwrap().id, "tb3");
    key(&mut m, &mut core, "ctrl-g o", true);
    assert_eq!(m.shown_tab().unwrap().id, "tb2");
    key(&mut m, &mut core, "ctrl-g i", true);
    assert_eq!(m.shown_tab().unwrap().id, "tb3");
    key(&mut m, &mut core, "ctrl-g i", true);
    assert_eq!(m.shown_tab().unwrap().id, "tb4");
}

#[test]
fn attention_alerts_brings_settled_tabs_back_and_looking_marks_done_seen() {
    let mut core = agents();
    let mut view = View::default();
    view.settle("tb3", 5);
    let mut m = model::Model::with_view(HOME, view);
    drive(&mut m, &mut core, Event::Connected);
    let snapshot = core.snapshot();
    drive(&mut m, &mut core, snapshot);
    assert!(m.view().settled("tb3"));
    // a3 goes blocked: always an alert, and its settled tab comes back.
    let mut a3 = agent("a3", StatusState::Blocked);
    a3.turn = 1;
    let fx = drive(
        &mut m,
        &mut core,
        Event::Terminals(model::TerminalUpdate {
            terminals: vec![a3],
            ..Default::default()
        }),
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Alert(r) if r.terminal == "a3"))
    );
    assert!(!m.view().settled("tb3"));
    // a2 finishes while the window is in the background: done until looked at.
    drive(&mut m, &mut core, Event::WindowActive(false));
    let mut a2 = agent("a2", StatusState::Idle);
    a2.turn = 1;
    drive(
        &mut m,
        &mut core,
        Event::Terminals(model::TerminalUpdate {
            terminals: vec![a2],
            ..Default::default()
        }),
    );
    assert_eq!(m.attention().displayed("a2"), Some(Displayed::Done));
    drive(&mut m, &mut core, Action::FocusAgent("a2".into()));
    assert_eq!(
        m.attention().displayed("a2"),
        Some(Displayed::Done),
        "the window is in the back"
    );
    drive(&mut m, &mut core, Event::WindowActive(true));
    assert_eq!(m.attention().displayed("a2"), Some(Displayed::Idle));
}

#[test]
fn the_view_is_saved_when_it_changes_and_restores_where_its_ids_still_are() {
    let mut core = agents();
    let mut m = connected(&mut core);
    let fx = drive(&mut m, &mut core, Action::ShowTab("tb4".into()));
    assert!(fx.contains(&Effect::SaveView));
    core.now = 9.0;
    let fx = drive(&mut m, &mut core, Event::Tick);
    assert!(!fx.contains(&Effect::SaveView), "nothing changed");
    let saved = m.saved_view();
    assert_eq!(saved.active.as_deref(), Some("sp2"));
    assert_eq!(saved.spaces["sp2"], "tb4");
    let json = saved.to_json();
    let mut m = model::Model::with_view(HOME, View::from_json(&json).unwrap());
    drive(&mut m, &mut core, Event::Connected);
    assert_eq!(m.shown_tab().unwrap().id, "tb4");
    assert_eq!(m.focused_terminal(), Some("a4"));
}

#[test]
fn saved_attention_comes_back_while_the_agent_says_the_same_word() {
    let mut core = Core::new(
        doc(vec![space(
            "sp1",
            vec![
                tab("tb1", vec![pane("p1", "t1")]),
                tab("tb2", vec![pane("p2", "a2"), pane("p3", "a3")]),
            ],
        )]),
        vec![
            shell("t1", "/"),
            agent("a2", StatusState::Done),
            agent("a3", StatusState::Idle),
        ],
    );
    let mut view = View::default();
    for (address, word) in [("a2", StatusState::Done), ("a3", StatusState::Working)] {
        view.seen.insert(
            address.into(),
            model::view::SeenRecord {
                word: Some(word),
                seen: false,
            },
        );
    }
    let mut m = model::Model::with_view(HOME, view);
    drive(&mut m, &mut core, Event::Connected);
    let snapshot = core.snapshot();
    drive(&mut m, &mut core, snapshot);
    assert_eq!(
        m.attention().displayed("a2"),
        Some(Displayed::Done),
        "still unseen"
    );
    assert_eq!(
        m.attention().displayed("a3"),
        Some(Displayed::Idle),
        "a new word: no news"
    );
    let saved = m.saved_view();
    assert!(!saved.seen["a2"].seen);
}

#[test]
fn a_machine_not_read_yet_keeps_its_saved_choices() {
    let mut core = agents();
    let mut view = View {
        active: Some("devbox/sp9".into()),
        ..Default::default()
    };
    view.spaces.insert("devbox/sp9".into(), "devbox/tb9".into());
    view.focus("devbox/tb9", "devbox/p9");
    let mut m = model::Model::with_view(HOME, view);
    drive(&mut m, &mut core, Event::Connected);
    let snapshot = core.snapshot();
    drive(&mut m, &mut core, snapshot);
    // devbox isn't connected yet: this machine's first space shows meanwhile, but the choice
    // stays saved for when devbox's layout arrives.
    let saved = m.saved_view();
    assert_eq!(
        saved.spaces.get("devbox/sp9").map(String::as_str),
        Some("devbox/tb9")
    );
    assert_eq!(
        saved.tabs["devbox/tb9"].focused.as_deref(),
        Some("devbox/p9")
    );
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![space(
                "sp9",
                vec![tab("tb9", vec![pane("p0", "t0"), pane("p9", "t9")])],
            )]),
            1,
        ),
    );
    core.terminals
        .insert("devbox/t0".into(), shell("devbox/t0", "/"));
    core.terminals
        .insert("devbox/t9".into(), shell("devbox/t9", "/"));
    let mut devbox = herdr_proto::types::MachineView {
        state: herdr_proto::types::MachineState::Connected,
        ..Default::default()
    };
    devbox.machine.name = "devbox".into();
    assert_eq!(
        saved.active.as_deref(),
        Some("devbox/sp9"),
        "not overwritten by the fallback"
    );
    assert_eq!(m.active_space().map(|s| s.id.as_str()), Some("sp1"));
    drive(&mut m, &mut core, Event::Machines(vec![devbox]));
    assert_eq!(
        m.active_space().map(|s| s.id.as_str()),
        Some("devbox/sp9"),
        "shown once read"
    );
    assert_eq!(m.shown_tab().map(|t| t.id.as_str()), Some("devbox/tb9"));
    assert_eq!(
        m.focused_pane().as_deref(),
        Some("devbox/p9"),
        "the saved focus, not the first pane"
    );
}

#[test]
fn a_saved_remote_space_gone_by_the_time_its_machine_loads_leaves_the_screen_where_it_is() {
    let mut core = agents();
    let view = View {
        active: Some("devbox/sp9".into()),
        ..Default::default()
    };
    let mut m = model::Model::with_view(HOME, view);
    drive(&mut m, &mut core, Event::Connected);
    let snapshot = core.snapshot();
    drive(&mut m, &mut core, snapshot);
    assert_eq!(
        m.active_space().map(|s| s.id.as_str()),
        Some("sp1"),
        "the fallback meanwhile"
    );
    // Closed elsewhere while this screen was away: devbox has only sp8 now.
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![space("sp8", vec![tab("tb8", vec![pane("p8", "t8")])])]),
            1,
        ),
    );
    core.terminals
        .insert("devbox/t8".into(), shell("devbox/t8", "/"));
    let mut devbox = herdr_proto::types::MachineView {
        state: herdr_proto::types::MachineState::Connected,
        ..Default::default()
    };
    devbox.machine.name = "devbox".into();
    drive(&mut m, &mut core, Event::Machines(vec![devbox]));
    assert!(m.world().loaded(Some("devbox")));
    assert_eq!(m.active_space().map(|s| s.id.as_str()), Some("sp1"));
    assert_eq!(m.view().active.as_deref(), Some("sp1"));
}

#[test]
fn unbuilt_features_say_not_yet() {
    let mut core = agents();
    let mut m = connected(&mut core);
    let fx = press(&mut m, "cmd-r", true);
    assert!(fx.list.contains(&Effect::NotYet(Action::Review)));
    for action in [
        Action::Peek("a2".into()),
        Action::FoldSubagents("a2".into()),
        Action::RenamePaneOf("p1".into()),
        Action::OpenLink {
            pane: "p1".into(),
            link: "x.rs:3".into(),
        },
    ] {
        let fx = m.step(m.now(), action.clone());
        assert!(fx.list.contains(&Effect::NotYet(action)));
    }
}

/// Every binding of the default keymap, pressed in every key context and mode, steps without
/// panicking and leaves a mode that makes sense.
#[test]
fn every_binding_runs_in_every_mode() {
    let keymap = model::Keymap::eli();
    for setup in [
        None,
        Some("cmd-p"),
        Some("ctrl-g ["),
        Some("ctrl-;"),
        Some("f2"),
        Some("ctrl-g x"),
    ] {
        for binding in keymap.bindings() {
            for terminal in [false, true] {
                let mut core = agents();
                let mut m = connected(&mut core);
                if let Some(setup) = setup {
                    key(&mut m, &mut core, setup, true);
                }
                for chord in &binding.keys {
                    drive(&mut m, &mut core, Event::key(*chord, terminal));
                }
                core.now = 10.0;
                drive(&mut m, &mut core, Event::Tick);
                assert!(
                    !matches!(m.mode(), Mode::Prefix { .. }),
                    "{setup:?} then {:?}: a prefix outlived its timeout",
                    binding.keys
                );
                let doc = core.doc(None);
                assert!(doc.spaces.len() <= 2, "{:?}", binding.keys);
            }
        }
    }
}

fn keymap(json: &str) -> model::Keymap {
    model::Keymap::from_json(json).unwrap()
}

#[test]
fn a_prefix_reads_either_spelling_of_a_shifted_key_and_shows_what_waits() {
    let mut core = agents();
    let mut m = connected(&mut core);
    // Shift+2 types `@`; Eli binds both `ctrl-g shift-2` and `ctrl-g @` to the second row.
    let shift_2 = model::Press {
        chord: Chord::parse("shift-2").unwrap(),
        typed: Some(Chord::parse("@").unwrap()),
    };
    let only_at = model::Press::new(Chord::parse("@").unwrap());
    for second in [shift_2, only_at] {
        drive(&mut m, &mut core, Action::FocusPane("p1".into()));
        press(&mut m, "ctrl-g", true);
        let waiting: Vec<String> = m
            .mode()
            .pending_keys()
            .unwrap()
            .iter()
            .map(|p| p.to_string())
            .collect();
        assert_eq!(waiting, ["ctrl-g"]);
        drive(
            &mut m,
            &mut core,
            Event::Key {
                press: second,
                terminal: true,
            },
        );
        assert_eq!(m.focused_terminal(), Some(m.agent_rows()[1].as_str()));
        assert_eq!(m.mode().pending_keys(), None);
    }
}

#[test]
fn a_key_the_file_gives_the_terminal_reaches_the_program_over_a_global_binding() {
    let mut core = agents();
    let mut m = connected(&mut core);
    m.set_keymap(keymap(
        r#"{"global": {"ctrl-h": "pane.go:left", "cmd-t": "tab.new"},
            "terminal": {"ctrl-h": "terminal"}}"#,
    ));
    assert!(
        !press(&mut m, "ctrl-h", true).handled,
        "the program gets it"
    );
    assert!(
        press(&mut m, "ctrl-h", false).handled,
        "no terminal: global"
    );
    assert!(press(&mut m, "cmd-t", true).handled);
    // Keys the file leaves out are unbound: Eli's cmd-d isn't there.
    assert!(!press(&mut m, "cmd-d", true).handled);
}

#[test]
fn a_key_files_popup_and_copy_keys_come_before_their_own() {
    let mut core = agents();
    let mut m = connected(&mut core);
    m.set_keymap(keymap(
        r#"{"global": {"cmd-p": "mod.open:picker", "ctrl-g [": "mod.open:copy-mode", "cmd-t": "tab.new"},
            "popup": {"ctrl-j": "popup.down", "escape": "popup.choose"},
            "copy": {"x": "copy_mode.yank", "j": "copy_mode.up"}}"#,
    ));
    press(&mut m, "cmd-p", true);
    m.step(m.now(), Event::PopupRows(3));
    press(&mut m, "ctrl-j", false);
    let Mode::Popup(PopupState::List(list)) = m.mode() else {
        panic!("{:?}", m.mode())
    };
    assert_eq!(list.cursor, 1);
    // The file's Escape picks instead of closing.
    let fx = press(&mut m, "escape", false);
    assert!(fx.list.iter().any(|e| matches!(e, Effect::Pick { .. })));
    assert_eq!(*m.mode(), Mode::Normal);

    drive(&mut m, &mut core, Action::FocusPane("p1".into()));
    press(&mut m, "ctrl-g [", true);
    assert!(matches!(m.mode(), Mode::Copy(_)));
    let fx = press(&mut m, "j", true);
    assert!(fx.list.contains(&Effect::Terminal {
        address: "t1".into(),
        command: TerminalCommand::CopyMove(CopyAction::Up),
    }));
    // Copy mode's own keys still work where the file doesn't bind them.
    let fx = press(&mut m, "l", true);
    assert!(fx.list.contains(&Effect::Terminal {
        address: "t1".into(),
        command: TerminalCommand::CopyMove(CopyAction::Right),
    }));
    press(&mut m, "x", true);
    assert_eq!(*m.mode(), Mode::Normal, "x yanks and leaves");
}

#[test]
fn elis_copy_mode_escape_drops_the_selection_then_leaves() {
    let mut core = agents();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::FocusPane("p1".into()));
    press(&mut m, "ctrl-g [", true);
    press(&mut m, "v", true);
    press(&mut m, "escape", true);
    assert!(matches!(
        m.mode(),
        Mode::Copy(CopyState { visual: None, .. })
    ));
    press(&mut m, "escape", true);
    assert_eq!(*m.mode(), Mode::Normal);
}
