//! The review view (`cmd-r`, `feature-inventory.md` §13) and the peek (§15) through
//! `Model::step`, against a fake core.

mod support;

use herdr_proto::types::{MachineState, MachineView};
use herdr_proto::{Reason, StatusState, TerminalState};
use herdr_shared::Displayed;
use model::peek::{Go, PeekBar};
use model::review::{REVIEW_ARGV, REVIEW_NAME};
use model::{
    Action, Call, CallId, Effect, Event, Focus, Model, Popup, REVIEW_LABEL, TerminalAction,
    TerminalCommand, TerminalUpdate,
};
use serde_json::json;
use support::*;

fn devbox() -> MachineView {
    let mut view = MachineView {
        state: MachineState::Connected,
        ..Default::default()
    };
    view.machine.name = "devbox".into();
    view
}

/// This machine: sp1 (tb1: p1 t1, tb2: p2 the agent a2). devbox: sp8 (tb8: p8 t8) and sp9
/// (tb9: p9 the agent a9). s5 is a2's sub-agent and s6 is s5's, s7 a9's; none has a pane.
fn world() -> Core {
    let mut a2 = agent("a2", StatusState::Idle);
    a2.name = Some("pi".into());
    let mut s5 = agent("s5", StatusState::Working);
    s5.parent = Some("a2".into());
    s5.name = Some("explore".into());
    let mut s6 = agent("s6", StatusState::Blocked);
    s6.parent = Some("s5".into());
    let mut s7 = agent("devbox/s7", StatusState::Idle);
    s7.parent = Some("devbox/a9".into());
    let mut core = Core::new(
        doc(vec![space(
            "sp1",
            vec![
                tab("tb1", vec![pane("p1", "t1")]),
                tab("tb2", vec![pane("p2", "a2")]),
            ],
        )]),
        vec![
            shell("t1", "/home/me/code/one"),
            a2,
            s5,
            s6,
            s7,
            shell("devbox/t8", "/srv"),
            agent("devbox/a9", StatusState::Idle),
        ],
    );
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![
                space("sp8", vec![tab("tb8", vec![pane("p8", "t8")])]),
                space("sp9", vec![tab("tb9", vec![pane("p9", "a9")])]),
            ]),
            1,
        ),
    );
    core
}

fn connected_with_devbox(core: &mut Core) -> Model {
    let mut m = connected(core);
    drive(&mut m, core, Event::Machines(vec![devbox()]));
    m
}

fn review(address: &str) -> herdr_proto::Terminal {
    let mut t = shell(address, "/home/me/code/one");
    t.labels.insert(REVIEW_LABEL.into(), "1".into());
    t
}

fn ended(core: &mut Core, address: &str) -> Event {
    let t = core.terminals.get_mut(address).unwrap();
    t.state = TerminalState::Finished;
    t.reason = Some(Reason::Exited);
    Event::Terminals(TerminalUpdate {
        terminals: vec![t.clone()],
        ..Default::default()
    })
}

// ---- Review ----------------------------------------------------------------------------------

#[test]
fn cmd_r_goes_to_this_machines_review_and_back() {
    let mut core = world();
    core.terminals.insert("t3".into(), review("t3"));
    core.terminals
        .insert("devbox/t4".into(), review("devbox/t4"));
    core.docs.insert(
        None,
        (
            doc(vec![space(
                "sp1",
                vec![
                    tab("tb1", vec![pane("p1", "t1")]),
                    tab("tb2", vec![pane("p2", "a2")]),
                    tab("tb3", vec![pane("p3", "t3")]),
                ],
            )]),
            1,
        ),
    );
    let mut m = connected_with_devbox(&mut core);
    assert_eq!(m.focused_terminal(), Some("t1"));
    key(&mut m, &mut core, "cmd-r", true);
    assert_eq!(m.focused_terminal(), Some("t3"), "this machine's review");
    assert!(core.runs().is_empty(), "nothing started");
    key(&mut m, &mut core, "cmd-r", true);
    assert_eq!(m.focused_terminal(), Some("t1"), "back where it came from");
    // From an agent's tab, back goes there.
    drive(&mut m, &mut core, Action::FocusAgent("a2".into()));
    key(&mut m, &mut core, "cmd-r", true);
    key(&mut m, &mut core, "cmd-r", true);
    assert_eq!(m.focused_terminal(), Some("a2"));
    // Back to a terminal that ended stays on the review.
    key(&mut m, &mut core, "cmd-r", true);
    let gone = ended(&mut core, "a2");
    drive(&mut m, &mut core, gone);
    assert_eq!(m.focused_terminal(), Some("t3"));
    key(&mut m, &mut core, "cmd-r", true);
    assert_eq!(m.focused_terminal(), Some("t3"));
}

#[test]
fn a_space_on_another_machine_uses_that_machines_review() {
    let mut core = world();
    core.terminals.insert("t3".into(), review("t3"));
    core.terminals
        .insert("devbox/t4".into(), review("devbox/t4"));
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![space(
                "sp8",
                vec![
                    tab("tb8", vec![pane("p8", "t8")]),
                    tab("tb4", vec![pane("p4", "t4")]),
                ],
            )]),
            1,
        ),
    );
    let mut m = connected_with_devbox(&mut core);
    drive(&mut m, &mut core, Action::FocusPane("devbox/p8".into()));
    key(&mut m, &mut core, "cmd-r", true);
    assert_eq!(m.focused_terminal(), Some("devbox/t4"));
    key(&mut m, &mut core, "cmd-r", true);
    assert_eq!(m.focused_terminal(), Some("devbox/t8"));
}

#[test]
fn without_one_cmd_r_starts_a_labelled_review_tab_in_the_shown_space() {
    let mut core = world();
    // An ended review, and a running one on another machine, don't count.
    let mut old = review("t3");
    old.state = TerminalState::Finished;
    core.terminals.insert("t3".into(), old);
    core.terminals
        .insert("devbox/t4".into(), review("devbox/t4"));
    let mut m = connected_with_devbox(&mut core);
    m.set_shell("/opt/homebrew/bin/fish");
    key(&mut m, &mut core, "cmd-r", true);
    let runs = core.runs();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    // In the login shell, as Herdr app's `tab.new` runs a program, so `hc` and `review` are
    // on PATH; the script ends the terminal itself, so the shell after it never stays.
    let review = REVIEW_ARGV.map(str::to_owned).to_vec();
    assert_eq!(
        run.argv,
        model::shell::login("/opt/homebrew/bin/fish", review.clone())
    );
    assert_eq!(run.argv[4..], review);
    assert!(review[2].starts_with("hc label \"$HC_TERMINAL_ID\" dotfiles.review=1"));
    assert!(review[2].ends_with("review; hc kill \"$HC_TERMINAL_ID\""));
    assert_eq!(run.on, None);
    assert_eq!(run.cwd.as_deref(), Some("/home/me/code/one"), "where t1 is");
    assert_eq!(run.labels["herdr.space"], "sp1");
    let address = m.focused_terminal().unwrap().to_owned();
    let tab = m.shown_tab().unwrap();
    assert_eq!(tab.name, REVIEW_NAME);
    assert_eq!(
        tab_ids(&core.doc(None), "sp1"),
        ["tb1".to_owned(), tab.id.clone(), "tb2".to_owned()],
        "after the selected tab"
    );
    // The terminal labels itself; its tab leaves the bar, and the next press goes back.
    let labelled = core.label_elsewhere(&address, REVIEW_LABEL, "1");
    drive(&mut m, &mut core, labelled);
    let space = m.active_space().unwrap();
    let shown: Vec<&str> = m.shown_tabs(space).iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        shown,
        ["tb1"],
        "the review's tab is hidden, as the agent's is"
    );
    key(&mut m, &mut core, "cmd-r", true);
    assert_eq!(m.focused_terminal(), Some("t1"));
    key(&mut m, &mut core, "cmd-r", true);
    assert_eq!(m.focused_terminal(), Some(address.as_str()));
    assert_eq!(core.runs().len(), 1, "found, not started again");
}

#[test]
fn a_review_started_on_another_machines_space_runs_there() {
    let mut core = world();
    let mut m = connected_with_devbox(&mut core);
    drive(&mut m, &mut core, Action::FocusPane("devbox/p8".into()));
    key(&mut m, &mut core, "cmd-r", true);
    let run = core.runs().pop().unwrap();
    assert_eq!(run.on.as_deref(), Some("devbox"));
    assert_eq!(
        run.argv,
        model::shell::remote(REVIEW_ARGV.map(str::to_owned).to_vec()),
        "in the shell there"
    );
    assert_eq!(run.labels["herdr.space"], "sp8", "bare on its machine");
    assert_eq!(m.active_space().unwrap().id, "devbox/sp8");
    assert_eq!(m.shown_tab().unwrap().name, REVIEW_NAME);
}

// ---- Peek ------------------------------------------------------------------------------------

#[test]
fn a_peek_shows_a_sub_agent_over_the_window_and_leaves_focus_where_it_was() {
    let mut core = world();
    let mut m = connected_with_devbox(&mut core);
    let fx = drive(&mut m, &mut core, Action::Peek("s5".into()));
    assert_eq!(m.peeked(), Some("s5"));
    assert!(fx.contains(&Effect::Focus(Focus::Peek)));
    assert_eq!(m.focused_terminal(), Some("t1"), "the pane keeps focus");
    assert_eq!(
        m.keyboard_terminal(),
        Some("s5"),
        "the peek has the keyboard"
    );
    assert!(
        m.visible_terminals().contains(&"s5".to_owned()),
        "a peek is seen"
    );
    assert_eq!(
        m.peek_bar(),
        Some(PeekBar {
            address: "s5".into(),
            status: Some(Displayed::Working),
            from: Some("pi".into()),
            go: Some(Go::Keep("sp1".into())),
        })
    );
    assert_eq!(m.status()["overlays"], json!(["terminal-peek"]));
    // Its own sub-agent is from it, by name; an agent with a pane goes to it.
    drive(&mut m, &mut core, Action::Peek("s6".into()));
    let bar = m.peek_bar().unwrap();
    assert_eq!(
        (bar.status, bar.from.as_deref()),
        (Some(Displayed::Blocked), Some("explore"))
    );
    drive(&mut m, &mut core, Action::Peek("a2".into()));
    assert_eq!(m.peek_bar().unwrap().go, Some(Go::Pane("p2".into())));
    // A terminal that isn't running isn't peeked.
    drive(&mut m, &mut core, Action::ClosePeek);
    drive(&mut m, &mut core, Action::Peek("t99".into()));
    assert_eq!(m.peeked(), None);
}

#[test]
fn keys_go_to_the_peeked_terminal_and_closing_a_tab_closes_the_peek() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::Peek("s5".into()));
    // Esc is the terminal's: agents use it.
    let fx = m.step(
        m.now(),
        Event::key(model::Chord::parse("escape").unwrap(), true),
    );
    assert!(!fx.handled);
    assert_eq!(m.peeked(), Some("s5"));
    // Terminal commands go to it.
    let fx = key(&mut m, &mut core, "cmd-c", true);
    assert!(fx.contains(&Effect::Terminal {
        address: "s5".into(),
        command: TerminalCommand::Action(TerminalAction::Copy),
    }));
    // Closing a tab or pane by key closes the peek, not what is behind it.
    let tabs = tab_ids(&core.doc(None), "sp1");
    key(&mut m, &mut core, "cmd-w", true);
    assert_eq!(m.peeked(), None);
    let fx = drive(&mut m, &mut core, Action::Peek("s5".into()));
    assert!(fx.contains(&Effect::Focus(Focus::Peek)));
    key(&mut m, &mut core, "ctrl-g q", true);
    assert_eq!(m.peeked(), None);
    assert_eq!(tab_ids(&core.doc(None), "sp1"), tabs);
    assert!(core.kills().is_empty());
    // Focus goes back to the pane.
    assert_eq!(m.focused_terminal(), Some("t1"));
}

#[test]
fn a_popup_opens_over_the_peek_and_gives_the_keyboard_back() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::Peek("s5".into()));
    let fx = drive(&mut m, &mut core, Action::Open(Popup::Help));
    assert!(fx.contains(&Effect::Focus(Focus::Popup)));
    assert_eq!(m.status()["overlays"], json!(["terminal-peek", "help"]));
    let fx = key(&mut m, &mut core, "escape", true);
    assert!(fx.contains(&Effect::Focus(Focus::Peek)));
    assert_eq!(m.peeked(), Some("s5"));
    // A peek opened from a popup closes it.
    drive(&mut m, &mut core, Action::Open(Popup::Help));
    drive(&mut m, &mut core, Action::Peek("s6".into()));
    assert_eq!(*m.mode(), model::Mode::Normal);
    assert_eq!(m.peeked(), Some("s6"));
}

#[test]
fn the_peek_closes_when_its_terminal_ends() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::Peek("s5".into()));
    let gone = ended(&mut core, "s5");
    let fx = drive(&mut m, &mut core, gone);
    assert_eq!(m.peeked(), None);
    assert!(fx.contains(&Effect::Focus(Focus::Pane("p1".into()))));
}

#[test]
fn go_to_focuses_its_pane_and_closes_the_peek() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::Peek("a2".into()));
    drive(&mut m, &mut core, Action::KeepPeek);
    assert_eq!(m.peeked(), None);
    assert_eq!(m.focused_terminal(), Some("a2"));
    assert!(core.runs().is_empty(), "nothing started");
}

#[test]
fn keep_labels_the_terminal_keeps_it_and_gives_it_a_tab_in_the_shown_space() {
    let mut core = world();
    let mut m = connected(&mut core);
    let before = core.calls.len();
    drive(&mut m, &mut core, Action::Peek("s5".into()));
    drive(&mut m, &mut core, Action::KeepPeek);
    assert_eq!(m.peeked(), None);
    let calls = &core.calls[before..];
    // (Other label calls go by too: the seen turns the window shares.)
    let label = calls
        .iter()
        .find_map(|c| match c {
            model::Request::Label(l) if l.labels.contains_key("herdr.space") => Some(l.clone()),
            _ => None,
        })
        .expect("labelled");
    assert_eq!(label.terminal, "s5");
    assert_eq!(label.labels["herdr.space"], "sp1");
    let tab = label.labels["herdr.tab"].clone();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, model::Request::Keep(k) if k.terminal == "s5" && k.keep)),
        "kept"
    );
    // Its own tab, named after it, after the selected one, and focused; adopted once.
    let doc = core.doc(None);
    assert_eq!(
        tab_ids(&doc, "sp1"),
        ["tb1".to_owned(), tab.clone(), "tb2".to_owned()]
    );
    assert_eq!(doc.tab(&tab).unwrap().name, "explore");
    assert_eq!(m.focused_terminal(), Some("s5"));
    assert_eq!(m.shown_tab().unwrap().id, tab);
    let placed = doc
        .spaces
        .iter()
        .flat_map(|s| &s.tabs)
        .flat_map(|t| t.root.addresses())
        .filter(|a| a == "s5")
        .count();
    assert_eq!(placed, 1);
}

#[test]
fn keep_uses_the_terminals_machine_its_nearest_ancestors_space_there_else_the_first() {
    let mut core = world();
    let mut m = connected_with_devbox(&mut core);
    // The shown space is on this machine; s7 runs on devbox, where a9 (its parent) is in sp9.
    drive(&mut m, &mut core, Action::Peek("devbox/s7".into()));
    assert_eq!(
        m.peek_bar().unwrap().go,
        Some(Go::Keep("devbox/sp9".into()))
    );
    drive(&mut m, &mut core, Action::KeepPeek);
    assert_eq!(m.active_space().unwrap().id, "devbox/sp9", "shown first");
    assert_eq!(m.focused_terminal(), Some("devbox/s7"));
    let label = core
        .calls
        .iter()
        .find_map(|c| match c {
            model::Request::Label(l)
                if l.terminal == "devbox/s7" && l.labels.contains_key("herdr.space") =>
            {
                Some(l.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(label.labels["herdr.space"], "sp9", "bare on its machine");
    // With no ancestor in a pane there, the first space there.
    let mut lone = agent("devbox/s10", StatusState::Idle);
    lone.parent = Some("devbox/nobody".into());
    core.terminals.insert("devbox/s10".into(), lone.clone());
    drive(
        &mut m,
        &mut core,
        Event::Terminals(TerminalUpdate {
            terminals: vec![lone],
            ..Default::default()
        }),
    );
    drive(&mut m, &mut core, Action::FocusPane("p1".into()));
    drive(&mut m, &mut core, Action::Peek("devbox/s10".into()));
    assert_eq!(
        m.peek_bar().unwrap().go,
        Some(Go::Keep("devbox/sp8".into()))
    );
}

#[test]
fn keep_is_hidden_when_the_terminals_machine_has_no_space() {
    let mut core = world();
    core.docs.insert(Some("devbox".into()), (doc(vec![]), 1));
    let mut m = connected_with_devbox(&mut core);
    drive(&mut m, &mut core, Action::Peek("devbox/s7".into()));
    assert_eq!(m.peek_bar().unwrap().go, None);
    drive(&mut m, &mut core, Action::KeepPeek);
    assert_eq!(m.peeked(), Some("devbox/s7"), "nothing to do");
}

#[test]
fn a_keep_whose_space_closed_meanwhile_takes_its_labels_back() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::Peek("s5".into()));
    // Step the keep alone, then close the space elsewhere before its calls come back.
    let fx = m.step(m.now(), Action::KeepPeek).list;
    let closed = core.write_elsewhere(None, doc(vec![]));
    drive(&mut m, &mut core, closed);
    run_effects(&mut m, &mut core, fx);
    let s5 = &core.terminals["s5"];
    assert!(!s5.labels.contains_key("herdr.space"), "{:?}", s5.labels);
    assert!(!s5.keep);
    assert_eq!(m.world().pane_of("s5"), None);
}

#[test]
fn jumping_to_an_agent_without_a_pane_peeks_it() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::FocusAgent("s5".into()));
    assert_eq!(m.peeked(), Some("s5"));
    assert_eq!(m.focused_terminal(), Some("t1"));
}

#[test]
fn do_terminal_peek_and_agent_focus_peek_a_running_terminal() {
    let mut core = world();
    let mut m = connected(&mut core);
    let call = |m: &mut Model, core: &mut Core, name: &str, args| {
        let fx = drive(
            m,
            core,
            Call {
                id: CallId(3),
                name: name.into(),
                args,
            },
        );
        fx.into_iter()
            .find_map(|e| match e {
                Effect::Answer { reply, .. } => Some(reply),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(
        call(
            &mut m,
            &mut core,
            "terminal.peek",
            json!({"terminal": "s6"})
        ),
        json!({"ok": true})
    );
    assert_eq!(m.peeked(), Some("s6"));
    assert_eq!(
        call(&mut m, &mut core, "agent.focus", json!({"terminal": "s5"})),
        json!({"ok": true})
    );
    assert_eq!(m.peeked(), Some("s5"));
    assert_eq!(
        call(
            &mut m,
            &mut core,
            "terminal.peek",
            json!({"terminal": "t99"})
        ),
        json!({"error": "no running terminal \"t99\""})
    );
    assert_eq!(
        call(&mut m, &mut core, "terminal.peek", json!({})),
        json!({"error": "terminal.peek needs terminal"})
    );
}
