//! Named states for `--snap --scene NAME` (`--list-scenes` lists them).

use crate::State;
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
    s.help_filter.text = "pane".into();
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

/// `fixture` alone in a focused pane: what the Ghostty pixel diff compares.
fn one(s: &mut State, fixture: &Fixture) {
    let mut pane = TerminalPane::new("pane-1", fixture.snapshot(&s.theme));
    pane.focused = true;
    s.panes = vec![pane];
}

/// The colors screen with three rows selected, in an unfocused pane (a hollow cursor).
fn selection(s: &mut State) {
    let mut snapshot = COLORS.snapshot(&s.theme);
    fixtures::select(&mut snapshot, 15..=17, 8, 40);
    s.panes = vec![TerminalPane::new("pane-1", snapshot)];
}

/// Four panes, 2 × 2, the Neovim one focused.
fn panes(s: &mut State) {
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
/// ([`fixtures::screen_file`]); an empty 80 × 30 screen without one.
fn screen_file(s: &mut State) {
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
