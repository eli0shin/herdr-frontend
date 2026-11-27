# herdr-frontend

A Rust/Rune desktop client of herdr-core. It must interoperate with a running Herdr app on the same
core: never break the shared layout documents, labels, or terminal state that Herdr app relies on.

## Scope (decided with Eli)

- macOS first. Rune is cross-platform. CI builds and tests the whole workspace on Linux because
  GitHub's Linux runners are free, but Linux and Windows aren't targets: nothing else is built or
  tested for them.
- UI is anomalyco/rune, the GPU-rendered native Rust UI framework, not a TUI library. Sidebar, tabs,
  popups, palette and toasts are native Rune elements; terminal panes are GPU-rendered views.
- Features are built in Rust, configured by files (keys, theme, settings). No JS mod runtime.
- Parity target is Eli's own setup (`docs/research/feature-inventory.md`): his eli layout and arctic
  theme, with the real Operator Mono family (Book weight) at his current 18.7 pt. Shipped-but-unused Herdr features are out of scope.
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
- Switching is instant (Eli's rule): switching spaces, tabs, panes, agents or terminals swaps
  everything the switch changes at once, with no fade, slide or crossfade. When a question like
  this comes up, the answer is instant. Animations unrelated to switching are out of this rule.
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
- Work on a branch via `repos`, open a PR to the landing branch, the required checks must pass,
  squash merges.
- Two checks are required. GitHub's `ci` job runs `just --list` and `just test` on a free Linux
  runner (Eli's choice: PR CI stays on Linux; macOS runners are used only for release builds),
  and must be green on the PR. `just ci` (scripts/local-ci)
  runs the same steps on this Mac, where the macOS-only code and tests run too, and prints
  `local-ci: PASS <sha>`. Run it on your pushed head, plus `just e2e` when your change adds or
  changes scenarios, then tell the orchestrator with `ask_orchestrator`, including that line and
  the green GitHub run. The orchestrator reruns `just ci` on the same commit before merging.
- Run checks through the `justfile`, the way Eli does: `just test` before every push, plus the
  recipes your change touches (`just run`, `just snap`, `just e2e`, `just bundle`, …). Never report
  a recipe as passing unless you ran that recipe and it passed; if a tool is missing, say so and
  stop instead of substituting other commands. A recipe you add or change must be run in the PR.
- Every user-visible PR adds a changeset (`bunx changeset`, or a `.changeset/*.md` by hand), as in
  Eli's other repos. The `changeset check` workflow fails a PR that changes the app's sources
  without one; a change there that Eli wouldn't see adds `bunx changeset --empty`. Never edit the
  version by hand: the "chore: version packages" PR sets it (`docs/release.md`).
- Research and design docs go in `docs/`; keep them current with the code they describe.

## Safety

Never stop, signal or restart the user's running herdr-core or Herdr app. Test against an isolated
core (its own socket/home) and an isolated app home. Read-only queries (`hc ls`, `hc schema`,
`hc events`) against the real core are fine.
