# Eli's Herdr app feature inventory (the parity checklist)

herdr-frontend reaches parity when Eli can do everything below without opening Herdr app. Eli's
rule: everything he uses is customized, so his customized config is the spec. This file lists
that config feature by feature, says what each feature depends on, and records the limits his
mods work around, since those limits are why this project exists.

## Sources and versions

Every claim cites one of these sources. Short prefixes:

| Prefix | Path | What it is |
|---|---|---|
| `cfg/` | `~/.dotfiles/home/.config/herdr-app/` (dotfiles `640dab20`, 2026-11-23) | Eli's Herdr app home (`~/.config/herdr-app` is a symlink to it) |
| `cfg/README.md` | | Eli's own description of his setup and why |
| `cfg/AGENTS.md`, `cfg/herdr.d.ts` | | The app's mod guide and mod API, written by the app on start (0.1.18) |
| `app/` | `~/code/herdr-app` at `origin/master` `321c9a8` (read only) | Herdr app source, for built-in behavior |
| `dot/` | `~/.dotfiles/` | Hooks and tools that integrate with herdr-core and the app |

Installed versions: Herdr app 0.1.18 (`herdr-app --version`, `cfg/update-notes.json`), herdr-core
`hc` 0.1.22. The local herdr-app checkout is older than the installed app, so the installed files
in `cfg/` and `herdr-app commands` (0.1.18) are the authority for commands and shipped mods.
`app/` is used only for app internals the mod files don't show (alerts, overlay sizes, toast
timeouts).

`cfg/.gitignore` decides what is Eli's: it tracks `keys.json`, `settings.json`, `programs.json`,
`program-icons/neovim.svg`, `shared.defaults.json`, `layouts/eli.json` (+ `eli.svg`),
`themes/arctic.json` and these 12 mods. `cfg/installed.json` lists the 24 shipped, unmodified
mods and layouts.

| Eli's mod | Cloned from | Role |
|---|---|---|
| `eli-chrome` | `workspace-chrome` 0.1.13 | Sidebar top row / Agents header |
| `eli-agents` | `agents` 0.1.13 | Agents list (the sidebar) |
| `eli-tabs` | `tabs` 0.1.13 | Tab bar |
| `eli-toast` | `toast` 0.1.13 | Notifications |
| `navigator-status` | `workspace-status` 0.1.13 | Invisible key-command host: pane/Neovim navigation, tab keys, numbered agents, review, jump list, dead-space sweep, stow preferences |
| `agent-jump` | new | Ctrl+; jump bar |
| `agent-machines` | new | Machine filter popup |
| `space-picker` | new | Ctrl+G S space switcher |
| `pr-finder` | new | Ctrl+G F "which agent owns this PR" |
| `new-space` | new | Cmd+N folder picker for new spaces |
| `copy-mode` | new | Ctrl+G [ keyboard copy mode |
| `nvim-links` | new | Cmd-click file paths open in a Neovim split |

## How to read the checklist

Each item is one checkable behavior. Tags:

- **[Eli]**: Eli's own mod, layout, key or setting implements it, or he changed it.
- **[Shipped, used]**: a built-in or unmodified shipped mod that Eli's layout or keys reach.
- **[Shipped, unused]**: shipped, but Eli's layout and keys don't reach it (listed in
  [Shipped but unused](#20-shipped-but-unused) so it isn't mistaken for a requirement).

"Depends on" names the app API a mod calls and the herdr-core concept underneath it: terminals,
labels, OSC 7501 agent status, `exec --on` for other machines, and the per-machine `herdr-app`
layout document. herdr-frontend talks to herdr-core directly, so the core concept is what it must
reproduce. Keys are written as in `keys.json`: `ctrl-g s` means press Ctrl+G, release, then S.

"The TUI" means legacy Herdr, the terminal program Eli used before Herdr app and whose keys
his setup carries over. "The review TUI" is Eli's `review` program, which runs in a terminal
pane. Neither describes herdr-frontend. Like Herdr app, herdr-frontend is a native desktop app on
[Rune](https://github.com/anomalyco/rune): sidebars, tab bars, popups, the palette and toasts are
native elements drawn in its window, and terminal panes are GPU-rendered views inside it.

---

## 0. Interop contract (must not break)

herdr-frontend runs on the same core as a live Herdr app (`AGENTS.md`). These are the shared
things Eli's setup reads or writes. Breaking any of them breaks Herdr app, his mods or his
tools.

- [ ] **Layout document.** Each machine's core keeps spaces, tabs and splits in a `herdr-app`
  document. Newest accepted write wins, with no merging. Focus, zoom, settled tabs and
  seen-attention are per screen, not in the document (`cfg/AGENTS.md` "The model"). Mod pane
  props are stored as given.
- [ ] **Terminal adoption labels.** `herdr.space=<id>` and `herdr.tab=<id>` place a terminal in a
  space/tab. Remote ids carry a `<machine>/` prefix in the app, but labels on that machine use
  bare ids (`cfg/AGENTS.md`). `spawn-worker` and `agent-status` start terminals with
  `hc run --bg --keep --no-parent --label herdr.space=… --label herdr.tab=…`
  (`~/.agents/skills/orchestrator/scripts/spawn-worker:311`, `~/.agents/skills/agent-status/ACTIONS.md:32`).
  `repos` uses `space.new {focus: false}` instead (§0 CLI surface).
  - An id the app doesn't know becomes a new space or tab, unnamed so Herdr names it
    (`cfg/AGENTS.md:154`), and it shows up without taking focus. `spawn-worker` creates
    worker spaces and tabs this way with fresh `sp…`/`tb…` ids, because `space.new` and
    `tab.new` always focused what they opened (herdr-app#447;
    `~/.agents/skills/orchestrator/scripts/spawn-worker:292-295`). `agent-status` resume adds
    a tab to an existing space the same way (`~/.agents/skills/agent-status/ACTIONS.md:32`).
    herdr-frontend must adopt such terminals the same way: create the unknown space or tab and
    leave focus where it is.
- [ ] **App labels on terminals.** `herdr.pinned` (`terminal.pin`), `herdr.right-click=program`
  (`terminal.right_click`) and `herdr.seen` (done → idle shared across clients; seen on a live
  terminal with `hc get t4 --json`).
- [ ] **Eli's labels.**
  - `dotfiles.navigator.editor=<v:servername>` and `dotfiles.navigator=<seq>:<seq>:<dir>`: the
    Neovim bridge (`cfg/mods/navigator-status/navigation.js:4-8`,
    `dot/home/.config/nvim/lua/herdr-app-navigator.lua`).
  - `dotfiles.review=1`: the review terminal (`cfg/mods/navigator-status/review-view.js:13-18`).
  - `dotfiles.pi_pr_watch=<json>`: a Pi session's role, orchestration, ticket, PRs, refs, sha and
    paused state (`cfg/mods/eli-agents/watch.js`, `dot/home/.pi/agent/extensions/README.md:15`).
  - `dotfiles.pi_session` and `dotfiles.pi_shell`: Pi resume after a reboot
    (`dot/home/.config/fish/conf.d/hc-pi-resume.fish`).
- [ ] **App-local shared state** (`cfg/shared.json`, per machine, not in core): `layout`,
  `workspace.panels`, `frame.welcomed`, `space.colors`, `sidebar.width`, `sidebar.collapsed`,
  `agents.sort`, `dotfiles.agents.order`, `dotfiles.agents.jumping` and
  `dotfiles.agents.hidden-machines`. `space.colors` is shared with the shipped picker's color
  algorithm (`cfg/mods/eli-agents/colors.js`).
- [ ] **CLI surface other tools drive** (`herdr-app ls | do | status`). herdr-frontend either
  serves these or Herdr app must stay running for them.
  - `repos`: `ls`, `space.new {focus:false}`, `space.select`, `space.rename`, `space.close`
    (`~/code/repos/src/workspace-manager/herdr-app-adapter.ts:162-503`).
  - `spawn-worker`: `ls`, `space.rename`.
  - `agent-status` skill: `pane.close`, `agent.focus`, `tab.rename`.
  - Pi `auto-session-name.ts`: `tab.rename` from a tab's first Pi pane.
  - `dot stow` (`dot/scripts/configure-herdr-app.sh`): `status`, `layout.switch`,
    `navigator-status.set-shared:<json>`.

## 1. Window chrome and layout regions

The `eli` layout (`cfg/layouts/eli.json`, a clone of `main` 0.1.13), outside in:

```
root row  (background canvas, terminal_padding [0,0], titlebar none, traffic_lights [-80,4])
├─ sidebar column  400px (min 200, max 600), hidden by default, background chrome
│  ├─ brand      38px  eli-chrome   window_drag
│  └─ sidebar-lists column
│     └─ agents  weight 1, min 90   eli-agents
└─ work column
   ├─ tabs       38px  eli-tabs     window_drag
   ├─ work-body row (padding 0, gap 0)
   │  ├─ main    tab: active, pane {background "background", radius 0, gap 1}; block background "border"
   │  └─ panel column 368px (min 280, max 900), hidden
   │     └─ panel-body stack [files, changes], active files
   └─ status     0px   navigator-status   (draws nothing)
link_handlers: [nvim-links]    toast: eli-toast    preview: layouts/eli.svg
```

- [ ] **[Eli] No system title bar.** `"titlebar": "none"`. The tabs row and the sidebar's top row
  drag the window, and double-clicking them maximizes (`window_drag`, `cfg/AGENTS.md`).
  Controls inside a drag row stop the press (`cfg/mods/eli-tabs/main.js:63-65`,
  `cfg/mods/eli-chrome/main.js:40-42`).
- [ ] **[Eli] No macOS traffic lights.** Herdr has no setting to hide them, so
  `"traffic_lights": [-80, 4]` parks them off the window's left edge. Close and quit with keys
  (`cfg/README.md` "Herdr app configuration"). The tab bar's traffic-light inset then falls to
  its 10px minimum (`cfg/mods/eli-tabs/main.js:88-92`).
- [ ] **[Eli] Bare panes.** No pane headers, no frames and radius 0. A 1px gap shows the tab
  block's `border` background, so the only line in the window is between split panes. A lone
  pane and a split's outer edges are bare. The focused pane has no outline; its cursor shows
  focus (`cfg/README.md`).
- [ ] **[Eli] No status bar row.** `navigator-status` sits at 0px height: only shown mods get
  commands, so it must be in the layout but draws nothing. Terminals reach the window's bottom
  (`cfg/mods/navigator-status/main.js:51-60`).
- [ ] **[Eli] Sidebar that lists agents only.** Spaces live in the Ctrl+G S picker. It is
  toggled with `cmd-b` / `ctrl-g b` (`layout.toggle:sidebar`). Its rows have no dividers. The
  layout file hides it by default, but the saved clicks show it now at 366px
  (`cfg/layouts/.state/eli.json`). The boundary to the work column can be dragged between 200
  and 600px.
- [ ] **[Eli] 38px rows with matching padding.** The sidebar's top row and the tab bar are both
  38px: 6px above the 29px tabs and 3px below, so the "Agents" label lines up with the tabs
  (`cfg/mods/eli-chrome/main.js:34-35`, `cfg/mods/eli-tabs/main.js:106-108`).
- [ ] **[Shipped, used] Window state.** Size, position and display persist (`cfg/window.json`).
  `window.close` (`ctrl-g d`) closes the window while core terminals keep running (legacy
  detach, `cfg/README.md` Keys).
- [ ] **[Shipped, used] Terminal grid centering.** Herdr centers the grid in its pane, so the
  leftover part of a row splits above and below (0 to half a row). Eli notes that no layout or
  setting pins it to the top (`cfg/README.md`). Parity means matching this, or fixing it, which
  he would want.
- [ ] **[Shipped, used] Escape hatch.** `ctrl-alt-shift-escape` sets the layout and keys aside and
  returns to `main`. It can't be rebound (`cfg/AGENTS.md`).
- [ ] **[Shipped, used] Layout switch.** `cmd-shift-l` cycles layouts (`layout.switch`). Eli stays
  on `eli`; `dot stow` selects it.

## 2. Look: theme, fonts, settings

- [ ] **[Eli] Arctic theme** (`cfg/themes/arctic.json`). The terminal background `#1e1e1e`,
  foreground `#D4D4D4`, selection `#04395E` and the 16 ANSI colors match Ghostty and Neovim.
  The UI backgrounds (canvas, chrome, surface, popover) all equal the terminal's `#1e1e1e`.
  Raised is `#2A2D2E`, hover `#2D2D2D`, selected `#04395E`, border `#858585`, input `#A9A9A9`,
  muted/dim `#8F8F8F`, accent `#858585`. Status colors: working `#3794FF`, blocked `#CCA700`,
  done `#89D185`. The layout sets no theme, so `settings.json`'s `"theme": "arctic"` applies.
- [ ] **[Eli] Settings** (`cfg/settings.json`): `zoom` 1.1, sounds off, terminal and UI font
  `Operator Mono Book Herdr` at `font_size` 17 (the README's "16px" is stale; commit
  `319e2276` set 17), ligatures off, line height, cell width and scroll speed 1.0,
  `minimum_contrast` 1.0 (program colors untouched), `pane_scrollbars` false, `appearance`
  dark, `telemetry` false.
  - Left at defaults: `copy_on_select` true, `clipboard_write` true (OSC 52) and
    `option_as_alt` auto. Alt+Shift+H/J/K/L and the Alt word-motion keys rely on `option_as_alt`
    (`cfg/README.md` Keys).
- [ ] **[Eli] Fonts with corrected weights.** Herdr 0.1.13 chose families only and hardcoded
  Medium (500). The original Operator Mono labels Book 325, Medium 350 and Bold 400, so normal
  text came out Bold. `dot/scripts/prepare-herdr-fonts.py` builds an isolated family (Book=500,
  Bold=700, unchanged outlines) into `cfg/fonts/`, which Herdr loads itself. Bold and Bold
  Italic are used only when a program asks for them (`cfg/README.md`).
  - 0.1.18 has `font_weight` and `font_weight_bold` settings (`cfg/AGENTS.md`), but Eli still
    relies on the prepared family.
  - herdr-frontend doesn't carry this over (ticket 039): it draws the installed Operator Mono's
    Book, Bold and their italics, each style its own face, as Ghostty does
    (`docs/settings.md` "Fonts").
- [ ] **[Eli] Program look** (`cfg/programs.json`): Pi is purple `#a78bfa`. Neovim uses the
  full-color logo `program-icons/neovim.svg` (`tint: false`). The tab bar draws one-color
  glyphs in their program's color instead of gray (§4).
- [ ] **[Shipped, used] Zoom keys.** `cmd-=` / `cmd-+` zoom in, `cmd--` out, `cmd-0` reset. Zoom
  scales terminals, mods and layout px.
- [ ] **[Shipped, used] Settings window.** `cmd-,` / `ctrl-g c` (`app.settings`) opens an 880px
  dialog with sections Look (layout picker with previews, appearance, theme, terminal and
  interface fonts, size, weight, line height), Keyboard (macOS `option_as_alt`), Machines
  (list, rename, remove, Add a machine), Alerts (sounds), Updates and Privacy (telemetry). It
  edits the same files (`cfg/frame/settings/main.js:9-16`; `app/src/window.rs` `app.settings`).
- [ ] **[Eli] Preferences applied without a restart.** The app reads `shared.json` once and
  rewrites it from memory, so `dot stow` sends `layout.switch` and
  `navigator-status.set-shared:<json>` to the running app instead of editing the file
  (`cfg/mods/navigator-status/preferences.js`, `dot/scripts/configure-herdr-app.sh`).
  herdr-frontend needs its own answer to "dotfiles-managed preferences while running".

## 3. Spaces

- [ ] **[Shipped, used] Space model.** A space lives on one machine. An unnamed space is called
  after its first terminal's repo or folder and follows it; a named one keeps its name; renaming
  to "" goes back. Spaces are ordered local machine first, then machines by name. Closing the
  last tab closes the space (`cfg/AGENTS.md` "The model").
- [ ] **[Eli] Space picker** (`ctrl-g s`, `mod.open:space-picker`). A popup 520 wide, 0.6 high,
  40px from the top (`cfg/mods/space-picker/mod.json`), titled "Spaces" with a "Go to space…"
  field.
  - Rows: space number (1–9 only), status glyph (●, ◐ for working, ○ idle, ·), name (semibold
    when shown), machine when remote, "N tabs".
  - 12 rows visible with "↑ N more" / "↓ N more", and the window follows the selection.
  - Opens with the shown space selected; typing selects the best match. ↑/↓ or Ctrl+N/P move,
    Enter goes, Esc closes (`cfg/mods/space-picker/main.js`).
  - Matching (`cfg/mods/space-picker/match.js`), fzf-style: every term must match. The name
    counts above branch, folder (`~` for home) and machine. A whole word beats scattered letters
    and a word start beats mid-word; shorter names win; ties keep space order.
  - Edge: if the chosen space's selected tab belongs to an agent or the review, it shows the
    space's first terminal tab instead. Ctrl+1–9 don't do this (`match.js:71-79`, `main.js:51-59`).
- [ ] **[Shipped, used] Select by number.** `ctrl-1`…`ctrl-9` (`space.select:N`) in space order,
  the same numbers the picker shows.
- [ ] **[Shipped, used] Next/previous space.** `cmd-alt-]` / `cmd-alt-[`.
- [ ] **[Shipped, used] Rename space.** `shift-f2` (`space.rename`) opens the prompt dialog (§10).
- [ ] **[Shipped, used] Close space.** `ctrl-g x` (`space.close`) asks first in the confirm dialog
  and ends the space's terminals.
- [ ] **[Eli] New space in a picked folder** (`cmd-n`, `ctrl-g ctrl-s`): see §11.
- [ ] **[Eli] Space colors.** Each space gets a color from `ansi6, ansi3, ansi4, ansi5, ansi2,
  ansi1`: the one fewest spaces use, assigned once and stored in shared `space.colors` by space
  id. A picked `#rrggbb` is kept as is. Home (`~`) gets none. The algorithm is copied from the
  shipped picker so they agree (`cfg/mods/picker/main.js:13-35`).
  - The names map to fixed colors, not the theme's ANSI palette: ansi6 `#22a99a`, ansi3
    `#e3814f`, ansi4 `#5b8def`, ansi5 `#8b7cf6`, ansi2 `#3fae6a`, ansi1 `#d9649a`. The picker
    draws avatars in these. The Agents list shows the space name in that color lifted 28% toward
    `#e8e8e8` (`cfg/mods/eli-agents/colors.js:9, 28-33`).
- [ ] **[Eli] Close spaces whose folder was deleted** (`cfg/mods/navigator-status/dead-spaces.js`).
  - Why: `gh pr merge --delete-branch`, `git worktree remove` and `rm -rf` remove a worktree
    without telling Herdr.
  - Every 30s, a space qualifies when it is local, not shown, has live terminals (by their
    `herdr.space` label), none of them reports a working agent, and its folder (home if none)
    and every live terminal's cwd are gone. Idle, done, blocked and unreported agents and
    foreground programs such as Neovim don't keep it (closing it ends them).
  - It must qualify on two consecutive sweeps with the same folders, so it closes 30–60s after
    the folder goes, with an info toast "Closed space <name>". A failed check closes nothing.
    Depends on `exec` (`sh -c 'test -d'`), terminals' `foreground.cwd`, their status and the
    `herdr.space` label. herdr-frontend: `docs/dead-spaces.md`.
- [ ] **[External] `repos` integration.** `repos` creates spaces with `space.new {focus:false}`,
  selects and closes them, and matches spaces by the folder they were made in
  (`~/code/repos/src/workspace-manager/herdr-app-adapter.ts`). Plain `space.new` takes the
  focused terminal's folder, and nothing can change a space's folder later (`cfg/README.md`
  "New space in a picked folder").

## 4. Tabs

- [ ] **[Eli] Tab bar** (`eli-tabs`). Tabs are shown even when there is only one.
  - Pills are 29px high, rounded 7, 12.8px text and at most 170px wide (truncated, with a
    tooltip). The active pill is raised with a border.
  - Each pill shows the program icon of the tab's agent pane, else its first pane. One-color
    glyphs are drawn in `program.color ?? accent`; pictures keep their colors.
  - Status: working shows a spinner (rotates once per 900ms), blocked or done a 6px dot. A
    zoomed tab gets a " Z" suffix. Only the active tab has a × close button.
  - The bar scrolls horizontally (`cfg/mods/eli-tabs/main.js:66-87`).
- [ ] **[Eli] Agents' tabs stay out of the bar.** Any tab with an entry in `agents()`, including
  split tabs and the active one, is hidden, as is the review tab (§13). A program that hasn't
  reported status keeps its tab until it does (`cfg/mods/eli-tabs/main.js:55-60`,
  `cfg/README.md`).
- [ ] **[Eli] Unnamed tabs are numbered among the shown tabs**, matching Cmd+1–9, not by their
  position among all tabs (`main.js:68-70`).
- [ ] **[Eli] Tab names from Pi.** An unnamed tab whose first Pi pane's title is
  `π - <session> - <folder>` shows `<session>`. Duplicate names in a space gain ` · <tab index>`
  (`.2`, `.3` if that clashes too). Split tabs use the first Pi pane, never the focused pane, so
  names don't change with focus (`cfg/mods/eli-tabs/names.js`, identical in `eli-agents` and
  `pr-finder`).
  - Pi's `auto-session-name.ts` also writes the real tab name with `herdr-app do tab.rename` on
    every name change (`dot/home/.pi/agent/extensions/README.md:20`).
- [ ] **[Eli] Mouse on tabs.**
  - Click selects (on release). Double-click renames (`tab.rename` prompt).
  - Press on one tab and release on another to move it there (`tab.move {id, index}` by id,
    since the shown set can change mid-drag).
  - Right-click opens the tab menu (§10).
  - The scroll wheel steps tabs with `navigator-status.tab-prev/next`, so hidden tabs are
    skipped (`main.js:74-81, 109-111`).
- [ ] **[Eli] Machine label at the bar's end.** A monitor icon with "This computer", or a server
  icon with the machine name, for the shown space. It replaces the new-tab button (Cmd+T,
  Ctrl+G T) (`main.js:113-117`).
- [ ] **[Eli] No Files/Changes buttons.** They are hidden by shared `workspace.panels: []`, which
  `dot stow` sets (`cfg/shared.defaults.json`, `main.js:93-119`). The per-space panel memory
  still runs and keeps the panel hidden (`syncPanel`, `main.js:24-50`).
- [ ] **[Eli] Window buttons where the OS draws none** (Linux/Windows): minimize, maximize/restore
  and close at the bar's end (`main.js:120-122`).
- [ ] **[Eli] Numbered tabs skip hidden tabs.** In the eli layout `cmd-1…9` and `ctrl-g 1…9` run
  `navigator-status.tab-N`, which counts only shown tabs. The app's `tab.select:N` would count
  agents' tabs (`cfg/mods/navigator-status/tab-order.js`).
- [ ] **[Eli] Next/previous tab skips hidden and settled tabs, wrapping**: `ctrl-tab`,
  `ctrl-shift-tab`, `cmd-shift-]`, `cmd-shift-[`, `ctrl-g n`, `ctrl-g p`.
  - On an agent's tab they move down/up the Agents list order instead (wrapping, across spaces)
    and never leave it for a tab.
  - An agent the order leaves out starts from outside: next goes to the top row, prev to the
    bottom (`tab-order.js:37-47`).
- [ ] **[Eli] Focus after closing a tab** (`cfg/mods/navigator-status/tab-close-focus.js`).
  - Closing the shown tab shows the nearest shown, unsettled tab to its left, else to its right,
    as the TUI did. The app picks the right one.
  - Closing an agent's tab focuses the next agent in the Agents list order (wrapping, across
    spaces), falling back to a shown tab only when no other agent is left.
  - Only for the shown space; a background close keeps the app's choice.
- [ ] **[Eli] Jump list of tabs** (`ctrl-g o` back, `ctrl-g i` forward;
  `cfg/mods/navigator-status/jump-list.js`).
  - A tab joins once it has been on screen for 1s, however it got there. Pane moves inside a tab
    don't count, and returning shows the pane you left focused.
  - Like Neovim's default (not `jumpoptions=stack`): a tab is in the list once, at its last
    position, and going somewhere new after jumping back drops nothing.
  - Closed tabs are skipped. The list holds 100 entries, lives in memory and starts empty.
- [ ] **[Shipped, used] Tab commands.** `cmd-t` / `ctrl-g t` new tab (in the focused terminal's
  folder). `cmd-w` / `ctrl-g shift-q` close tab; closing the last tab asks, since it closes the
  space. `f2` / `ctrl-g r` rename (prompt).

## 5. Panes, splits and zoom

- [ ] **[Shipped, used] Split.** `cmd-d` / `ctrl-g v` right, `cmd-shift-d` / `ctrl-g h` down. A
  split stays on the machine of the pane it splits (`cfg/README.md` "File links").
- [ ] **[Shipped, used] Close pane.** `cmd-shift-w` / `ctrl-g q`. This ends its terminal.
- [ ] **[Shipped, used] Zoom.** `cmd-shift-enter` (`pane.zoom`). Zooming again unzooms. A
  one-pane tab doesn't zoom, and a zoom ends when the tab is back to one pane. The tab pill
  shows " Z".
- [ ] **[Shipped, used] Focus by direction.** `cmd-alt-arrows` (`pane.go`) prefers the most
  recently focused pane along the nearest edge (`cfg/AGENTS.md` Commands).
- [ ] **[Eli] Neovim-aware Ctrl+H/J/K/L** (`layout:eli` → `navigator-status.left/down/up/right`;
  `cfg/mods/navigator-status/navigation.js`), after vim-tmux-navigator:
  - If the focused terminal has `dotfiles.navigator.editor=<server>`, it runs
    `nvim --server <server> --remote-send <C-h>` (on that terminal's machine through
    `exec --on`, 2s timeout). Neovim treats this as a typed key and moves between its own
    splits.
  - At a window edge, Neovim writes a unique `dotfiles.navigator=<n>:<n>:<dir>` label. The mod
    consumes it once, and only while that terminal is still focused, then runs `pane.go`. Old
    requests are marked seen, so they never replay.
  - Without the label it runs `pane.go`. If the labelled Neovim is gone (nvim exits non-zero),
    it clears the label and moves the pane. If nvim can't run, it shows an error toast.
  - Never `hc keys`: core holds an agent's keys until ~3s after the person stops typing
    (`HC_QUIET_MS`), so Ctrl+L right after Alt+J in Diffview waited 3s (`cfg/README.md`, commit
    `d1ad0a13`).
  - The Neovim side, `dot/home/.config/nvim/lua/herdr-app-navigator.lua`, sets the label on
    start and resume, clears it on suspend, `:detach` and exit, and skips headless runs and tmux.
    It works under wrappers (`fish -c`, `git commit`, an agent's external editor).
  - Without the eli layout, `ctrl-h/j/k/l` are plain `pane.go` (global section).
- [ ] **[Shipped, used] Swap.** `cmd-alt-shift-arrows` (`pane.swap`). It moves panes, not focus.
- [ ] **[Shipped, used] Resize from the keyboard.** `alt-shift-h/j/k/l` (`pane.resize`, 0.1.17)
  moves the nearest divider 40px, as in the TUI.
- [ ] **[Shipped, used] Drag to rearrange.** The top-center grip on hover drags a pane within its
  tab: center drop swaps, edge drop places beside. Not in zoomed or one-pane tabs. On by
  default; eli doesn't set `movable` (`cfg/AGENTS.md`).
- [ ] **[Shipped, used] Drag dividers to resize.** The `main` tab block's splits are resizable.
- [ ] **[Shipped, used] Pane rename.** From the terminal right-click menu (`pane.rename`). Eli's
  layout shows no pane names.

## 6. Agents list (`eli-agents`)

The main way to reach agents. Agents are terminals that report OSC 7501 status: `working`,
`blocked`, `idle`, `done` (finished unseen; it becomes idle once looked at on any client)
(`cfg/AGENTS.md`).

- [ ] **[Eli] Sidebar header row** (`eli-chrome`). It shows "Agents" (11.5px, dim), the machine
  filter button (a monitor icon; "N hidden" in the working blue while any machines are hidden;
  a tooltip names Ctrl+G /) and the sort button (clock icon = priority, layers icon = grouped).
  There is no brand, search or sidebar button (`cfg/mods/eli-chrome/main.js`).
- [ ] **[Eli] Sort menu.** A popup with "Priority" and "Grouped by space" and a check on the
  current one. It sets shared `agents.sort` (default priority). It is `eli-agents` opened as a
  popup with `props.sortMenu` (`cfg/mods/eli-agents/main.js:165-189`).
- [ ] **[Eli] Sections.**
  - Pinned agents come first, under "Pinned". Pins made earlier keep their section, but rows
    have no pin button.
  - Priority view: sections Needs you, Done, Working, Idle, each header with a count, ordered by
    rank. Ties break by the time of the last state change, then the core's change sequence.
  - Grouped view: space-name headers in model order.
  - Clicking a header folds it. A folded header shows its count and, in grouped view, the most
    urgent status, plus a chevron. The first header sits right under the top row with no
    padding (`main.js:468-509`).
- [ ] **[Eli] Rank.** A parent takes its most urgent live sub-agent's rank. An orchestrator takes
  its workers': a worker needing you moves it to Needs you, and a working worker lifts an idle
  orchestrator. A done worker doesn't move it. A sub-agent's `error` counts as blocked and its
  `done` as idle (`main.js:56-62, 270-276`).
- [ ] **[Eli] A viewed done agent stays in Done until you leave it** (`hold`, `main.js:151-163`).
- [ ] **[Eli] Row layout** (two lines; picked from prototype variant H,
  `dot/docs/prototypes/herdr-agents-readability.prototype.html`).
  - Line 1: program icon (16px), title (14.5px, no bold, `#F2F2F2` when live or focused, else
    the foreground at 90%), fold control, status, jump label.
  - Line 2 (12.5px, dim):
    - Space name in its color, only without a space header above (priority view or pinned), and
      not for workers.
    - The Jira ticket for an orchestrator.
    - A "N workers" chip and a PR chip (first PR plus "+N"; the tooltip has the whole list,
      `· paused` when paused, italic while paused).
    - The machine for remote agents.
    - Idle time (`<1m`, `Nm`, `Nh`, `Nd`), colored by status for live rows.
  - Live rows (not idle) get a 2.5px status-colored left edge. The focused row gets a faint
    working-blue wash and a thin working-blue outline; every row has a border so none shift
    (`main.js:385-466`).
  - Code differs from commit `97d78ffc`'s message: titles read bright when live or focused.
- [ ] **[Eli] Status marks.** Working: a spinner rotating every 900ms. Blocked: a "Needs you"
  pill, or a compact "!" under a priority header. Done: a filled dot. Idle: a ring
  (`main.js:236-245`).
- [ ] **[Eli] Row title.**
  - A named tab's name.
  - For an unnamed tab, or one named after its program, the agent's live terminal title: leading
    non-letters are stripped, and the title is ignored if it is a path, contains `@`, or equals
    the program.
  - An automatic tab number is never shown unless two rows in a space would otherwise read the
    same; the program name stands in for it.
  - Pi session names come from `names.js` (§4) (`main.js:37-55, 346-362`).
- [ ] **[Eli] Times survive restarts.** Each agent's last state change (done and idle count as
  one) is stored in mod `localStorage` and pruned after 30 days for gone agents. A 30s timer
  refreshes the times (`main.js:99-125`).
- [ ] **[Shipped, used] Sub-agents.** Terminals started by an agent with `hc run` (core
  `parent`) nest under it with tree lines (└ ├ │).
  - Rows are 26px, 12.5px, muted unless shown, with icon and name (`name`, `template`, program
    or address) and status.
  - Clicking peeks (`terminal.peek`, §15). The icon or the chevron with a count folds them
    (`main.js:195-234, 320-344`).
- [ ] **[Eli] Pi orchestrator and worker nesting** (`cfg/mods/eli-agents/watch.js`).
  - The `dotfiles.pi_pr_watch` label is trusted only while the terminal runs Pi.
  - Workers whose orchestrator is listed nest under it with tree lines and leave out the space.
  - An orchestrator's workers show only while it or one of its workers is focused (no fold
    button).
  - A worker whose orchestrator isn't listed is an ordinary row.
  - The orchestrator's PR chip covers every PR it watches, even ones a worker also shows.
- [ ] **[Eli] Jump labels on every row.** Rows read 1–9 and 0, then home-row letters
  `asdfghjklqwertyuiopzxcvbnm`. With more than 36 rows, every row past the tenth gets two
  letters so no label begins another. Labels sit dim at the row's end and brighten (working
  blue, semibold) while agent-jump runs (`cfg/mods/eli-agents/jump.js`).
- [ ] **[Eli] Published order.**
  - The list calls `setAgentOrder(rows + all workers)`, so `agent.next/prev` follow it.
  - It writes shared `dotfiles.agents.order = {inputs, addresses}`. `inputs` is a signature of
    sort, focused agent and each agent's address, status and pin, so readers know when it's
    stale.
  - `navigator-status`, `agent-jump` and `pr-finder` read it (`main.js:282-291`).
- [ ] **[Shipped, unused] Narrow rail** (sidebar under 160px, decided by width, not a flag).
  Inherited from the stock Agents list, but unreachable in eli: the sidebar's `min` is 200, a
  divider drag clamps to `min` (`app/src/window.rs` `drag_to`), and no key or palette entry runs
  `layout.resize`. If reached, rows are 26px with the label and status only, workers show flat,
  sub-agents are left out, and a "»" button restores shared `sidebar.width` (default 268); the
  rail is 76px on macOS, 44px elsewhere (`cfg/mods/eli-agents/main.js:25-35, 305, 392-404,
  522-537`).
- [ ] **[Eli] Machine filter applied.** Agents on hidden machines leave the list, its numbers,
  agent-jump, Ctrl+G Shift+N and tab-close focus. They are still tracked, so their times hold.
  `agent.next/prev` still reach them, because the app appends unlisted agents (`cfg/README.md`
  "Filtering agents by machine").
- [ ] **[Eli] Empty states.** "No agent status yet. Agents show up here when they report their
  status…", or "Every agent is on a hidden machine. Ctrl+G, / chooses which machines show."
- [ ] **[Eli] Click a row** to focus its pane (`pane.focus {pane}`).
- [ ] **[Eli] Old local pins carried over.** Pins kept in old `localStorage` are written to
  `terminal.pin` once the terminal is live, and kept until core echoes `herdr.pinned`
  (`main.js:127-145`). This is legacy, but it shows pins are core labels.

## 7. Agent navigation and jump

- [ ] **[Shipped, used] Next/previous agent.** `cmd-]` / `cmd-[`, `ctrl-g j` / `ctrl-g k`
  (`agent.next/prev`) in the list's published order, including agents the list leaves out.
- [ ] **[Eli] Numbered agents.** `ctrl-g shift-1…9` and `shift-0` (also bound as `! @ # $ % ^ & *
  ( )`) run `navigator-status.agent-N` and focus the Nth row (`agent.focus`).
  - Why not the app's `agent.select {index}`: it counts the `setAgentOrder` order, which holds
    every orchestrator's workers even while folded and then appends agents on hidden machines.
    The row labels follow only the rows shown (`cfg/mods/eli-agents/main.js:287-289`). Mods
    can set that order but not read it back (`cfg/herdr.d.ts` `setAgentOrder`), so the list
    publishes its shown rows in shared state instead.
  - If the published order is stale (for example the sidebar hasn't been shown since launch, so
    its mod never ran), `agent-order.js` mirrors the sort: pinned first, then priority with
    parent rank, ties by `seq`, or grouped order. That fallback ignores the done hold and doesn't
    nest Pi workers (`cfg/mods/navigator-status/agent-order.js`).
- [ ] **[Eli] Agent jump** (`ctrl-;` → `mod.open:agent-jump`). A 32px bar at the top (width 360,
  offset 6) shows "JUMP", the label range ("1–9 · 0 · letters") and Esc, and holds the keyboard.
  - Pressing a full label focuses that row (`agent.focus`). The first of two letters waits for
    the second. Modified keys are ignored. Esc, Ctrl+; or a click outside leaves.
  - A hidden sidebar is shown for the jump and hidden again after.
  - It sets shared `dotfiles.agents.jumping {at}`, refreshed every 20s; rows stop brightening if
    the stamp is a minute old.
  - The overlay closes before focusing, so it can't take focus back
    (`cfg/mods/agent-jump/main.js`).
- [ ] **[Shipped, used] `agent.focus` shows sub-agents too**, used by toasts, pr-finder and jump.

## 8. Agent status, attention, notifications, toasts

- [ ] **[Shipped, used] Attention rule** (`app/src/model.rs` `alert`): turning blocked (or error)
  always alerts. Finishing (working/blocked → anything else) alerts only when the person isn't
  looking: the window is active and the agent is visible.
  - Sub-agents never alert, nor do terminals not placed in a space (`alert_for`).
- [ ] **[Shipped, used] What an alert does** (`app/src/window.rs` `fn alert`): it plays the
  done/blocked sound if sounds are on (Eli: off), brings a settled tab back, and, unless you're
  looking, posts a toast.
  - Title "<name> needs you" (level error) or "<name> finished" (level info). Body
    "<space> / <tab>". The click action is `agent.focus {terminal}`.
- [ ] **[Shipped, used] Done becomes idle** once anyone looks at it on any Herdr client (shared
  through core; `herdr.seen` label).
- [ ] **[Eli] Toasts** (`"toast": "eli-toast"`, a clone of `toast`).
  - 340×72 cards, bottom right 16px in, stacked with 8px gaps.
  - Default timeout 5s, or 8s for error/warning; a toast can set its own (`app/src/window.rs`
    `open_overlay`).
  - Content: an icon or a level dot (error=blocked color, warning=working, success=done,
    info=accent), a 13px semibold title and a 12px muted body, one line each unless `fitHeight`.
  - Clicking runs the action and dismisses.
  - A "<m> needs login; run hc machine login <m>" body becomes "<m> needs a login: run" plus the
    command and a Copy button (`cfg/mods/eli-toast/main.js`).
- [ ] **[Eli] Notes about agents on hidden machines close at once.** If the toast's action targets
  a terminal (or a pane mapped through `panes()`) on a machine in
  `dotfiles.agents.hidden-machines`, it closes on init (`cfg/mods/eli-toast/filter.js`).
- [ ] **[Shipped, used] Other toasts Eli sees.**
  - Mod-posted: dead space closed, Neovim navigation failed, copy mode with no focused terminal,
    `set-shared` misuse.
  - App: terminal failed to start (non-zero exit within 2s, with its last lines), edits moved to
    `<name>-edited`, layout needs missing mods, core update failed (with dismiss).
- [ ] **[Shipped, unused] Dock badge and window title** (`setBadge`, `setWindowTitle`). No mod of
  Eli's calls them.

## 9. Machines

- [ ] **[Shipped, used] Remote spaces.** Spaces can live on another machine from
  `hc machine ls`. Eli's: `devbox`, `omarchy`, `work-mac`. Its tabs and splits stay there;
  remote ids carry `<machine>/`. Temporary machines (`--temp`) are left out (`cfg/AGENTS.md`).
- [ ] **[Eli] Which machine is shown**: the tab bar's end label (§4).
- [ ] **[Eli] Agents' machine** on row line 2 for remote agents (§6).
- [ ] **[Eli] Machine filter popup** (`ctrl-g /` via `mod.open`: width 340, 0.5 high, 40px from
  the top; or the header button: 280×260) (`cfg/mods/agent-machines/`).
  - Rows: "All machines", This computer, connected machines in Herdr's order, then machines
    known only from an agent or the hidden list. Each shows a checkbox, the agent count,
    "offline" when unavailable, and "⏎ only" on the cursor row.
  - ↑/↓ or J/K move. Space toggles. Enter shows only that machine and closes. Esc or Q closes.
    Click toggles.
  - The hidden list is stored in shared `dotfiles.agents.hidden-machines` (`""` = this computer;
    null when empty), so a newly connected machine shows until hidden.
- [ ] **[Eli] Remote-aware features.**
  - Neovim navigation (`exec --on`).
  - nvim-links resolves paths and runs Neovim on the pane's machine.
  - new-space lists the remote machine's zoxide and repos folders and creates the space there.
  - review is per machine.
  - pr-finder and the toast honor the machine filter.
- [ ] **[Shipped, used] Machine management.** Settings → Machines lists, renames and removes ssh
  machines and has "Add a machine", which opens the shipped `machine-add` dialog: ssh-config
  hosts, `hc machine add`, and instructions when a host needs a password, a host key or
  herdr-core installed (`cfg/frame/settings/main.js:735`, `cfg/mods/machine-add/main.js`).
  - Login guidance (`hc machine login <name>` with Copy), lingering warnings and Connect for a
    rebuilt machine also appear there (`cfg/AGENTS.md` App).
- [ ] **[Shipped, unused] Sidebar machine groups / warning marks** in the shipped `spaces` mod.

## 10. Pickers, palette, prompt, confirm, menus, help

- [ ] **[Shipped, used] Tab/agent picker** (`cmd-p`, `mod.open:picker`): 640 wide, 0.8 high, 40px
  from the top, "Go to…" (`cfg/mods/picker/main.js`).
  - Groups by space with colored avatars (first emoji or letter). Spaces with something running
    come first, blocked first. Quiet spaces fold into "earlier: N quiet spaces ▾".
  - Rows: status glyph, tab name, "needs you" / "finished", the agents' program names and a
    sub-agent count "+N". It lists agents' tabs too.
  - The last rows are "new tab", "new space" and "new space on <machine>" for each connected
    machine (these run plain `space.new`, not new-space).
  - Typing filters by substring over space, tab and program names. ↑/↓ and Enter go, Ctrl+D
    closes the selected tab, Esc leaves.
- [ ] **[Shipped, used] Command palette** (`cmd-shift-p`, `app.palette`; the app's
  `frame-palette`, popup 620 wide, 0.85 high).
  - Every command by name with its key, fuzzy-matched. Empty shows recent commands first, then
    groups Tab, Pane, Space, Layout, Terminal, Agent, Window, Mod, App.
  - Commands that need a name ask for it. Mods can add `palette` entries in `mod.json`; none of
    Eli's do (`cfg/frame/palette/main.js`).
- [ ] **[Shipped, used] Prompt dialog** (`prompt` mod; the app opens it for `tab.rename`,
  `space.rename` and `pane.rename` without a name).
  - 380 wide, a titled "Name" field with the current value selected and a hint ("empty: its
    number"), Cancel/Save.
  - Enter saves, Esc cancels. A second rename press closes it instead of stacking
    (`cfg/mods/prompt/main.js`, `app/src/window.rs` rename).
- [ ] **[Shipped, used] Confirm dialog** (`confirm` mod) for `space.close` and closing a space's
  last tab: 380 wide, title, detail, Cancel/Confirm. Enter confirms, Esc cancels
  (`cfg/mods/confirm/main.js`).
- [ ] **[Shipped, used] Tab menu** (right-click a tab in `eli-tabs`): Rename, New tab, Close. Each
  item shows its `global` key hint and an icon. ↑/↓, Enter, Esc (`cfg/mods/menu/main.js:58-63`).
- [ ] **[Shipped, used] Terminal menu** (right-click, or Shift+right-click when the program takes
  the mouse).
  - Rename (pane), Split right, Split down, Zoom.
  - "Send right-clicks to pane" / "Use Herdr right-click menu".
  - "Send back to <parent>" for a kept sub-agent (`terminal.park`), Close pane.
  - Pane items are left out for a terminal shown outside a tab (`menu/main.js:28-57`).
- [ ] **[Shipped, used] Keys help** (`cmd-/`, `cmd-shift-/`; `help` mod as a 640×520 dialog).
  - Every binding from `keys()`, grouped spaces, tabs, panes, agents, terminal, window, with
    non-global sections labelled, and filtered by typing. Esc closes.
  - Limits: `<mod>.<command>` bindings show as raw words ("navigator-status tab-1") under
    "window", and the mods' own in-popup keys aren't listed (`cfg/mods/help/main.js`).
- [ ] **[Shipped, unused] Space menu** (rename, New/Open/Remove worktree, close): only the
  shipped `spaces` sidebar opens it (`cfg/mods/spaces/main.js:305`).
- [ ] **[Shipped, unused] Global menu** (`menu` with `tab: "global"`: new space, keybinds, theme
  presets).
- [ ] **[Shipped, used] Welcome / What's new / Feedback** frames (`app.welcome`,
  `app.whats_new`, `app.feedback`): through the palette only. What's new opens once after
  spotlight updates.

## 11. New space and worktree flows

- [ ] **[Eli] New space in a picked folder** (`cmd-n`, `ctrl-g ctrl-s` → `mod.open:new-space`;
  `cfg/mods/new-space/`). A popup 640 wide, 0.6 high, 40px from the top, titled "New space" or
  "New space on <machine>".
  - Folders in order: the focused terminal's folder (tag "here"), so Enter right away equals
    plain `space.new`; then `zoxide query -l` in frecency order; then `repos` config paths
    (`${XDG_CONFIG_HOME:-~/.config}/repos/config.json` `repos[].path`, tag "repo") that zoxide
    hasn't seen. Nothing is walked, so it opens at once; it shows "Loading folders…" until zoxide
    and the config arrive (5s timeout each).
  - Rows: folder name (semibold), parent path (dim, `~`-abbreviated) and tag. 12 visible, with
    "↓ N more".
  - Matching: fzf-style, every term must match. The last path segment beats elsewhere, a segment
    start beats mid-word, shorter names win, ties keep gathered order. A typed `/…`, `~` or `~/…`
    path is offered first with tag "path".
  - ↑/↓ or Ctrl+N/P move, Enter makes the space (`space.new {machine, folder}`), Esc closes.
  - Edge: Enter while loading with nothing matching waits instead of falling back.
  - In a space on another machine, the folders and `$HOME` come from that machine (`exec --on`)
    and the space is made there.
- [ ] **[External] Worktrees through `repos`.** Eli makes worktree spaces with `repos` (§3), not
  the app's worktree dialog.
- [ ] **[Shipped, unused] Worktree dialog** (`worktree` mod: New/Open/Remove worktree, checkouts
  in `<worktrees.directory>/<repo>/<branch>`). Only the shipped `spaces` menu and `new-menu`
  open it, and Eli's layout has neither.
- [ ] **[Shipped, unused] New menu** (`new-menu`: a terminal or any installed agent, New/Open
  worktree) from the shipped tab bar's "+".

## 12. PR finder (`ctrl-g f`)

- [ ] **[Eli] Which agent owns this PR** (`mod.open:pr-finder`; `cfg/mods/pr-finder/`). A popup
  620 wide, 0.6 high, "Find a PR", with a "PR number or repo…" field.
  - Rows: agents watching at least one PR, in the Agents list's published order, minus hidden
    machines.
  - Each row looks like an Agents row without status, jump label or time: icon, title, the space
    in its color, ticket and "N workers" for orchestrators, up to 3 matched PR chips plus "+N",
    and the machine.
  - Live rows keep the status-colored edge. 9 visible with "↓ N more".
  - Empty states: "No agent is watching a PR" / "No agent watches a PR like that".
  - Matching (`match.js`): terms split on whitespace and `#`, and every term must match one PR.
    A number matches the PR number (exact 1000, then prefix, then substring). A word matches the
    repo (exact name, prefix, substring in owner/repo, then letters in order: "fmr" finds
    ffs-mono-repo). Rows rank by their best PR; ties keep list order.
  - PR data comes from the `dotfiles.pi_pr_watch` label's `prs` (display names like `#1901` or
    `tool#7`) and `refs` (`owner/repo#N`). Older Pi sessions without `refs` fall back to the
    space name's repo. A merge SHA watch (`merge of #N`) counts as that PR.
  - ↑/↓ or Ctrl+N/P move, Enter or a click focuses that agent (`agent.focus`), Esc closes.

## 13. Review view (`cmd-r`)

- [ ] **[Eli] The review TUI as a view** (`navigator-status.review`;
  `cfg/mods/navigator-status/review-view.js`).
  - One review terminal per machine, found by label `dotfiles.review` on the focused space's
    machine.
  - Cmd+R focuses it. Cmd+R on the review goes back to the terminal you came from, if still
    live.
  - With none running, it starts one in the current space: `tab.new {space, name: "review",
    argv: ["/bin/sh","-c","hc label \"$HC_TERMINAL_ID\" dotfiles.review=1 …; review; hc kill
    \"$HC_TERMINAL_ID\""]}`. The terminal ends when review quits, so the next press starts
    fresh, and the launcher leaves no shell behind.
  - It must live in a space because `review` opens its Review and Diff tabs in the space and
    folder it runs in.
  - Its tab is hidden from the tab bar and tab keys, and the space picker avoids it as an
    agent's tab (§3, §4). A review started by hand isn't labelled and stays an ordinary tab.

## 14. Changes and Files views

- [ ] **[Shipped, unused, in layout] Files panel** (`files` mod): a lazy browser of the focused
  pane's folder (repo-aware, git status), with bounded read-only previews in the terminal font.
  Lists are limited to 500 entries per folder and 5,000 total (`cfg/mods/files/main.js`).
- [ ] **[Shipped, unused, in layout] Changes panel** (`changes` mod): uncommitted files with status
  letter and +/− counts, a virtualized diff of the chosen file, and a commit-message field with a
  Commit button. At most 1,000 files are shown (`cfg/mods/changes/main.js:296-331`).
- Both sit in the eli layout's hidden `panel` stack, 368px. With `workspace.panels: []` the tab
  bar draws no buttons, and the palette hides `layout.show`, so neither is reachable. Eli opted
  out on purpose: stow "removes the top-right Files/Changes buttons" (`cfg/README.md` Setup).
  Diffs are reviewed in the review TUI (§13) and Neovim instead.

## 15. Peek

- [ ] **[Shipped, used] Peek a terminal** (`terminal.peek`): an 80%×80% dialog holding the live
  terminal under a 34px `peek` bar (`app/src/window.rs` `peek_terminal`).
  - The bar shows a status glyph (braille spinner frames at 90ms while working), the icon, the
    name (name, template, program or address), a status word (working, needs you, finished,
    idle) and "from <parent>".
  - "↗ go to" focuses its pane, if it has one. "↗ keep" gives it a pane via `terminal.keep` in
    the shown space if it's on the same machine, else the nearest ancestor's space there, else
    the first space there; it is hidden if that machine has no space. "✕" closes.
  - Esc goes to the terminal, since agents use it (`cfg/mods/peek/main.js`).
  - Opened from: an Agents list sub-agent row (§6), or Cmd-click on a live terminal address in
    terminal text. Bare addresses mean the showing terminal's machine; `machine/address` means
    another (`cfg/AGENTS.md`).

## 16. Copy mode (`ctrl-g [`)

- [ ] **[Eli] Keyboard copy mode** (`mod.open:copy-mode`; `cfg/mods/copy-mode/`). Built on the
  0.1.16 selection commands; the app has no built-in copy mode.
  - A 32px bar at the top (width 420, offset 6) shows the mode (COPY in accent; VISUAL or VISUAL
    LINE in working blue), hints, and the cursor row "N/total". The selection is drawn in the
    terminal itself.
  - It starts at the program's cursor: it runs `terminal.select {at: "cursor"}` and polls up to
    500ms for the selection to land. With no focused terminal it shows a warning toast and
    leaves.
  - Motions:
    - h/j/k/l and arrows; `0`/Home is the first column.
    - `^` and `$`/End go to the first and last text on the row.
    - g/G are the top and bottom of history; H/M/L the top, middle and bottom of the view.
    - Ctrl+U/D move half a page; Ctrl+B/F and PageUp/PageDown a full page.
    - j/k keep the wanted column. The view follows the cursor through history.
  - `v` or Space selects characters and `V` lines. Pressing the same key again or Esc drops the
    selection.
  - `y`, Enter or Cmd+C copy (`terminal.copy`) and leave; with no selection they just leave.
    `q`, Ctrl+C, a click outside, or Esc with nothing selected leave without copying.
  - Leaving clears the selection and scrolls to the bottom when the screen has history.
  - Esc reaches the mod only through `keys.json` `"mod:copy-mode": {"escape":
    "copy-mode.escape"}`; otherwise the popup would close before the mod saw it.
  - Mechanics (`copy.js`): `run()` only queues commands and returns nothing, so the mod keeps its
    own cursor and computes each motion as `terminal.adjust_selection` steps (moves stop at
    edges, up/down keep the column, line_start/line_end, home/end).
    - `select` doesn't scroll the view, so it steps away and back to make the view follow.
    - `^`/`$` select the row as a line for a moment and read where Ghostty's trimmed line
      selection starts and ends. Keys pressed meanwhile queue.
- **Current limits** (`cfg/README.md` Copy mode, `dot/.scratch/herdr-app-parity/spec.md`):
  - No word motions (w/b/e).
  - No search (/ ? n N): the mod API exposes no terminal text.
  - `^`/`$` need the select-line trick, and a blank row selects nothing.
  - On the alternate screen (vim, less, full-screen agents) there is no history to move through.
  - herdr-frontend owns the terminal buffer and can lift all of these.

## 17. Terminal behavior

- [ ] **[Shipped, used] Search** (`cmd-f`, `terminal.search`): a find bar in the focused terminal
  over the screen and scrollback.
  - Enter/Shift+Enter move through matches and Escape closes while the field has focus.
  - Clicking the terminal returns typing to the program, while the bar and matches stay. Each
    terminal keeps its own query, and reopening selects it (`cfg/AGENTS.md` Keys).
- [ ] **[Shipped, used] Copy/paste.**
  - `cmd-c` copies the selection; with nothing selected the key goes to the program.
  - `cmd-v` and `ctrl-v` paste. Copied files paste as if dropped, and an image alone pastes as
    `terminal.paste_image`.
  - `ctrl-shift-v` pastes the clipboard image: to a remote terminal it is copied there and its
    path pasted.
  - `copy_on_select` (default on) copies a mouse selection on release. TIFF/BMP are converted to
    PNG.
- [ ] **[Eli] Editing keys sent as bytes** (`terminal` section): `alt-left`/`alt-right` send `ESC
  b`/`ESC f` (word back/forward), `alt-backspace` sends `^W`, `cmd-left`/`cmd-right` send
  `^A`/`^E`, `cmd-backspace` sends `^U`, and `ctrl-g ctrl-l` sends `^L` (clear screen, since
  `ctrl-l` is navigation).
- [ ] **[Shipped, used] Mouse.**
  - Dragging past the edge scrolls while held. Shift+click extends the selection. Programs that
    track the mouse still get events; hold Shift to select instead.
  - A drag stays with its terminal.
  - Right-click goes to the program or the menu (per terminal, label `herdr.right-click`).
  - Pane scrollbars are off (Eli).
- [ ] **[Shipped, used] Links.** Cmd-click a URL to open it. Cmd-click a file path to send it
  through `link_handlers` (`nvim-links`), then the system opener. Cmd-click a terminal address
  to peek it.
- [ ] **[Eli] File links open in Neovim** (`"link_handlers": ["nvim-links"]`;
  `cfg/mods/nvim-links/`).
  - Local clicks use Herdr's resolved `link.path`: relative to the clicked pane's live folder,
    with `~/` and `$HOME` expanded. Folders (`test -d`) and missing paths pass to Herdr's opener.
  - Opened in Neovim: files with no extension (Makefile, Dockerfile, `.gitignore`) and the
    extensions `install-neovim-app` registers (txt, md, json, yaml, toml, source languages,
    html, css…, list in `open.js:17-22`). Others (images, PDFs, media) pass.
  - It runs `pane.split {direction: "right", pane: <clicked pane>, argv: ["nvim", "+42" |
    "+call cursor(42, 7)", "--", path]}` for `file:42` / `file:42:7`. A trailing sentence period
    is dropped, and a `:N` that is part of a real filename isn't a place.
  - Remote panes: Herdr gives `link.path = null`, so the mod runs a POSIX `sh` script on that
    machine (`exec --on`, 10s timeout).
    - The script resolves each spelling as Herdr would: `~/`, a leading `$VAR` read with
      `printenv` (never evaluated), `$PWD`, absolute, or relative to the pane's cwd there; plus
      `file:` URIs with an empty or localhost host.
    - It requires a regular file (`test -f`). The split and Neovim run on that machine. There is
      no remote opener for anything else.
  - Without a pane (a layout terminal block), it splits beside the focused one. A handler error
    shows a toast and stops the click.
- [ ] **[Shipped, used] Kitty graphics, OSC 52 clipboard writes (≤1 MiB, reads never),
  `minimum_contrast` and file drag-and-drop into terminals** (remote: copied there) (`cfg/AGENTS.md`).
- [ ] **[Shipped, used] Select all** (`terminal.select_all`), from the palette only. The docs give
  Cmd+A as the macOS default, but Eli's edited `keys.json` doesn't bind it, an edited keys file
  keeps no new defaults (`cfg/AGENTS.md` Keys), and the terminal menu doesn't offer it. The
  palette lists it because it isn't in the palette's hidden set
  (`cfg/frame/palette/main.js` `HIDDEN`).

## 18. Session continuity and naming (hooks outside the app)

- [ ] **[Eli] Pi resumes after a reboot or crash.** When herdr-core reopens a lost kept terminal
  in place with a fresh login shell and keeps its labels:
  - `hc-session-label.ts` has labelled `dotfiles.pi_session` and `dotfiles.pi_shell` and left a
    marker.
  - On the first prompt, `conf.d/hc-pi-resume.fish` finds a session labelled under another
    shell's token and runs ` PI_PROGRAM_STATUS=1 pi --session <file>` from the command line, as
    if typed.
  - The status variable is needed because on a reopened terminal fish's startup DA reply
    reaches Pi before its OSC 7501 query reply, so Pi wouldn't report.
  - When Pi returns to its shell, the labels (including `dotfiles.pi_pr_watch`) are cleared, so
    a Pi you quit stays quit.
  - `~/.config/herdr-core/config.toml` `restore_screen = true` shows the lost screen above the
    new shell (`dot/home/.config/fish/conf.d/hc-pi-resume.fish`, `cfg/README.md`).
  - Mostly core and shell. herdr-frontend must treat a reopened terminal as the same address
    with its labels, and must not answer 7501/DA in a way that breaks Pi's detection.
- [ ] **[Eli] Tab names follow Pi session names** (§4): Pi writes them with
  `herdr-app do tab.rename`. herdr-frontend must keep that command working, or give Pi
  another way to name a tab.
- [ ] **[Shipped, used] Kept terminals.** Terminal panes are kept by core (`keep: true`). A lost
  terminal gets a shell in its last folder with the same address; its command never reruns.
  Closing a pane ends its terminal.

## 19. Housekeeping state Eli relies on

- [ ] Mod `localStorage` persisted under `cfg/data/<mod>/store.json`: eli-agents' `changed` times
  and pending pins, and eli-tabs' per-space panel choice.
- [ ] Layout clicks (sizes, shown/hidden, stack child) persist in `layouts/.state/`. The sidebar's
  shown state and width survive restarts.
- [ ] `view.json`: this screen's selected space, tab and pane, zoom, settled tabs, recent spaces
  and seen attention.

## 20. Shipped but unused

Shipped (in `cfg/installed.json`), unmodified, and not reached from the eli layout or Eli's keys.
They define no parity requirement unless Eli asks for them.

| Shipped | What it does | Why it's unused |
|---|---|---|
| `spaces` | Spaces sidebar: colored avatars, worktrees nested under their repo, machine groups, branch and ahead/behind, drag to reorder, space menu, Add machine | Replaced by the agents-only sidebar plus `space-picker`. Eli's `eli-spaces` clone was removed in `25c8ffdf` |
| `agents` | Agents list with header, sort, pin button | Cloned as `eli-agents` |
| `tabs` | Tab bar with "+" (`new-menu`), Files/Changes buttons | Cloned as `eli-tabs` |
| `workspace-chrome` | Brand row, search, sidebar toggle, Add machine | Cloned as `eli-chrome`. Its icons (`spinner`, `monitor`, `server`, `check`, `clock`, `layers`, `chevron-*`) are still used by Eli's mods |
| `workspace-status` | Status bar (branch, agent count, machine) | Cloned as the invisible `navigator-status` |
| `toast` | Notifications | Cloned as `eli-toast` |
| `new-menu` | New terminal / agent / worktree menu | No "+" button; Cmd+T and Ctrl+G T make a tab |
| `worktree` | New/Open/Remove worktree dialog | `repos` does worktrees |
| `files`, `changes` | Side panels | In the layout but unreachable on purpose (§14) |
| `pane`, `terminals-pane` | Pane headers | The eli layout has no pane headers. `eli-tabs` still loads its close icon from `mods/pane/close.svg` |
| `tabs` icons | `mods/tabs/win-*.svg` | Used by `eli-tabs` for Windows window buttons (`eli-tabs/main.js:120-122`) |
| `terminals-bar`, layout `terminals` | Title-bar tabs with a space picker | Other layout |
| `win98`, layout `win98` | Windows 98 desktop | Other layout |
| layout `main` | Default layout | `eli` is the clone in use |
| Space menu, global menu | See §10 | No `spaces` sidebar |
| `tab.settle` / `tab.unsettle` | Put tabs away | Shipped layouts don't use it; Eli hides agent tabs his own way. The tab keys still skip settled tabs |
| `setBadge`, `setWindowTitle`, floating blocks, `window` overlays, `terminal.show`, agent blocks (`{"agent": "focused"}`) | Mod API features | Not used by Eli's mods or layout |
| `app.theme` presets (graphite, ink, moss, paper, atom-one-dark) | Themes | Arctic only |

## 21. Friction points and limits (motivation)

What Eli's mods work around. Each is a place where herdr-frontend can do better by owning the
client.

1. **Mods can't read terminal text.** Copy mode has no word motions or search, and `^`/`$` must
   select a line to find text edges (§16). Edit-scrollback (old Ctrl+G E) wasn't migrated
   (`cfg/README.md` "Not migrated").
2. **`run()` is fire-and-forget.** Copy mode keeps a shadow cursor and replays
   `adjust_selection` steps, polling `terminals()` for selections to land (`copy-mode/copy.js`
   header, `main.js:42-54`).
3. **Only shown mods get `<mod>.<command>` keys.** Hence a 0px `navigator-status` block that
   exists to receive commands (`navigator-status/main.js:51-54`). A hidden sidebar's mod isn't
   running until first shown, so the published agent order can be stale and `agent-order.js`
   re-implements the sort as a fallback that can disagree (§7).
4. **The navigation order can be set but not read, and has one shape.** `agent.select {index}`
   counts the full `setAgentOrder` list (folded workers, then agents on hidden machines), which
   isn't the shown rows. So the list publishes its own order through shared state with an
   `inputs` signature for staleness, and three mods read it (§6, §7).
5. **Tab commands don't know about hidden tabs.** `tab.select {index}`, `tab.next/prev` and
   tab-close focus count agents' tabs, so `navigator-status` re-implements numbered tabs,
   stepping and close-focus, and the tab bar, space picker and tab keys each keep their own copy
   of the "hidden tab" rule (`tab-order.js`, `eli-tabs/main.js:55-60`,
   `space-picker/main.js:51-59`).
6. **Mods can't share code.** Herdr sandboxes imports per mod, so `names.js`, `jump.js`,
   `watch.js` and `colors.js` are byte-identical copies kept equal by tests. Constants (shared
   keys, label names, `LIVE_TITLE`) carry "keep in sync" comments across up to 5 files.
7. **Overlays steal focus while opening.** Every popup refocuses itself after 600ms
   (`space-picker`, `pr-finder`, `new-space`, `agent-jump`, `agent-machines`, `copy-mode`,
   shipped `picker`).
8. **Esc closes a popup before the mod sees it.** Copy mode needs a `mod:copy-mode` keys section
   to receive it (§16).
9. **`hc keys` delays agent input by ~3s** after typing (`HC_QUIET_MS`), so the Neovim bridge
   shells out to `nvim --remote-send` on the right machine for every Ctrl+H/J/K/L (§5).
10. **Core reports only the foreground process group leader.** Neovim under `fish -c`, `git` or
    an agent looked like a shell, so Neovim labels its own terminal (commit `9e5ea27c`).
11. **No hidden traffic lights.** They are parked at x = −80 (§1). The terminal grid is centered
    and can't be pinned to the top (§1).
12. **Font weight selection.** 0.1.13 hardcoded Medium, so Eli needs a generated font family
    with rewritten weight metadata (§2).
13. **`shared.json` is read once and rewritten from memory.** Dotfiles can't manage live
    preferences without a command channel (`preferences.js`, `configure-herdr-app.sh`).
14. **A space's folder is fixed at creation**, and worktree deletion isn't noticed. Hence
    new-space's folder picker and the dead-space sweep (§3, §11).
15. **No way to filter Herdr's own notifications.** `eli-toast` must open and immediately close
    notes about hidden machines (§8).
16. **Remote links get no path.** nvim-links resolves paths itself with a shell script over
    `exec --on` (§17).
17. **Pi can't name remote tabs.** `herdr-app do` is local, so the tab bar and Agents list parse
    Pi's OSC title `π - <name> - <folder>` (§4).
18. **Session resume isn't first-class** (herdr-app#429, herdr-core#15). Shell hooks and labels
    re-launch Pi and must coax it into reporting status (§18).
19. **The review tool must live in a space** because it opens its tabs where it runs; hiding its
    tab means another copy of the hidden-tab rule (§13).
20. **Help can't describe mod commands or in-popup keys** (§10). Eli's real keymap lives in his
    README.
21. **The cloned Agents list carries dead rail code.** The shipped list's narrow rail is
    width-based and writes a legacy `sidebar.collapsed` flag for older mods, but eli's
    `min: 200` sidebar never gets narrow enough to use it (`eli-agents/main.js:25-35`, §6).
    Cloning a shipped mod brings along behavior the layout can't reach.
22. **Not migrated from the TUI:** edit scrollback (Ctrl+G E), the annotation/document-review
    plugin keys (Ctrl+G A, Shift+A, Ctrl+A, M, O, Shift+O) and the pane navigation-mode
    H/J/K/L (`cfg/README.md` "Not migrated"). Ctrl+G O has since been taken by the jump list
    (`keys.json` `layout:eli`), so restoring the annotation keys would need a new key for one of
    them.

## 22. Keybinding table

From `cfg/keys.json`. Effective means what the key does in the eli layout: `layout:eli` overrides
`global`, and `terminal` (a terminal has focus) overrides both (`cfg/AGENTS.md` Keys). Prefix
keys are GPUI sequences: press Ctrl+G, release, then the key.

### Global and layout:eli

| Key | Effective action (eli) | Global binding, if overridden | What |
|---|---|---|---|
| `cmd-q` | `app.quit` | | Quit |
| `cmd-h` | `app.hide` | | Hide app |
| `cmd-alt-h` | `app.hide_others` | | Hide others |
| `cmd-t`, `ctrl-g t` | `tab.new` | | New tab |
| `cmd-w`, `ctrl-g shift-q` | `tab.close` | | Close tab |
| `ctrl-tab`, `cmd-shift-]`, `ctrl-g n` | `navigator-status.tab-next` | `tab.next` | Next shown tab / next agent on an agent's tab |
| `ctrl-shift-tab`, `cmd-shift-[`, `ctrl-g p` | `navigator-status.tab-prev` | `tab.prev` | Previous shown tab / previous agent |
| `cmd-1`…`cmd-9`, `ctrl-g 1`…`ctrl-g 9` | `navigator-status.tab-1…9` | `tab.select:1…9` | Nth shown tab |
| `cmd-alt-]` / `cmd-alt-[` | `space.next` / `space.prev` | | Next / previous space |
| `ctrl-1`…`ctrl-9` | `space.select:1…9` | | Nth space |
| `cmd-]`, `ctrl-g j` | `agent.next` | | Next agent (list order) |
| `cmd-[`, `ctrl-g k` | `agent.prev` | | Previous agent |
| `ctrl-g shift-1`…`shift-9` (also `! @ # $ % ^ & * (`) | `navigator-status.agent-1…9` | | Focus the Nth Agents row |
| `ctrl-g shift-0` (also `)`) | `navigator-status.agent-10` | | Focus the tenth row |
| `ctrl-;` | `mod.open:agent-jump` | | Jump to any agent by label |
| `ctrl-g o` / `ctrl-g i` | `navigator-status.jump-list-back` / `-forward` | | Tab jump list |
| `cmd-r` | `navigator-status.review` | | Review view / back |
| `cmd-shift-l` | `layout.switch` | | Next layout |
| `cmd-n`, `ctrl-g ctrl-s` | `mod.open:new-space` | | New space in a picked folder |
| `cmd-p` | `mod.open:picker` | | Tab/agent picker |
| `ctrl-g s` | `mod.open:space-picker` | | Space picker |
| `ctrl-g /` | `mod.open:agent-machines` | | Machine filter |
| `ctrl-g f` | `mod.open:pr-finder` | | Find a PR's agent |
| `ctrl-g [` | `mod.open:copy-mode` | | Copy mode |
| `cmd-shift-p` | `app.palette` | | Command palette |
| `cmd-,`, `ctrl-g c` | `app.settings` | | Settings |
| `cmd-/`, `cmd-shift-/` | `app.help` | | Keys help |
| `cmd-b`, `ctrl-g b` | `layout.toggle:sidebar` | | Toggle sidebar |
| `cmd-=`, `cmd-+` / `cmd--` / `cmd-0` | `app.zoom_in` / `app.zoom_out` / `app.zoom_reset` | | Zoom |
| `cmd-d`, `ctrl-g v` | `pane.split:right` | | Split right |
| `cmd-shift-d`, `ctrl-g h` | `pane.split:down` | | Split down |
| `cmd-shift-w`, `ctrl-g q` | `pane.close` | | Close pane |
| `cmd-shift-enter` | `pane.zoom` | | Zoom pane |
| `cmd-alt-left/right/up/down` | `pane.go:<dir>` | | Focus pane by direction |
| `ctrl-h` / `ctrl-j` / `ctrl-k` / `ctrl-l` | `navigator-status.left/down/up/right` | `pane.go:<dir>` | Neovim-aware pane navigation |
| `cmd-alt-shift-left/right/up/down` | `pane.swap:<dir>` | | Swap pane |
| `alt-shift-h/j/k/l` | `pane.resize:left/down/up/right` | | Resize by 40px |
| `f2`, `ctrl-g r` | `tab.rename` | | Rename tab |
| `shift-f2` | `space.rename` | | Rename space |
| `ctrl-g x` | `space.close` | | Close space (confirm) |
| `ctrl-g d` | `window.close` | | Close window (terminals keep running) |
| `ctrl-alt-shift-escape` | `app.reset` (built in, unbindable) | | Escape hatch |

### terminal (a terminal has focus)

| Key | Action | What |
|---|---|---|
| `cmd-f` | `terminal.search` | Find in screen and scrollback |
| `cmd-c` | `terminal.copy` | Copy selection (no selection: key to program) |
| `cmd-v`, `ctrl-v` | `terminal.paste` | Paste (files as dropped) |
| `ctrl-shift-v` | `terminal.paste_image` | Paste clipboard image |
| `alt-left` / `alt-right` | `send:\u001bb` / `send:\u001bf` | Word back / forward |
| `alt-backspace` | `send:\u0017` | Delete word (^W) |
| `cmd-left` / `cmd-right` | `send:\u0001` / `send:\u0005` | Line start / end (^A / ^E) |
| `cmd-backspace` | `send:\u0015` | Delete to line start (^U) |
| `ctrl-g ctrl-l` | `send:\f` | ^L to the program (clear screen) |

### mod:copy-mode

| Key | Action |
|---|---|
| `escape` | `copy-mode.escape` (Esc reaches copy mode instead of closing its popup) |

### Keys inside Eli's popups (handled by the mods, not keys.json)

| Popup | Keys |
|---|---|
| agent-jump | `1`–`9`, `0`, letters (`asdf…`, or two letters past 36 rows): jump. Esc, `ctrl-;` or click outside: leave |
| agent-machines | ↑/↓ or J/K: move. Space: show/hide. Enter: only this machine. Esc or Q: close. Click: toggle |
| space-picker | Type to filter. ↑/↓ or Ctrl+N/P: move. Enter: go. Esc: close |
| pr-finder | Type to filter. ↑/↓ or Ctrl+N/P: move. Enter or click: focus agent. Esc: close |
| new-space | Type to filter (or a `/…`, `~/…` path). ↑/↓ or Ctrl+N/P: move. Enter: make the space. Esc: close |
| copy-mode | h j k l / arrows, `0` / Home, `^`, `$` / End, `g` `G`, `H` `M` `L`, Ctrl+U/D, Ctrl+B/F / PageUp/PageDown: move. `v` / Space: select characters. `V`: select lines. `y` / Enter / Cmd+C: copy and leave. `q` / Ctrl+C: leave. Esc: drop the selection, else leave |
| sort menu (eli-chrome) | Click an option |
| picker (shipped) | Type to filter. ↑/↓: move. Enter: go. Ctrl+D: close the selected tab. Esc: close |
| menu (shipped) | ↑/↓: move. Enter: run. Esc: close |
| prompt / confirm (shipped) | Enter: save or confirm. Esc: cancel |
| help (shipped) | Type to filter. Esc: close |

### Mouse

| Where | Gesture | Does |
|---|---|---|
| Tab pill | Click / double-click / drag onto another tab / right-click / × | Select / rename / move / tab menu / close |
| Tab bar | Scroll wheel | Previous/next shown tab |
| Tab bar, sidebar top row | Drag / double-click | Move / maximize window |
| Agents row | Click | Focus its pane |
| Sub-agent row | Click | Peek |
| Agent icon / fold chevron | Click | Fold sub-agents |
| Section header | Click | Fold section |
| Header machine / sort buttons | Click | Machine filter / sort menu |
| Terminal | Cmd-click path / URL / address | nvim-links or opener / open URL / peek |
| Terminal | Right-click, Shift+right-click | Terminal menu (or program) |
| Pane top edge | Drag grip | Rearrange panes |

## Appendix: Eli's regression checks

The dotfiles hold executable specs for most Eli features. They are useful as acceptance oracles
for herdr-frontend's equivalents (`cfg/README.md` "Regression checks"; `dot/tests/`):

| Test | Covers |
|---|---|
| `tests/herdr-app-navigation.mjs`, `tests/herdr-app-navigation.lua` | Neovim-aware Ctrl+H/J/K/L |
| `tests/herdr-app-jump-list.mjs` | Jump list |
| `tests/herdr-app-agent-tabs.mjs`, `tests/herdr-app-tab-close-focus.mjs` | Hidden agent tabs, tab keys, close focus |
| `tests/herdr-app-tab-icons.mjs` | Tinted tab icons |
| `tests/herdr-app-session-names.mjs` | Pi session names, identical `names.js` copies |
| `tests/herdr-app-agent-jump.mjs` | Jump labels |
| `tests/herdr-app-agent-watch.mjs` | Pi worker nesting, PR chips |
| `tests/herdr-app-agent-colors.mjs` | Space colors |
| `tests/herdr-app-agent-machines.mjs` | Machine filter, toast filter |
| `tests/herdr-app-space-picker.mjs` | Space matching, terminal-tab choice |
| `tests/herdr-app-pr-finder.mjs` | PR matching |
| `tests/herdr-app-new-space.mjs` | Folder gathering and ranking |
| `tests/herdr-app-copy-mode.mjs` | Copy-mode state machine |
| `tests/herdr-app-nvim-links.mjs` | Link planning, remote lookup, extension list |
| `tests/herdr-app-review-view.mjs` | Review toggle |
| `tests/herdr-app-dead-spaces.mjs` | Dead-space sweep |
| `tests/hc-pi-resume.sh` | Pi resume under a real pty |
| `tests/herdr-font-preparation.py`, `tests/herdr-app-font.swift` | Font family preparation and face selection |
