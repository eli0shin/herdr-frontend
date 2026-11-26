# Renderer gates: `rune::grid` against Ghostty

Ticket `012-terminal-renderer-mvp-on-rune-grid`, with gate 1 and the fidelity work measured in
ticket `018-terminal-rendering-fidelity-for-neovim`. Measured 2026-11-26 on an Apple M1 Pro (8 cores),
macOS 27, built-in 2x display, against Rune `4918969f` and Ghostty.app 1.4.0. The font is
JetBrains Mono 2.304 at 14 pt, the face Ghostty embeds, so both apps use the same file. Its cell is
17 × 37 px at 2x. This doc answers the four gates in
[`terminal-rendering.md` §2](terminal-rendering.md#where-the-renderer-draws) and stage 4 of §8.

## Recommendation

**Stay on `rune::grid`.** Nothing measured here argues for the custom `Compositor` pass:

- **Gate 1 (glyph placement) passes.** Over 2,700 text cells on three screens, our glyphs' ink sits
  0.1 px left of Ghostty's and level with it (within 0.005 px), with the same ink weight (ratio
  1.000). No text pixel is off by more than 64 of 255.
- **Gate 3 (many panes) passes** with room to spare. The worst case, a screen's worth of new text
  every frame split over up to nine panes, takes at most 4.2 ms of the 8.3 ms frame at 120 Hz.
- **Gate 2 (sprites) passes.** Box drawing, blocks, braille, Ghostty's geometric shapes and
  Powerline, and all five underline styles, are drawn on a port of Ghostty's sprite canvas and
  its z2d rasterizer. Against Ghostty, 335 sprite cells have no pixel off by more than 16 of 255,
  blocks and braille are identical, and every underline style, undercurl included, is identical.
- **Gate 4 (layering) passes for the MVP.** Below-text kitty images will need two grids per pane,
  which is a contained change.
Follow-ups for later tickets:

- **The 0.1 px:** grid can't shift a row's line by less than a pixel, so it can't add Ghostty's
  centering offset of `(cell − advance) / 2` (0.1 px for JetBrains Mono at 14 pt, 0.215 px for
  Operator Mono at 18.7 pt). It's under the gate, but closing it means asking Rune for a
  fractional x offset on `GridRow`'s line, one addition in `paint_grid`.
- **Sprites not ported yet:** Symbols for Legacy Computing (U+1FB00 to U+1FBEF and its
  supplement) and branch drawing (U+F5D0 to U+F60D) still come from the font. `pixel-diff` lists
  them as `FontSprite`.
- **Kitty graphics:** build the two-grid pane for below-text placements, and measure it.

## What was built

The renderer lives in `crates/app/src/view/terminal/`:

- `font.rs`: registers a font's faces with Rune and measures Ghostty's whole-pixel cell for it at
  a display scale (`terminal::Metrics`, ticket 010). The row text style is the face at its point
  size, one cell tall, with tracking of `cell − advance` so glyphs land exactly one cell apart, and
  `calt` and `liga` off. It also says which characters the regular face has glyphs for.
- `rows.rs`: the `GridRow` builder, a port of Ghostty's `rebuildRow` color rules onto the grid.
  - Inverse swaps foreground and background, defaults included.
  - Selection takes `selection-background`. When that isn't set, it's the default background on
    the default foreground, as in Ghostty. A wide character's spacer is selected with it.
  - Faint text gets `faint-opacity` (0.5). Invisible text isn't drawn, read or decorated, even
    under a block cursor.
  - The narrow cells the regular face covers go into the row's line, with a span wherever color,
    weight or slant changes.
  - Backgrounds merge into fills.
  - Wide characters, multi-code-point clusters and characters the face lacks are pinned to their
    column.
  - Underline (single and double), strikethrough and overline are boxes at Ghostty's positions
    and thicknesses.
  - `RowCache` hands an unchanged row (the same snapshot `Arc`) back as the same `Rc<GridRow>`. A
    new face, look or default color clears it.
- `sprite/`: Ghostty's sprite face (ticket 018; see [Fidelity](#fidelity)). The character stays
  in the line, drawn transparent, so a screen reader still reads it.
- `glyph/`: the glyph atlas for wide, fallback, emoji and icon cells (ticket 018; see
  [Fidelity](#fidelity)).
- `cursor.rs`: block, bar, underline and hollow block in Ghostty's geometry
  (`sprite/draw/special.zig`), as an overlay. An unfocused pane shows a hollow block. A cursor on
  a wide character covers both cells. The glyph under a block cursor is drawn again in the
  cursor-text color (the terminal's background by default). The grid's accessibility caret
  follows the cursor.
- `mod.rs`: `TerminalPane`, which draws one snapshot at its own grid (the terminal's, not the
  pane's) over the terminal's background. The grid hugs the pane's top left, after an optional
  padding. That's Ghostty's `window-padding-balance = false`, and it's the "pin to top" Eli wants
  (`feature-inventory.md` §1, "Terminal grid centering"). A caller that wants balanced padding
  computes it with `terminal::Size` and passes it in.
- `look.rs`: the colors a view adds (selection, cursor, cursor text, faint opacity), and Eli's
  arctic theme.

`herdr_frontend::app()` turns Rune's font smoothing off (`FontSmoothing::Off`) for the whole
window. That's how Ghostty rasterizes by default (`font-thicken = false`); see
[Font smoothing](#font-smoothing).

The scenes `terminal-colors`, `terminal-boxes`, `terminal-nvim`, `terminal-nvim-checklist`,
`terminal-selection` and `terminal-panes` draw the fixture screens in `crates/app/fixtures/`
(`just snap terminal-nvim`).
`tests/terminal_view.rs` checks them headless:

- the grid is the terminal's size at the pane's top left;
- every row reads as the terminal's text;
- the cursor sits on its cell;
- four panes each draw;
- an unchanged row isn't built again.

The unit tests in `rows.rs`, `cursor.rs`, `sprite/` and `glyph/` pin the color rules, runs, pinning,
decorations, sprite boxes and cursor shapes.

## The pixel diff

`just ghostty-diff` (`scripts/ghostty-diff`) runs each screen through two tools:

1. `scripts/ghostty-capture` starts a separate Ghostty instance with only its own config, written
   to match the renderer's defaults: 14 pt, ligatures off, no padding, no decoration, arctic
   colors and a steady block cursor. It makes the window fullscreen in its AeroSpace workspace
   and runs the screen's program under `script`, which records every byte. Then it captures the
   window with `screencapture -l`, converts the capture from the display's Display P3 to sRGB,
   and kills only that instance.
2. Our renderer draws the recorded bytes at Ghostty's grid (`--snap --scene screen-file`), and
   `pixel-diff` compares the two cell by cell. It replays the bytes to tell what each cell holds,
   so it reports text, box lines, other sprites, pinned cells and backgrounds apart. For text it
   also measures where each cell's ink sits.

Ghostty runs the programs live rather than replaying the committed fixtures. Under a tiling
window manager its grid is whatever the window gets, and Neovim's output only replays correctly
at the grid it was drawn for.

Limits:

- It needs Screen Recording permission for the terminal that runs it, and an unlocked screen.
  Ghostty creates no window while the screen is locked, or when it's started in the background
  (`open -g`); `ghostty-capture` then fails with "no window for Ghostty".
- It shows a Ghostty window for about three seconds per screen. It types and clicks nothing.
- It's macOS at 2x only, with JetBrains Mono at 14 pt. Eli's Operator Mono Book Herdr isn't
  installed system-wide, so Ghostty can't load it with a clean config.

## Results

Ticket 018 ran `scripts/ghostty-diff` on all screens at a 177 × 51 grid, with the final harness
(captures converted to sRGB, our font smoothing off). Per screen, after the sprite and underline
work:

| screen | cells | identical | pixels differing | > 16 | > 64 | mean abs diff (0-255) |
| --- | --- | --- | --- | --- | --- | --- |
| `colors` | 9027 | 8267 | 3.42% | 0.30% | 0.02% | 0.11 |
| `boxes` | 9027 | 8735 | 0.39% | 0.11% | 0.00% | 0.03 |
| `nvim` (before the sprite work) | 9027 | 6543 | 9.17% | 1.15% | 0.02% | 0.37 |

By kind of cell:

- **Blank** cells match but for a faint fringe (2.4% of their pixels, none over 16): neighboring
  glyphs' antialiasing, 0.1 px apart (gate 1).
- **Text** cells differ in about 19% of their pixels, 4% by more than 16 and none by more than 64:
  the same 0.1 px. See [Gate 1](#gate-1-glyph-placement).
- **Sprites** and **decorations**: see [Gate 2](#gate-2-sprites).
- **Pinned** cells (wide, fallback and emoji): see [Fidelity](#fidelity).
- **Colors** match exactly, the background (`#1e1e1e`) included.

## Gate 1: glyph placement

**Passes.** The text ink of every text cell, ours minus Ghostty:

| screen | text cells | centroid dx (px) | mean \|dx\| | centroid dy (px) | mean \|dy\| | ink weight ratio |
| --- | --- | --- | --- | --- | --- | --- |
| `colors` | 428 | −0.097 | 0.097 | +0.002 | 0.005 | 1.000 |
| `boxes` | 191 | −0.100 | 0.100 | +0.001 | 0.004 | 1.001 |
| `nvim` | 2094 | −0.098 | 0.098 | +0.002 | 0.003 | 1.000 |

- **Horizontal.** Every glyph sits 0.1 px left of Ghostty's, the offset predicted here before:
  Ghostty centers each glyph in the rounded cell by `(cell − advance) / 2`
  (`ghostty:src/font/face/coretext.zig:363-378`), and grid places the line's glyphs one cell apart
  from the cell's left edge. For JetBrains Mono at 14 pt and 2x that's 0.1 px. Grid snaps each
  row's origin to a whole device pixel, so the renderer can't add it (see the follow-ups).
- **Vertical.** Level, within 0.005 px. Ghostty puts the baseline on a whole pixel,
  `cell_baseline` up from the cell's bottom; Rune centers the line box in the line height and
  snaps the baseline, and for both fonts at 2x the two land on the same pixel row (29 px from the
  cell's top for JetBrains Mono, 36 for Operator Mono). The −0.3 px of the first, smoothed run was
  the smoothing's dilation.
- **Weight.** The same: the ratio is 1.000 with font smoothing off (it was 1.294 with it on).

## Gate 2: sprites

**Passes.** Every sprite this port draws (see [Fidelity](#fidelity)) comes out within 16 of 255 of
Ghostty's, most of them identical. From the `boxes` and `colors` screens:

| sprites | cells | identical | pixels differing | > 16 | > 64 |
| --- | --- | --- | --- | --- | --- |
| all ported sprites | 335 | 236 | 1.81% | 0.00% | 0.00% |
| box-drawing lines, dashes, corners, tees, crosses | 262 | 187 | ≤ 3.1% a character | 0.00% | 0.00% |
| arcs `╭╮╯╰` | 8 | 0 | ≤ 5.0% | 0.00% | 0.00% |
| diagonals `╱╲╳` | 3 | 0 | ≤ 1.9% | 0.00% | 0.00% |
| blocks, shades, quadrants | 33 | 33 | 0.00% | 0.00% | 0.00% |
| braille | 16 | 16 | 0.00% | 0.00% | 0.00% |
| geometric shapes `◢◣◤◥◸◹◺◿` | 8 | 0 | ≤ 1.0% | 0.00% | 0.00% |
| Powerline `` | 5 | 0 | ≤ 54% | 0.00% | 0.00% |
| underlines (single, double, curly, dotted, dashed) | — | identical rows | 0.00% | 0.00% | 0.00% |

- The differences are rounding of a level or two in a color channel: an arc's antialiased pixels
  are byte-identical to Ghostty's in the channel checked, and the Powerline cells differ by at
  most a level across their colored fill.
- The `─` and `│` cells that differ at all do so where a neighbor's glyph reaches into them.
- The underlines were compared row by row in cells with no descender: the undercurl (with its
  round caps), the dots and the dashes are Ghostty's pixels exactly, in the underline color.

## Gate 3: many panes

`just pane-budget` (`crates/app/examples/pane-budget.rs`, release build) drives N panes through
Rune's real runtime with no window. Each frame:

1. every pane's terminal takes new output and hands over a snapshot, as its worker thread would;
2. the runtime builds the frame: the view builds the changed rows, Rune shapes them, lays the
   window out and paints its instances.

Step 2 is the UI thread's whole frame. Step 1 runs on the terminals' worker threads in the app,
so it's timed apart. GPU time isn't included. Each load runs for 240 frames. Every line of output
is new text, so nothing comes from the text system's shaping cache across panes or frames. The
pane layouts fill a 14-inch MacBook Pro's 1512 × 945 pt screen, plus Rune's own 200 × 60
reference.

| panes | load | rows built a frame | UI p50 ms | UI p95 ms | UI p99 ms | parse + snapshot p50 ms (all panes) |
| --- | --- | --- | --- | --- | --- | --- |
| 1 × 200×60 | one row | 1 | 0.23 | 0.35 | 0.87 | 0.01 |
| 1 × 177×51 | one row | 1 | 0.15 | 0.20 | 0.28 | 0.01 |
| 2 × 88×51 | one row | 3 | 0.15 | 0.21 | 0.85 | 0.01 |
| 4 × 88×25 | one row | 5 | 0.18 | 0.27 | 0.73 | 0.02 |
| 9 × 58×17 | one row | 11 | 0.24 | 0.42 | 0.88 | 0.03 |
| 1 × 200×60 | one line | 60 | 1.01 | 1.18 | 1.23 | 0.44 |
| 1 × 177×51 | one line | 51 | 0.77 | 0.93 | 0.96 | 0.33 |
| 2 × 88×51 | one line | 102 | 0.78 | 1.00 | 1.05 | 0.35 |
| 4 × 88×25 | one line | 100 | 0.84 | 1.04 | 1.11 | 0.34 |
| 9 × 58×17 | one line | 153 | 0.96 | 1.21 | 1.27 | 0.36 |
| 1 × 200×60 | screen scrolls | 60 | 4.42 | 4.69 | 4.80 | 0.52 |
| 1 × 177×51 | screen scrolls | 51 | 3.32 | 3.57 | 3.63 | 0.37 |
| 2 × 88×51 | screen scrolls | 102 | 3.52 | 3.87 | 4.02 | 0.43 |
| 4 × 88×25 | screen scrolls | 100 | 3.49 | 3.81 | 3.96 | 0.41 |
| 9 × 58×17 | screen scrolls | 153 | 3.76 | 4.13 | 4.23 | 0.46 |
| 1 × 200×60 | full repaint | 60 | 4.47 | 4.76 | 4.85 | 0.49 |
| 1 × 177×51 | full repaint | 51 | 3.35 | 3.59 | 3.67 | 0.37 |
| 2 × 88×51 | full repaint | 102 | 3.56 | 3.93 | 4.03 | 0.41 |
| 4 × 88×25 | full repaint | 100 | 3.48 | 3.83 | 3.95 | 0.38 |
| 9 × 58×17 | full repaint | 153 | 3.78 | 4.09 | 4.19 | 0.46 |

The loads:

- **one row:** a character typed at a prompt.
- **one line:** a log printing a line a frame.
- **screen scrolls:** a screenful of new lines every frame.
- **full repaint:** every row rewritten in place with new text.

**Passes.** The worst case is a screen's worth of new text every frame, however many panes it's
split into. Even then the UI thread needs 3.3 to 3.8 ms at p50 and at most 4.2 ms at p99, about
half of the 8.3 ms frame at 120 Hz. The cost follows the number of cells, not the number of
panes: nine panes of 58 × 17 cost about what one pane of 177 × 51 does.

- **Scrolling rebuilds every row.** libghostty marks every row dirty when the screen scrolls
  (`terminal-rendering.md` §2 "Dirty tracking"), so "one line" rebuilds every row, as the
  "rows built" column shows. It still costs only about 1 ms: the shifted rows' text is the same,
  so the text system finds their shaped layouts in its cache.
- **Typing is cheap.** One changed row costs 0.15 to 0.25 ms at p50, whatever the pane count.
- **Rune's reference is a different measure.** Rune's own 200 × 60 full redraw figure is 2.46 ms.
  Ours is 4.4 ms, but it includes building every `GridRow` and shaping text that has never been
  seen before.

## Gate 4: layering

Grid paints each row in one pass: its fills, then its line clipped to the row, then its pinned
cells, then its vector layer (`rune:crates/rune/src/runtime/paint/grid.rs:50-139`). Nothing outside
the grid can draw between a row's backgrounds and its text. This MVP needs nothing there, so
nothing it draws is affected. Three later features are:

- **Find highlights under the text** (ticket 019) can go in the row's fills, as selection does
  here. A highlight is then part of the row, and changing it rebuilds that row, which "one row"
  above shows costs about 0.2 ms.
- **Kitty images in the below-text layer** (z below 0, `types.h:380-407`, which includes Unicode
  placeholders) need two grids per pane: one with the fills only, and one with the text only,
  with the images between them. Both take the same rows' data, and each row is cached on its
  own. The kitty graphics ticket has to build and measure this. A pane with no below-text
  placements keeps the one grid.
- **Glyphs that overflow their row** are clipped. That affects tall accents and descenders in
  fonts whose glyphs reach outside the line, which `Metrics.zig:249-256` allows for. Pinned cells
  aren't clipped. How often it shows for JetBrains Mono and Operator Mono is for the Ghostty diff
to say (it's not measured here).

**Passes for the MVP.** Below-text images are the one known case that needs the two-grid
arrangement. That's a contained change inside the pane view, not a reason to leave grid.

## Fidelity

Ticket 018, stage 5 of [`terminal-rendering.md` §8](terminal-rendering.md#staged-plan).

### Sprites

`crates/app/src/view/terminal/sprite/` is Ghostty's sprite face (`ghostty:src/font/sprite/` at
`v1.4.0`):

- `canvas.rs` is its canvas: an alpha-only surface the cell's size plus a quarter cell of padding
  on each side, so a sprite can reach past its cell as Ghostty's do.
- `raster.rs` ports the z2d 0.13 fill Ghostty pins (`40af3e84`):
  - curves flattened by de Casteljau subdivision to 0.1 px, and arcs split as z2d splits them;
  - edges at 4× scale, sampled 4 × 4 a pixel, crossings rounded to a quarter pixel;
  - alpha `16 × coverage − 1`, composited in z2d's integer arithmetic.

  Strokes become fills of the same region: a quad per segment, z2d's pen as a disc at round
  joins and caps, a wedge at miter joins, all wound the same way. That region is what z2d's own
  outline encloses, and the measured arcs, diagonals and undercurl match its pixels.
- `box_drawing.rs`, `blocks.rs` (blocks, braille, geometric shapes), `powerline.rs` and
  `special.rs` (the underline styles, strikethrough, overline) port the `sprite/draw/*.zig`
  files of those ranges.

A finished canvas becomes boxes of one alpha each, merged along rows and down columns. The row's
vector layer fills them at whole device pixels in the cell's color at that alpha. Rune blends
8-bit targets in sRGB-encoded space, as Ghostty does on macOS (`alpha-blending = native`), so a
partly covered pixel comes out the same. Each sprite is drawn once per code point and cell size.

The decorations follow Ghostty's `rebuildRow`: every cell draws its own underline, strikethrough
and overline sprite, a wide character's spacer too, so a curl or dots start again in each cell.
The underline takes the underline color (SGR 58) if one is set.

Where it differs from Ghostty:

- **Underlines draw over the text.** Ghostty draws them under the glyphs; grid's vector layer is
  above the row's line. Only pixels where a descender crosses an underline of another color
  differ.
- **A sprite reaching into the next row** (an underline set low by its font) is painted under
  that row's background fill, where Ghostty paints all backgrounds first.
- **Inner strokes** (`` `` and the outlined triangles `◸◹◺◿`) stroke the path inset by
  half the line width directly, rather than through z2d's path offsetter. The triangles measured
  within 1% of pixels, none over 16; the two outlined half circles aren't in a screen yet.
- **Not ported yet:** Symbols for Legacy Computing and branch drawing still come from the font.

### Glyph atlas

`crates/app/src/view/terminal/glyph/` draws the cells the row's line can't place the way Ghostty
does: wide characters, characters the terminal font lacks, emoji and Nerd Font icons. Each is
rasterized once, with the same CoreText calls as Ghostty's `coretext.zig` `renderGlyph`, and drawn
as an image over the grid at its device pixels. Its pinned text stays in the row, drawn
transparent, so the row still reads.

- **Fallback** follows Ghostty's resolver:
  1. the Symbols Nerd Font 3.4.0 Ghostty embeds (bundled in `assets/fonts/`, MIT);
  2. Apple Color Emoji for emoji presentation;
  3. fallback faces already loaded;
  4. CoreText's fallback for the character, scaled to match the terminal font's ideograph width
     (Ghostty's `ic_width` size adjustment).

  A variation selector picks the presentation; without one, it's the Unicode
  `Emoji_Presentation` property (from the `emoji-data.txt` of the uucode Ghostty pins), as in
  Ghostty's `CodepointResolver`.
- **Constraints** port `RenderOptions.Constraint`:
  - every emoji covers its cells, centered, with 2.5% side padding;
  - Nerd Font icons follow Ghostty's generated table (`glyph/tables.rs`, from
    `scripts/gen_ghostty_tables.py`);
  - other symbol-like characters shrink to fit;
  - a symbol followed by a blank cell may take two cells (`constraintWidth`).
- **Rasterizing** is antialiased with no font smoothing, subpixel positioned without quantizing,
  with the glyph's box constrained and centered in the rounded cell, and bitmap (sbix) glyphs
  quantized to whole pixels. Text glyphs are alpha masks drawn in the cell's color; a block cursor
  draws one again in the cursor-text color.
- **Color glyphs are drawn in sRGB.** Ghostty draws them in Display P3 because its surface is P3;
  Rune's is sRGB, so sRGB gives the colors Ghostty shows.

Measured on the `colors` screen's 28 pinned cells, before and after the atlas (the atlas run drew
emoji in Display P3 and didn't yet reuse loaded fallback faces):

| | pixels differing | > 16 | > 64 | mean abs diff (0-255) |
| --- | --- | --- | --- | --- |
| before (Rune's pinned text) | 28.46% | 22.78% | 14.59% | 21.09 |
| atlas | 20.74% | 14.90% | 5.73% | 10.39 |

- Kanji `日本語` came out within 2.5% of pixels, none over 16.
- The kana and Hangul came from a different fallback face than Ghostty's. Ghostty uses a face
  it has already loaded before asking CoreText again, so the kana take the kanji's face. The
  atlas now does the same.
- The emoji had Ghostty's size and place (no pixel over 64) but not its colors, which the sRGB
  fix addresses.

**Not measured yet:** those two fixes, and the Neovim checklist screen, need a Ghostty capture,
and the screen was locked. `scripts/ghostty-diff colors nvim-checklist` measures them.

What it doesn't take yet:

- **Clusters of several code points** (combining marks, emoji ZWJ sequences, skin tones) stay
  pinned cells that Rune shapes, as before.
- **Bold and italic** cells fall back to the fallback faces' regular style.
- **Off macOS**, every such cell stays a pinned cell.

### Neovim checklist

`fixtures/nvim-checklist.bin` (scene `terminal-nvim-checklist`, `tests/terminal_view.rs`) and the
`nvim-checklist` screen of `scripts/ghostty-diff` run `fixtures/nvim-checklist.lua`.

| item | how it draws | Ghostty diff |
| --- | --- | --- |
| splits | `│` separators as sprites; the `nvim` screen | lines ≤ 3.1% of pixels, none over 16 |
| floats | rounded border: `╭╮╯╰` arcs and `─│` lines as sprites | arcs ≤ 5%, none over 16 |
| undercurl | per-cell curl sprites in the SGR 58 color; dotted and dashed too | identical |
| statusline icons | Nerd Font glyphs through the atlas with Ghostty's constraints; Powerline separators as sprites | separators within 16; icons not measured yet |
| cmdline | text on the line, the cursor in it | not measured yet |
| cursor | the cell's whole-pixel box, the glyph under a block redrawn | identical on every screen measured |
| CJK and emoji | the atlas | see [Glyph atlas](#glyph-atlas) |
| ligatures off | `calt` and `liga` off, as Ghostty's `-calt`, `-liga` | `-> => != fi fl` measure as text |

## Font smoothing

Rune draws text on macOS with CoreText's font smoothing on by default
(`FontSmoothing::On`), which dilates outlines by a radius set by the text color. Ghostty doesn't
smooth unless `font-thicken` is set, and Eli's config doesn't set it. In the earlier run, our text
carried 29% more ink than Ghostty's (ratio 1.294).

So `herdr_frontend::app()` sets `FontSmoothing::Off`, Rune's CoreText pipeline without the
dilation. Rune applies it per window, so the app's own text (sidebar, tabs, palette) draws the
same way. With it off, the ratio is 1.000 ([Gate 1](#gate-1-glyph-placement)).
