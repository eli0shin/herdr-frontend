//! The tab bar (`feature-inventory.md` §4, `cfg/mods/eli-tabs`): what its pills read and show,
//! the mouse's actions on them, and the tab menu, through `Model::step` against a fake core.

mod support;

use herdr_proto::{StatusState, Terminal};
use herdr_shared::Displayed;
use model::mode::{MenuTarget, PopupState, Rename};
use model::{Action, Effect, Mode, PopupAction, TabPill};
use serde_json::json;
use support::*;

fn is_pi(t: &Terminal) -> bool {
    t.argv.first().is_some_and(|a| a == "pi")
}

fn pi(address: &str, title: &str) -> Terminal {
    Terminal {
        argv: vec!["pi".into()],
        title: title.into(),
        ..shell(address, "/home/me/code/herdr")
    }
}

fn named(id: &str, name: &str, children: Vec<serde_json::Value>) -> serde_json::Value {
    json!({"id": id, "name": name, "root": {"id": id, "type": "row", "children": children}})
}

fn pills(m: &model::Model) -> Vec<TabPill> {
    m.tab_pills(m.active_space().unwrap(), is_pi)
}

fn labels(m: &model::Model) -> Vec<String> {
    pills(m).into_iter().map(|p| p.label).collect()
}

/// tb1 shell, tb2 an agent (hidden), tb3 shell, tb4 named "logs", tb5 and tb6 Pi sessions named
/// alike, tb7 a Pi split whose second pane is the Pi one.
fn strip() -> Core {
    Core::new(
        doc(vec![space(
            "sp1",
            vec![
                tab("tb1", vec![pane("p1", "t1")]),
                tab("tb2", vec![pane("p2", "a2")]),
                tab("tb3", vec![pane("p3", "t3")]),
                named("tb4", "logs", vec![pane("p4", "t4")]),
                tab("tb5", vec![pane("p5", "pi5")]),
                tab("tb6", vec![pane("p6", "pi6")]),
                tab("tb7", vec![pane("p7", "t7"), pane("p8", "pi8")]),
            ],
        )]),
        vec![
            shell("t1", "/"),
            agent("a2", StatusState::Working),
            shell("t3", "/"),
            shell("t4", "/"),
            pi("pi5", "π - fix the bar - herdr"),
            pi("pi6", "π - fix the bar - herdr"),
            shell("t7", "/"),
            pi("pi8", "π - review - herdr"),
        ],
    )
}

#[test]
fn unnamed_tabs_are_numbered_among_the_shown_tabs_and_pi_tabs_take_their_session() {
    let mut core = strip();
    let m = connected(&mut core);
    assert_eq!(
        labels(&m),
        [
            "1",
            // The agent's tb2 is hidden, so tb3 is the second shown tab, as Cmd+2 counts.
            "2",
            "logs",
            // Same Pi session twice: each gains its place among all the space's tabs.
            "fix the bar \u{b7} 5",
            "fix the bar \u{b7} 6",
            // A split's first Pi pane names it, not its first pane.
            "review",
        ]
    );
    let first = &pills(&m)[0];
    assert!(first.active && !first.zoomed && first.status.is_none());
    assert!(pills(&m)[1..].iter().all(|p| !p.active));
}

#[test]
fn a_suffix_someone_already_used_gets_a_further_one() {
    let mut core = Core::new(
        doc(vec![space(
            "sp1",
            vec![
                named("tb1", "x \u{b7} 2", vec![pane("p1", "t1")]),
                tab("tb2", vec![pane("p2", "pi2")]),
                tab("tb3", vec![pane("p3", "pi3")]),
            ],
        )]),
        vec![
            shell("t1", "/"),
            pi("pi2", "π - x - herdr"),
            pi("pi3", "π - x - herdr"),
        ],
    );
    let m = connected(&mut core);
    assert_eq!(labels(&m), ["x \u{b7} 2", "x \u{b7} 2.2", "x \u{b7} 3"]);
}

#[test]
fn pills_show_the_selected_tab_its_zoom_and_its_terminals_status() {
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
    drive(&mut m, &mut core, Action::PaneZoom);
    let p = pills(&m);
    assert!(p[0].active && p[0].zoomed);
    assert!(!p[1].active && !p[1].zoomed);
    drive(&mut m, &mut core, Action::ShowTab("tb2".into()));
    let p = pills(&m);
    assert!(!p[0].active && p[0].zoomed, "the zoom is the tab's own");
    assert!(p[1].active);
    // Shells report no status. (A terminal that does makes its tab an agent's, which the bar
    // leaves out, so in Eli's layout a pill's status mark rarely shows.)
    assert_eq!(p[1].status, None::<Displayed>);
}

#[test]
fn clicking_moving_and_closing_tabs_from_their_pills() {
    let mut core = strip();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::ShowTab("tb3".into()));
    assert_eq!(m.shown_tab().unwrap().id, "tb3");
    // Press on tb1, release on tb4: tb1 goes where tb4 is among all the space's tabs.
    drive(
        &mut m,
        &mut core,
        Action::MoveTab {
            tab: "tb1".into(),
            to: 3,
        },
    );
    assert_eq!(
        tab_ids(&core.doc(None), "sp1"),
        ["tb2", "tb3", "tb4", "tb1", "tb5", "tb6", "tb7"]
    );
    assert_eq!(m.shown_tab().unwrap().id, "tb3", "moving selects nothing");
    // The pill's ×: Eli's rule shows the nearest shown tab to the left.
    drive(&mut m, &mut core, Action::CloseTabOf("tb3".into()));
    assert!(core.doc(None).tab("tb3").is_none());
    assert_eq!(core.kills(), ["t3"]);
    assert_eq!(
        m.shown_tab().unwrap().id,
        "tb4",
        "tb2 to its left is an agent's"
    );
    // Double-click renames.
    drive(&mut m, &mut core, Action::RenameTabOf("tb4".into()));
    assert!(matches!(
        m.mode(),
        Mode::Popup(PopupState::Prompt(p)) if p.rename == Rename::Tab("tb4".into()) && p.value == "logs"
    ));
}

#[test]
fn the_wheel_steps_shown_tabs_as_tab_next_and_prev_do() {
    let mut core = strip();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::TabNext);
    assert_eq!(m.shown_tab().unwrap().id, "tb3", "over the agent's tab");
    drive(&mut m, &mut core, Action::TabPrev);
    drive(&mut m, &mut core, Action::TabPrev);
    assert_eq!(m.shown_tab().unwrap().id, "tb7", "wrapping");
}

fn menu(m: &model::Model) -> &model::Menu {
    match m.mode() {
        Mode::Popup(PopupState::Menu(menu)) => menu,
        other => panic!("no menu: {other:?}"),
    }
}

#[test]
fn right_click_opens_the_tab_menu_with_its_items_and_key_hints() {
    let mut core = strip();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::TabMenu("tb3".into()));
    let open = menu(&m);
    assert_eq!(open.target, MenuTarget::Tab("tb3".into()));
    let rows: Vec<(String, Option<String>)> = open
        .items
        .iter()
        .map(|i| {
            let hint = i
                .hint
                .as_ref()
                .and_then(|h| model::help::hint(m.keymap(), h));
            (i.label.clone(), hint)
        })
        .collect();
    assert_eq!(
        rows,
        [
            // Herdr app's hint: the first `global` key, in sorted order.
            ("Rename".into(), Some("ctrl-g r".into())),
            ("New tab".into(), Some("cmd-t".into())),
            ("Close".into(), Some("cmd-w".into())),
        ]
    );
    assert!(m.mode().holds_keyboard());
    // It holds every key: none reaches the terminal.
    let fx = m.step(m.now(), model::Event::key(model::Chord::char('x'), true));
    assert!(fx.handled);
    assert!(matches!(m.mode(), Mode::Popup(PopupState::Menu(_))));
    // Esc closes it.
    key(&mut m, &mut core, "escape", false);
    assert_eq!(*m.mode(), Mode::Normal);
    // A tab that isn't there opens nothing.
    drive(&mut m, &mut core, Action::TabMenu("nope".into()));
    assert_eq!(*m.mode(), Mode::Normal);
}

#[test]
fn the_tab_menu_runs_rename_new_tab_and_close_on_its_tab() {
    let mut core = strip();
    let mut m = connected(&mut core);
    // Rename: the first item, the prompt for that tab (not the shown one).
    drive(&mut m, &mut core, Action::TabMenu("tb4".into()));
    key(&mut m, &mut core, "enter", false);
    assert!(matches!(
        m.mode(),
        Mode::Popup(PopupState::Prompt(p)) if p.rename == Rename::Tab("tb4".into())
    ));
    drive(&mut m, &mut core, Action::Popup(PopupAction::Close));
    // New tab: down once; it goes after the selected tab, in its space.
    drive(&mut m, &mut core, Action::TabMenu("tb4".into()));
    key(&mut m, &mut core, "down", false);
    assert_eq!(menu(&m).cursor, 1);
    key(&mut m, &mut core, "enter", false);
    assert_eq!(*m.mode(), Mode::Normal);
    let ids = tab_ids(&core.doc(None), "sp1");
    assert_eq!(ids.len(), 8);
    assert_eq!(ids[0], "tb1");
    let new = ids[1].clone();
    assert_eq!(m.shown_tab().unwrap().id, new, "the new tab shows");
    let run = core.runs().pop().expect("a terminal for the new tab");
    assert_eq!(
        run.labels.get("herdr.space").map(String::as_str),
        Some("sp1")
    );
    // Close: up from the top wraps to it; it closes that tab, not the shown one.
    drive(&mut m, &mut core, Action::TabMenu("tb3".into()));
    key(&mut m, &mut core, "up", false);
    key(&mut m, &mut core, "enter", false);
    assert!(core.doc(None).tab("tb3").is_none());
    assert_eq!(m.shown_tab().unwrap().id, new);
}

#[test]
fn a_new_tab_in_a_space_not_shown_starts_in_that_spaces_folder() {
    let mut core = Core::new(
        doc(vec![
            space("sp1", vec![tab("tb1", vec![pane("p1", "t1")])]),
            json!({"id": "sp2", "name": "", "folder": "/home/me/code/two",
                   "tabs": [tab("tb2", vec![pane("p2", "t2")])]}),
        ]),
        vec![shell("t1", "/home/me/code/one"), shell("t2", "/")],
    );
    let mut m = connected(&mut core);
    let fx = drive(&mut m, &mut core, Action::NewTabIn("sp2".into()));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Toast(_))), "{fx:?}");
    let run = core.runs().pop().unwrap();
    assert_eq!(run.cwd.as_deref(), Some("/home/me/code/two"));
    assert_eq!(tab_ids(&core.doc(None), "sp2").len(), 2);
    assert_eq!(m.active_space().unwrap().id, "sp2", "the new tab shows");
    drive(&mut m, &mut core, Action::NewTabIn("gone".into()));
    assert_eq!(core.runs().len(), 1, "no space, no tab");
}
