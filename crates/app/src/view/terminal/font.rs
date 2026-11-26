//! The terminal's font: its faces registered with Rune, Ghostty's whole-pixel cell for it at a
//! display scale, and which characters its regular face draws.
//!
//! Rune lays a grid row's narrow cells out as one shaped line, so the text style gets tracking
//! that spaces glyphs exactly one cell apart, a line height of one cell, and ligatures off
//! (`docs/research/terminal-rendering.md` §2 "Font fallback and shaping").

use std::sync::Arc;

use rune::{Family, TextStyle};
use skrifa::{FontRef, MetadataProvider};
use terminal::{DesiredSize, FaceError, FaceMetrics, Metrics};

/// JetBrains Mono 2.304, the face Ghostty embeds as its default, so the pixel diff can compare
/// the two with no font installed. Its italic is a separate file.
pub const JETBRAINS_MONO: &[u8] =
    include_bytes!("../../../../../assets/fonts/JetBrainsMono-Variable.ttf");
pub const JETBRAINS_MONO_ITALIC: &[u8] =
    include_bytes!("../../../../../assets/fonts/JetBrainsMono-Italic-Variable.ttf");

/// A terminal font family: the files of its faces, registered with Rune's text system.
pub struct Font {
    /// The family name Rune resolves, from the regular face's name table.
    family: &'static str,
    /// The regular face, which the cell metrics and the character map come from.
    regular: Arc<[u8]>,
    pub points: f32,
}

impl Font {
    /// Registers the faces (regular first) in Rune's process-wide font collection and names the
    /// family the regular face belongs to. Registering the same bytes again is free.
    pub fn new(faces: &[Arc<[u8]>], points: f32) -> Result<Font, FontError> {
        let regular = faces.first().cloned().ok_or(FontError::NoFaces)?;
        FontRef::new(&regular).map_err(|err| FontError::Face(FaceError::Font(err)))?;
        let mut family = None;
        for face in faces {
            let names = rune::text::add_font(face.clone());
            family = family.or_else(|| names.into_iter().next());
        }
        let family = family.ok_or(FontError::Unregistered)?;
        Ok(Font {
            // A text style names its family with a `&'static str`; a font is made a few times
            // in the app's life, so its name is kept for good. A name with no comma is taken
            // as it is, never parsed as a CSS list, so it needs no quotes.
            family: Box::leak(family.into_boxed_str()),
            regular,
            points,
        })
    }

    /// JetBrains Mono, the fixtures' and the pixel diff's font.
    pub fn jetbrains_mono(points: f32) -> Font {
        Font::new(
            &[Arc::from(JETBRAINS_MONO), Arc::from(JETBRAINS_MONO_ITALIC)],
            points,
        )
        .expect("the bundled JetBrains Mono registers")
    }

    pub fn family(&self) -> &'static str {
        self.family
    }

    /// The font at display scale `scale`: Ghostty's cell and the style that draws on it.
    pub fn at(&self, scale: f32) -> Result<Face, FontError> {
        let face = FaceMetrics::from_data(&self.regular, 0, DesiredSize::new(self.points, scale))
            .map_err(FontError::Face)?;
        let metrics = Metrics::calc(&face);
        let font =
            FontRef::new(&self.regular).map_err(|err| FontError::Face(FaceError::Font(err)))?;
        let charmap = font.charmap();
        // Printable ASCII and Latin-1 are what nearly every cell holds; the rest is looked up.
        let mut latin = [false; 256];
        for (c, covered) in latin.iter_mut().enumerate() {
            *covered = charmap.map(c as u32).is_some();
        }
        Ok(Face {
            family: self.family,
            points: self.points,
            scale,
            metrics,
            advance_px: face.cell_width,
            latin,
            regular: self.regular.clone(),
        })
    }
}

#[derive(Debug)]
pub enum FontError {
    NoFaces,
    Face(FaceError),
    /// Rune's text system didn't take the regular face.
    Unregistered,
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::NoFaces => write!(f, "a terminal font needs at least one face"),
            FontError::Face(err) => write!(f, "{err}"),
            FontError::Unregistered => write!(f, "the text system didn't register the font"),
        }
    }
}

impl std::error::Error for FontError {}

/// A [`Font`] at one display scale: everything a grid row is laid out with.
#[derive(Clone)]
pub struct Face {
    family: &'static str,
    pub points: f32,
    pub scale: f32,
    /// Ghostty's cell, in device pixels.
    pub metrics: Metrics,
    /// The face's own advance (its widest ASCII advance), in device pixels.
    pub advance_px: f64,
    latin: [bool; 256],
    regular: Arc<[u8]>,
}

impl Face {
    /// The cell, in points: the grid's `cell_w` and `cell_h`.
    pub fn cell_pt(&self) -> (f32, f32) {
        (
            self.metrics.cell_width as f32 / self.scale,
            self.metrics.cell_height as f32 / self.scale,
        )
    }

    /// The style a row's line is shaped with: the face at its size, one cell tall, glyphs one
    /// cell apart, no ligatures.
    pub fn style(&self) -> TextStyle {
        let (cell_w, cell_h) = self.cell_pt();
        let advance = (self.advance_px / f64::from(self.scale)) as f32;
        TextStyle::new(self.points, 400.0)
            .family(Family::Named(self.family))
            .lh(cell_h)
            .tracking(cell_w - advance)
            .features("'calt' 0, 'liga' 0")
    }

    /// The regular face has a glyph for `c`. A cell it can't draw is pinned to its column, so
    /// a fallback face's advance can't move the cells after it.
    pub fn covers(&self, c: char) -> bool {
        match self.latin.get(c as usize) {
            Some(&covered) => covered,
            None => FontRef::new(&self.regular)
                .map(|font| font.charmap().map(c).is_some())
                .unwrap_or(false),
        }
    }

    /// What identifies this face's layouts in a row cache: rows built for another face, size
    /// or scale are built again.
    pub fn key(&self) -> (usize, u32, u32) {
        (
            self.family.as_ptr() as usize,
            self.points.to_bits(),
            self.scale.to_bits(),
        )
    }
}
