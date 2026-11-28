//! Every glyph of a long line draws on its own cell, under the cursor when the cursor is there,
//! as in Ghostty. Drawn by the real renderer (`--snap`, as the Ghostty pixel diff draws), at a
//! zoomed scale: the cell is measured at Ghostty's whole-number DPI, so at 2.2 (zoom 1.1 on a 2x
//! display) a line shaped at another size drifts a fraction of a pixel a column, cells by the
//! line's end.

use std::path::{Path, PathBuf};
use std::process::Command;

use herdr_frontend::TERMINAL_POINTS;
use herdr_frontend::view::terminal::font::Font;

/// A scale that isn't a whole number, where the drift showed.
const SCALE: f32 = 2.2;
const COLS: u16 = 200;
/// The cursor's columns: at the line's start, a fifth of the way, halfway and near its end.
const CURSOR_COLS: [u16; 4] = [0, 40, 100, 160];

/// An RGBA image.
struct Image {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Image {
    fn read(path: &Path) -> Image {
        let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba, "the snap is RGBA");
        let (width, height) = (info.width as usize, info.height as usize);
        Image {
            width,
            height,
            rgba: buf[..width * height * 4].to_vec(),
        }
    }

    /// How far pixel `(x, y)` is from `bg`, summed over the channels.
    fn ink(&self, x: usize, y: usize, bg: [u8; 3]) -> u32 {
        let p = &self.rgba[(y * self.width + x) * 4..][..3];
        (0..3).map(|i| u32::from(p[i].abs_diff(bg[i]))).sum()
    }
}

/// A 200-column line of text with an `o` alone at each cursor column (blanks either side), then
/// the cursor as an underline at column `cursor`, so it sits below the `o` rather than over it.
fn screen(cursor: u16) -> Vec<u8> {
    let filler = "the quick brown fox jumps over the lazy dog ";
    let mut line: Vec<char> = filler.chars().cycle().take(usize::from(COLS)).collect();
    for col in CURSOR_COLS.map(usize::from) {
        line[col] = 'o';
        if col > 0 {
            line[col - 1] = ' ';
        }
        line[col + 1] = ' ';
    }
    let line: String = line.into_iter().collect();
    format!("\x1b[4 q{line}\x1b[1;{}H", cursor + 1).into_bytes()
}

/// Draws `bytes` on a `COLS` × 2 grid at [`SCALE`], with no settings: JetBrains Mono at 14 pt.
fn snap(bytes: &[u8], dir: &Path, name: &str, size: (f32, f32)) -> Image {
    let screen = dir.join(format!("{name}.bin"));
    let png = dir.join(format!("{name}.png"));
    std::fs::write(&screen, bytes).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_herdr-frontend"))
        .env("HERDR_FRONTEND_SCREEN", &screen)
        .env("HERDR_FRONTEND_GRID", format!("{COLS}x2"))
        .env_remove("HERDR_FRONTEND_SETTINGS")
        .args(["--snap"])
        .arg(&png)
        .args(["--scene", "screen-file", "--scale", &SCALE.to_string()])
        .arg("--size")
        .arg(format!("{}x{}", size.0.ceil(), size.1.ceil()))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    Image::read(&png)
}

fn scratch() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("glyph-placement");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_long_lines_glyphs_sit_inside_the_cursor_cell_in_every_column() {
    let face = Font::jetbrains_mono(TERMINAL_POINTS).at(SCALE).unwrap();
    let m = face.metrics;
    let (cw, ch) = (m.cell_width as usize, m.cell_height as usize);
    // The text sits above the baseline (`o` has no descender); the underline cursor below it.
    let baseline = ch - m.cell_baseline as usize;
    let (cell_w, cell_h) = face.cell_pt();
    let size = (f32::from(COLS) * cell_w, 2.0 * cell_h);
    let dir = scratch();
    for cursor in CURSOR_COLS {
        let image = snap(&screen(cursor), &dir, &format!("cursor-{cursor}"), size);
        assert!(image.width >= usize::from(COLS) * cw && image.height >= 2 * ch);
        // The second row is blank: the terminal's background.
        let bg = {
            let p = &image.rgba[((ch + ch / 2) * image.width + 1) * 4..][..3];
            [p[0], p[1], p[2]]
        };
        let col = usize::from(cursor);
        let cell = col * cw..(col + 1) * cw;
        // From the blank cell before the `o` to the blank cell after it.
        let around = col.saturating_sub(1) * cw..(col + 2) * cw;

        // The cursor: its columns of ink below the baseline.
        let under: Vec<usize> = around
            .clone()
            .filter(|&x| (baseline + 1..ch).any(|y| image.ink(x, y, bg) > 96))
            .collect();
        assert_eq!(
            (under.first().copied(), under.last().map(|x| x + 1)),
            (Some(cell.start), Some(cell.end)),
            "the cursor at column {cursor} covers its cell"
        );

        // The `o`: all its ink above the baseline inside the cursor's cell, centered in it. Ink
        // in the blank cells either side is the `o`, or a neighbor, drawn off its column.
        let (mut sum, mut weight, mut outside) = (0.0, 0.0, Vec::new());
        for x in around.clone() {
            for y in 0..baseline {
                let ink = image.ink(x, y, bg);
                if ink == 0 {
                    continue;
                }
                if !cell.contains(&x) {
                    outside.push((x, y, ink));
                }
                sum += f64::from(ink) * (x as f64 + 0.5);
                weight += f64::from(ink);
            }
        }
        assert!(weight > 0.0, "column {cursor} draws its `o`");
        assert!(
            outside.is_empty(),
            "column {cursor}: ink beside the cursor's cell {cell:?}, in a blank cell: {outside:?}"
        );
        let center = (cell.start + cell.end) as f64 / 2.0;
        let off = sum / weight - center;
        assert!(
            off.abs() <= 1.0,
            "column {cursor}: the `o`'s ink is {off:+.2} px from its cell's center"
        );
    }
}
