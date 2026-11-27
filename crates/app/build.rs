//! Embeds the bundled program icons (`assets/icons/*.svg`, `assets/program-icons/*.svg`) as
//! `ICONS: &[(&str, &[u8])]`, keyed by the home-relative path `programs.json` names them by
//! (`icons/pi.svg`), for `src/config/programs.rs`.

use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let assets = Path::new(&manifest).join("../../assets");
    let mut lines = Vec::new();
    for folder in ["icons", "program-icons"] {
        let dir = assets.join(folder);
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "svg"))
            .collect();
        files.sort();
        for path in files {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let path = path.canonicalize().unwrap();
            lines.push(format!(
                "    ({:?}, include_bytes!({:?})),",
                format!("{folder}/{name}"),
                path.display().to_string()
            ));
        }
    }
    let code = format!(
        "/// Every bundled icon, by its home-relative path.\npub const ICONS: &[(&str, &[u8])] = &[\n{}\n];\n",
        lines.join("\n")
    );
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icons.rs");
    std::fs::write(out, code).expect("writing icons.rs");
}
