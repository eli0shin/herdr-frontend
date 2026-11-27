// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! What the popups list and what picking a row runs (`feature-inventory.md` §3, §10): the space
//! picker, the tab and agent picker, the command palette, and the tab and terminal menus. Pure:
//! the window draws these rows and turns a pick into the row's [`Action`]s, so every rule here
//! is tested without a window.
//!
//! Every filter is Eli's ([`crate::filter`]). The window passes what only it knows: the
//! programs' names (from `programs.json`) for the picker.

use herdr_proto::Terminal;
use herdr_shared::{Displayed, Space, Tab, labels, machine_of};

use crate::action::{Action, AppCommand, Direction, Popup, Split, TerminalAction};
use crate::filter;
use crate::folders::Folder;
use crate::mode::{Menu, MenuIcon, MenuItem, MenuTarget};
use crate::model::Model;
use crate::world::tab_name;

/// A space's status, rolled up over its terminals.
fn space_status(m: &Model, space: &Space) -> Option<Displayed> {
    let addresses: Vec<String> = space.tabs.iter().flat_map(|t| t.root.addresses()).collect();
    m.attention().rollup(addresses.iter().map(String::as_str))
}

fn tab_status(m: &Model, tab: &Tab) -> Option<Displayed> {
    let addresses = tab.root.addresses();
    m.attention().rollup(addresses.iter().map(String::as_str))
}

/// The machine a space lives on, `None` for this computer.
fn machine_name(id: &str) -> Option<String> {
    machine_of(id).map(str::to_owned)
}

// ---- The space picker (`ctrl-g s`, `cfg/mods/space-picker`) ----------------------------------

/// One space picker row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceRow {
    pub id: String,
    /// Its place in space order, the `ctrl-1`…`ctrl-9` that shows it; `None` past 9.
    pub number: Option<usize>,
    pub status: Option<Displayed>,
    pub name: String,
    /// The machine, for a space on another one.
    pub machine: Option<String>,
    pub tabs: usize,
    /// The space on screen (its name is semibold).
    pub shown: bool,
}

/// The status glyph the space picker shows (`GLYPH`): ● needs you or finished, ◐ working,
/// ○ idle, · no agent.
pub fn glyph(status: Option<Displayed>) -> &'static str {
    match status {
        Some(Displayed::Blocked | Displayed::Done) => "●",
        Some(Displayed::Working) => "◐",
        Some(Displayed::Idle) => "○",
        None => "·",
    }
}

/// The branch of the terminal a space shows first: its selected tab's focused pane's
/// ([`crate::checkouts`]; `app:src/window.rs` `publish`).
pub fn space_branch(m: &Model, space: &Space) -> Option<String> {
    let world = m.world();
    let tab = m.view().selected_tab(world, space)?;
    let pane = m.view().focused_in(tab)?;
    let address = world.address_of(&pane)?;
    world.checkout(address)?.branch.clone()
}

/// The spaces matching `query`, best first (`rankSpaces`): the name counts above the branch of
/// the terminal it shows first, its folder (`~` for home) and its machine.
pub fn space_rows(m: &Model, query: &str) -> Vec<SpaceRow> {
    let shown = m.active_space().map(|s| s.id.clone());
    let all: Vec<SpaceRow> = m
        .world()
        .spaces()
        .enumerate()
        .map(|(i, space)| SpaceRow {
            id: space.id.clone(),
            number: (i < 9).then_some(i + 1),
            status: space_status(m, space),
            name: m.world().space_name(space, m.home()),
            machine: machine_name(&space.id),
            tabs: space.tabs.len(),
            shown: shown.as_deref() == Some(space.id.as_str()),
        })
        .collect();
    let fields: Vec<(Option<String>, String)> = m
        .world()
        .spaces()
        .map(|s| {
            (
                space_branch(m, s),
                filter::display_path(&s.folder, m.home()),
            )
        })
        .collect();
    let order = filter::rank(query, &(0..all.len()).collect::<Vec<_>>(), |&i| {
        let row = &all[i];
        let (branch, folder) = &fields[i];
        let others = [branch.clone(), Some(folder.clone()), row.machine.clone()];
        (row.name.clone(), others.into_iter().flatten().collect())
    });
    order.into_iter().map(|i| all[i].clone()).collect()
}

/// Where the space picker's cursor starts: on the shown space with no query, else on the best
/// match.
pub fn space_cursor(rows: &[SpaceRow], query: &str) -> usize {
    if !query.trim().is_empty() {
        return 0;
    }
    rows.iter().position(|r| r.shown).unwrap_or(0)
}

/// The tab going to a space shows: its selected tab unless that is hidden (an agent's or the
/// review's), else its first shown tab; `None` keeps the space as it was (`terminalTab`).
pub fn terminal_tab(m: &Model, space: &Space) -> Option<String> {
    let selected = m.view().selected_tab(m.world(), space);
    if let Some(tab) = selected
        && !m.is_hidden_tab(tab)
    {
        return Some(tab.id.clone());
    }
    space
        .tabs
        .iter()
        .find(|t| !m.is_hidden_tab(t))
        .map(|t| t.id.clone())
}

// ---- The new-space folder picker (`cmd-n`, `cfg/mods/new-space`) -------------------------------

/// The folder picker's rows for `query` ([`crate::folders::NewSpace::rows`]): a typed path first,
/// then the folders it matches, best first.
pub fn new_space_rows(m: &Model, query: &str) -> Vec<Folder> {
    m.folder_picker()
        .map(|ns| ns.rows(query))
        .unwrap_or_default()
}

/// What Enter on row `cursor` runs: a space on the picker's machine in that folder; with no such
/// row (nothing matched), where plain `space.new` would start it (`new-space/main.js` `go`).
pub fn new_space_pick(m: &Model, query: &str, cursor: usize) -> Option<Action> {
    let ns = m.folder_picker()?;
    Some(Action::NewSpaceIn {
        machine: ns.machine.clone(),
        folder: ns.rows(query).into_iter().nth(cursor).map(|f| f.path),
    })
}

// ---- The tab and agent picker (`cmd-p`, the shipped `picker`) ---------------------------------

/// One picker row. Group titles can't be picked; every other row can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerRow {
    /// A space's title: its avatar (first emoji or letter) and name.
    Head { space: String, name: String },
    Tab {
        space: String,
        tab: String,
        name: String,
        status: Option<Displayed>,
        /// The tab on screen (semibold).
        here: bool,
        /// Its agents' program names, ` · ` between.
        who: String,
        /// How many sub-agents run under them.
        team: usize,
    },
    /// "earlier: N quiet spaces ▾": spaces with no agent, folded until picked (or a filter is
    /// typed).
    Earlier { count: usize, open: bool },
    /// "new tab".
    NewTab,
    /// "new space", or "new space on <machine>" for each connected machine.
    NewSpace(Option<String>),
}

impl PickerRow {
    pub fn pickable(&self) -> bool {
        !matches!(self, PickerRow::Head { .. })
    }

    /// What Enter (or a click) on it runs. The "earlier" row folds and unfolds instead: `None`.
    pub fn action(&self) -> Option<Action> {
        match self {
            PickerRow::Tab { tab, .. } => Some(Action::ShowTab(tab.clone())),
            PickerRow::NewTab => Some(Action::TabNew),
            PickerRow::NewSpace(machine) => Some(Action::SpaceNew(machine.clone())),
            PickerRow::Head { .. } | PickerRow::Earlier { .. } => None,
        }
    }

    /// What Ctrl+D on it runs: closing a tab.
    pub fn close_item(&self) -> Option<Action> {
        match self {
            PickerRow::Tab { tab, .. } => Some(Action::CloseTabOf(tab.clone())),
            _ => None,
        }
    }
}

/// The picker's rows (`Picker.rows`): spaces with an agent first (one that needs you first),
/// each titled and followed by its tabs (agents' tabs too); then quiet spaces folded into
/// "earlier" unless `earlier` is open or a filter is typed; then the new-work rows. A filter
/// keeps tabs whose space, tab or agents' program names match it. `program` names a terminal's
/// program.
pub fn picker_rows(
    m: &Model,
    query: &str,
    earlier: bool,
    program: &dyn Fn(&Terminal) -> String,
) -> Vec<PickerRow> {
    let world = m.world();
    let terms = filter::terms(query);
    let shown_tab = m.shown_tab().map(|t| t.id.clone());
    let live: Vec<&Terminal> = world.live().collect();
    let team = |agents: &[String]| {
        let mut n = 0;
        let mut queue: Vec<String> = agents.to_vec();
        while let Some(a) = queue.pop() {
            for kid in live
                .iter()
                .filter(|t| t.parent.as_deref() == Some(a.as_str()))
            {
                n += 1;
                queue.push(kid.address.clone());
            }
        }
        n
    };
    struct Group {
        head: PickerRow,
        status: Option<Displayed>,
        tabs: Vec<PickerRow>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for space in world.spaces() {
        let name = world.space_name(space, m.home());
        let tabs: Vec<PickerRow> = space
            .tabs
            .iter()
            .filter_map(|tab| {
                let agents: Vec<String> = tab
                    .root
                    .addresses()
                    .into_iter()
                    .filter(|a| world.is_agent(a))
                    .collect();
                let names: Vec<String> = agents
                    .iter()
                    .map(|a| world.terminal(a).map_or_else(|| "agent".into(), program))
                    .collect();
                let tab_label = tab_name(space, tab);
                if !terms.is_empty() {
                    let mut others = vec![name.clone()];
                    others.extend(names.iter().cloned());
                    let others: Vec<&str> = others.iter().map(String::as_str).collect();
                    filter::score(&terms, &tab_label, &others)?;
                }
                Some(PickerRow::Tab {
                    space: space.id.clone(),
                    tab: tab.id.clone(),
                    name: tab_label,
                    status: tab_status(m, tab),
                    here: shown_tab.as_deref() == Some(tab.id.as_str()),
                    who: names.join(" · "),
                    team: team(&agents),
                })
            })
            .collect();
        if tabs.is_empty() {
            continue;
        }
        groups.push(Group {
            head: PickerRow::Head {
                space: space.id.clone(),
                name,
            },
            status: space_status(m, space),
            tabs,
        });
    }
    let (mut busy, quiet): (Vec<Group>, Vec<Group>) =
        groups.into_iter().partition(|g| g.status.is_some());
    // Stable: blocked spaces first, the rest in space order.
    busy.sort_by_key(|g| g.status != Some(Displayed::Blocked));
    let mut out = Vec::new();
    let add = |g: Group, out: &mut Vec<PickerRow>| {
        out.push(g.head);
        out.extend(g.tabs);
    };
    for g in busy {
        add(g, &mut out);
    }
    if !quiet.is_empty() {
        out.push(PickerRow::Earlier {
            count: quiet.len(),
            open: earlier,
        });
        if earlier || !terms.is_empty() {
            for g in quiet {
                add(g, &mut out);
            }
        }
    }
    out.push(PickerRow::NewTab);
    out.push(PickerRow::NewSpace(None));
    for (key, machine) in world.machines() {
        if let Some(name) = key
            && machine.reachable()
        {
            out.push(PickerRow::NewSpace(Some(name.clone())));
        }
    }
    out
}

/// The rows the picker's cursor moves over: every row but the group titles.
pub fn pickable(rows: &[PickerRow]) -> Vec<&PickerRow> {
    rows.iter().filter(|r| r.pickable()).collect()
}

/// Where the picker's cursor starts: on the tab on screen when it opens; on the first tab once a
/// filter is typed.
pub fn picker_cursor(rows: &[PickerRow], query: &str) -> usize {
    let rows = pickable(rows);
    let at = if query.is_empty() {
        rows.iter()
            .position(|r| matches!(r, PickerRow::Tab { here: true, .. }))
    } else {
        rows.iter().position(|r| matches!(r, PickerRow::Tab { .. }))
    };
    at.unwrap_or(0)
}

/// A space's avatar: its name's first emoji, else its first letter, uppercased (`spaceAvatar`).
pub fn avatar(name: &str) -> String {
    let mut chars = name.chars().peekable();
    let Some(first) = chars.next() else {
        return "?".into();
    };
    if !is_pictograph(first) {
        return first.to_uppercase().collect();
    }
    // The whole emoji: its variation selector, skin tone and joined parts.
    let mut out = String::from(first);
    while let Some(&c) = chars.peek() {
        let joins = c == '\u{200d}';
        if joins || c == '\u{fe0f}' || ('\u{1f3fb}'..='\u{1f3ff}').contains(&c) {
            out.push(c);
            chars.next();
            if joins && let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            break;
        }
    }
    out
}

fn is_pictograph(c: char) -> bool {
    matches!(c, '\u{1f000}'..='\u{1faff}' | '\u{2600}'..='\u{27bf}' | '\u{2b00}'..='\u{2bff}')
}

// ---- The command palette (`cmd-shift-p`, the app's `frame-palette`) ----------------------------

/// The palette's groups, in the order they show (`GROUPS`).
pub const PALETTE_GROUPS: [&str; 10] = [
    "Recent", "Tab", "Pane", "Space", "Layout", "Terminal", "Agent", "Window", "Mod", "App",
];

/// How many recent commands the empty palette shows first.
pub const PALETTE_RECENT_SHOWN: usize = 5;

/// One palette row.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteRow {
    pub action: Action,
    /// One of [`PALETTE_GROUPS`] (not "Recent").
    pub group: &'static str,
    pub title: String,
    /// The group title it shows under when the palette is empty: "Recent" or its group.
    pub head: &'static str,
    /// The first key that runs it, in Eli's syntax.
    pub key: Option<String>,
}

/// Every palette entry: `(action, group, title, words)`, Herdr app's names (`NAMED`) for the
/// commands herdr-frontend has, plus its own popups. Places (spaces, tabs) are in the pickers;
/// commands that need a target (numbers, ids) aren't listed.
fn entries() -> Vec<(Action, &'static str, String, &'static str)> {
    let dir = |d: Direction| d.as_str();
    let mut list: Vec<(Action, &'static str, String, &'static str)> = vec![
        (
            Action::TabNew,
            "Tab",
            "New tab".into(),
            "tab agent new task",
        ),
        (
            Action::Open(Popup::Picker),
            "Tab",
            "Go to space or tab…".into(),
            "switch jump find picker",
        ),
        (Action::TabRename, "Tab", "Rename tab".into(), "tab"),
        (Action::TabClose, "Tab", "Close tab".into(), "tab kill"),
        (Action::TabNext, "Tab", "Next tab".into(), "tab"),
        (Action::TabPrev, "Tab", "Previous tab".into(), "tab"),
        (
            Action::JumpBack,
            "Tab",
            "Back in the tab jump list".into(),
            "jump history",
        ),
        (
            Action::JumpForward,
            "Tab",
            "Forward in the tab jump list".into(),
            "jump history",
        ),
        (
            Action::PaneSplit(Split::Right),
            "Pane",
            "Split right".into(),
            "vertical",
        ),
        (
            Action::PaneSplit(Split::Down),
            "Pane",
            "Split down".into(),
            "horizontal",
        ),
        (Action::PaneClose, "Pane", "Close pane".into(), "kill"),
        (
            Action::PaneZoom,
            "Pane",
            "Zoom pane".into(),
            "maximize fullscreen",
        ),
    ];
    for d in Direction::ALL {
        list.push((
            Action::PaneGo(d),
            "Pane",
            format!("Focus pane {}", dir(d)),
            "move go",
        ));
    }
    for d in Direction::ALL {
        list.push((
            Action::PaneSwap(d),
            "Pane",
            format!("Swap pane {}", dir(d)),
            "move",
        ));
    }
    list.extend([
        (
            Action::SpaceNew(None),
            "Space",
            "New space".into(),
            "workspace",
        ),
        (
            Action::Open(Popup::NewSpace),
            "Space",
            "New space in a folder…".into(),
            "workspace folder worktree",
        ),
        (
            Action::Open(Popup::SpacePicker),
            "Space",
            "Go to space…".into(),
            "workspace switch picker",
        ),
        (
            Action::SpaceRename,
            "Space",
            "Rename space".into(),
            "workspace",
        ),
        (
            Action::SpaceClose,
            "Space",
            "Close space".into(),
            "workspace kill",
        ),
        (Action::SpaceNext, "Space", "Next space".into(), "workspace"),
        (
            Action::SpacePrev,
            "Space",
            "Previous space".into(),
            "workspace",
        ),
        (
            Action::App(AppCommand::LayoutSwitch),
            "Layout",
            "Next layout".into(),
            "look switch",
        ),
        (
            Action::App(AppCommand::ToggleSidebar),
            "Layout",
            "Toggle sidebar".into(),
            "agents panel hide show",
        ),
        (
            Action::Terminal(TerminalAction::Copy),
            "Terminal",
            "Copy".into(),
            "clipboard",
        ),
        (
            Action::Terminal(TerminalAction::Paste),
            "Terminal",
            "Paste".into(),
            "clipboard",
        ),
        (
            Action::Terminal(TerminalAction::PasteImage),
            "Terminal",
            "Paste an image".into(),
            "clipboard picture",
        ),
        (
            Action::Terminal(TerminalAction::SelectAll),
            "Terminal",
            "Select all".into(),
            "selection",
        ),
        (
            Action::Terminal(TerminalAction::Search),
            "Terminal",
            "Find in screen and scrollback".into(),
            "search",
        ),
        (
            Action::CopyMode,
            "Terminal",
            "Copy mode".into(),
            "vim select keyboard",
        ),
        (Action::AgentNext, "Agent", "Next agent".into(), ""),
        (Action::AgentPrev, "Agent", "Previous agent".into(), ""),
        (
            Action::Open(Popup::AgentJump),
            "Agent",
            "Jump to an agent by label".into(),
            "jump",
        ),
        (
            Action::Open(Popup::AgentMachines),
            "Agent",
            "Machine filter".into(),
            "machines hide show",
        ),
        (
            Action::Open(Popup::PrFinder),
            "Agent",
            "Find a PR's agent".into(),
            "pull request github",
        ),
        (
            Action::Review,
            "Agent",
            "Review view".into(),
            "diff changes",
        ),
        (
            Action::App(AppCommand::CloseWindow),
            "Window",
            "Close window".into(),
            "terminals keep running",
        ),
        (
            Action::Open(Popup::Settings),
            "App",
            "Settings".into(),
            "preferences options config look font theme keys",
        ),
        (
            Action::Open(Popup::Machines),
            "App",
            "Machines".into(),
            "ssh remote servers login connect rename remove status",
        ),
        (
            Action::Open(Popup::MachineAdd),
            "App",
            "Add a machine".into(),
            "ssh remote server new",
        ),
        (
            Action::Open(Popup::Help),
            "App",
            "Keys".into(),
            "keybindings shortcuts help",
        ),
        (
            Action::App(AppCommand::Reset),
            "App",
            "Reset to the default keys".into(),
            "escape hatch broken restore",
        ),
        (
            Action::App(AppCommand::ZoomIn),
            "App",
            "Zoom in".into(),
            "bigger font size",
        ),
        (
            Action::App(AppCommand::ZoomOut),
            "App",
            "Zoom out".into(),
            "smaller font size",
        ),
        (
            Action::App(AppCommand::ZoomReset),
            "App",
            "Reset zoom".into(),
            "font size",
        ),
        (
            Action::App(AppCommand::Quit),
            "App",
            "Quit herdr-frontend".into(),
            "exit",
        ),
    ]);
    list
}

/// The palette's rows. Empty: up to five recent commands first, then every group in order.
/// Typed: every entry the query matches (Eli's matching over the title first, then the group,
/// command name and keywords), best first, recent ones first among equals.
pub fn palette_rows(m: &Model, query: &str) -> Vec<PaletteRow> {
    let keys = crate::help::rows(m.keymap());
    let key = |action: &Action| {
        let name = action.name()?;
        keys.iter()
            .find(|r| r.name == name && r.section.is_none())
            .and_then(|r| r.keys.first().cloned())
    };
    let recent = &m.view().palette;
    let rank_of = |action: &Action| {
        action
            .name()
            .and_then(|n| recent.iter().position(|r| *r == n))
    };
    let row = |(action, group, title, _): &(Action, &'static str, String, &str), head| PaletteRow {
        key: key(action),
        action: action.clone(),
        group,
        title: title.clone(),
        head,
    };
    let all = entries();
    if query.trim().is_empty() {
        let mut recent_rows: Vec<(usize, &_)> = all
            .iter()
            .filter_map(|e| rank_of(&e.0).map(|r| (r, e)))
            .collect();
        recent_rows.sort_by_key(|(r, _)| *r);
        recent_rows.truncate(PALETTE_RECENT_SHOWN);
        let shown: Vec<&Action> = recent_rows.iter().map(|(_, e)| &e.0).collect();
        let mut out: Vec<PaletteRow> = recent_rows.iter().map(|(_, e)| row(e, "Recent")).collect();
        for group in &PALETTE_GROUPS[1..] {
            out.extend(
                all.iter()
                    .filter(|e| e.1 == *group && !shown.contains(&&e.0))
                    .map(|e| row(e, e.1)),
            );
        }
        return out;
    }
    // Recent ones first among equals: rank keeps the order it is given for ties.
    let mut ordered: Vec<&_> = all.iter().collect();
    ordered.sort_by_key(|e| rank_of(&e.0).unwrap_or(usize::MAX));
    filter::rank(query, &ordered, |e| {
        let name = e.0.name().unwrap_or_default();
        (e.2.clone(), vec![e.1.to_owned(), name, e.3.to_owned()])
    })
    .into_iter()
    .map(|i| row(ordered[i], ordered[i].1))
    .collect()
}

// ---- Menus (`cfg/mods/menu`) -------------------------------------------------------------------

/// The terminal menu (a right-click on a pane): Rename, Split right, Split down, Zoom; where its
/// right-clicks go; "Send back to <parent>" for a sub-agent kept in a pane; Close pane. Pane
/// items are left out for a terminal outside a tab (`menu/main.js` `items`, `"pane"`).
pub fn terminal_menu(m: &Model, pane: &str) -> Option<Menu> {
    let world = m.world();
    let in_tab = world
        .tab_of(pane)
        .is_some_and(|t| t.root.find(pane).is_some());
    let record = world.address_of(pane).and_then(|a| world.terminal(a));
    if !in_tab && record.is_none() {
        return None;
    }
    let items = terminal_items(m, in_tab.then_some(pane), record);
    // Herdr app's terminal menu default (`terminal_menu_spec`).
    Some(Menu::new(MenuTarget::Pane(pane.to_owned()), items, 220.0))
}

/// The terminal menu over the peek (a right-click on its terminal): the peeked terminal's
/// items, without the pane items, as for any terminal outside a tab.
pub fn peek_menu(m: &Model) -> Option<Menu> {
    let record = m.world().terminal(m.peeked()?)?;
    let items = terminal_items(m, None, Some(record));
    Some(Menu::new(
        MenuTarget::Terminal(record.address.clone()),
        items,
        220.0,
    ))
}

/// The terminal menu's items: the pane items for `pane`, a pane in a tab, around the items for
/// its terminal `record`.
fn terminal_items(m: &Model, pane: Option<&str>, record: Option<&Terminal>) -> Vec<MenuItem> {
    let world = m.world();
    let on = |pane: &str, action: Action| vec![Action::FocusPane(pane.into()), action];
    let mut items = Vec::new();
    if let Some(pane) = pane {
        items.extend([
            MenuItem::new(
                "Rename",
                MenuIcon::Rename,
                vec![Action::RenamePaneOf(pane.into())],
            ),
            MenuItem::new(
                "Split right",
                MenuIcon::SplitRight,
                on(pane, Action::PaneSplit(Split::Right)),
            )
            .hint(Action::PaneSplit(Split::Right)),
            MenuItem::new(
                "Split down",
                MenuIcon::SplitDown,
                on(pane, Action::PaneSplit(Split::Down)),
            )
            .hint(Action::PaneSplit(Split::Down)),
            MenuItem::new("Zoom", MenuIcon::Zoom, on(pane, Action::PaneZoom))
                .hint(Action::PaneZoom),
        ]);
    }
    if let Some(record) = record {
        let to_program = labels::right_click_to_program(&record.labels);
        let label = if to_program {
            "Use Herdr right-click menu"
        } else {
            "Send right-clicks to pane"
        };
        items.push(MenuItem::new(
            label,
            MenuIcon::Terminal,
            vec![Action::RightClicks {
                terminal: record.address.clone(),
                to_program: !to_program,
            }],
        ));
        let parent = record.parent.as_deref().and_then(|p| world.terminal(p));
        if let Some(parent) = parent {
            let name = parent
                .name
                .clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| parent.address.clone());
            items.push(MenuItem::new(
                format!("Send back to {name}"),
                MenuIcon::Terminal,
                vec![Action::Park(record.address.clone())],
            ));
        }
    }
    if let Some(pane) = pane {
        items.push(
            MenuItem::new("Close pane", MenuIcon::Close, on(pane, Action::PaneClose))
                .hint(Action::PaneClose),
        );
    }
    items
}
