//! Reading [`FaceMetrics`] from a font file, the way Ghostty's macOS face does.
//!
//! A port of `getMetrics` in `ghostty:src/font/face/coretext.zig` at `vt-1.0.2` (`573c4a21`).
//! Ghostty reads the `head`, `hhea`, `OS/2` and `post` tables and asks CoreText for the size,
//! advances and glyph bounds. Here skrifa reads the same tables from the font data, and the
//! advances and bounds come from `hmtx` and the outlines at the default variation location.
//! `tests/coretext.rs` checks the two agree.
//!
//! Where Ghostty falls back to CoreText's own numbers:
//! - with no `head` or `hhea` table, this port reports [`FaceError::MissingTable`];
//! - with no cap or ex height in `OS/2`, it uses CoreText's estimate (see [`coretext_height`]).

use skrifa::instance::{LocationRef, Size};
use skrifa::raw::tables::os2::SelectionFlags;
use skrifa::raw::{ReadError, TableProvider};
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::metrics::FaceMetrics;

/// Ghostty's `font.face.default_dpi`: the DPI of a 1x screen.
pub const DEFAULT_DPI: f32 = if cfg!(target_os = "macos") {
    72.0
} else {
    96.0
};

/// The size to load a face at: points and the screen's DPI, as Ghostty's `DesiredSize`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesiredSize {
    pub points: f32,
    pub xdpi: u16,
    pub ydpi: u16,
}

impl DesiredSize {
    /// The size for a screen with this content scale (2.0 on Retina), as Ghostty's `Surface`
    /// computes it: the DPI is `round(scale × default_dpi)`.
    pub fn new(points: f32, content_scale: f32) -> DesiredSize {
        let dpi = (content_scale * DEFAULT_DPI).round() as u16;
        DesiredSize {
            points,
            xdpi: dpi,
            ydpi: dpi,
        }
    }

    /// Pixels per em: `points × ydpi / 72`, in `f32` as Ghostty computes it.
    pub fn pixels(self) -> f32 {
        (self.points * f32::from(self.ydpi)) / 72.0
    }
}

/// Why a face's metrics couldn't be read.
#[derive(Debug, Clone, PartialEq)]
pub enum FaceError {
    /// The data isn't a font, or the index is out of range.
    Font(ReadError),
    /// A table Ghostty would replace with CoreText's numbers is missing or unreadable.
    MissingTable(&'static str),
}

impl std::fmt::Display for FaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FaceError::Font(err) => write!(f, "can't read the font: {err}"),
            FaceError::MissingTable(tag) => write!(f, "the font has no readable {tag:?} table"),
        }
    }
}

impl std::error::Error for FaceError {}

impl FaceMetrics {
    /// Read the metrics of face `index` in a font file (0 unless it's a collection).
    pub fn from_data(data: &[u8], index: u32, size: DesiredSize) -> Result<FaceMetrics, FaceError> {
        let font = FontRef::from_index(data, index).map_err(FaceError::Font)?;
        FaceMetrics::from_font(&font, size)
    }

    /// Read the metrics of a face at a size.
    pub fn from_font(font: &FontRef, size: DesiredSize) -> Result<FaceMetrics, FaceError> {
        let head = font.head().map_err(|_| FaceError::MissingTable("head"))?;
        let hhea = font.hhea().map_err(|_| FaceError::MissingTable("hhea"))?;
        let os2 = font.os2().ok();
        let post = font.post().ok();

        let units_per_em = f64::from(head.units_per_em());
        let px_per_em = f64::from(size.pixels());
        let px_per_unit = px_per_em / units_per_em;
        let px = |units: f64| units * px_per_unit;

        let (ascent, descent, line_gap) = {
            let hhea_metrics = (
                px(f64::from(hhea.ascender().to_i16())),
                px(f64::from(hhea.descender().to_i16())),
                px(f64::from(hhea.line_gap().to_i16())),
            );
            match &os2 {
                // No OS/2: hhea, blindly.
                None => hhea_metrics,
                Some(os2) => {
                    let typo = (
                        px(f64::from(os2.s_typo_ascender())),
                        px(f64::from(os2.s_typo_descender())),
                        px(f64::from(os2.s_typo_line_gap())),
                    );
                    if os2
                        .fs_selection()
                        .contains(SelectionFlags::USE_TYPO_METRICS)
                    {
                        typo
                    } else if hhea.ascender().to_i16() != 0 || hhea.descender().to_i16() != 0 {
                        hhea_metrics
                    } else if os2.s_typo_ascender() != 0 || os2.s_typo_descender() != 0 {
                        typo
                    } else {
                        // usWinDescent is positive downward, so it flips sign.
                        (
                            px(f64::from(os2.us_win_ascent())),
                            -px(f64::from(os2.us_win_descent())),
                            0.0,
                        )
                    }
                }
            }
        };

        // A `post` thickness of 0 is broken: the thickness is unknown, and so is the position if
        // it's 0 too.
        let (underline_position, underline_thickness) = match &post {
            None => (None, None),
            Some(post) => broken_or(
                post.underline_position().to_i16(),
                post.underline_thickness().to_i16(),
                px,
            ),
        };
        let (strikethrough_position, strikethrough_thickness) = match &os2 {
            None => (None, None),
            Some(os2) => broken_or(os2.y_strikeout_position(), os2.y_strikeout_size(), px),
        };

        let charmap = font.charmap();
        let glyphs = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        let top = |ch: char| {
            charmap
                .map(ch)
                .and_then(|gid| glyphs.bounds(gid))
                .map(|b| f64::from(b.y_max))
        };

        let os2_heights = os2
            .as_ref()
            .map(|os2| (os2.s_cap_height(), os2.sx_height()));
        let cap_height = match os2_heights {
            Some((Some(cap), _)) => Some(px(f64::from(cap))),
            _ => coretext_height(top('H'), top('O')).map(px),
        };
        let ex_height = match os2_heights {
            Some((_, Some(ex))) => Some(px(f64::from(ex))),
            _ => coretext_height(top('x'), top('o')).map(px),
        };

        // The widest printable ASCII advance (not just `M`), and the height of the box around all
        // of them. A missing character measures as glyph 0, as CoreText reports it.
        let mut max_advance: f64 = 0.0;
        let (mut ascii_top, mut ascii_bottom): (f64, f64) = (0.0, 0.0);
        for ch in ' '..='~' {
            let gid = charmap.map(ch).unwrap_or(GlyphId::NOTDEF);
            max_advance = max_advance.max(px(f64::from(glyphs.advance_width(gid).unwrap_or(0.0))));
            if let Some(b) = glyphs.bounds(gid) {
                ascii_top = ascii_top.max(px(f64::from(b.y_max)));
                ascii_bottom = ascii_bottom.min(px(f64::from(b.y_min)));
            }
        }
        let ascii_height = Some(ascii_top - ascii_bottom).filter(|h| *h > 0.0);

        // The advance of "水", unless the glyph is wider than it (a mangled Nerd Font patch).
        let ic_width = charmap
            .map('水')
            .filter(|gid| *gid != GlyphId::NOTDEF)
            .and_then(|gid| {
                let advance = px(f64::from(glyphs.advance_width(gid)?));
                let width = glyphs
                    .bounds(gid)
                    .map_or(0.0, |b| px(f64::from(b.x_max - b.x_min)));
                (width <= advance).then_some(advance)
            });

        Ok(FaceMetrics {
            px_per_em,
            cell_width: max_advance,
            ascent,
            descent,
            line_gap,
            underline_position,
            underline_thickness,
            strikethrough_position,
            strikethrough_thickness,
            cap_height,
            ex_height,
            ascii_height,
            ic_width,
        })
    }
}

/// CoreText's cap height (`H` and `O`) or ex height (`x` and `o`) for a font whose `OS/2` table
/// doesn't give one: the mean of the flat and the round letter's top, floored, in font units.
///
/// Not documented by Apple; observed on macOS 27, where it matches CoreText exactly for every font
/// on the system that lacks the `OS/2` heights and has the letters (35 of 49; the rest have no Latin
/// letters, so they'd never be a terminal's primary font). Without the letters this returns `None`
/// and Ghostty's own estimate applies.
fn coretext_height(flat: Option<f64>, round: Option<f64>) -> Option<f64> {
    Some(((flat? + round?) / 2.0).floor())
}

/// A line's position and thickness in pixels, with Ghostty's rule for degenerate tables.
fn broken_or(position: i16, thickness: i16, px: impl Fn(f64) -> f64) -> (Option<f64>, Option<f64>) {
    let broken = thickness == 0;
    let position = (!(broken && position == 0)).then(|| px(f64::from(position)));
    let thickness = (!broken).then(|| px(f64::from(thickness)));
    (position, thickness)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desired_size_pixels() {
        assert_eq!(
            DesiredSize::new(14.0, 1.0).pixels(),
            14.0 * DEFAULT_DPI / 72.0
        );
        let retina = DesiredSize::new(14.0, 2.0);
        assert_eq!(retina.ydpi, (2.0 * DEFAULT_DPI) as u16);
        assert_eq!(retina.pixels(), 2.0 * 14.0 * DEFAULT_DPI / 72.0);
        // Eli's 17 pt × zoom 1.1 stays in f32, as Ghostty's points are.
        assert_eq!(DesiredSize::new(17.0 * 1.1, 1.0).points, 18.7_f32);
    }

    #[test]
    fn broken_tables() {
        let px = |v: f64| v / 10.0;
        assert_eq!(broken_or(-100, 50, px), (Some(-10.0), Some(5.0)));
        assert_eq!(broken_or(-100, 0, px), (Some(-10.0), None));
        assert_eq!(broken_or(0, 0, px), (None, None));
    }

    #[test]
    fn coretext_height_floors_the_mean() {
        // Courier: H tops at 1186 and O at 1219; CoreText says 1202.
        assert_eq!(coretext_height(Some(1186.0), Some(1219.0)), Some(1202.0));
        assert_eq!(coretext_height(Some(924.0), Some(946.0)), Some(935.0));
        assert_eq!(coretext_height(None, Some(946.0)), None);
    }

    #[test]
    fn not_a_font() {
        let size = DesiredSize::new(12.0, 1.0);
        assert!(matches!(
            FaceMetrics::from_data(b"not a font", 0, size),
            Err(FaceError::Font(_))
        ));
    }
}
