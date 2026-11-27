# Machines

Other machines, as `docs/research/feature-inventory.md` §9 lists them and
`docs/research/herdr-core-protocol.md` §1 ("Remote machines") describes the protocol. Machines
are added to the local core (`hc machine add`), never dialed by herdr-frontend: their terminals
come in the same snapshot and events as `devbox/t4`, and their layouts are each machine's own
`herdr-app` document.

The rules are pure model code (`crates/model/src/machines.rs`, the machine parts of
`crates/model/src/world.rs`); the window draws them (`crates/app/src/view/machines.rs`) and makes
the calls (`crates/app/src/machines.rs`, `Link::machine`). Tests: `crates/model/src/machines.rs`,
`crates/app/tests/machines.rs` (headless, on `fixtures::machines_core`) and
`crates/app/tests/remote_machine.rs` (real isolated cores, below). Scenes: `shell-machine-filter`,
`shell-machines`, `shell-machine-add`.

## Spaces and terminals on other machines

- `machine.list` is polled every 2.5 s, at once on each connection, after terminals come or go,
  and right after every machine call (herdr-client's `MachineWatcher`). Temporary machines
  (agents' sandboxes) are left out, with their spaces.
- Each machine that is connected has its layout read from its own `herdr-app` document, through
  its own `Mirror` (`doc.*` with `on`), and written back the same way under compare-and-set. A
  machine's document holds bare ids; herdr-frontend qualifies them (`devbox/sp…`, `devbox/t4`)
  and spaces show this computer's first, then each machine's by name. A machine that is down
  keeps its spaces as last read; edits to it are refused with "This machine's layout is
  unavailable".
- A split, a new tab and "new space on <machine>" start their terminal on that machine.
- Where it shows: the tab bar's end names the shown space's machine (a server and its name, or
  a monitor and "This computer"); an agent's row says its machine on line 2; the space picker
  lists it; the tab and agent picker offers "new space on <machine>" per connected machine.
- A record's `parent` may name a machine by that core's host name or core id, or name this
  machine itself (§5 "Parents and children"). It is normalized before sub-agents are matched to
  their parents: this core's id or name (from its `hello`) means local; otherwise the machine
  with that name, then that id, then the only one with that host.

## The machine filter (`ctrl-g /`)

Eli's `agent-machines` mod. It opens from `ctrl-g /`, the palette's "Machine filter", or the
Agents header's monitor button (which reads "N hidden" in the working blue while machines are
hidden).

- 340 wide, 40 from the top, at most half the window. "Agents from", then "All machines", this
  computer, the machines `machine.list` has, then any machine known only from an agent or the
  hidden list.
- Each row: a checkbox (its agents show), the name, "offline" when it isn't connected or isn't
  added, the agent count (every agent in a pane, hidden ones too) and "⏎ only" on the cursor's
  row.
- ↑/↓ or J/K move (wrapping). Space or a click shows or hides the machine and keeps the filter
  open. Enter shows only that machine and closes; on "All machines" it shows every machine.
  Esc or Q closes.
- The choice is shared `dotfiles.agents.hidden-machines` in herdr-frontend's `shared.json`: the
  hidden keys (`""` is this computer), `null` when nothing is hidden, so a machine connected later
  shows until it's hidden.
- Agents on hidden machines leave the Agents list, its numbers and jump labels; they are still
  tracked, so their times hold. "Every agent is on a hidden machine…" when nothing is left.

## Machines (status, rename, remove)

Herdr app's Settings → Machines, as a 520-wide dialog. It opens from the palette's "Machines" or
the settings dialog's Machines row.

- This computer: "Connected · hc <version>" from the core's `hello`.
- Each machine: a dot (connected, connecting, or not working), its name, then how it is reached,
  its state and its hc version (`devbox · needs login · hc 0.1.22`), the core's `why` when it
  isn't connected, and what it needs run:
  - a login: `hc machine login <name>`;
  - a core update: `hc machine update <name>`;
  - on a server that ends programs at logout (`keeps_running_after_logout: false`):
    `sudo loginctl enable-linger -- <user>`, run there once.
- "Add a machine" opens the add dialog.
- Enter or a click on a machine opens its menu: "Connect now" while it isn't connected
  (`machine.connect`), "Copy <command>" for each command it needs, Rename (the prompt;
  `machine.rename`; addresses with the old name stop working) and Remove (asks first;
  `machine.rm`; its terminals keep running there). Connecting and copying come back to the
  dialog, on that machine.
- Each call says how it went in a toast ("Renamed omarchy to arch", "Couldn't connect to
  devbox: <why>"), except a connect that worked: the dialog shows the machine's new line.

## Adding a machine

The shipped `machine-add` mod, as a 560-wide dialog.

- The field takes how ssh reaches the machine (`vps`, `can@10.0.0.7`, `can@10.0.0.7:2222`) and
  filters the hosts of `~/.ssh/config`, read off the UI thread each time the dialog opens, with
  the files its `Include`s name (`~` is home, a relative path is under `~/.ssh`, `*` and `?`
  match file names; patterns aren't hosts). Hosts already added (by target, name or host) are
  left out. What's typed is offered as "Add <target>" unless a host goes by that name.
- Enter (or a click) adds the row's machine (`machine.add`, up to 2 minutes for ssh). The row
  says "Connecting…"; once added, a toast says so, the host leaves the list and a typed target
  empties the field. The dialog stays open.
- A failure shows under its row. When the machine needs answers only a terminal can take (a
  password, a new host key, herdr-core installed there), it says to run `hc machine add <target>`
  in a terminal, and Enter on that row copies the command. Failures are forgotten when the
  dialog closes, so opening it again tries again.

## Tests with a fake second machine

`crates/app/tests/remote_machine.rs` runs on two isolated cores (`test-*` sessions with scratch
dirs, `docs/plan.md` "Testing rules"). The second is added to the first as `devbox` with
`hc machine add --via '<env …> sh -c' --name devbox` (`IsolatedCore::via`): like ssh, the via
command runs the line it is given, here under the second core's session on this computer. The
app then reads devbox's own document, draws and types into its terminal, counts its agent in
the filter and hides it, shows its line and hc version in the Machines dialog, connects to it,
fails to add an unreachable target, renames it (its space follows) and forgets it.

## Where it differs from Herdr app

- The filter opens in the same place from the header button as from `ctrl-g /`; Herdr app
  opens a 280 × 260 popup from the button.
- Machines are listed by name, the order their spaces use.
- Machine management is a dialog of its own (herdr-frontend has no 880 pt settings window), and
  its actions are a menu instead of buttons on each row. "Update" (copying this hc there) isn't
  offered: the dialog gives `hc machine update <name>` to run when a machine needs it.
- The add dialog doesn't list the machines already added (the Machines dialog does), and the
  one-time macOS card ("allow its folders once", Full Disk Access on an added Mac) isn't built.
