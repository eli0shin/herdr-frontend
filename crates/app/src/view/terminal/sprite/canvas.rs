//! Ghostty's sprite canvas (`ghostty:src/font/sprite/canvas.zig`): an alpha-only surface the
//! cell's size plus a quarter cell of padding on every side, in device pixels with the origin at
//! the cell's top left. Boxes set pixels; paths fill and stroke through [`super::raster`], z2d's
//! rasterizer.
//!
//! A finished canvas becomes a [`Sprite`]: its pixels as boxes of one alpha each, which the row's
//! vector layer fills in the cell's color.

use super::raster::{Cap, FillRule, Join, Path, Polygon};

/// Alpha-only pixels for one sprite.
pub struct Canvas {
    width: i32,
    height: i32,
    pub pad_x: i32,
    pub pad_y: i32,
    buf: Vec<u8>,
}

/// A sprite's pixels as boxes in device pixels from the cell's top left, each at one alpha.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sprite {
    pub boxes: Vec<AlphaBox>,
}

/// Pixels `x0..x1` by `y0..y1` from the cell's top left, which may reach past the cell, at
/// coverage `alpha`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlphaBox {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub alpha: u8,
}

impl Canvas {
    /// A canvas for a cell `width` × `height`, padded by a quarter of each (`Face.renderGlyph`).
    pub fn new(width: u32, height: u32) -> Canvas {
        let (pad_x, pad_y) = ((width / 4) as i32, (height / 4) as i32);
        let (w, h) = (width as i32 + 2 * pad_x, height as i32 + 2 * pad_y);
        Canvas {
            width: w,
            height: h,
            pad_x,
            pad_y,
            buf: vec![0; (w * h) as usize],
        }
    }

    /// Sets pixel (`x`, `y`) of the cell, as `Canvas.pixel` does; outside the canvas is dropped.
    pub fn pixel(&mut self, x: i32, y: i32, alpha: u8) {
        let (x, y) = (x + self.pad_x, y + self.pad_y);
        if x >= 0 && y >= 0 && x < self.width && y < self.height {
            self.buf[(y * self.width + x) as usize] = alpha;
        }
    }

    /// Sets the pixels `x..x + w` by `y..y + h` (`Canvas.rect`).
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, alpha: u8) {
        for py in y..y + h {
            for px in x..x + w {
                self.pixel(px, py, alpha);
            }
        }
    }

    /// Sets the pixels between two corners, in either order (`Canvas.box`).
    pub fn box_(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, alpha: u8) {
        let (l, r) = (x0.min(x1), x0.max(x1));
        let (t, b) = (y0.min(y1), y0.max(y1));
        self.rect(l, t, r - l, b - t, alpha);
    }

    /// Fills `path` (in the cell's pixels) non-zero, fully on.
    pub fn fill(&mut self, path: &Path) {
        self.fill_with(path, FillRule::NonZero);
    }

    pub fn fill_with(&mut self, path: &Path, rule: FillRule) {
        let polygon = Polygon::from_fill(&self.padded(path));
        polygon.rasterize(rule, &mut self.buf, self.width, self.height);
    }

    /// A filled triangle (`Canvas.triangle`).
    pub fn triangle(&mut self, p: [(f64, f64); 3]) {
        let path = Path::new()
            .move_to(p[0].0, p[0].1)
            .line_to(p[1].0, p[1].1)
            .line_to(p[2].0, p[2].1)
            .close();
        self.fill(&path);
    }

    /// Strokes `path` `width` wide with miter joins (z2d's default).
    pub fn stroke(&mut self, path: &Path, width: f64, cap: Cap) {
        let polygon = Polygon::from_stroke(&self.padded(path), width, cap, Join::Miter);
        polygon.rasterize(FillRule::NonZero, &mut self.buf, self.width, self.height);
    }

    /// A straight line `width` wide with butt caps (`Canvas.line`).
    pub fn line(&mut self, from: (f64, f64), to: (f64, f64), width: f64) {
        let path = Path::new().move_to(from.0, from.1).line_to(to.0, to.1);
        self.stroke(&path, width, Cap::Butt);
    }

    /// Mirrors the canvas left to right (`Canvas.flipHorizontal`). The padding is the same on
    /// both sides, so the cell lands on itself.
    pub fn flip_horizontal(&mut self) {
        let w = self.width as usize;
        for row in self.buf.chunks_mut(w) {
            row.reverse();
        }
    }

    /// `path` moved from the cell's pixels to the canvas's.
    fn padded(&self, path: &Path) -> Path {
        path.translated(f64::from(self.pad_x), f64::from(self.pad_y))
    }

    /// The canvas's pixels as boxes: runs of one alpha along each row, merged down while the
    /// next row has the same run.
    pub fn sprite(&self) -> Sprite {
        let w = self.width as usize;
        let mut open: Vec<AlphaBox> = Vec::new();
        let mut done: Vec<AlphaBox> = Vec::new();
        for (y, row) in self.buf.chunks(w).enumerate() {
            let y = y as i32 - self.pad_y;
            let mut runs = Vec::new();
            let mut x = 0;
            while x < w {
                let a = row[x];
                let start = x;
                while x < w && row[x] == a {
                    x += 1;
                }
                if a > 0 {
                    runs.push((start as i32 - self.pad_x, x as i32 - self.pad_x, a));
                }
            }
            let mut next = Vec::with_capacity(runs.len());
            for (x0, x1, alpha) in runs {
                match open
                    .iter()
                    .position(|b| b.x0 == x0 && b.x1 == x1 && b.alpha == alpha && b.y1 == y)
                {
                    Some(i) => {
                        let mut b = open.swap_remove(i);
                        b.y1 = y + 1;
                        next.push(b);
                    }
                    None => next.push(AlphaBox {
                        x0,
                        y0: y,
                        x1,
                        y1: y + 1,
                        alpha,
                    }),
                }
            }
            done.append(&mut open);
            open = next;
        }
        done.append(&mut open);
        done.sort_by_key(|b| (b.y0, b.x0));
        Sprite { boxes: done }
    }
}

impl Sprite {
    /// Whether the sprite fills pixel (`x`, `y`) of its cell fully.
    pub fn covers(&self, x: i32, y: i32) -> bool {
        self.boxes
            .iter()
            .any(|b| b.alpha == 255 && (b.x0..b.x1).contains(&x) && (b.y0..b.y1).contains(&y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_merge_down_and_keep_their_alpha() {
        let mut c = Canvas::new(8, 16);
        c.box_(0, 0, 8, 8, 255);
        c.box_(2, 8, 4, 10, 0x80);
        assert_eq!(
            c.sprite().boxes,
            [
                AlphaBox {
                    x0: 0,
                    y0: 0,
                    x1: 8,
                    y1: 8,
                    alpha: 255
                },
                AlphaBox {
                    x0: 2,
                    y0: 8,
                    x1: 4,
                    y1: 10,
                    alpha: 0x80
                },
            ]
        );
    }

    #[test]
    fn the_padding_lets_a_sprite_reach_past_its_cell() {
        let mut c = Canvas::new(8, 16);
        c.rect(-2, -4, 2, 4, 255);
        c.rect(-3, 0, 1, 1, 255);
        let s = c.sprite();
        assert!(s.covers(-2, -4) && s.covers(-1, -1));
        assert!(!s.covers(-3, 0), "past the padding is dropped");
    }

    #[test]
    fn a_horizontal_flip_mirrors_the_cell_onto_itself() {
        let mut c = Canvas::new(8, 16);
        c.rect(0, 0, 2, 16, 255);
        c.flip_horizontal();
        let s = c.sprite();
        assert!(s.covers(6, 0) && s.covers(7, 15) && !s.covers(0, 0));
    }
}
