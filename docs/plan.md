# herdr-frontend: architecture and build plan

This is the plan the implementation tickets come from. The research it rests on:

- `docs/research/herdr-core-protocol.md`: the client protocol and the rules for sharing a core with Herdr app.
- `docs/research/terminal-rendering.md`: libghostty-vt and our own renderer.
- `docs/research/rune.md`: building the app on Rune.
- `docs/research/feature-inventory.md`: Eli's parity checklist (the spec).

Scope rules are in `AGENTS.md`.

## Decisions

- **Client of herdr-core, side by side with Herdr app.** It uses the same core, machines,
  terminals and labels, and the same per-machine `herdr-app` layout document under herdr-app's
  exact schema. Writes go through CAS, with the edit replayed on conflict. Per-screen view state
  (focus, zoom, active tab, recents, seen) stays in herdr-frontend's own home, never in shared
  state. Protocol rules 1-10 (`herdr-core-protocol.md` "The rules in one place") are binding.
- **Terminals:** libghostty-vt 1.0.2, from the pinned and checksummed xcframework (macOS slice,
  no Zig), behind our own `ghostty-vt-sys`. Each terminal runs on its own worker thread and
  hands out one snapshot per frame. We draw it ourselves, first on `rune::grid`. Cell metrics
  are Ghostty's, computed in whole device pixels. Ligatures are off.
- **UI:** Rune, pinned to one `dev` rev, with its `[patch.crates-io]` block, its toolchain, and
  rune-kit for shell pieces. There is one keymap and prefix-mode machine. Effects run outside
  handlers, and there is no I/O on the UI thread.
- **Features are Rust**, configured by files in herdr-frontend's home: `keys.toml` (or `.json`),
  `settings`, and the theme. Eli's Herdr mods are specs, not code to port. There is no mod
  runtime.
- **No OS notifications and no sounds.** Attention shows up in-app only.
- **No collapsible sidebar sections.** Eli collapses them by accident; sub-agent folding stays.
- **Window like Eli's Ghostty** (`window-decoration = false`): borderless, square corners, no
  frame, no traffic lights, no title bar.
- **The CLI surface Eli's tools drive** (`herdr-app ls | do | status` from repos, spawn-worker,
  agent-status, Pi and dot): herdr-frontend serves a compatible `ls | do | status` through its
  own binary. After parity, each tool reads one frontend setting and calls
  herdr-frontend, herdr-app or `hc` directly, with no shim; switching back is one edit (ticket 035).

## Workspace layout

```text
crates/
  ghostty-vt-sys/   libghostty-vt bindings + link of the pinned xcframework
  herdr-proto/      pure: frame codec, JSON-RPC types, Event, TerminalSet::apply
  herdr-client/     I/O: Connection (hc connect / socket), threads, calls, attach, subscribe, Supervisor
  herdr-shared/     pure: lossless herdr-app Document, Mirror with CAS replay, labels, seen, attention, adoption
  terminal/         libghostty-vt wrapper, per-terminal worker, snapshots, input encoders, metrics
  model/            pure app state: spaces/tabs/pane trees from the mirror, focus, view state, Intents
  app/              Rune binary: views, keymap, scenes, headless tests, control CLI
```

## Tickets and order

The tracker holds the dependency graph. These are the stages:

1. **Foundation:** the workspace skeleton (Rune pin, toolchain, CI, a hello window with a
   headless test).
2. **In parallel:**
   - the protocol crate, then the client and the shared-state crates;
   - the vt bindings, then the terminal core;
   - cell metrics.
3. **Renderer MVP** on `rune::grid`, plus the app state model.
4. **App shell with live terminals.** The first build Eli can run against his real core,
   read-mostly.
5. **Input and keymap, then features in parallel:**
   - fidelity, scrollback and search, links, copy mode;
   - tabs, panes, the agents sidebar, agent navigation, toasts, pickers and palette;
   - the new-space flow, the PR finder, machines, review and peek;
   - theme and settings, the control CLI, multi-viewer sizing, packaging.
6. **End-to-end harness** (ticket 034): the full app running for real against an isolated core,
   driven by scripts, with full-window screenshots, and the same scenarios run in Herdr app.
7. **Parity audit** (ticket 033) against `feature-inventory.md`, item by item, with side-by-side
   full-app screenshots of both apps. Gaps are fixed. Only then is it "ready for Eli to test".
8. **Tool routing** (ticket 035): one frontend setting, read by each of Eli's tools directly (no shim).

## Testing rules for every ticket

- Never touch Eli's live core except for read-only calls. Integration tests start an isolated
  core (`HC_SESSION=test-*` with scratch XDG dirs, as `herdr-core-protocol.md` "Probe" does) and
  an isolated herdr-frontend home.
- UI is checked headlessly: Rune scenes, `--snap`, `--tree`, `--script`, and
  `App::headless_runtime` tests. Never drive input on Eli's real display.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass in CI on the
  `xcode-27` (macOS 27) runner.
