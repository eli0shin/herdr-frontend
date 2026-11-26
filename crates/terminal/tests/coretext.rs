//! Cross-check: the face metrics read with skrifa against the numbers Ghostty's macOS face gets
//! from CoreText (`ghostty:src/font/face/coretext.zig` `getMetrics`, at `vt-1.0.2`).
//!
//! Ghostty reads the `head`, `hhea`, `OS/2` and `post` tables from the font itself, so those agree
//! by construction. What CoreText computes is checked here: the pixel size, the ASCII advances,
//! the ASCII bounding box, the "水" advance, and the cap and ex heights when `OS/2` lacks them.
//! Then the cell metrics from both must be identical.

#![cfg(target_os = "macos")]

mod common;

use std::ptr::NonNull;

use objc2_core_foundation::{CFData, CGRect, CGSize};
use objc2_core_text::{CTFont, CTFontManagerCreateFontDescriptorFromData, CTFontOrientation};
use skrifa::FontRef;
use skrifa::raw::TableProvider;
use terminal::{DesiredSize, FaceMetrics, Metrics};

/// What Ghostty asks CoreText for, at one size.
#[derive(Debug)]
struct CoreText {
    px_per_em: f64,
    max_advance: f64,
    ascii_height: f64,
    ic_width: Option<f64>,
    cap_height: f64,
    x_height: f64,
}

/// Ghostty's steps: a descriptor from the data, a font at 12, a copy at `size.pixels()`.
fn coretext(data: &[u8], size: DesiredSize) -> CoreText {
    let data = CFData::from_bytes(data);
    // SAFETY: plain CoreText calls on live objects; the buffers outlive the calls and their
    // lengths match the counts passed.
    unsafe {
        let descriptor =
            CTFontManagerCreateFontDescriptorFromData(&data).expect("a font descriptor");
        let base = CTFont::with_font_descriptor(&descriptor, 12.0, std::ptr::null());
        let font = base.copy_with_attributes(f64::from(size.pixels()), std::ptr::null(), None);

        let mut chars: Vec<u16> = (32u16..127).collect();
        let mut glyphs = vec![0u16; chars.len()];
        font.glyphs_for_characters(
            NonNull::new_unchecked(chars.as_mut_ptr()),
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            chars.len() as isize,
        );
        let mut advances = vec![
            CGSize {
                width: 0.0,
                height: 0.0
            };
            glyphs.len()
        ];
        font.advances_for_glyphs(
            CTFontOrientation::Horizontal,
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            advances.as_mut_ptr(),
            glyphs.len() as isize,
        );
        let max_advance = advances.iter().fold(0.0_f64, |max, a| max.max(a.width));
        let rect: CGRect = font.bounding_rects_for_glyphs(
            CTFontOrientation::Horizontal,
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            std::ptr::null_mut(),
            glyphs.len() as isize,
        );

        let mut water = [0x6C34u16];
        let mut water_glyph = [0u16];
        let ic_width = font
            .glyphs_for_characters(
                NonNull::new_unchecked(water.as_mut_ptr()),
                NonNull::new_unchecked(water_glyph.as_mut_ptr()),
                1,
            )
            .then(|| {
                let glyph = NonNull::new_unchecked(water_glyph.as_mut_ptr());
                let advance = font.advances_for_glyphs(
                    CTFontOrientation::Horizontal,
                    glyph,
                    std::ptr::null_mut(),
                    1,
                );
                let bounds = font.bounding_rects_for_glyphs(
                    CTFontOrientation::Horizontal,
                    glyph,
                    std::ptr::null_mut(),
                    1,
                );
                (bounds.size.width <= advance).then_some(advance)
            })
            .flatten()
            .filter(|_| water_glyph[0] != 0);

        CoreText {
            px_per_em: font.size(),
            max_advance,
            ascii_height: rect.size.height,
            ic_width,
            cap_height: font.cap_height(),
            x_height: font.x_height(),
        }
    }
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

/// Every whole-pixel metric equal, and the unrounded ones within a hair.
fn assert_same_cells(ours: &Metrics, ghostty: &Metrics, at: &str) {
    let whole = |m: &Metrics| {
        [
            m.cell_width,
            m.cell_height,
            m.cell_baseline,
            m.underline_position,
            m.underline_thickness,
            m.strikethrough_position,
            m.strikethrough_thickness,
            m.overline_position as u32,
            m.overline_thickness,
            m.box_thickness,
            m.cursor_thickness,
            m.cursor_height,
        ]
    };
    assert_eq!(whole(ours), whole(ghostty), "cell metrics, {at}");
    let fractional = |m: &Metrics| {
        [
            m.icon_height,
            m.icon_height_single,
            m.face_width,
            m.face_height,
            m.face_y,
        ]
    };
    for (a, b) in fractional(ours).into_iter().zip(fractional(ghostty)) {
        assert!(close(a, b, 1e-3), "{ours:?} vs {ghostty:?}, {at}");
    }
}

#[test]
fn skrifa_matches_coretext() {
    let mut checked = 0;
    for font in common::all_fonts() {
        let Some(data) = font.load() else { continue };
        let face = FontRef::from_index(&data, font.index).unwrap();
        let os2 = face.os2().ok();
        let os2_cap = os2.as_ref().and_then(|t| t.s_cap_height()).is_some();
        let os2_ex = os2.as_ref().and_then(|t| t.sx_height()).is_some();

        for points in common::POINTS {
            for scale in common::SCALES {
                let size = DesiredSize::new(points, scale);
                let ours = FaceMetrics::from_font(&face, size).unwrap();
                let ct = coretext(&data, size);
                let at = format!("{} at {points} pt × {scale}", font.name);

                assert_eq!(ours.px_per_em, ct.px_per_em, "px_per_em, {at}");
                // CoreText scales some variable fonts' advances by 1 − 2⁻¹⁶ (Geist Mono: 599.99 of 600
                // units), far from moving a rounded cell.
                assert!(
                    close(ours.cell_width, ct.max_advance, 1e-4 * ct.max_advance),
                    "advance {} vs {}, {at}",
                    ours.cell_width,
                    ct.max_advance
                );
                let ascii = ours.ascii_height.unwrap();
                assert!(
                    close(ascii, ct.ascii_height, 1e-9),
                    "ascii height {ascii} vs {}, {at}",
                    ct.ascii_height
                );
                match (ours.ic_width, ct.ic_width) {
                    (Some(a), Some(b)) => {
                        assert!(close(a, b, 1e-4 * b), "ic width {a} vs {b}, {at}")
                    }
                    (a, b) => assert_eq!(a, b, "ic width, {at}"),
                }
                if !os2_cap {
                    let cap = ours.cap_height.unwrap();
                    assert!(
                        close(cap, ct.cap_height, 1e-9),
                        "cap height {cap} vs {}, {at}",
                        ct.cap_height
                    );
                }
                if !os2_ex {
                    let ex = ours.ex_height.unwrap();
                    assert!(
                        close(ex, ct.x_height, 1e-9),
                        "ex height {ex} vs {}, {at}",
                        ct.x_height
                    );
                }

                // Ghostty's FaceMetrics: ours with CoreText's measurements swapped in.
                let ghostty = FaceMetrics {
                    px_per_em: ct.px_per_em,
                    cell_width: ct.max_advance,
                    ascii_height: Some(ct.ascii_height),
                    ic_width: ct.ic_width,
                    cap_height: if os2_cap {
                        ours.cap_height
                    } else {
                        Some(ct.cap_height)
                    },
                    ex_height: if os2_ex {
                        ours.ex_height
                    } else {
                        Some(ct.x_height)
                    },
                    ..ours
                };
                assert_same_cells(&Metrics::calc(&ours), &Metrics::calc(&ghostty), &at);
                checked += 1;
            }
        }
    }
    // The checked-in fonts are always there.
    assert!(checked >= 2 * common::POINTS.len() * common::SCALES.len());
}
