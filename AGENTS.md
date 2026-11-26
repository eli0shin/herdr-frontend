# herdr-frontend

A Rust/Rune desktop client of herdr-core. It must interoperate with a running Herdr app on the same
core: never break the shared layout documents, labels, or terminal state that Herdr app relies on.

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
