# herdr-frontend

A native desktop client for [herdr-core](https://github.com/herdrdev/herdr-core), written in Rust on
[Rune](https://github.com/anomalyco/rune), with terminals built on Ghostty's `libghostty-vt`.

It is a second front end on the same herdr-core as Herdr app: the same machines, agents, terminals,
spaces and tabs, so you can switch between the two at any time.

Status: under construction. See `docs/` for research and the plan.

## Develop

macOS only for now. Needs [rustup](https://rustup.rs) (the toolchain comes from
`rust-toolchain.toml`) and [just](https://just.systems).

```sh
just run          # open the window
just test         # pin check, fmt, clippy, tests and bindings check, as CI runs them
just snap [scene] # render a scene headless to target/snap/<scene>.png
just ghostty-diff # compare the terminal renderer with Ghostty.app (opens Ghostty windows)
just pane-budget  # time many terminal panes changing at once against the 120 Hz frame
just check-pins   # check the Rune pin against Rune's patch list at that rev
just bindings     # regenerate the libghostty-vt bindings (`--check` fails on a diff)
```

The first build downloads the pinned libghostty-vt xcframework (16 MB), checks its SHA-256 and
caches it in `~/Library/Caches/ghostty-vt-sys` (set `GHOSTTY_VT_CACHE_DIR` to move it). It needs
`curl`, `unzip` and `lipo` from the Xcode Command Line Tools, but no Zig. The bindings in
`crates/ghostty-vt-sys/src/bindings.rs` are committed, so only `just bindings` needs libclang.

herdr-client's tests start their own isolated herdr-core (a `test-*` session with scratch dirs under
`/tmp`), never yours. They use the `hc` on your machine, or `HERDR_TEST_HC=/path/to/herdr-core`, and
skip without one. CI installs the pinned hc release.

Rune is pinned to one `dev` rev in `Cargo.toml`; bump it as `docs/research/rune.md` §1 describes.
