//! Finding a font family by name among font files: the home's `fonts/`, then Herdr app's
//! (`$HERDR_APP_HOME/fonts`, where Eli's prepared Operator Mono Book Herdr lives, read only), as
//! herdr-app loads its home's `fonts/` (`app:src/settings.rs` `load_home_fonts`).
//!
//! Fonts installed on the system aren't searched: a terminal font needs its file for Ghostty's
//! cell metrics. A family found nowhere falls back to the bundled JetBrains Mono, as herdr-app's
//! terminals do.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use skrifa::raw::FileRef;
use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider};

/// One face in a font file.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceFile {
    pub path: PathBuf,
    /// The whole file; `index` is the face's in a collection.
    pub bytes: Arc<[u8]>,
    pub index: u32,
    pub weight: f32,
    pub italic: bool,
}

/// A family's faces, the regular one first: the one CSS font matching picks for weight 400
/// upright, which is the face Rune draws normal text with and the cell is measured from.
#[derive(Clone, Debug, PartialEq)]
pub struct FamilyFiles {
    pub name: String,
    pub faces: Vec<FaceFile>,
}

/// The `.ttf`, `.otf` and `.ttc` files in `dirs`, each directory sorted by name. A folder that
/// doesn't exist is skipped.
pub fn files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_font(p))
            .collect();
        found.sort();
        out.extend(found);
    }
    out
}

fn is_font(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
}

/// The family named `name` (case-insensitive, by its typographic or legacy family name) among
/// the font files in `dirs`, the first folder that has it winning.
pub fn find(name: &str, dirs: &[PathBuf]) -> Option<FamilyFiles> {
    let want = name.trim().to_lowercase();
    for dir in dirs {
        let mut faces = Vec::new();
        for path in files(std::slice::from_ref(dir)) {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let bytes: Arc<[u8]> = Arc::from(bytes);
            for (index, font) in fonts_in(&bytes) {
                if names(&font).iter().any(|n| n.to_lowercase() == want) {
                    let attributes = font.attributes();
                    faces.push(FaceFile {
                        path: path.clone(),
                        bytes: bytes.clone(),
                        index,
                        weight: attributes.weight.value(),
                        italic: attributes.style != skrifa::attribute::Style::Normal,
                    });
                }
            }
        }
        if !faces.is_empty() {
            let regular = regular(&faces);
            let first = faces.remove(regular);
            faces.insert(0, first);
            return Some(FamilyFiles {
                name: name.trim().to_owned(),
                faces,
            });
        }
    }
    None
}

/// Every face in a file, with its index.
fn fonts_in(bytes: &[u8]) -> Vec<(u32, FontRef<'_>)> {
    match FileRef::new(bytes) {
        Ok(FileRef::Font(font)) => vec![(0, font)],
        Ok(FileRef::Collection(collection)) => collection
            .iter()
            .enumerate()
            .filter_map(|(i, f)| Some((i as u32, f.ok()?)))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// A face's family names: typographic (name ID 16) and legacy (1).
fn names(font: &FontRef) -> Vec<String> {
    [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]
        .into_iter()
        .filter_map(|id| font.localized_strings(id).english_or_first())
        .map(|s| s.to_string())
        .collect()
}

/// The face CSS font matching picks for weight 400 upright: an upright face before an italic
/// one; then 400 to 500, the lightest first; then the heaviest below 400; then the lightest
/// above 500.
fn regular(faces: &[FaceFile]) -> usize {
    let rank = |f: &FaceFile| {
        let w = f.weight;
        let band = if (400.0..=500.0).contains(&w) {
            (0, w)
        } else if w < 400.0 {
            (1, -w)
        } else {
            (2, w)
        };
        (f.italic, band.0, band.1)
    };
    (0..faces.len())
        .min_by(|&a, &b| {
            let (a, b) = (rank(&faces[a]), rank(&faces[b]));
            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-frontend-fonts-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn face(path: &str, weight: f32, italic: bool) -> FaceFile {
        FaceFile {
            path: path.into(),
            bytes: Arc::from(&[][..]),
            index: 0,
            weight,
            italic,
        }
    }

    #[test]
    fn the_regular_face_is_the_one_css_picks_for_400() {
        // Eli's prepared Operator Mono: Book is 500, Bold 700, and their italics.
        let faces = [
            face("BoldItalic", 700.0, true),
            face("Bold", 700.0, false),
            face("Italic", 500.0, true),
            face("Book", 500.0, false),
        ];
        assert_eq!(faces[regular(&faces)].path, Path::new("Book"));
        let faces = [face("Light", 300.0, false), face("Semi", 600.0, false)];
        assert_eq!(faces[regular(&faces)].path, Path::new("Light"));
        let faces = [face("Semi", 600.0, false), face("Bold", 700.0, false)];
        assert_eq!(faces[regular(&faces)].path, Path::new("Semi"));
    }

    #[test]
    fn a_family_is_found_by_name_in_the_first_folder_that_has_it() {
        let (a, b) = (scratch("a"), scratch("b"));
        let fonts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        std::fs::copy(
            fonts.join("JetBrainsMono-Italic-Variable.ttf"),
            b.join("JetBrainsMono-Italic-Variable.ttf"),
        )
        .unwrap();
        std::fs::copy(
            fonts.join("JetBrainsMono-Variable.ttf"),
            b.join("JetBrainsMono-Variable.ttf"),
        )
        .unwrap();
        std::fs::write(b.join("notes.txt"), "not a font").unwrap();
        std::fs::write(b.join("broken.otf"), "not a font either").unwrap();

        let dirs = [a.join("missing"), a.clone(), b.clone()];
        let family = find("jetbrains mono", &dirs).expect("found in b");
        assert_eq!(family.faces.len(), 2);
        assert!(!family.faces[0].italic, "the upright face first");
        assert!(family.faces[1].italic);
        assert!(find("Operator Mono Book Herdr", &dirs).is_none());
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}
