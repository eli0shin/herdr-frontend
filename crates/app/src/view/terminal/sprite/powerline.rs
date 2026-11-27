//! Powerline and Powerline Extra symbols Ghostty draws itself (U+E0B0 to U+E0BF, U+E0D2, U+E0D4):
//! a port of `ghostty:src/font/sprite/draw/powerline.zig` at `v1.4.0`. A wide cell's sprite is
//! as wide as both its cells.

use terminal::Metrics;

use super::box_drawing;
use super::canvas::Canvas;
use super::raster::{Cap, Path};

/// Draws Powerline symbol `cp` in a `width` × `height` canvas.
pub(super) fn draw(cp: u32, canvas: &mut Canvas, width: u32, height: u32, m: &Metrics) {
    let (w, h) = (f64::from(width), f64::from(height));
    match cp {
        0xE0B0 => canvas.triangle([(0.0, 0.0), (w, h / 2.0), (0.0, h)]),
        0xE0B1 => angle(canvas, w, h, m),
        0xE0B2 => canvas.triangle([(w, 0.0), (0.0, h / 2.0), (w, h)]),
        0xE0B3 => {
            angle(canvas, w, h, m);
            canvas.flip_horizontal();
        }
        0xE0B4 => half_disc(canvas, w, h),
        0xE0B5 => half_circle(canvas, w, h, m),
        0xE0B6 => {
            half_disc(canvas, w, h);
            canvas.flip_horizontal();
        }
        0xE0B7 => {
            half_circle(canvas, w, h, m);
            canvas.flip_horizontal();
        }
        0xE0B8 => canvas.triangle([(0.0, 0.0), (w, h), (0.0, h)]),
        0xE0B9 | 0xE0BF => box_drawing::draw(0x2572, canvas, m),
        0xE0BA => canvas.triangle([(w, 0.0), (w, h), (0.0, h)]),
        0xE0BB | 0xE0BD => box_drawing::draw(0x2571, canvas, m),
        0xE0BC => canvas.triangle([(0.0, 0.0), (w, 0.0), (0.0, h)]),
        0xE0BE => canvas.triangle([(0.0, 0.0), (w, 0.0), (w, h)]),
        0xE0D2 => flame(canvas, w, h, m),
        0xE0D4 => {
            flame(canvas, w, h, m);
            canvas.flip_horizontal();
        }
        _ => {}
    }
}

/// ``: a light chevron, stroked with butt caps.
fn angle(canvas: &mut Canvas, w: f64, h: f64, m: &Metrics) {
    let path = Path::new()
        .move_to(0.0, 0.0)
        .line_to(w, h / 2.0)
        .line_to(0.0, h);
    canvas.stroke(&path, f64::from(m.box_thickness), Cap::Butt);
}

/// The circular-arc handle length for a quarter turn.
fn arc_handle() -> f64 {
    (std::f64::consts::SQRT_2 - 1.0) * 4.0 / 3.0
}

/// ``: a filled half disc against the left edge.
fn half_disc(canvas: &mut Canvas, w: f64, h: f64) {
    let c = arc_handle();
    let r = w.min(h / 2.0);
    let path = Path::new()
        .move_to(0.0, 0.0)
        .curve_to(r * c, 0.0, r, r - r * c, r, r)
        .line_to(r, h - r)
        .curve_to(r, h - r + r * c, r * c, h, 0.0, h)
        .close();
    canvas.fill(&path);
}

/// ``: the half disc's outline, `box_thickness` wide on its inside. Ghostty strokes the path
/// inset by half the width (`innerStrokePath`); this strokes the same path moved in toward the
/// disc's center by half the width along each point's normal.
fn half_circle(canvas: &mut Canvas, w: f64, h: f64, m: &Metrics) {
    let c = arc_handle();
    let t = f64::from(m.box_thickness);
    // The outline is a half disc of radius r; its inset is the same shape of radius r − t/2,
    // with the flat ends kept at the cell's edges.
    let r = w.min(h / 2.0);
    let k = t / 2.0;
    let ri = (r - k).max(0.0);
    let path = Path::new()
        .move_to(0.0, k)
        .line_to(1.0, k)
        .curve_to(ri * c, k, ri, r - ri * c, ri, r)
        .line_to(ri, h - r)
        .curve_to(ri, h - r + ri * c, ri * c, h - k, 1.0, h - k)
        .line_to(0.0, h - k);
    canvas.stroke(&path, t, Cap::Butt);
}

/// ``: two wedges pointing at the middle of the right edge, a line's width apart.
fn flame(canvas: &mut Canvas, w: f64, h: f64, m: &Metrics) {
    let t = f64::from(m.box_thickness);
    let top = Path::new()
        .move_to(0.0, 0.0)
        .line_to(w, 0.0)
        .line_to(w / 2.0, h / 2.0 - t / 2.0)
        .line_to(0.0, h / 2.0 - t / 2.0)
        .close();
    canvas.fill(&top);
    let bottom = Path::new()
        .move_to(0.0, h)
        .line_to(w, h)
        .line_to(w / 2.0, h / 2.0 + t / 2.0)
        .line_to(0.0, h / 2.0 + t / 2.0)
        .close();
    canvas.fill(&bottom);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::terminal::sprite::tests::metrics;

    #[test]
    fn the_arrow_fills_the_left_edge_and_points_at_the_right() {
        let m = metrics(17, 37, 2);
        let mut c = Canvas::new(17, 37);
        draw(0xE0B0, &mut c, 17, 37, &m);
        let s = c.sprite();
        // The corners are cut by the slanted edges, so they're antialiased.
        assert!(s.covers(0, 2) && s.covers(0, 34) && s.covers(15, 18));
        assert!(!s.covers(0, 0) && s.boxes.iter().any(|b| b.alpha < 255));
        assert!(!s.covers(16, 0) && !s.covers(16, 36));
    }

    #[test]
    fn the_left_arrow_is_the_right_one_mirrored() {
        let m = metrics(17, 37, 2);
        let (mut a, mut b) = (Canvas::new(17, 37), Canvas::new(17, 37));
        draw(0xE0B4, &mut a, 17, 37, &m);
        draw(0xE0B6, &mut b, 17, 37, &m);
        let (a, b) = (a.sprite(), b.sprite());
        for y in 0..37 {
            for x in 0..17 {
                assert_eq!(a.covers(x, y), b.covers(16 - x, y), "({x}, {y})");
            }
        }
    }
}
