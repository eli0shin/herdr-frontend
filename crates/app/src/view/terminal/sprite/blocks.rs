//! Block Elements (U+2580 to U+259F), Braille Patterns (U+2800 to U+28FF) and the geometric
//! shapes Ghostty draws (U+25E2 to U+25E5, U+25F8 to U+25FA, U+25FF): ports of
//! `ghostty:src/font/sprite/draw/block.zig`, `braille.zig` and `geometric_shapes.zig` at
//! `v1.4.0`.

use terminal::Metrics;

use super::canvas::Canvas;
use super::raster::{FillRule, Path};

/// `Shade`: the alpha of a shaded block.
const LIGHT: u8 = 0x40;
const MEDIUM: u8 = 0x80;
const DARK: u8 = 0xc0;
const ON: u8 = 0xff;

#[derive(Clone, Copy)]
enum Align {
    Upper,
    Lower,
    Left,
    Right,
}

/// Draws block element `cp` (U+2580 to U+259F).
pub(super) fn draw_block(cp: u32, canvas: &mut Canvas, m: &Metrics) {
    use Align::*;
    let e = |n: f64| n / 8.0;
    match cp {
        0x2580 => block(m, canvas, Upper, 1.0, 0.5),
        0x2581..=0x2587 => block(m, canvas, Lower, 1.0, e(f64::from(cp - 0x2580))),
        0x2588 => full(m, canvas, ON),
        0x2589..=0x258F => block(m, canvas, Left, e(f64::from(0x2590 - cp)), 1.0),
        0x2590 => block(m, canvas, Right, 0.5, 1.0),
        0x2591 => full(m, canvas, LIGHT),
        0x2592 => full(m, canvas, MEDIUM),
        0x2593 => full(m, canvas, DARK),
        0x2594 => block(m, canvas, Upper, 1.0, e(1.0)),
        0x2595 => block(m, canvas, Right, e(1.0), 1.0),
        // Quadrants: top left, top right, bottom left, bottom right.
        0x2596 => quadrants(m, canvas, [false, false, true, false]),
        0x2597 => quadrants(m, canvas, [false, false, false, true]),
        0x2598 => quadrants(m, canvas, [true, false, false, false]),
        0x2599 => quadrants(m, canvas, [true, false, true, true]),
        0x259A => quadrants(m, canvas, [true, false, false, true]),
        0x259B => quadrants(m, canvas, [true, true, true, false]),
        0x259C => quadrants(m, canvas, [true, true, false, true]),
        0x259D => quadrants(m, canvas, [false, true, false, false]),
        0x259E => quadrants(m, canvas, [false, true, true, false]),
        0x259F => quadrants(m, canvas, [false, true, true, true]),
        _ => {}
    }
}

/// `blockShade` fully on: a `width` × `height` fraction of the cell against one edge.
fn block(m: &Metrics, canvas: &mut Canvas, align: Align, width: f64, height: f64) {
    let w = (f64::from(m.cell_width) * width).round() as u32;
    let h = (f64::from(m.cell_height) * height).round() as u32;
    let x = match align {
        Align::Right => m.cell_width - w,
        _ => 0,
    };
    let y = match align {
        Align::Lower => m.cell_height - h,
        _ => 0,
    };
    canvas.rect(x as i32, y as i32, w as i32, h as i32, ON);
}

fn full(m: &Metrics, canvas: &mut Canvas, alpha: u8) {
    canvas.box_(0, 0, m.cell_width as i32, m.cell_height as i32, alpha);
}

/// `Fraction.min`: a fraction's left or top edge, rounded from the far end so the two halves of
/// an odd size tile.
fn frac_min(f: f64, size: u32) -> i32 {
    let s = f64::from(size);
    (s - ((1.0 - f) * s).round()) as i32
}

/// `Fraction.max`.
fn frac_max(f: f64, size: u32) -> i32 {
    (f * f64::from(size)).round() as i32
}

/// `common.fill` between fractions of the cell.
fn fill(m: &Metrics, canvas: &mut Canvas, x0: f64, x1: f64, y0: f64, y1: f64) {
    canvas.box_(
        frac_min(x0, m.cell_width),
        frac_min(y0, m.cell_height),
        frac_max(x1, m.cell_width),
        frac_max(y1, m.cell_height),
        ON,
    );
}

fn quadrants(m: &Metrics, canvas: &mut Canvas, [tl, tr, bl, br]: [bool; 4]) {
    if tl {
        fill(m, canvas, 0.0, 0.5, 0.0, 0.5);
    }
    if tr {
        fill(m, canvas, 0.5, 1.0, 0.0, 0.5);
    }
    if bl {
        fill(m, canvas, 0.0, 0.5, 0.5, 1.0);
    }
    if br {
        fill(m, canvas, 0.5, 1.0, 0.5, 1.0);
    }
}

/// Draws braille pattern `cp` (U+2800 to U+28FF) in a `width` × `height` canvas: square dots on
/// a 2 × 4 grid, with the pixels left over shared out as `braille.zig` does.
pub(super) fn draw_braille(cp: u32, canvas: &mut Canvas, width: u32, height: u32) {
    let (width, height) = (width as i32, height as i32);
    let mut w = (width / 4).min(height / 8);
    let mut x_spacing = width / 4;
    let mut y_spacing = height / 8;
    let mut x_margin = x_spacing.div_euclid(2);
    let mut y_margin = y_spacing.div_euclid(2);
    let mut x_left = width - 2 * x_margin - x_spacing - 2 * w;
    let mut y_left = height - 2 * y_margin - 3 * y_spacing - 4 * w;

    if x_left >= 2 && y_left >= 4 && w == 0 {
        w += 1;
        x_left -= 2;
        y_left -= 4;
    }
    if x_left >= 2 && x_margin == 0 {
        x_margin = 1;
        x_left -= 2;
    }
    if y_left >= 2 && y_margin == 0 {
        y_margin = 1;
        y_left -= 2;
    }
    if x_left >= 1 {
        x_spacing += 1;
        x_left -= 1;
    }
    if y_left >= 3 {
        y_spacing += 1;
        y_left -= 3;
    }
    if x_left >= 2 {
        x_margin += 1;
        x_left -= 2;
    }
    if y_left >= 2 {
        y_margin += 1;
        y_left -= 2;
    }
    if x_left >= 2 && y_left >= 4 {
        w += 1;
    }

    let x = [x_margin, x_margin + w + x_spacing];
    let y0 = y_margin;
    let y = [
        y0,
        y0 + w + y_spacing,
        y0 + 2 * (w + y_spacing),
        y0 + 3 * (w + y_spacing),
    ];
    // The code point's low byte: bits 0-2 are the left column's top three dots, 3-5 the right
    // column's, 6 and 7 the bottom row's left and right.
    let bits = cp & 0xff;
    let dots = [
        (0, 0, 0),
        (1, 0, 1),
        (2, 0, 2),
        (3, 1, 0),
        (4, 1, 1),
        (5, 1, 2),
        (6, 0, 3),
        (7, 1, 3),
    ];
    for (bit, col, row) in dots {
        if bits & (1 << bit) != 0 {
            let (dx, dy) = (x[col], y[row]);
            canvas.box_(dx, dy, dx + w, dy + w, ON);
        }
    }
}

/// Draws geometric shape `cp`: a filled corner triangle (◢◣◤◥) or a corner triangle's outline
/// (◸◹◺◿).
pub(super) fn draw_geometric(cp: u32, canvas: &mut Canvas, m: &Metrics) {
    let (w, h) = (f64::from(m.cell_width), f64::from(m.cell_height));
    let tl = [(0.0, 0.0), (0.0, h), (w, 0.0)];
    let tr = [(0.0, 0.0), (w, h), (w, 0.0)];
    let bl = [(0.0, 0.0), (0.0, h), (w, h)];
    let br = [(0.0, h), (w, h), (w, 0.0)];
    match cp {
        0x25E2 => canvas.triangle(br),
        0x25E3 => canvas.triangle(bl),
        0x25E4 => canvas.triangle(tl),
        0x25E5 => canvas.triangle(tr),
        0x25F8 => triangle_outline(canvas, tl, f64::from(m.box_thickness)),
        0x25F9 => triangle_outline(canvas, tr, f64::from(m.box_thickness)),
        0x25FA => triangle_outline(canvas, bl, f64::from(m.box_thickness)),
        0x25FF => triangle_outline(canvas, br, f64::from(m.box_thickness)),
        _ => {}
    }
}

/// `cornerTriangleOutline`: an inner stroke of the triangle, `thick` wide. With miter joins (all
/// within z2d's limit in a cell) that's the triangle less the triangle inset by `thick`.
fn triangle_outline(canvas: &mut Canvas, t: [(f64, f64); 3], thick: f64) {
    let inner = inset(&t, thick);
    let mut path = Path::new()
        .move_to(t[0].0, t[0].1)
        .line_to(t[1].0, t[1].1)
        .line_to(t[2].0, t[2].1)
        .close();
    if let Some(inner) = inner {
        path = path
            .move_to(inner[0].0, inner[0].1)
            .line_to(inner[1].0, inner[1].1)
            .line_to(inner[2].0, inner[2].1)
            .close();
    }
    canvas.fill_with(&path, FillRule::EvenOdd);
}

/// The triangle `t` with every edge moved `by` toward its inside, or `None` if nothing's left.
pub(super) fn inset(t: &[(f64, f64); 3], by: f64) -> Option<[(f64, f64); 3]> {
    let area = (t[1].0 - t[0].0) * (t[2].1 - t[0].1) - (t[2].0 - t[0].0) * (t[1].1 - t[0].1);
    let sign = area.signum();
    // Each edge's line, moved inward: a point on it and its direction.
    let lines: Vec<((f64, f64), (f64, f64))> = (0..3)
        .map(|i| {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt();
            // The inward normal of a counter-clockwise (in y-down, positive area) triangle.
            let (nx, ny) = (-dy / len * sign, dx / len * sign);
            ((a.0 + nx * by, a.1 + ny * by), (dx, dy))
        })
        .collect();
    let meet = |(p, d): ((f64, f64), (f64, f64)), (q, e): ((f64, f64), (f64, f64))| {
        let denom = d.0 * e.1 - d.1 * e.0;
        let t = ((q.0 - p.0) * e.1 - (q.1 - p.1) * e.0) / denom;
        (p.0 + t * d.0, p.1 + t * d.1)
    };
    let out = [
        meet(lines[2], lines[0]),
        meet(lines[0], lines[1]),
        meet(lines[1], lines[2]),
    ];
    let inner_area = (out[1].0 - out[0].0) * (out[2].1 - out[0].1)
        - (out[2].0 - out[0].0) * (out[1].1 - out[0].1);
    (inner_area.signum() == sign && inner_area != 0.0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::terminal::sprite::tests::metrics;

    #[test]
    fn halves_tile_an_odd_cell() {
        // `Fraction.min` and `max` split 37 rows into 0..19 and 18..37 so neither is short.
        assert_eq!((frac_min(0.0, 37), frac_max(0.5, 37)), (0, 19));
        assert_eq!((frac_min(0.5, 37), frac_max(1.0, 37)), (18, 37));
    }

    #[test]
    fn eighth_blocks_round_to_whole_pixels() {
        let m = metrics(17, 37, 2);
        let mut c = Canvas::new(17, 37);
        draw_block(0x2581, &mut c, &m);
        let s = c.sprite();
        // ▁ is round(37 / 8) = 5 rows at the bottom.
        assert_eq!(s.boxes.len(), 1);
        assert_eq!((s.boxes[0].y0, s.boxes[0].y1), (32, 37));
    }

    #[test]
    fn a_full_braille_cell_has_eight_square_dots() {
        let mut c = Canvas::new(17, 37);
        draw_braille(0x28FF, &mut c, 17, 37);
        let s = c.sprite();
        let dots: usize = s
            .boxes
            .iter()
            .map(|b| ((b.y1 - b.y0) / (b.x1 - b.x0)) as usize)
            .sum();
        assert_eq!(dots, 8, "{:?}", s.boxes);
        assert!(s.boxes.iter().all(|b| b.alpha == 255));
    }

    #[test]
    fn shades_are_partial_alpha() {
        let m = metrics(17, 37, 2);
        let mut c = Canvas::new(17, 37);
        draw_block(0x2592, &mut c, &m);
        assert_eq!(c.sprite().boxes[0].alpha, 0x80);
    }

    #[test]
    fn a_triangle_inset_stays_inside() {
        let t = [(0.0, 0.0), (0.0, 37.0), (17.0, 0.0)];
        let i = inset(&t, 2.0).unwrap();
        assert!(i.iter().all(|p| p.0 >= 2.0 - 1e-9 && p.1 >= 2.0 - 1e-9));
    }
}
