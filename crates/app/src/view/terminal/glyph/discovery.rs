//! Finding a fallback face for a character as Ghostty's CoreText discovery does
//! (`ghostty:src/font/discovery.zig` `CoreText.discoverFallback`, at `v1.4.0`):
//!
//! - **Han ideographs** (U+4E00 to U+9FFF) take CoreText's own fallback from the terminal font
//!   (`CTFontCreateForString`), which follows the system's language.
//! - **Everything else** takes the fonts CoreText matches for a descriptor holding just the
//!   character, best first by Ghostty's score. Only when none match does it take CoreText's
//!   fallback.
//!
//! The two pick different faces: for kana CoreText's fallback is Hiragino Sans, while Ghostty's
//! score picks the face with the most glyphs (Arial Unicode MS on a stock macOS).

use objc2_core_foundation::{
    CFArray, CFCharacterSet, CFData, CFDictionary, CFNumber, CFRange, CFRetained, CFString, CFType,
};
use objc2_core_text::{
    CTFont, CTFontCollection, CTFontDescriptor, CTFontSymbolicTraits, CTFontTableOptions,
    kCTFontCharacterSetAttribute, kCTFontSizeAttribute, kCTFontStyleNameAttribute,
    kCTFontSymbolicTrait, kCTFontTableHead, kCTFontTableOS2, kCTFontTraitsAttribute,
    kCTFontVariationAttribute, kCTFontVariationAxesAttribute, kCTFontVariationAxisDefaultValueKey,
    kCTFontVariationAxisIdentifierKey, kCTFontVariationAxisNameKey,
};

/// Ghostty asks CoreText's own fallback for these (`discoverFallback`): the CJK Unified
/// Ideographs block.
fn is_han(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c)
}

/// The faces to try for `c`, best first, at an arbitrary size: Ghostty's `discoverFallback` for a
/// regular-style codepoint. The caller still checks each for the glyph and its presentation.
pub(super) fn candidates(primary: &CTFont, c: char, points: f64) -> Vec<CFRetained<CTFont>> {
    if !is_han(c) {
        let matched = matching(c, points);
        if !matched.is_empty() {
            return matched;
        }
    }
    for_string(primary, c).into_iter().collect()
}

/// `discoverCodepoint`: CoreText's fallback for `c` from `primary`, but never its LastResort face.
fn for_string(primary: &CTFont, c: char) -> Option<CFRetained<CTFont>> {
    let text = CFString::from_str(&c.to_string());
    let len = c.len_utf16() as isize;
    // SAFETY: CoreText calls on live objects; the range is the whole string.
    let found = unsafe { primary.for_string(&text, CFRange::new(0, len)) };
    // SAFETY: a CoreText getter on a live font.
    let name = unsafe { found.post_script_name() }.to_string();
    (name != "LastResort").then_some(found)
}

/// `CoreText.discover` for a descriptor of just `c` at `points`: every font CoreText matches,
/// sorted by [`score`], best first.
fn matching(c: char, points: f64) -> Vec<CFRetained<CTFont>> {
    // SAFETY: CoreFoundation and CoreText constructors and getters on live objects, with the
    // attribute keys their documentation gives and values of the types it gives.
    unsafe {
        let Some(set) = CFCharacterSet::with_characters_in_range(None, CFRange::new(c as isize, 1))
        else {
            return Vec::new();
        };
        // Ghostty rounds the size to a whole number of points.
        let size = CFNumber::new_i32(points.round() as i32);
        let keys: [&CFString; 2] = [kCTFontCharacterSetAttribute, kCTFontSizeAttribute];
        let values: [&CFType; 2] = [&set, &size];
        let attributes = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
        let descriptor = CTFontDescriptor::with_attributes(attributes.as_opaque());
        let query = CFArray::<CTFontDescriptor>::from_objects(&[&*descriptor]);
        let collection = CTFontCollection::with_font_descriptors(Some(query.as_opaque()), None);
        let Some(list) = collection.matching_font_descriptors() else {
            return Vec::new();
        };
        let list = CFRetained::cast_unchecked::<CFArray<CTFontDescriptor>>(list);
        let mut scored: Vec<(u32, CFRetained<CTFont>)> = list
            .iter()
            .map(|d| {
                let font = CTFont::with_font_descriptor(&d, 12.0, std::ptr::null());
                (score(&d, &font, c), font)
            })
            .collect();
        // Ghostty's sort isn't stable; a stable one keeps CoreText's order among equals.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.into_iter().map(|(_, f)| f).collect()
    }
}

/// `Score` for a regular-style request for `c`, as one integer: whether the font has `c`, then
/// whether it's monospace, matches the style name exactly, isn't italic, isn't bold, how closely
/// its style name matches, and finally how many glyphs it has (`discovery.zig` `Score`).
fn score(descriptor: &CTFontDescriptor, font: &CTFont, c: char) -> u32 {
    // SAFETY: CoreText getters on live objects.
    let (glyphs, traits, style) = unsafe {
        (
            font.glyph_count(),
            descriptor.attribute(kCTFontTraitsAttribute),
            descriptor.attribute(kCTFontStyleNameAttribute),
        )
    };
    let glyph_count = u32::try_from(glyphs).unwrap_or(0).min(u32::from(u16::MAX));
    let has = super::coretext::glyph(font, c).is_some();
    let symbolic = traits
        .and_then(|t| t.downcast::<CFDictionary>().ok())
        .and_then(|t| {
            // SAFETY: the traits attribute is a dictionary of CFString keys.
            let t = unsafe { CFRetained::cast_unchecked::<CFDictionary<CFString, CFType>>(t) };
            t.get(unsafe { kCTFontSymbolicTrait })
        })
        .and_then(|n| n.downcast::<CFNumber>().ok())
        .and_then(|n| n.as_i64())
        .map_or(CTFontSymbolicTraits(0), |n| CTFontSymbolicTraits(n as u32));
    let monospace = symbolic.contains(CTFontSymbolicTraits::TraitMonoSpace);
    let (bold, italic) = bold_italic(font, symbolic);
    let style = style
        .and_then(|s| s.downcast::<CFString>().ok())
        .map(|s| s.to_string().to_lowercase())
        .unwrap_or_default();
    let exact = style == "regular";
    // Fewer characters outside the desired style names score higher.
    let mut fuzzy = style.len();
    for s in ["regular", "upright"] {
        if style.contains(s) {
            fuzzy = fuzzy.saturating_sub(s.len());
        }
    }
    let fuzzy = 255u32.saturating_sub(u32::try_from(fuzzy).unwrap_or(u32::MAX));
    glyph_count
        | fuzzy << 16
        | u32::from(!bold) << 24
        | u32::from(!italic) << 25
        | u32::from(exact) << 26
        | u32::from(monospace) << 27
        | u32::from(has) << 28
}

/// Whether `font` is bold and italic, from its symbolic traits, its `head` and `OS/2` tables and
/// its `wght`, `ital` and `slnt` variation axes, as Ghostty's `Score.score` derives them.
fn bold_italic(font: &CTFont, symbolic: CTFontSymbolicTraits) -> (bool, bool) {
    let mut bold = symbolic.contains(CTFontSymbolicTraits::TraitBold);
    let mut italic = symbolic.contains(CTFontSymbolicTraits::TraitItalic);
    // SAFETY: a CoreText getter on a live font.
    let table = |tag| unsafe { font.table(tag, CTFontTableOptions(0)) }.map(|d| CFData::to_vec(&d));
    let be16 = |t: &[u8], at: usize| t.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    // `head.macStyle`: bit 0 bold, bit 1 italic.
    if let Some(mac_style) = table(kCTFontTableHead).and_then(|t| be16(&t, 44)) {
        bold |= mac_style & 1 != 0;
        italic |= mac_style & 2 != 0;
    }
    // `OS/2.fsSelection`: bit 0 italic, bit 5 bold.
    if let Some(selection) = table(kCTFontTableOS2).and_then(|t| be16(&t, 62)) {
        italic |= selection & 1 != 0;
        bold |= selection & (1 << 5) != 0;
    }
    // SAFETY: CoreText getters on a live font; the values are of the types CoreText documents.
    unsafe {
        let axes = font
            .attribute(kCTFontVariationAxesAttribute)
            .and_then(|a| a.downcast::<CFArray>().ok());
        let values = font
            .attribute(kCTFontVariationAttribute)
            .and_then(|v| v.downcast::<CFDictionary>().ok());
        if let (Some(axes), Some(values)) = (axes, values) {
            let axes = CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(axes);
            let values = CFRetained::cast_unchecked::<CFDictionary<CFNumber, CFNumber>>(values);
            let mut ital_seen = false;
            for axis in axes.iter() {
                let number =
                    |key: &CFString| axis.get(key).and_then(|n| n.downcast::<CFNumber>().ok());
                let name = axis
                    .get(kCTFontVariationAxisNameKey)
                    .and_then(|n| n.downcast::<CFString>().ok())
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                let default = number(kCTFontVariationAxisDefaultValueKey)
                    .and_then(|n| n.as_f64())
                    .unwrap_or(0.0);
                let value = number(kCTFontVariationAxisIdentifierKey)
                    .and_then(|id| values.get(&id))
                    .and_then(|n| n.as_f64())
                    .unwrap_or(default);
                match name.as_str() {
                    "wght" => bold = value > 600.0,
                    "ital" => {
                        italic = value > 0.5;
                        ital_seen = true;
                    }
                    "slnt" if !ital_seen => italic = value <= -5.0,
                    _ => {}
                }
            }
        }
    }
    (bold, italic)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primary() -> CFRetained<CTFont> {
        let name = CFString::from_static_str("Menlo-Regular");
        // SAFETY: a CoreText constructor.
        unsafe { CTFont::with_name(&name, 14.0, std::ptr::null()) }
    }

    #[test]
    fn the_first_candidate_has_the_character() {
        for c in ['の', '한', 'Ω', '日'] {
            let found = candidates(&primary(), c, 14.0);
            let first = found.first().unwrap_or_else(|| panic!("no face for {c}"));
            assert!(super::super::coretext::glyph(first, c).is_some(), "{c}");
        }
    }

    #[test]
    fn han_takes_coretexts_fallback_alone() {
        assert_eq!(candidates(&primary(), '日', 14.0).len(), 1);
    }

    #[test]
    fn a_regular_face_outscores_a_bold_one_of_the_same_font() {
        let regular = CFString::from_static_str("Menlo-Regular");
        let bold = CFString::from_static_str("Menlo-Bold");
        // SAFETY: CoreText constructors and getters.
        let (r, b) = unsafe {
            let r = CTFont::with_name(&regular, 12.0, std::ptr::null());
            let b = CTFont::with_name(&bold, 12.0, std::ptr::null());
            (
                score(&r.font_descriptor(), &r, 'a'),
                score(&b.font_descriptor(), &b, 'a'),
            )
        };
        assert!(r > b, "{r:x} {b:x}");
    }
}
