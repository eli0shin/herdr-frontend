# Third-party notices

herdr-frontend is MIT-licensed ([LICENSE](LICENSE)), except for the files below that keep another
license. This lists the code ported into it and everything it bundles, each with its license. Full
license texts are in [licenses/](licenses/) unless another place is given.

## z2d (MPL-2.0): the ported file stays MPL

- Source: [vancluever/z2d](https://github.com/vancluever/z2d) at
  [`40af3e84a4e15a9f31d126b3f281f23ce6a3dae5`](https://github.com/vancluever/z2d/tree/40af3e84a4e15a9f31d126b3f281f23ce6a3dae5)
  (z2d 0.13, the revision Ghostty 1.4.0 pins). Copyright © 2024-2026 Chris Marchesi.
- License: Mozilla Public License 2.0 ([licenses/MPL-2.0.txt](licenses/MPL-2.0.txt)). z2d's own
  notice, with the Cairo, Pixman and tiny-skia notices for the code it adapted:
  [licenses/z2d-LICENSE.txt](licenses/z2d-LICENSE.txt).

| File | Ported from | License |
| ---- | ----------- | ------- |
| `crates/app/src/view/terminal/sprite/raster.rs` | `src/painter.zig` `fill`, `src/internal/tess/fill_plotter.zig`, `Spline.zig`, `Polygon.zig`, `Pen.zig`, `src/internal/arc.zig`, `src/internal/raster/multisample.zig`, `src/compositor.zig` (source-over). `Spline.zig`, `arc.zig` and `Pen.zig` are themselves adapted from Cairo (© 2002 University of Southern California, used by z2d under the MPL). | MPL-2.0 |

That file carries the MPL header, says it was modified, and stays MPL-2.0: changes to it are
MPL-2.0 too, and a binary that contains it must come with a way to get its source. It is the only MPL file. The crate that holds it,
`herdr-frontend` (`crates/app`), is `MIT AND MPL-2.0` in its `Cargo.toml`.

## Herdr app (Apache-2.0)

- Source: [herdrdev/herdr-app](https://github.com/herdrdev/herdr-app). License: Apache License 2.0,
  [assets/programs/LICENSE-herdr-app.txt](assets/programs/LICENSE-herdr-app.txt). Herdr app has
  no `NOTICE` file.

Source files: ported from Herdr app at tag
[v0.1.18](https://github.com/herdrdev/herdr-app/tree/9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c)
(`9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c`; Herdr app paths below are at that revision) and
modified for herdr-frontend: rewritten onto its own types and crates, split differently, and
changed where its design differs. Each carries a notice saying so.

| File | Ported from (Herdr app path, functions) |
| ---- | --------------------------------------- |
| `crates/herdr-client/src/transport.rs` | `src/core_bin.rs` (finding herdr-core, `login_path_answer`) |
| `crates/herdr-shared/src/document.rs` | `src/mirror.rs` `Document`, `qualify`, `local`; `src/layout.rs` `Node`; `src/model.rs` `fresh_id` |
| `crates/herdr-shared/src/tree.rs` | `src/layout.rs` `Node`, `src/model.rs` `Model` (`move_pane`, `swap_panes`), `src/window/pane_drag.rs` `inherited_movable` |
| `crates/herdr-shared/src/intent.rs` | `src/window.rs` `adopt`; `src/model.rs` edits |
| `crates/herdr-shared/src/mirror.rs` | `src/mirror.rs`, `src/mirror_sync.rs` (reading and writing the layout document) |
| `crates/herdr-shared/src/adoption.rs` | `src/window.rs` `reconcile`, `start_adoption`, `adopt`, `adoptable` |
| `crates/herdr-shared/src/closed.rs` | `src/window.rs` `terminal_ended`, `check_closed`, `checked_closed`, `just_born`, `close_terminals`, `kill_targets`, `launch_failure` |
| `crates/herdr-shared/src/attention.rs` | `src/model.rs` `status`, `alert`, `alert_for`, `priority` |
| `crates/herdr-shared/src/labels.rs` | `src/window.rs` `share_seen`; `docs/labels.md` |
| `crates/terminal/src/links.rs` | `src/links.rs` `word` and its link pattern (itself Ghostty's, below) |
| `crates/model/src/links.rs` | `src/links.rs` `has_scheme`, `decode_file_url_path` |
| `crates/model/src/geometry.rs` | `src/window/pane_size.rs` `shares`, `block_bounds`; `src/window.rs` `nearest`, `drag_to`, `resize_pane`; `src/window/pane_drag.rs` |
| `crates/model/src/actions.rs` | `src/window.rs` `resize_pane`; `src/model.rs` `move_pane` |
| `crates/model/src/checkouts.rs` | `src/window.rs` `track_folders`, `folder_info`, `recheck` |
| `crates/model/src/control.rs` | `src/commands.rs` (the commands, `validate_args`, `shorthand`); `src/window.rs` `world`, `control`, `shown_agent`, `new_space`, `pane.close` |
| `crates/model/src/model.rs` | `src/window.rs` `keep_terminal`, `require_document`, `alert`, `notify_launch_failure`, `share_seen`, `track_folders`, `run_key`; `src/mirror.rs` `View::apply_machine` |
| `crates/model/src/peek.rs` | `src/window.rs` `peek_terminal`, `keep_terminal`; `src/links.rs` |
| `crates/model/src/shell.rs` | `src/window.rs` `agent_argv`, `unix_agent_argv`; `src/remote_shell.rs` `launch` |
| `crates/model/src/view.rs` | `src/mirror.rs` `View`; `src/model.rs` `Tab::focus` |
| `crates/model/src/world.rs` | `src/model.rs` (tab titles, folder names) |
| `crates/app/src/control.rs` | `src/control.rs` (the control socket, `parse_args`) |
| `crates/app/src/config/programs.rs` | `src/programs.rs` (the program table and how a terminal's program is resolved) |
| `crates/app/src/config/theme.rs` | `src/settings.rs` (the theme format and the colors derived from it) |
| `crates/app/src/keys.rs` | `src/terminal.rs` `map_key` (the shifted-character table and key mapping) |
| `crates/app/src/view/terminal/scrollbar.rs` | `src/terminal.rs` `scrollbar_thumb` |

Other files cite Herdr app's source (`app:` paths) only for behavior, file formats or constants
they match so both apps work on one layout, such as `docs/labels.md`'s labels, `settings.json`'s
key names, the find bar's size or a divider's handle width. They hold no code from it.

Assets, copied as they are:

| File | Copied from | Changed? |
| ---- | ----------- | -------- |
| `assets/programs/herdr.json` | [`defaults/programs.json`](https://github.com/herdrdev/herdr-app/blob/1180a5cec3b8bd4dfebf4a9b217ab8cb59a96ffa/defaults/programs.json) at [`1180a5c`](https://github.com/herdrdev/herdr-app/tree/1180a5cec3b8bd4dfebf4a9b217ab8cb59a96ffa) | Unchanged |
| `assets/icons/` | [`defaults/icons/`](https://github.com/herdrdev/herdr-app/tree/1180a5cec3b8bd4dfebf4a9b217ab8cb59a96ffa/defaults/icons) at `1180a5c` | Unchanged, except the last line of `LICENSES.md`, which now points here. The icons' own licenses are under "Icons" below. |

## Ghostty (MIT)

- Source: [ghostty-org/ghostty](https://github.com/ghostty-org/ghostty) at tag
  [`vt-1.0.2`](https://github.com/ghostty-org/ghostty/tree/573c4a21e02311ae87844af26b6a7a179f82858e)
  (`573c4a21`) and tag [`v1.4.0`](https://github.com/ghostty-org/ghostty/tree/83fb95624b0439ee660c2679b43b38c8b63769cd)
  (`83fb9562`). Copyright (c) 2024 Mitchell Hashimoto, Ghostty contributors.
- License: MIT, [licenses/Ghostty-MIT.txt](licenses/Ghostty-MIT.txt).

Ported, and modified for Rust and Rune:

| File | Ported from (Ghostty path) | Tag |
| ---- | -------------------------- | --- |
| `crates/terminal/src/metrics.rs` | `src/font/Metrics.zig` | vt-1.0.2 |
| `crates/terminal/src/face.rs` | `src/font/face/coretext.zig` `getMetrics` | vt-1.0.2 |
| `crates/terminal/src/size.rs` | `src/renderer/size.zig`, the padding parts of `src/Surface.zig` | vt-1.0.2 |
| `crates/terminal/src/links.rs` | `src/config/url.zig` (the URL and path pattern and its test cases) | vt-1.0.2 |
| `crates/app/src/view/terminal/rows.rs` | `rebuildRow`'s color and run rules | vt-1.0.2 |
| `crates/app/src/view/terminal/cursor.rs` | `src/font/sprite/draw/special.zig` `cursor_*` | vt-1.0.2 |
| `crates/app/src/view/terminal/glyph/constraint.rs` | `src/font/Glyph.zig` `RenderOptions.Constraint`, `src/renderer/cell.zig`, `src/font/SharedGrid.zig`, `src/font/nerd_font_attributes.zig` | v1.4.0 |
| `crates/app/src/view/terminal/glyph/tables.rs` | Generated by `scripts/gen_ghostty_tables.py` from `src/font/nerd_font_tables.zig`, and from Unicode data (see "Unicode") | v1.4.0 |
| `crates/app/src/view/terminal/glyph/discovery.rs` | `src/font/discovery.zig` | v1.4.0 |
| `crates/app/src/view/terminal/glyph/coretext.rs` | `src/font/face/coretext.zig` `renderGlyph`, `src/font/Collection.zig` `scaleFactor` | v1.4.0 |
| `crates/app/src/config/fonts.rs` | `src/font/SharedGridSet.zig` `Key.init` and `collection` | v1.4.0 |
| `crates/app/src/view/terminal/sprite/mod.rs` | `src/font/sprite/Face.zig` | v1.4.0 |
| `crates/app/src/view/terminal/sprite/canvas.rs` | `src/font/sprite/canvas.zig` | v1.4.0 |
| `crates/app/src/view/terminal/sprite/box_drawing.rs` | `src/font/sprite/draw/box.zig` | v1.4.0 |
| `crates/app/src/view/terminal/sprite/blocks.rs` | `src/font/sprite/draw/block.zig`, `braille.zig`, `geometric_shapes.zig` | v1.4.0 |
| `crates/app/src/view/terminal/sprite/powerline.rs` | `src/font/sprite/draw/powerline.zig` | v1.4.0 |
| `crates/app/src/view/terminal/sprite/special.rs` | `src/font/sprite/draw/special.zig` (underlines, strikethrough, overline) | v1.4.0 |
| `crates/ghostty-vt-sys/src/bindings.rs` | Generated by bindgen from libghostty-vt's headers (`include/ghostty/vt/`) | vt-1.0.2 |

## libghostty-vt and what it links (statically linked into every build)

`crates/ghostty-vt-sys/build.rs` downloads Ghostty's prebuilt libghostty-vt 1.0.2 xcframework and
links its static library into the binary. The library is Ghostty (MIT, above), plus the
third-party code Ghostty lists as linked into it
([`deps/libghostty-vt-third-party.json`](https://github.com/ghostty-org/ghostty/blob/573c4a21e02311ae87844af26b6a7a179f82858e/deps/libghostty-vt-third-party.json))
and Zig's compiler runtime:

| Component | Source and revision | License | Texts |
| --------- | ------------------- | ------- | ----- |
| Highway 1.2.0 | [google/highway](https://github.com/google/highway/tree/66486a10623fa0d72fe91260f96c892e41aceb06) `66486a10` | Apache-2.0 OR BSD-3-Clause | [highway-Apache-2.0.txt](licenses/highway-Apache-2.0.txt), [highway-BSD-3-Clause.txt](licenses/highway-BSD-3-Clause.txt) |
| simdutf 9.0.0 | [simdutf/simdutf](https://github.com/simdutf/simdutf/tree/v9.0.0) `v9.0.0` | Apache-2.0 OR MIT | [simdutf-Apache-2.0.txt](licenses/simdutf-Apache-2.0.txt), [simdutf-MIT.txt](licenses/simdutf-MIT.txt) |
| translate-c 2.0.0 | [ziglang/translate-c](https://github.com/ziglang/translate-c), Ghostty's [`02ff0c52` archive](https://deps.files.ghostty.org/translate_c-02ff0c523fb92a38939b04d0145c4a9602a74d56.tar.gz) | MIT | [translate-c-MIT.txt](licenses/translate-c-MIT.txt) |
| uucode 0.2.0 | [jacobsandlund/uucode](https://github.com/jacobsandlund/uucode/tree/1fb73433bba5d93366c57f23ff2e9d7939746500) `1fb73433` | MIT AND Unicode-3.0 | [uucode-LICENSE.md](licenses/uucode-LICENSE.md), [uucode-LICENSE_unicode.txt](licenses/uucode-LICENSE_unicode.txt), [uucode-LICENSE_Bjoern_Hoehrmann.txt](licenses/uucode-LICENSE_Bjoern_Hoehrmann.txt) |
| Wuffs 0.4.0-alpha.10 | [google/wuffs](https://github.com/google/wuffs), Ghostty's [`7411f488` archive](https://deps.files.ghostty.org/wuffs-7411f488fe2e2c205c3d3b3d28638b7356522930.tar.gz) | Apache-2.0 OR MIT | [wuffs-LICENSE.txt](licenses/wuffs-LICENSE.txt), [wuffs-Apache-2.0.txt](licenses/wuffs-Apache-2.0.txt), [wuffs-MIT.txt](licenses/wuffs-MIT.txt) |
| Zig compiler runtime (`compiler_rt`) | [ziglang/zig](https://codeberg.org/ziglang/zig), the Zig that built the release (0.17.0 or later, Ghostty's minimum; the text is from [0.17.0](https://codeberg.org/ziglang/zig/src/tag/0.17.0)) | MIT | [Zig-MIT.txt](licenses/Zig-MIT.txt) |

## Rune (MIT, dependency)

- [anomalyco/rune](https://github.com/anomalyco/rune) at
  [`4918969f04dce020f63b34e024368a86e5e1857d`](https://github.com/anomalyco/rune/tree/4918969f04dce020f63b34e024368a86e5e1857d),
  the UI framework (`rune`, `rune-kit`). Copyright (c) 2026 Anomaly. MIT,
  [licenses/Rune-MIT.txt](licenses/Rune-MIT.txt). Its forks of winit, wgpu-hal, naga,
  parley_engine and accesskit, which `Cargo.toml` patches in from the same repository, keep their
  upstream licenses (see "Rust crates").
- `scripts/check_rune_pins.py` is copied unchanged from Rune's `scripts/check_rune_pins.py` at that
  revision and carries Rune's license in its header.

## Unicode data (Unicode-3.0)

`crates/app/src/view/terminal/glyph/tables.rs` holds the `Emoji_Presentation` ranges of Unicode's
`emoji-data.txt` (the copy in the uucode Ghostty v1.4.0 pins). Unicode License v3,
[licenses/Unicode-3.0.txt](licenses/Unicode-3.0.txt).

## Fonts (embedded in the binary)

| Font | Files | Source | License |
| ---- | ----- | ------ | ------- |
| JetBrains Mono 2.304 | `assets/fonts/JetBrainsMono-Variable.ttf`, `assets/fonts/JetBrainsMono-Italic-Variable.ttf` | [JetBrains/JetBrainsMono](https://github.com/JetBrains/JetBrainsMono) | SIL Open Font License 1.1, [assets/fonts/OFL-JetBrainsMono.txt](assets/fonts/OFL-JetBrainsMono.txt) |
| Symbols Nerd Font 3.4.0 | `assets/fonts/SymbolsNerdFont-Regular.ttf` | [ryanoasis/nerd-fonts](https://github.com/ryanoasis/nerd-fonts) | MIT, [assets/fonts/MIT-SymbolsNerdFont.txt](assets/fonts/MIT-SymbolsNerdFont.txt) |
| Geist Mono (tests only, not in the app) | `crates/terminal/tests/fonts/GeistMono-Variable.ttf` | [vercel/geist-font](https://github.com/vercel/geist-font) | SIL Open Font License 1.1, [crates/terminal/tests/fonts/OFL-GeistMono.txt](crates/terminal/tests/fonts/OFL-GeistMono.txt) |

## Icons (embedded in the binary)

- `assets/icons/`: Herdr app's icons (above), under their own licenses, listed in
  [assets/icons/LICENSES.md](assets/icons/LICENSES.md): Simple Icons (CC0 1.0,
  [licenses/CC0-1.0.txt](licenses/CC0-1.0.txt)), Lucide (ISC, with MIT Feather icons,
  [assets/icons/LICENSE-Lucide.txt](assets/icons/LICENSE-Lucide.txt)), LobeHub icons (MIT,
  [assets/icons/LICENSE-LobeHub.txt](assets/icons/LICENSE-LobeHub.txt)) and oh-my-pi (MIT,
  [assets/icons/LICENSE-oh-my-pi.txt](assets/icons/LICENSE-oh-my-pi.txt)). Brands are their
  owners' trademarks; these licenses grant no trademark rights.
- `assets/program-icons/neovim.svg`: the Neovim logo from
  [devicons/devicon](https://github.com/devicons/devicon/blob/32fe1d3b3f36ad11f90ab94f7db59738139c2bf6/icons/neovim/neovim-original.svg).
  MIT, [licenses/devicon-MIT.txt](licenses/devicon-MIT.txt).

## Rust crates

The other crates in `Cargo.lock` are dependencies Cargo fetches at build time, each under its own
license (mostly MIT or Apache-2.0), and they are compiled into the binary. herdr-frontend ships no
binaries yet, so it doesn't generate a notice file for them; a binary release must include their
notices.

## Not third-party

Files described as ports of `cfg/mods/...` (`crates/model/src/filter.rs`, `folders.rs`, `prs.rs`)
port Eli Oshinsky's own Herdr app mods from his dotfiles, as do `assets/programs/eli.json` and
`assets/themes/arctic.json`. They are his work and fall under the MIT License above.
