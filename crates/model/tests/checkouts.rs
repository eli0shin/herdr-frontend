//! Each terminal's git checkout through `Model::step` (`model::checkouts`): looked up on the
//! terminal's machine, again only when the answer could have changed, and used for space names
//! and the space picker's branch matching.

mod support;

use herdr_proto::types::{MachineState, MachineView};
use herdr_proto::{Foreground, Terminal};
use model::checkouts::FALLBACK;
use model::{Effect, Event, Front, Model, TerminalUpdate, popups};
use support::*;

const REPO: &str = "/home/me/code/repo";

/// sp1: tb1 (t1 in `repo/crates/app`), tb2 (t2 at the top of `repo`); sp2: tb3 (t3 at home);
/// sp3: tb4 (t4 in `/tmp/scratch`, no repo). All unnamed. `repo` is on `feature/x`.
fn world() -> Core {
    let mut core = Core::new(
        doc(vec![
            space(
                "sp1",
                vec![
                    tab("tb1", vec![pane("p1", "t1")]),
                    tab("tb2", vec![pane("p2", "t2")]),
                ],
            ),
            space("sp2", vec![tab("tb3", vec![pane("p3", "t3")])]),
            space("sp3", vec![tab("tb4", vec![pane("p4", "t4")])]),
        ]),
        vec![
            shell("t1", "/home/me/code/repo/crates/app"),
            shell("t2", REPO),
            shell("t3", HOME),
            shell("t4", "/tmp/scratch"),
        ],
    );
    core.repos
        .push((None, REPO.into(), Some("feature/x".into())));
    core
}

fn names(m: &Model) -> Vec<String> {
    m.world()
        .spaces()
        .map(|s| m.world().space_name(s, m.home()))
        .collect()
}

fn folders(core: &Core) -> Vec<&str> {
    core.git.iter().map(|(cwd, _)| cwd.as_str()).collect()
}

/// `address` now runs `program` in `cwd`; the event the core posts.
fn in_front(core: &mut Core, address: &str, program: &str, cwd: &str) -> Event {
    let t = core.terminals.get_mut(address).expect("a terminal");
    t.foreground = Some(Foreground {
        name: program.into(),
        argv: vec![program.into()],
        cwd: cwd.into(),
        ..Default::default()
    });
    Event::Terminals(TerminalUpdate {
        terminals: vec![t.clone()],
        ..Default::default()
    })
}

/// The shell in front is zsh; anything else is a program.
fn zsh_front() -> Front {
    Front::new(|t: &Terminal| t.foreground.as_ref().is_none_or(|f| f.name == "zsh"))
}

fn tick(m: &mut Model, core: &mut Core, at: f64) -> Vec<Effect> {
    core.now = at;
    drive(m, core, Event::Tick)
}

#[test]
fn unnamed_spaces_are_called_after_their_first_terminals_checkout() {
    let mut core = world();
    let m = connected(&mut core);
    assert_eq!(names(&m), ["repo", "~", "scratch"]);
    // Each folder on this machine once (in folder order), the home folder without asking.
    assert_eq!(
        folders(&core),
        [REPO, "/home/me/code/repo/crates/app", "/tmp/scratch"]
    );
    assert!(core.git.iter().all(|(_, on)| on.is_none()));
    assert!(core.calls.is_empty(), "no other calls");
    let checkout = m.world().checkout("t1").unwrap();
    assert_eq!(checkout.top, REPO);
    assert_eq!(checkout.branch.as_deref(), Some("feature/x"));
    assert_eq!(m.world().checkout("t4"), None, "not a repo");
}

#[test]
fn terminals_in_one_folder_share_a_lookup() {
    let mut core = world();
    core.terminals
        .insert("t9".into(), shell("t9", "/tmp/scratch"));
    let m = connected(&mut core);
    assert_eq!(
        folders(&core)
            .iter()
            .filter(|f| **f == "/tmp/scratch")
            .count(),
        1
    );
    assert_eq!(m.world().checkout("t9"), None);
}

#[test]
fn a_terminal_that_moves_is_looked_up_again_and_keeps_its_checkout_until_git_answers() {
    let mut core = world();
    let mut m = connected(&mut core);
    core.git.clear();
    core.git_held = Some(Vec::new());
    let moved = in_front(&mut core, "t1", "zsh", "/tmp/scratch");
    drive(&mut m, &mut core, moved);
    assert_eq!(folders(&core), ["/tmp/scratch"]);
    assert_eq!(
        names(&m)[0],
        "repo",
        "the checkout stands until git answers"
    );
    let back = in_front(&mut core, "t1", "zsh", "/home/me/code/repo/docs");
    drive(&mut m, &mut core, back);
    let held = core.git_held.take().unwrap();
    assert_eq!(held.len(), 2);
    // The answer for the folder it left changes nothing.
    let (token, cwd, on) = held[0].clone();
    let result = core.git_answer(&cwd, on.as_deref());
    drive(&mut m, &mut core, Event::Executed { token, result });
    assert_eq!(m.world().checkout("t1").map(|c| c.top.as_str()), Some(REPO));
    // Out of the repo, the space takes the folder's own name.
    let gone = in_front(&mut core, "t1", "zsh", "/tmp/scratch");
    drive(&mut m, &mut core, gone);
    assert_eq!(names(&m)[0], "scratch");
}

#[test]
fn a_program_ending_in_front_has_its_folder_looked_up_again() {
    let mut core = world();
    let mut m = Model::new(HOME);
    m.set_front(zsh_front());
    drive(&mut m, &mut core, Event::Connected);
    let snapshot = core.snapshot();
    drive(&mut m, &mut core, snapshot);
    core.git.clear();
    let git = in_front(&mut core, "t2", "git", REPO);
    drive(&mut m, &mut core, git);
    assert!(core.git.is_empty(), "nothing to ask while it runs");
    core.repos[0].2 = Some("main".into());
    let done = in_front(&mut core, "t2", "zsh", REPO);
    drive(&mut m, &mut core, done);
    assert_eq!(folders(&core), [REPO]);
    assert_eq!(
        m.world().checkout("t2").and_then(|c| c.branch.as_deref()),
        Some("main")
    );
    let again = in_front(&mut core, "t2", "zsh", REPO);
    drive(&mut m, &mut core, again);
    assert_eq!(core.git.len(), 1, "quiet stays quiet");
}

#[test]
fn a_repo_is_looked_up_again_after_a_minute_while_the_window_is_in_front() {
    let mut core = world();
    let mut m = Model::new(HOME);
    drive(&mut m, &mut core, Event::Connected);
    let snapshot = core.snapshot();
    let fx = drive(&mut m, &mut core, snapshot);
    assert!(fx.contains(&Effect::WakeAt(FALLBACK)), "{fx:?}");
    core.git.clear();
    // An earlier wake (a prefix's timeout) takes the app's one timer: the fallback asks again.
    let fx = tick(&mut m, &mut core, FALLBACK - 1.0);
    assert!(core.git.is_empty());
    assert!(fx.contains(&Effect::WakeAt(FALLBACK)), "{fx:?}");
    let fx = tick(&mut m, &mut core, FALLBACK);
    assert_eq!(
        folders(&core),
        [REPO, "/home/me/code/repo/crates/app"],
        "repos only"
    );
    assert!(fx.contains(&Effect::WakeAt(FALLBACK * 2.0)), "{fx:?}");
    core.git.clear();
    drive(&mut m, &mut core, Event::WindowActive(false));
    tick(&mut m, &mut core, FALLBACK * 3.0);
    assert!(core.git.is_empty(), "the window is behind");
    drive(&mut m, &mut core, Event::WindowActive(true));
    assert_eq!(core.git.len(), 2, "back in front, what is due is asked");
}

#[test]
fn another_machines_terminal_is_looked_up_there_once_it_is_reachable() {
    let mut core = world();
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![space("sp9", vec![tab("tb9", vec![pane("p9", "t9")])])]),
            1,
        ),
    );
    core.terminals
        .insert("devbox/t9".into(), shell("devbox/t9", "/srv/app/src"));
    core.repos.push((
        Some("devbox".into()),
        "/srv/app".into(),
        Some("main".into()),
    ));
    let mut m = connected(&mut core);
    let mut devbox = MachineView {
        state: MachineState::Unreachable,
        ..Default::default()
    };
    devbox.machine.name = "devbox".into();
    drive(&mut m, &mut core, Event::Machines(vec![devbox.clone()]));
    assert!(core.git.iter().all(|(_, on)| on.is_none()), "not reachable");
    devbox.state = MachineState::Connected;
    drive(&mut m, &mut core, Event::Machines(vec![devbox]));
    assert!(
        core.git
            .contains(&("/srv/app/src".into(), Some("devbox".into())))
    );
    assert_eq!(names(&m).last().map(String::as_str), Some("app"));
}

#[test]
fn connecting_again_looks_every_folder_up_again() {
    let mut core = world();
    let mut m = connected(&mut core);
    core.git.clear();
    drive(&mut m, &mut core, Event::Connected);
    let snapshot = core.snapshot();
    drive(&mut m, &mut core, snapshot);
    assert_eq!(core.git.len(), 3);
}

#[test]
fn the_space_picker_matches_the_branch_of_the_terminal_a_space_shows_first() {
    let mut core = world();
    let m = connected(&mut core);
    let sp1 = m.world().space("sp1").unwrap();
    assert_eq!(popups::space_branch(&m, sp1).as_deref(), Some("feature/x"));
    let ranked: Vec<String> = popups::space_rows(&m, "feat")
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(ranked, ["repo"]);
}
