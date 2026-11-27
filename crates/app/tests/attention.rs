//! Attention on the agents core (`feature-inventory.md` §8), through Rune's real runtime with no
//! window: an agent that needs you or finishes while you look elsewhere shows an in-app toast,
//! a click goes to it and shares `herdr.seen`, and notes about agents on hidden machines close.

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::agents_core;
use herdr_frontend::link::Fake;
use herdr_frontend::{SIZE, State, app};
use herdr_proto::StatusState;
use model::Level;
use rune::{ElementInfo, Env, Runtime};
use serde_json::json;

fn runtime() -> (Runtime<State>, Fake) {
    let fake = agents_core();
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

fn element(rt: &Runtime<State>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

/// The core's agent at `address` reports `state`, its labels kept.
fn report(rt: &mut Runtime<State>, fake: &Fake, address: &str, state: StatusState, turn: u64) {
    let mut t = fake.core().terminals[address].clone();
    t.status.as_mut().expect("an agent").state = state;
    t.turn = turn;
    fake.update(t);
    rt.state.settle();
    frames(rt, 2);
}

fn shown(rt: &Runtime<State>) -> Vec<(String, String, Level)> {
    rt.state
        .toasts
        .shown
        .iter()
        .map(|t| (t.toast.title.clone(), t.toast.body.clone(), t.toast.level))
        .collect()
}

fn seen_label(fake: &Fake, address: &str) -> Option<String> {
    fake.core().terminals[address]
        .labels
        .get("herdr.seen")
        .cloned()
}

#[test]
fn an_agent_elsewhere_alerts_with_a_toast_that_goes_to_it() {
    let (mut rt, fake) = runtime();
    let looking_at = rt.state.model.focused_terminal().map(str::to_owned);
    assert_eq!(looking_at.as_deref(), Some("t1"));
    // The orchestrator needs you, then the sidebar's Claude finishes.
    report(&mut rt, &fake, "t12", StatusState::Blocked, 0);
    report(&mut rt, &fake, "t10", StatusState::Idle, 1);
    assert_eq!(
        shown(&rt),
        [
            (
                "Pi needs you".into(),
                "dotfiles / orchestrate".into(),
                Level::Error
            ),
            (
                "Claude Code finished".into(),
                "herdr-frontend / 2".into(),
                Level::Info
            ),
        ]
    );
    // Sub-agents never alert.
    report(&mut rt, &fake, "t20", StatusState::Blocked, 0);
    assert_eq!(shown(&rt).len(), 2);

    // Herdr app's stack: the oldest at the bottom, 16 pt in, newer ones above it.
    let [old, new] = [0, 1].map(|i| {
        let id = rt.state.toasts.shown[i].id;
        element(&rt, &format!("toast-{id}")).expect("a card")
    });
    assert_eq!(old.rect[1] + old.rect[3], SIZE.1 - 16.0);
    assert_eq!(new.rect[1] + new.rect[3], old.rect[1] - 8.0);

    // Clicking the older one goes to the orchestrator and dismisses it.
    let id = rt.state.toasts.shown[0].id;
    let (x, y) = rt.find(&format!("toast-{id}")).expect("on screen");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.state.settle();
    frames(&mut rt, 2);
    assert_eq!(rt.state.model.focused_terminal(), Some("t12"));
    assert_eq!(
        shown(&rt),
        [(
            "Claude Code finished".into(),
            "herdr-frontend / 2".into(),
            Level::Info
        )]
    );
    assert_eq!(
        seen_label(&fake, "t12").as_deref(),
        Some("0"),
        "looked at, for every client"
    );

    // Going to the finished one makes it idle everywhere.
    assert_eq!(
        rt.state.model.attention().displayed("t10"),
        Some(herdr_shared::Displayed::Done)
    );
    let id = rt.state.toasts.shown[0].id;
    let (x, y) = rt.find(&format!("toast-{id}")).unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.state.settle();
    frames(&mut rt, 2);
    assert!(rt.state.toasts.shown.is_empty());
    assert_eq!(seen_label(&fake, "t10").as_deref(), Some("1"));
    assert_eq!(
        rt.state.model.attention().displayed("t10"),
        Some(herdr_shared::Displayed::Idle)
    );
}

#[test]
fn an_agent_on_screen_alerts_without_a_toast() {
    let (mut rt, fake) = runtime();
    rt.state.step(0.0, model::Action::FocusAgent("t12".into()));
    rt.state.settle();
    report(&mut rt, &fake, "t12", StatusState::Blocked, 0);
    assert_eq!(shown(&rt), []);
}

#[test]
fn notes_about_agents_on_hidden_machines_close_at_once() {
    let (mut rt, fake) = runtime();
    rt.state
        .share(model::agents::HIDDEN_MACHINES_KEY, json!([""]));
    report(&mut rt, &fake, "t12", StatusState::Blocked, 0);
    assert_eq!(shown(&rt), [], "this computer is hidden");
}
