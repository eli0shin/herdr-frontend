# Keys: the keymap, the key file and the keys help

herdr-frontend has one keymap and one mode machine (`docs/plan.md`). The keymap is data in
`crates/model` (`keys.rs`), and the model's `Mode` holds the sequence, popup or copy mode in
progress. The app turns each Rune key event into a press and steps the model with it.

## The key file

`keys.json` sits in herdr-frontend's home (`$HERDR_FRONTEND_HOME`, else
`$XDG_CONFIG_HOME/herdr-frontend`, else `~/.config/herdr-frontend`). It uses the format of Eli's
Herdr app `keys.json`, and the default file is that file verbatim (`crates/model/src/keys.json`,
the table of `research/feature-inventory.md` §22).

```json
{
  "global":        { "cmd-t": "tab.new", "ctrl-g s": "mod.open:space-picker" },
  "layout:eli":    { "ctrl-h": "navigator-status.left" },
  "terminal":      { "alt-left": "send:\u001bb", "ctrl-h": "terminal" },
  "popup":         { "ctrl-j": "popup.down" },
  "mod:copy-mode": { "escape": "copy-mode.escape" }
}
```

- **Sections** are key contexts. `global` holds the base bindings. `layout:eli` wins over
  `global`. `terminal` applies while a terminal pane has the keyboard and wins over both. `popup`
  applies while a popup or dialog is open, and is read before the popup's own keys. `copy` (or
  Eli's `mod:copy-mode`) applies in copy mode, before copy mode's own keys. The app skips other
  `layout:<name>` and `mod:<name>` sections, which belong to layouts and mods herdr-frontend
  doesn't have. Any other section name is an error.
- **Keys** use Eli's syntax: modifiers `cmd`, `ctrl`, `alt` and `shift` joined to the key by
  `-`, and a sequence separated by spaces (`ctrl-g shift-q`: press Ctrl+G, release, then
  Shift+Q). A shifted key matches when written either way: `ctrl-g shift-1` and `ctrl-g !`, or
  `cmd-shift-]` and `cmd-}`. Popups and copy mode take single keys only.
- **Actions** are command names (`model::Action::parse`, including Herdr app's names such as
  `navigator-status.tab-next` and `mod.open:picker`) or `send:<text>`. The action `"terminal"`
  passes the key to the program over a less specific binding. It takes a single key, outside
  popups and copy mode.
- The file is the whole keymap, as Herdr app's is: a key it leaves out is unbound.
- `ctrl-alt-shift-escape` is built in and can't be bound. It works in every mode and sets the
  file aside as `keys.json.reset-<seconds>`, then writes the defaults in its place.

### Live reload

A background thread (`crates/app/src/keyfile.rs`) reads the file every 500 ms and parses each
new text off the UI thread. A good file replaces the keymap at once. A bad file keeps the last
good keys. Its problems, every one with its section and key, go to an error toast and to
`keys.error` in the home, which is removed once the file is good again. A text that doesn't
parse counts as bad only after it stays the same for another look, so the app doesn't report a
half-saved file. At start, a home without a key file gets the defaults written to it.

## How a key travels

1. `chord::press` names the key by what it types with no modifiers (`Tx::key_base`). When Shift
   changed the character, the press carries that spelling too. A headless key with no base
   counts a shifted US symbol as its key.
2. While something has focus, the window root's `on_key_preview` (`State::preview_key`) steps
   the model with the press, before the focused pane sees the key. The terminal context applies
   when the focused element is a pane.
3. While nothing has focus, previews don't run. `AppSpec::on_key` (`State::on_key`) steps the
   model instead, and types an unused key into the model's focused pane. Keys a focused element
   passed on reach `AppSpec::on_key` with `tx.focused()` set and go nowhere, so no key is
   matched twice.
4. When the model uses a key (`Effects::handled`), nothing else gets it. Otherwise the focused
   pane's `on_key` types it.

## Modes

- **Prefix.** A key that starts longer bindings waits up to 1 s for the next one. The tab strip's
  right end, before the machine label, shows the keys so far (`ctrl-g …`) until the sequence
  runs, is dropped (an unbound sequence is swallowed whole) or times out.
- **Popup.** The file's `popup` keys come first, then the popup's own keys (§22 "Keys inside
  Eli's popups"), then one-key global bindings. `tab.close` and `pane.close` close the popup
  instead. Lists read ↑/↓, Ctrl+P/N, Page Up/Down (ten rows, `popup.page_up` and
  `popup.page_down`), Enter and Esc; the picker also Ctrl+D. A context menu (the tab menu, the
  terminal menu) reads ↑ and ↓ (wrapping), Enter and Esc, and holds every other key; prompts and
  confirm dialogs read Enter and Esc. What each popup lists is `docs/popups.md`.
- **Copy** (`ctrl-g [`). The file's `copy` keys come first, then Cmd+C (copy and leave), then
  copy mode's Vim grammar (`crates/copy-mode`, `model::mode::vim_key`), then one-key global
  bindings for keys the grammar doesn't read (Cmd and Alt chords, Ctrl-H/J/K/L). `tab.close` and
  `pane.close` leave copy mode instead. A key the file binds to `copy_mode.<name>` stands for the
  keys that name (`copy_mode.top` is `g g`, `copy_mode.yank` is Enter). Copy mode holds the
  keyboard: no key reaches the program. The keys:

  | Keys | What |
  | --- | --- |
  | `h` `j` `k` `l`, arrows, Backspace, Ctrl-N/P, with a count (`5j`) | Cells and rows; `j`/`k` keep the column |
  | `w` `b` `e` `ge`, `W` `B` `E` `gE` | Words (Vim's character classes), across soft wraps |
  | `0` / Home, `^`, `$` / End, `\|`, `+`, `-` | The row's start, first text, last text; a column; the next or previous row's first text |
  | `gg`, `G`, `5G`, `50%` | The top, the last row with text, row 5, halfway down |
  | `H` `M` `L` | The view's top, middle and bottom |
  | `f` `F` `t` `T` and a character, `;` `,` | Along the line, and again |
  | `%`, `{` `}` | The matching bracket; the previous or next empty line |
  | Ctrl-U/D, Ctrl-B/F or Page Up/Down, Ctrl-Y/E | Half a view, a view, a row: the view and the cursor |
  | `/` `?` and a query, Enter (`3/foo`: the third match); `n` `N`; `*` `#` | Search down or up (literal, ASCII letters either case), again, the word under the cursor |
  | `m` and a letter; `'` or `` ` `` and a letter; `''` | Set a mark; go to its row or its cell; back before the last jump |
  | `v` or Space, `V`, Ctrl-V; `o` | Select characters, lines, a block (the same key again drops it); the other end |
  | `iw` `aw` `iW` `aW` `ip` `ap` `i"` `a'` `` i` `` `i(` `a[` `i{` `a<` `ib` `aB` | Text objects, in a selection or after `y`; again in a selection, the next one grows it (`viwiw`, `va(a(`) |
  | `y`, Enter, Cmd+C | Copy the selection and leave (Enter with nothing selected just leaves) |
  | `yy` / `Y`, `y` and a motion or object (`yw`, `y$`, `yiw`, `y/foo`) | Copy rows, or what the motion or object covers, and leave |
  | A yank that finds nothing (`y/nowhere`, `yiw` past a line's text) | Stays in copy mode and says why in the bar |
  | Esc | Cancel a key part-way through, else drop the selection, else leave |
  | `q`, Ctrl-C, a press beside the bar | Leave without copying |

  The bar (420 × 32 at the top) says COPY, or VISUAL, VISUAL LINE or VISUAL BLOCK, then the hints,
  the `/` prompt or what the last command said, the keys typed so far, and the cursor's row of
  all the rows. How it runs on the terminal's buffer is `research/terminal-rendering.md` §6.
  Scene: `shell-copy-mode`.

## Not yet

Every action goes through `model::step`. An action whose feature isn't built yet shows a
"Not yet" toast that names the command. A popup the window doesn't draw yet closes at once with
that toast, so it can't hold the keyboard unseen. The window draws the keys help, the palette,
the pickers (the new-space folder picker and the PR finder too), the prompt and confirm
dialogs, the menus, the settings dialog (`docs/popups.md`), the sidebar's sort menu, the
agent-jump bar (`docs/sidebar.md`), the machine popups (`docs/machines.md`) and copy mode's bar
(above). The terminal's sends, paste, copy, paste-image, select all and copy mode run in the
terminal that has the keyboard (`Model::keyboard_terminal`): the peeked one while a peek is open
(`docs/review-and-peek.md`), else the focused pane's (`docs/research/terminal-rendering.md` §3).
Find (`cmd-f`, the terminal's find bar, §2 "What herdr-frontend does (ticket 019)") is the one
exception: it runs in the focused pane's terminal, and in the peek it says "not yet".

## Keys help

`cmd-/` (or `cmd-shift-/`) opens a 640 × 520 dialog. `model::help::rows` builds its rows from the
effective keymap. A `global` key that `layout:eli` rebinds shows only under its new action. Rows
are grouped spaces, tabs, panes, agents, terminal and window, then popups and copy mode. Keys
from the `terminal`, `popup` and `copy` sections carry their section's name. Typing filters by
substring over titles, command names and keys. Esc, `cmd-/` again or a click outside closes the
dialog. Scenes: `shell-keys-help`, `shell-prefix`, `shell-not-yet`.
