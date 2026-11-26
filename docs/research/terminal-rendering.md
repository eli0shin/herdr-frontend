# Terminal rendering: full libghostty vs libghostty-vt with our own renderer

Ticket `003-research-terminal-rendering-with-libghostty-vt`. Researched 2026-11-26.

## Recommendation

Build on **libghostty-vt 1.0.2** for terminal state, render state, selection, search and input
encoding. Render the terminals ourselves inside the Rune window, porting Ghostty's renderer
logic: `Metrics.calc`, `rebuildRow`'s run and color rules, the sprite font, and glyph
placement. Don't embed Ghostty's surface (`ghostty.h`), upstream or forked.

Where our renderer draws:

1. **First, on Rune's `rune::grid` element.** It's "a terminal's screen as one element". It
   caches each row, has accessibility, and is already measured, and opencode-rune draws its
   libghostty-vt terminal with it.
2. **If grid fails the fidelity or many-pane gates in [§2](#where-the-renderer-draws), move to
   a custom wgpu pass** that a Rune `Compositor` paints at `El::effect` slots, porting Ghostty's
   cell shaders.

Ticket 004 (PR #2) reaches the same order.

Why, in short:

- **It's the only supported option.** `ghostty.h` calls itself internal and "not designed for
  external use". The maintainers have said the same thing three times, most recently on
  2026-11-23: "this isn't a publicly supported API ... You should use libghostty-vt if you want
  proper 3rd party support". libghostty-vt has had a stable, semver C ABI since 1.0.0
  (2026-11-06), and CI enforces it.
- **The surface can't take bytes from herdr-core.** Upstream termio has one backend, `exec`,
  which spawns a local PTY. Every app that feeds a surface from the network (cmux, Geistty,
  gterm, VVTerm) carries a fork that adds an IO backend.
- **The surface can't composite inside a Rune window.** It draws into an `NSView` or `UIView`
  that it owns, through its own `IOSurfaceLayer` and Metal device. It supports only macOS and
  iOS. Overlays above panes, transforms, clipping and fades would all have to be native view
  tricks.
- **Fidelity comes from the data, and libghostty-vt has it.** Ghostty's own renderer reads the
  same `terminal.RenderState` that libghostty-vt's C render API wraps. Porting its cell-building
  code is a translation, not a reverse-engineering job.
- **Many panes are cheaper.** Each Ghostty surface brings its own renderer thread, IO thread and
  swap chain. Our panes are elements in Rune's one frame, sharing its text system and atlas.
  The grid measures 0.40 ms per frame with one changed row on a 200×60 screen.
- **iOS reuse is better.** The libghostty-vt xcframework ships iOS slices, and Rune's
  `LayerHost` runs a Rune runtime on a `CAMetalLayer`, so the same renderer and core run on iOS.

Discussion #12131 hasn't changed. Since the May 2026 "just write your own renderer" answer,
libghostty-vt went 1.0 and gained the renderer-facing features you'd want: row identity,
overscan, two-phase updates, render holds and image placements. Upstream main still has no
embeddable surface IO and no new rendering package. Details are in
[§0](#0-full-libghostty-vs-libghostty-vt).

## Sources and revisions

Every path below is at these revisions unless a different one is given.

| Source                                   | Revision                                                                                                      |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Ghostty (`ghostty:`)                     | tag `vt-1.0.2` = `573c4a21e02311ae87844af26b6a7a179f82858e` (2026-11-24). Main checked at `69023b8169` (2026-11-26) |
| herdr-app (`herdr-app:`)                 | `b721deda985270ec8256552692729a4add389b2a`. Its `vendor/ghostty` pin is `291fd050b9e9c293c8adb6a140742649b7e42d3a` (2026-10-14) |
| anomalyco/opencode-rune (`opencode-rune:`) | `7420a0a1330622526f716963f9d1afa2a0bd85af` (dev, 2026-11-25). Pins Ghostty `v1.4.0` = `83fb95624b`          |
| anomalyco/rune (`rune:`)                 | `4918969f04dce020f63b34e024368a86e5e1857d` (dev, 2026-11-25)                                                    |
| herdr-core protocol                      | `hc schema`, herdr-core 0.1.22 (read only)                                                                      |
| Ghostty discussions                      | [#12131](https://github.com/ghostty-org/ghostty/discussions/12131), [#15254](https://github.com/ghostty-org/ghostty/discussions/15254), [#15295](https://github.com/ghostty-org/ghostty/discussions/15295) |
| Forks                                    | [manaflow-ai/cmux `docs/ghostty-fork.md`](https://github.com/manaflow-ai/cmux/blob/27810b92/docs/ghostty-fork.md), [daiimus/geistty `ARCHITECTURE.md`](https://github.com/daiimus/geistty/blob/main/ARCHITECTURE.md), [madeye/gterm `PLAN.md`](https://github.com/madeye/gterm/blob/main/PLAN.md) |

The libghostty-vt guides are Doxygen pages in `ghostty:dist/doxygen/pages/guides/*.md`. They're
published at <https://lib.ghostty.org/vt/1.0.2/>.

## 0. Full libghostty vs libghostty-vt

### What each option is

**(a) Full libghostty** is `include/ghostty.h`, the embedder API the macOS app uses
(`ghostty_app_*`, `ghostty_surface_*`). A surface owns its terminal, font stack, renderer and IO.
It comes in three flavors:

1. **Upstream.** Its only termio backend is `exec`: `ghostty:src/termio/backend.zig:14`
   (`pub const Kind = enum { exec };`). There is no way to give a surface bytes from herdr-core.
2. **A fork with manual or external IO.** Each of these adds a termio backend and C entry points
   for "here are output bytes" and "send these input bytes":
   - cmux (`manaflow-ai/ghostty`) adds `src/termio/Manual.zig`, `GHOSTTY_SURFACE_IO_MANUAL`,
     `io_write_cb` and `ghostty_surface_process_output`. Its fork notes say that upstream
     `4dcb09ada` "does not expose equivalent libghostty surface IO selection, write callback ...
     or output C API". cmux also documents a long list of other fork patches and a prebuilt
     xcframework pipeline with pinned checksums to maintain them.
   - Geistty (`daiimus/ghostty`, branch `ios-external-backend`) adds `src/termio/External.zig`
     (about 380 lines), `ghostty_surface_write_output()`, a write callback and a resize callback.
     Its README calls the project "essentially abandonware".
   - gterm (`madeye/ghostty`) adds a `passthru` backend with `ghostty_surface_pty_data()`, and
     the app also patches libghostty's event loop for iOS.
3. **A newer official rendering package.** There isn't one. The only library option in
   `ghostty:src/build/Config.zig:114` is `emit-lib-vt`. Everything else builds the app or the
   internal xcframework. The header still says "External embedders should instead use
   `libghostty-vt`" (`ghostty:include/ghostty.h:1-11`).

**(b) libghostty-vt** is `include/ghostty/vt.h`: terminal state, a render state API, key, mouse,
focus and paste encoding, selection, search, a formatter, kitty graphics and snapshots, with no
font, renderer or IO. We feed it bytes and draw it ourselves.

### Has anything changed since #12131?

- 2026-05-12, #12131, pluiedev: "Ghostty can make very opinionated decisions on rendering and we
  don't want to expose that to libghostty consumers. Just write your own renderer."
- 2026-11-06: libghostty-vt 1.0.0 shipped, and "The C ABI is stable from this release on"
  (`ghostty:dist/doxygen/pages/release-notes.md`). The library has its own `vt-X.Y.Z` tags,
  signed source tarballs and prebuilt binaries (`ghostty:PACKAGING-LIBGHOSTTY.md`).
- 2026-11-23, #15295: someone embedding `ghostty.h` asked for a surface API to read kitty images.
  mitchellh suggested a render state instead. When they said `GhosttyKit` doesn't export the vt
  symbols, he answered: "Ah, this isn't a publicly supported API, that's for internal use only.
  You should use libghostty-vt if you want proper 3rd party support."
- 2026-11-23, #15254: mitchellh gave the official answer for embedders whose cells aren't whole
  pixels (see [§4](#4-sizing)). The project is now supporting people who write their own
  renderers on libghostty-vt.
- Upstream main at `69023b8169` (2026-11-26) still has only the `exec` backend and no surface IO
  API.

So nothing has changed, and the direction is firmer than it was.

### Comparison

| Criterion | (a) Ghostty surface (fork with manual IO) | (b) libghostty-vt + our Rune renderer |
| --- | --- | --- |
| Upstream support | None. The API is internal and "tailored to the needs of the macOS app" (`ghostty.h:1-11`, #15295). Manual IO exists only in forks. | Stable semver C ABI. Minor releases only add, and CI checks each change against ABI goldens (`dist/doxygen/pages/stability.md`, `PACKAGING-LIBGHOSTTY.md` "Versioning and ABI"). |
| IO from herdr-core | Needs a fork. The forks feed output through `Termio.processOutput` under the renderer mutex and get input through a callback on Ghostty's IO thread (Geistty `ARCHITECTURE.md`, gterm `PLAN.md`). | Built in: `ghostty_terminal_vt_write()` in, `GHOSTTY_TERMINAL_OPT_WRITE_PTY` callback out. herdr-app already works this way (`herdr-app:src/vt.rs:385`). |
| Compositing in a Rune/wgpu window | The surface turns a caller's `NSView`/`UIView` into a layer-hosting view with its own `IOSurfaceLayer` (`ghostty:src/apprt/embedded.zig:382`, `ghostty:src/renderer/Metal.zig:91-129`). Rune overlays, popovers and drag previews can't draw above it except as separate native views or windows. Sharing one GPU frame would need yet another fork patch to export the IOSurface into Rune's `NativeSurface` (`rune:crates/rune-media/src/native_surface/mod.rs`), with one frame of latency and cross-device sync. | Drawn inside Rune's own frame. A `rune::grid` is an ordinary element. A custom `Compositor` paints each pane's `El::effect` slot "in painter order, inside the slot's isolated layers, so the slot's opacity (an exit fade included), clip and transform already apply" (`rune:crates/rune/src/compose.rs:9-14`). Either way, overlays, splits and animations just work. |
| Platforms | macOS and iOS only: `Platform = union { macos, ios }` (`embedded.zig:382`). | macOS, iOS, Linux, Windows and wasm. Prebuilt xcframework, Windows zips and wasm modules ship with each release. |
| Many panes | Each surface spawns a renderer thread and an IO thread (`ghostty:src/Surface.zig:99-136`, `:754`) and has its own swap chain and layer. Fonts are shared through `SharedGridSet`. | One Rune frame, one text system and atlas. Each pane costs a terminal, a render state and its changed rows. Grid at 200×60, headless thread CPU p50: 2.46 ms full redraw, 0.40 ms one row, 0.30 ms nothing (`rune:changelog/unreleased/2026-11-13-grid.md`). |
| Rendering fidelity | Exactly Ghostty's. | Same input data, because Ghostty's renderer consumes `terminal.RenderState` (`ghostty:src/renderer/generic.zig:275`) and the C API wraps the same type (`ghostty:src/terminal/c/render.zig:24`). Matching Ghostty is up to our port of `Metrics.calc`, `rebuildRow`, glyph placement and the sprite font, plus how close Rune's text rasterizer is. See the gates in [§2](#where-the-renderer-draws) and [Risks](#8-risks-and-staged-plan). |
| Terminal data for our features | Only what `ghostty.h` exports. #15295 shows embedders patching it just to find an image under a click. | Full grid, tracked references, selection, search, formatter, OSC 133 prompts and kitty placements, which is what copy mode, find, herdr labels and links need. |
| Configuration and fonts | Ghostty's own config system and font discovery. Our settings would have to be translated into Ghostty config. | Ours. |
| Future iOS app | Possible on a fork (Geistty, gterm and cmux iOS all ship one). The app would have two renderers: Ghostty's for terminals and Rune's for everything else. | The xcframework has iOS device and simulator slices, verified in the 1.0.2 zip, and Rune's `LayerHost` attaches a runtime to a `CAMetalLayer` (`rune:ARCHITECTURE.md` "Hosts"). Same core, same renderer. |
| Maintenance | A fork rebased onto a moving internal API. cmux's `docs/ghostty-fork.md` shows what that costs. | Pin a signed release tarball and port renderer changes when we choose to. |

Option (a) wins only on "pixel-identical with zero renderer work, on Apple platforms". It loses
on support, IO, compositing, platforms, data access and pane cost. Those are the requirements
herdr-frontend can't drop, so I recommend (b).

## 1. The libghostty-vt API we need, building and pinning

### API surface

All of these are in `ghostty:include/ghostty/vt/*.h` at `vt-1.0.2`. Options and data keys are
`GHOSTTY_TERMINAL_OPT_*` and `GHOSTTY_TERMINAL_DATA_*` (`terminal.h:2034-3091`).

| Need | API |
| --- | --- |
| Lifecycle and bytes | `ghostty_terminal_new/free/reset`, `ghostty_terminal_vt_write`, `ghostty_terminal_resize(cols, rows, cell_w_px, cell_h_px)` (`terminal.h:3148-3176`) |
| Effects (callbacks) | `WRITE_PTY` (query replies), `SIZE` (XTWINOPS and mode 2048), `RENDER_HOLD` (mode 2026), `TITLE_CHANGED`, `PWD_CHANGED`, `BELL`, `CLIPBOARD_WRITE/READ` (OSC 52), `DESKTOP_NOTIFICATION`, `PROGRESS_REPORT`, `COLOR_SCHEME`, `DEVICE_ATTRIBUTES`, `XTVERSION`, `SEMANTIC_PROMPT`, `RESIZE_WINDOW` (`guides/effects.md`) |
| Behavior | `MODE_DEFAULT` (turn on 2027 grapheme clustering, as herdr-app does at `vt.rs:646`), `COLOR_FOREGROUND/BACKGROUND/CURSOR/PALETTE`, `DEFAULT_CURSOR_STYLE/BLINK`, `SCROLLBACK_MAX_BYTES/LINES` (the default is only 10,000 bytes, `guides/scrollback.md`), `KITTY_IMAGE_STORAGE_LIMIT`, `KITTY_IMAGE_MEDIUM_*`, `GLYPH_PROTOCOL` (off by default) |
| Rendering | `ghostty_render_state_*` (new, update, begin/end update, get, set, clean), row iterator (`next`, `next_dirty`), row cells, image iterator (`render.h`, `guides/rendering.md`, `guides/images.md`) |
| Cells and styles | `GhosttyStyle` with underline kind (`GHOSTTY_SGR_UNDERLINE_SINGLE/DOUBLE/CURLY/DOTTED/DASHED`, `sgr.h:91-97`) and `underline_color` (`style.h:235`), plus resolved fg and bg colors and graphemes per cell |
| Input | `ghostty_key_encoder_*`, `ghostty_key_event_*`, `ghostty_mouse_encoder_*`, `ghostty_mouse_event_*`, `ghostty_focus_encode`, `ghostty_terminal_paste`, `ghostty_paste_is_safe` (`guides/input.md`) |
| Scrollback | `ghostty_terminal_scroll_viewport` (top, bottom, delta, row, prompt delta), `DATA_SCROLLBAR` (total, offset, len), `DATA_VIEWPORT_ACTIVE`, `DATA_TOTAL_ROWS`, `DATA_SCROLLBACK_ROWS`, `ghostty_terminal_compress` (`guides/scrollback.md`) |
| Selection and copy | `ghostty_terminal_select_word/word_between/line/output/all`, `ghostty_terminal_selection_adjust/order/ordered/contains`, `OPT_SELECTION`, `ghostty_selection_gesture_*`, `ghostty_formatter_*` (`guides/selection.md`) |
| Grid access | `ghostty_terminal_grid_ref`, `ghostty_terminal_point_from_grid_ref`, `ghostty_grid_ref_cell/row/graphemes/style/hyperlink_uri`, tracked references (`guides/grid.md`) |
| Search | `ghostty_search_*` (`guides/search.md`) |
| Kitty graphics | `ghostty_kitty_graphics_*`, plus a PNG decoder installed with `ghostty_sys_set` (herdr-app does this at `vt.rs:442-468`) |
| Version | `ghostty_version()`, `ghostty_build_info()`, `GHOSTTY_VT_VERSION_AT_LEAST` (`stability.md`) |

Not in libghostty-vt: fonts, shaping, rasterization, IME preedit storage, configuration, and OSC
72 drag and drop (the C API ignores it, per the 1.0.2 release notes).

### Building and linking from Rust

**macOS and iOS: link the prebuilt xcframework, so the dev loop needs no Zig.** Release
`SHA256SUMS` (fetched from `release.files.ghostty.org/libghostty-vt/1.0.2/`):

```
15477209ce86ad1c944f217e925c84acf796ecc51aba7a935aaa1a7769890bd6  ghostty-vt-1.0.2.xcframework.zip
1dcafa8a3357f3a6bbf4f2696e61de611400c5469518716793dfffd35f008b3e  libghostty-vt-1.0.2.tar.gz
c56156dbea82fdf71f13bf0d704cdf4393b6a04208d7f9691e0d7f60c7f9ca4d  libghostty-vt-1.0.2-deps.tar.gz
```

The zip matches its checksum. It contains `macos-arm64_x86_64/libghostty-vt.a` (universal,
27 MB), `ios-arm64/libghostty-vt-fat.a` and `ios-arm64-simulator/libghostty-vt-fat.a`, each with
`Headers/`.

I linked a C smoke test against the macOS archive with plain `cc smoke.c libghostty-vt.a` and no
other libraries. It wrote VT bytes, resized to 100x30 with a 17x37 cell, and read back
`version=1.0.2 grid=100x30 px=1700x1110`.

**Linux, Windows and source builds.** Use the source tarball and the deps tarball from the same
version with Zig 0.17.x. Major and minor must match exactly, and any 0.17 patch works:

```sh
zig build --system ../libghostty-vt-1.0.2-deps -Doptimize=ReleaseFast -Dcpu=baseline \
  -Dtarget=<arch-os-abi> --prefix <out>
```

This is from `ghostty:PACKAGING-LIBGHOSTTY.md` "Building libghostty-vt". A git checkout needs
`-Demit-lib-vt`, because without a `LIB_VERSION` file `zig build` builds the app. Windows has
prebuilt zips with `ghostty-vt-static.lib`. Link statically, as both reference apps do. I didn't
run a Zig build in this research.

**Bindings.** Run bindgen against `ghostty/vt.h` with the allowlists herdr-app uses
(`herdr-app:crates/ghostty-sys/build.rs`: `ghostty_.*`, `Ghostty.*`, `GHOSTTY_.*`,
`prepend_enum_name(false)`, `derive_default(true)`). Because the ABI is frozen within 1.x, commit
the generated `bindings.rs` for each pinned release, and have CI regenerate it and fail on a
diff. That keeps libclang out of normal builds. At startup, check `ghostty_build_info()` against
the header version (`stability.md` "Checking the Version").

**Pin:** libghostty-vt `vt-1.0.2` (`573c4a21e0`), from the signed source tarball, not a git
submodule. Move to later 1.x releases on purpose. They only add API.

Don't copy either reference pin:

- herdr-app builds a submodule at `291fd050` (pre-1.0, `minimum_zig_version = "0.16.0"`). Its
  API is pre-ABI-freeze: `ghostty_key_encoder_setopt`, renamed to `ghostty_key_encoder_set` in
  1.0. It also misses the 1.0.1 and 1.0.2 fixes, such as "Insert and delete lines ... no longer
  break the soft wraps" and the left/right-margin IL/DL speedup "how Neovim scrolls a vertical
  split" (`release-notes.md`).
- opencode-rune pins app tag `v1.4.0` (`83fb95624b`), which is also pre-1.0, with hand-written
  FFI (`opencode-rune:crates/terminal/src/ffi.rs`).

## 2. Renderer design

### Where the renderer draws

Rune has a built-in element for this: `rune::grid(cols, cell_w, cell_h, rows: Vec<Rc<GridRow>>)`,
"a terminal's screen as one element" (`rune:crates/rune/src/el/grid.rs:1-10,161`). A
`GridRow` has these parts (`grid.rs:73-110`):

- the narrow cells' text as one shaped line, styled by spans (color, weight, italic, single
  underline)
- `cell(col, width, text, style, color)` for wide or fallback glyphs pinned to their column
- `fill(start, end, color)` for background and selection runs
- `shape(Shape)` for a vector layer over the text
- `wraps`

A row is shaped once while the app hands back the same `Rc`, and redraws its last instances
while nothing it depends on changed. Accessibility reads it as role `terminal`. On 200×60,
headless thread CPU per frame at p50, it costs 2.46 ms for a full redraw, 0.40 ms with one row
changed and 0.30 ms with nothing changed (`rune:changelog/unreleased/2026-11-13-grid.md`).
opencode-rune draws its terminal this way (§7).

**Recommendation: build on `rune::grid` first.** Our Ghostty port lives in the `GridRow` builder:
run breaking, colors, sprites, underline styles and pinned cells. Find highlights go into the
row's fills, like selection (gate 5). A hovered link changes the underline in the builder, as
Ghostty does: single, or double if the text is already single-underlined
(`ghostty:src/renderer/generic.zig:3245-3258`). The cursor and IME preedit are sibling elements
over the grid. Then check it against these gates, which are where grid's model differs from
Ghostty's:

1. **Glyph placement.** Ghostty places each glyph on its own cell. It also centers it in the
   rounded cell by `(cell_width − face_width) / 2`, keeping only the fractional part when the
   cell is narrower (`ghostty:src/font/face/coretext.zig:363-378`). Grid places narrow cells by
   tracking one shaped line, which puts the extra space after each glyph. We need the same
   per-row offset, or a check that the difference is invisible.
2. **Ligatures.** Ghostty keeps `calt`/`liga` on. opencode-rune turns them off "so ligatures
   can't shift cells" on a tracked line (`opencode-rune:features/terminal-surface/src/grid/mod.rs:101-102`).
   To match Ghostty, a ligature run would have to become a pinned multi-column `cell` with
   ligatures on in its own style. That's unverified.
3. **Sprites.** Ghostty rasterizes box drawing and the other sprite glyphs at the cell size.
   Grid draws them as a vector `Shape`, so we need a pixel diff on box drawing, Powerline and
   braille.
4. **Many panes.** Profile N visible panes scrolling at once against the 8.3 ms frame at
   120 Hz. A full redraw is 2.46 ms per 200×60 pane, so four panes all scrolling at once would
   miss it unless they're smaller.
5. **Layering.** Grid paints one row at a time: fills, then the line clipped to its row, then
   pinned cells, then the vector layer (`rune:crates/rune/src/runtime/paint/grid.rs:50-139`).
   Nothing outside the grid can sit between a row's backgrounds and its text. Ghostty draws
   all backgrounds, then the layers between, then unclipped text. That breaks three things:
   - kitty images in the below-text layer. They must sit above cell backgrounds and below text
     (`types.h:380-407`). Placeholders use z −1, so they land in this layer.
   - find highlights under the text. Drawn under the grid, they disappear beneath the explicit
     backgrounds that most Neovim colorschemes set on every cell. Drawn over it, they tint the
     glyphs.
   - glyphs that overflow their row, which `Metrics.zig:249-256` expects from some fonts. Grid
     clips them.

   Options on the grid side:
   - put find highlights into `GridRow::fill`, as selection already is.
   - draw two grids per pane, one with fills only and one with text, with below-text images
     between them.
   - or count this as a failed gate and move to the custom pass.

**Fallback: a custom wgpu pass.** A Rune `Compositor` draws each pane at its `El::effect` slot
with instanced cell shaders ported from Ghostty. Ghostty's pieces:

- `updateFrame` copies the render state under the lock, and `rebuildCells` and `rebuildRow`
  turn it into GPU buffers (`ghostty:src/renderer/generic.zig:1371`, `:2630`, `:2925`).
- The bg pass is a full-screen fragment shader over a per-cell bg buffer (`cell_bg_fragment`,
  `shaders.metal:451`, and `cell.zig:42-74` `bg_cells[row * cols + col]`).
- Text is instanced quads (`cell_text_vertex`, `shaders.metal:556`), with minimum contrast and
  cursor-cell recoloring in the shader (`shaders.metal:653-662`).

That gives pixel-for-pixel control, but costs a lot:

- our own rasterizer and atlas. PR #2 found no public API for a compositor to sample Rune's
  glyph atlas.
- no grid accessibility tree, no CPU painter, and no `--snap` parity.
- possibly `GpuBackend::Wgpu` on macOS, because Rune's native Metal backend supports only a
  subset of wgpu (PR #2, `docs/research/rune.md` §3).

### Data flow, bytes to frame

```
herdr-core stream (DATA frames on the attach channel)
  └─ terminal worker thread: ghostty_terminal_vt_write(term, bytes)        [term mutex held]
       ├─ WRITE_PTY → replies → AUTOMATIC_INPUT frame to core   (see §4 on duplicate replies)
       ├─ RENDER_HOLD(true) → ghostty_render_state_update() now (capture the finished frame)
       └─ TITLE/PWD/BELL/CLIPBOARD/NOTIFICATION → app events; wake the UI
UI frame (Rune display link, `poll`):
  for each visible pane:
     lock; render_state_begin_update(); read SCROLLBAR, VIEWPORT_ACTIVE, MOUSE_TRACKING; unlock
     render_state_end_update()                      [no terminal lock, guides/rendering.md]
     dirty == FALSE and nothing of ours changed → hand back the same rows
     else rebuild dirty rows (RowId cache) → Rc<GridRow>: spans, pinned cells, fills, sprites
     render_state_clean()
  view: grid(cols, cell_w, cell_h, rows) + cursor / preedit overlays
        + kitty images as image elements: below-bg ones under the grid, above-text ones over it,
          and below-text ones only through gate 5's workaround (two grids)
  (fallback: Compositor::render paints the same rows as instanced cells at El::effect slots)
```

This is the split Ghostty uses: copy under the lock, then build the frame from the copy.

Rune runs its UI on one thread with no render thread today (`rune:ARCHITECTURE.md` "Single-thread
frame model"). So VT parsing must stay on our own worker thread, and the two-phase update keeps
the shared lock short.

### Dirty tracking and damage

- Use libghostty's two layers: global `DIRTY_FALSE/PARTIAL/FULL`, and row `DIRTY`. Iterate with
  `ghostty_render_state_row_iterator_next_dirty`, then call `ghostty_render_state_clean` after a
  complete frame. An update only sets dirty state and never clears it (`guides/rendering.md`
  "Dirty tracking").
- Cache per-row work (the built `Rc<GridRow>`, or instance ranges in the fallback pass) by `GHOSTTY_RENDER_STATE_ROW_DATA_ID`. Reuse
  an entry when the id is present and the row isn't dirty. Ids are never reused ("Row identity").
  Every row is currently marked dirty after the viewport scrolls, so smooth scrolling has to
  rebuild rows; overscan supplies the rows at the edges.
- Our own invalidations: font, scale or theme change (full), selection or search overlay change,
  focus (hollow cursor), cursor blink, IME preedit, hover link.
- Damage-based redraw means doing no work when nothing changed. If nothing changed, the view
  hands back the same `Rc<GridRow>`s, and Rune redraws their last instances (0.30 ms at 200×60)
  or goes idle. A custom-pass compositor would report `animating() == false` so the display link
  can rest (`rune:crates/rune/src/compose.rs:71-76`). Otherwise only changed rows are rebuilt.
  On macOS, Rune submits one drawable per display tick from a CVDisplayLink. That's 120 Hz on
  ProMotion, and it paces at the cap in Low Power Mode
  (`rune:crates/rune/src/platform/display_link.rs:1-11`). The 120 Hz budget comes from per-row
  caching, not from partial-surface presents.
- Render holds: register `RENDER_HOLD`, capture the frame at the start of a hold, stop updating
  while it lasts, and give up after 1 s. That matches Ghostty's `sync_reset_ms = 1000`
  (`ghostty:src/termio/Thread.zig:38`). herdr-app already does this (`vt.rs:619-628`, `:1534-1546`).

### Glyph atlas

Ghostty keeps two atlases per font grid: grayscale R8 for text and sprites, and color BGRA for
emoji. They're packed by `ghostty:src/font/Atlas.zig` and synced only when modified
(`generic.zig:1891-1899`). A `SharedGrid` is ref-counted per font configuration and shared by
every surface (`ghostty:src/font/SharedGridSet.zig:89-96`).

On `rune::grid`, Rune's text system owns the atlas, and every pane shares it. The custom-pass
fallback would do what Ghostty does: one atlas pair per (font configuration, device scale),
shared by all panes and keyed by (face, glyph id, render options), with sprites rasterized into
the grayscale atlas once per (codepoint, cell size).

### Font fallback and shaping

- Ghostty's `Collection` holds regular, bold, italic and bold-italic faces plus fallbacks.
  `CodepointResolver` resolves a codepoint in this order (`ghostty:src/font/CodepointResolver.zig:86-118`):
  1. codepoint overrides
  2. the sprite font
  3. loaded faces in the requested style and presentation
  4. loaded faces in regular style
  5. system discovery
  6. any presentation

  Presentation follows VS15/VS16 and skin tone modifiers (`ghostty:src/font/shaper/run.zig:170-188`).
- Run breaking (`run.zig:90-210`) splits on any style change except background, on font face
  changes, on selection boundaries, and around the cursor (`font-shaping-break = cursor`, the
  default, `Config.zig:382`). It also splits the "bad ligatures" `fi`, `fl` and `st`
  (`run.zig:126-142`). The shaped glyphs are then placed on cell boundaries.
- **Ligature policy:** do what Ghostty does by default. Keep the font's `calt` and `liga`, break
  runs at the cursor and at style and selection boundaries, never join `fi`, `fl` or `st`, and
  offer a setting equivalent to `font-feature = -calt, -liga, -dlig` (`Config.zig:238-253`).
- For shaping and rasterization, use rune-text's stack: Parley, swash and skrifa, with
  CoreGraphics-matched smoothing (`rune:ARCHITECTURE.md`). Font lookup works like this
  (`rune:crates/rune-text/Cargo.toml:54-67`):
  - fontique finds installed fonts by family name.
  - on Apple platforms, CoreText does font fallback and family lookups
    (`fonts/system_fonts/core_text.rs`), the same system fallback Ghostty uses.

  On `rune::grid`, Rune does the shaping. What we always need is the raw face metrics (hhea,
  OS/2, post, and the ASCII advances) for `Metrics.calc`, plus each codepoint's fallback face, to
  decide which cells to pin. skrifa can read the metrics from the same font file. The
  custom-pass fallback would also need glyph-level output: glyph ids, clusters and advances for
  a run. Ticket 004 should confirm what rune-text exposes. If it isn't enough, call skrifa and
  swash directly and keep rune-text's font discovery and fallback.

### Cell metrics identical to Ghostty

Port `ghostty:src/font/Metrics.zig:227-330` (`calc`) and its inputs from
`ghostty:src/font/face/coretext.zig:633-851`:

1. **Pixel size:** `px = points × dpi / 72`, with `dpi = 72 × content_scale` on macOS (96 ×
   scale elsewhere). See `ghostty:src/font/face.zig:29,55-58` and `ghostty:src/Surface.zig:512-525`.
2. **Vertical metrics:** if OS/2 `fsSelection.USE_TYPO_METRICS` is set, use `sTypoAscender`,
   `sTypoDescender` and `sTypoLineGap`. Otherwise use hhea if its ascender or descender is
   non-zero, else sTypo*, else usWin* with no line gap (`coretext.zig:642-700`).
3. **Face width:** the largest advance among printable ASCII 0x20–0x7E, not the advance of `M`
   (`coretext.zig:767-800`).
4. `cell_width = round(face_width)`, `cell_height = round(ascent − descent + line_gap)`
   (`Metrics.zig:265-266`).
5. `cell_baseline` (from the bottom) = `round(line_gap/2 − descent − (cell_height − face_height)/2)`
   (`Metrics.zig:271-283`).
6. Underline and strikethrough thickness are `max(1, ceil(...))`, from the `post` and `OS/2`
   tables or estimates: underline 0.15 × ex-height, strikethrough centered at half the
   ex-height. Their positions are rounded from the top of the cell. Overline and box thickness equal underline
   thickness (`Metrics.zig:296-324`, `:150-220`).
7. Apply the `adjust-*` modifiers, then clamp (`Metrics.zig:330+`).

All values are whole device pixels. The grid is laid out in device pixels, so column `k` starts
at exactly `k × cell_width`.

Worked example, JetBrains Mono (bundled by herdr-app; `upm 1000`, typo 1020/−300/0,
`USE_TYPO_METRICS` set, ASCII max advance 600) at 14 pt:

- 1x: 8.4 × 18.48 rounds to an **8 × 18** px cell.
- 2x: 16.8 × 36.96 rounds to a **17 × 37** px cell.

### Sprite glyphs

Ghostty doesn't take these from fonts. It draws them procedurally at the exact cell size with z2d
(`ghostty:src/font/sprite/Face.zig`, `sprite/draw/*.zig`). The ranges, from the `draw<CP>`
function names:

- `2500–257F` box drawing, `2580–259F` blocks, `2800–28FF` braille
- `25E2–25E5`, `25F8–25FA`, `25FF` geometric shapes
- `1FB00–1FBEF` and `1CC1B–1CEAF` symbols for legacy computing
- Powerline `E0B0–E0BF`, `E0D2`, `E0D4`
- branch drawing `F5D0–F60D`

The same sprite face draws all five underline styles and the cursor shapes:
`sprite/draw/special.zig` (`underline`, `underline_double`, `underline_dotted`,
`underline_dashed`, `underline_curly`, `cursor_rect`, `cursor_hollow_rect`, `cursor_bar`,
`cursor_underline`).

Port Ghostty's geometry to Rust: the arm weights, the thicknesses from `box_thickness`, and the
corner, diagonal and Powerline paths. On `rune::grid`, emit them into the row's `shape` layer
with edges on whole device pixels, as opencode-rune does
(`opencode-rune:features/terminal-surface/src/draw.rs`). That's gate 3 in
[Where the renderer draws](#where-the-renderer-draws). If vector output doesn't match Ghostty's
raster pixels, rasterize each sprite once per (codepoint, cell metrics) with tiny-skia, which is
already in Rune's tree. Ghostty's canvas is a plain path rasterizer. Then draw the result as an
image cell, or in the custom pass's grayscale atlas.

### Cursor

The render state gives the cursor's position, visual style (block, bar, underline, hollow
block), `visible`, `blinking`, `wide_tail` and `password_input`. It also gives an explicit cursor
color if the program set one (`render.h`, `GhosttyRenderStateCursor`).

Draw the cursor as a sibling element over the grid, at the cell's whole-pixel rectangle, in
Ghostty's shapes. Set `grid_caret(col, row)` on the grid so screen readers see the caret, since
it draws nothing itself (`rune:crates/rune/src/el/grid.rs:172`).
Ghostty recolors the glyph under a block cursor in its text shader, with the `cursor_pos`,
`cursor_color` and `cursor_wide` uniforms (`shaders.metal:20-23,659-662`). On grid, do the same
by drawing that cell's glyph again above the block in the cursor-text color. Draw a hollow block
when the pane is unfocused. Run the blink timer only when `blinking` is set and the pane is
focused.

### Selection overlay

Read `GHOSTTY_RENDER_STATE_ROW_DATA_SELECTION` once per row for its start and end columns. Apply
selection colors while building the row and break shaping runs at the selection edges, as
Ghostty does. Install the selection with `OPT_SELECTION` so the render state reports it.

### Scrollback viewport

- Scroll with `ghostty_terminal_scroll_viewport`. Each frame, poll `DATA_SCROLLBAR` and
  `DATA_VIEWPORT_ACTIVE` together with `begin_update` ("The scrollbar" in `guides/scrollback.md`).
- For smooth pixel scrolling, request one row of overscan with `RENDER_STATE_OPT_OVERSCAN` and
  shift the grid by a fraction of a row (`guides/rendering.md` "Overscan").
- Set the scrollback limits explicitly. Optionally compress idle scrollback with
  `ghostty_terminal_compress` on an idle timer.

### Kitty graphics

Read placements with the render-state image iterator. `IMAGE_PLACEMENTS_GENERATION` tells us
when to re-read the list, and each image's `IMAGE_GENERATION` keys its texture cache.

Draw each placement in its layer, ordered by z within the layer. The three layers are below the
cell backgrounds, between backgrounds and text, and above text (`types.h:380-407`,
`render.h:197-375`, `guides/images.md`). On `rune::grid`, the middle layer needs gate 5's
two-grid workaround. Clip fixed and relative placements to the viewport. Pixel values are in the
cell size passed to `ghostty_terminal_resize`. Because our cell is whole pixels, no extra scaling
is needed.

## 3. Input

- **Keys and the kitty keyboard protocol.**
  - Before each encode, sync the encoder with `ghostty_key_encoder_set_from_terminal`, then set
    `MACOS_OPTION_AS_ALT` again, because the sync resets it (`guides/input.md`).
  - Each event needs:
    - the action: press, repeat or release. Releases matter under kitty flag 2.
    - the physical key.
    - the mods.
    - **the consumed mods.** herdr-app found that "without it the kitty keyboard protocol
      reports shift+; and Neovim never sees ':'" (`herdr-app:src/vt.rs:1687-1690`).
    - the UTF-8 text and the unshifted codepoint.
  - The encoder writes nothing for a lone modifier, so check the byte count. A 128-byte buffer is
    enough. On `OUT_OF_SPACE`, retry with a bigger one.
- **IME.**
  - Set `ghostty_key_event_set_composing(true)` for keys that are part of a composition.
  - libghostty-vt doesn't store preedit text. We draw it at the cursor as overlay cells, as
    Ghostty does with `addPreeditCell` (`generic.zig:3628`), and send only committed text.
  - Put the candidate window at the cursor cell, even when the program hides the cursor. herdr-app
    keeps `cursor_cell` for this reason (`vt.rs:111-113`).
  - Rune's IME shells are in `rune:crates/rune/src/platform/ime/`, which ticket 004 covers.
- **Mouse.**
  - Sync with `ghostty_mouse_encoder_set_from_terminal`.
  - Set `OPT_SIZE` to a screen of `cols × cell` by `rows × cell` with our whole-pixel cell, set
    `ANY_BUTTON_PRESSED` yourself, and turn on `TRACK_LAST_CELL`. The encoder supports X10,
    UTF-8, SGR, URxvt and SGR-Pixels.
  - Use `DATA_MOUSE_TRACKING` to choose between reporting and local selection, with Shift
    overriding reporting.
  - Feed local selection through `ghostty_selection_gesture_*`, which handles click counts,
    dragging and autoscroll ticks. Ghostty's `selection_scroll_ms` is 15 (`Thread.zig:41`).
- **Paste.** `ghostty_terminal_paste` reads the modes itself. It handles bracketed paste (2004)
  and kitty paste events (5522), strips unsafe bytes, and returns `GHOSTTY_REJECTED` for unsafe
  text. Confirm with the user, then retry with `allow_unsafe` (`guides/input.md` "Paste").
- **Focus.** `ghostty_focus_encode`, sent only when `GHOSTTY_MODE_FOCUS_EVENT` is set. herdr-core
  also has a FOCUS control frame. Ticket 001 should decide whether the core or the viewer encodes
  focus for a terminal that several viewers share.

## 4. Sizing

### Rules

1. **Cell size** comes from font metrics as in [§2](#cell-metrics-identical-to-ghostty): whole
   device pixels.
2. **Padding.**
   - Ghostty subtracts explicit padding (`window-padding-x/y`, in points, scaled by content
     scale), then computes `cols = floor(w / cell_w)` and `rows = floor(h / cell_h)`, at least 1
     each (`ghostty:src/renderer/size.zig:253-262`).
   - It then splits the leftover space as balanced padding (`size.zig:46-80`, `:279-307`).
     `window-padding-balance` can be `false`, `true` (top capped) or `equal`.
3. **What we report must match one whole-pixel cell.** mitchellh, #15254 (2026-11-23):
   "`CSI 14 t`, `CSI 16 t`, mode 2048, `ws_xpixel`/`ws_ypixel`, and SGR-Pixels coordinates are
   all integers by spec ... What matters is that everything the program sees is consistent with
   one whole-pixel cell." That means:
   - pass the integer cell to `ghostty_terminal_resize` and to the `SIZE` callback.
   - report the pixel size as `cols × cell` by `rows × cell`.
   - give the mouse encoder that same geometry.

   Ghostty's own app sets `ws_xpixel` to the padded terminal size
   (`ghostty:src/termio/Exec.zig:1149-1154`), which with balanced padding is within a pixel of
   `cols × cell`.
4. **Debouncing.** Ghostty coalesces resize messages for 25 ms on the IO thread
   (`Thread.zig:28-31`). We should:
   - recompute the pane's grid every frame while the user resizes, and draw the current
     terminal clipped to the pane.
   - send the pane's grid to the core as `RESIZE` at most once every 25 ms, always including the
     final size. That's a request. The local terminal is resized to the size the core actually
     gives the PTY (see the next section), not straight to the pane.
   - send `FACTS` with the new cell size before `RESIZE`, so a program woken by the resize asks
     for the new cell. herdr-app does the same (`terminal.rs:190-207`).

### Several viewers on one terminal (herdr-core)

These come from the schema and from herdr-app's own integration test.

- `terminal.attach` takes the viewer's `cols`, `rows` and `display` (fg, bg, cursor, and `cell`
  in pixels, "used to answer display queries"). Also `observe` ("Watch only: never type or
  resize"), `resumed` ("sets the size only once active again") and `resize_notifications`
  ("Observer understands core-to-viewer RESIZE before replacement paints") (`hc schema`,
  `AttachResult`).
- herdr-app's test `two_app_viewers_answer_queries_without_idle_size_theft` (`herdr-app:src/hc.rs:704-760`)
  attaches two viewers at 80×24 and 100×30. The terminal ends up at the active viewer's 100
  columns. An idle viewer's replies, sent as `AUTOMATIC_INPUT`, don't take the size back. And
  **both viewers answer each query**: the program receives "six kitty replies (two per query)".

That leads to two requirements for herdr-frontend:

1. **The local terminal follows the PTY size, not the pane.** herdr-app never asks for
   `resize_notifications` (`hc.rs:466-470`). It resizes its local terminal to its own pane
   (`terminal.rs:190-200`). While another viewer owns the size, it parses bytes laid out for 100
   columns into an 80-column terminal, so wrapping breaks. Instead:
   - Attach with `resize_notifications: true`.
   - **If `AttachResult.resize_notifications` comes back true,** resize the local terminal only
     on core-to-viewer `RESIZE`. Draw that grid in the pane: clipped, letterboxed, and marked as
     "sized by another viewer". Typing or focusing the pane claims the size again by sending our
     `RESIZE`.
   - **If it comes back false or absent,** the schema says "the observer must track size changes
     separately". That can happen when a hop on the path, such as a `machine/t7` address through
     another core, doesn't relay the notices. Fall back to herdr-app's behavior: resize locally
     to the pane. Re-read `cols` and `rows` with `terminal.get` whenever the pane gains focus or
     sends input, and resize locally to match.
   - Ticket 001 should confirm whether the core also sends `RESIZE` back to the viewer that
     asked for the size, or only to the others.
2. **Decide once who answers queries.** Duplicate replies are harmless for some queries and
   wrong for others, such as DA, DSR and kitty graphics `a=q`. Ticket 001, or herdr-core itself,
   should settle whether only the active viewer forwards `WRITE_PTY` replies or the core answers
   alone.

## 5. Why herdr-app's size reporting and Neovim rendering are wrong

Each item is checked against `herdr-app@b721ded`. I read the code and didn't take screenshots,
so symptoms are stated as what the code does.

### Size reporting

1. **Viewers fight over the size.** This is the size-reporting defect. When another client owns
   the PTY size, herdr-app's local terminal keeps its own pane's size and lays the program's
   output out at the wrong width. See [§4](#several-viewers-on-one-terminal-herdr-core).
2. **Idle viewers send size reports with the wrong size.** Every viewer's local terminal answers
   the program's queries, and the core forwards each viewer's `AUTOMATIC_INPUT`. The
   `hc.rs:704-760` test shows the program receiving "six kitty replies (two per query)". Size
   reports go the same way:
   - Turning on mode 2048 makes libghostty-vt report the terminal's own geometry right away
     (`ghostty:src/terminal/stream_terminal.zig:2082`, herdr-app's test at `vt.rs:1958-1970`).
   - `fit` forwards the report again after each pane resize (`terminal.rs:200-206`).
   - The local terminal is sized to its own pane (item 1).

   So next to the active viewer's 100×30 report, the program also gets the idle viewer's 80×24
   report, an in-band size report that contradicts the PTY. That test turns 2048 on every round.
   I haven't checked whether herdr-core's own answers, built from `FACTS`, add more.
3. **The rounded cell it reports is not a bug.** `device_cell()` reports
   `round(logical width × scale)` (`terminal.rs:212-215`): 8 px at 1x for a cell drawn at 8.4.
   - It hands that same cell to `ghostty_terminal_resize` and the `SIZE` callback
     (`vt.rs:1234-1242`, `:630-636`) and sends it in `FACTS` (`terminal.rs:193-195`).
   - The mouse encoder works in the same whole-pixel grid (`vt.rs:1739-1757`).
   - That's the rule mitchellh gives in #15254 (§4), with one gap: images.
     - Unicode-placeholder placements carry `cell_pixels` and are scaled to the real cell
       (`vt.rs:172-176`, `:308`), so they fill their cells.
     - Direct placements have `cell_pixels: None` (`vt.rs:830`) and draw at libghostty's
       `pixel_width` and `pixel_height`, divided only by the display scale
       (`terminal.rs:1602-1608`). Those sizes are measured in the reported cell. So an N-column
       image is 8N px wide over 8.4N px of cells at 1x and stops about 5% short. At 2x it
       overshoots slightly (17 vs 16.8).

     #15254 says to "multiply by real cell / reported cell when you draw". Ghostty doesn't
     need that step, because its cell is a whole number of pixels.

### Neovim rendering

4. **The cell width is a fraction of a pixel.** `cell_size()` takes the font's advance of `'M'`
   without rounding: "The width stays the font's own advance" (`src/terminal.rs:1427-1438`). At
   JetBrains Mono 14 pt, the cell is 8.4 logical px wide, so column `k` starts at `8.4k`.
   Ghostty uses `round(max ASCII advance)` in device pixels (§2), so its columns are all the
   same width. herdr-app avoids seams by snapping, but the snapping has costs of its own:
   - Backgrounds go through `snap_box`, which rounds both edges to device pixels ("cells side by
     side meet without seams", `terminal.rs:1622-1627`, `:1670`). Box drawing, blocks, braille
     and Powerline are sprites snapped the same way (`src/sprites.rs:174-187`). So at 1x the
     cell edges land at 0, 8, 17, 25, 34, 42, 50…, and the columns are **a mix of 8 and 9
     device pixels wide**.
   - The snapped centre lines of `│` and `─` sit up to a pixel off-centre, varying from cell
     to cell.
   - Text glyphs move onto the pixel grid only after they drift more than 1 px (the comment in
     `cell_size`). So text can sit up to 1 px off the snapped backgrounds and box drawing
     beneath it: Neovim's cursorline, visual selection, split separators and float borders.
5. **The cell height has the wrong formula.** It's `ceil((ascent + |descent|) × 1.15 × scale) / scale`
   (`terminal.rs:1434-1437`). Ghostty's is `round(ascent − descent + line_gap)`, with the
   baseline centered (§2).
   - That's 22 px against Ghostty's 18 at 14 pt 1x, and 43 against 37 at 2x. These numbers
     assume GPUI reports hhea ascent and descent, which are 1020/−300 for this font.
   - Rows are about 20% taller than in Ghostty. The underline is a fixed 1 px rather than the
     font's position and thickness.
6. **All underlines draw as one plain line.** `underline: style.underline != 0` (`src/vt.rs:1679`)
   turns every underline style into one flag, and the run is drawn with `wavy: false` in the
   foreground color (`terminal.rs:1554`).
   - Neovim's diagnostics (`4:3` undercurl with `58:2::r:g:b`), spelling and LSP highlights lose
     both the curl and the color.
   - Double, dotted and dashed underlines become a single straight line.
   - `vt::Cell` has no field for overline or blink, so those are dropped.
7. **Text is laid out as GPUI runs, not on the cell grid.** `row_ops` merges cells of equal style
   into a run and shapes it with GPUI's forced width (`terminal.rs:1467-1560`). It shrinks
   fallback glyphs to fit and sizes Nerd Font icons by a heuristic (`:1500-1520`). Ghostty
   places every glyph at its cell and constrains icons with `nerd_font_attributes.zig`. So CJK,
   emoji and icons in Neovim file trees and statuslines don't line up the way they do in Ghostty.
8. **The libghostty-vt pin predates 1.0.** `291fd050` is missing these 1.0.1 and 1.0.2 fixes
   (`release-notes.md`):
   - "Insert and delete lines (IL and DL) no longer break the soft wraps of rows that move
     together."
   - "A failed render state update no longer marks unfinished rows as clean", which could crash.
   - Two resize fixes: "Resizing the column count no longer leaves stale shell prompts behind",
     and "A resize that grows rows and shrinks columns at the same time now pulls scrollback down
     correctly".

## 6. Copy/visual mode with Vim motions over scrollback

libghostty-vt has every primitive this needs. The motion engine is ours.

| Need | libghostty-vt primitive |
| --- | --- |
| Address any cell in history | `GhosttyPoint` with `ACTIVE`, `VIEWPORT`, `SCREEN` (oldest scrollback row = 0) or `HISTORY` tags. `ghostty_terminal_grid_ref` resolves a point, and `ghostty_terminal_point_from_grid_ref` converts back (`guides/grid.md` "Coordinates"). Bounds come from `DATA_TOTAL_ROWS` and `DATA_SCROLLBACK_ROWS`. |
| Read text | `ghostty_grid_ref_cell` and `ghostty_cell_get` (`CODEPOINT`, `WIDE`, `CONTENT_TAG`, `SEMANTIC_CONTENT`), and `ghostty_grid_ref_graphemes` (`screen.h:161-241`) |
| Logical lines (wrapped rows) | `ghostty_row_get`: `ROW_DATA_WRAP` and `WRAP_CONTINUATION` (`screen.h:274-331`) |
| Prompt jumps (`[[`, `]]`) | `ROW_DATA_SEMANTIC_PROMPT`, cell `SEMANTIC_CONTENT`, and `scroll_viewport` with `DELTA_PROMPT` (needs OSC 133) |
| Cursor and visual anchor that survive output | Tracked references (`ghostty_terminal_grid_ref_track`) follow scrolling, reflow and pruning. Handle `has_value() == false` after a prune (`guides/grid.md` "Tracked references"). |
| `v`, `V`, `Ctrl-v` | A `GhosttySelection` with `rectangle` for block mode, installed with `OPT_SELECTION` so the render state shows it. `ghostty_terminal_selection_adjust` moves the logical end. |
| Text objects (`iw`, `aW`, line, output) | `ghostty_terminal_select_word` and `select_word_between` with custom boundary codepoints, so `w` and `W` can differ, plus `select_line` and `select_output` |
| `y` (yank) | The formatter with `OPT_SELECTION`, `UNWRAP` and `TRIM`, which gives the same text Ghostty copies (`guides/selection.md` "Copying it out") |
| `/`, `?`, `n`, `N` | `ghostty_search_*`, incremental over scrollback with viewport matches (herdr-app `vt.rs:1321-1475`) |

**Design.**

- Copy mode pins the viewport with `scroll_viewport(ROW)` while output keeps arriving.
- Its cursor and anchor are tracked references.
- Motions run over a line provider that reads logical lines lazily through grid references,
  starting near the cursor, and caches them by tracked row.
- Lookups with the `SCREEN` or `HISTORY` tag "may have to walk the full scrollback page list", so
  walk outward from a reference you already have, rather than resolving absolute rows over and
  over.

**Caveat.** An untracked reference dies on any terminal change (`guides/grid.md` "When a
reference dies"). Never keep one across a `vt_write`.

## 7. How opencode-rune does it

- **Build** (`crates/terminal/build.rs`):
  - downloads Zig 0.17.0 and the Ghostty `v1.4.0` source tarball, both SHA-256-verified.
  - builds a static ReleaseFast archive and caches it under `~/.cache/seer/ghostty-vt/<key>/`.
  - `GHOSTTY_VT_LIB` and `GHOSTTY_SOURCE_DIR` override that.
  - also builds `wasm32-freestanding` for the web app (`crates/terminal/README.md`).
  - FFI is hand-written (`ffi.rs`, `ffi_fns.rs`) against a pre-1.0 API.
- **Model** (`crates/terminal/src/render_extract.rs`): each frame it updates the render state and
  produces an immutable `TerminalSnapshot`, reusing `Arc<TerminalRow>` for clean rows.
- **Drawing** (`features/terminal-surface/src/grid/`, `draw.rs`):
  - draws each terminal as one `rune::grid` element (`grid/mod.rs:409`), not a pipeline of its
    own.
  - builds an `Rc<GridRow>` per row and caches it by (row id, content hash, font and cell key,
    theme), so unchanged rows go back to Rune as the same `Rc` (`grid/rows.rs:14-55`).
  - lays out the narrow cells as one line, with tracking that forces each glyph's advance to the
    cell width (`cell_tracking`, `grid/mod.rs:117-121`). Wide and other-width cells go into
    `GridRow::cell` (`rows.rs:172`), and backgrounds and selection into `fill`
    (`rows.rs:165-168`).
  - puts box drawing, blocks, braille, Powerline and the special underline styles in the row's
    vector `shape`, with edges snapped to device pixels (`draw.rs`, `rows.rs:93-103`).
  - draws the cursor, find hits, link underlines and IME preedit as sibling elements over the
    grid (`grid/mod.rs`).
- **Metrics** (`grid/mod.rs:106-151`):
  - width is `round(advance("M"…) × scale) / scale`, a whole number of device pixels.
  - height is special-cased for JetBrains Mono and otherwise uses the style's line height. It's
    documented as matching ghostty-web's `measureFont`, not Ghostty's `Metrics.calc`.
  - turns ligatures off (`'calt' 0, 'liga' 0`, `grid/mod.rs:101-102`).

**What to take from it.** It proves libghostty-vt on `rune::grid` works, including on the web.
Its build caching and checksum verification are worth copying, and so is its row cache. We
should replace three things:

- its metrics, with Ghostty's `Metrics.calc` (§2).
- its ligatures-off policy, with Ghostty's (gate 2).
- its pre-1.0 FFI, with 1.0.2 bindings.

Then check grid against the gates in [§2](#where-the-renderer-draws).

## 8. Risks and staged plan

### Risks

| Risk | Mitigation |
| --- | --- |
| Our rasterizer doesn't match Ghostty's CoreText output, and fidelity is the goal. | rune-text claims CoreGraphics-matched smoothing. Build a pixel-diff harness against Ghostty.app (same font, size and scale) early, in stage 4. Fall back to CoreText rasterization on macOS if diffs remain. |
| rune-text may not expose raw face metrics, per-codepoint fallback, or (for the fallback pass) glyph-level shaping. | Ticket 004 checks. If not, read the font tables with skrifa, and use swash and harfrust directly for the pass, keeping rune-text's discovery and its CoreText fallback on Apple. |
| `rune::grid` fails a fidelity gate (glyph centering, ligatures, sprite pixels, layering) or the many-pane budget. | Gates in §2, measured in stage 4. The fallback is the custom `Compositor` pass, which needs its own rasterizer and atlas and takes over frame encoding for the whole app. |
| Single-threaded Rune UI with heavy output from many panes. | Parse on worker threads, use two-phase updates, cap the bytes per frame per pane, and use render holds. |
| Multi-viewer size and duplicate replies need herdr-core decisions. | Raise them in ticket 001 (§4). Until then, use `resize_notifications` and a letterboxed grid. |
| Ghostty's renderer keeps changing upstream, and our port drifts. | Pin the Ghostty revision the port came from (`vt-1.0.2` here), keep a parity corpus, and re-port on purpose. |
| Zig is still needed for Linux. | Prebuilt xcframework (macOS, iOS) and Windows zips. One Zig 0.17 CI job builds Linux from the signed tarball. |
| Gaps in libghostty-vt (no preedit, no OSC 72 drops, no fonts). | All of these are renderer or app concerns, handled in our layer. |

### Staged plan

Each stage ends with tests in `cargo test` plus a fixture or screenshot check.

1. **`ghostty-vt-sys`:** link the pinned 1.0.2 xcframework on macOS and the signed source build
   elsewhere. Commit the bindings, and check the version at startup. Smoke test: write bytes and
   read the render state, as above.
2. **Terminal core:** a worker-thread terminal wrapper with callbacks, render holds, scrollback
   limits, and herdr-core attach bytes in and replies out. Test against recorded streams. It runs
   headless and draws nothing.
3. **Metrics and grid:** port face-metric extraction and `Metrics.calc`, plus grid and padding
   math. Golden tests against Ghostty's numbers for several fonts, sizes and scales. Wire up
   `RESIZE` and `FACTS`.
4. **Renderer MVP on `rune::grid`:** a `GridRow` builder from the render state (runs, colors,
   fills, pinned cells), the row-id cache, cursor and selection. One pane, then many. Start the
   pixel-diff harness against Ghostty.app, and measure the §2 gates. If a gate fails, decide
   here whether to move to the `Compositor` pass.
5. **Fidelity:** port the sprites (`sprite/draw/*.zig`), underline styles and colors, Nerd Font
   icon constraints, the color emoji atlas, fallback, and the ligature policy. Neovim checklist:
   splits, floats, undercurl, statusline icons, CJK.
6. **Input:** keys (kitty, consumed mods, releases), IME preedit, mouse, paste, focus.
7. **Scrollback:** viewport and scrollbar, smooth scroll with overscan, search, OSC 8 and links.
8. **Copy/visual mode** (§6).
9. **Kitty graphics.**
10. **Multi-viewer sizing** with herdr-core (§4). Then an iOS spike: xcframework iOS slice plus
    Rune `LayerHost`.
