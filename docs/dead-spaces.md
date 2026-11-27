# Closing spaces whose folder was deleted

`repos clean` closes a worktree's space, but other ways of removing a worktree don't tell anyone:
`gh pr merge --delete-branch` run from the main checkout, a plain `git worktree remove`, `rm -rf`.
The space then lingers with terminals in a folder that no longer exists. Eli's
`cfg/mods/navigator-status/dead-spaces.js` closes such a space (`feature-inventory.md` §3), and
herdr-frontend does the same. The port follows the mod as it is in Eli's dotfiles working tree on
2026-11-27, which no longer requires plain shells.

The rules are pure model code (`crates/model/src/dead_spaces.rs`), run by `Model::step`
(`crates/model/src/model.rs` `sweep_dead_spaces`, `swept`).

## When a space closes

Every 30 s, starting 30 s after the first listing, a space qualifies when all of these hold:

- it is on this computer and isn't the shown space;
- it has live terminals: this computer's terminals whose `herdr.space` label names it;
- none of them reports a working agent. Idle, done, blocked and unreported agents, and foreground
  programs such as Neovim, don't keep it;
- its folder (home, for a space made without one) and each of those terminals' live folder
  (`foreground.cwd`, else the launch folder) are all gone. A terminal whose folder is unknown keeps
  the space.

One `machine.exec` on this computer checks every qualifying space's folders at once (a `[ -d ]`
per folder, passed as arguments, within 10 s). When it answers, the model is read again: a space
whose folders changed, that an agent started working in, or that was shown meanwhile is dropped
from this sweep.

A space must be found dead on two sweeps in a row with the same folders, so it closes 30–60 s
after the folder goes, and a folder that is only briefly missing keeps its space. A failed or
timed-out check closes nothing. Only one check runs at a time.

Closing works like `space.close {confirmed: true}`: the space leaves the layout, its panes'
terminals end, and it joins the recently closed spaces. An info toast says "Closed space <name>",
with "Its folder no longer exists: <folder>" below.

## Tests

- `crates/model/tests/dead_spaces.rs` runs the cases of Eli's `tests/herdr-app-dead-spaces.mjs`
  through `Model::step` on a fake core and clock, plus the schedule, the launch-folder fallback, a
  check still under way, and a reconnect.
- `crates/model/src/dead_spaces.rs` tests the check itself under `sh` and how its answer is read.
- The end-to-end scenario `03-spaces` adopts an idle worker into a space of its own, deletes its
  folder from another shell, and waits for both apps to close it.

## Where it differs from Herdr app

- The folder check has a 10 s timeout; the mod waits as long as the check takes.
- No sweep starts while this computer's layout can't take an edit (not read yet, unavailable),
  so a sweep doesn't repeat the "Layout unavailable" toast every 30 s. The "Closed space" toast
  only shows when the close applied.
