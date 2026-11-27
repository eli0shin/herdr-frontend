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
| A pane appears, in any tab, and again after a dropped connection or a restarted program | `terminal.attach` with `resumed: true` and the core's current grid. It takes nothing. |
| A pane becomes the pane in use: focused, with the window in front | FACTS (if the cell or colors changed), then RESIZE with the pane's grid, then FOCUS(1). |
| The pane in use changes size (window, divider, zoom, font) | RESIZE, at most every 25 ms, always ending with the final grid. |
| The pane stops being in use (another pane, the window to the background) | FOCUS(0). It doesn't take the size again until it is used again. |
| A click in the pane in use after another viewer moved its grid | RESIZE, which takes the size back. Herdr app does the same on a mouse-down. |
| Typing in the pane in use | DATA. The core applies the grid told last. |
| DATA from a pane that isn't in use (a drop or paste into a window in the background, a file landing after `terminal.put`) | RESIZE first, with its pane's grid, or with the core's grid when no shown pane draws it. That way the core never applies a stale grid. |
| The terminal engine answers the program's query | AUTOMATIC_INPUT when the stream confirmed it (`hello.features` and the attach reply). If the stream didn't, only the pane in use answers, as DATA. Other panes drop the reply, because as DATA it would take the size. |

Every terminal is drawn at the core's grid, which the record's `cols` × `rows` keep current from
`size` events. The pane's own grid isn't used. When another viewer holds the size, a pane bigger
than the grid shows background around it, and a smaller pane clips it. Nothing in herdr-frontend
takes the size back without real use.

Choices made on purpose:

- **No observers.** Hidden panes attach `resumed`, not `observe: true`. If herdr-frontend is a
  terminal's only viewer and its panes were observers, the terminal would drop to the headless
  200×120. Resumed viewers still size it: with no active viewer, the last attached wins.
  `resize_notifications` is honored only for observers, so herdr-frontend never asks for it and
  follows `size` events instead.
- **Reattaching doesn't take the size, being in use does.** After a reconnect every pane
  reattaches `resumed`. If the window is in front, its focused pane then counts as in use and
  takes the size, as it would on any return to the window.
- **Herdr app's own takeovers can't be prevented.** It attaches every pane as an active viewer,
  refits hidden tabs about a second after its window settles, and reattaches without `resumed`
  after a core restart (`herdr-core-protocol.md` §4, items 1-3). herdr-frontend follows those
  sizes without fighting them, and takes the size back on its next real use.

## Tests

- `crates/herdr-client/tests/multi_viewer.rs`: two framed connections on an isolated core check
  the core's rules the policy depends on. Last active wins. A resumed attach takes nothing.
  FOCUS(1) applies the grid reported last. AUTOMATIC_INPUT and FOCUS(0) aren't activity. A
  RESIZE at an unchanged grid is. The leader leaving hands the size back, and no viewer left
  means 200×120. Resumed viewers alone go by the last attached. Only an observer gets RESIZE
  (before its first paint, and before each repaint, which starts with CAN).
- `crates/app/tests/multi_viewer.rs`: the app next to a second connection that plays Herdr app
  on an isolated core. Only the pane in use takes the size. The unfocused pane follows Herdr
  app's grid. A mode-2048 report from the app carries the core's grid, and as AUTOMATIC_INPUT it
  doesn't move the size. The app never takes the size back while idle or in the background. A
  click or typing takes it back. After a core restart, the app's `resumed` reattach leaves
  Herdr app's new grid alone.
- `crates/app/tests/shell.rs` and `input.rs` check the same policy frame by frame on the fake
  core.

## Checklist: both apps open

Run this by hand with Herdr app and herdr-frontend open on your usual core. Make a scratch space
first, so nothing you rely on is resized. In a scratch shell, `stty size` prints the PTY's
`rows cols`. `hc get <t> --json` shows the same as `cols` and `rows`.

1. **Same layout.** Both apps list the same spaces, tabs and panes. Creating, renaming or
   closing a tab in Herdr app shows up in herdr-frontend. The reverse holds too, for the
   commands herdr-frontend has.
2. **Only the pane in use takes the size.** Put the two windows at different sizes. Focus a
   scratch shell in herdr-frontend: `stty size` there matches herdr-frontend's pane. Switch
   panes in herdr-frontend: the pane you left keeps its size until Herdr app or a new focus
   changes it.
3. **Herdr app takes it, herdr-frontend follows.** Click into the same shell in Herdr app and
   type. `stty size` now matches Herdr app's pane. herdr-frontend redraws the shell at that grid,
   letterboxed or clipped, without wrapping it wrong and without taking the size back.
4. **Real use takes it back.** Bring herdr-frontend to the front: the focused pane takes its size
   at once. Let Herdr app take it again, then click inside the herdr-frontend pane without
   typing. The size comes back on the click. Typing brings it back the same way.
5. **No size taken in the background.** With herdr-frontend in the background, use the shell in
   Herdr app, resize Herdr app's window, and switch its tabs. The size follows Herdr app only.
6. **Hidden tabs.** Open a tab in herdr-frontend, then switch away from it. Use that tab's
   terminals in Herdr app. Coming back to the tab in herdr-frontend is instant and draws at the
   current grid. Only its focused pane takes the size.
7. **Neovim.** Open `nvim` in a scratch pane and move between the apps a few times. Each time,
   the app you use redraws it full-size, with the statusline on the last row. The other app shows
   it at the core's grid, and nothing is left wrapped or torn.
8. **Queries.** Run something that asks the terminal for sizes or images, such as `nvim` with
   images, `yazi`, or `kitten icat`. With both apps showing it, the program behaves as it does
   in one app, and the size doesn't jump when it asks.
9. **Core restart (optional).** This restarts your real core. It is a planned restart, so
   programs keep running, but skip it while agents are in the middle of a turn. Run
   `hc server restart` with herdr-frontend in the background. Both apps reconnect. Herdr app takes the sizes, as it always
   does on reconnect. herdr-frontend reattaches without moving them, and takes the focused pane's
   size once you bring it to the front.
10. **View state stays per app.** Switching tabs, focus or zoom in herdr-frontend doesn't switch
    them in Herdr app, and switching them in Herdr app doesn't switch them in herdr-frontend.

If a step fails, note which app was in front, which pane was focused, and the `stty size` before
and after.
