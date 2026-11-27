//! Golden cell metrics: Ghostty's numbers for the fonts, sizes and scales herdr-frontend cares
//! about. Eli's parity font first (`docs/research/terminal-rendering.md` §2 and §8 stage 3).
//!
//! The cell sizes for Operator Mono and JetBrains Mono are worked by hand in the research doc. The
//! rest of each row (baseline, lines, box and cursor) follows from the font tables the same way;
//! `tests/coretext.rs` checks the inputs against CoreText, which is where Ghostty gets them.
//!
//! The numbers are Ghostty's on macOS, so every size here is built at its 72 DPI
//! ([`macos_size`]), and they hold on any platform.

mod common;

use common::FontFile;
use terminal::{CellSize, DesiredSize, FaceMetrics, Metrics, Modifier, Modifiers};

/// The whole-pixel metrics, in the order of [`Metrics`].
#[derive(Debug, PartialEq, Eq)]
struct Cell {
    size: (u32, u32),
    baseline: u32,
    underline: (u32, u32),
    strikethrough: (u32, u32),
    /// Box and overline thickness, both the underline thickness before modifiers.
    box_thickness: u32,
    cursor_height: u32,
}

fn cell(m: &Metrics) -> Cell {
    assert_eq!(m.overline_thickness, m.underline_thickness);
    assert_eq!((m.overline_position, m.cursor_thickness), (0, 1));
    Cell {
        size: (m.cell_width, m.cell_height),
        baseline: m.cell_baseline,
        underline: (m.underline_position, m.underline_thickness),
        strikethrough: (m.strikethrough_position, m.strikethrough_thickness),
        box_thickness: m.box_thickness,
        cursor_height: m.cursor_height,
    }
}

/// The size Ghostty loads a face at on macOS, whose default DPI is 72: `round(scale × 72)`.
/// [`DesiredSize::new`] uses the platform's, 96 off macOS.
fn macos_size(points: f32, scale: f32) -> DesiredSize {
    let dpi = (scale * 72.0).round() as u16;
    DesiredSize {
        points,
        xdpi: dpi,
        ydpi: dpi,
    }
}

fn metrics(data: &[u8], points: f32, scale: f32) -> Metrics {
    Metrics::calc(&FaceMetrics::from_data(data, 0, macos_size(points, scale)).unwrap())
}

#[track_caller]
fn check(font: &FontFile, data: &[u8], points: f32, scale: f32, expected: Cell) {
    assert_eq!(
        cell(&metrics(data, points, scale)),
        expected,
        "{} at {points} pt × {scale}",
        font.name
    );
}

/// Eli's settings: `font_size` 17 × `zoom` 1.1, multiplied in f32 as herdr-app does.
const ELI_POINTS: f32 = 17.0 * 1.1;

#[test]
fn operator_mono_book() {
    let font = common::operator_mono("Book");
    let Some(data) = font.load() else { return };

    // The parity cell, the same in Herdr app (0.1.18 and later) and in Ghostty.
    check(
        &font,
        &data,
        ELI_POINTS,
        1.0,
        Cell {
            size: (10, 22),
            baseline: 5,
            underline: (19, 2),
            strikethrough: (11, 2),
            box_thickness: 2,
            cursor_height: 22,
        },
    );
    check(
        &font,
        &data,
        ELI_POINTS,
        2.0,
        Cell {
            size: (21, 45),
            baseline: 9,
            underline: (39, 3),
            strikethrough: (24, 3),
            box_thickness: 3,
            cursor_height: 45,
        },
    );
    // Zoom reset (17 pt) and one step in (× 1.2), on Retina.
    check(
        &font,
        &data,
        17.0,
        2.0,
        Cell {
            size: (19, 41),
            baseline: 8,
            underline: (36, 3),
            strikethrough: (22, 3),
            box_thickness: 3,
            cursor_height: 41,
        },
    );
    check(
        &font,
        &data,
        17.0 * 1.2,
        2.0,
        Cell {
            size: (22, 49),
            baseline: 10,
            underline: (42, 3),
            strikethrough: (26, 3),
            box_thickness: 3,
            cursor_height: 49,
        },
    );

    let m = metrics(&data, ELI_POINTS, 1.0);
    // 550 units at 18.7 px per em, and hhea's 960 + 240 (USE_TYPO_METRICS is off).
    assert!((m.face_width - 10.285).abs() < 1e-5, "{}", m.face_width);
    assert!((m.face_height - 22.44).abs() < 1e-5, "{}", m.face_height);
}

#[test]
fn operator_mono_styles_have_the_regular_cell() {
    // Ghostty sizes the grid from the primary (regular) face only, but the styles agree anyway.
    for style in ["Bold", "BookItalic", "BoldItalic"] {
        let font = common::operator_mono(style);
        let Some(data) = font.load() else { continue };
        assert_eq!(
            metrics(&data, ELI_POINTS, 1.0).cell_size(),
            CellSize {
                width: 10,
                height: 22
            },
            "{style}"
        );
        assert_eq!(
            metrics(&data, ELI_POINTS, 2.0).cell_size(),
            CellSize {
                width: 21,
                height: 45
            },
            "{style}"
        );
    }
}

#[test]
fn jetbrains_mono() {
    let font = common::jetbrains_mono();
    let data = font.load().expect("checked in");

    // herdr-app's default font at 14 pt: 8.4 × 18.48 at 1x, 16.8 × 36.96 at 2x.
    check(
        &font,
        &data,
        14.0,
        1.0,
        Cell {
            size: (8, 18),
            baseline: 4,
            underline: (16, 1),
            strikethrough: (10, 1),
            box_thickness: 1,
            cursor_height: 18,
        },
    );
    check(
        &font,
        &data,
        14.0,
        2.0,
        Cell {
            size: (17, 37),
            baseline: 8,
            underline: (33, 2),
            strikethrough: (20, 2),
            box_thickness: 2,
            cursor_height: 37,
        },
    );
    check(
        &font,
        &data,
        13.0,
        2.0,
        Cell {
            size: (16, 34),
            baseline: 8,
            underline: (30, 2),
            strikethrough: (18, 2),
            box_thickness: 2,
            cursor_height: 34,
        },
    );
    // A fractional scale: 21 px per em.
    check(
        &font,
        &data,
        14.0,
        1.5,
        Cell {
            size: (13, 28),
            baseline: 6,
            underline: (25, 2),
            strikethrough: (15, 2),
            box_thickness: 2,
            cursor_height: 28,
        },
    );
}

#[test]
fn geist_mono() {
    let font = common::geist_mono();
    let data = font.load().expect("checked in");

    check(
        &font,
        &data,
        13.0,
        1.0,
        Cell {
            size: (8, 17),
            baseline: 4,
            underline: (14, 1),
            strikethrough: (9, 1),
            box_thickness: 1,
            cursor_height: 17,
        },
    );
    check(
        &font,
        &data,
        13.0,
        2.0,
        Cell {
            size: (16, 34),
            baseline: 8,
            underline: (29, 2),
            strikethrough: (18, 2),
            box_thickness: 2,
            cursor_height: 34,
        },
    );
    check(
        &font,
        &data,
        ELI_POINTS,
        2.0,
        Cell {
            size: (22, 49),
            baseline: 11,
            underline: (42, 2),
            strikethrough: (26, 2),
            box_thickness: 2,
            cursor_height: 49,
        },
    );
}

#[test]
fn menlo() {
    // macOS's terminal font: hhea metrics with a typo line gap that USE_TYPO_METRICS doesn't pick.
    let font = FontFile {
        name: "Menlo",
        path: "/System/Library/Fonts/Menlo.ttc".into(),
        index: 0,
    };
    let Some(data) = font.load() else { return };

    check(
        &font,
        &data,
        12.0,
        1.0,
        Cell {
            size: (7, 14),
            baseline: 3,
            underline: (12, 1),
            strikethrough: (8, 1),
            box_thickness: 1,
            cursor_height: 14,
        },
    );
    check(
        &font,
        &data,
        13.0,
        2.0,
        Cell {
            size: (16, 30),
            baseline: 6,
            underline: (26, 2),
            strikethrough: (17, 2),
            box_thickness: 2,
            cursor_height: 30,
        },
    );
}

#[test]
fn modifiers_on_the_parity_font() {
    let font = common::operator_mono("Book");
    let Some(data) = font.load() else { return };
    let face = FaceMetrics::from_data(&data, 0, macos_size(ELI_POINTS, 2.0)).unwrap();

    // adjust-cell-height = 20%: 45 → 54, 9 px more, split around the face. face_y is 0.024 and
    // centered would be (45 − 44.88) / 2 = 0.06, so the face sits low and the odd pixel goes below.
    let taller = Metrics::with_modifiers(
        &face,
        &Modifiers {
            cell_height: Some(Modifier::parse("20%").unwrap()),
            ..Default::default()
        },
    );
    assert_eq!(
        taller.cell_size(),
        CellSize {
            width: 21,
            height: 54
        }
    );
    assert_eq!(taller.cell_baseline, 9 + 5);
    assert_eq!(taller.underline_position, 39 + 4);
    assert_eq!(taller.strikethrough_position, 24 + 4);
    assert_eq!(taller.overline_position, 4);
    assert_eq!(taller.cursor_height, 45);

    // adjust-cell-width = -1 and adjust-underline-thickness = -10: clamped to 1 px.
    let narrower = Metrics::with_modifiers(
        &face,
        &Modifiers {
            cell_width: Some(Modifier::parse("-1").unwrap()),
            underline_thickness: Some(Modifier::parse("-10").unwrap()),
            ..Default::default()
        },
    );
    assert_eq!(
        narrower.cell_size(),
        CellSize {
            width: 20,
            height: 45
        }
    );
    assert_eq!(narrower.underline_thickness, 1);
}
