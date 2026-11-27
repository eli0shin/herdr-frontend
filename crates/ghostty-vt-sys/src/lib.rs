//! libghostty-vt bindings and the link of the pinned, checksummed release.
//!
//! `build.rs` fetches the pinned release, checks its SHA-256 and statically links it: the
//! xcframework's macOS library, or on Linux the library it builds from the release's source. The raw bindings are bindgen's output for `ghostty/vt.h`, committed in
//! `src/bindings.rs` and regenerated with `just bindings`. Everything here is `unsafe` C API
//! except [`check_version`], which a program calls once when it starts.

use std::ffi::CStr;
use std::fmt;

pub use bindings::*;

/// bindgen's output, `include!`d so rustfmt leaves it as generated.
#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unsafe_op_in_unsafe_fn,
    clippy::all
)]
mod bindings {
    include!("bindings.rs");
}

/// The release `build.rs` pins and links.
pub const PINNED_VERSION: &str = env!("GHOSTTY_VT_PINNED_VERSION");

/// The linked library's version, as `ghostty_build_info` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: usize,
    pub minor: usize,
    pub patch: usize,
    /// The full version string, such as `1.0.2` or `1.1.0-dev+abc1234`.
    pub full: String,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.full)
    }
}

/// The linked library can't be used with the headers the bindings came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionMismatch {
    pub library: Version,
    pub headers: &'static str,
}

impl fmt::Display for VersionMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libghostty-vt {} is linked, but the bindings need {}.x at {} or newer",
            self.library, GHOSTTY_VT_VERSION_MAJOR, self.headers
        )
    }
}

impl std::error::Error for VersionMismatch {}

/// Reads the linked library's version.
pub fn library_version() -> Version {
    // SAFETY: each key's output type is `size_t`, and `ghostty_version` returns a static,
    // NUL-terminated string (`build_info.h`).
    unsafe {
        let number = |key| {
            let mut n = 0usize;
            let result = ghostty_build_info(key, (&raw mut n).cast());
            assert_eq!(result, GHOSTTY_SUCCESS, "ghostty_build_info({key})");
            n
        };
        Version {
            major: number(GHOSTTY_BUILD_INFO_VERSION_MAJOR),
            minor: number(GHOSTTY_BUILD_INFO_VERSION_MINOR),
            patch: number(GHOSTTY_BUILD_INFO_VERSION_PATCH),
            full: CStr::from_ptr(ghostty_version())
                .to_string_lossy()
                .into_owned(),
        }
    }
}

/// Checks the linked library against the headers the bindings were generated from, as
/// `stability.md` "Checking the Version" says to at startup: the major versions match, and the
/// library is at least the headers' minor version.
pub fn check_version() -> Result<Version, VersionMismatch> {
    let library = library_version();
    let compatible = library.major == GHOSTTY_VT_VERSION_MAJOR as usize
        && library.minor >= GHOSTTY_VT_VERSION_MINOR as usize;
    if compatible {
        Ok(library)
    } else {
        Err(VersionMismatch {
            library,
            headers: header_version(),
        })
    }
}

/// The headers' version string (`GHOSTTY_VT_VERSION`), without bindgen's trailing NUL.
pub fn header_version() -> &'static str {
    CStr::from_bytes_with_nul(GHOSTTY_VT_VERSION)
        .expect("GHOSTTY_VT_VERSION is NUL-terminated")
        .to_str()
        .expect("GHOSTTY_VT_VERSION is ASCII")
}
