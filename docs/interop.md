# Running beside Herdr app on one core

herdr-frontend and Herdr app can be open at the same time on the same herdr-core, showing the
same spaces and the same terminals. This page covers what each app does to a shared terminal's
size, how that is tested, and the checklist for trying both apps by hand.

The protocol facts behind it are in `docs/research/herdr-core-protocol.md` §4: "Who owns the PTY
size", the viewer policy, and what herdr-app does that causes wrong sizes.

## The size of a shared terminal

A terminal has one grid. The core gives it to the non-observe viewer that was active most
recently. A viewer becomes active when it attaches without `resumed`, sends DATA (typing), sends
RESIZE (even at an unchanged grid), or sends FOCUS(1). FOCUS(1) applies the grid that viewer
reported last. AUTOMATIC_INPUT, FACTS and FOCUS(0) are not activity.

What herdr-frontend does (`crates/app/src/state.rs`):

| When | What it sends |
| --- | --- |
| herdr-frontend starts a terminal for a pane (new space or tab, split, a program it runs, `space.new` from the control CLI) | `terminal.run` with `size`, the grid the new pane will hold (worked out as its RESIZE will), and `display`, the window's FACTS. Only before the window is first laid out is there no grid to give: the terminal then starts at the core's headless 200×120 until its pane's first RESIZE. |
| A pane appears, in any tab, and again after a dropped connection or a restarted program | `terminal.attach` with `resumed: true` and the core's current grid. It takes nothing. |
| A pane is shown in the window while the window is in front: its tab is shown, the peek opens on it, or the window comes to the front | FACTS (if the cell or colors changed), then RESIZE with the pane's grid. Every shown pane does this, not just the focused one. |
| A shown pane's grid changes while the window is in front (split, close, divider drag, keyboard resize, sidebar toggle, window resize, zoom, font) | RESIZE, at most every 25 ms per pane, always ending with the final grid. Every pane whose grid changed sends its own. |
| A pane becomes the pane in use: focus moves to it, the window comes to the front with it focused, or a tab or space switch shows it focused | RESIZE with its pane's grid, even when that grid was told already, so the core takes it back from any other viewer, then FOCUS(1). If the program has mode 2048 on, the in-band size report again after them, as AUTOMATIC_INPUT from herdr-frontend's terminal (a resize to a new grid sends it on its own). |
| The pane stops being in use (another pane, the window to the background) | FOCUS(0). |
| A pane stops being shown, or the window goes to the background | Nothing. It doesn't take the size again until it is shown in the front window again. |
| A click in the pane in use after another viewer moved its grid | RESIZE, which takes the size back. Herdr app does the same on a mouse-down. |
| Typing in the pane in use | DATA. The core applies the grid told last. |
| DATA from a pane that isn't shown in the front window (a drop or paste into a window in the background, a file landing after `terminal.put`) | RESIZE first, with its pane's grid, or with the core's grid when no shown pane draws it. That way the core never applies a stale grid. |
| The terminal engine answers the program's query | AUTOMATIC_INPUT when the stream confirmed it (`hello.features` and the attach reply). If the stream didn't, only a pane shown in the front window answers, as DATA, and only while no other viewer has moved its grid. Every other reply is dropped, because as DATA it would take the size. |

Every terminal is drawn at the core's grid, which the record's `cols` × `rows` keep current from
`size` events. The pane's own grid isn't used. When another viewer holds the size, a pane bigger
than the grid shows background around it, and a smaller pane clips it. What the pane's height
leaves under whole rows is background above the grid, so a grid at the pane's size has its
bottom row on the pane's bottom edge, and the rows of a grid at another size stay where they are
when the resize lands. Split panes are laid out in whole cells, so a grid that holds its pane's size fills it to
the divider. herdr-frontend takes the size only for panes it shows while its window is in front,
and takes it back from another viewer only on real use (focusing the pane, a click, typing) or
when the pane's grid changes.

The one exception is a grid that herdr-frontend itself makes the core apply: RESIZE, or DATA that
applies the grid it told. Its local terminal moves to that grid as the frame is sent, as Ghostty
resizes its terminal along with the pty. The program redraws for the new grid as soon as the core
applies it, and that output can arrive before the `size` event, which comes on another channel.
Parsed at the old grid, the redraw would leave the screen torn until the program next redrew it
(a Pi session's bottom rows after the sidebar closed, ticket 047). When the event for a grid it
took comes in, herdr-frontend recognizes it as its own. Only another viewer's size moves the
terminal after that.

Choices made on purpose:

- **No observers.** Hidden panes attach `resumed`, not `observe: true`. If herdr-frontend is a
  terminal's only viewer and its panes were observers, the terminal would drop to the headless
  200×120. Resumed viewers still size it: with no active viewer, the last attached wins.
  `resize_notifications` is honored only for observers, so herdr-frontend never asks for it and
  follows `size` events instead.
- **Shown panes hold their size, not only the focused one.** herdr-frontend is replacing Herdr
  app, so it no longer leaves the panes you aren't typing in at another viewer's size: after a
  split, the pane that shrank redraws at once, without a click. That is the reverse of the
  earlier rule, which took the size only for the pane in use, so that Herdr app showing the same
  terminals kept its sizes. With both apps in front of the same space, a split or a resize in
  herdr-frontend now takes the size of every pane it shows.
- **Reattaching doesn't take the size, being shown does.** After a reconnect every pane
  reattaches `resumed`. If the window is in front, its shown panes then take the size, as they
  would on any return to the window.
- **Herdr app's own takeovers can't be prevented.** It attaches every pane as an active viewer,
  refits hidden tabs about a second after its window settles, and reattaches without `resumed`
  after a core restart (`herdr-core-protocol.md` §4, items 1-3). herdr-frontend follows those
  sizes without fighting them, and takes the size back on its next real use: focusing the pane,
  a click, or typing.

## Two viewers and in-band size reports

A program that turns on in-band size reports (mode 2048; Neovim does whenever the terminal
supports it) ignores SIGWINCH from then on, and lays out at the last `CSI 48 ; rows ; cols ; … t`
report it hears. herdr-core strips its own screen's reports and leaves them to the viewers, and it
passes every non-observe viewer's AUTOMATIC_INPUT on to the program, the leader's or not. So when
Neovim turns the mode on, every viewer's terminal answers at that viewer's grid, and whichever
answer arrives last decides how Neovim lays out.

herdr-frontend's terminal is at the core's grid, so its answer matches the pty. Herdr app's is at
its own pane's grid, and for a pane in a tab it isn't showing, that's the grid the record had when
it attached. If Herdr app answers last, Neovim draws for a grid that is neither the pty's nor
herdr-frontend's: its rows overflow and wrap, windows land in the wrong places, the file panel
and statuslines go missing, and the core's own screen is just as wrong (ticket 059: Neovim's
DiffviewOpen in a fish pane Eli had opened in a new tab a moment earlier, with Herdr app open on
the same core; Herdr app had attached it at the core's headless 200×120). The Herdr iOS app next to Herdr app shows the same
thing for the same reason.

herdr-frontend can't keep another viewer from answering, so it makes the answers agree, and takes
the last word back when the pane is used:

- **A terminal starts at its pane's grid.** `terminal.run` carries the new pane's grid and the
  window's FACTS. A viewer that sizes a pane it doesn't show from the record attaches at that
  grid, so its answer matches, instead of 200×120.
- **Focusing a pane re-asserts the size.** RESIZE with the pane's grid and FOCUS(1), then, under
  mode 2048, herdr-frontend's own size report again, so Neovim lays out at herdr-frontend's grid whatever
  another viewer told it before. Herdr app's focus applies its grid too, and its mouse-down
  sends RESIZE. A program that only goes by SIGWINCH needs nothing more: the pty is at the
  leader's grid.

What's left, with Herdr app open:

- Herdr app never follows `size` events, so its copy of a terminal stays at the grid it attached
  or last refit it at. Once herdr-frontend changes that pane's grid (a split, the sidebar, the
  window, zoom), the two answers differ again. Neovim started in that pane while it's already
  focused is no focus change, so nothing re-asserts the size: if Herdr app answers last, Neovim
  lays out at Herdr app's stale grid.
- Herdr app refitting a terminal to its own window answers last the same way.

Either way Neovim keeps Herdr app's grid until the pane is focused again in herdr-frontend, or
its grid changes. Quitting Herdr app ends it.

## Tests

- `crates/herdr-client/tests/multi_viewer.rs`: two framed connections on an isolated core check
  the core's rules the policy depends on. Last active wins. A resumed attach takes nothing.
  FOCUS(1) applies the grid reported last. AUTOMATIC_INPUT and FOCUS(0) aren't activity. A
  RESIZE at an unchanged grid is. The leader leaving hands the size back, and no viewer left
  means 200×120. Resumed viewers alone go by the last attached. Only an observer gets RESIZE
  (before its first paint, and before each repaint, which starts with CAN).
- `crates/app/tests/multi_viewer.rs`: the app next to a second connection that plays Herdr app
  on an isolated core. Every pane shown in the front window takes its size, the unfocused one
  too, and takes it again when its grid changes. A pane in a hidden tab follows Herdr app's
  grid. A mode-2048 report from the app carries the core's grid, and as AUTOMATIC_INPUT it
  doesn't move the size. The app never takes the size back while idle or in the background. A
  click or typing takes it back. After a core restart, the app's `resumed` reattach leaves
  Herdr app's new grid alone.
- `crates/app/tests/two_viewers.rs`: the app next to a second connection that answers mode-2048
  queries at its own grid. A terminal the app opens in a new tab starts at its pane's grid, so a
  viewer attaching it at the record's grid right away gets the app's grid, not 200×120. After
  the other viewer's report came last, focusing the pane in the app makes the program's last
  report the app's grid again.
- `crates/app/tests/resize_redraw.rs`: a program that redraws its whole screen on SIGWINCH, in
  a shown pane, while the window grows and shrinks. After each resize the app shows exactly
  the core's screen at the core's grid.
- `crates/app/tests/shell.rs` and `input.rs` check the same policy frame by frame on the fake
  core.

## Checklist: both apps open

Run this by hand with Herdr app and herdr-frontend open on your usual core. Make a scratch space
first, so nothing you rely on is resized. In a scratch shell, `stty size` prints the PTY's
`rows cols`. `hc get <t> --json` shows the same as `cols` and `rows`.

1. **Same layout.** Both apps list the same spaces, tabs and panes. Creating, renaming or
   closing a tab in Herdr app shows up in herdr-frontend. The reverse holds too, for the
   commands herdr-frontend has.
2. **Shown panes take the size.** Put the two windows at different sizes. Bring herdr-frontend
   to the front with two scratch shells split side by side: `stty size` in each matches its
   herdr-frontend pane, the unfocused one too. Drag the divider, or split again: each shell's
   size follows its pane. Switch to another tab in herdr-frontend: the tab you left keeps its
   sizes until Herdr app or a return to that tab changes them.
3. **Herdr app takes it, herdr-frontend follows.** Click into the same shell in Herdr app and
   type. `stty size` now matches Herdr app's pane. herdr-frontend redraws the shell at that grid,
   letterboxed or clipped, without wrapping it wrong and without taking the size back.
4. **Real use takes it back.** Bring herdr-frontend to the front: every shown pane takes its size
   at once. Let Herdr app take it again, then click inside the herdr-frontend pane without
   typing. The size comes back on the click. Typing brings it back the same way.
5. **No size taken in the background.** With herdr-frontend in the background, use the shell in
   Herdr app, resize Herdr app's window, and switch its tabs. The size follows Herdr app only.
6. **Hidden tabs.** Open a tab in herdr-frontend, then switch away from it. Use that tab's
   terminals in Herdr app. Coming back to the tab in herdr-frontend is instant and draws at the
   current grid, and each of its panes then takes its size.
7. **Neovim.** Open `nvim` in a scratch pane and move between the apps a few times. Each time,
   the app you use redraws it full-size, with the statusline on the last row. The other app shows
   it at the core's grid, and nothing is left wrapped or torn.
8. **Queries.** Run something that asks the terminal for sizes or images, such as `nvim` with
   images, `yazi`, or `kitten icat`. With both apps showing it, the program behaves as it does
   in one app, and the size doesn't jump when it asks.
9. **Core restart (optional).** This restarts your real core. It is a planned restart, so
   programs keep running, but skip it while agents are in the middle of a turn. Run
   `hc server restart` with herdr-frontend in the background. Both apps reconnect. Herdr app takes the sizes, as it always
   does on reconnect. herdr-frontend reattaches without moving them, and takes the shown panes'
   sizes once you bring it to the front.
10. **View state stays per app.** Switching tabs, focus or zoom in herdr-frontend doesn't switch
    them in Herdr app, and switching them in Herdr app doesn't switch them in herdr-frontend.

If a step fails, note which app was in front, which pane was focused, and the `stty size` before
and after.
