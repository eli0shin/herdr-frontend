//! Fetches the pinned libghostty-vt xcframework, checks its SHA-256 and links its macOS slice.
//!
//! No Zig: the release's prebuilt static library is linked as is (`terminal-rendering.md` §1).
//! The zip is cached outside `target/` so `cargo clean` and new worktrees don't download it again:
//! `$GHOSTTY_VT_CACHE_DIR`, or `~/Library/Caches/ghostty-vt-sys`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use sha2::{Digest, Sha256};

/// The pinned release. Bump all four together, then regenerate `src/bindings.rs`.
const VERSION: &str = "1.0.2";
const ZIP: &str = "ghostty-vt-1.0.2.xcframework.zip";
const URL: &str =
    "https://release.files.ghostty.org/libghostty-vt/1.0.2/ghostty-vt-1.0.2.xcframework.zip";
/// From the release's `SHA256SUMS`.
const SHA256: &str = "15477209ce86ad1c944f217e925c84acf796ecc51aba7a935aaa1a7769890bd6";

/// The macOS slice: a universal (arm64 + x86_64) static library and its headers.
const SLICE: &str = "ghostty-vt.xcframework/macos-arm64_x86_64";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GHOSTTY_VT_CACHE_DIR");

    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    assert!(
        os == "macos",
        "ghostty-vt-sys links only the macOS slice of the libghostty-vt xcframework, not {os}"
    );
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("the libghostty-vt macOS slice has no {other} architecture"),
    };

    let zip = fetch(&cache_dir());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let slice = extract(&zip, &out);

    // Thin the universal archive to the target's architecture, alone in its search directory.
    let lib = out.join("lib");
    fs::create_dir_all(&lib).unwrap();
    run(Command::new("lipo")
        .arg(slice.join("libghostty-vt.a"))
        .args(["-thin", arch, "-output"])
        .arg(lib.join("libghostty-vt.a")));

    let headers = slice.join("Headers");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
    // `DEP_GHOSTTY_VT_INCLUDE` for dependents' build scripts, and the bindgen binary's header path.
    println!("cargo:include={}", headers.display());
    println!(
        "cargo:rustc-env=GHOSTTY_VT_INCLUDE_DIR={}",
        headers.display()
    );
    println!("cargo:rustc-env=GHOSTTY_VT_PINNED_VERSION={VERSION}");
}

fn cache_dir() -> PathBuf {
    if let Some(dir) = env::var_os("GHOSTTY_VT_CACHE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = env::var_os("HOME").expect("HOME is unset; set GHOSTTY_VT_CACHE_DIR instead");
    PathBuf::from(home).join("Library/Caches/ghostty-vt-sys")
}

/// Returns the cached zip, downloading it first if it's missing or doesn't match the checksum.
fn fetch(cache: &Path) -> PathBuf {
    let zip = cache.join(ZIP);
    if zip.exists() {
        let actual = sha256(&zip);
        if actual == SHA256 {
            return zip;
        }
        println!(
            "cargo:warning=cached {} has SHA-256 {actual}, expected {SHA256}; downloading again",
            zip.display()
        );
        fs::remove_file(&zip).unwrap();
    }

    fs::create_dir_all(cache)
        .unwrap_or_else(|e| panic!("cannot create cache dir {}: {e}", cache.display()));
    // A per-process name, renamed into place once verified, so concurrent builds never read a
    // partial or unverified file.
    let part = cache.join(format!("{ZIP}.{}.part", std::process::id()));
    run(Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--retry", "3", "--output"])
        .arg(&part)
        .arg(URL));
    let actual = sha256(&part);
    if actual != SHA256 {
        let _ = fs::remove_file(&part);
        panic!("{URL} has SHA-256 {actual}, expected {SHA256}");
    }
    fs::rename(&part, &zip).unwrap();
    zip
}

/// Unzips only the macOS slice into `out`, replacing any earlier extraction.
fn extract(zip: &Path, out: &Path) -> PathBuf {
    let dest = out.join("xcframework");
    if dest.exists() {
        fs::remove_dir_all(&dest).unwrap();
    }
    run(Command::new("unzip")
        .args(["-q", "-o"])
        .arg(zip)
        .arg(format!("{SLICE}/*"))
        .arg("-d")
        .arg(&dest));
    dest.join(SLICE)
}

fn sha256(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed: {status}");
}
