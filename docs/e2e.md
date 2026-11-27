# The end-to-end harness

Component snapshots (`--snap`, `--tree`, headless tests) check pieces. Before Eli gets a build, the
harness runs the **whole app**: herdr-frontend in a real window, on a real herdr-core, driven
through scenarios that cover every area of `docs/research/feature-inventory.md`. It runs Herdr app
through the same scenarios and puts the two apps' screenshots side by side.

## Run it

```sh
just e2e                                  # every scenario, both apps
just e2e --only 03 --only peek            # scenarios whose file name contains 03 or peek
just e2e --app frontend                   # herdr-frontend only (or --app herdr-app)
just e2e --settle 2.5                     # give Herdr app 2.5 s per step (default 1.5)
just e2e --list                           # list the scenarios
```

`just e2e` is `HERDR_E2E=1 cargo test -p herdr-frontend --test e2e -- <args>`. Without
`HERDR_E2E`, `cargo test` only builds the harness and prints that it skipped, so CI and plain
`cargo test` never open windows.

It needs:

- `hc` (herdr-core), found as the tests find it: `HERDR_TEST_HC`, else on the usual paths.
- For the Herdr app side, `herdr-app` (`HERDR_E2E_HERDR_APP`, else on `PATH`, else
  `~/.local/bin/herdr-app`) and the config its home copies (`HERDR_E2E_APP_CONFIG`, else
  `~/.dotfiles/home/.config/herdr-app`). Without them it runs herdr-frontend alone and says so.
- Screen Recording permission for the terminal you run it from: herdr-frontend's screenshots are
  `screencapture -l <window>`.

A full run takes about ten minutes. Windows open and close on your display while it runs. Leave
them uncovered: macOS stops drawing a window that is hidden. A tiling window manager (AeroSpace)
may tile the Herdr app window, so its screenshots can be a different size from herdr-frontend's.

## What it touches, and what it never does

- **Its own cores only.** Each app run of each scenario gets a fresh herdr-core: `HC_SESSION=test-hfe<pid>-<n>`
  with scratch XDG directories under `/tmp/hfe-<pid>-<n>/`, every inherited `HC_*` variable
  removed. Both apps start with that environment. When the run ends, the harness stops that core with
  `hc server stop` and removes the scratch folder.
- **Its own homes only.** herdr-frontend gets `HERDR_FRONTEND_HOME=<scratch>/frontend`. Herdr app
  gets `HERDR_APP_HOME=<scratch>/herdr-app`, a copy of Eli's config without his running
  window's socket, `view.json`, `window.json` and `seen.json`, and without what his use left
  behind: the mods' stores (`data/`) and `shared.json`, which starts as his tracked
  `shared.defaults.json` (as a fresh `dot stow` leaves it). So neither app starts with his hidden
  machines, his agents' times or his recent commands. The source config is only read.
- **Not Eli's core.** Once an app opens, the harness waits until it lists exactly the seeded spaces. An app that lists any
  other space is not on the isolated core: the harness kills it at once and fails the scenario.
- **Not Eli's Herdr app.** The harness talks only to its own copy, through `herdr-app ls | do | status | shot` with
  `HERDR_APP_HOME` set to the copy's home. It quits that copy with `app.quit` and kills only the
  processes it started. The copy's update checks and telemetry go to a closed local port
  (`HERDR_RELEASES_URL`, `HERDR_TELEMETRY_URL`), so it never downloads an update next to Eli's install.
- **No OS-level input.** Keys reach herdr-frontend through its control socket and Rune's
  `Tx::send_key`, which delivers them as typed keys. Herdr app has no way to take keys from a script. Steps
  drive it through `herdr-app do`, the equivalent command for each key.

## How a run goes

For each scenario, and for each app in turn:

1. Start an isolated core and seed it (below).
2. Open the app's window on that core with an isolated home, and wait for it to list the seeded
   spaces.
3. Capture the start, then run each step and capture after it.
4. Quit the app and stop the core.

Then write the report.

A capture first waits for the window to come to rest. herdr-frontend is at rest once the element tree of its
drawn frames has stayed the same for half a second (up to 6 s). Herdr app gets a fixed `--settle`.
Then the harness saves:

- the whole window as a PNG: `screencapture -x -o -l <window id>` for herdr-frontend, and Herdr
  app's own `herdr-app shot`;
- `status` and `ls` from the app's CLI (`*.state.json`);
- herdr-frontend's element tree (`*.tree.json`), from the frame it last drew.

## Output

```text
target/e2e/<YYYYMMDD-HHMMSS>/
  index.html                 every scenario, each step's two screenshots side by side
  run.json                   the same as data: what each step did, how each app answered
  logs/<scenario>.<app>.log  the apps' stdout and stderr
  <scenario>/NN-<step>.frontend.png   .frontend.state.json   .frontend.tree.json
  <scenario>/NN-<step>.herdr-app.png  .herdr-app.state.json
```

The harness rewrites the report after each scenario, so a run stopped halfway still has one. A
red action in the report means the app refused the step (a command it doesn't serve yet). That is
expected while features are missing, and it is what the parity audit (ticket 033) works through.
The harness exits 1 only when a run itself failed: an app didn't start, didn't show the seed,
wasn't on the isolated core, exited during the scenario, or didn't open again after a `restart`.
The scenario stops there, and the report keeps the steps captured before it.

## The seed

Every scenario starts from the same world (`crates/app/tests/e2e/seed.rs`). Another client
writes it to the core before either app opens:

| Space | Tab | Panes |
| --- | --- | --- |
| `work` (named) | `work-1` | shells `w1` \| (`w2` / `w3`) |
| | `work-2` | agent `pi`: working, `app=pi`, title `π - fix-login - work` |
| | `work-3` | agent `done`: working, then done, `app=claude` |
| `api` (unnamed, after its folder) | `api-1`, named `server` | shell `a1` |
| | `api-2` | agent `blocked`: blocked (permission), `app=claude` |
| `notes` (named) | `notes-1` | shell `n1` |
| | `notes-2` | agent `idle`: idle, `app=pi` |

The `explore` sub-agent (`parent` = `pi`, working, `app=claude`) has no pane. Folders are
`<scratch>/code/<space>`.

Fake agents are `sh` scripts that report OSC 7501 status the way Pi and Claude do. Each reads
its status from a control file every 200 ms and reports it when it changes, so a step can move
an agent to a new status without typing into it.

Ids come from names and are the same on every core. Scenarios write `{{sp:work}}`,
`{{tb:work-2}}`, `{{p:w2}}`, `{{s:work-1}}` (a split), `{{t:pi}}` (a terminal's address, which
differs per core) and `{{folder:notes}}`. An id for something a step makes later
(`{{sp:worker-space}}`) resolves the same way.

## Scenarios

`crates/app/tests/e2e/scenarios/NN-<area>.json`, one per area of the feature inventory (§0–§19).
When a feature ticket adds behavior, it adds steps to its area's file, or a new file:

```json
{
  "title": "Spaces",
  "inventory": "§3",
  "notes": "Shown above the steps in the report.",
  "steps": [
    {"name": "select space 2", "do": "space.select", "args": {"index": 2}},
    {"name": "space picker", "keys": "ctrl-g s",
     "herdr-app": {"do": "mod.open", "args": {"name": "space-picker"}}},
    {"name": "type in it", "type": "api", "herdr-app": {"skip": "typing goes to herdr-frontend only"}},
    {"name": "pi finishes", "status": {"agent": "pi", "to": "state=done:app=pi"}, "wait": 0.8}
  ]
}
```

A step does these in order, then waits `wait` more seconds and captures:

1. Changes the core both apps see:
   - `status {agent, to}` moves a fake agent to another OSC 7501 status;
   - `adopt {name, space, tab, status}` starts an agent labeled `herdr.space`/`herdr.tab`, as
     `spawn-worker` does;
   - `send {terminal, text}` runs `hc send`.
2. `restart: true` quits the app and opens it again on the same home and core.
3. Drives the app:
   - `do` with `args` runs through each app's CLI (`herdr-frontend do` / `herdr-app do`);
   - `keys` (`keys.json` syntax, `ctrl-g s`) and `type` (text, `\n` is Return) go to
     herdr-frontend;
   - `herdr-app` replaces what Herdr app does: `{"do", "args"}` with the command its key runs in
     Eli's `keys.json`, or `{"skip": "why"}`.

## herdr-frontend's harness requests

A herdr-frontend window started with `HERDR_FRONTEND_E2E=1` answers four more requests on its
control socket (`docs/cli.md` "Transport"), one JSON line each way (`crates/app/src/e2e.rs`). A
window started without it refuses them like any unknown request, so Eli's own window takes no
keys from scripts.

| Request | Answer |
| --- | --- |
| `{"keys": "ctrl-g s"}` | `{"ok": true}`. The chords, in order, as typed keys (Cmd chords as macOS sends them) |
| `{"type": "echo hi\n"}` | `{"ok": true}`. The text, one key per character |
| `{"tree": true}` | The element tree of the last frame the window drew: `{frame, viewport, scale, focused, elements}`, each element as `--tree` writes it (`path`, `key`, `role`, `label`, `text`, `rect`, colors, `focused`) |
| `{"window": true}` | `{"window": <CGWindowID or null>, "pid": <pid>}` |

The tree comes from `App::on_frame_drawn`, so a headless run (no drawn frames) answers
`{"error": "the window has drawn no frame yet"}`.

Tests: `crates/app/tests/harness_probes.rs` runs the app headless on an isolated core and drives
these requests, and `crates/app/src/e2e.rs` tests the key parsing.
