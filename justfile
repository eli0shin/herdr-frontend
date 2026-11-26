# herdr-frontend tasks. The toolchain comes from rust-toolchain.toml.

bin := "target/debug/herdr-frontend"

# List the recipes.
default:
    @just --list

# Build every crate.
build:
    cargo build

# Run the pin check, fmt, clippy, the tests and the bindings check, as CI does.
test: check-pins
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

# Check the Rune pin against Rune's own patch list at the pinned rev.
check-pins:
    scripts/check-rune-pins

# Regenerate crates/ghostty-vt-sys/src/bindings.rs from the pinned headers (`--check` fails on a diff).
bindings *args:
    cargo run --locked -p ghostty-vt-sys --features bindgen --bin ghostty-vt-bindgen -- {{args}}
