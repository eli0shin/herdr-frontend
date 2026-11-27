//! The dead-space sweep through `Model::step` (`feature-inventory.md` §3): Eli's
//! `cfg/mods/navigator-status/dead-spaces.js`, case by case as its regression check
//! `tests/herdr-app-dead-spaces.mjs` runs it.

mod support;

use std::collections::BTreeSet;

use herdr_proto::types::{MachineState, MachineView};
use herdr_proto::{Foreground, Status, StatusState, Terminal, TerminalState};
use model::dead_spaces::SWEEP_EVERY;
use model::{Effect, Event, Level, Model, Request, TerminalUpdate, Toast};
use serde_json::{Value, json};
use support::*;

/// A space with one tab holding a pane for each terminal.
fn space(id: &str, folder: &str, terminals: &[&str]) -> Value {
    let panes = terminals
        .iter()
        .map(|t| pane(&format!("p-{t}"), t))
        .collect();
    json!({"id": id, "name": id, "folder": folder, "tabs": [tab(&format!("tb-{id}"), panes)]})
}

/// An idle shell labeled for `space`, its live folder `cwd`.
fn term(address: &str, space: &str, cwd: &str) -> Terminal {
    Terminal {
        labels: [("herdr.space".to_owned(), space.to_owned())].into(),
        foreground: Some(Foreground {
            name: "fish".into(),
            cwd: cwd.into(),
            ..Default::default()
        }),
        ..shell(address, cwd)
    }
}

fn with_status(mut t: Terminal, state: StatusState) -> Terminal {
    t.status = Some(Status {
        state,
        app: "pi".into(),
        ..Default::default()
    });
    t
}

/// The shown space, which the sweep never closes: first, so it shows.
const SHOWN: &str = "shown";

/// A model on a core with the shown space and `spaces`, `terminals` in them, and the folders
/// that still exist.
struct Rig {
    m: Model,
    core: Core,
    present: BTreeSet<String>,
    /// The spaces on this computer at the start.
    ids: Vec<String>,
    execs: usize,
    toasts: Vec<Toast>,
}

impl Rig {
    fn new(spaces: Vec<Value>, terminals: Vec<Terminal>, present: &[&str]) -> Rig {
        let mut all = vec![space(SHOWN, "/w/shown", &["t0"])];
        all.extend(spaces);
        let mut terms = vec![term("t0", SHOWN, "/w/shown")];
        terms.extend(terminals);
        let mut core = Core::new(doc(all), terms);
        let ids = core.doc(None).spaces.iter().map(|s| s.id.clone()).collect();
        let m = connected(&mut core);
        assert_eq!(m.active_space().map(|s| s.id.as_str()), Some(SHOWN));
        Rig {
            m,
            core,
            present: present.iter().map(|p| (*p).to_owned()).collect(),
            ids,
            execs: 0,
            toasts: Vec::new(),
        }
    }

    /// The next sweep, its folder check answered from `present`.
    fn sweep(&mut self) {
        self.sweep_with(|_| {});
    }

    /// The next sweep, with `during` run while its folder check is under way.
    fn sweep_with(&mut self, during: impl FnOnce(&mut Rig)) {
        self.core.now += SWEEP_EVERY;
        let fx = self.m.step(self.core.now, Event::Tick).list;
        let mut check = None;
        let mut rest = Vec::new();
        for effect in fx {
            match effect {
                Effect::Call {
                    token,
                    request: Request::Exec(exec),
                } => {
                    assert!(check.is_none(), "one check per sweep");
                    check = Some((token, exec));
                }
                other => rest.push(other),
            }
        }
        let out = run_effects(&mut self.m, &mut self.core, rest);
        self.keep_toasts(out);
        let Some((token, exec)) = check else {
            return;
        };
        self.execs += 1;
        assert_eq!(exec.argv[..2], ["sh", "-c"]);
        assert_eq!(exec.argv[3], "sh");
        assert_eq!(exec.on, None, "on this computer");
        during(self);
        let stdout: String = exec.argv[4..]
            .iter()
            .map(|p| {
                if self.present.contains(p) {
                    "1\n"
                } else {
                    "0\n"
                }
            })
            .collect();
        let fx = self
            .m
            .step(
                self.core.now,
                Event::Executed {
                    token,
                    result: printed(&stdout),
                },
            )
            .list;
        let out = run_effects(&mut self.m, &mut self.core, fx);
        self.keep_toasts(out);
    }

    fn sweep_twice(&mut self) {
        self.sweep();
        self.sweep();
    }

    fn keep_toasts(&mut self, effects: Vec<Effect>) {
        self.toasts
            .extend(effects.into_iter().filter_map(|e| match e {
                Effect::Toast(t) => Some(t),
                _ => None,
            }));
    }

    /// The spaces the sweep closed, from the core's document.
    fn closed(&self) -> Vec<String> {
        let doc = self.core.doc(None);
        self.ids
            .iter()
            .filter(|id| doc.space(id).is_none())
            .cloned()
            .collect()
    }

    /// Another client changes a terminal; the event the core posts.
    fn change(&mut self, address: &str, f: impl FnOnce(&mut Terminal)) {
        let t = self.core.terminals.get_mut(address).expect("a terminal");
        f(t);
        let event = Event::Terminals(TerminalUpdate {
            terminals: vec![t.clone()],
            ..Default::default()
        });
        drive(&mut self.m, &mut self.core, event);
    }
}

#[test]
fn a_space_whose_folder_was_deleted_closes_on_the_second_sweep() {
    let mut r = Rig::new(
        vec![space("gone", "/w/gone", &["t1"])],
        vec![term("t1", "gone", "/w/gone")],
        &["/w/shown"],
    );
    r.sweep();
    assert!(r.closed().is_empty(), "not on the first sweep");
    r.sweep();
    assert_eq!(r.closed(), ["gone"]);
    assert_eq!(r.core.kills(), ["t1"], "its terminals end");
    assert_eq!(
        r.toasts,
        [Toast::new(
            Level::Info,
            "Closed space gone",
            "Its folder no longer exists: /w/gone"
        )]
    );
}

#[test]
fn the_first_sweep_is_one_interval_after_the_listing_and_they_keep_coming() {
    let mut r = Rig::new(
        vec![space("gone", "/w/gone", &["t1"])],
        vec![term("t1", "gone", "/w/gone")],
        &[],
    );
    // Just before it's due: nothing.
    r.core.now = SWEEP_EVERY - 1.0;
    let fx = r.m.step(r.core.now, Event::Tick).list;
    assert!(!fx.iter().any(|e| matches!(e, Effect::Call { .. })));
    // The tick that came for something else asks for the sweep's wake-up again.
    assert!(fx.contains(&Effect::WakeAt(SWEEP_EVERY)), "{fx:?}");
    r.core.now = 0.0;
    r.sweep();
    assert_eq!(r.execs, 1);
    r.sweep();
    assert_eq!(r.execs, 2);
}

#[test]
fn a_folder_that_comes_back_between_sweeps_keeps_its_space() {
    let mut r = Rig::new(
        vec![space("blip", "/w/blip", &["t1"])],
        vec![term("t1", "blip", "/w/blip")],
        &[],
    );
    r.sweep();
    r.present.insert("/w/blip".into());
    r.sweep();
    r.present.remove("/w/blip");
    r.sweep();
    assert!(r.closed().is_empty(), "the confirmation starts over");
    r.sweep();
    assert_eq!(r.closed(), ["blip"]);
}

#[test]
fn a_folder_that_exists_keeps_its_space() {
    let mut r = Rig::new(
        vec![space("here", "/w/here", &["t1"])],
        vec![term("t1", "here", "/w/here")],
        &["/w/here"],
    );
    r.sweep_twice();
    assert!(r.closed().is_empty());
}

#[test]
fn a_shell_that_moved_somewhere_that_exists_keeps_its_space() {
    let mut r = Rig::new(
        vec![space("moved", "/w/moved", &["t1"])],
        vec![term("t1", "moved", "/w/other")],
        &["/w/other"],
    );
    r.sweep_twice();
    assert!(r.closed().is_empty());
}

#[test]
fn any_terminal_in_a_living_folder_keeps_its_space() {
    let mut r = Rig::new(
        vec![space("split", "/w/split", &["t1", "t2"])],
        vec![
            term("t1", "split", "/w/split"),
            term("t2", "split", "/w/alive"),
        ],
        &["/w/alive"],
    );
    r.sweep_twice();
    assert!(r.closed().is_empty());
}

#[test]
fn only_a_working_agent_keeps_a_dead_space() {
    let mut nvim = term("t1", "dead", "/w/dead");
    nvim.foreground.as_mut().unwrap().name = "nvim".into();
    let cases = [
        with_status(term("t1", "dead", "/w/dead"), StatusState::Done),
        with_status(term("t1", "dead", "/w/dead"), StatusState::Idle),
        with_status(term("t1", "dead", "/w/dead"), StatusState::Blocked),
        with_status(term("t1", "dead", "/w/dead"), StatusState::Error),
        nvim,
    ];
    for t in cases {
        let what = format!("{:?} {:?}", t.status, t.foreground);
        let mut r = Rig::new(vec![space("dead", "/w/dead", &["t1"])], vec![t], &[]);
        r.sweep();
        assert!(r.closed().is_empty(), "not on the first sweep: {what}");
        r.sweep();
        assert_eq!(r.closed(), ["dead"], "closed with {what}");
    }
}

#[test]
fn a_working_agent_keeps_its_space_without_a_check() {
    let mut r = Rig::new(
        vec![space("busy", "/w/busy", &["t1"])],
        vec![with_status(
            term("t1", "busy", "/w/busy"),
            StatusState::Working,
        )],
        &[],
    );
    r.sweep_twice();
    assert!(r.closed().is_empty());
    assert_eq!(r.execs, 0);
}

#[test]
fn one_working_agent_keeps_the_whole_space_next_to_an_idle_shell() {
    let mut r = Rig::new(
        vec![space("mixed", "/w/mixed", &["t1", "t2"])],
        vec![
            term("t1", "mixed", "/w/mixed"),
            with_status(term("t2", "mixed", "/w/mixed"), StatusState::Working),
        ],
        &[],
    );
    r.sweep_twice();
    assert!(r.closed().is_empty());
}

#[test]
fn an_idle_agent_in_a_folder_that_exists_keeps_its_space() {
    let mut r = Rig::new(
        vec![space("mixed", "/w/mixed", &["t1", "t2"])],
        vec![
            term("t1", "mixed", "/w/mixed"),
            with_status(term("t2", "mixed", "/w/other"), StatusState::Idle),
        ],
        &["/w/other"],
    );
    r.sweep_twice();
    assert!(r.closed().is_empty());
    assert_eq!(r.execs, 2);
}

#[test]
fn the_shown_space_another_machines_space_and_a_space_without_live_terminals_stay() {
    let mut finished = term("t3", "empty", "/w/empty");
    finished.state = TerminalState::Finished;
    let mut remote = term("vps/t2", "sp", "/w/remote");
    remote.labels.insert("herdr.space".into(), "sp".into());
    let mut r = Rig::new(
        vec![space("empty", "/w/empty", &["t3"])],
        vec![finished, remote],
        &[],
    );
    // The shown space's folder is gone too.
    r.present.clear();
    r.core.docs.insert(
        Some("vps".into()),
        (doc(vec![space("sp", "/w/remote", &["t2"])]), 1),
    );
    let mut vps = MachineView {
        state: MachineState::Connected,
        ..Default::default()
    };
    vps.machine.name = "vps".into();
    drive(&mut r.m, &mut r.core, Event::Machines(vec![vps]));
    assert!(r.m.world().space("vps/sp").is_some());
    r.sweep_twice();
    assert!(r.m.world().space("vps/sp").is_some());
    assert!(r.m.world().space("empty").is_some());
    assert!(r.m.world().space(SHOWN).is_some());
    assert_eq!(r.execs, 0);
}

#[test]
fn a_terminal_whose_folder_is_unknown_keeps_its_space() {
    let mut t = term("t1", "unknown", "/w/unknown");
    t.foreground.as_mut().unwrap().cwd = String::new();
    let mut r = Rig::new(vec![space("unknown", "/w/unknown", &["t1"])], vec![t], &[]);
    r.sweep_twice();
    assert!(r.closed().is_empty());
    assert_eq!(r.execs, 0);
}

#[test]
fn without_a_foreground_the_launch_folder_counts() {
    let mut t = term("t1", "launched", "/w/launched");
    t.foreground = None;
    let mut r = Rig::new(
        vec![space("launched", "/w/launched", &["t1"])],
        vec![t],
        &[],
    );
    r.sweep_twice();
    assert_eq!(r.closed(), ["launched"]);
}

#[test]
fn a_space_without_a_folder_of_its_own_checks_home() {
    let mut r = Rig::new(
        vec![space("home", "", &["t1"])],
        vec![term("t1", "home", "/w/home-gone")],
        &[HOME],
    );
    r.sweep_twice();
    assert!(r.closed().is_empty());
    r.present.clear();
    r.sweep_twice();
    assert_eq!(r.closed(), ["home"]);
    assert_eq!(
        r.toasts.last().map(|t| t.body.as_str()),
        Some("Its folder no longer exists: /home/me")
    );
}

#[test]
fn a_failed_folder_check_closes_nothing() {
    let mut r = Rig::new(
        vec![space("gone", "/w/gone", &["t1"])],
        vec![term("t1", "gone", "/w/gone")],
        &[],
    );
    for _ in 0..3 {
        r.core.now += SWEEP_EVERY;
        let fx = r.m.step(r.core.now, Event::Tick).list;
        let token = fx
            .iter()
            .find_map(|e| match e {
                Effect::Call {
                    token,
                    request: Request::Exec(_),
                } => Some(*token),
                _ => None,
            })
            .expect("a check");
        r.m.step(
            r.core.now,
            Event::Executed {
                token,
                result: exited(1),
            },
        );
    }
    assert!(r.closed().is_empty());
}

#[test]
fn a_check_still_under_way_holds_the_next_sweep() {
    let mut r = Rig::new(
        vec![space("gone", "/w/gone", &["t1"])],
        vec![term("t1", "gone", "/w/gone")],
        &[],
    );
    r.core.exec_held = true;
    r.core.now += SWEEP_EVERY;
    drive(&mut r.m, &mut r.core, Event::Tick);
    r.core.now += SWEEP_EVERY;
    drive(&mut r.m, &mut r.core, Event::Tick);
    assert_eq!(r.core.execs().len(), 1);
}

#[test]
fn moving_to_another_missing_folder_starts_the_confirmation_over() {
    let mut r = Rig::new(
        vec![space("late", "/w/late", &["t1"])],
        vec![term("t1", "late", "/w/late")],
        &[],
    );
    r.sweep();
    r.change("t1", |t| {
        t.foreground.as_mut().unwrap().cwd = "/w/other".into()
    });
    r.sweep();
    assert!(r.closed().is_empty());
    r.sweep();
    assert_eq!(r.closed(), ["late"]);
}

#[test]
fn an_agent_that_starts_working_during_the_check_keeps_its_space() {
    let mut r = Rig::new(
        vec![space("woken", "/w/woken", &["t1"])],
        vec![with_status(
            term("t1", "woken", "/w/woken"),
            StatusState::Idle,
        )],
        &[],
    );
    r.sweep();
    r.sweep_with(|r| {
        let event = r.core.report("t1", StatusState::Working, 1);
        drive(&mut r.m, &mut r.core, event);
    });
    assert!(r.closed().is_empty());
    let event = r.core.report("t1", StatusState::Done, 2);
    drive(&mut r.m, &mut r.core, event);
    r.sweep();
    assert!(
        r.closed().is_empty(),
        "two new sweeps after the agent settles"
    );
    r.sweep();
    assert_eq!(r.closed(), ["woken"]);
}

#[test]
fn a_terminal_moving_or_the_space_shown_during_the_check_keeps_it() {
    let moved =
        |r: &mut Rig| r.change("t1", |t| t.foreground.as_mut().unwrap().cwd = "/w/x".into());
    let shown = |r: &mut Rig| {
        drive(
            &mut r.m,
            &mut r.core,
            model::Action::ShowSpace("race".into()),
        );
        assert_eq!(r.m.active_space().map(|s| s.id.as_str()), Some("race"));
    };
    for during in [moved as fn(&mut Rig), shown] {
        let mut r = Rig::new(
            vec![space("race", "/w/race", &["t1"])],
            vec![term("t1", "race", "/w/race")],
            &[],
        );
        r.sweep();
        r.sweep_with(during);
        assert!(r.closed().is_empty());
    }
}

#[test]
fn only_the_dead_space_among_several_closes() {
    let mut r = Rig::new(
        vec![space("a", "/w/a", &["t1"]), space("b", "/w/b", &["t2"])],
        vec![term("t1", "a", "/w/a"), term("t2", "b", "/w/b")],
        &["/w/b"],
    );
    r.sweep_twice();
    assert_eq!(r.closed(), ["a"]);
    assert_eq!(r.toasts.len(), 1);
}

#[test]
fn a_reconnect_forgets_the_check_under_way() {
    let mut r = Rig::new(
        vec![space("gone", "/w/gone", &["t1"])],
        vec![term("t1", "gone", "/w/gone")],
        &[],
    );
    r.core.exec_held = true;
    r.core.now += SWEEP_EVERY;
    drive(&mut r.m, &mut r.core, Event::Tick);
    drive(&mut r.m, &mut r.core, Event::Connected);
    let snapshot = r.core.snapshot();
    drive(&mut r.m, &mut r.core, snapshot);
    r.core.exec_held = false;
    r.sweep_twice();
    assert_eq!(r.closed(), ["gone"], "sweeps go on after the lost check");
}

#[test]
fn no_sweep_runs_while_the_layout_cant_take_the_close() {
    let mut r = Rig::new(
        vec![space("gone", "/w/gone", &["t1"])],
        vec![term("t1", "gone", "/w/gone")],
        &[],
    );
    r.core.down.push(None);
    let event = r.core.write_elsewhere(None, r.core.docs[&None].0.clone());
    drive(&mut r.m, &mut r.core, event);
    r.sweep_twice();
    assert_eq!(r.execs, 0);
    assert!(
        r.toasts.iter().all(|t| t.title != "Layout unavailable"),
        "{:?}",
        r.toasts
    );
    // The layout is read again on the next wake-up, after that sweep was skipped.
    r.core.down.clear();
    r.sweep();
    assert_eq!(r.execs, 0);
    assert!(r.m.edit_refused("gone").is_none());
    r.sweep_twice();
    assert_eq!(r.closed(), ["gone"]);
}

#[test]
fn a_tick_asks_again_for_the_next_deadline_whichever_it_is() {
    let mut r = Rig::new(
        vec![space("gone", "/w/gone", &["t1"])],
        vec![term("t1", "gone", "/w/gone")],
        &["/w/gone"],
    );
    // A layout read fails just before the sweep is due: its retry comes after the sweep.
    r.core.now = SWEEP_EVERY - 1.0;
    r.core.down.push(None);
    let event = r.core.write_elsewhere(None, r.core.docs[&None].0.clone());
    drive(&mut r.m, &mut r.core, event);
    let retry = r.core.now + model::RETRY;
    // The sweep's wake-up fires first and the app forgets the retry's: the tick asks again.
    r.core.now = SWEEP_EVERY;
    let fx = r.m.step(r.core.now, Event::Tick).list;
    assert!(fx.contains(&Effect::WakeAt(retry)), "{fx:?}");
}
