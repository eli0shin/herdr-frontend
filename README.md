# herdr-frontend

A native desktop client for [herdr-core](https://github.com/herdrdev/herdr-core), written in Rust on
[Rune](https://github.com/anomalyco/rune), with terminals built on Ghostty's `libghostty-vt`.

It is a second front end on the same herdr-core as Herdr app: the same machines, agents, terminals,
spaces and tabs, so you can switch between the two at any time. `docs/interop.md` covers how the two
share a terminal's size, and has a checklist for trying both at once.

Status: under construction. See `docs/` for research and the plan.

## Install

```sh
just install
```

This builds the release profile, bundles `herdr-frontend.app` (the binary, its icon and an
`Info.plist`, signed ad hoc) and installs it to `~/Applications`. It also links the
`herdr-frontend` command (the window, and its `ls | do | status` CLI, `docs/cli.md`) into
`~/.local/bin`, which must be on your `PATH`. `HERDR_FRONTEND_APP_DIR` and `HERDR_FRONTEND_BIN_DIR`
move either place. Run it again to update: it swaps the bundle, so a running window keeps going
until you quit and reopen it.

The app has its own name and bundle identifier (`dev.herdr.frontend`), so it installs beside
Herdr.app and never replaces it. Opened from the Dock or Finder, it finds `hc` as Herdr app does: on
`PATH`, in the usual install places, then on your login shell's `PATH`.

`just bundle` builds `target/release/herdr-frontend.app` without installing it. The release
profile links the whole program as one (fat LTO): a cold build takes about 6 minutes, and a rebuild
after an edit about 2. The ad hoc signature runs a local build, but Gatekeeper would stop a copy
downloaded from elsewhere.

## Develop

macOS only for now. Needs [rustup](https://rustup.rs) (the toolchain comes from
`rust-toolchain.toml`) and [just](https://just.systems).

```sh
just run          # open the window on your herdr-core
just test         # pin check, fmt, clippy, tests and bindings check, as CI runs them
just snap [scene] # render a scene headless to target/snap/<scene>.png
just bundle       # build target/release/herdr-frontend.app (`just install` installs it)
just icon         # regenerate assets/icon/herdr-frontend.icns from its SVG (needs ImageMagick)
just e2e          # the full app and Herdr app through every scenario, screenshots side by side
just ghostty-diff # compare the terminal renderer with Ghostty.app (opens Ghostty windows)
just pane-budget  # time many terminal panes changing at once against the 120 Hz frame
just check-pins   # check the Rune pin against Rune's patch list at that rev
just bindings     # regenerate the libghostty-vt bindings (`--check` fails on a diff)
```

`just run` connects with `hc connect`, as Herdr app does: the core `HC_SESSION` names, or your
default one. It reads the shared layout and never writes it except for edits it makes itself
(adopting a labeled terminal, say). Its own files (this window's view and place, the Agents
list's shared state, and its `settings.json`, themes and programs) live in
`$HERDR_FRONTEND_HOME`, else `~/.config/herdr-frontend`. With no settings it looks like Eli's
Herdr app: arctic, Operator Mono Book 17 pt (the installed family, else JetBrains Mono;
`docs/settings.md` "Fonts"), zoom 1.1; edits apply while it runs. `docs/settings.md` lists every
key. The Agents list's sort, space colors, machine filter and the sidebar's width start from
defaults and live only in that home (`docs/sidebar.md`). `--snap`, `--tree` and `--script` never connect
to a core and read no settings. Release builds keep those three flags too (Rune's `tooling`
feature).

The first build downloads the pinned libghostty-vt xcframework (16 MB), checks its SHA-256 and
caches it in `~/Library/Caches/ghostty-vt-sys` (set `GHOSTTY_VT_CACHE_DIR` to move it). It needs
`curl`, `unzip` and `lipo` from the Xcode Command Line Tools, but no Zig. The bindings in
`crates/ghostty-vt-sys/src/bindings.rs` are committed, so only `just bindings` needs libclang.

herdr-client's tests start their own isolated herdr-core (a `test-*` session with scratch dirs under
`/tmp`), never yours. They use the `hc` on your machine, or `HERDR_TEST_HC=/path/to/herdr-core`, and
skip without one. CI installs the pinned hc release.

`just e2e` opens real windows on your display, each app on its own isolated core and home, and
writes `target/e2e/<run>/index.html`. `docs/e2e.md` describes what it runs and what it never
touches.

Rune is pinned to one `dev` rev in `Cargo.toml`; bump it as `docs/research/rune.md` §1 describes.

## License

MIT ([LICENSE](LICENSE)), copyright Eli Oshinsky. herdr-frontend ports code from Ghostty (MIT),
Herdr app (Apache-2.0) and z2d (MPL-2.0), and bundles libghostty-vt, fonts and icons under their
own licenses. [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) lists each with its license. One
source file keeps a license other than MIT: `crates/app/src/view/terminal/sprite/raster.rs`, the
z2d port, is MPL-2.0.
