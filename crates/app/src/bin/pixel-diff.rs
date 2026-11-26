//! Compares what Ghostty.app drew for a screen with what herdr-frontend's renderer drew for the
//! same bytes, cell by cell (`docs/research/renderer-gates.md`, `scripts/ghostty-diff`).
//!
//!     pixel-diff GHOSTTY.png OURS.png SCREEN.bin COLS ROWS [DIFF.png]
//!
//! Both images are 2x captures with the grid at their top left: Ghostty's window and our
//! `--snap`. The screen is replayed into a terminal to tell each cell's kind, so the report can
//! say how text, decorated text, sprites and backgrounds each compare, and how each sprite
//! character does on its own. For text it also measures where the ink sits in each cell, which
//! is what the glyph placement gate asks. The diff image shows Ghostty's pixels dimmed, with
//! every differing pixel in red.

use std::fs::File;
use std::io::BufWriter;
use std::process::ExitCode;

use herdr_frontend::fixtures;
use herdr_frontend::view::terminal::font::Font;
use herdr_frontend::view::terminal::look;
use herdr_frontend::view::terminal::sprite;
use terminal::{Snapshot, Width};

/// An RGBA image.
struct Image {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Image {
    fn read(path: &str) -> Result<Image, String> {
        let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().map_err(|e| format!("{path}: {e}"))?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("image too large")?];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("{path}: {e}"))?;
        let (width, height) = (info.width as usize, info.height as usize);
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf[..width * height * 4].to_vec(),
            png::ColorType::Rgb => buf[..width * height * 3]
                .chunks(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => buf[..width * height * 2]
                .chunks(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Grayscale => buf[..width * height]
                .iter()
                .flat_map(|&g| [g, g, g, 255])
                .collect(),
            png::ColorType::Indexed => return Err(format!("{path}: indexed PNG")),
        };
        Ok(Image {
            width,
            height,
            rgba,
        })
    }

    fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }
}

/// What a cell holds, for grouping the comparison.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Nothing drawn but its background.
    Blank,
    /// A narrow character the regular face draws, on the row's line.
    Text,
    /// Text with an underline, strikethrough or overline.
    Decorated,
    /// A character our renderer draws as Ghostty's sprite does.
    Sprite,
    /// A character Ghostty draws as a sprite that we still take from a font.
    FontSprite,
    /// A wide, fallback or multi-code-point cell, pinned to its column.
    Pinned,
    /// Under the cursor.
    Cursor,
}

const KINDS: [Kind; 7] = [
    Kind::Blank,
    Kind::Text,
    Kind::Decorated,
    Kind::Sprite,
    Kind::FontSprite,
    Kind::Pinned,
    Kind::Cursor,
];

#[derive(Default, Clone, Copy)]
struct Tally {
    cells: usize,
    pixels: usize,
    /// Pixels whose largest channel difference is over 0, 16 and 64.
    over: [usize; 3],
    abs_sum: u64,
    identical_cells: usize,
}

impl Tally {
    fn line(&self, name: &str) -> String {
        let pct = |n: usize| 100.0 * n as f64 / self.pixels.max(1) as f64;
        format!(
            "| {name} | {} | {} | {:.2}% | {:.2}% | {:.2}% | {:.2} |",
            self.cells,
            self.identical_cells,
            pct(self.over[0]),
            pct(self.over[1]),
            pct(self.over[2]),
            self.abs_sum as f64 / (self.pixels.max(1) * 3) as f64,
        )
    }
}

/// Where a cell's ink sits: its weight and centroid, from the difference to the cell's corner.
fn ink(img: &Image, x0: usize, y0: usize, w: usize, h: usize) -> (f64, f64, f64) {
    let bg = img.px(x0, y0);
    let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let p = img.px(x, y);
            let d = (0..3)
                .map(|c| (f64::from(p[c]) - f64::from(bg[c])).abs())
                .sum::<f64>()
                / 3.0;
            sum += d;
            sx += d * (x - x0) as f64;
            sy += d * (y - y0) as f64;
        }
    }
    if sum == 0.0 {
        (0.0, 0.0, 0.0)
    } else {
        (sum, sx / sum, sy / sum)
    }
}

/// Each cell's kind, and its character if it's a sprite.
fn kinds(
    snapshot: &Snapshot,
    face: &herdr_frontend::view::terminal::font::Face,
) -> Vec<(Kind, Option<char>)> {
    let cursor = snapshot.cursor.position.filter(|_| snapshot.cursor.visible);
    let mut kinds = Vec::new();
    for (y, row) in snapshot.screen.iter().enumerate() {
        let mut spill = 0;
        for (x, cell) in row.cells.iter().enumerate() {
            let text = cell.text.as_str();
            let mut chars = text.chars();
            let single = match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            };
            let decorated = cell.underline != terminal::Underline::None
                || cell.flags.contains(terminal::CellFlags::STRIKETHROUGH)
                || cell.flags.contains(terminal::CellFlags::OVERLINE);
            let sprite_char = single.filter(|&c| sprite::ghostty_draws(c));
            let kind = if cursor == Some((x as u16, y as u16)) {
                Kind::Cursor
            } else if let Some(c) = sprite_char {
                if sprite::drawn(c) {
                    Kind::Sprite
                } else {
                    Kind::FontSprite
                }
            } else if spill > 0 {
                Kind::Pinned
            } else if text.is_empty() || text == " " || cell.width == Width::SpacerHead {
                if decorated {
                    Kind::Decorated
                } else {
                    Kind::Blank
                }
            } else if cell.width == Width::Wide || single.is_none() {
                Kind::Pinned
            } else if single.is_some_and(|c| face.covers(c)) {
                if decorated {
                    Kind::Decorated
                } else {
                    Kind::Text
                }
            } else {
                Kind::Pinned
            };
            spill = if cell.width == Width::Wide { 1 } else { 0 };
            kinds.push((kind, sprite_char));
        }
    }
    kinds
}

fn run(args: &[String]) -> Result<(), String> {
    let [ghostty, ours, screen, cols, rows, rest @ ..] = args else {
        return Err(
            "usage: pixel-diff GHOSTTY.png OURS.png SCREEN.bin COLS ROWS [DIFF.png]".into(),
        );
    };
    let cols: u16 = cols.parse().map_err(|_| "COLS isn't a number")?;
    let rows: u16 = rows.parse().map_err(|_| "ROWS isn't a number")?;
    let g = Image::read(ghostty)?;
    let o = Image::read(ours)?;
    let face = Font::jetbrains_mono(herdr_frontend::TERMINAL_POINTS)
        .at(2.0)
        .map_err(|e| e.to_string())?;
    let (cw, ch) = (
        face.metrics.cell_width as usize,
        face.metrics.cell_height as usize,
    );
    let (w, h) = (usize::from(cols) * cw, usize::from(rows) * ch);
    for (name, img) in [("Ghostty's", &g), ("our", &o)] {
        if img.width < w || img.height < h {
            return Err(format!(
                "{name} image is {}x{}, smaller than the {w}x{h} grid",
                img.width, img.height
            ));
        }
    }
    let bytes = std::fs::read(screen).map_err(|e| format!("{screen}: {e}"))?;
    let (theme, _) = look::arctic();
    let snapshot = fixtures::replay(&bytes, cols, rows, &theme);
    let kinds = kinds(&snapshot, &face);

    let mut tallies = [Tally::default(); 7];
    let mut by_char: std::collections::BTreeMap<char, Tally> = Default::default();
    let mut total = Tally::default();
    let mut diff = vec![0u8; w * h * 4];
    // Text placement: centroid offsets (ours − Ghostty) and ink ratios.
    let (mut n, mut dx, mut dy, mut adx, mut ady, mut ratio) = (0usize, 0.0, 0.0, 0.0, 0.0, 0.0);
    for cy in 0..usize::from(rows) {
        for cx in 0..usize::from(cols) {
            let (kind, sprite_char) = kinds[cy * usize::from(cols) + cx];
            let t = &mut tallies[KINDS.iter().position(|&k| k == kind).unwrap_or(0)];
            t.cells += 1;
            total.cells += 1;
            let mut own = Tally {
                cells: 1,
                ..Tally::default()
            };
            let mut same = true;
            for y in cy * ch..(cy + 1) * ch {
                for x in cx * cw..(cx + 1) * cw {
                    let (a, b) = (g.px(x, y), o.px(x, y));
                    let d = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
                    let sum: u64 = (0..3).map(|c| u64::from(a[c].abs_diff(b[c]))).sum();
                    for tally in [&mut *t, &mut total, &mut own] {
                        tally.pixels += 1;
                        tally.abs_sum += sum;
                        for (k, limit) in [0, 16, 64].into_iter().enumerate() {
                            if d > limit {
                                tally.over[k] += 1;
                            }
                        }
                    }
                    same &= d == 0;
                    let i = (y * w + x) * 4;
                    let dim = |v: u8| v / 3;
                    diff[i..i + 4].copy_from_slice(&if d > 16 {
                        [255, dim(a[1]), dim(a[2]), 255]
                    } else {
                        [dim(a[0]), dim(a[1]), dim(a[2]), 255]
                    });
                }
            }
            if same {
                t.identical_cells += 1;
                total.identical_cells += 1;
                own.identical_cells += 1;
            }
            if let Some(c) = sprite_char {
                let e = by_char.entry(c).or_default();
                e.cells += 1;
                e.pixels += own.pixels;
                e.abs_sum += own.abs_sum;
                e.identical_cells += own.identical_cells;
                for k in 0..3 {
                    e.over[k] += own.over[k];
                }
            }
            if kind == Kind::Text {
                let (gw, gx, gy) = ink(&g, cx * cw, cy * ch, cw, ch);
                let (ow, ox, oy) = ink(&o, cx * cw, cy * ch, cw, ch);
                if gw > 0.0 && ow > 0.0 {
                    n += 1;
                    dx += ox - gx;
                    dy += oy - gy;
                    adx += (ox - gx).abs();
                    ady += (oy - gy).abs();
                    ratio += ow / gw;
                }
            }
        }
    }

    println!("grid {cols}x{rows}, cell {cw}x{ch} px");
    println!(
        "background: Ghostty {:?}, ours {:?}",
        g.px(w - 1, h - 1),
        o.px(w - 1, h - 1)
    );
    println!();
    println!(
        "| cells | count | identical | pixels differing | > 16 | > 64 | mean abs diff (0-255) |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for (kind, t) in KINDS.iter().zip(&tallies) {
        if t.cells > 0 {
            println!("{}", t.line(&format!("{kind:?}")));
        }
    }
    println!("{}", total.line("all"));
    if n > 0 {
        let n = n as f64;
        println!();
        println!(
            "text ink, ours − Ghostty over {n} cells: centroid dx {:+.3} px (mean |dx| {:.3}), \
             dy {:+.3} px (mean |dy| {:.3}), ink weight ratio {:.3}",
            dx / n,
            adx / n,
            dy / n,
            ady / n,
            ratio / n
        );
    }
    if !by_char.is_empty() {
        println!();
        println!("sprites, by character (the cells holding each):");
        println!();
        println!(
            "| char | cells | identical | pixels differing | > 16 | > 64 | mean abs diff (0-255) |"
        );
        println!("| --- | --- | --- | --- | --- | --- | --- |");
        for (c, t) in &by_char {
            let drawn = if sprite::drawn(*c) { "" } else { " (font)" };
            println!("{}", t.line(&format!("`{c}` U+{:04X}{drawn}", *c as u32)));
        }
    }
    if let [out, ..] = rest {
        let file = File::create(out).map_err(|e| format!("{out}: {e}"))?;
        let mut encoder = png::Encoder::new(BufWriter::new(file), w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&diff).map_err(|e| e.to_string())?;
        println!();
        println!("diff: {out}");
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("pixel-diff: {err}");
            ExitCode::FAILURE
        }
    }
}
