//! Regenerates `src/bindings.rs` from the pinned release's `ghostty/vt.h`.
//!
//! `cargo run -p ghostty-vt-sys --features bindgen --bin ghostty-vt-bindgen` writes the file, and
//! `-- --check` fails if the committed file differs. The ABI is frozen within 1.x, so the bindings
//! only change when the pin moves. Needs libclang: Xcode's or the Command Line Tools' on macOS,
//! the system's on Linux. Both platforms generate the same file.

use std::path::Path;
use std::process::{Command, ExitCode};

const INCLUDE_DIR: &str = env!("GHOSTTY_VT_INCLUDE_DIR");
const BINDINGS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/bindings.rs");

fn main() -> ExitCode {
    let check = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--check") => true,
        Some(other) => {
            eprintln!("usage: ghostty-vt-bindgen [--check], not {other:?}");
            return ExitCode::FAILURE;
        }
    };

    let generated = generate();
    let path = Path::new(BINDINGS);
    if check {
        let committed = std::fs::read_to_string(path).unwrap_or_default();
        if committed != generated {
            eprintln!(
                "{} is stale: run `just bindings` and commit the result",
                path.display()
            );
            return ExitCode::FAILURE;
        }
        println!("{} is up to date", path.display());
    } else {
        std::fs::write(path, generated).unwrap();
        println!("wrote {}", path.display());
    }
    ExitCode::SUCCESS
}

/// The allowlists and options herdr-app uses (`terminal-rendering.md` §1), with layout tests kept
/// so a mismatched struct fails to compile, and one constant dropped so macOS and Linux generate
/// the same file.
fn generate() -> String {
    let include = Path::new(INCLUDE_DIR);
    let mut builder = bindgen::Builder::default()
        .header(include.join("ghostty/vt.h").to_str().unwrap())
        .clang_arg(format!("-I{}", include.display()));
    if cfg!(target_os = "macos") {
        // Apple's libclang has its builtin headers but needs the SDK for the C library.
        let sdk = Command::new("xcrun")
            .arg("--show-sdk-path")
            .output()
            .expect("cannot run xcrun --show-sdk-path");
        assert!(sdk.status.success(), "xcrun --show-sdk-path failed");
        let sdk = String::from_utf8(sdk.stdout).unwrap();
        builder = builder.clang_arg(format!("-isysroot{}", sdk.trim()));
    }

    let bindings = builder
        .allowlist_function("ghostty_.*")
        .allowlist_type("Ghostty.*")
        .allowlist_var("GHOSTTY_.*")
        // `#define GHOSTTY_ENUM_MAX_VALUE INT_MAX`: glibc's `INT_MAX` is the compiler's
        // `__INT_MAX__`, which bindgen can't evaluate, so only macOS would get the constant.
        // Nothing uses it; each enum keeps its own `*_MAX_VALUE` variant.
        .blocklist_item("GHOSTTY_ENUM_MAX_VALUE")
        .prepend_enum_name(false)
        .derive_default(true)
        .generate()
        .expect("bindgen failed on ghostty/vt.h");
    bindings.to_string()
}
