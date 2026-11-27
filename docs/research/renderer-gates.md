# Renderer gates: `rune::grid` against Ghostty

Ticket `012-terminal-renderer-mvp-on-rune-grid`, with gate 1 and the fidelity work measured in
ticket `018-terminal-rendering-fidelity-for-neovim`, and the Neovim checklist and glyph atlas in
ticket `037-ghostty-diffs-for-the-neovim-checklist-and-glyph-atlas`. Measured 2026-11-26 on an
Apple M1 Pro (8 cores), macOS 27, built-in 2x display, against Rune `4918969f` and Ghostty.app
1.4.0. The font is JetBrains Mono 2.304 at 14 pt, the face Ghostty embeds, so both apps use the
same file. Its cell is 17 × 37 px at 2x. This doc answers the four gates in
[`terminal-rendering.md` §2](terminal-rendering.md#where-the-renderer-draws) and stage 4 of §8.

## Recommendation

**Stay on `rune::grid`.** Nothing measured here argues for the custom `Compositor` pass:

- **Gate 1 (glyph placement) passes.** Over 3,100 text cells on four screens, our glyphs' ink sits
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
  - The part of a sprite below the row, an undercurl's lower half, is kept apart. The pane draws
    it over the grid, so it isn't hidden under the next row's background fill.
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
  pane's) over the terminal's background, with the rows' sprite spill, the atlas cells and the
  cursor over it. The grid hugs the pane's top left, after an optional
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
   also measures where each cell's ink sits. `--region NAME=COL,ROW,COLS,ROWS` reports a block of
   cells on its own; `ghostty-diff` names the checklist's split, float, statusline and cmdline
   that way.

`scripts/ghostty-diff --reuse` keeps the last Ghostty captures and only draws and compares ours
again, so a renderer fix can be checked without opening Ghostty.

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
- Converting the capture from Display P3 back to sRGB can move a color by a level. Neovim's
  statusline grey `#4f5258` (79, 82, 88) comes back as (80, 82, 87), so 77% of the statusline's
  pixels "differ", nearly all by a level.

## Results

Ticket 037 ran `scripts/ghostty-diff` on all four screens at a 177 × 51 grid, with the final
harness (captures converted to sRGB, our font smoothing off) and its fixes in (see
[Fidelity](#fidelity)):

| screen | cells | identical | pixels differing | > 16 | > 64 | mean abs diff (0-255) |
| --- | --- | --- | --- | --- | --- | --- |
| `colors` | 9027 | 8267 | 3.39% | 0.25% | 0.00% | 0.08 |
| `boxes` | 9027 | 8735 | 0.39% | 0.11% | 0.00% | 0.03 |
| `nvim` | 9027 | 6545 | 9.16% | 1.12% | 0.00% | 0.35 |
| `nvim-checklist` | 9027 | 8246 | 3.98% | 0.22% | 0.00% | 0.08 |

Ticket 018's last run, before them, had `colors` at 0.30% over 16 and 0.02% over 64, and `nvim`
(before the sprite work) at 1.15% and 0.02%.

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
| `nvim-checklist` | 442 | −0.095 | 0.095 | +0.001 | 0.003 | 1.000 |

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
| undercurl's lower half, over Neovim's filled next row | 3 | 0 | 2.12% | 0.00% | 0.00% |

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
  aren't clipped. For JetBrains Mono at 14 pt it doesn't show: no text pixel on the four screens
  is off by more than 64, and the ink weight matches. Operator Mono isn't measured.

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
- **A sprite reaching into the next row** (JetBrains Mono's undercurl) is drawn over the grid,
  after every row, as Ghostty paints all backgrounds first. The grid alone painted it under the
  next row's background fill. That only showed where the next row has a fill, as every row does
  under a Neovim colorscheme: the cells under the checklist's error curl had 2.54% of their
  pixels off by more than 64, and have none over 16 now. It's drawn over the next row's text
  too, like the underlines.
- **Inner strokes** (`` `` and the outlined triangles `◸◹◺◿`) stroke the path inset by
  half the line width directly, rather than through z2d's path offsetter. The triangles measured
  within 1% of pixels, none over 16; the two outlined half circles aren't in a screen yet.
- **Not ported yet:** Symbols for Legacy Computing and branch drawing still come from the font.

### Glyph atlas

`crates/app/src/view/terminal/glyph/` draws the cells the row's line can't place the way Ghostty
does: wide characters, characters the terminal font lacks, emoji and Nerd Font icons, and the
terminal font's own glyphs that Ghostty constrains. Each is rasterized once, with the same
CoreText calls as Ghostty's `coretext.zig` `renderGlyph`, and drawn as an image over the grid at
its device pixels. Its pinned text stays in the row, drawn transparent, so the row still reads.

- **Fallback** follows Ghostty's resolver:
  1. the terminal font itself;
  2. for a terminal font other than JetBrains Mono, the JetBrains Mono Ghostty embeds as its
     backup (ticket 017);
  3. the Symbols Nerd Font 3.4.0 Ghostty embeds (bundled in `assets/fonts/`, MIT);
  4. Apple Color Emoji for emoji presentation;
  5. fallback faces already loaded;
  6. Ghostty's discovery (`glyph/discovery.rs`, from `discovery.zig` `discoverFallback`). For a
     Han ideograph (U+4E00 to U+9FFF), it takes CoreText's fallback from the terminal font. For
     anything else, it takes the fonts CoreText matches for the character, sorted by Ghostty's
     score: has the character, then monospace, then the style name, then not italic, then not
     bold, then the most glyphs. The first face with the character in the wanted presentation
     wins. It's scaled to match the terminal font's ideograph width (Ghostty's `ic_width` size
     adjustment).

  A variation selector picks the presentation; without one, it's the Unicode
  `Emoji_Presentation` property (from the `emoji-data.txt` of the uucode Ghostty pins), as in
  Ghostty's `CodepointResolver`. On this Mac that gives Ghostty's faces: kanji from PingFang SC;
  kana from Arial Unicode MS, the score's pick (PingFang SC has no kana, and CoreText's own
  fallback would be Hiragino Sans); Hangul from Arial Unicode MS too, already loaded.
- **Constraints** port `RenderOptions.Constraint`, for every glyph as Ghostty's `addGlyph` does,
  whatever face it comes from:
  - every emoji covers its cells, centered, with 2.5% side padding;
  - Nerd Font icons follow Ghostty's generated table (`glyph/tables.rs`, from
    `scripts/gen_ghostty_tables.py`);
  - other symbol-like characters shrink to fit;
  - a symbol followed by a blank cell may take two cells (`constraintWidth`).

  So a character the terminal font has but Ghostty constrains, such as JetBrains Mono's
  Powerline branch `` or its arrows and dingbats, leaves the row's line for the atlas. The
  atlas has only the terminal font's regular face, so in a bold or italic cell (a prompt's bold
  `❯`) such a glyph stays on the line, in its style but unconstrained.
- **Rasterizing** is antialiased with no font smoothing, subpixel positioned without quantizing,
  with the glyph's box constrained and centered in the rounded cell, and bitmap (sbix) glyphs
  quantized to whole pixels. Text glyphs are alpha masks drawn in the cell's color; a block cursor
  draws one again in the cursor-text color.
- **Color glyphs are drawn in sRGB, with Ghostty's edges.** Ghostty draws them in Display P3
  because its surface is P3; Rune's is sRGB, so sRGB gives the colors Ghostty shows. Ghostty's
  color atlas is an sRGB texture, so its shader decodes CoreGraphics' premultiplied values,
  divides out the alpha and encodes again (`shaders.metal` `ATLAS_COLOR`). That darkens every
  partly covered pixel, and the atlas makes the same straight color, `encode(decode(v) / a)`.

Measured on the pinned cells of the `colors` screen (28 cells: kanji, kana, Hangul, two emoji
and two letters with combining marks) and the `nvim-checklist` screen (49: the comment's kanji,
kana and emoji, the statusline and float icons, and `✓`):

| screen | run | pixels differing | > 16 | > 64 | mean abs diff (0-255) |
| --- | --- | --- | --- | --- | --- |
| `colors` | before the atlas (Rune's pinned text) | 28.46% | 22.78% | 14.59% | 21.09 |
| `colors` | ticket 018's atlas | 20.74% | 14.90% | 5.73% | 10.39 |
| `colors` | its sRGB emoji and face reuse | 18.21% | 11.02% | 5.73% | 9.70 |
| `colors` | ticket 037 | 9.62% | 0.30% | 0.00% | 0.19 |
| `nvim-checklist` | before ticket 037 | 22.43% | 5.23% | 2.37% | 3.50 |
| `nvim-checklist` | ticket 037 | 21.46% | 0.00% | 0.00% | 0.13 |

Every atlas cell now has no pixel off by more than 16. What's left over 16 on `colors` is the two
letters with combining marks, which Rune still shapes. By character:

| characters | screen | pixels differing | > 16 | > 64 |
| --- | --- | --- | --- | --- |
| kanji `日本語` | both | ≤ 38% | 0.00% | 0.00% |
| kana `のキステト`, `のコトメン` | both | ≤ 25% | 0.00% (was ≤ 21%) | 0.00% (was ≤ 14%) |
| Hangul `한국어` | `colors` | ≤ 13% | 0.00% (was ≤ 39%) | 0.00% (was ≤ 26%) |
| emoji `🙂🚀` | both | ≤ 36% | 0.00% (was ≤ 8%) | 0.00% |
| Nerd Font icons `` | `nvim-checklist` | ≤ 47% | 0.00% | 0.00% |
| JetBrains Mono's `` and `✓`, constrained | `nvim-checklist` | ≤ 35% | 0.00% | 0.00% |

The pixels that differ by a level or two are rounding of antialiased edges; the share is high
for icons and emoji because they're mostly edge.

What it doesn't take yet:

- **Clusters of several code points** (combining marks, emoji ZWJ sequences, skin tones) stay
  pinned cells that Rune shapes, as before.
- **Bold and italic** cells fall back to the fallback faces' regular style, as in Ghostty. The
  terminal font's own constrained glyphs in those cells aren't constrained yet: that needs its
  bold and italic faces in the atlas.
- **Off macOS**, every such cell stays a pinned cell.

### Neovim checklist

`fixtures/nvim-checklist.bin` (scene `terminal-nvim-checklist`, `tests/terminal_view.rs`) and the
`nvim-checklist` screen of `scripts/ghostty-diff` run `fixtures/nvim-checklist.lua`. Ticket 037
measured it in Ghostty at 177 × 51, the regions as `ghostty-diff` names them:

| item | how it draws | Ghostty diff (pixels differing, > 16, > 64) |
| --- | --- | --- |
| splits | `│` separators as sprites | the split column's 49 cells identical; all 55 `│` cells identical |
| floats | rounded border: `╭╮╯╰` arcs and `─│` lines as sprites | the float's 150 cells: 4.89%, 1.98%, 0.00%, the > 16 all in its text (gate 1's 0.1 px); arcs 2.54%, 0.00%, 0.00%; every `─` and `│` cell on the screen identical |
| undercurl | per-cell curl sprites in the SGR 58 color, the lower half over the next row; dotted and dashed too | identical on `colors`; the cells under the error's curl 2.12%, 0.00%, 0.00% (they were 3.50%, 2.86%, 2.54% before ticket 037) |
| statusline icons | Nerd Font glyphs through the atlas with Ghostty's constraints, JetBrains Mono's `` too; Powerline separators as sprites | icons ≤ 47%, 0.00%, 0.00%; `` 34.90%, 0.00%, 0.00% (was 0.29% of the statusline over 64); separators ≤ 51%, 0.00%, 0.00%; the statusline's 177 cells 77.14%, 0.01%, 0.00%, the 77% from the capture's color conversion (see [limits](#the-pixel-diff)) |
| cmdline | text on the line | its 177 cells: 1.03%, 0.48%, 0.00%, the > 16 all in its text (gate 1's 0.1 px) |
| cursor | the cell's whole-pixel box, the glyph under a block redrawn | the cmdline cursor Neovim sets, after `:%s/nam/name`, identical, as on every screen measured |
| CJK | the atlas | kanji ≤ 38%, kana ≤ 25%, all 0.00% over 16 (see [Glyph atlas](#glyph-atlas)) |
| emoji | the atlas | `🚀` 30.52%, 0.00%, 0.00% |
| fallback | the atlas | `✓` (JetBrains Mono's, constrained) 5.88%, 0.00%, 0.00%; float icons ≤ 15%, 0.00%, 0.00% |
| ligatures off | `calt` and `liga` off, as Ghostty's `-calt`, `-liga` | `-> => != fi fl` measure as text |

Every item passes: no pixel of an item is off by more than 16, other than text's 0.1 px offset
(gate 1). The text on this screen measures as on the others: ink 0.095 px left of Ghostty's,
level, the same weight.

## Font smoothing

Rune draws text on macOS with CoreText's font smoothing on by default
(`FontSmoothing::On`), which dilates outlines by a radius set by the text color. Ghostty doesn't
smooth unless `font-thicken` is set, and Eli's config doesn't set it. In the earlier run, our text
carried 29% more ink than Ghostty's (ratio 1.294).

So `herdr_frontend::app()` sets `FontSmoothing::Off`, Rune's CoreText pipeline without the
dilation. Rune applies it per window, so the app's own text (sidebar, tabs, palette) draws the
same way. With it off, the ratio is 1.000 ([Gate 1](#gate-1-glyph-placement)).
