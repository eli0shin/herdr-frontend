//! The terminal renderer's fixture screens drawn headless through Rune's real runtime: the grid
//! sits at the terminal's own size at the pane's top left, every row reads as the terminal's text,
//! the cursor lands on its cell, and unchanged rows aren't built again.

use herdr_frontend::fixtures::{self, BOXES, COLORS, NVIM_SPLITS};
use herdr_frontend::{State, app, spec};
use rune::{ElementInfo, Env, Runtime};

/// JetBrains Mono 14 pt at 2x: a 17 × 37 px cell, 8.5 × 18.5 pt.
const CELL: (f32, f32) = (8.5, 18.5);

fn runtime(scene: &str, size: (f32, f32)) -> Runtime<State> {
    let mut rt = app().headless_runtime(size, Env::new(true, 2.0));
    let (_, apply) = spec()
        .scenes
        .iter()
        .find(|(name, _)| *name == scene)
        .copied()
        .unwrap_or_else(|| panic!("no scene {scene}"));
    apply(&mut rt.state);
    rt.frame(0.0);
    rt
}

fn element(rt: &Runtime<State>, path_end: &str) -> ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.path.ends_with(path_end))
        .unwrap_or_else(|| panic!("no element at …{path_end}"))
}

/// The lines a screen reader reads off the grids, in order.
fn read_lines(rt: &Runtime<State>) -> Vec<String> {
    rt.accessibility_tree()
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == rune::accesskit::Role::TextRun)
        .filter_map(|(_, node)| node.value().map(|v| v.trim_end().to_owned()))
        .collect()
}

#[test]
fn each_fixture_draws_at_its_real_grid_pinned_top_left() {
    for (scene, fixture) in [
        ("terminal-colors", &COLORS),
        ("terminal-boxes", &BOXES),
        ("terminal-nvim", &NVIM_SPLITS),
    ] {
        // A pane bigger than the grid: the grid keeps the terminal's size, at the top left.
        let rt = runtime(scene, (900.0, 700.0));
        let screen = element(&rt, "pane-1/#0/screen");
        assert_eq!(screen.role, "terminal", "{scene}");
        assert_eq!(
            screen.rect,
            [
                0.0,
                0.0,
                f32::from(fixtures::COLS) * CELL.0,
                f32::from(fixtures::ROWS) * CELL.1
            ],
            "{scene}: the grid is the terminal's, not the pane's"
        );
        let pane = element(&rt, "pane-1");
        assert_eq!(pane.rect, [0.0, 0.0, 900.0, 700.0], "{scene}");

        // Every row reads as the terminal's text, except what the program made invisible.
        let (theme, _) = herdr_frontend::view::terminal::look::arctic();
        let mut snapshot = fixture.snapshot(&theme);
        for row in &mut snapshot.screen {
            let row = std::sync::Arc::make_mut(row);
            for cell in &mut row.cells {
                if cell.flags.contains(terminal::CellFlags::INVISIBLE) {
                    cell.text = " ".into();
                }
            }
        }
        let expected: Vec<String> = (0..snapshot.screen.len())
            .map(|y| snapshot.line(y))
            .collect();
        assert_eq!(read_lines(&rt), expected, "{scene}");
    }
}

#[test]
fn neovim_splits_read_as_neovim_drew_them() {
    let rt = runtime("terminal-nvim", (680.0, 555.0));
    let lines = read_lines(&rt);
    assert!(
        lines[0].starts_with("  1 line 1 of the pager fixture"),
        "{:?}",
        lines[0]
    );
    assert!(lines.iter().any(|l| l.contains("meters.py")), "{lines:#?}");
    assert!(
        lines.iter().any(|l| l.starts_with("pager.txt")),
        "{lines:#?}"
    );
    assert!(
        lines[29].starts_with("-- VISUAL --") && lines[29].ends_with('3'),
        "{:?}",
        lines[29]
    );
}

#[test]
fn the_cursor_sits_on_its_cell() {
    // The colors screen parks a steady block on "$ " at row 29, column 2.
    let rt = runtime("terminal-colors", (680.0, 555.0));
    let cursor = element(&rt, "pane-1/#0/cursor");
    assert_eq!(cursor.rect, [2.0 * CELL.0, 28.0 * CELL.1, CELL.0, CELL.1]);
}

#[test]
fn an_unfocused_pane_still_shows_its_cursor() {
    let rt = runtime("terminal-selection", (680.0, 555.0));
    assert!(!rt.state.panes[0].focused);
    element(&rt, "pane-1/#0/cursor");
}

#[test]
fn four_panes_each_draw_their_terminal() {
    let rt = runtime("terminal-panes", (1200.0, 800.0));
    for i in 1..=4 {
        let screen = element(&rt, &format!("pane-{i}/#0/screen"));
        assert_eq!(screen.role, "terminal");
    }
    let lines = read_lines(&rt);
    assert_eq!(lines.len(), 4 * usize::from(fixtures::ROWS));
}

#[test]
fn unchanged_rows_are_not_built_again() {
    let mut rt = runtime("terminal-colors", (680.0, 555.0));
    assert_eq!(rt.state.panes[0].rows_built(), usize::from(fixtures::ROWS));

    // The same snapshot again: every row comes from the cache.
    rt.invalidate();
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.panes[0].rows_built(), 0);

    // One more line of output changes the rows it touches, not the screen.
    let (theme, _) = herdr_frontend::view::terminal::look::arctic();
    let mut vt = terminal::Vt::new(&terminal::Config {
        cols: fixtures::COLS,
        rows: fixtures::ROWS,
        theme,
        ..terminal::Config::default()
    });
    vt.write(COLORS.bytes);
    let first = vt.snapshot();
    rt.state.panes[0].show(first);
    rt.invalidate();
    rt.frame(1.0 / 60.0);
    vt.write(b"echo hi");
    let second = vt.snapshot();
    rt.state.panes[0].show(second);
    rt.invalidate();
    rt.frame(1.0 / 60.0);
    assert_eq!(
        rt.state.panes[0].rows_built(),
        1,
        "only the prompt row changed"
    );
}
