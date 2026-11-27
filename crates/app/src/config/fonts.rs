//! The terminal font, found by name among the installed fonts the way Ghostty finds its
//! `font-family` (`ghostty:src/font/SharedGridSet.zig` `Key.init` and `collection`, at `v1.4.0`,
//! `83fb9562`):
//!
//! - Each of the four styles asks CoreText for the family (`glyph::discovery`): the regular style
//!   with its `font_style` name if one is set; bold, italic and bold italic with theirs, or, with
//!   none, the bold and italic traits. The best face by Ghostty's `Score` draws the style.
//! - A family name is what CoreText matches: a typographic family ("Operator Mono") or a legacy
//!   one, which can name one weight ("Operator Mono Book").
//! - A regular style found nowhere means the family isn't installed: the terminals fall back to
//!   the bundled JetBrains Mono, as Ghostty falls back to the one it embeds.
//!
//! Each face found is read (never written) from its file and given to Rune as a private family
//! ([`sfnt`] says why). A style with no face is drawn from the faces there are: bold from the
//! regular face, italic as Rune's oblique of it. Ghostty synthesizes those
//! (`Collection.completeStyles`, `font-synthetic-style`); herdr-frontend doesn't yet.

pub mod sfnt;

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

/// A terminal text style: regular, bold, italic or bold italic (Ghostty's `font.Style`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Slot {
    pub const ALL: [Slot; 4] = [Slot::Regular, Slot::Bold, Slot::Italic, Slot::BoldItalic];

    pub fn bold(self) -> bool {
        matches!(self, Slot::Bold | Slot::BoldItalic)
    }

    pub fn italic(self) -> bool {
        matches!(self, Slot::Italic | Slot::BoldItalic)
    }

    /// The settings key that names this style's face.
    pub fn key(self) -> &'static str {
        match self {
            Slot::Regular => "font_style",
            Slot::Bold => "font_style_bold",
            Slot::Italic => "font_style_italic",
            Slot::BoldItalic => "font_style_bold_italic",
        }
    }
}

/// A face as found: its installed file, the file's bytes and the face's index in it (a
/// collection holds several).
pub type Source = (PathBuf, Arc<[u8]>, u32);

/// One face a style draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceFile {
    /// The installed file it came from, and its PostScript name.
    pub path: PathBuf,
    pub postscript: String,
    /// The face as a font of its own in the family's private name, tagged with its style.
    pub bytes: Arc<[u8]>,
}

/// A terminal font family: the face each style draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    /// The family the settings name.
    pub name: String,
    /// The name its faces are registered under with Rune: theirs alone, so two families (or the
    /// same one with other faces) never mix.
    pub private: String,
    /// Regular, bold, italic and bold italic, as [`Slot::ALL`]; the regular face is always there.
    pub faces: [Option<FaceFile>; 4],
}

impl Family {
    /// The family made of these faces, one [`Source`] a style. `None` without a regular face, or
    /// when one isn't a font.
    pub fn from_faces(name: &str, faces: [Option<Source>; 4]) -> Option<Family> {
        faces[0].as_ref()?;
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        faces.hash(&mut hasher);
        let private = format!("{name} herdr-frontend {:016x}", hasher.finish());
        let mut made: [Option<FaceFile>; 4] = Default::default();
        for (slot, (face, out)) in Slot::ALL.into_iter().zip(faces.iter().zip(&mut made)) {
            let Some((path, data, index)) = face else {
                continue;
            };
            let postscript = postscript(data, *index).unwrap_or_default();
            let bytes = sfnt::private_face(data, *index, &private, slot.bold(), slot.italic())?;
            *out = Some(FaceFile {
                path: path.clone(),
                postscript,
                bytes: Arc::from(bytes),
            });
        }
        Some(Family {
            name: name.to_owned(),
            private,
            faces: made,
        })
    }

    /// The regular face, which the cell is measured from.
    pub fn regular(&self) -> &FaceFile {
        self.faces[0]
            .as_ref()
            .expect("a family has its regular face")
    }

    pub fn face(&self, slot: Slot) -> Option<&FaceFile> {
        self.faces[slot as usize].as_ref()
    }
}

fn postscript(data: &[u8], index: u32) -> Option<String> {
    use skrifa::MetadataProvider;
    let font = skrifa::FontRef::from_index(data, index).ok()?;
    let name = font
        .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
        .english_or_first()?;
    Some(name.to_string())
}

/// What looking a family up found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// `None` when the family isn't installed (no regular face).
    pub family: Option<Family>,
    /// The styles whose style name no face of the family has.
    pub unmatched: Vec<Slot>,
}

/// The installed faces of `family` for each style, `styles` naming them as [`Slot::ALL`]
/// (`None` finds a style by its traits).
pub fn find(family: &str, styles: [Option<&str>; 4]) -> Found {
    let family = family.trim();
    let mut unmatched = Vec::new();
    let mut faces: [Option<Source>; 4] = Default::default();
    for (slot, out) in Slot::ALL.into_iter().zip(&mut faces) {
        let style = styles[slot as usize].map(str::trim);
        *out = discover(family, style, slot);
        if out.is_none() && style.is_some() {
            unmatched.push(slot);
        }
    }
    Found {
        family: Family::from_faces(family, faces),
        unmatched,
    }
}

/// The face Ghostty's discovery takes for one style: the first of the sorted matches
/// (`DiscoverIterator.next`), read from its file.
#[cfg(target_os = "macos")]
fn discover(family: &str, style: Option<&str>, slot: Slot) -> Option<Source> {
    use crate::view::terminal::glyph::discovery::{self, Descriptor};
    // An explicit style name replaces the traits: with both, a font-style-bold of "Italic" would
    // find nothing (`Key.init`).
    let want = Descriptor {
        family: Some(family),
        style,
        bold: style.is_none() && slot.bold(),
        italic: style.is_none() && slot.italic(),
        ..Descriptor::default()
    };
    let (descriptor, font) = discovery::discover(&want).into_iter().next()?;
    // SAFETY: CoreText getters on a live descriptor and font; the URL attribute is a CFURL.
    let (path, name) = unsafe {
        let url = descriptor.attribute(objc2_core_text::kCTFontURLAttribute)?;
        let url = url.downcast::<objc2_core_foundation::CFURL>().ok()?;
        (url.to_file_path()?, font.post_script_name().to_string())
    };
    let data: Arc<[u8]> = Arc::from(std::fs::read(&path).ok()?);
    let index = (0..face_count(&data)).find(|&i| postscript(&data, i).as_deref() == Some(&name))?;
    Some((path, data, index))
}

#[cfg(not(target_os = "macos"))]
fn discover(_: &str, _: Option<&str>, _: Slot) -> Option<Source> {
    None
}

/// How many faces a font file holds: a collection's count, else 1.
#[cfg(target_os = "macos")]
fn face_count(data: &[u8]) -> u32 {
    match skrifa::raw::FileRef::new(data) {
        Ok(skrifa::raw::FileRef::Collection(c)) => c.len(),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn file(path: &str) -> Source {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        let data = Arc::from(std::fs::read(&path).unwrap());
        (path, data, 0)
    }

    #[test]
    fn a_family_needs_its_regular_face_and_names_itself_by_its_faces() {
        let regular = file("../../assets/fonts/JetBrainsMono-Variable.ttf");
        let italic = file("../../assets/fonts/JetBrainsMono-Italic-Variable.ttf");
        assert!(Family::from_faces("Mono", [None, None, Some(italic.clone()), None]).is_none());
        let both =
            Family::from_faces("Mono", [Some(regular.clone()), None, Some(italic), None]).unwrap();
        assert_eq!(both.regular().postscript, "JetBrainsMono-Regular");
        assert_eq!(
            both.face(Slot::Italic).map(|f| f.postscript.as_str()),
            Some("JetBrainsMono-Italic")
        );
        assert!(both.face(Slot::Bold).is_none());
        let alone = Family::from_faces("Mono", [Some(regular), None, None, None]).unwrap();
        assert!(both.private.starts_with("Mono herdr-frontend "));
        assert_ne!(both.private, alone.private);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn menlo_is_found_by_traits_and_a_missing_family_isnt() {
        let found = find("Menlo", [None; 4]);
        let family = found.family.expect("Menlo ships with macOS");
        let names: Vec<&str> = Slot::ALL
            .iter()
            .map(|s| family.face(*s).unwrap().postscript.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Menlo-Regular",
                "Menlo-Bold",
                "Menlo-Italic",
                "Menlo-BoldItalic"
            ]
        );
        assert!(found.unmatched.is_empty());
        // Menlo is one collection: each face is its own font now.
        assert!(
            family
                .faces
                .iter()
                .flatten()
                .all(|f| !f.bytes.starts_with(b"ttcf"))
        );

        let none = find("No Such Family Anywhere", [None; 4]);
        assert!(none.family.is_none());
        let styled = find("Menlo", [None, Some("Heavy"), None, None]);
        assert_eq!(styled.unmatched, [Slot::Bold]);
        assert!(styled.family.unwrap().face(Slot::Bold).is_none());
    }

    /// Eli's default, when Operator Mono is installed (`~/Library/Fonts`): Book and its own
    /// Bold, found by style name, nothing synthesized.
    #[cfg(target_os = "macos")]
    #[test]
    fn operator_mono_is_book_and_bold_by_style_name() {
        let settings = super::super::settings::Settings::default();
        let found = find(&settings.font, settings.font_styles());
        let Some(family) = found.family else {
            eprintln!("skipping: Operator Mono isn't installed");
            return;
        };
        let names: Vec<&str> = Slot::ALL
            .iter()
            .map(|s| family.face(*s).map_or("", |f| f.postscript.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                "OperatorMono-Book",
                "OperatorMono-Bold",
                "OperatorMono-BookItalic",
                "OperatorMono-BoldItalic"
            ]
        );
        assert!(found.unmatched.is_empty());
    }
}
