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
- **Copy.** The file's `copy` keys come first, then copy mode's own keys, then one-key global
  bindings.

## Not yet

Every action goes through `model::step`. An action whose feature isn't built yet shows a
"Not yet" toast that names the command. A popup or copy mode the window doesn't draw yet closes
at once with that toast, so it can't hold the keyboard unseen. The window draws the keys help,
the palette, the pickers, the prompt and confirm dialogs, the menus, the settings dialog
(`docs/popups.md`) and the sidebar's sort menu; the new-space folder picker, the PR finder, the
machine filter and the agent-jump bar still say "not yet". The terminal's sends, paste, copy and paste-image run
in the focused terminal (`docs/research/terminal-rendering.md` §3); find and select all say "not
yet" until ticket 019.

## Keys help

`cmd-/` (or `cmd-shift-/`) opens a 640 × 520 dialog. `model::help::rows` builds its rows from the
effective keymap. A `global` key that `layout:eli` rebinds shows only under its new action. Rows
are grouped spaces, tabs, panes, agents, terminal and window, then popups and copy mode. Keys
from the `terminal`, `popup` and `copy` sections carry their section's name. Typing filters by
substring over titles, command names and keys. Esc, `cmd-/` again or a click outside closes the
dialog. Scenes: `shell-keys-help`, `shell-prefix`, `shell-not-yet`.
