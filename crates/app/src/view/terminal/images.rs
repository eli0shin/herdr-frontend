//! Kitty images over a pane's grid, in Ghostty's three layers (`docs/research/terminal-rendering.md`
//! "Kitty graphics"): under the cell backgrounds, between the backgrounds and the text, and over
//! the text. Within a layer they draw in the snapshot's order, by z.
//!
//! A snapshot's sizes and positions are in the terminal's cell, the one it was last resized
//! with. They're drawn in the face's cell: the same cell on a settled pane, and when the two
//! differ (the frame after a zoom or a move to another display), every image scales by the face's
//! cell over the terminal's, so it still covers the cells it was placed on. Herdr app drew
//! direct placements at their pixel size over a fractional cell, which left them short of their
//! cells (§5, item 3).
//!
//! The pixels are uploaded once per image generation and shared by every placement of it; an
//! image gone from the snapshot leaves the cache on the next view.

use std::collections::HashMap;

use rune::{El, Pixels, PixelsWrap, el};
use terminal::{ImageLayer, Placement, Snapshot};

use super::font::Face;

/// The pixels of the images a pane showed last, by image generation.
#[derive(Default)]
pub struct ImageCache {
    pixels: HashMap<u64, Pixels>,
}

/// A snapshot's images, one element per layer that has any.
#[derive(Default)]
pub struct Layers {
    pub below_background: Option<El>,
    pub below_text: Option<El>,
    pub above_text: Option<El>,
}

impl ImageCache {
    /// How many images it holds pixels for.
    pub fn len(&self) -> usize {
        self.pixels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }

    /// `snapshot`'s images as layers over a grid `grid` points in size, each clipped to it.
    pub fn layers(&mut self, snapshot: &Snapshot, face: &Face, grid: (f32, f32)) -> Layers {
        self.pixels
            .retain(|g, _| snapshot.images.iter().any(|p| p.image.generation == *g));
        if snapshot.images.is_empty() || snapshot.cell_width == 0 || snapshot.cell_height == 0 {
            return Layers::default();
        }
        // The face's cell over the terminal's.
        let k = (
            face.metrics.cell_width as f32 / snapshot.cell_width as f32,
            face.metrics.cell_height as f32 / snapshot.cell_height as f32,
        );
        let mut layers = Layers::default();
        for (layer, key) in [
            (ImageLayer::BelowBackground, "images-below-background"),
            (ImageLayer::BelowText, "images-below-text"),
            (ImageLayer::AboveText, "images-above-text"),
        ] {
            let mut images = snapshot
                .images
                .iter()
                .filter(|p| p.layer == layer)
                .peekable();
            if images.peek().is_none() {
                continue;
            }
            let children: Vec<El> = images
                .map(|p| {
                    let pixels = self.pixels(p);
                    placed(p, &pixels, face, k)
                })
                .collect();
            let group = el()
                .key(key)
                .absolute(0.0, 0.0)
                .size(grid.0, grid.1)
                .clip_soft(0.0)
                .a11y_hidden()
                .inert()
                .children(children);
            match layer {
                ImageLayer::BelowBackground => layers.below_background = Some(group),
                ImageLayer::BelowText => layers.below_text = Some(group),
                ImageLayer::AboveText => layers.above_text = Some(group),
            }
        }
        layers
    }

    fn pixels(&mut self, p: &Placement) -> Pixels {
        self.pixels
            .entry(p.image.generation)
            .or_insert_with(|| {
                let pixels = Pixels::new();
                pixels.set(p.image.width, p.image.height, p.image.rgba.to_vec());
                pixels
            })
            .clone()
    }
}

/// One placement at its cell, `k` the face's cell over the terminal's.
fn placed(p: &Placement, pixels: &Pixels, face: &Face, k: (f32, f32)) -> El {
    let (cell_w, cell_h) = (
        face.metrics.cell_width as f32,
        face.metrics.cell_height as f32,
    );
    let s = face.scale;
    let x = (p.col as f32 * cell_w + p.offset_x as f32 * k.0) / s;
    let y = (p.row as f32 * cell_h + p.offset_y as f32 * k.1) / s;
    let (w, h) = (p.width as f32 * k.0 / s, p.height as f32 * k.1 / s);
    let image = el().absolute(x, y).size(w, h);
    let src = p.source;
    if (src.x, src.y, src.width, src.height) == (0, 0, p.image.width, p.image.height) {
        return image.pixels(pixels);
    }
    // Part of the image: the whole image scaled so that part fills the box, and the box clips
    // the rest.
    let (fx, fy) = (w / src.width as f32, h / src.height as f32);
    image.clip_soft(0.0).pixels_wrap(
        pixels,
        [
            -(src.x as f32) * fx,
            -(src.y as f32) * fy,
            p.image.width as f32 * fx,
            p.image.height as f32 * fy,
        ],
        PixelsWrap::Extend,
    )
}
