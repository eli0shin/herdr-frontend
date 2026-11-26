//! Text decorations as Ghostty's sprite face draws them, one cell at a time: a port of the
//! `underline*`, `strikethrough` and `overline` functions in
//! `ghostty:src/font/sprite/draw/special.zig` at `v1.4.0`. The canvas reaches a quarter cell past
//! the cell, so an underline can sit below it.

use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};

use terminal::{Metrics, Underline};

use super::canvas::Canvas;
use super::raster::{Cap, Path};

/// Draws `underline` in a `width` × `height` canvas.
pub(super) fn underline(
    style: Underline,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    m: &Metrics,
) {
    let (w, h) = (width as i32, height as i32);
    let pos = m.underline_position as i32;
    let thick = m.underline_thickness as i32;
    let pad = canvas.pad_y;
    match style {
        Underline::None => {}
        Underline::Single => {
            let y = pos.min((h + pad - thick).max(0));
            canvas.rect(0, y, w, thick, 255);
        }
        Underline::Double => {
            let y = pos.min((h + pad - 2 * thick).max(0));
            canvas.rect(0, (y - thick).max(0), w, thick, 255);
            canvas.rect(0, y + thick, w, thick, 255);
        }
        Underline::Dotted => dotted(canvas, width, height, m),
        Underline::Dashed => {
            let y = pos.min((h + pad - thick).max(0));
            let dash = width / 3 + 1;
            let count = width / dash + 1;
            for i in (0..count).step_by(2) {
                canvas.rect((i * dash) as i32, y, dash as i32, thick, 255);
            }
        }
        Underline::Curly => curly(canvas, width, height, m),
    }
}

/// `underline_dotted`: round dots √2 times the underline's thickness across, spaced about a
/// diameter apart.
fn dotted(canvas: &mut Canvas, width: u32, height: u32, m: &Metrics) {
    let (w, h) = (f64::from(width), f64::from(height));
    let thick = f64::from(m.underline_thickness);
    let radius = FRAC_1_SQRT_2 * thick;
    let pad = f64::from(canvas.pad_y);
    let y = (f64::from(m.underline_position) + 0.5 * thick).min(h + pad - radius.ceil());
    let count = (w / (4.0 * radius))
        .ceil()
        .min((w / (3.0 * radius)).floor())
        .min((w / (2.0 * radius + 1.0)).floor())
        .max(1.0);
    let mut path = Path::new();
    let mut x = (w / count) / 2.0;
    for _ in 0..count as usize {
        path = path.arc(x, y, radius, 0.0, TAU).close();
        x += w / count;
    }
    canvas.fill(&path);
}

/// `underline_curly`: one wave a cell, peaking at its middle, stroked the underline's thickness
/// wide with round caps.
fn curly(canvas: &mut Canvas, width: u32, height: u32, m: &Metrics) {
    let (w, h) = (f64::from(width), f64::from(height));
    let line = f64::from(m.underline_thickness);
    let amplitude = w / PI;
    let pad = f64::from(canvas.pad_y);
    let top = f64::from(m.underline_position).min(h + pad - amplitude - line);
    let bottom = top + amplitude;
    let r = 0.4;
    let center = 0.5 * w;
    let path = Path::new()
        .move_to(0.0, bottom)
        .curve_to(center * r, bottom, center - center * r, top, center, top)
        .curve_to(center + center * r, top, w - center * r, bottom, w, bottom);
    canvas.stroke(&path, line, Cap::Round);
}

/// `strikethrough`.
pub(super) fn strikethrough(canvas: &mut Canvas, width: u32, m: &Metrics) {
    canvas.rect(
        0,
        m.strikethrough_position as i32,
        width as i32,
        m.strikethrough_thickness as i32,
        255,
    );
}

/// `overline`: at the font's position, which may be above the cell, but no higher than the
/// canvas.
pub(super) fn overline(canvas: &mut Canvas, width: u32, m: &Metrics) {
    let y = m.overline_position.max(-canvas.pad_y);
    canvas.rect(0, y, width as i32, m.overline_thickness as i32, 255);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::terminal::sprite::tests::metrics;

    fn drawn(style: Underline) -> super::super::canvas::Sprite {
        let m = metrics(17, 37, 2);
        let mut c = Canvas::new(17, 37);
        underline(style, &mut c, 17, 37, &m);
        c.sprite()
    }

    #[test]
    fn a_single_underline_is_one_solid_box() {
        let m = metrics(17, 37, 2);
        let s = drawn(Underline::Single);
        assert_eq!(s.boxes.len(), 1);
        let b = s.boxes[0];
        assert_eq!((b.x0, b.x1), (0, 17));
        assert_eq!(
            (b.y0, b.y1 - b.y0),
            (m.underline_position as i32, m.underline_thickness as i32)
        );
    }

    #[test]
    fn a_curl_is_antialiased_and_spans_the_cell() {
        let s = drawn(Underline::Curly);
        assert!(s.boxes.iter().any(|b| b.alpha < 255), "antialiased");
        assert!(s.boxes.iter().any(|b| b.x0 <= 0) && s.boxes.iter().any(|b| b.x1 >= 17));
        // A 17 px cell's curl is 17 / π ≈ 5.4 px deep.
        let (top, bottom) = (
            s.boxes.iter().map(|b| b.y0).min().unwrap(),
            s.boxes.iter().map(|b| b.y1).max().unwrap(),
        );
        assert!((6..=10).contains(&(bottom - top)), "{top}..{bottom}");
    }

    #[test]
    fn dashes_and_dots_leave_gaps() {
        let m = metrics(17, 37, 2);
        let y = m.underline_position as i32;
        let dashed = drawn(Underline::Dashed);
        // 17 / 3 + 1 = 6 px dashes: on 0..6, off 6..12, on 12..17.
        assert!(dashed.covers(0, y) && !dashed.covers(8, y) && dashed.covers(13, y));
        let dotted = drawn(Underline::Dotted);
        assert!(dotted.boxes.iter().any(|b| b.alpha < 255));
    }
}
