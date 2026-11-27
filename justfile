# herdr-frontend tasks. The toolchain comes from rust-toolchain.toml.

bin := "target/debug/herdr-frontend"

# List the recipes.
default:
    @just --list

# Build every crate.
build:
    cargo build

# Run the pin and version checks, fmt, clippy, the tests and the bindings check, as CI does.
test: check-pins
    scripts/sync-version --check
    cargo fmt --all -- --check
    cargo clippy --all-targets --locked -- -D warnings
    cargo test --locked
    cargo clippy --locked -p ghostty-vt-sys --all-targets --features bindgen -- -D warnings
    just bindings --check

# Open the app window.
run *args:
    cargo run -p herdr-frontend -- {{args}}

# Render a scene headless to target/snap/<scene>.png (`just snap --list` lists scenes).
snap scene="hello" *args: build
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{scene}}" = "--list" ]; then exec {{bin}} --list-scenes; fi
    mkdir -p target/snap
    {{bin}} --snap target/snap/{{scene}}.png --scene {{scene}} {{args}}
    echo target/snap/{{scene}}.png

# Run the full app and Herdr app through the scenarios on isolated cores, screenshot each step
# into target/e2e/<run>/ (opens windows on your display; docs/e2e.md).
e2e *args:
    HERDR_E2E=1 cargo test -p herdr-frontend --test e2e -- {{args}}

# Compare the terminal renderer with Ghostty.app, pixel by pixel (opens a Ghostty window per screen).
ghostty-diff *screens:
    scripts/ghostty-diff {{screens}}

# Build target/release/herdr-frontend.app: the release build, icon and Info.plist, signed ad hoc.
bundle:
    scripts/bundle-macos

# Build the release download, target/dist/herdr-frontend-<version>-macos.dmg and .sha256 (universal; docs/release.md).
package:
    scripts/release-macos

# Run `just package`'s Developer ID signing with a throwaway self-signed identity, into target/dist-dry-run/.
package-dry-run:
    scripts/release-signing-dry-run

# Install herdr-frontend.app to ~/Applications and the herdr-frontend command to ~/.local/bin.
install:
    scripts/install-macos

# Regenerate assets/icon/herdr-frontend.icns from its SVG (needs ImageMagick).
icon:
    scripts/make-icon

# Time N terminal panes all changing at once against the 120 Hz frame (renderer gate 3).
pane-budget:
    cargo run --release -p herdr-frontend --example pane-budget

# Check the Rune pin against Rune's own patch list at the pinned rev.
check-pins:
    scripts/check-rune-pins

# Regenerate crates/ghostty-vt-sys/src/bindings.rs from the pinned headers (`--check` fails on a diff).
bindings *args:
    cargo run --locked -p ghostty-vt-sys --features bindgen --bin ghostty-vt-bindgen -- {{args}}

# Run what CI runs, on this Mac, so the macOS-only code and tests are checked too: prints `local-ci: PASS <sha>`.
ci:
    scripts/local-ci
