# Attention and in-app toasts

Agent status, seen handling and toasts (`docs/research/feature-inventory.md` §8, Eli's
`eli-toast` mod; `docs/research/herdr-core-protocol.md` §3 "Labels" and §5). Attention shows in
the app only: the sidebar, the tab bar's marks and in-app toasts. herdr-frontend never posts a
native notification or plays a sound.

The rules are pure logic: `crates/herdr-shared/src/attention.rs` (`Attention`, `alert`) and
`labels.rs` (`SeenTracker`). `crates/model/src/model.rs` runs them on every step, and the app
draws toasts in `crates/app/src/toast.rs`. `crates/model/tests/attention.rs` and
`crates/app/tests/attention.rs` test them headless, and the `shell-alerts` scene shows two alert
toasts (`just snap shell-alerts`).

## Status and markers

An agent shows as blocked (blocked or error), working, done, or idle. Done means it finished
and no client has looked since (`turn > herdr.seen`). Without the label, this screen's own memory
of whether it looked decides, saved in the view. Tabs roll their terminals up by blocked > done >
working > idle. The sidebar draws the marks (`docs/sidebar.md`). A tab pill shows a spinner while
working and a dot when blocked or done.

## Seen, shared through `herdr.seen`

After every step, the model looks at the agents drawn on screen while the window is in the
foreground. It writes `herdr.seen=<turn>` for each one with `terminal.label`, the way Herdr app's
`share_seen` does:

- The write happens once per turn while the agent stays on screen, and only when it raises the
  label (or the label is missing). So when another client marks an agent unread, a screen that is
  just sitting on that agent doesn't undo it. Looking away and back marks it seen again.
- The agent shows as seen at once, without waiting for the core's echo. A write the core hasn't
  echoed yet still counts, using the per-batch `herdr.seen` highs (`TerminalUpdate::seen`).
- A failed write is forgotten and tried again after the core's next news, not in the step that
  failed, so a write that keeps failing (a machine that is down) can't loop. A new connection
  starts over.
- When any client writes the label, a done agent turns idle here too.

## Alerts

An agent in a pane alerts when it becomes blocked or errors, and when it finishes while the
person isn't looking. Sub-agents never alert, and nothing alerts on the first listing. An alert
always brings its settled tab back. It shows a toast only when the person isn't looking at that
agent (`Model::alert_toast`):

- Title: "<name> needs you" (error) or "<name> finished" (info). The name is the terminal's own
  `name`, else the program table's name for its status's `app` ("Claude Code", "Pi"), else "An
  agent".
- Body: "<space> / <tab>".
- Click: `Action::FocusAgent` on the agent.

## Toasts

- Cards are 340 × 72 pt at the window's bottom right, 16 pt in, with 8 pt gaps. As in Herdr app,
  the oldest is at the bottom and newer ones stack above it. At most four show at a time; the
  oldest goes first.
- Each card has a level dot (error is `blocked`, warning `working`, success `done`, info
  `accent`), a 13 pt semibold title and a 12 pt muted body, one line each. It uses the popover
  color, takes the hover color under the pointer, and has an 8 pt radius and a border.
- A toast goes after 5 s, or 8 s for errors and warnings.
- Clicking one dismisses it and runs its action.
- A toast whose click goes to a terminal on a machine in `dotfiles.agents.hidden-machines` closes
  at once (`eli-toast/filter.js`), so an agent the Agents list hides stays out of view.

## Other toasts

- **Terminal failed to start** (error): a terminal this window started exited on its own with a
  non-zero code within 2 s of starting (`app:src/window.rs` `launch_failure`). The body is "Exit
  code N" and the last five lines it wrote that aren't blank (`terminal.read`), joined with " · "
  on one line, where Herdr app breaks lines. Its tab or pane goes, as any ended program's does.
- **Couldn't start a terminal**: `terminal.run` itself failed.
- **<machine> needs a login: run** (error): a new space, tab or split on a machine that needs a
  login failed (Herdr app's failed New space toast). As `eli-toast` draws it, the body is
  `hc machine login <machine>` with a Copy button beside it; Copy puts the command on the
  clipboard, reads "Copied" and leaves the card up.
- **Pane navigation failed**: Ctrl+H/J/K/L couldn't run `nvim` (`docs/keys.md`).
- **Layout unavailable** / **Layout changed elsewhere**: an edit a machine's layout refused, or
  edits another screen's write dropped.
- **Not yet**: a command whose feature isn't built.

## Not here yet

- No toast sets its own timeout; Herdr app's 12 s notes have no counterpart here.
