//! The pickers, the palette, the prompt, the confirm dialog and the menus through `Model::step`
//! and `model::popups` (`feature-inventory.md` §3, §10).

mod support;

use herdr_proto::types::{MachineState, MachineView};
use herdr_proto::{StatusState, Terminal};
use herdr_shared::Displayed;
use model::mode::{Confirmed, ListPopup, PopupState, Rename};
use model::popups::{self, PickerRow};
use model::{Action, Effect, Event, MenuIcon, Mode, Popup, PopupAction, Request, Split};
use serde_json::json;
use support::*;

/// sp1 "one" (folder ~/code/one): tb1 (t1, a shell), tb2 (agent a2, working, with sub-agent k2);
/// sp2 "two": tb3 (agent a3, blocked); sp3 (unnamed, folder ~/code/quiet): tb4 (t4, a shell).
fn world() -> Core {
    let named = |id: &str, name: &str, folder: &str, tabs| json!({"id": id, "name": name, "folder": folder, "tabs": tabs});
    let mut kid = agent("k2", StatusState::Working);
    kid.parent = Some("a2".into());
    kid.keep = false;
    Core::new(
        doc(vec![
            named(
                "sp1",
                "one",
                "/home/me/code/one",
                vec![
                    tab("tb1", vec![pane("p1", "t1")]),
                    tab("tb2", vec![pane("p2", "a2")]),
                ],
            ),
            named(
                "sp2",
                "two",
                "/home/me/code/two",
                vec![tab("tb3", vec![pane("p3", "a3")])],
            ),
            named(
                "sp3",
                "",
                "/home/me/code/quiet",
                vec![tab("tb4", vec![pane("p4", "t4")])],
            ),
        ]),
        vec![
            shell("t1", "/home/me/code/one"),
            agent("a2", StatusState::Working),
            kid,
            agent("a3", StatusState::Blocked),
            shell("t4", "/home/me/code/quiet"),
        ],
    )
}

fn program(t: &Terminal) -> String {
    if t.status.is_some() { "pi" } else { "shell" }.into()
}

fn list(m: &model::Model) -> ListPopup {
    match m.mode() {
        Mode::Popup(PopupState::List(l)) => l.clone(),
        other => panic!("{other:?}"),
    }
}

fn press(m: &mut model::Model, keys: &str) -> model::Effects {
    let mut last = model::Effects::default();
    for chord in model::Chord::parse_sequence(keys).unwrap() {
        last = m.step(m.now(), Event::key(chord, false));
    }
    last
}

#[test]
fn the_space_picker_numbers_spaces_and_ranks_names_above_folders() {
    let mut core = world();
    let m = connected(&mut core);
    let rows = popups::space_rows(&m, "");
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["one", "two", "quiet"]);
    assert_eq!(rows[0].number, Some(1));
    assert!(rows[0].shown);
    assert_eq!(rows[0].tabs, 2);
    assert_eq!(rows[0].status, Some(Displayed::Working));
    assert_eq!(rows[1].status, Some(Displayed::Blocked));
    assert_eq!(rows[2].status, None);
    assert_eq!(popups::glyph(rows[0].status), "◐");
    assert_eq!(popups::glyph(rows[1].status), "●");
    assert_eq!(popups::glyph(None), "·");
    assert_eq!(popups::space_cursor(&rows, ""), 0);

    // "o" is in every folder (~/code/…), but a name match counts first.
    let ranked: Vec<String> = popups::space_rows(&m, "o")
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(ranked, ["one", "two", "quiet"]);
    let ranked: Vec<String> = popups::space_rows(&m, "tw")
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(ranked, ["two"]);
    // The folder counts too, with `~` for home.
    let ranked: Vec<String> = popups::space_rows(&m, "~/code/qu")
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(ranked, ["quiet"]);
    assert_eq!(popups::space_cursor(&popups::space_rows(&m, "tw"), "tw"), 0);
}

#[test]
fn going_to_a_space_shows_a_terminal_tab_when_its_selected_one_is_an_agents() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::ShowTab("tb2".into()));
    drive(&mut m, &mut core, Action::ShowTab("tb4".into()));
    drive(&mut m, &mut core, Action::ShowSpace("sp1".into()));
    assert_eq!(m.shown_tab().unwrap().id, "tb1", "tb2 is an agent's");
    // A space with only agents' tabs shows as it was.
    drive(&mut m, &mut core, Action::ShowSpace("sp2".into()));
    assert_eq!(m.active_space().unwrap().id, "sp2");
    assert_eq!(m.shown_tab().unwrap().id, "tb3");
}

#[test]
fn the_picker_puts_busy_spaces_first_and_folds_quiet_ones() {
    let mut core = world();
    let mut m = connected(&mut core);
    let mut devbox = MachineView {
        state: MachineState::Connected,
        ..Default::default()
    };
    devbox.machine.name = "devbox".into();
    drive(&mut m, &mut core, Event::Machines(vec![devbox]));
    let rows = popups::picker_rows(&m, "", false, &program);
    let shape: Vec<String> = rows
        .iter()
        .map(|r| match r {
            PickerRow::Head { name, .. } => format!("# {name}"),
            PickerRow::Tab { name, .. } => format!("tab {name}"),
            PickerRow::Earlier { count, open } => format!("earlier {count} {open}"),
            PickerRow::NewTab => "new tab".into(),
            PickerRow::NewSpace(m) => format!("new space {m:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "# two",
            "tab 1",
            "# one",
            "tab 1",
            "tab 2",
            "earlier 1 false",
            "new tab",
            "new space None",
            "new space Some(\"devbox\")",
        ],
        "blocked first, then working; the quiet space folds"
    );
    let PickerRow::Tab {
        who, team, status, ..
    } = &rows[4]
    else {
        panic!()
    };
    assert_eq!((who.as_str(), *team), ("pi", 1));
    assert_eq!(*status, Some(Displayed::Working));
    // Opens on the tab on screen: sp1's tb1, the third pickable row.
    assert_eq!(popups::picker_cursor(&rows, ""), 1);
    let open = popups::picker_rows(&m, "", true, &program);
    assert!(
        open.iter()
            .any(|r| matches!(r, PickerRow::Tab { tab, .. } if tab == "tb4"))
    );
    assert!(open.contains(&PickerRow::Earlier {
        count: 1,
        open: true
    }));

    // A filter keeps matching tabs by space, tab or program, and shows quiet spaces too.
    let pi = popups::picker_rows(&m, "pi", false, &program);
    let tabs: Vec<&str> = pi
        .iter()
        .filter_map(|r| match r {
            PickerRow::Tab { tab, .. } => Some(tab.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(tabs, ["tb3", "tb2"]);
    let quiet = popups::picker_rows(&m, "quiet", false, &program);
    assert!(
        quiet
            .iter()
            .any(|r| matches!(r, PickerRow::Tab { tab, .. } if tab == "tb4"))
    );
    assert_eq!(
        popups::picker_cursor(&quiet, "quiet"),
        1,
        "the first tab, past earlier"
    );

    // What picking runs.
    assert_eq!(rows[1].action(), Some(Action::ShowTab("tb3".into())));
    assert_eq!(rows[1].close_item(), Some(Action::CloseTabOf("tb3".into())));
    assert_eq!(rows[6].action(), Some(Action::TabNew));
    assert_eq!(
        rows[8].action(),
        Some(Action::SpaceNew(Some("devbox".into())))
    );
    assert_eq!(rows[5].action(), None, "earlier folds");
    assert!(!rows[0].pickable());
}

#[test]
fn avatars_are_the_first_emoji_or_letter() {
    assert_eq!(popups::avatar("herdr"), "H");
    assert_eq!(popups::avatar("🦀 crab"), "🦀");
    assert_eq!(popups::avatar("👩‍💻 dev"), "👩‍💻");
    assert_eq!(popups::avatar("❤️ love"), "❤️");
    assert_eq!(popups::avatar(""), "?");
}

#[test]
fn a_new_space_from_the_picker_starts_in_the_focused_folder_and_shows() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::SpaceNew(None));
    let run = core.runs().pop().expect("terminal.run");
    assert_eq!(run.cwd.as_deref(), Some("/home/me/code/one"));
    assert!(run.argv.is_empty());
    assert_eq!(core.doc(None).spaces.len(), 4);
    assert_eq!(m.active_space().unwrap().id, core.doc(None).spaces[3].id);
}

#[test]
fn the_palette_lists_recent_commands_first_then_every_group() {
    let mut core = world();
    let mut m = connected(&mut core);
    let rows = popups::palette_rows(&m, "");
    assert_eq!(rows[0].title, "New tab");
    assert_eq!(rows[0].head, "Tab");
    assert_eq!(rows[0].key.as_deref(), Some("cmd-t"));
    let groups: Vec<&str> = rows.iter().map(|r| r.head).collect();
    let mut order: Vec<&str> = groups.clone();
    order.dedup();
    assert_eq!(
        order,
        [
            "Tab", "Pane", "Space", "Layout", "Terminal", "Agent", "Window", "App"
        ]
    );

    // Running one from the palette remembers it first.
    drive(
        &mut m,
        &mut core,
        Action::FromPalette(Box::new(Action::PaneZoom)),
    );
    drive(
        &mut m,
        &mut core,
        Action::FromPalette(Box::new(Action::App(model::AppCommand::ZoomIn))),
    );
    assert_eq!(m.view().palette, ["app.zoom_in", "pane.zoom"]);
    let rows = popups::palette_rows(&m, "");
    assert_eq!(rows[0].title, "Zoom in");
    assert_eq!(rows[1].title, "Zoom pane");
    assert_eq!(
        (rows[0].head, rows[1].head, rows[2].head),
        ("Recent", "Recent", "Tab")
    );
    assert_eq!(
        rows.iter().filter(|r| r.title == "Zoom pane").count(),
        1,
        "a recent one isn't listed twice"
    );

    // Typing ranks by Eli's matching: the title first.
    let typed = popups::palette_rows(&m, "split r");
    assert_eq!(typed[0].action, Action::PaneSplit(Split::Right));
    // Every term matches somewhere: "r" in "Split down"'s keyword "horizontal".
    assert_eq!(typed[1].action, Action::PaneSplit(Split::Down));
    let typed = popups::palette_rows(&m, "kill");
    let titles: Vec<&str> = typed.iter().map(|r| r.title.as_str()).collect();
    // In the keywords: a whole field ("kill") beats a word in a longer one.
    assert_eq!(titles, ["Close pane", "Close tab", "Close space"]);
    assert!(popups::palette_rows(&m, "nothing like this").is_empty());
}

#[test]
fn a_palette_command_that_needs_a_name_asks_for_it() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::Open(Popup::Palette));
    assert!(!list(&m).wrap, "the palette stops at its ends");
    // The window picks, closes the palette, then runs the row.
    drive(&mut m, &mut core, Action::Popup(PopupAction::Close));
    drive(
        &mut m,
        &mut core,
        Action::FromPalette(Box::new(Action::TabRename)),
    );
    let Mode::Popup(PopupState::Prompt(prompt)) = m.mode() else {
        panic!("{:?}", m.mode())
    };
    assert_eq!(prompt.rename, Rename::Tab("tb1".into()));
}

#[test]
fn the_palette_cursor_stops_at_the_ends_and_pages_by_ten() {
    let mut core = world();
    let mut m = connected(&mut core);
    press(&mut m, "cmd-shift-p");
    m.step(m.now(), Event::PopupRows(25));
    press(&mut m, "up");
    assert_eq!(list(&m).cursor, 0);
    press(&mut m, "pagedown");
    assert_eq!(list(&m).cursor, 10);
    press(&mut m, "pagedown pagedown");
    assert_eq!(list(&m).cursor, 24);
    press(&mut m, "down");
    assert_eq!(list(&m).cursor, 24);
    press(&mut m, "pageup");
    assert_eq!(list(&m).cursor, 14);
    m.step(m.now(), Event::PopupCursor(3));
    assert_eq!(list(&m).cursor, 3);
    m.step(m.now(), Event::PopupCursor(99));
    assert_eq!(list(&m).cursor, 24, "clamped to the rows");
    // Other lists wrap.
    press(&mut m, "escape");
    press(&mut m, "ctrl-g s");
    press(&mut m, "up");
    assert_eq!(list(&m).cursor, 2);
}

#[test]
fn the_prompt_saves_trimmed_names_and_the_confirm_closes_the_space() {
    let mut core = world();
    let mut m = connected(&mut core);
    press(&mut m, "f2");
    drive(
        &mut m,
        &mut core,
        Action::Popup(PopupAction::Query("  mine ".into())),
    );
    let fx = drive(
        &mut m,
        &mut core,
        Event::key(model::Chord::parse("enter").unwrap(), false),
    );
    assert!(fx.iter().all(|e| !matches!(e, Effect::NotYet(_))));
    assert_eq!(*m.mode(), Mode::Normal);
    assert_eq!(core.doc(None).space("sp1").unwrap().tabs[0].name, "mine");

    // Escape cancels.
    press(&mut m, "shift-f2");
    m.step(m.now(), Action::Popup(PopupAction::Query("x".into())));
    press(&mut m, "escape");
    assert_eq!(core.doc(None).space("sp1").unwrap().name, "one");

    press(&mut m, "ctrl-g x");
    let Mode::Popup(PopupState::Confirm(confirm)) = m.mode() else {
        panic!("{:?}", m.mode())
    };
    assert_eq!(confirm.title, "Close space?");
    assert_eq!(confirm.detail, "one \u{2014} 2 tabs, 2 terminals");
    assert_eq!(confirm.then, Confirmed::CloseSpace("sp1".into()));
    key(&mut m, &mut core, "enter", false);
    assert_eq!(*m.mode(), Mode::Normal);
    assert!(core.doc(None).space("sp1").is_none());
    assert_eq!(core.kills(), ["t1", "a2"]);
}

#[test]
fn the_tab_menu_renames_makes_and_closes_tabs() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::TabMenu("tb1".into()));
    let Mode::Popup(PopupState::Menu(menu)) = m.mode() else {
        panic!("{:?}", m.mode())
    };
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Rename", "New tab", "Close"]);
    assert_eq!(menu.items[2].icon, MenuIcon::Close);
    assert!(menu.items[2].divided);
    assert_eq!(menu.items[0].hint, Some(Action::TabRename));
    assert_eq!(
        model::help::hint(m.keymap(), menu.items[1].hint.as_ref().unwrap()).as_deref(),
        Some("cmd-t")
    );
    // ↑ wraps to Close; Enter runs it (tb1 isn't the space's last tab).
    press(&mut m, "up");
    let fx = key(&mut m, &mut core, "enter", false);
    assert!(fx.iter().all(|e| !matches!(e, Effect::NotYet(_))));
    assert_eq!(*m.mode(), Mode::Normal);
    assert_eq!(tab_ids(&core.doc(None), "sp1"), ["tb2"]);
    // A tab that isn't there opens nothing.
    drive(&mut m, &mut core, Action::TabMenu("gone".into()));
    assert_eq!(*m.mode(), Mode::Normal);
}

#[test]
fn the_terminal_menu_splits_zooms_routes_right_clicks_and_sends_sub_agents_back() {
    let mut core = world();
    // k2, a2's sub-agent, kept in a pane of its own.
    let mut d = core.docs[&None].0.clone();
    d["spaces"][0]["tabs"][1]["root"]["children"]
        .as_array_mut()
        .unwrap()
        .push(pane("pk", "k2"));
    core.docs.insert(None, (d, 1));
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::TerminalMenu("pk".into()));
    let Mode::Popup(PopupState::Menu(menu)) = m.mode().clone() else {
        panic!("{:?}", m.mode())
    };
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Rename",
            "Split right",
            "Split down",
            "Zoom",
            "Send right-clicks to pane",
            "Send back to a2",
            "Close pane"
        ]
    );
    assert_eq!(menu.items[1].hint, Some(Action::PaneSplit(Split::Right)));
    assert_eq!(
        menu.items[1].actions,
        [
            Action::FocusPane("pk".into()),
            Action::PaneSplit(Split::Right)
        ]
    );
    // Rename says not yet (pane names come later).
    let fx = m.step(m.now(), Action::Popup(PopupAction::Choose));
    assert!(
        fx.list
            .contains(&Effect::NotYet(Action::RenamePaneOf("pk".into())))
    );

    // Right-clicks to the program, then back to the menu.
    drive(&mut m, &mut core, Action::TerminalMenu("pk".into()));
    m.step(m.now(), Event::PopupCursor(4));
    key(&mut m, &mut core, "enter", false);
    assert_eq!(
        core.terminals["k2"]
            .labels
            .get("herdr.right-click")
            .map(String::as_str),
        Some("program")
    );
    let menu = popups::terminal_menu(&m, "pk").unwrap();
    assert_eq!(menu.items[4].label, "Use Herdr right-click menu");
    drive(&mut m, &mut core, menu.items[4].actions[0].clone());
    assert!(
        !core.terminals["k2"]
            .labels
            .contains_key("herdr.right-click")
    );

    // Send back: labels cleared, keep off, then the pane goes; the terminal keeps running.
    let fx = drive(&mut m, &mut core, Action::Park("k2".into()));
    assert!(fx.iter().all(|e| !matches!(e, Effect::Toast(_))), "{fx:?}");
    let k2 = &core.terminals["k2"];
    assert!(!k2.keep);
    assert!(!k2.labels.contains_key("herdr.space"));
    assert!(core.doc(None).tabs().all(|t| t.root.find("pk").is_none()));
    assert!(core.kills().is_empty());
    let parks: Vec<&Request> = core
        .calls
        .iter()
        .filter(|c| matches!(c, Request::Label(_) | Request::Keep(_)))
        .collect();
    assert!(matches!(parks.last(), Some(Request::Keep(k)) if !k.keep));

    // A shell's menu has no "Send back".
    let menu = popups::terminal_menu(&m, "p1").unwrap();
    assert!(menu.items.iter().all(|i| !i.label.starts_with("Send back")));
    // Split right from the menu splits that pane.
    drive(&mut m, &mut core, Action::TerminalMenu("p1".into()));
    m.step(m.now(), Event::PopupCursor(1));
    key(&mut m, &mut core, "enter", false);
    let tb1 = core.doc(None).tab("tb1").cloned().unwrap();
    assert_eq!(tb1.root.panes().len(), 2);
}

#[test]
fn settings_files_open_in_a_new_tab_running_the_editor() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(
        &mut m,
        &mut core,
        Action::EditFile("/home/me/.config/herdr-frontend/settings.json".into()),
    );
    let run = core.runs().pop().expect("terminal.run");
    assert_eq!(
        run.argv,
        [
            "/bin/sh",
            "-c",
            "exec ${EDITOR:-nvim} \"$1\"",
            "sh",
            "/home/me/.config/herdr-frontend/settings.json"
        ]
    );
    assert_eq!(run.cwd.as_deref(), Some("/home/me/.config/herdr-frontend"));
    assert_eq!(run.on, None);
    let tabs = tab_ids(&core.doc(None), "sp1");
    assert_eq!(tabs.len(), 3, "a new tab in the shown space: {tabs:?}");
    assert_eq!(
        m.shown_tab().unwrap().id,
        tabs[1],
        "after the selected one, shown"
    );
}
