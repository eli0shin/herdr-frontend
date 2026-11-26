# Renderer gates: `rune::grid` against Ghostty

Ticket `012-terminal-renderer-mvp-on-rune-grid`. Measured 2026-11-26 on an Apple M1 Pro (8 cores),
macOS 27, built-in 2x display, against Rune `4918969f` and Ghostty.app 1.4.0. The font is
JetBrains Mono 2.304 at 14 pt, the face Ghostty embeds, so both apps use the same file. Its cell is
17 × 37 px at 2x. This doc answers the four gates in
[`terminal-rendering.md` §2](terminal-rendering.md#where-the-renderer-draws) and stage 4 of §8.

## Recommendation

**Stay on `rune::grid`.** Nothing measured here argues for the custom `Compositor` pass:

- **Gate 3 (many panes) passes** with room to spare. The worst case, a screen's worth of new text
  every frame split over up to nine panes, takes at most 4.2 ms of the 8.3 ms frame at 120 Hz.
- **Gate 2 (sprites): the box-drawing lines pass in our own rendering.** They come out
  pixel-exact, Ghostty's boxes on whole device pixels with no antialiasing, at 1x and at 2x. The
  other sprites still come from the font; that's the fidelity ticket's work, and grid's vector
  layer can carry them.
- **Gate 4 (layering) passes for the MVP.** Below-text kitty images will need two grids per pane,
  which is a contained change.
- **Gate 1 (glyph placement) isn't measured yet.** The Ghostty captures it needs couldn't be taken
  for this ticket (see [The pixel diff](#the-pixel-diff)). The one earlier comparison put our
  glyphs within a third of a pixel of Ghostty's, and the known horizontal difference is smaller
  than a quarter pixel. Nothing there suggests a failure, but it's an open gate.

Follow-ups for later tickets:

- **Ticket 018:** run `just ghostty-diff` with the screen unlocked and record gate 1 here, with the
  text ink ratio now that font smoothing is off. Then port the rest of `sprite/draw/*` and check
  each sprite in the same diff.
- **Gate 1, if the diff shows it:** grid can't shift a row's line by less than a pixel, so it can't
  add Ghostty's centering offset of `(cell − advance) / 2`. Ask Rune for a fractional x offset on
  `GridRow`'s line; it's one addition in `paint_grid`.
- **Kitty graphics:** build the two-grid pane for below-text placements, and measure it.

## What was built

The renderer lives in `crates/app/src/view/terminal/`:

- `font.rs`: registers a font's faces with Rune and measures Ghostty's whole-pixel cell for it at
  a display scale (`terminal::Metrics`, ticket 010). The row text style is the face at its point
  size, one cell tall, with tracking of `cell − advance` so glyphs land exactly one cell apart, and
  `calt` and `liga` off. It also says which characters the regular face has glyphs for.
- `rows.rs`: the `GridRow` builder, a port of Ghostty's `rebuildRow` color rules onto the grid.
  - Inverse swaps foreground and background, defaults included.
  - Selection takes `selection-background`, or swaps colors when that isn't set, as Ghostty does.
  - Faint text gets `faint-opacity` (0.5), and invisible text isn't drawn or read.
  - The narrow cells the regular face covers go into the row's line, with a span wherever color,
    weight or slant changes.
  - Backgrounds merge into fills.
  - Wide characters, multi-code-point clusters and characters the face lacks are pinned to their
    column.
  - Underline (single and double), strikethrough and overline are boxes at Ghostty's positions
    and thicknesses.
  - `RowCache` hands an unchanged row (the same snapshot `Arc`) back as the same `Rc<GridRow>`. A
    new face, look or default color clears it.
- `sprite.rs`: Ghostty's `linesChar` (`sprite/draw/box.zig`), ported for the 109 box-drawing line
  characters it draws: light, heavy and double lines, corners, tees, crosses and half lines. They
  go into the row's vector layer at whole device pixels. The character stays in the line, drawn
  transparent, so a screen reader still reads it.
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

The scenes `terminal-colors`, `terminal-boxes`, `terminal-nvim`, `terminal-selection` and
`terminal-panes` draw the fixture screens in `crates/app/fixtures/` (`just snap terminal-nvim`).
`tests/terminal_view.rs` checks them headless:

- the grid is the terminal's size at the pane's top left;
- every row reads as the terminal's text;
- the cursor sits on its cell;
- four panes each draw;
- an unchanged row isn't built again.

The unit tests in `rows.rs`, `cursor.rs` and `sprite.rs` pin the color rules, runs, pinning,
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
  (`open -g`). The screen locked partway through this ticket, so the final captures weren't
  taken.
- It shows a Ghostty window for about three seconds per screen. It types and clicks nothing.
- It's macOS at 2x only, with JetBrains Mono at 14 pt. Eli's Operator Mono Book Herdr isn't
  installed system-wide, so Ghostty can't load it with a clean config.

## Results

One full run came before the screen locked. It was on the `colors` screen at a 177 × 51 grid, and
it predates two harness fixes: the capture wasn't converted to sRGB yet, and our renderer still
had Rune's default font smoothing on. It found:

| cells | count | identical | pixels differing | > 16 | > 64 | mean abs diff (0-255) |
| --- | --- | --- | --- | --- | --- | --- |
| Blank | 8525 | 7990 | 6.26% | 3.16% | 0.63% | 0.80 |
| Text | 473 | 12 | 28.64% | 15.29% | 4.21% | 7.13 |
| Pinned | 28 | 0 | 29.05% | 24.46% | 15.21% | 21.74 |
| Cursor | 1 | 1 | 0.00% | 0.00% | 0.00% | 0.00 |
| all | 9027 | 8003 | 7.51% | 3.86% | 0.87% | 1.20 |

- **Text ink:** over 461 cells, ours minus Ghostty, the centroid was dx −0.083 px (mean |dx|
  0.087) and dy −0.317 px (mean |dy| 0.318). The ink weight ratio was 1.294.
- **Colors:** the background matched exactly (`#1e1e1e` in both). Saturated colors in the 256-color
  cube and the palette differed, because the capture was still in Display P3. Converting it to
  sRGB fixes that; on a later, partial capture the cube's colors and the 16-color text matched.

## Gate 1: glyph placement

**Not measured yet.** The earlier run's numbers point the right way but can't settle it, for
three reasons:

- Rune's font smoothing was still on, which grows every glyph's outline.
- Only 461 text cells were measured.
- No run was made with the final harness.

What's known:

- **Horizontal.** Ghostty centers each glyph in the rounded cell by `(cell − advance) / 2`
  (`ghostty:src/font/face/coretext.zig:363-378`). Grid places the line's glyphs one cell apart
  from the cell's left edge (the tracking), so they sit that far to the left of Ghostty's:
  - 0.1 px for JetBrains Mono at 14 pt and 2x;
  - 0.215 px for Eli's Operator Mono at 18.7 pt and 2x.

  The run's dx of −0.083 px fits the first. Grid snaps each row's origin to a whole device pixel,
  so the renderer can't add the offset.
- **Vertical.** Ghostty puts the baseline on a whole pixel, `cell_baseline` up from the cell's
  bottom. Rune centers the line box in the line height, then snaps the baseline. Worked out for
  both fonts at 2x, the two land on the same pixel row: 29 px from the cell's top for JetBrains
  Mono, 36 for Operator Mono. The `terminal-boxes` snap confirms the first: the bottom of the `L`
  in row 0 is the pixel row just above y = 29. The run's dy of −0.3 px may come from the
  smoothing's dilation; the next run will tell.

## Gate 2: sprites

**The box-drawing lines pass in our rendering.** `sprite.rs` ports Ghostty's `linesChar` for its
109 characters, and the unit tests pin the boxes to Ghostty's formulas. In the snaps, every line
lands on whole device pixels in the line color, with no antialiasing on its edges:

- a light `│` and `─` are 2 px at 2x and 1 px at 1x;
- a heavy `┃` is 2 px at 1x.

Ghostty fills the same boxes at full coverage, so the two should be pixel-identical. The
Ghostty diff will confirm it. Its `Lines` row counts these cells apart from the others.

Everything else Ghostty draws as a sprite still comes from the font: arcs, dashes, diagonals,
blocks, shades, braille, Powerline and legacy computing. That's ticket 018, which can draw them
the same way.

- **Arcs and diagonals need antialiased vector paths.** Grid's vector layer draws those too.
- **Blocks, shades and braille** are boxes like the lines.
- **Shades** are partial-alpha fills.

If a path doesn't match Ghostty's raster closely enough, the fallback is the research doc's:
rasterize it once per cell size with tiny-skia and draw it as an image cell.

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

## Font smoothing

Rune draws text on macOS with CoreText's font smoothing on by default
(`FontSmoothing::On`), which dilates outlines by a radius set by the text color. Ghostty doesn't
smooth unless `font-thicken` is set, and Eli's config doesn't set it. In the earlier run, our text
carried 29% more ink than Ghostty's (ratio 1.294).

So `herdr_frontend::app()` sets `FontSmoothing::Off`, Rune's CoreText pipeline without the
dilation. Rune applies it per window, so the app's own text (sidebar, tabs, palette) draws the
same way. The ratio with smoothing off is for the next Ghostty run to
measure.
