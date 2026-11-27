//! The terminal's font: its faces registered with Rune, Ghostty's whole-pixel cell for it at a
//! display scale, and which characters its regular face draws.
//!
//! Rune lays a grid row's narrow cells out as one shaped line, so the text style gets tracking
//! that spaces glyphs exactly one cell apart, a line height of one cell, and ligatures off
//! (`docs/research/terminal-rendering.md` §2 "Font fallback and shaping").
//!
//! A font other than JetBrains Mono keeps Ghostty's fallback order: its own faces, then the
//! JetBrains Mono Ghostty embeds as its backup (`ghostty:src/font/SharedGridSet.zig:260-316`),
//! then what the system finds. The text style names both families, so Rune tries them in that
//! order before its own fallback.

use std::sync::Arc;

use rune::{Family, TextStyle};
use skrifa::{FontRef, MetadataProvider};
use terminal::{DesiredSize, FaceError, FaceMetrics, Metrics, Modifier, Modifiers};

use crate::config::fonts::FamilyFiles;

/// JetBrains Mono's family name, as its file names it.
pub const JETBRAINS_MONO_FAMILY: &str = "JetBrains Mono";

/// JetBrains Mono 2.304, the face Ghostty embeds as its default, so the pixel diff can compare
/// the two with no font installed. Its italic is a separate file.
pub const JETBRAINS_MONO: &[u8] =
    include_bytes!("../../../../../assets/fonts/JetBrainsMono-Variable.ttf");
pub const JETBRAINS_MONO_ITALIC: &[u8] =
    include_bytes!("../../../../../assets/fonts/JetBrainsMono-Italic-Variable.ttf");

/// A terminal font family: the files of its faces, registered with Rune's text system.
#[derive(Clone)]
pub struct Font {
    /// The family name Rune resolves, from the regular face's name table.
    family: &'static str,
    /// What the text style names: the family, then its backups.
    families: &'static str,
    /// Whether the bundled JetBrains Mono backs the family up (any family but itself).
    backup: bool,
    /// The regular face, which the cell metrics and the character map come from, and its index
    /// in a collection.
    regular: Arc<[u8]>,
    index: u32,
    pub points: f32,
    /// Ghostty's `adjust-cell-height` and `adjust-cell-width` (`terminal_line_height`,
    /// `terminal_cell_width`).
    pub modifiers: Modifiers,
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
        // A text style names its family with a `&'static str`; a font is made a few times in
        // the app's life (at start and when the settings change it), so its name is kept for
        // good. A name with no comma is taken as it is, never parsed as a CSS list, so it needs
        // no quotes.
        let family: &'static str = Box::leak(family.into_boxed_str());
        Ok(Font {
            family,
            families: family,
            backup: false,
            regular,
            index: 0,
            points,
            modifiers: Modifiers::default(),
        })
    }

    /// A family found in the font folders, with the bundled JetBrains Mono behind it, as
    /// Ghostty's fallback order has it.
    pub fn from_files(files: &FamilyFiles, points: f32) -> Result<Font, FontError> {
        let backup = Font::jetbrains_mono(points);
        let regular = files.faces.first().ok_or(FontError::NoFaces)?;
        FontRef::from_index(&regular.bytes, regular.index)
            .map_err(|err| FontError::Face(FaceError::Font(err)))?;
        let mut family = None;
        let mut registered: Vec<*const u8> = Vec::new();
        for face in &files.faces {
            // A collection's faces share one file: it registers once.
            if registered.contains(&face.bytes.as_ptr()) {
                continue;
            }
            registered.push(face.bytes.as_ptr());
            let names = rune::text::add_font(face.bytes.clone());
            if family.is_none() {
                family = names
                    .iter()
                    .find(|n| n.eq_ignore_ascii_case(&files.name))
                    .or(names.first())
                    .cloned();
            }
        }
        let family = family.ok_or(FontError::Unregistered)?;
        if family.eq_ignore_ascii_case(JETBRAINS_MONO_FAMILY) {
            return Ok(backup);
        }
        let quote = |name: &str| format!("\"{}\"", name.replace('"', ""));
        let families = format!("{}, {}", quote(&family), quote(backup.family));
        Ok(Font {
            family: Box::leak(family.into_boxed_str()),
            families: Box::leak(families.into_boxed_str()),
            backup: true,
            regular: regular.bytes.clone(),
            index: regular.index,
            points,
            modifiers: Modifiers::default(),
        })
    }

    /// The font with rows `line_height` times as tall and columns `cell_width` times as wide
    /// (1 leaves them as the face makes them).
    pub fn adjusted(mut self, line_height: f32, cell_width: f32) -> Font {
        let factor = |v: f32| (v != 1.0).then(|| Modifier::Percent(f64::from(v)));
        self.modifiers.cell_height = factor(line_height);
        self.modifiers.cell_width = factor(cell_width);
        self
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

    /// The families the text style names, the font's own first.
    pub fn families(&self) -> &'static str {
        self.families
    }

    /// The font at display scale `scale`: Ghostty's cell and the style that draws on it.
    pub fn at(&self, scale: f32) -> Result<Face, FontError> {
        let face = FaceMetrics::from_data(
            &self.regular,
            self.index,
            DesiredSize::new(self.points, scale),
        )
        .map_err(FontError::Face)?;
        let metrics = Metrics::with_modifiers(&face, &self.modifiers);
        let font = FontRef::from_index(&self.regular, self.index)
            .map_err(|err| FontError::Face(FaceError::Font(err)))?;
        let charmap = font.charmap();
        // Printable ASCII and Latin-1 are what nearly every cell holds; the rest is looked up.
        let mut latin = [false; 256];
        for (c, covered) in latin.iter_mut().enumerate() {
            *covered = charmap.map(c as u32).is_some();
        }
        Ok(Face {
            family: self.families,
            backup: self.backup,
            index: self.index,
            points: self.points,
            scale,
            metrics,
            advance_px: face.cell_width,
            face_metrics: face,
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

/// [`Face::key`]: the families, size, scale and cell.
pub type FaceKey = (usize, u32, u32, u32, u32);

/// A [`Font`] at one display scale: everything a grid row is laid out with.
#[derive(Clone)]
pub struct Face {
    /// The families the style names.
    family: &'static str,
    backup: bool,
    index: u32,
    pub points: f32,
    pub scale: f32,
    /// Ghostty's cell, in device pixels.
    pub metrics: Metrics,
    /// The face's own advance (its widest ASCII advance), in device pixels.
    pub advance_px: f64,
    /// The regular face's metrics at this size, which fallback faces are scaled to match.
    pub face_metrics: FaceMetrics,
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
            None => FontRef::from_index(&self.regular, self.index)
                .map(|font| font.charmap().map(c).is_some())
                .unwrap_or(false),
        }
    }

    /// The backup face's file, JetBrains Mono's, when the font has one: the glyph atlas tries it
    /// before the Symbols Nerd Font, as Ghostty's collection does.
    pub fn backup(&self) -> Option<&'static [u8]> {
        self.backup.then_some(JETBRAINS_MONO)
    }

    /// The regular face's font file.
    pub fn regular_data(&self) -> &Arc<[u8]> {
        &self.regular
    }

    /// The regular face's index in its file, which is a collection when it holds several.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The face's size in device pixels (Ghostty's `points × dpi / 72`).
    pub fn pixels(&self) -> f64 {
        f64::from(DesiredSize::new(self.points, self.scale).pixels())
    }

    /// What identifies this face's layouts in a row cache: rows built for another face, size,
    /// scale or cell are built again.
    pub fn key(&self) -> FaceKey {
        (
            self.family.as_ptr() as usize,
            self.points.to_bits(),
            self.scale.to_bits(),
            self.metrics.cell_width,
            self.metrics.cell_height,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::config::fonts;

    #[test]
    fn another_family_keeps_jetbrains_mono_as_its_backup() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../terminal/tests/fonts");
        let files = fonts::find("Geist Mono", &[dir]).expect("the test font");
        let font = Font::from_files(&files, 17.0).unwrap();
        assert_eq!(font.family(), "Geist Mono");
        assert_eq!(font.families(), "\"Geist Mono\", \"JetBrains Mono\"");
        assert_eq!(font.points, 17.0);
        // JetBrains Mono itself needs no backup.
        assert_eq!(Font::jetbrains_mono(14.0).families(), JETBRAINS_MONO_FAMILY);
    }

    #[test]
    fn line_height_and_cell_width_scale_the_cell() {
        let plain = Font::jetbrains_mono(14.0).at(2.0).unwrap().metrics;
        let same = Font::jetbrains_mono(14.0)
            .adjusted(1.0, 1.0)
            .at(2.0)
            .unwrap()
            .metrics;
        assert_eq!(same, plain, "1 leaves the cell as the face makes it");
        let face = Font::jetbrains_mono(14.0)
            .adjusted(1.5, 1.2)
            .at(2.0)
            .unwrap();
        let m = face.metrics;
        assert_eq!(
            m.cell_height,
            (f64::from(plain.cell_height) * 1.5).round() as u32
        );
        assert_eq!(
            m.cell_width,
            (f64::from(plain.cell_width) * f64::from(1.2f32)).round() as u32
        );
        assert_ne!(
            face.key(),
            Font::jetbrains_mono(14.0).at(2.0).unwrap().key()
        );
    }
}
