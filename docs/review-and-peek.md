# The review view and the peek

`research/feature-inventory.md` §13 and §15, as herdr-frontend builds them. The model side is
`crates/model/src/review.rs` and `crates/model/src/peek.rs`. The window side is
`crates/app/src/view/peek.rs` plus the peek surface in `State` (`state::PEEK`).

## The review view (`cmd-r`)

This is Eli's `navigator-status.review` (`cfg/mods/navigator-status/review-view.js`). It shows his
review TUI (`review`) as a view of the window.

- **One per machine.** The review is a live terminal with a non-empty `dotfiles.review` label,
  looked up on the shown space's machine.
- **`cmd-r`** focuses the review's pane, showing its tab and space. On the review, `cmd-r` goes
  back to the terminal it came from, if that terminal still runs. The terminal to go back to is
  per screen and kept in memory only.
- **None running.** `cmd-r` starts a review in a new tab after the selected one in the shown
  space. The tab is named `review` and starts in the focused terminal's folder, on that space's
  machine. It runs
  `/bin/sh -c 'hc label "$HC_TERMINAL_ID" dotfiles.review=1 >/dev/null 2>&1; review; hc kill "$HC_TERMINAL_ID"'`:
  the terminal labels itself, and it ends when `review` quits. The next press then starts a
  fresh one, and no shell is left behind. As Herdr app's `tab.new` does with any program, the
  command runs inside the login shell (`model::shell`), so `hc` and `review` are on `PATH`. On
  this computer that shell is `$SHELL`; on another machine it is the `$SHELL` there. The review needs to live in a space because it opens
  its Review and Diff tabs in the space and folder it runs in.
- **Its tab is hidden** from the tab bar and the tab keys, the same as an agent's tab
  (`Model::is_hidden_tab`). A review started by hand has no label, so its tab stays an ordinary
  one.

## The peek

The peek is Herdr app's `terminal.peek` plus its shipped `peek` mod (`app/src/window.rs`
`peek_terminal`, `keep_terminal`; `cfg/mods/peek/main.js`). It shows a running terminal over the
window without moving focus.

### Opening and closing it

- **It opens from:**
  - a sub-agent's row in the Agents list;
  - a jump to an agent that has no pane (`agent.next`/`prev`, the numbered rows, the agent-jump
    bar, `agent.focus`);
  - `do terminal.peek {terminal}`;
  - a Cmd-click on a live terminal's address in terminal text.

  A new peek replaces the one that's open, and it closes any open popup or copy mode. A terminal
  that isn't running isn't peeked.
- **The keyboard** goes to the peeked terminal (`Focus::Peek`, element `peek-terminal`), and so
  does Esc, because agents use it. A popup can open over the peek and gives the keyboard back to
  it when it closes. The focused pane stays focused behind the peek, but doesn't draw as focused.
- **Terminal commands follow the keyboard** (`Model::keyboard_terminal`): copy, paste, `send:`
  bindings, pastes, dropped files and copy mode go to the peeked terminal.
- **The size.** While the peek has the keyboard and the window is in front, the peeked terminal
  is the one in use (`herdr-core-protocol.md` §4). It takes the size of the peek's terminal area,
  then gives it back when the peek closes (FOCUS(0)).
- **Attention.** A peeked agent counts as on screen: what it shows is marked seen while the
  window is in front.
- **It closes on:**
  - its ✕, or a press outside it;
  - "↗ go to" or "↗ keep";
  - a key that closes a tab or pane (`cmd-w`, `ctrl-g q`, …), which closes the peek instead of
    what's behind it, as Herdr app's `run_key` does;
  - its terminal ending.

### The dialog and its bar

The dialog is 80% × 80% of the window, centered over the window dimmed 35%. It uses the popups'
frame (popover color, 1 pt border, radius 8, shadow). A 34 pt bar sits above the terminal and
shows, from left to right:

- **A status glyph** in the status's color. While the terminal works, it cycles through braille
  frames (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`), one every 90 ms. Otherwise it is `○` for idle, `●` for done or
  needs-you, and `·` with no status.
- **The program's icon.**
- **A name:** the terminal's name, else its template, else its program's name, else its address.
- **A status word:** working, needs you, finished or idle.
- **"from \<parent\>":** the terminal that started it, by name, else by address.
- **"↗ go to"** focuses the terminal's pane, when it has one.
- **"↗ keep"** gives a terminal with no pane a pane of its own. The new tab goes after the
  selected tab, in a space on the terminal's machine (a pane must stay on its machine): the
  shown space when it's on that machine, else the space of the nearest ancestor that has a pane
  there, else that machine's first space. The button is hidden when the machine has no space.
  Keeping works like Herdr app's `terminal.keep`:
  1. The space is shown.
  2. The terminal is labeled `herdr.space` and `herdr.tab` (bare ids on its machine).
  3. `keep` is set.
  4. Only then does the tab go into the layout, named after the terminal (its name, else its
     program, else its address), and get focus.

  This screen's own adoption leaves the terminal alone while a keep is under way. If the space
  closed in the meantime, the labels come back off and `keep` is turned off, the way parking
  does.
- **✕** closes the peek.

### Terminal addresses

Cmd-click works as in Herdr app's `link_at`. herdr-frontend looks only at the soft-wrapped line
under the pointer in the last snapshot (`view::peek::word_at`), not at lines a program broke
itself.

1. **The word** under the pointer is the run of letters, digits, `-`, `_`, `/` and `.` around
   it, without dots at its ends (`model::peek::word`).
2. **The address** (`model::peek::terminal_address`). A bare word means the machine of the
   terminal that shows it. `machine/address` names another machine. Only a live terminal's
   address counts, so names aren't links.
3. **The click** opens the peek and goes no further: no selection, nothing to the program. A
   Cmd-click that isn't on an address goes on as a plain click.

## Not yet

- While Cmd is held, Herdr app underlines a link under the pointer. herdr-frontend underlines
  URLs and paths that way (`docs/research/terminal-rendering.md` §2, "What herdr-frontend does
  (ticket 019)"), but not addresses yet; they should share it.
- The terminal menu isn't offered inside the peek.
- Find (`cmd-f`) has no bar in the peek: it says "not yet" there.

## Scenes

`just snap shell-peek`: the agents core peeking the `explore` sub-agent.
