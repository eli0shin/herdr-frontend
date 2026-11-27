# herdr-frontend

A Rust/Rune desktop client of herdr-core. It must interoperate with a running Herdr app on the same
core: never break the shared layout documents, labels, or terminal state that Herdr app relies on.

## Scope (decided with Eli)

- macOS first. Rune is cross-platform, but nothing is built or tested for Linux or Windows yet.
- UI is anomalyco/rune, the GPU-rendered native Rust UI framework, not a TUI library. Sidebar, tabs,
  popups, palette and toasts are native Rune elements; terminal panes are GPU-rendered views.
- Features are built in Rust, configured by files (keys, theme, settings). No JS mod runtime.
- Parity target is Eli's own setup (`docs/research/feature-inventory.md`): his eli layout and arctic
  theme; terminal fonts follow his Ghostty config. Shipped-but-unused Herdr features are out of scope.
- herdr-frontend is a client of herdr-core, not of Herdr app. Shared state is exactly what lives in
  herdr-core (terminals, agents, labels, the per-machine layout document). Never read, write,
  import from or sync with Herdr app's files (`~/.config/herdr-app`, its `shared.json`, its fonts,
  its sockets). Everything else lives in herdr-frontend's own home. (The e2e harness may run Herdr
  app with an isolated copy of a config, for side-by-side screenshots only.)
- The terminal is Ghostty with Herdr around it. Eli's Herdr config contains workarounds for Herdr
  app's limits (for example split "Operator Mono Book Herdr" font files, traffic lights parked
  off-screen). Never port a workaround; build what it was working around, the way Ghostty or the
  intent does. Terminal look and behavior follow Eli's Ghostty config
  (`~/.dotfiles/home/.config/ghostty/config`). If unsure whether something is a workaround, ask.
- No native OS notifications and no sounds, ever. Attention shows in the app only (sidebar,
  markers, in-app toasts).
- When you need "the latest" version of anything, look it up live without putting a version or
  date in the query; don't trust remembered versions.

## Tracker

Tickets live in the `herdr-frontend` Tickets project (`tickets --project herdr-frontend ...`).
A worker claims its ticket by writing its identity to `Assigned-To`, moves it to `in-progress`,
and runs `tickets lint`. Only the orchestrator runs `tickets done`.

## Conventions

- Commit messages: lowercase conventional commits, single line, no AI co-author lines.
- Work on a branch via `repos`, open a PR to the landing branch, CI must pass, squash merges.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` must pass.
- Research and design docs go in `docs/`; keep them current with the code they describe.

## Safety

Never stop, signal or restart the user's running herdr-core or Herdr app. Test against an isolated
core (its own socket/home) and an isolated app home. Read-only queries (`hc ls`, `hc schema`,
`hc events`) against the real core are fine.
