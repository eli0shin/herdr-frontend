# herdr-frontend

A native desktop client for [herdr-core](https://github.com/herdrdev/herdr-core), written in Rust on
[Rune](https://github.com/anomalyco/rune), with terminals built on Ghostty's `libghostty-vt`.

It is a second front end on the same herdr-core as Herdr app: the same machines, agents, terminals,
spaces and tabs, so you can switch between the two at any time. `docs/interop.md` covers how the two
share a terminal's size, and has a checklist for trying both at once.

Status: under construction. See `docs/` for research and the plan.

## Install

Each [GitHub Release](https://github.com/eli0shin/herdr-frontend/releases) has
`herdr-frontend-<version>-macos.dmg`, the app for Apple silicon Macs (there's no Intel build),
notarized by Apple (the release notes say so when it isn't). Open it, drag herdr-frontend into
Applications, and link the command:

```sh
ln -sfn /Applications/herdr-frontend.app/Contents/MacOS/herdr-frontend ~/.local/bin/herdr-frontend
```

From a checkout:

```sh
just install
```

This builds the release profile, bundles `herdr-frontend.app` (the binary, its icon, an
`Info.plist` and the license files, signed ad hoc) and installs it to `~/Applications`. It also links the
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

### With AeroSpace

The window is borderless like Ghostty's with `window-decoration = false`: no title bar, no traffic
lights, square corners. [AeroSpace](https://github.com/nikitabobko/AeroSpace) floats a window that
has no full-screen button, as it does dialogs, unless it knows the app by its ID (Ghostty, kitty,
Alacritty and a few more). herdr-frontend isn't one of them, so to have AeroSpace tile it like any
other window, add this to `aerospace.toml`:

```toml
[[on-window-detected]]
if.app-id = "dev.herdr.frontend"
run = "layout tiling"
```

(or the same table inside an existing `on-window-detected = [...]` list). AeroSpace then sets the
window's frame. herdr-frontend saves its size and position only when it closes or quits, never
reopens zoomed or full screen, and a double click on its tab strip or the sidebar's top row does
nothing (unless System Settings › Desktop & Dock says what that double click does), so it doesn't
fight the layout (`crates/app/src/window.rs`).

## Develop

macOS on Apple silicon only. Needs [rustup](https://rustup.rs) (the toolchain comes from
`rust-toolchain.toml`) and [just](https://just.systems).

```sh
just run          # open the window on your herdr-core
just test         # pin check, fmt, clippy, tests and bindings check, as CI runs them
just snap [scene] # render a scene headless to target/snap/<scene>.png
just bundle       # build target/release/herdr-frontend.app (`just install` installs it)
just package      # build the release .dmg, arm64, in target/dist/ (docs/release.md)
just icon         # regenerate assets/icon/herdr-frontend.icns from its SVG (needs ImageMagick)
just e2e          # the full app and Herdr app through every scenario, screenshots side by side
just ghostty-diff # compare the terminal renderer with Ghostty.app (opens Ghostty windows)
just pane-budget  # time many terminal panes changing at once against the 120 Hz frame
just check-pins   # check the Rune pin against Rune's patch list at that rev
just bindings     # regenerate the libghostty-vt bindings (`--check` fails on a diff)
```

`just run` connects with `hc connect`, as Herdr app does: the core `HC_SESSION` names, or your
default one. It reads the shared layout and never writes it except for edits it makes itself
(adopting a labeled terminal, say). Its config (`settings.json`, `keys.json`, themes and
programs) lives in `~/.config/herdr-frontend` and is never written by the app, so it can live in
dotfiles. What the app writes (this window's view, place and zoom, the Agents list's shared state,
the control socket) lives in `~/.local/state/herdr-frontend` (`docs/settings.md` "Config and
state"; `$XDG_CONFIG_HOME` and `$XDG_STATE_HOME` move them, and `$HERDR_FRONTEND_HOME` is one
folder for both). With no settings it looks like Eli's
Herdr app: arctic, Operator Mono Book 17 pt (the installed family, else JetBrains Mono;
`docs/settings.md` "Fonts"), zoom 1.1; edits apply while it runs. `docs/settings.md` lists every
key. The Agents list's sort, space colors, machine filter and the sidebar's width start from
defaults and live only in its state folder (`docs/sidebar.md`). `--snap`, `--tree` and `--script` never connect
to a core and read no settings. Release builds keep those three flags too (Rune's `tooling`
feature).

The first build downloads the pinned libghostty-vt xcframework (16 MB), checks its SHA-256 and
caches it in `~/Library/Caches/ghostty-vt-sys` (set `GHOSTTY_VT_CACHE_DIR` to move it). It needs
`curl`, `unzip` and `lipo` from the Xcode Command Line Tools, but no Zig. The bindings in
`crates/ghostty-vt-sys/src/bindings.rs` are committed, so only `just bindings` needs libclang.
On Linux, which CI builds and tests on, the first build instead downloads the release's source
and the pinned Zig, checks them and builds the library once into `~/.cache/ghostty-vt-sys`
(`crates/ghostty-vt-sys/build.rs`).

herdr-client's tests start their own isolated herdr-core (a `test-*` session with scratch dirs under
`/tmp`), never yours. They use the `hc` on your machine, or `HERDR_TEST_HC=/path/to/herdr-core`, and
skip without one. CI installs the pinned hc release.

`just e2e` opens real windows on your display, each app on its own isolated core and home, and
writes `target/e2e/<run>/index.html`. `docs/e2e.md` describes what it runs and what it never
touches.

Rune is pinned to one `dev` rev in `Cargo.toml`; bump it as `docs/research/rune.md` §1 describes.

Releases are versioned with Changesets, which needs [Bun](https://bun.sh): each user-visible PR
adds a changeset (`bun install` once, then `bunx changeset`), and merging the "chore: version
packages" PR tags the version and creates its GitHub Release, and a macOS runner builds, signs,
notarizes and uploads its .dmg. `herdr-frontend --version` prints it. `docs/release.md` has the
whole flow.

## License

MIT ([LICENSE](LICENSE)), copyright Eli Oshinsky. herdr-frontend ports code from Ghostty (MIT),
Herdr app (Apache-2.0) and z2d (MPL-2.0), and bundles libghostty-vt, fonts and icons under their
own licenses. [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) lists each with its license. One
source file keeps a license other than MIT: `crates/app/src/view/terminal/sprite/raster.rs`, the
z2d port, is MPL-2.0.
