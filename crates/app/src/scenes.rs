//! Named states for `--snap --scene NAME` (`--list-scenes` lists them).

use crate::State;
use crate::config::settings::Settings;
use crate::fixtures::{self, BOXES, COLORS, Fixture, NVIM_CHECKLIST, NVIM_SPLITS};
use crate::link::{Fake, Offline};
use crate::state::Status;
use crate::view::terminal::TerminalPane;

/// A scene: its name and how it changes the first state.
type Scene = (&'static str, fn(&mut State));

pub(crate) const SCENES: &[Scene] = &[
    ("hello", |_| {}),
    ("terminal-colors", |s| one(s, &COLORS)),
    ("terminal-boxes", |s| one(s, &BOXES)),
    ("terminal-nvim", |s| one(s, &NVIM_SPLITS)),
    ("terminal-nvim-checklist", |s| one(s, &NVIM_CHECKLIST)),
    ("terminal-selection", selection),
    ("terminal-panes", panes),
    ("screen-file", screen_file),
    ("shell", shell),
    ("shell-one-pane", shell_one_pane),
    ("shell-no-spaces", shell_no_spaces),
    ("shell-connecting", shell_connecting),
    ("shell-core-down", shell_core_down),
    ("shell-keys-help", shell_keys_help),
    ("shell-prefix", shell_prefix),
    ("shell-not-yet", shell_not_yet),
    ("shell-tabs", shell_tabs),
    ("shell-agents", shell_agents),
    ("shell-agents-grouped", shell_agents_grouped),
    ("shell-sort-menu", shell_sort_menu),
    ("shell-agent-jump", shell_agent_jump),
    ("shell-agents-empty", shell_agents_empty),
    ("shell-alerts", shell_alerts),
    ("shell-palette", shell_palette),
    ("shell-palette-typed", shell_palette_typed),
    ("shell-picker", shell_picker),
    ("shell-picker-earlier", shell_picker_earlier),
    ("shell-space-picker", shell_space_picker),
    ("shell-space-picker-typed", shell_space_picker_typed),
    ("shell-prompt", shell_prompt),
    ("shell-confirm", shell_confirm),
    ("shell-tab-menu", shell_tab_menu),
    ("shell-terminal-menu", shell_terminal_menu),
    ("shell-settings", shell_settings),
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

/// The demo core part-way through a sequence: `ctrl-g` pressed, the indicator waiting.
fn shell_prefix(s: &mut State) {
    shell(s);
    s.step(0.0, model::Event::key(model::Chord::ctrl('g'), true));
}

/// The demo core after a key whose feature isn't built: its "not yet" toast.
fn shell_not_yet(s: &mut State) {
    shell(s);
    s.step(0.0, model::Action::Review);
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

/// The demo core, whose terminals report no status: the list says how agents show up.
fn shell_agents_empty(s: &mut State) {
    shell(s);
}

/// The busy core ([`fixtures::busy_core`]): agents in other spaces, for the pickers.
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

/// `fixture` alone in a focused pane: what the Ghostty pixel diff compares, at Ghostty's
/// defaults.
fn one(s: &mut State, fixture: &Fixture) {
    s.with_settings(Settings::fixture());
    let mut pane = TerminalPane::new("pane-1", fixture.snapshot(&s.theme));
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
    let mut pane = TerminalPane::new("pane-1", fixtures::replay(&bytes, cols, rows, &s.theme));
    pane.focused = true;
    s.panes = vec![pane];
}
