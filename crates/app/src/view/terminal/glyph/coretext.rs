//! Font fallback and rasterizing with CoreText, as Ghostty's macOS face does
//! (`ghostty:src/font/face/coretext.zig` `renderGlyph`, `font/Collection.zig` `scaleFactor`, at
//! `v1.4.0`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFData, CFRange, CFRetained, CFString, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, kCGColorSpaceLinearGray, kCGColorSpaceSRGB,
};
use objc2_core_text::{
    CTFont, CTFontManagerCreateFontDescriptorFromData, CTFontOrientation, CTFontSymbolicTraits,
};
use terminal::Metrics;

use super::constraint::{self, Constraint, GlyphBox};
use super::{Presentation, SYMBOLS_NERD_FONT};
use crate::view::terminal::font::Face;

/// A glyph's pixels and place, as `renderGlyph` returns them.
pub(super) struct Rendered {
    pub x: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub color: bool,
    /// RGBA, straight alpha, for a color glyph; an alpha mask otherwise.
    pub data: Vec<u8>,
}

/// The faces one terminal font falls back to, at its size.
struct Faces {
    nerd: CFRetained<CTFont>,
    emoji: Option<CFRetained<CTFont>>,
    /// The terminal font, which CoreText's fallback is asked from.
    primary: CFRetained<CTFont>,
    /// Fallback faces CoreText found, resized to match the terminal font, in the order they were
    /// found, with their PostScript names.
    fallbacks: Vec<(String, CFRetained<CTFont>)>,
}

thread_local! {
    static FACES: RefCell<HashMap<(usize, u32, u32), Faces>> = RefCell::new(HashMap::new());
}

/// A font from a font file's bytes at `px` pixels.
fn from_data(data: &[u8], px: f64) -> Option<CFRetained<CTFont>> {
    let data = CFData::from_bytes(data);
    // SAFETY: CoreText calls on live objects.
    unsafe {
        let descriptor = CTFontManagerCreateFontDescriptorFromData(&data)?;
        Some(CTFont::with_font_descriptor(
            &descriptor,
            px,
            std::ptr::null(),
        ))
    }
}

/// The glyph `font` has for `c`, if any.
fn glyph(font: &CTFont, c: char) -> Option<u16> {
    let mut units = [0u16; 2];
    let encoded = c.encode_utf16(&mut units);
    let n = encoded.len();
    let mut glyphs = [0u16; 2];
    // SAFETY: both buffers hold `n` (1 or 2) elements.
    let mapped = unsafe {
        font.glyphs_for_characters(
            NonNull::new_unchecked(units.as_mut_ptr()),
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            n as isize,
        )
    };
    (mapped && glyphs[0] != 0).then_some(glyphs[0])
}

fn is_color(font: &CTFont) -> bool {
    // SAFETY: a CoreText getter on a live font.
    unsafe { font.symbolic_traits() }.contains(CTFontSymbolicTraits::TraitColorGlyphs)
}

impl Faces {
    fn new(face: &Face) -> Option<Faces> {
        let px = face.pixels();
        let primary = from_data(face.regular_data(), px)?;
        let nerd = from_data(SYMBOLS_NERD_FONT, px)?;
        let name = CFString::from_static_str("AppleColorEmoji");
        // SAFETY: a CoreText constructor.
        let emoji = unsafe { CTFont::with_name(&name, px, std::ptr::null()) };
        let emoji = is_color(&emoji).then_some(emoji);
        Some(Faces {
            nerd,
            emoji,
            primary,
            fallbacks: Vec::new(),
        })
    }

    /// `CodepointResolver.getIndex`, after the terminal font itself: the face for `c` in
    /// `presentation`.
    fn resolve(
        &mut self,
        face: &Face,
        c: char,
        presentation: Presentation,
        wide: bool,
    ) -> Option<(CFRetained<CTFont>, u16)> {
        let emoji_has = self.emoji.as_ref().and_then(|e| glyph(e, c));
        let emoji_wanted = match presentation {
            Presentation::Emoji => true,
            Presentation::Text => false,
            Presentation::Default => wide && emoji_has.is_some(),
        };
        if !emoji_wanted && let Some(g) = glyph(&self.nerd, c) {
            return Some((self.nerd.clone(), g));
        }
        if emoji_wanted && let (Some(e), Some(g)) = (&self.emoji, emoji_has) {
            return Some((e.clone(), g));
        }
        // Faces already loaded come before asking CoreText again (`Collection.getIndex`).
        if !emoji_wanted
            && let Some(found) = self
                .fallbacks
                .iter()
                .filter(|(_, f)| !is_color(f))
                .find_map(|(_, f)| glyph(f, c).map(|g| (f.clone(), g)))
        {
            return Some(found);
        }
        if let Some(found) = self.discover(face, c, emoji_wanted) {
            return Some(found);
        }
        // Any presentation.
        if let (Some(e), Some(g)) = (&self.emoji, emoji_has) {
            return Some((e.clone(), g));
        }
        glyph(&self.nerd, c).map(|g| (self.nerd.clone(), g))
    }

    /// CoreText's fallback for `c`, resized so its ideograph width matches the terminal font's.
    fn discover(&mut self, face: &Face, c: char, emoji: bool) -> Option<(CFRetained<CTFont>, u16)> {
        let text = CFString::from_str(&c.to_string());
        let len = c.len_utf16() as isize;
        // SAFETY: CoreText calls on live objects; the range is the whole string.
        let found = unsafe { self.primary.for_string(&text, CFRange::new(0, len)) };
        glyph(&found, c)?;
        if !emoji && is_color(&found) {
            // A color font for a text request is left for the any-presentation pass.
            return None;
        }
        // SAFETY: CoreText getters on a live font.
        let name = unsafe { found.post_script_name() }.to_string();
        let font = match self.fallbacks.iter().find(|(n, _)| *n == name) {
            Some((_, f)) => f.clone(),
            None => {
                let scale = if is_color(&found) {
                    1.0
                } else {
                    scale_factor(face, &found)
                };
                // SAFETY: a CoreText constructor from a live font.
                let resized = unsafe {
                    found.copy_with_attributes(face.pixels() * scale, std::ptr::null(), None)
                };
                self.fallbacks.push((name, resized.clone()));
                resized
            }
        };
        let g = glyph(&font, c)?;
        Some((font, g))
    }
}

/// `Collection.scaleFactor` with `ic_width`: the ratio of the terminal font's ideograph width
/// to `font`'s, each in ems, falling back to the ex height, the cap height and the line height
/// when `font` doesn't have the one before.
fn scale_factor(face: &Face, font: &CTFont) -> f64 {
    let p = &face.face_metrics;
    let p_em = 1.0 / p.px_per_em;
    // SAFETY: CoreText getters on a live font.
    let (size, x_height, cap_height, line) = unsafe {
        (
            font.size(),
            font.x_height(),
            font.cap_height(),
            font.ascent() + font.descent() + font.leading(),
        )
    };
    let f_em = 1.0 / size;
    let ic = glyph(font, '水').map(|g| {
        let mut glyphs = [g];
        let mut advance = [CGSize {
            width: 0.0,
            height: 0.0,
        }];
        // SAFETY: one glyph, one advance.
        unsafe {
            font.advances_for_glyphs(
                CTFontOrientation::Horizontal,
                NonNull::new_unchecked(glyphs.as_mut_ptr()),
                advance.as_mut_ptr(),
                1,
            );
        }
        advance[0].width
    });
    let (primary, own) = match ic {
        Some(ic) if ic > 0.0 => (p.ic_width() * p_em, ic * f_em),
        _ if x_height > 0.0 => (p.ex_height() * p_em, x_height * f_em),
        _ if cap_height > 0.0 => (p.cap_height() * p_em, cap_height * f_em),
        _ => (p.line_height() * p_em, line * f_em),
    };
    if own > 0.0 { primary / own } else { 1.0 }
}

/// The constraint Ghostty gives this glyph: every emoji covers its cells; anything else takes
/// its Nerd Font rule, `fit` if it's another symbol, or none.
fn constraint_for(c: char, color: bool) -> Constraint {
    if color {
        Constraint::EMOJI
    } else {
        constraint::for_text(c as u32)
    }
}

/// Resolves and rasterizes `c` for a cell.
pub(super) fn render(
    face: &Face,
    c: char,
    presentation: Presentation,
    wide: bool,
    constraint_width: u32,
) -> Option<Rendered> {
    FACES.with(|faces| {
        let mut faces = faces.borrow_mut();
        let faces = match faces.entry(face.key()) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => e.insert(Faces::new(face)?),
        };
        let (font, g) = faces.resolve(face, c, presentation, wide)?;
        let color = is_color(&font);
        rasterize(
            &font,
            g,
            color,
            constraint_for(c, color),
            &face.metrics,
            constraint_width,
        )
    })
}

/// `renderGlyph`: the glyph's box, constrained and centered in the cell, drawn at its subpixel
/// position into a bitmap just big enough for it.
fn rasterize(
    font: &CTFont,
    glyph: u16,
    color: bool,
    constraint: Constraint,
    m: &Metrics,
    constraint_width: u32,
) -> Option<Rendered> {
    let mut glyphs = [glyph];
    // SAFETY: one glyph.
    let rect: CGRect = unsafe {
        font.bounding_rects_for_glyphs(
            CTFontOrientation::Horizontal,
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            std::ptr::null_mut(),
            1,
        )
    };
    if rect.size.width < 0.25 || rect.size.height < 0.25 {
        return None;
    }
    let cell_w = f64::from(m.cell_width);
    let cell_h = f64::from(m.cell_height);
    let sized = constraint.constrain(
        GlyphBox {
            width: rect.size.width,
            height: rect.size.height,
            x: rect.origin.x,
            y: rect.origin.y + f64::from(m.cell_baseline),
        },
        m,
        constraint_width,
    );
    let (mut x, mut y, mut width, mut height) = (sized.x, sized.y, sized.width, sized.height);
    if constraint.size != constraint::Size::Stretch {
        let dx = (cell_w - m.face_width) / 2.0;
        x += dx;
        if dx < 0.0 {
            x -= dx.trunc();
        }
    }
    // A bitmap glyph (sbix) renders in whole pixels, so its box is quantized to them.
    if color {
        width = cell_w - (cell_w - width - x).round() - x.round();
        height = cell_h - (cell_h - height - y).round() - y.round();
        x = x.round();
        y = y.round();
    }
    let px_x = x.floor() as i32;
    let px_y = y.floor() as i32;
    let frac_x = x - x.floor();
    let frac_y = y - y.floor();
    let px_w = (width + frac_x).ceil().max(0.0) as usize;
    let px_h = (height + frac_y).ceil().max(0.0) as usize;
    if px_w == 0 || px_h == 0 {
        return None;
    }
    let depth = if color { 4 } else { 1 };
    let mut buf = vec![0u8; px_w * px_h * depth];
    // SAFETY: the bitmap context draws into `buf`, which is `px_w * px_h * depth` bytes with rows
    // of `px_w * depth`, and outlives the context; the rest are CoreGraphics calls on it.
    unsafe {
        // Ghostty draws color glyphs in Display P3 for its P3 surface; ours is sRGB, so they're
        // drawn in sRGB, which is what Ghostty's P3 colors are once shown on an sRGB capture.
        let space = CGColorSpace::with_name(Some(if color {
            kCGColorSpaceSRGB
        } else {
            kCGColorSpaceLinearGray
        }));
        // kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst, or kCGImageAlphaOnly.
        let info = if color { (2 << 12) | 2 } else { 7 };
        let data = buf.as_mut_ptr() as *mut c_void;
        let create = |space: Option<&CGColorSpace>| {
            CGBitmapContextCreate(data, px_w, px_h, 8, px_w * depth, space, info)
        };
        // Ghostty draws a mask in linear gray; CoreGraphics may want no color space for one.
        let ctx: CFRetained<CGContext> = create(space.as_deref()).or_else(|| create(None))?;
        let cg: &CGContext = &ctx;
        let ctx = Some(cg);
        let full = CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(px_w as f64, px_h as f64),
        );
        if color {
            CGContext::set_rgb_fill_color(ctx, 0.0, 0.0, 0.0, 0.0);
        } else {
            CGContext::set_gray_fill_color(ctx, 0.0, 0.0);
        }
        CGContext::fill_rect(ctx, full);
        CGContext::set_allows_font_smoothing(ctx, true);
        CGContext::set_should_smooth_fonts(ctx, false);
        CGContext::set_allows_font_subpixel_positioning(ctx, true);
        CGContext::set_should_subpixel_position_fonts(ctx, true);
        CGContext::set_allows_font_subpixel_quantization(ctx, false);
        CGContext::set_should_subpixel_quantize_fonts(ctx, false);
        CGContext::set_allows_antialiasing(ctx, true);
        CGContext::set_should_antialias(ctx, true);
        if color {
            CGContext::set_rgb_fill_color(ctx, 1.0, 1.0, 1.0, 1.0);
            CGContext::set_rgb_stroke_color(ctx, 1.0, 1.0, 1.0, 1.0);
        } else {
            CGContext::set_gray_fill_color(ctx, 1.0, 1.0);
            CGContext::set_gray_stroke_color(ctx, 1.0, 1.0);
        }
        CGContext::translate_ctm(ctx, frac_x, frac_y);
        CGContext::scale_ctm(ctx, width / rect.size.width, height / rect.size.height);
        let mut positions = [CGPoint::new(-rect.origin.x, -rect.origin.y)];
        font.draw_glyphs(
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            NonNull::new_unchecked(positions.as_mut_ptr()),
            1,
            cg,
        );
    }
    let data = if color {
        // BGRA premultiplied (little-endian ARGB) to RGBA with straight alpha.
        buf.as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                let a = p[3];
                let un = |v: u8| {
                    if a == 0 {
                        0
                    } else {
                        ((u16::from(v) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8
                    }
                };
                [un(p[2]), un(p[1]), un(p[0]), a]
            })
            .collect()
    } else {
        buf
    };
    // Ghostty's offsets: from the cell's left to the bitmap's, and from the cell's bottom to the
    // bitmap's top.
    let offset_y = px_y + px_h as i32;
    Some(Rendered {
        x: px_x,
        top: m.cell_height as i32 - offset_y,
        width: px_w as u32,
        height: px_h as u32,
        color,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::terminal::font::Font;

    fn face() -> Face {
        Font::jetbrains_mono(14.0).at(2.0).unwrap()
    }

    #[test]
    fn an_emoji_covers_its_two_cells_less_its_padding() {
        let face = face();
        let r = render(&face, '🙂', Presentation::Default, true, 2).expect("Apple Color Emoji");
        assert!(r.color);
        // Two 17 px cells, 2.5% of a face width in from each side.
        let m = face.metrics;
        assert!(r.x >= 0 && r.x <= 2, "{}", r.x);
        assert!(r.width as i32 + r.x <= 2 * m.cell_width as i32);
        assert!(r.width >= 2 * m.cell_width - 4, "{}", r.width);
        assert_eq!(r.data.len(), (r.width * r.height * 4) as usize);
    }

    #[test]
    fn a_nerd_font_icon_comes_from_the_symbols_font_as_a_mask() {
        let face = face();
        let r = render(&face, '\u{f015}', Presentation::Default, false, 1).expect("nf-fa-home");
        assert!(!r.color);
        assert!(r.data.contains(&255));
        assert!(r.width <= face.metrics.cell_width + 1, "{}", r.width);
    }

    #[test]
    fn cjk_falls_back_to_a_system_face_on_the_baseline() {
        let face = face();
        let r = render(&face, '日', Presentation::Default, true, 2).expect("a CJK fallback");
        assert!(!r.color);
        let m = face.metrics;
        // The ideograph sits inside its two cells, above the cell's bottom.
        assert!(r.top >= 0 && r.top + (r.height as i32) <= m.cell_height as i32);
        assert!(r.x >= 0 && r.x + (r.width as i32) <= 2 * m.cell_width as i32 + 1);
    }
}
