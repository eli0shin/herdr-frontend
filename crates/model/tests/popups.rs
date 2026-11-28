//! The pickers, the palette, the prompt, the confirm dialog and the menus through `Model::step`
//! and `model::popups` (`feature-inventory.md` §3, §10).

mod support;

use herdr_proto::types::{MachineState, MachineView};
use herdr_proto::{StatusState, Terminal};
use herdr_shared::{Displayed, Document};
use model::mode::{Confirmed, ListPopup, PopupState, Rename};
use model::popups::{self, PickerRow};
use model::{
    Action, AppCommand, Effect, Event, MenuIcon, Mode, Popup, PopupAction, Request, Split,
};
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
    // Eli's prefix key, not cmd-t; with none, another (zoom has only cmd-shift-enter).
    assert_eq!(rows[0].key.as_deref(), Some("ctrl-g t"));
    let key_of = |title: &str| {
        rows.iter()
            .find(|r| r.title == title)
            .unwrap_or_else(|| panic!("no {title}"))
            .key
            .clone()
    };
    assert_eq!(key_of("Next tab").as_deref(), Some("ctrl-g n"));
    assert_eq!(key_of("Previous tab").as_deref(), Some("ctrl-g p"));
    assert_eq!(key_of("Close tab").as_deref(), Some("ctrl-g shift-q"));
    assert_eq!(key_of("Rename tab").as_deref(), Some("ctrl-g r"));
    assert_eq!(key_of("Zoom pane").as_deref(), Some("cmd-shift-enter"));
    // There are no defaults to reset to, only Eli's keys.
    assert!(
        rows.iter()
            .all(|r| r.action != Action::App(AppCommand::Reset))
    );
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
        Some("ctrl-g t")
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

/// A pane's name is its block's `title`, as Herdr app's `pane.rename` writes it: trimmed, shown
/// back in the prompt, and taken away by an empty name (`app:src/window.rs` `pane.rename`).
#[test]
fn the_terminal_menus_rename_names_the_pane_in_the_layout() {
    let mut core = world();
    let mut m = connected(&mut core);
    let rename = |m: &mut model::Model, core: &mut Core, name: &str| {
        drive(m, core, Action::TerminalMenu("p2".into()));
        // Rename is the menu's first item.
        key(m, core, "enter", false);
        let Mode::Popup(PopupState::Prompt(prompt)) = m.mode().clone() else {
            panic!("{:?}", m.mode())
        };
        drive(m, core, Action::Popup(PopupAction::Query(name.into())));
        key(m, core, "enter", false);
        assert_eq!(*m.mode(), Mode::Normal);
        prompt.value
    };
    let title = |core: &Core| {
        let doc = core.doc(None);
        let node = doc.tab("tb2").unwrap().root.find("p2").unwrap();
        node.extra.get("title").cloned()
    };

    assert_eq!(
        rename(&mut m, &mut core, "  server  "),
        "",
        "unnamed: empty"
    );
    assert_eq!(title(&core), Some(json!("server")), "trimmed");
    assert_eq!(
        m.world()
            .tab("tb2")
            .unwrap()
            .root
            .find("p2")
            .unwrap()
            .title(),
        Some("server")
    );
    // The prompt starts with the name; an empty one takes it away.
    assert_eq!(rename(&mut m, &mut core, " "), "server");
    assert_eq!(title(&core), None);
    assert_eq!(
        core.doc(None),
        Document::from_value(world().docs[&None].0.clone()).unwrap()
    );

    // A second rename while the prompt is open closes it; a pane that's gone opens nothing.
    drive(&mut m, &mut core, Action::RenamePaneOf("p2".into()));
    drive(&mut m, &mut core, Action::RenamePaneOf("p2".into()));
    assert_eq!(*m.mode(), Mode::Normal);
    drive(&mut m, &mut core, Action::RenamePaneOf("gone".into()));
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
    // Rename asks for the pane's name in the prompt.
    let fx = m.step(m.now(), Action::Popup(PopupAction::Choose));
    assert!(fx.list.iter().all(|e| !matches!(e, Effect::NotYet(_))));
    let Mode::Popup(PopupState::Prompt(prompt)) = m.mode() else {
        panic!("{:?}", m.mode())
    };
    assert_eq!(prompt.rename, Rename::Pane("pk".into()));
    press(&mut m, "escape");

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

/// A right-click in the peek: the terminal menu for the peeked terminal, without the pane items,
/// over the peek (`menu/main.js` `items`, a terminal outside a tab).
#[test]
fn the_peeks_terminal_menu_leaves_the_pane_items_out() {
    let mut core = world();
    let mut m = connected(&mut core);
    assert_eq!(popups::peek_menu(&m), None, "no peek, no menu");
    drive(&mut m, &mut core, Action::PeekMenu);
    assert_eq!(*m.mode(), Mode::Normal);
    // k2, a2's sub-agent, has no pane.
    drive(&mut m, &mut core, Action::Peek("k2".into()));
    drive(&mut m, &mut core, Action::PeekMenu);
    let Mode::Popup(PopupState::Menu(menu)) = m.mode().clone() else {
        panic!("{:?}", m.mode())
    };
    assert_eq!(menu.target, model::MenuTarget::Terminal("k2".into()));
    assert_eq!(menu.width, 220.0);
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Send right-clicks to pane", "Send back to a2"]);
    assert_eq!(m.peeked(), Some("k2"), "the menu is over the peek");
    press(&mut m, "escape");
    assert_eq!(*m.mode(), Mode::Normal);
    assert_eq!(m.peeked(), Some("k2"));

    // Send back, with no pane to take away: its place labels and its keep go, as Herdr app's
    // `terminal.park` does, and it keeps running.
    let before = core.calls.len();
    let fx = drive(&mut m, &mut core, Action::Park("k2".into()));
    assert!(fx.iter().all(|e| !matches!(e, Effect::Toast(_))), "{fx:?}");
    let calls = &core.calls[before..];
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Request::Label(l) if l.terminal == "k2"
            && l.labels.get("herdr.space").map(String::as_str) == Some(""))),
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Request::Keep(k) if k.terminal == "k2" && !k.keep)),
        "{calls:?}"
    );
    assert!(core.kills().is_empty());
    assert_eq!(m.peeked(), Some("k2"));
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

// ---- The new-space folder picker (`cmd-n`, §11) ----------------------------------------------

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

const ZOXIDE: &[&str] = &["zoxide", "query", "-l"];

fn repos_argv() -> Vec<String> {
    argv(&["sh", "-c", model::folders::REPOS_CONFIG])
}

/// The picker's folders, as shown: `~/…` paths and tags.
fn folder_rows(m: &model::Model, query: &str) -> Vec<String> {
    popups::new_space_rows(m, query)
        .iter()
        .map(|f| {
            let shown = model::folders::display_path(&f.path, Some(HOME));
            match f.source.tag() {
                Some(tag) => format!("{shown} ({tag})"),
                None => shown,
            }
        })
        .collect()
}

/// `world()` whose zoxide knows two folders and whose repos config lists two more.
fn world_with_folders() -> Core {
    let mut core = world();
    core.exec_by.insert(
        argv(ZOXIDE),
        printed("/home/me/code/api\n/home/me/code/one\n"),
    );
    core.exec_by.insert(
        repos_argv(),
        printed(r#"{"repos": [{"path": "/home/me/code/api"}, {"path": "/home/me/code/web/"}]}"#),
    );
    core
}

#[test]
fn the_folder_picker_offers_where_you_are_then_zoxide_then_repos() {
    let mut core = world_with_folders();
    let mut m = connected(&mut core);
    let fx = key(&mut m, &mut core, "cmd-n", false);
    assert!(fx.iter().all(|e| !matches!(e, Effect::NotYet(_))));
    assert_eq!(list(&m).popup, Popup::NewSpace);
    assert_eq!(
        core.execs(),
        [(argv(ZOXIDE), None), (repos_argv(), None)],
        "this computer's lookups, and its home is known"
    );
    let timeouts: Vec<Option<f64>> = core
        .calls
        .iter()
        .filter_map(|c| match c {
            Request::Exec(e) => Some(e.timeout),
            _ => None,
        })
        .collect();
    assert_eq!(timeouts, [Some(5.0), Some(5.0)]);
    let picker = m.folder_picker().unwrap();
    assert!(!picker.loading());
    assert_eq!(picker.machine, None);
    assert_eq!(
        folder_rows(&m, ""),
        ["~/code/one (here)", "~/code/api", "~/code/web (repo)"]
    );
    assert_eq!(folder_rows(&m, "we"), ["~/code/web (repo)"]);
    assert_eq!(
        folder_rows(&m, "~/code/new/"),
        ["~/code/new (path)"],
        "a typed path is offered even where nothing matches"
    );
    // `ctrl-g ctrl-s` opens it too.
    drive(&mut m, &mut core, Action::Popup(PopupAction::Close));
    key(&mut m, &mut core, "ctrl-g ctrl-s", false);
    assert_eq!(list(&m).popup, Popup::NewSpace);
}

#[test]
fn failed_lookups_leave_only_where_you_are() {
    let mut core = world();
    core.exec = Err(error(
        herdr_proto::ErrorKind::Other("spawn_failed".into()),
        "no zoxide",
    ));
    core.exec_by.insert(repos_argv(), exited(1));
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::Open(Popup::NewSpace));
    assert!(!m.folder_picker().unwrap().loading());
    assert_eq!(folder_rows(&m, ""), ["~/code/one (here)"]);
}

#[test]
fn enter_while_the_folders_are_loading_waits_when_nothing_matches() {
    let mut core = world_with_folders();
    core.exec_held = true;
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::Open(Popup::NewSpace));
    assert!(m.folder_picker().unwrap().loading());
    assert_eq!(folder_rows(&m, ""), ["~/code/one (here)"]);
    drive(
        &mut m,
        &mut core,
        Action::Popup(PopupAction::Query("zzz".into())),
    );
    let fx = drive(&mut m, &mut core, Action::Popup(PopupAction::Choose));
    assert!(fx.iter().all(|e| !matches!(e, Effect::Pick { .. })));
    assert_eq!(list(&m).popup, Popup::NewSpace, "still open");
    // Matching where you are, Enter goes at once.
    drive(
        &mut m,
        &mut core,
        Action::Popup(PopupAction::Query("one".into())),
    );
    let fx = drive(&mut m, &mut core, Action::Popup(PopupAction::Choose));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Pick {
            popup: Popup::NewSpace,
            ..
        }
    )));
    assert_eq!(*m.mode(), Mode::Normal);
}

#[test]
fn picking_a_folder_makes_the_space_there_and_shows_it() {
    let mut core = world_with_folders();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::Open(Popup::NewSpace));
    let action = popups::new_space_pick(&m, "api", 0).unwrap();
    assert_eq!(
        action,
        Action::NewSpaceIn {
            machine: None,
            folder: Some("/home/me/code/api".into())
        }
    );
    drive(&mut m, &mut core, Action::Popup(PopupAction::Close));
    drive(&mut m, &mut core, action);
    let run = core.runs().pop().unwrap();
    assert_eq!(run.cwd.as_deref(), Some("/home/me/code/api"));
    assert_eq!(run.on, None);
    let doc = core.doc(None);
    let space = doc.space(&run.labels["herdr.space"]).unwrap();
    assert_eq!(space.folder, "/home/me/code/api");
    assert_eq!(space.name, "", "named after its folder");
    assert_eq!(m.active_space().unwrap().id, space.id, "shown");

    // Nothing matched: where plain `space.new` starts, the focused terminal's folder.
    drive(&mut m, &mut core, Action::ShowTab("tb1".into()));
    drive(&mut m, &mut core, Action::Open(Popup::NewSpace));
    let none = popups::new_space_pick(&m, "zzz", 0).unwrap();
    assert_eq!(
        none,
        Action::NewSpaceIn {
            machine: None,
            folder: None
        }
    );
    drive(&mut m, &mut core, none);
    assert_eq!(
        core.runs().pop().unwrap().cwd.as_deref(),
        Some("/home/me/code/one")
    );
}

#[test]
fn on_another_machine_the_folders_and_home_come_from_there() {
    let mut core = world();
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![space("sp9", vec![tab("tb9", vec![pane("p9", "t9")])])]),
            4,
        ),
    );
    core.terminals
        .insert("devbox/t9".into(), shell("devbox/t9", "/srv/app"));
    let home = argv(&["sh", "-c", r#"printf %s "$HOME""#]);
    core.exec_by.insert(home.clone(), printed("/root"));
    core.exec_by
        .insert(argv(ZOXIDE), printed("/root/code/svc\n/srv/app\n"));
    let mut m = connected(&mut core);
    let mut devbox = MachineView {
        state: MachineState::Connected,
        ..Default::default()
    };
    devbox.machine.name = "devbox".into();
    drive(&mut m, &mut core, Event::Machines(vec![devbox]));
    drive(&mut m, &mut core, Action::ShowSpace("devbox/sp9".into()));
    assert_eq!(m.focused_terminal(), Some("devbox/t9"));
    drive(&mut m, &mut core, Action::Open(Popup::NewSpace));
    let on = Some("devbox".to_owned());
    assert_eq!(
        core.execs(),
        [
            (home, on.clone()),
            (argv(ZOXIDE), on.clone()),
            (repos_argv(), on.clone())
        ]
    );
    let picker = m.folder_picker().unwrap();
    assert_eq!(picker.machine.as_deref(), Some("devbox"));
    assert_eq!(picker.home.as_deref(), Some("/root"));
    let rows: Vec<String> = popups::new_space_rows(&m, "")
        .iter()
        .map(|f| model::folders::display_path(&f.path, picker.home.as_deref()))
        .collect();
    assert_eq!(rows, ["/srv/app", "~/code/svc"]);
    assert_eq!(
        popups::new_space_rows(&m, "~/x")[0].path,
        "/root/x",
        "`~` is that machine's home"
    );
    let action = popups::new_space_pick(&m, "svc", 0).unwrap();
    drive(&mut m, &mut core, Action::Popup(PopupAction::Close));
    drive(&mut m, &mut core, action);
    let run = core.runs().pop().unwrap();
    assert_eq!(run.on.as_deref(), Some("devbox"));
    assert_eq!(run.cwd.as_deref(), Some("/root/code/svc"));
    assert_eq!(core.doc(Some("devbox")).spaces.len(), 2, "made there");
}

#[test]
fn a_late_answer_for_an_earlier_opening_is_dropped() {
    let mut core = world_with_folders();
    core.exec_held = true;
    let mut m = connected(&mut core);
    let first = m.step(core.now, Action::Open(Popup::NewSpace)).list;
    let tokens: Vec<model::Token> = first
        .iter()
        .filter_map(|e| match e {
            Effect::Call {
                token,
                request: Request::Exec(_),
            } => Some(*token),
            _ => None,
        })
        .collect();
    assert_eq!(tokens.len(), 2);
    drive(&mut m, &mut core, Action::Popup(PopupAction::Close));
    drive(&mut m, &mut core, Action::Open(Popup::NewSpace));
    for token in tokens {
        drive(
            &mut m,
            &mut core,
            Event::Executed {
                token,
                result: printed("/elsewhere"),
            },
        );
    }
    let picker = m.folder_picker().unwrap();
    assert!(picker.loading(), "the new opening still waits");
    assert_eq!(folder_rows(&m, ""), ["~/code/one (here)"]);
    // A dropped connection stops the waiting.
    drive(&mut m, &mut core, Event::Lost);
    assert!(!m.folder_picker().unwrap().loading());
}
