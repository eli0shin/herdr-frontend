//! The terminal renderer's fixture screens drawn headless through Rune's real runtime: the grid
//! sits at the terminal's own size at the pane's top left, every row reads as the terminal's text,
//! the cursor lands on its cell, and unchanged rows aren't built again.

use herdr_frontend::fixtures::{self, BOXES, COLORS, NVIM_CHECKLIST, NVIM_SPLITS};
use herdr_frontend::{State, app, spec};
use rune::{ElementInfo, Env, Runtime};

/// JetBrains Mono 14 pt at 2x: a 17 × 37 px cell, 8.5 × 18.5 pt, at macOS's 72 DPI, and a
/// 22 × 49 px one, 11 × 24.5 pt, at the 96 DPI Ghostty gives other platforms.
const CELL: (f32, f32) = if cfg!(target_os = "macos") {
    (8.5, 18.5)
} else {
    (11.0, 24.5)
};

/// The fixtures' grid, in points: 680 × 555 on macOS.
const GRID: (f32, f32) = (
    fixtures::COLS as f32 * CELL.0,
    fixtures::ROWS as f32 * CELL.1,
);

/// A pane bigger than the grid: 900 × 700 on macOS.
const BIG: (f32, f32) = (GRID.0 + 220.0, GRID.1 + 145.0);

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
        ("terminal-nvim-checklist", &NVIM_CHECKLIST),
    ] {
        // A pane bigger than the grid: the grid keeps the terminal's size, at the top left.
        let rt = runtime(scene, BIG);
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
        assert_eq!(pane.rect, [0.0, 0.0, BIG.0, BIG.1], "{scene}");

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
    let rt = runtime("terminal-nvim", GRID);
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
fn the_neovim_checklist_draws_as_ghostty_does() {
    use herdr_frontend::view::terminal::{font::Font, look, rows};

    let (theme, look) = look::arctic();
    let snapshot = NVIM_CHECKLIST.snapshot(&theme);
    let face = Font::jetbrains_mono(herdr_frontend::TERMINAL_POINTS)
        .at(2.0)
        .unwrap();
    let row_with = |needle: &str| {
        let y = (0..snapshot.screen.len())
            .find(|&y| snapshot.line(y).contains(needle))
            .unwrap_or_else(|| panic!("no row with {needle:?}"));
        (
            y,
            rows::build(&snapshot.screen[y], &snapshot.colors, &face, &look),
        )
    };

    // The float's rounded corners are Ghostty's antialiased arcs.
    let (_, border) = row_with("╭");
    assert!(border.boxes.iter().any(|(_, c)| c.a < 1.0 && c.a > 0.0));

    // The warning's undercurl is a curl, in its own color. (The error's is hidden by the
    // cmdline's live preview of the substitution.)
    let (_, warning) = row_with("local unused");
    let yellow = look::color(terminal::Rgb::new(0xfc, 0xe0, 0x94));
    let curl: Vec<_> = warning
        .boxes
        .iter()
        .filter(|(_, c)| (c.r - yellow.r).abs() < 1e-4 && (c.g - yellow.g).abs() < 1e-4)
        .collect();
    assert!(curl.len() > 3, "{:?}", warning.boxes);
    assert!(curl.iter().any(|(_, c)| c.a < 1.0), "antialiased");
    // Its lower half reaches into the next row, which fills its background: the view draws it
    // over the grid, as Ghostty draws every background first.
    assert!(
        warning
            .spill
            .iter()
            .any(|(_, c)| (c.r - yellow.r).abs() < 1e-4 && (c.g - yellow.g).abs() < 1e-4),
        "{:?}",
        warning.spill
    );

    // The statusline: Powerline separators are sprites, the icons come from the Symbols Nerd
    // Font through the atlas (on macOS).
    let (_, status) = row_with("main");
    assert!(!status.boxes.is_empty(), "Powerline separators");
    if cfg!(target_os = "macos") {
        assert!(status.atlas.len() >= 3, "{:?}", status.atlas);
        // The row still reads as Neovim's text.
        assert!(status.cells.iter().any(|c| c.text == "\u{f15c}"));
    }

    // The cursor is in the cmdline, after what's typed.
    let rt = runtime("terminal-nvim-checklist", GRID);
    assert_eq!(element(&rt, "pane-1/#0/spill").rect[..2], [0.0, 0.0]);
    let cursor = element(&rt, "pane-1/#0/cursor");
    assert_eq!(cursor.rect[1], 29.0 * CELL.1);
    assert_eq!(cursor.rect[0], 12.0 * CELL.0, "after \":%s/nam/name\"");
}

#[test]
fn the_cursor_sits_on_its_cell() {
    // The colors screen parks a steady block on "$ " at row 29, column 2.
    let rt = runtime("terminal-colors", GRID);
    let cursor = element(&rt, "pane-1/#0/cursor");
    assert_eq!(cursor.rect, [2.0 * CELL.0, 28.0 * CELL.1, CELL.0, CELL.1]);
}

#[test]
fn an_unfocused_pane_still_shows_its_cursor() {
    let rt = runtime("terminal-selection", GRID);
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
    let mut rt = runtime("terminal-colors", GRID);
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

/// The rects of the images in one of a pane's image layers.
fn images_in(rt: &Runtime<State>, layer: &str) -> Vec<[f32; 4]> {
    let prefix = format!("pane-1/#0/{layer}/");
    rt.elements()
        .into_iter()
        .filter(|e| e.path.contains(&prefix))
        .map(|e| e.rect)
        .collect()
}

/// `kitten icat`'s recording draws in Ghostty's order: images under the backgrounds, the
/// backgrounds, images under the text, the text, the cursor, then images over it. Each image
/// sits on its cells at its size, clipped to the grid, and placeholder cells read as blanks.
#[test]
fn kitty_images_draw_in_their_layers() {
    let rt = runtime("terminal-images", BIG);
    let paths: Vec<String> = rt.elements().into_iter().map(|e| e.path).collect();
    let at = |key: &str| {
        let path = format!("pane-1/#0/{key}");
        paths
            .iter()
            .position(|p| p.ends_with(&path))
            .unwrap_or_else(|| panic!("no {path}"))
    };
    let order = [
        at("images-below-background"),
        at("screen-backgrounds"),
        at("images-below-text"),
        at("screen"),
        at("images-above-text"),
    ];
    assert!(order.is_sorted(), "layers out of order: {order:?}");
    let grid = [
        0.0,
        0.0,
        f32::from(fixtures::COLS) * CELL.0,
        f32::from(fixtures::ROWS) * CELL.1,
    ];
    assert_eq!(element(&rt, "pane-1/#0/images-above-text").rect, grid);

    // photo.png at its own 136 x 74 px from column 0 of row 1, icat's 272 x 148 px copy from
    // column 10, glass.png and the dots on row 6, and the zlib RGBA image by hand on row 21.
    let above = images_in(&rt, "images-above-text");
    for rect in [
        [0.0, CELL.1, 68.0, 37.0],
        [10.0 * CELL.0, CELL.1, 136.0, 74.0],
        [12.0 * CELL.0, 6.0 * CELL.1, 34.0, 18.5],
        [46.0 * CELL.0, 6.0 * CELL.1, 17.0, 18.5],
        [7.0 * CELL.0, 21.0 * CELL.1, 16.0, 16.0],
    ] {
        assert!(above.contains(&rect), "no {rect:?} in {above:?}");
    }
    // The crop: from 5, 6 px into column 1 of row 21, to the end of its 3 x 1 cells.
    assert!(above.contains(&[
        CELL.0 + 2.5,
        21.0 * CELL.1 + 3.0,
        3.0 * CELL.0 - 2.5,
        CELL.1 - 3.0
    ]));
    assert_eq!(
        images_in(&rt, "images-below-background"),
        vec![[56.0 * CELL.0, CELL.1, 102.0, 55.5]]
    );
    // Under the text: icat's centred copy, and the placeholders one row of cells at a time. The
    // image fills its 16 x 4 placeholder cells exactly at the macOS cell; at another cell it's
    // centred in them, so there only the columns are exact.
    let below = images_in(&rt, "images-below-text");
    assert_eq!(below.len(), 7);
    assert!(below.contains(&[32.0 * CELL.0, CELL.1, 102.0, 55.5]));
    let placeholders: Vec<_> = below.iter().filter(|r| r[0] == 20.0 * CELL.0).collect();
    assert_eq!(placeholders.len(), 4, "{below:?}");
    for (row, rect) in (10..14).zip(placeholders) {
        let y = row as f32 * CELL.1;
        if cfg!(target_os = "macos") {
            assert_eq!(*rect, [20.0 * CELL.0, y, 16.0 * CELL.0, CELL.1]);
        } else {
            assert_eq!(rect[2], 16.0 * CELL.0, "{rect:?}");
            assert!((rect[1] - y).abs() <= 1.0, "{rect:?} on row {row}");
        }
    }

    let lines = read_lines(&rt);
    assert!(lines.iter().all(|l| !l.contains('\u{10EEEE}')));
    assert_eq!(lines[10], "", "a row of placeholders reads blank");
}

/// A pane draws its terminal's images in its own cell: when the terminal was last sized with
/// another cell (the frame after a zoom or a move to another display), each image still covers
/// the cells it was placed on. A pane lets go of an image's pixels once it's gone.
#[test]
fn images_scale_to_the_cell_the_pane_draws() {
    let mut rt = runtime("terminal-colors", GRID);
    let (theme, _) = herdr_frontend::view::terminal::look::arctic();
    // A 10 x 22 px cell, where a 20 x 44 px image covers 2 x 2 cells.
    let mut vt = terminal::Vt::new(&terminal::Config {
        cols: fixtures::COLS,
        rows: fixtures::ROWS,
        cell_width: 10,
        cell_height: 22,
        theme,
        ..terminal::Config::default()
    });
    let rgb = [255u8; 20 * 44 * 3];
    let payload = base64(&rgb);
    vt.write(format!("\x1b[3;5H\x1b_Ga=T,f=24,s=20,v=44,q=2;{payload}\x1b\\").as_bytes());
    rt.state.panes[0].show(vt.snapshot());
    rt.invalidate();
    rt.frame(1.0 / 60.0);
    assert_eq!(
        images_in(&rt, "images-above-text"),
        vec![[4.0 * CELL.0, 2.0 * CELL.1, 2.0 * CELL.0, 2.0 * CELL.1]]
    );
    assert_eq!(rt.state.panes[0].images_held(), 1);

    vt.write(b"\x1b_Ga=d,d=A,q=2\x1b\\");
    rt.state.panes[0].show(vt.snapshot());
    rt.invalidate();
    rt.frame(1.0 / 60.0);
    assert!(images_in(&rt, "images-above-text").is_empty());
    assert_eq!(rt.state.panes[0].images_held(), 0);
}

fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
