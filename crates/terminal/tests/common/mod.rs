//! The fonts the metrics tests measure, and where to find them.
//!
//! The fonts whose licenses allow it are checked in (OFL): Geist Mono in `tests/fonts/`, and
//! JetBrains Mono in the workspace's `assets/fonts/`, which the app draws terminals with too.
//! Eli's Operator Mono is commercial, so it's read from where it's installed (`~/Library/Fonts`)
//! and the tests that need it skip when it isn't there. System fonts are read from `/System/Library/Fonts` and skip
//! the same way.

#![allow(dead_code)]

use std::path::PathBuf;

/// A font file, and the face in it.
#[derive(Debug, Clone)]
pub struct FontFile {
    pub name: &'static str,
    pub path: PathBuf,
    pub index: u32,
}

impl FontFile {
    /// The font's bytes, or `None` (with a note on stderr) when it isn't on this machine.
    pub fn load(&self) -> Option<Vec<u8>> {
        match std::fs::read(&self.path) {
            Ok(data) => Some(data),
            Err(err) => {
                eprintln!("skipping {}: {}: {err}", self.name, self.path.display());
                None
            }
        }
    }
}

pub fn checked_in(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fonts")
        .join(file)
}

/// Where Eli's Operator Mono is installed, `~/Library/Fonts`.
pub fn eli_fonts() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Library/Fonts")
}

/// A font in the workspace's `assets/fonts/`.
pub fn asset(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/fonts")
        .join(file)
}

pub fn jetbrains_mono() -> FontFile {
    FontFile {
        name: "JetBrains Mono",
        path: asset("JetBrainsMono-Variable.ttf"),
        index: 0,
    }
}

pub fn geist_mono() -> FontFile {
    FontFile {
        name: "Geist Mono",
        path: checked_in("GeistMono-Variable.ttf"),
        index: 0,
    }
}

/// One of the installed Operator Mono faces the terminal draws with: `Book`, `Bold`,
/// `BookItalic` or `BoldItalic` (`OperatorMono-<style>.otf`).
pub fn operator_mono(style: &'static str) -> FontFile {
    FontFile {
        name: "Operator Mono",
        path: eli_fonts().join(format!("OperatorMono-{style}.otf")),
        index: 0,
    }
}

fn system(name: &'static str, path: &str) -> FontFile {
    FontFile {
        name,
        path: PathBuf::from(path),
        index: 0,
    }
}

/// Every font the tests know about, present or not.
pub fn all_fonts() -> Vec<FontFile> {
    vec![
        jetbrains_mono(),
        geist_mono(),
        operator_mono("Book"),
        operator_mono("Bold"),
        operator_mono("BookItalic"),
        operator_mono("BoldItalic"),
        system("Menlo", "/System/Library/Fonts/Menlo.ttc"),
        system("Monaco", "/System/Library/Fonts/Monaco.ttf"),
        system("SF Mono", "/System/Library/Fonts/SFNSMono.ttf"),
        system("Courier", "/System/Library/Fonts/Courier.ttc"),
        system(
            "Courier New",
            "/System/Library/Fonts/Supplemental/Courier New.ttf",
        ),
        system(
            "Andale Mono",
            "/System/Library/Fonts/Supplemental/Andale Mono.ttf",
        ),
        system("PT Mono", "/System/Library/Fonts/Supplemental/PTMono.ttc"),
    ]
}

/// The sizes and scales the cross-checks run at: Eli's 17 pt × zoom 1.1, herdr-app's default
/// 14 pt, and a few more, at 1x, Retina 2x and a fractional scale.
pub const POINTS: [f32; 6] = [17.0 * 1.1, 14.0, 13.0, 12.0, 15.5, 24.0];
pub const SCALES: [f32; 3] = [1.0, 2.0, 1.5];
