//! A face rewritten as a standalone font file in a private family, tagged with the style it
//! draws: what Rune's text system is given for the terminal font ([`super::Family`]).
//!
//! Rune can't name a face for a span: a span asks its family for a weight and a slant, and
//! fontique picks the face whose `OS/2` weight class and `fsSelection` come closest. Operator Mono
//! tags its faces in a way that picks wrong: Bold has weight class 400 and the REGULAR bit, Book
//! has 325, so a 400 request draws Bold. So each face Ghostty's discovery chose is copied into a
//! family of its own name with exactly the tags of its style: 400 or 700, upright or italic. Only
//! the `name` table, the `OS/2` weight class and `fsSelection` style bits, and `head.macStyle`
//! change; every other table, outlines and metrics included, is the face's own. Nothing is
//! written to disk.

use skrifa::raw::FontRef;
use skrifa::string::StringId;
use skrifa::{MetadataProvider, Tag};

/// Face `index` of `data` as a font of its own in `family`, tagged `bold` and `italic`. `None`
/// when `data` isn't a font or the face is out of range.
pub fn private_face(
    data: &[u8],
    index: u32,
    family: &str,
    bold: bool,
    italic: bool,
) -> Option<Vec<u8>> {
    let font = FontRef::from_index(data, index).ok()?;
    let postscript = font
        .localized_strings(StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "HerdrFrontendTerminal".into());
    let subfamily = match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    };
    let mut tables = Vec::new();
    for record in font.table_directory().table_records() {
        let tag = record.tag();
        let mut bytes = font.table_data(tag)?.as_bytes().to_vec();
        match &tag.to_be_bytes() {
            b"name" => continue,
            b"OS/2" if bytes.len() >= 64 => {
                let weight: u16 = if bold { 700 } else { 400 };
                bytes[4..6].copy_from_slice(&weight.to_be_bytes());
                // fsSelection: ITALIC (bit 0), BOLD (5) and REGULAR (6) say the style; OBLIQUE
                // (9) is cleared so italic reads as italic. The rest (USE_TYPO_METRICS) stays.
                let mut selection = u16::from_be_bytes([bytes[62], bytes[63]]);
                selection &= !(1 | 1 << 5 | 1 << 6 | 1 << 9);
                selection |= match (bold, italic) {
                    (false, false) => 1 << 6,
                    _ => u16::from(italic) | u16::from(bold) << 5,
                };
                bytes[62..64].copy_from_slice(&selection.to_be_bytes());
            }
            b"head" if bytes.len() >= 54 => {
                // checkSumAdjustment is set once the file is whole.
                bytes[8..12].fill(0);
                let mut style = u16::from_be_bytes([bytes[44], bytes[45]]) & !3;
                style |= u16::from(bold) | u16::from(italic) << 1;
                bytes[44..46].copy_from_slice(&style.to_be_bytes());
            }
            _ => {}
        }
        tables.push((tag, bytes));
    }
    let full = format!("{family} {subfamily}");
    tables.push((
        Tag::new(b"name"),
        name_table(&[
            (StringId::FAMILY_NAME, family),
            (StringId::SUBFAMILY_NAME, subfamily),
            (StringId::FULL_NAME, &full),
            (StringId::POSTSCRIPT_NAME, &postscript),
            (StringId::TYPOGRAPHIC_FAMILY_NAME, family),
            (StringId::TYPOGRAPHIC_SUBFAMILY_NAME, subfamily),
        ]),
    ));
    Some(write(font.table_directory().sfnt_version(), tables))
}

/// A format 0 `name` table with `names` as Windows Unicode English strings.
fn name_table(names: &[(StringId, &str)]) -> Vec<u8> {
    let count = names.len() as u16;
    let storage = 6 + 12 * count;
    let mut records = Vec::new();
    let mut strings: Vec<u8> = Vec::new();
    for (id, text) in names {
        let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        // platform 3 (Windows), encoding 1 (Unicode BMP), language 0x409 (English, US).
        for v in [
            3,
            1,
            0x409,
            id.to_u16(),
            utf16.len() as u16,
            strings.len() as u16,
        ] {
            records.extend_from_slice(&u16::to_be_bytes(v));
        }
        strings.extend_from_slice(&utf16);
    }
    let mut out = Vec::new();
    for v in [0, count, storage] {
        out.extend_from_slice(&u16::to_be_bytes(v));
    }
    out.extend_from_slice(&records);
    out.extend_from_slice(&strings);
    out
}

/// The sum of `data` as big-endian 32-bit words, zero-padded to a whole word.
fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, word| {
        let mut w = [0u8; 4];
        w[..word.len()].copy_from_slice(word);
        sum.wrapping_add(u32::from_be_bytes(w))
    })
}

/// An sfnt file of `tables`: the table directory sorted by tag, each table on a 4-byte boundary,
/// with its checksum, and `head`'s checkSumAdjustment for the whole file.
fn write(sfnt_version: u32, mut tables: Vec<(Tag, Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| *tag);
    let n = tables.len() as u16;
    let entry_selector = (u16::BITS - 1).saturating_sub(n.leading_zeros()) as u16;
    let search_range = (1u16 << entry_selector) * 16;
    let mut out = Vec::new();
    out.extend_from_slice(&sfnt_version.to_be_bytes());
    for v in [n, search_range, entry_selector, n * 16 - search_range] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + 16 * u32::from(n);
    let mut head_at = None;
    let mut data = Vec::new();
    for (tag, bytes) in &tables {
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&checksum(bytes).to_be_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        if tag == &Tag::new(b"head") {
            head_at = Some(offset as usize);
        }
        data.extend_from_slice(bytes);
        let padded = bytes.len().next_multiple_of(4);
        data.resize(data.len() + padded - bytes.len(), 0);
        offset += padded as u32;
    }
    out.extend_from_slice(&data);
    if let Some(at) = head_at {
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[at + 8..at + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use skrifa::attribute::{Style, Weight};
    use skrifa::raw::TableProvider;

    use super::*;

    fn read(file: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(file)).unwrap()
    }

    #[test]
    fn a_face_takes_the_family_and_tags_it_is_given_and_keeps_everything_else() {
        let data = read("../terminal/tests/fonts/GeistMono-Variable.ttf");
        let out = private_face(&data, 0, "Private Mono", true, true).unwrap();
        let (font, original) = (FontRef::new(&out).unwrap(), FontRef::new(&data).unwrap());
        let name = |f: &FontRef, id| {
            f.localized_strings(id)
                .english_or_first()
                .map(|s| s.to_string())
        };
        assert_eq!(
            name(&font, StringId::TYPOGRAPHIC_FAMILY_NAME).as_deref(),
            Some("Private Mono")
        );
        assert_eq!(
            name(&font, StringId::FAMILY_NAME).as_deref(),
            Some("Private Mono")
        );
        assert_eq!(
            name(&font, StringId::POSTSCRIPT_NAME),
            name(&original, StringId::POSTSCRIPT_NAME)
        );
        let attributes = font.attributes();
        assert_eq!(attributes.weight, Weight::BOLD);
        assert_eq!(attributes.style, Style::Italic);
        // The outlines, metrics and the rest are the face's own bytes.
        for tag in [b"glyf", b"hmtx", b"hhea", b"cmap", b"fvar"] {
            let tag = Tag::new(tag);
            assert_eq!(
                font.table_data(tag).map(|d| d.as_bytes().to_vec()),
                original.table_data(tag).map(|d| d.as_bytes().to_vec()),
                "{tag}"
            );
        }
        // A whole file sums to the magic number.
        assert_eq!(checksum(&out), 0xB1B0_AFBA);
    }

    #[test]
    fn a_regular_face_is_400_upright_with_the_regular_bit() {
        let data = read("../../assets/fonts/JetBrainsMono-Italic-Variable.ttf");
        let out = private_face(&data, 0, "Private", false, false).unwrap();
        let font = FontRef::new(&out).unwrap();
        assert_eq!(font.attributes().weight, Weight::NORMAL);
        assert_eq!(font.attributes().style, Style::Normal);
        let selection = font.os2().unwrap().fs_selection().bits();
        assert_eq!(selection & (1 | 1 << 5 | 1 << 6), 1 << 6);
        assert!(private_face(b"not a font", 0, "Private", false, false).is_none());
    }
}
