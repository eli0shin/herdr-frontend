//! Links the pinned libghostty-vt release, checksum-checked, as a static library.
//!
//! - macOS: the release's prebuilt xcframework, its macOS slice thinned to the target's
//!   architecture. No Zig (`terminal-rendering.md` §1).
//! - Linux: built from the release's source and deps tarballs with the pinned Zig, as
//!   `PACKAGING-LIBGHOSTTY.md` "Building libghostty-vt" describes. The release has no prebuilt
//!   Linux library.
//!
//! Downloads and the Linux build are cached outside `target/` so `cargo clean` and new worktrees
//! don't repeat them: `$GHOSTTY_VT_CACHE_DIR`, else `~/Library/Caches/ghostty-vt-sys` on macOS and
//! `$XDG_CACHE_HOME/ghostty-vt-sys` (or `~/.cache/ghostty-vt-sys`) on Linux.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use sha2::{Digest, Sha256};

/// The pinned release. Bump it with the three downloads below, then regenerate `src/bindings.rs`.
const VERSION: &str = "1.0.2";
const RELEASE: &str = "https://release.files.ghostty.org/libghostty-vt/1.0.2";

/// A pinned file: its name, where it's published, and its SHA-256 (from the release's `SHA256SUMS`,
/// or Zig's `download/index.json`).
struct Download {
    file: &'static str,
    base: &'static str,
    sha256: &'static str,
}

const XCFRAMEWORK: Download = Download {
    file: "ghostty-vt-1.0.2.xcframework.zip",
    base: RELEASE,
    sha256: "15477209ce86ad1c944f217e925c84acf796ecc51aba7a935aaa1a7769890bd6",
};
const SOURCE: Download = Download {
    file: "libghostty-vt-1.0.2.tar.gz",
    base: RELEASE,
    sha256: "1dcafa8a3357f3a6bbf4f2696e61de611400c5469518716793dfffd35f008b3e",
};
const DEPS: Download = Download {
    file: "libghostty-vt-1.0.2-deps.tar.gz",
    base: RELEASE,
    sha256: "c56156dbea82fdf71f13bf0d704cdf4393b6a04208d7f9691e0d7f60c7f9ca4d",
};

/// The Zig the Linux build uses: 1.0.2's `minimum_zig_version` is 0.17.0, and any 0.17 patch works.
const ZIG_VERSION: &str = "0.17.1";
const ZIG_BASE: &str = "https://ziglang.org/download/0.17.1";
/// Zig for the machine running the build, by `<arch>-<os>` as Zig names its downloads.
const ZIG: &[(&str, Download)] = &[
    (
        "x86_64-linux",
        Download {
            file: "zig-x86_64-linux-0.17.1.tar.xz",
            base: ZIG_BASE,
            sha256: "3581863e3cbcfd0fce0efc682ce7e9e93da96668a977791d356a26d2a7e9b17a",
        },
    ),
    (
        "aarch64-linux",
        Download {
            file: "zig-aarch64-linux-0.17.1.tar.xz",
            base: ZIG_BASE,
            sha256: "a1546d5646eb184437918d6635ca6b762a0e889eac09cef720ee7a1ed3fe7795",
        },
    ),
    (
        "aarch64-macos",
        Download {
            file: "zig-aarch64-macos-0.17.1.tar.xz",
            base: ZIG_BASE,
            sha256: "75f1bc7a1e9852f17ec9a0c1895f767c4ec56d46bc8a834920eece1d0359dd8d",
        },
    ),
    (
        "x86_64-macos",
        Download {
            file: "zig-x86_64-macos-0.17.1.tar.xz",
            base: ZIG_BASE,
            sha256: "9af07b20319fbd0c92ee46ad23e81d6a782b1aaea2cfb8dcbff35f1d2c7ddbb7",
        },
    ),
];

/// The macOS slice: a universal (arm64 + x86_64) static library and its headers.
const SLICE: &str = "ghostty-vt.xcframework/macos-arm64_x86_64";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GHOSTTY_VT_CACHE_DIR");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cache = cache_dir();
    // The static library goes alone in its search directory, so the linker can't pick another.
    let lib = out.join("lib");
    fs::create_dir_all(&lib).unwrap();

    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let headers = match os.as_str() {
        "macos" => link_xcframework(&cache, &out, &lib),
        "linux" => link_source_build(&cache, &out, &lib),
        other => panic!("ghostty-vt-sys links libghostty-vt on macOS and Linux, not {other}"),
    };

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

/// Thins the xcframework's universal archive into `lib` and returns its headers.
fn link_xcframework(cache: &Path, out: &Path, lib: &Path) -> PathBuf {
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("the libghostty-vt macOS slice has no {other} architecture"),
    };
    let zip = fetch(cache, &XCFRAMEWORK);
    let dest = fresh_dir(&out.join("xcframework"));
    run(Command::new("unzip")
        .args(["-q", "-o"])
        .arg(&zip)
        .arg(format!("{SLICE}/*"))
        .arg("-d")
        .arg(&dest));
    let slice = dest.join(SLICE);
    run(Command::new("lipo")
        .arg(slice.join("libghostty-vt.a"))
        .args(["-thin", arch, "-output"])
        .arg(lib.join("libghostty-vt.a")));
    slice.join("Headers")
}

/// Copies the Linux build's static library into `lib`, building it first if it isn't cached, and
/// returns its headers.
fn link_source_build(cache: &Path, out: &Path, lib: &Path) -> PathBuf {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let abi = env::var("CARGO_CFG_TARGET_ENV").unwrap();
    assert!(
        matches!(arch.as_str(), "x86_64" | "aarch64") && matches!(abi.as_str(), "gnu" | "musl"),
        "ghostty-vt-sys builds libghostty-vt for x86_64 and aarch64 Linux (gnu or musl), not {arch}-{abi}"
    );
    let target = format!("{arch}-linux-{abi}");
    let tree = cache.join(format!(
        "libghostty-vt-{VERSION}-zig-{ZIG_VERSION}-{target}"
    ));
    if !tree.join("lib/libghostty-vt.a").exists() {
        build_from_source(cache, out, &target, &tree);
    }
    fs::copy(
        tree.join("lib/libghostty-vt.a"),
        lib.join("libghostty-vt.a"),
    )
    .unwrap();
    tree.join("include")
}

/// Builds libghostty-vt for `target` with the pinned Zig and installs it to `tree`.
fn build_from_source(cache: &Path, out: &Path, target: &str, tree: &Path) {
    let host = env::var("HOST").unwrap();
    let host_arch = host.split('-').next().unwrap();
    let host_os = if host.contains("-linux") {
        "linux"
    } else if host.contains("-apple-darwin") {
        "macos"
    } else {
        panic!("no pinned Zig for the build machine {host}")
    };
    let zig_key = format!("{host_arch}-{host_os}");
    let (_, zig) = ZIG
        .iter()
        .find(|(key, _)| *key == zig_key)
        .unwrap_or_else(|| panic!("no pinned Zig for the build machine {zig_key}"));

    let work = fresh_dir(&out.join("libghostty-vt-build"));
    for download in [zig, &SOURCE, &DEPS] {
        let archive = fetch(cache, download);
        run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&work));
    }
    let zig = work.join(zig.file.trim_end_matches(".tar.xz")).join("zig");
    let source = work.join(format!("libghostty-vt-{VERSION}"));
    let deps = work.join(format!("libghostty-vt-{VERSION}-deps"));

    // Installed beside the cache entry and renamed into place once complete, so concurrent builds
    // never read a partial tree.
    let name = tree.file_name().unwrap().to_str().unwrap();
    let part = tree.with_file_name(format!("{name}.{}.part", std::process::id()));
    let _ = fs::remove_dir_all(&part);
    println!("cargo:warning=building libghostty-vt {VERSION} for {target} with Zig {ZIG_VERSION}");
    run(Command::new(&zig)
        .arg("build")
        .arg("--prefix")
        .arg(&part)
        .arg("--system")
        .arg(&deps)
        .args(["-Doptimize=ReleaseFast", "-Dcpu=baseline"])
        .arg(format!("-Dtarget={target}"))
        .current_dir(&source)
        // A new, empty global cache, as PACKAGING-LIBGHOSTTY.md requires for Zig 0.17.
        .env("ZIG_GLOBAL_CACHE_DIR", work.join("zig-global-cache")));
    if fs::rename(&part, tree).is_err() {
        // Another build finished first; keep its tree.
        let _ = fs::remove_dir_all(&part);
    }
    let _ = fs::remove_dir_all(&work);
}

fn cache_dir() -> PathBuf {
    if let Some(dir) = env::var_os("GHOSTTY_VT_CACHE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = || {
        PathBuf::from(env::var_os("HOME").expect("HOME is unset; set GHOSTTY_VT_CACHE_DIR instead"))
    };
    if env::var("HOST").unwrap().contains("-apple-darwin") {
        return home().join("Library/Caches/ghostty-vt-sys");
    }
    env::var_os("XDG_CACHE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".cache"))
        .join("ghostty-vt-sys")
}

/// Returns the cached file, downloading it first if it's missing or doesn't match the checksum.
fn fetch(cache: &Path, download: &Download) -> PathBuf {
    let Download {
        file,
        base,
        sha256: expected,
    } = download;
    let path = cache.join(file);
    if path.exists() {
        let actual = sha256(&path);
        if actual == *expected {
            return path;
        }
        println!(
            "cargo:warning=cached {} has SHA-256 {actual}, expected {expected}; downloading again",
            path.display()
        );
        fs::remove_file(&path).unwrap();
    }

    fs::create_dir_all(cache)
        .unwrap_or_else(|e| panic!("cannot create cache dir {}: {e}", cache.display()));
    // A per-process name, renamed into place once verified, so concurrent builds never read a
    // partial or unverified file.
    let part = cache.join(format!("{file}.{}.part", std::process::id()));
    let url = format!("{base}/{file}");
    run(Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--retry", "3", "--output"])
        .arg(&part)
        .arg(&url));
    let actual = sha256(&part);
    if actual != *expected {
        let _ = fs::remove_file(&part);
        panic!("{url} has SHA-256 {actual}, expected {expected}");
    }
    fs::rename(&part, &path).unwrap();
    path
}

/// `dir`, emptied of any earlier contents.
fn fresh_dir(dir: &Path) -> PathBuf {
    if dir.exists() {
        fs::remove_dir_all(dir).unwrap();
    }
    fs::create_dir_all(dir).unwrap();
    dir.to_path_buf()
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
