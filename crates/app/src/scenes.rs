//! Named states for `--snap --scene NAME` (`--list-scenes` lists them).

use crate::State;
use crate::config::settings::Settings;
use crate::fixtures::{self, BOXES, COLORS, Fixture, IMAGES, NVIM_CHECKLIST, NVIM_SPLITS};
use crate::link::{Fake, Offline};
use crate::state::Status;
use crate::view::terminal::TerminalPane;

/// A scene: its name and how it changes the first state.
type Scene = (&'static str, fn(&mut State));

/// The display scale the terminal scenes size their terminals' cells for: the pixel diff's.
const SNAP_SCALE: f32 = 2.0;

pub(crate) const SCENES: &[Scene] = &[
    ("hello", |_| {}),
    ("terminal-colors", |s| one(s, &COLORS)),
    ("terminal-boxes", |s| one(s, &BOXES)),
    ("terminal-nvim", |s| one(s, &NVIM_SPLITS)),
    ("terminal-nvim-checklist", |s| one(s, &NVIM_CHECKLIST)),
    ("terminal-images", |s| one(s, &IMAGES)),
    ("terminal-selection", selection),
    ("terminal-panes", panes),
    ("terminal-scrolling", |s| scrolling(s, 0.0)),
    ("terminal-scrolling-ahead", |s| scrolling(s, -2.5)),
    ("screen-file", screen_file),
    ("shell", shell),
    ("shell-one-pane", shell_one_pane),
    ("shell-no-spaces", shell_no_spaces),
    ("shell-connecting", shell_connecting),
    ("shell-core-down", shell_core_down),
    ("shell-keys-help", shell_keys_help),
    ("shell-not-yet", shell_not_yet),
    ("shell-tabs", shell_tabs),
    ("shell-spaces", busy),
    ("shell-agents", shell_agents),
    ("shell-agents-grouped", shell_agents_grouped),
    ("shell-sort-menu", shell_sort_menu),
    ("shell-agent-jump", shell_agent_jump),
    ("shell-peek", shell_peek),
    ("shell-peek-find", shell_peek_find),
    ("shell-peek-menu", shell_peek_menu),
    ("shell-agents-empty", shell_agents_empty),
    ("shell-alerts", shell_alerts),
    ("shell-palette", shell_palette),
    ("shell-palette-typed", shell_palette_typed),
    ("shell-picker", shell_picker),
    ("shell-picker-earlier", shell_picker_earlier),
    ("shell-space-picker", shell_space_picker),
    ("shell-space-picker-typed", shell_space_picker_typed),
    ("shell-new-space", shell_new_space),
    ("shell-new-space-typed", shell_new_space_typed),
    ("shell-pr-finder", shell_pr_finder),
    ("shell-pr-finder-typed", shell_pr_finder_typed),
    ("shell-prompt", shell_prompt),
    ("shell-confirm", shell_confirm),
    ("shell-tab-menu", shell_tab_menu),
    ("shell-terminal-menu", shell_terminal_menu),
    ("shell-settings", shell_settings),
    ("shell-machine-filter", shell_machine_filter),
    ("shell-machines", shell_machines),
    ("shell-machine-add", shell_machine_add),
    ("shell-find", shell_find),
    ("shell-copy-mode", shell_copy_mode),
];

/// The app on the demo core ([`fixtures::demo_core`]): the sidebar, the tab strip with `work`
/// selected, and its three live panes, Neovim focused.
fn shell(s: &mut State) {
    s.connect(Box::new(fixtures::demo_core()));
    s.settle();
}

/// One tab, one pane: the colors screen.
fn shell_one_pane(s: &mut State) {
    let doc = serde_json::json!({"spaces": [{
        "id": "sp-1", "name": "solo", "folder": "/tmp",
        "tabs": [fixtures::tab("tb-1", "colors", fixtures::split("tb-1", "row", vec![fixtures::pane("pn-1", "t1")]))],
    }]});
    let fake = Fake::new(
        doc,
        vec![fixtures::terminal("t1", fixtures::COLS, fixtures::ROWS)],
    );
    fake.core()
        .screens
        .insert("t1".into(), COLORS.bytes.to_vec());
    s.connect(Box::new(fake));
    s.settle();
}

/// Connected to a core whose layout has no spaces.
fn shell_no_spaces(s: &mut State) {
    s.connect(Box::new(Fake::new(
        serde_json::json!({"spaces": []}),
        Vec::new(),
    )));
    s.settle();
}

/// Waiting for the core.
fn shell_connecting(s: &mut State) {
    s.connect(Box::new(Offline));
}

/// The core can't be reached; the supervisor keeps trying.
fn shell_core_down(s: &mut State) {
    s.connect(Box::new(Offline));
    s.status = Status::Down("hc connect: no core is running".into());
}

/// The demo core with the keys help open (`cmd-/`), filtered to the pane keys.
fn shell_keys_help(s: &mut State) {
    shell(s);
    s.step(0.0, model::Action::Open(model::Popup::Help));
    s.popup_field.text = "pane".into();
    s.step(
        0.0,
        model::Action::Popup(model::PopupAction::Query("pane".into())),
    );
}

/// The demo core after a key whose feature isn't built: its "not yet" toast.
fn shell_not_yet(s: &mut State) {
    shell(s);
    s.step(0.0, model::Action::App(model::AppCommand::Hide));
}

/// The tab strip's pills ([`fixtures::tab_strip_core`]): `work` selected with Neovim zoomed, a
/// Pi session's name, the agent's tab left out, a shell numbered 3 and a long name cut short.
fn shell_tabs(s: &mut State) {
    s.connect(Box::new(fixtures::tab_strip_core()));
    s.settle();
    s.step(0.0, model::Action::PaneZoom);
    s.settle();
}

/// The agents core ([`fixtures::agents_core`]) in the priority view, a Pi worker focused, so its
/// orchestration is open; each agent's last change a while ago.
fn shell_agents(s: &mut State) {
    let now = crate::state::wall_ms();
    let minutes = |m: u64| now - m * 60_000;
    for (address, state, at) in [
        ("t10", "working", minutes(12)),
        ("t11", "blocked", minutes(3)),
        ("t12", "idle", minutes(120)),
        ("t13", "working", minutes(25)),
        ("t14", "idle", minutes(40)),
        ("t15", "idle", minutes(3 * 24 * 60)),
    ] {
        s.agents.changed.insert(
            address.into(),
            model::agents::Changed {
                s: state.into(),
                at,
            },
        );
    }
    s.connect(Box::new(fixtures::agents_core()));
    s.settle();
    s.step(0.0, model::Action::FocusAgent("t13".into()));
    s.settle();
}

/// [`shell_agents`] grouped by space.
fn shell_agents_grouped(s: &mut State) {
    s.set_sort(model::agents::Sort::Grouped);
    shell_agents(s);
}

/// [`shell_agents`] with the sort menu open.
fn shell_sort_menu(s: &mut State) {
    shell_agents(s);
    s.step(0.0, model::Action::Open(model::Popup::SortMenu));
    s.settle();
}

/// The agents core while you work in `work`: the Pi orchestrator needs you, then the sidebar's
/// Claude finishes. Their toasts stack at the bottom right, the older one lowest.
fn shell_alerts(s: &mut State) {
    let fake = fixtures::agents_core();
    s.connect(Box::new(fake.clone()));
    s.settle();
    for (address, state, turn) in [
        ("t12", herdr_proto::StatusState::Blocked, 0),
        ("t10", herdr_proto::StatusState::Idle, 1),
    ] {
        let mut t = fake.core().terminals[address].clone();
        if let Some(status) = t.status.as_mut() {
            status.state = state;
        }
        t.turn = turn;
        fake.update(t);
        s.settle();
    }
}

/// [`shell_agents`] with the agent-jump bar open (`ctrl-;`): the rows' labels bright.
fn shell_agent_jump(s: &mut State) {
    shell_agents(s);
    s.step(0.0, model::Action::Open(model::Popup::AgentJump));
    s.settle();
}

/// [`shell_agents`] peeking the `explore` sub-agent, which has no pane.
fn shell_peek(s: &mut State) {
    shell_agents(s);
    s.step(0.0, model::Action::Peek("t20".into()));
    s.settle();
}

/// [`shell_peek`] with the peeked terminal's find bar open (`cmd-f`) on `sidebar`, the
/// newest hit selected.
fn shell_peek_find(s: &mut State) {
    shell_peek(s);
    s.step(0.0, model::Action::Terminal(model::TerminalAction::Search));
    s.search("t20", "sidebar");
    if let Some(viewer) = s.viewers.get("t20") {
        viewer.terminal.find_next(false);
    }
    s.settle();
}

/// [`shell_peek`] with the terminal menu opened by a right-click in it: no pane items.
fn shell_peek_menu(s: &mut State) {
    shell_peek(s);
    s.menu_at = Some((560.0, 300.0));
    s.step(0.0, model::Action::PeekMenu);
}

/// The demo core, whose terminals report no status: the list says how agents show up.
fn shell_agents_empty(s: &mut State) {
    shell(s);
}

/// The busy core ([`fixtures::busy_core`]): agents in other spaces, for the pickers and for
/// switching spaces (`shell-spaces`).
fn busy(s: &mut State) {
    s.connect(Box::new(fixtures::busy_core()));
    s.settle();
}

/// Types `text` into the open popup's field, as the field reports it.
fn type_in(s: &mut State, text: &str) {
    s.popup_field = rune::TextEdit::new(text);
    s.step(
        0.0,
        model::Action::Popup(model::PopupAction::Query(text.into())),
    );
}

/// The palette, empty: recent commands (zoom pane) first, then every group.
fn shell_palette(s: &mut State) {
    shell(s);
    s.step(
        0.0,
        model::Action::FromPalette(Box::new(model::Action::PaneZoom)),
    );
    s.step(
        0.0,
        model::Action::FromPalette(Box::new(model::Action::PaneZoom)),
    );
    s.step(0.0, model::Action::Open(model::Popup::Palette));
}

/// The palette with "split" typed.
fn shell_palette_typed(s: &mut State) {
    shell(s);
    s.step(0.0, model::Action::Open(model::Popup::Palette));
    type_in(s, "split");
}

/// The picker on the busy core: the space that needs you first, the quiet one folded.
fn shell_picker(s: &mut State) {
    busy(s);
    s.step(0.0, model::Action::Open(model::Popup::Picker));
}

/// The picker with its quiet spaces unfolded.
fn shell_picker_earlier(s: &mut State) {
    busy(s);
    s.step(0.0, model::Action::Open(model::Popup::Picker));
    // The "earlier" row: past the busy spaces' five tabs.
    s.step(0.0, model::Event::PopupCursor(5));
    s.step(0.0, model::Action::Popup(model::PopupAction::Choose));
}

/// The space picker on the busy core, on the shown space.
fn shell_space_picker(s: &mut State) {
    busy(s);
    s.step(0.0, model::Action::Open(model::Popup::SpacePicker));
}

/// The space picker with "re" typed: names first.
fn shell_space_picker_typed(s: &mut State) {
    busy(s);
    s.step(0.0, model::Action::Open(model::Popup::SpacePicker));
    type_in(s, "re");
}

/// The new-space folder picker on the demo core: where you are, then zoxide's folders, then
/// the repos config's.
fn shell_new_space(s: &mut State) {
    s.connect(Box::new(fixtures::folders_core()));
    s.settle();
    s.step(0.0, model::Action::Open(model::Popup::NewSpace));
    s.settle();
}

/// The folder picker with a path typed: it leads, tagged "path".
fn shell_new_space_typed(s: &mut State) {
    shell_new_space(s);
    type_in(s, "/code/new");
}

/// The PR finder on the agents core: the orchestrator and its working worker watch PRs.
fn shell_pr_finder(s: &mut State) {
    s.connect(Box::new(fixtures::agents_core()));
    s.settle();
    s.step(0.0, model::Action::Open(model::Popup::PrFinder));
    s.settle();
}

/// The PR finder with "22" typed: only the worker watching #22.
fn shell_pr_finder_typed(s: &mut State) {
    shell_pr_finder(s);
    type_in(s, "22");
}

/// Renaming the `work` tab: its name in the field, selected.
fn shell_prompt(s: &mut State) {
    shell(s);
    s.step(0.0, model::Action::TabRename);
}

/// Closing the space asks first.
fn shell_confirm(s: &mut State) {
    shell(s);
    s.step(0.0, model::Action::SpaceClose);
}

/// The tab menu, opened by a right-click on the tab strip.
fn shell_tab_menu(s: &mut State) {
    shell(s);
    s.menu_at = Some((420.0, 30.0));
    s.step(0.0, model::Action::TabMenu("tb-work".into()));
}

/// The terminal menu over the Neovim pane.
fn shell_terminal_menu(s: &mut State) {
    shell(s);
    s.menu_at = Some((560.0, 220.0));
    s.step(0.0, model::Action::TerminalMenu("pn-1".into()));
}

/// The settings dialog.
fn shell_settings(s: &mut State) {
    shell(s);
    s.step(0.0, model::Action::Open(model::Popup::Settings));
}

/// The agents core with other machines ([`fixtures::machines_core`]).
fn machines(s: &mut State) {
    s.connect(Box::new(fixtures::machines_core()));
    s.settle();
}

/// The machine filter, devbox hidden, the cursor on it.
fn shell_machine_filter(s: &mut State) {
    machines(s);
    s.set_hidden_machines(["devbox".to_owned()].into());
    s.step(0.0, model::Action::Open(model::Popup::AgentMachines));
    s.step(0.0, model::Action::Popup(model::PopupAction::Down));
    s.step(0.0, model::Action::Popup(model::PopupAction::Down));
}

/// The Machines dialog: devbox up, omarchy needing a login and lingering.
fn shell_machines(s: &mut State) {
    machines(s);
    s.step(0.0, model::Action::Open(model::Popup::Machines));
}

/// Adding a machine: the ssh config's hosts (devbox, added already, left out), one that failed
/// asking for a password.
fn shell_machine_add(s: &mut State) {
    machines(s);
    let dir = std::env::temp_dir().join(format!("herdr-frontend-scene-ssh-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let config = dir.join("config");
    let _ = std::fs::write(
        &config,
        "Host devbox
  HostName 10.0.0.7
Host box
  User eli
  HostName box.local
Host vps-eu vps
  User can
",
    );
    s.machines.ssh_config = Some(config);
    s.step(0.0, model::Action::Open(model::Popup::MachineAdd));
    s.machines.failed.insert(
        "box".into(),
        model::machines::add_failure(
            "box",
            &herdr_proto::RpcError {
                code: -32000,
                message: "Permission denied (publickey,password)".into(),
                kind: herdr_proto::ErrorKind::Unreachable,
                hint: None,
                data: Default::default(),
            },
        ),
    );
}

/// The demo core with the colors pane's find bar open (`cmd-f`) on `bg`, its hits marked and
/// the newest one selected.
fn shell_find(s: &mut State) {
    shell(s);
    s.step(0.0, model::Action::FocusPane("pn-2".into()));
    s.settle();
    s.step(0.0, model::Action::Terminal(model::TerminalAction::Search));
    s.search("t2", "bg");
    if let Some(viewer) = s.viewers.get("t2") {
        viewer.terminal.find_next(false);
    }
    s.settle();
}

/// Copy mode in the colors pane (`ctrl-g [`): three rows selected as lines from the view's
/// top, the bar at the window's top saying VISUAL LINE and the cursor's row.
fn shell_copy_mode(s: &mut State) {
    shell(s);
    s.step(0.0, model::Action::FocusPane("pn-2".into()));
    s.settle();
    s.step(0.0, model::Action::CopyMode);
    for key in ["shift-h", "j", "w", "shift-v", "j", "j"] {
        let chord = model::Chord::parse(key).expect("a key");
        s.step(0.0, model::Event::key(chord, false));
    }
    s.settle();
}

/// `fixture` alone in a focused pane: what the Ghostty pixel diff compares, at Ghostty's
/// defaults. Its terminal has the cell of a 2x snap, as the pixel diff's; at another scale the
/// pane scales its images to the cell it draws.
fn one(s: &mut State, fixture: &Fixture) {
    s.with_settings(Settings::fixture());
    let face = s.face(SNAP_SCALE);
    let mut pane = TerminalPane::new("pane-1", fixture.snapshot_in(&s.theme, &face));
    pane.focused = true;
    s.panes = vec![pane];
}

/// The colors screen with three rows selected, in an unfocused pane (a hollow cursor).
fn selection(s: &mut State) {
    s.with_settings(Settings::fixture());
    let mut snapshot = COLORS.snapshot(&s.theme);
    fixtures::select(&mut snapshot, 15..=17, 8, 40);
    s.panes = vec![TerminalPane::new("pane-1", snapshot)];
}

/// A focused pane scrolled smoothly 40.4 rows up, so the screen sits 0.4 of a row low with part
/// of the row above in view; `ahead` more rows the worker hasn't caught up with yet (negative is
/// down, so the rows below the screen show).
fn scrolling(s: &mut State, ahead: f32) {
    s.with_settings(Settings::fixture());
    let face = s.face(SNAP_SCALE);
    let snapshot = fixtures::scrolled_in(fixtures::COLS, fixtures::ROWS, &s.theme, &face, 40.4);
    let mut pane = TerminalPane::new("pane-1", snapshot);
    pane.focused = true;
    pane.ahead = ahead;
    s.panes = vec![pane];
}

/// Four panes, 2 × 2, the Neovim one focused.
fn panes(s: &mut State) {
    s.with_settings(Settings::fixture());
    let mut selected = COLORS.snapshot(&s.theme);
    fixtures::select(&mut selected, 15..=17, 8, 40);
    let snapshots = [
        NVIM_SPLITS.snapshot(&s.theme),
        BOXES.snapshot(&s.theme),
        COLORS.snapshot(&s.theme),
        selected,
    ];
    s.panes = snapshots
        .into_iter()
        .enumerate()
        .map(|(i, snapshot)| {
            let mut pane = TerminalPane::new(format!("pane-{}", i + 1), snapshot);
            pane.focused = i == 0;
            pane
        })
        .collect();
}

/// A screen file in a focused pane, at the grid `HERDR_FRONTEND_GRID` names
/// ([`fixtures::screen_file`]); an empty 80 × 30 screen without one. It draws with the settings
/// `HERDR_FRONTEND_SETTINGS` names, its fonts looked up as the window's are
/// ([`fixtures::screen_settings`]), else Ghostty's defaults.
fn screen_file(s: &mut State) {
    match fixtures::screen_settings() {
        Ok(Some(settings)) => s.configure(crate::config::Config::with_fonts(settings)),
        Ok(None) => s.with_settings(Settings::fixture()),
        Err(e) => {
            eprintln!("screen-file: {e}");
            s.with_settings(Settings::fixture());
        }
    }
    let (bytes, cols, rows) = fixtures::screen_file().unwrap_or_default();
    let (cols, rows) = if cols == 0 {
        (fixtures::COLS, fixtures::ROWS)
    } else {
        (cols, rows)
    };
    let face = s.face(SNAP_SCALE);
    let snapshot = fixtures::replay_in(&bytes, cols, rows, &s.theme, &face);
    let mut pane = TerminalPane::new("pane-1", snapshot);
    pane.focused = true;
    s.panes = vec![pane];
}
