# Look and settings

herdr-frontend reads its look and preferences from files in its home (`$HERDR_FRONTEND_HOME`,
else `$XDG_CONFIG_HOME/herdr-frontend`, else `~/.config/herdr-frontend`). With no files at all it
looks like Eli's Herdr app (`docs/research/feature-inventory.md` §2): the arctic theme,
Operator Mono Book Herdr at 17 pt, zoom 1.1. The code is `crates/app/src/config/`.

| File | What it is |
| --- | --- |
| `settings.json` | Zoom, fonts, terminal metrics, minimum contrast, theme, color overrides |
| `themes/<name>.json` | A theme in Herdr app's format; `arctic` is built in |
| `programs.json` | Program names, icons and colors over the built-in table |
| `icons/`, `program-icons/` | Icon files `programs.json` names, by path relative to the home |
| `fonts/` | `.ttf`, `.otf` and `.ttc` files the font settings can name |

## Live reload

A thread looks at these files twice a second (`config::Watcher`). When one changes, it reads them
all again and the window applies the result at once: a new font, size or line height re-measures
the cell and tells the core (FACTS, then RESIZE for the pane in use); a new theme recolors the
panels and every terminal on screen; a new zoom lays the window out again. A file that can't be
read keeps what the window had and says why in `herdr-frontend status`'s `errors`.

Each look is compared with the files as they were just before the last load (at start, just
before the first one), so a file written while a load runs is loaded on the next look: `dot stow`
writing `settings.json` and then the theme it names.

This is herdr-frontend's answer to "dotfiles-managed preferences while running": `dot stow` (or
anyone) can write or link `settings.json`, and the running window follows. Herdr app needed
`navigator-status.set-shared` sent to the running app instead.

## `settings.json`

The keys are Herdr app's (`~/.config/herdr-app/settings.json`), so a line copied from one works in
the other. Every key is optional; a missing one takes the default, which is Eli's current value.

| Key | Default | What it does |
| --- | --- | --- |
| `zoom` | `1.1` | Scales the whole window: terminals, panels and the layout's sizes (0.5 to 3). Rune's page zoom (`Cx::zoom`): the view lays out in `1 / zoom` as many points at `zoom` times the display scale, so cells stay whole device pixels. |
| `font` | `"Operator Mono Book Herdr"` | The terminals' font family. |
| `font_size` | `17` | The terminals' size in points at zoom 1 (6 to 72). |
| `ui_font` | `"Operator Mono Book Herdr"` | The panels' font family. Their text is 13 pt at zoom 1. |
| `terminal_line_height` | `1` | Row height multiplier, clamped to 0.8..2 (Ghostty's `adjust-cell-height` as a factor; text stays centered). |
| `terminal_cell_width` | `1` | Column width multiplier, clamped to 0.8..2 (Ghostty's `adjust-cell-width`). |
| `terminal_scroll_speed` | `1` | Wheel and trackpad multiplier, clamped to 0.1..5. Read once terminal scrolling lands. |
| `minimum_contrast` | `1` | 1 to 21. Text below this contrast ratio with its background is drawn white or black, whichever contrasts more (Ghostty's `minimum-contrast`). Box drawing, block elements, legacy computing and Powerline keep their colors. 1 leaves program colors alone. |
| `pane_scrollbars` | `false` | A scroll thumb at a pane's right edge. herdr-frontend draws none either way until scrollback lands. |
| `appearance` | `"dark"` | `"dark"`, `"light"` or `"system"`: picks Rune's token set for shadows. |
| `theme` | `"arctic"` | `arctic`, or the name of a file in `themes/`. |
| `colors` | `{}` | Theme colors by name over the theme's own (`{"accent": "#ff8800"}`), `#rrggbb` or `#rrggbbaa`. Terminal colors too (`background`, `terminal_foreground`, `cursor`, `selection`, `ansi0`…`ansi15`). |

A value out of range takes its default (or is clamped, where noted) and is reported; a `colors`
value that isn't a color is skipped and reported, and the rest apply. Keys
herdr-frontend doesn't read are ignored: `sounds` and `telemetry` (it has neither, ever),
`font_ligatures` (terminals are drawn without ligatures), `core`, `worktrees`, `option_as_alt`,
`clipboard_write` and `copy_on_select` (the input and clipboard tickets read their own).

### Zoom keys

`cmd-=` / `cmd-+` zoom in a tenth, `cmd--` out, `cmd-0` back to 1, as Herdr app's do. They reach
the window as `AppCommand::ZoomIn`, `ZoomOut` and `ZoomReset` effects of `model::step`
(`State::zoom_by`). The new zoom is written to `settings.json` with the file's other keys kept
(keys come back sorted). A symlinked `settings.json` is written through to its target. A file
that isn't a JSON object is never overwritten: the zoom then holds until the window closes.

## Themes

A theme file is Herdr app's format:

```json
{ "background": "#1e1e1e", "foreground": "#D4D4D4", "selection": "#04395E", "cursor": "#D4D4D4",
  "ansi": ["#1e1e1e", "…16 colors…"],
  "colors": { "canvas": "#1e1e1e", "raised": "#2A2D2E", "working": "#3794FF" } }
```

The panel colors not in `colors` are derived from the palette as Herdr app derives them
(`config::theme::resolve`, a port of `app:src/settings.rs` `resolve`). The window uses three
views of the result:

- **Terminals** (`Theme::terminal`, `Theme::look`): background, foreground, cursor, the 16 ANSI
  colors and the selection, set on every terminal worker and reported to the core in FACTS.
- **Panels** (`Theme::chrome`, the `Chrome` struct): `canvas`, `chrome`, `surface`, `popover`,
  `raised`, `hover`, `selected`, `border`, `input`, `foreground`, `muted`, `dim`, `accent`,
  `accent_foreground`, `ring`, `working`, `blocked`, `done`, `idle`.
- **Rune's tokens** (`Theme::tokens`, a `rune::tok::Theme`): `bg` is canvas, `card` surface,
  `text` foreground, `text_secondary` muted, `separator` border, `hover` and `press` hover and
  selected, for what Rune draws with its own tokens.

`assets/themes/arctic.json` is Eli's arctic. A home's `themes/arctic.json` replaces it.

## Fonts

A font setting names a family. herdr-frontend looks for its files in the home's `fonts/`, then in
Herdr app's (`$HERDR_APP_HOME/fonts`, else `~/.config/herdr-app/fonts`), which is where
`dot/scripts/prepare-herdr-fonts.py` puts Eli's Operator Mono Book Herdr. Herdr app's folder is
only read. Fonts installed on the system aren't searched: a terminal font needs its file for
Ghostty's cell metrics. A family found nowhere falls back to the bundled JetBrains Mono, with an
error in `status`.

The regular face, which the cell is measured from, is the one CSS font matching picks for weight
400 upright: Book (500) in Eli's prepared family. Bold cells ask for 700.

A terminal font other than JetBrains Mono keeps Ghostty's fallback order: its own faces, then the
JetBrains Mono Ghostty embeds as its backup (`ghostty:src/font/SharedGridSet.zig:260-316`), then
what the system finds. The text style names `"<font>", "JetBrains Mono"`, so Rune tries them in
that order before its own fallback, and the glyph atlas (cells the font lacks) tries JetBrains
Mono, resized by its ideograph width as Ghostty resizes a fallback, before the Symbols Nerd Font,
Apple Color Emoji and CoreText's discovery.

## Programs

The tab strip draws each tab's program icon: its first agent's, else its first pane's. A program
comes from the terminal's status `app` (an agent's OSC 7501 name), else its foreground argv, else
the argv it started with, resolved through the table as Herdr app resolves it
(`config::programs`, a port of `app:src/programs.rs`). One-color glyphs (`tint`) are drawn in the
program's `color`, else the theme's accent; pictures keep their colors.

The table is Herdr app's built-in one (`assets/programs/herdr.json`, its icons in
`assets/icons/`), then Eli's `programs.json` (Pi `#a78bfa`; Neovim's full-color logo
`program-icons/neovim.svg`), then the home's `programs.json`, entry by entry. An icon path is
relative to the home: a file there wins, else the bundled one by the same path.
`assets/programs/README.md` has where each file comes from and its license.

## `status`

`State::window_status` is what only the window knows for `herdr-frontend status`: `zoom`,
`font_size`, `ui_font_size` (13) and `errors` from the files. The control socket's frame-loop
wiring (ticket 036) passes it to `Control::drain`.
