//! Attention through `Model::step` (`feature-inventory.md` §8): the `herdr.seen` this screen
//! shares with every other Herdr client, and the in-app toasts alerts show.

mod support;

use herdr_proto::methods::TerminalLabel;
use herdr_proto::{ErrorKind, StatusState};
use herdr_shared::labels::SEEN;
use herdr_shared::{Alert, Displayed, Raise};
use model::{Action, Effect, Event, Level, Request};
use support::*;

/// sp1: tb1 (t1), tb2 (agent a2); sp2 "work": tb3 (agent a3), tb4 "review" (agent a4).
fn agents() -> Core {
    let mut work = space(
        "sp2",
        vec![tab("tb3", vec![pane("p3", "a3")]), {
            let mut t = tab("tb4", vec![pane("p4", "a4")]);
            t["name"] = "review".into();
            t
        }],
    );
    work["name"] = "work".into();
    Core::new(
        doc(vec![
            space(
                "sp1",
                vec![
                    tab("tb1", vec![pane("p1", "t1")]),
                    tab("tb2", vec![pane("p2", "a2")]),
                ],
            ),
            work,
        ]),
        vec![
            shell("t1", "/home/me/code/one"),
            agent("a2", StatusState::Working),
            agent("a3", StatusState::Working),
            agent("a4", StatusState::Working),
        ],
    )
}

/// The `herdr.seen` writes made so far, as (terminal, value).
fn seen_writes(core: &Core) -> Vec<(String, String)> {
    core.calls
        .iter()
        .filter_map(|c| match c {
            Request::Label(TerminalLabel {
                terminal, labels, ..
            }) => Some((terminal.clone(), labels.get(SEEN)?.clone())),
            _ => None,
        })
        .collect()
}

fn seen_label(core: &Core, address: &str) -> Option<String> {
    core.terminals[address].labels.get(SEEN).cloned()
}

fn alerts(fx: &[Effect]) -> Vec<Raise> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Alert(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn looking_at_an_agent_shares_herdr_seen_once_per_turn() {
    let mut core = agents();
    let mut m = connected(&mut core);
    // The shell on screen reports no status: nothing to share.
    assert_eq!(seen_writes(&core), []);
    drive(&mut m, &mut core, Action::FocusAgent("a2".into()));
    assert_eq!(seen_writes(&core), [("a2".into(), "0".into())]);
    assert_eq!(seen_label(&core, "a2").as_deref(), Some("0"));
    drive(&mut m, &mut core, Event::Tick);
    assert_eq!(seen_writes(&core).len(), 1, "once per turn");
    // It finishes on screen: seen at its new turn, no alert, idle everywhere.
    let finished = core.report("a2", StatusState::Idle, 1);
    let fx = drive(&mut m, &mut core, finished);
    assert_eq!(alerts(&fx), []);
    assert_eq!(seen_label(&core, "a2").as_deref(), Some("1"));
    assert_eq!(m.attention().displayed("a2"), Some(Displayed::Idle));
}

#[test]
fn a_mark_unread_elsewhere_holds_until_the_agent_is_looked_at_again() {
    let mut core = agents();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::FocusAgent("a2".into()));
    let finished = core.report("a2", StatusState::Idle, 1);
    drive(&mut m, &mut core, finished);
    assert_eq!(seen_label(&core, "a2").as_deref(), Some("1"));
    // A phone marks it unread while this screen still shows it: it stays unread.
    let unread = core.label_elsewhere("a2", SEEN, "0");
    drive(&mut m, &mut core, unread);
    assert_eq!(m.attention().displayed("a2"), Some(Displayed::Done));
    assert_eq!(seen_label(&core, "a2").as_deref(), Some("0"));
    // Looking away and back sees it again.
    drive(&mut m, &mut core, Action::ShowTab("tb1".into()));
    drive(&mut m, &mut core, Action::FocusAgent("a2".into()));
    assert_eq!(seen_label(&core, "a2").as_deref(), Some("1"));
    assert_eq!(m.attention().displayed("a2"), Some(Displayed::Idle));
}

#[test]
fn nothing_is_seen_while_the_window_is_in_the_background() {
    let mut core = agents();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Event::WindowActive(false));
    drive(&mut m, &mut core, Action::FocusAgent("a2".into()));
    assert_eq!(seen_writes(&core), []);
    let finished = core.report("a2", StatusState::Idle, 1);
    let fx = drive(&mut m, &mut core, finished);
    assert_eq!(m.attention().displayed("a2"), Some(Displayed::Done));
    assert!(
        alerts(&fx)
            .iter()
            .any(|r| r.terminal == "a2" && r.alert == Alert::Done && r.toast)
    );
    drive(&mut m, &mut core, Event::WindowActive(true));
    assert_eq!(seen_label(&core, "a2").as_deref(), Some("1"));
    assert_eq!(m.attention().displayed("a2"), Some(Displayed::Idle));
}

#[test]
fn done_turns_idle_when_another_client_looks() {
    let mut core = agents();
    let mut m = connected(&mut core);
    let finished = core.report("a3", StatusState::Idle, 1);
    drive(&mut m, &mut core, finished);
    assert_eq!(m.attention().displayed("a3"), Some(Displayed::Done));
    let seen = core.label_elsewhere("a3", SEEN, "1");
    drive(&mut m, &mut core, seen);
    assert_eq!(m.attention().displayed("a3"), Some(Displayed::Idle));
    assert!(
        !seen_writes(&core).iter().any(|(t, _)| t == "a3"),
        "a3 isn't on this screen"
    );
}

#[test]
fn a_failed_seen_write_is_tried_again_after_the_core_s_next_news() {
    let mut core = agents();
    let mut m = connected(&mut core);
    let label_call = |fx: &model::Effects| {
        fx.iter().find_map(|e| match e {
            Effect::Call {
                token,
                request: Request::Label(l),
            } if l.terminal == "a2" => Some(*token),
            _ => None,
        })
    };
    let fx = m.step(core.now, Action::FocusAgent("a2".into()));
    let token = label_call(&fx).expect("a seen write");
    let failed = || Err(error(ErrorKind::Unavailable, "machine down"));
    let fx = m.step(
        core.now,
        Event::Reply {
            token,
            result: failed(),
        },
    );
    assert_eq!(label_call(&fx), None, "not in the failure's own step");
    assert_eq!(label_call(&m.step(core.now, Event::Tick)), None);
    // The core's next news: written again, and failing again waits again.
    let news = core.label_elsewhere("t1", "other", "x");
    let token = label_call(&m.step(core.now, news)).expect("tried again");
    let fx = m.step(
        core.now,
        Event::Reply {
            token,
            result: failed(),
        },
    );
    assert_eq!(label_call(&fx), None);
}

#[test]
fn an_alert_s_toast_names_the_agent_and_where_it_is_and_goes_there() {
    let mut core = agents();
    core.terminals
        .get_mut("a3")
        .unwrap()
        .status
        .as_mut()
        .unwrap()
        .app = "claude-code".into();
    core.terminals.get_mut("a4").unwrap().name = Some("fixer".into());
    let mut m = connected(&mut core);
    let name = |app: &str| format!("<{app}>");

    let finished = core.report("a3", StatusState::Idle, 1);
    let fx = drive(&mut m, &mut core, finished);
    let raise = alerts(&fx).pop().expect("a3 finished unseen");
    let toast = m.alert_toast(&raise, name).expect("not looking");
    assert_eq!(toast.title, "<claude-code> finished");
    assert_eq!(toast.level, Level::Info);
    assert_eq!(toast.body, "work / 1");
    assert_eq!(toast.action, Some(Action::FocusAgent("a3".into())));
    assert_eq!(toast.terminal(), Some("a3"));

    let blocked = core.report("a4", StatusState::Blocked, 0);
    let fx = drive(&mut m, &mut core, blocked);
    let raise = alerts(&fx).pop().expect("a4 needs you");
    let toast = m.alert_toast(&raise, name).unwrap();
    assert_eq!(toast.title, "fixer needs you");
    assert_eq!(toast.level, Level::Error);
    assert_eq!(toast.body, "work / review");

    // No name and no app.
    let blocked = core.report("a2", StatusState::Blocked, 0);
    let fx = drive(&mut m, &mut core, blocked);
    let raise = alerts(&fx).pop().unwrap();
    assert_eq!(
        m.alert_toast(&raise, name).unwrap().title,
        "An agent needs you"
    );

    // On screen: an alert, but no toast.
    drive(&mut m, &mut core, Action::FocusAgent("a3".into()));
    let working = core.report("a3", StatusState::Working, 1);
    drive(&mut m, &mut core, working);
    let blocked = core.report("a3", StatusState::Blocked, 1);
    let fx = drive(&mut m, &mut core, blocked);
    let raise = alerts(&fx).pop().expect("blocked always alerts");
    assert!(!raise.toast);
    assert_eq!(m.alert_toast(&raise, name), None);
}
