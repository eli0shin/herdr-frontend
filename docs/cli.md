# herdr-frontend's control CLI: `ls | do | status`

Eli's tools drive the Herdr app window with `herdr-app ls | do | status`
(`docs/research/feature-inventory.md` §0 "CLI surface other tools drive"). herdr-frontend serves
the same subcommands from its own binary, for the commands those tools use. A tool reads which
app to drive from one file ([Frontend selection](#frontend-selection)), then calls `herdr-frontend`
where it calls `herdr-app`, with the same arguments, and reads the same JSON and exit codes.

## Frontend selection

The contract every tool follows. There is no shim: no wrapper binary or dispatch layer sits
between a tool and the apps. Each tool reads the file itself and calls the app it names.

- **File:** `${XDG_CONFIG_HOME:-$HOME/.config}/herdr/frontend`, so `~/.config/herdr/frontend`
  unless `XDG_CONFIG_HOME` is set. An empty `XDG_CONFIG_HOME` counts as unset, as in herdr-core's
  rule for its config folder. Tests select an app through a scratch `XDG_CONFIG_HOME`, never the
  real file.
- **Contents:** one word, `herdr-app` or `herdr-frontend`. Whitespace around it, including the
  trailing newline, is ignored.
- **Default:** a missing or unreadable file means `herdr-app`, which is how the tools behave
  today. Any other contents also mean `herdr-app`.
- **When:** a tool reads the file each time it runs (each call for a long-running one such as Pi's
  extension), so a switch applies to the next call without restarting anything.
- **Switching** is one edit:

  ```sh
  mkdir -p ~/.config/herdr                         # the first time
  echo herdr-frontend > ~/.config/herdr/frontend   # drive herdr-frontend
  echo herdr-app > ~/.config/herdr/frontend        # back to Herdr app (or delete the file)
  ```

What changes with `herdr-frontend` selected:

| | `herdr-app` | `herdr-frontend` |
| --- | --- | --- |
| Command | `herdr-app` | `herdr-frontend` |
| Home variable | `HERDR_APP_HOME` | `HERDR_FRONTEND_HOME` ([Transport](#transport)) |
| Socket | `<home>/herdr-app.sock` | `<state folder>/herdr-frontend.sock` ([Transport](#transport)) |

A tool never builds the socket path itself: it runs `herdr-frontend ls | do | status`, and the
binary finds the socket.

The subcommands, arguments, JSON fields, exit codes, the `no Herdr window at` message and the bare
launch that brings the window forward stay the same. Work herdr-core does itself (starting a
terminal with `hc run`, the `herdr.space` and `herdr.tab` labels that place it, `hc send`,
`hc get`) goes to `hc` directly, whichever app is selected.

### What each tool calls

Every call below is in `crates/app/tests/tool_calls.rs`, made the way the tool makes it.

| Tool | Calls |
| --- | --- |
| `repos` (`src/workspace-manager/herdr-app-adapter.ts`) | `ls`; `do <command> '<json>'` for `space.new {name, folder, focus: false}` (and without `focus`), `space.rename {id, name}`, `space.select {id}`, `space.close {id, confirmed: true}`; the bare command to bring the window forward; `/no Herdr window/` on stderr |
| `spawn-worker` | `ls`; `hc run --bg --keep --no-parent --cwd <worktree> --label herdr.space=<new or found id> --label herdr.tab=<new id>`, then `ls` until the space appears; `do space.rename --id <space> --name <worker>` |
| `agent-status` skill (`scripts/agent-status`, `ACTIONS.md`) | `ls` (spaces' `id`, `name`, `machine`; tabs' `id`, `name`; terminals' `terminal`, `pane`, `app`, `status`); `do agent.focus --terminal <t>`; `do pane.close --pane <p>`; resume with `hc run … --label herdr.space=<space> --label herdr.tab=<new id>`, then `do tab.rename --id <tab> --name <label>` |
| Pi `auto-session-name.ts` | `ls` (the tab holding `HC_TERMINAL_ID`, its first terminal with `app: "pi"`); `do tab.rename '{"id", "name"}'` |
| code-review-tui `review` (`src/tools/herdr-adapter.ts`) | `ls` (the local space holding `HC_TERMINAL_ID`); `hc run --bg --keep --no-parent --json --cwd <folder> --label herdr.space=<space> --label herdr.tab=<new id>`, then `ls` until that tab shows the terminal; `do tab.rename '{"id", "name"}'`; `do agent.focus '{"terminal"}'`. The tab's program ends with `do agent.focus '{"terminal": <Review Queue>}'`, and the tab leaves when its program exits |
| `dot stow` (`configure-herdr-app.sh`) | `status`, with `no Herdr window at ` on stderr when no window is open; `do layout.switch --name eli` |

`dot stow` also runs `navigator-status.set-shared:<json>`, a command of Eli's navigator-status
mod. herdr-frontend has no mods and doesn't serve it: it fails as an unknown command. The script
configures Herdr app's own home (its `shared.json`), which herdr-frontend never reads;
herdr-frontend's settings are its own files (`docs/settings.md`).

Code: `crates/app/src/control.rs` (socket and client) and `crates/model/src/control.rs` (commands,
`ls` and `status`).

## Transport

- **State folder:** `$HERDR_FRONTEND_HOME`, else `$XDG_STATE_HOME/herdr-frontend`, else
  `~/.local/state/herdr-frontend`; empty variables count as unset. The socket is state, not
  config, so it lives here and not beside `settings.json` (`docs/settings.md` "Config and
  state"). herdr-app keeps its socket in its home (`$HERDR_APP_HOME`, `$XDG_CONFIG_HOME/herdr-app`,
  `~/.config/herdr-app`; `src/mods.rs` `app_dir`). The window and the CLI use the same rule
  (`control::state_home`), so they agree on the socket. `$HERDR_FRONTEND_HOME` is one folder for
  config and state both, which tests use to isolate a window.
- **Socket:** `<state folder>/herdr-frontend.sock`. The window serves it. When nothing answers
  there, the CLI and a bare launch try `<config folder>/herdr-frontend.sock`
  (`$XDG_CONFIG_HOME/herdr-frontend`, else `~/.config/herdr-frontend`; `control::socket_paths`),
  where a window of 0.1.1 or older keeps serving until it's quit. Without that, right after an
  install, tools would find no window, and a bare launch would open a second one. With no
  window on either, the `no Herdr window at` message names the state folder's socket. One window serves a home: a
  second one finds the live socket and doesn't take it. A socket that nobody answers on is left
  from a window that died, and the next window replaces it. A window removes its socket when it
  closes.
- **Wire:** one JSON line each way, as `herdr-app.sock` uses (herdr-app `src/control.rs`).
  Requests are `{"ls": true}`, `{"status": true}`, `{"commands": true}`, `{"raise": true}` or
  `{"do": "<command>", "args": <object or null>}`. The reply is the JSON the CLI prints. The window
  answers within 5 s or the reply is `{"error": "the window did not answer"}`.
- A window the end-to-end harness started (`HERDR_FRONTEND_E2E=1`) also answers `keys`, `type`,
  `tree` and `window` (`docs/e2e.md` "herdr-frontend's harness requests"). Any other window
  refuses them as unknown requests. The CLI has no subcommand for them.
- The window runs every request on its UI thread, through the model (`Model::ls`,
  `Model::status`, and `Model::step` with a `Call` for `do`), so a command changes state exactly as
  the same key or click would.

## Subcommands

```sh
herdr-frontend ls                            # spaces > tabs > terminals, with ids
herdr-frontend status                        # what the window shows, and layout errors
herdr-frontend commands                      # the served commands and their arguments
herdr-frontend do <command> --arg value ...  # or: herdr-frontend do <command> '<json object>'
herdr-frontend                               # bring the window forward, or open one
herdr-frontend --version                     # the released version (docs/release.md)
```

- `herdr-frontend` with no arguments, while a window serves its home, brings that window forward
  (restored if minimized, the app activated) and exits 0 without opening another, as herdr-app's
  bare launch does. It sends `{"raise": true}`; the window answers `{"ok": true}`. With no window
  on the home, it opens one.
- `do` takes `--key value` pairs or one JSON object. A value that parses as JSON (and isn't a
  JSON string) is that JSON: `--focus false` is the boolean, `--index 3` the number and
  `--argv '["pi"]'` the array. Other values are strings. This is herdr-app's `parse_args`.
- The reply is printed to stdout as pretty JSON. Object key order isn't part of the contract.
- `commands` prints the list without a window.
- `--version` prints `herdr-frontend <version>` and exits 0, without a window or a core.

### Exit codes and errors

| Code | When | Output |
| --- | --- | --- |
| 0 | The reply has no `error` | the reply on stdout |
| 1 | The reply is `{"error": "…"}` | the reply on stdout |
| 2 | Usage error, or no window | `herdr-frontend: <message>` on stderr |

With no window, the message is `herdr-frontend: no Herdr window at <socket>: <os error>`. It keeps
herdr-app's `no Herdr window at` phrase, which `repos` (`/no Herdr window/`) and `dot stow`
(`*"no Herdr window at "*`) match.

## `ls`

herdr-app's shape (herdr-app `src/window.rs` `world`):

```json
{
  "spaces": [
    {
      "id": "sp1", "name": "work", "folder": "/Users/eli/code/x", "machine": null, "active": true,
      "tabs": [
        {
          "id": "tb1", "name": "1", "active": true, "settled": false,
          "terminals": [
            {"terminal": "t2", "pane": "p2", "status": "idle", "app": "pi", "name": null,
             "sub_agents": ["t5"]}
          ]
        }
      ]
    }
  ],
  "recent": [{"name": "old", "folder": "/Users/eli/code/old"}]
}
```

- Spaces come in herdr-frontend's order: this machine's, then each other machine's by name.
  Another machine's ids carry its prefix (`devbox/sp9`, `devbox/t2`), as in herdr-app, and
  `machine` names it (`null` here).
- `name` is what the window shows. An unnamed space is called after its first terminal's git
  checkout (a worktree's own folder), else that terminal's folder, and an unnamed tab by its
  number.
- `active`: the shown space, and each space's selected tab. `settled`: the tab is put away.
- `status` is the displayed agent status (`idle`, `working`, `done`, `blocked`), `null` for a
  terminal that isn't an agent. `app` is the program its status names. `name` is the core's
  terminal name.
- `sub_agents`: live terminals whose `parent` is this one.
- `recent`: closed spaces, most recent first.

## `status`

```json
{
  "space": "sp1", "tab": "tb1", "focused_pane": "p2", "layout": "eli", "agent": "t2",
  "floats": {}, "overlays": [], "errors": ["layout devbox: <why>"],
  "zoom": 1.1, "font_size": 17.0, "ui_font_size": 13.0
}
```

- `space`, `tab`, `focused_pane`: what is shown and focused.
- `layout` is always `"eli"`, the only layout herdr-frontend has.
- `agent`: the focused terminal when it's an agent, else the first agent in the agent order.
- `floats` is always `{}`.
- `overlays`: the open popup (`picker`, `palette`, …, `agent-jump`), `prompt`, `confirm` or
  `copy-mode`.
- `errors`: the window's own errors (settings, keys) first, then `layout <machine|local>: <why>`
  for each machine whose layout can't be read or written.
- `zoom`, `font_size` and `ui_font_size` come from the window, not the model
  (`State::window_status`, with the settings files' errors; `docs/settings.md`).

## `do` commands

Arguments follow herdr-app's rules (`src/commands.rs` `validate_args`, `missing`):

- An unknown argument fails: `<command>: unknown argument(s): <names>; valid arguments: <list>`.
- Target ids (`id`, `pane`, `space`, `terminal`, `target`) must be strings:
  `<command>: argument <key> must be a string (or null to omit it)`.
- `null` is the same as leaving the argument out. Arguments that aren't an object fail.
- A required argument that is missing, null or blank fails: `<command> needs <arg>`.
- `name:value` works as in key files: the value fills the first argument, and a number fills
  `index` (`space.select:2`).
- An unknown command fails:
  ``unknown command "<name>": see `herdr-frontend commands` for the ones it serves``.
- A script is never asked anything. A rename without a name, or a close without `confirmed`,
  fails instead of opening a dialog, as herdr-app does for commands from `do`.
- Edits need the core connection and the target machine's layout. Without them the command fails
  (`herdr-frontend is not connected to herdr-core`, or the layout's error) before it changes
  anything.

Success is `{"ok": true}` unless a command below says otherwise.

| Command | Arguments | What it does |
| --- | --- | --- |
| `space.new` | `name?`, `folder?`, `tab?`, `machine?`, `focus?` (`argv`, `task` refused) | Starts a terminal labeled for a new space, then puts the space in the layout and answers `{"space", "tab", "terminal"}`. Its name leaves `ls`'s `recent`. `folder` defaults to the focused terminal's when it's on that machine, else home (another machine's home there). `machine` defaults to this computer, and `""` also means this computer. An unknown machine fails. `focus: false` keeps the shown space and tab; with no space shown, the new one shows. A `focus` that isn't a boolean fails. |
| `space.select` | `id?` or `index?` (1 = first) | Shows the space. A missing one fails: `no such space`. |
| `space.rename` | `id?` (default: shown), `name` | Writes the trimmed name. `""` goes back to the automatic name. |
| `space.close` | `id?` (default: shown), `confirmed` | Without `confirmed: true` it fails: `space.close ends its terminals: pass confirmed: true`. With it, removes the space and ends its terminals. While they end, they still carry the space's `herdr.space` label, but they aren't adopted back into it. |
| `tab.rename` | `id?` (default: shown), `name` | Writes the trimmed name. `""` goes back to the tab's number. |
| `pane.close` | `pane?` (default: focused) | Removes the pane and ends its terminal. A tab's last pane closes the tab. A space's last pane fails as `space.close` without `confirmed` does, which matches herdr-app. A non-pane id fails: `no pane "<id>"`. |
| `agent.focus` | `terminal` | Focuses the terminal's pane, showing its tab and space. A terminal that isn't running fails: `no running terminal "<id>"`. A sub-agent without a pane is peeked instead (`docs/review-and-peek.md`). |
| `terminal.peek` | `terminal` | Peeks at a running terminal: shows it over the window without moving focus. A terminal that isn't running fails: `no running terminal "<id>"`. While the peek is open, `status`'s `overlays` starts with `terminal-peek`, Herdr app's id for it. |
| `layout.switch` | `name?` | `"eli"` or no name succeeds and changes nothing. Any other name fails: `no layout "<name>": herdr-frontend has only the "eli" layout`. herdr-app ignores unknown names; herdr-frontend reports them. |

`space.new`'s answer arrives once `terminal.run` returns, within the 5 s answer window. If the
terminal fails to start, the answer is the core's error. If the core connection drops first, it
is `the connection to herdr-core dropped`.

## Not served

These are herdr-app commands Eli's tools don't use: `tab.new`, `pane.split`, `terminal.*` (except `terminal.peek`),
`agent.next`/`prev`/`select`, the rest of `layout.*`, `mod.*`, `shot`, `feedback` and the mods'
own `<mod>.<command>`. Each one gets the unknown-command error.

## Status

The window serves the socket. `State::setup` listens on the socket in its state folder before the core link
starts. Each `State::pump` runs the requests that came in through `Control::drain`, carries out
their effects as it does any step's, and sends each later `Effect::Answer` (`space.new`'s) through
`Control::answer`. `on_close` takes the socket away. A window with no home (scenes, `--snap`,
headless tests on `app()`) serves nothing.

Tests:

- `crates/app/tests/window_control.rs` runs the whole app (`live`, headless) on an isolated core
  and an isolated home, and drives every command above with the real binary.
- `crates/app/tests/tool_calls.rs` does the same with each tool's own calls
  ([What each tool calls](#what-each-tool-calls)), one isolated core and home per tool.
- `crates/app/tests/control.rs` and `crates/model/tests/control.rs` cover the socket, the client
  and each command's rules against the model on a fake core.
