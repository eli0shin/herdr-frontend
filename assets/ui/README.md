# Panel glyphs

The one-color glyphs the panels draw, tinted at draw time (`crates/app/src/view/sidebar.rs`):

- `clock.svg`, `layers.svg`, `check.svg`, `chevron-down.svg`, `chevron-right.svg`, `monitor.svg`
  and `spinner.svg`: Herdr app's `defaults/mods/workspace-chrome/` (identical to what Herdr app
  0.1.18 installs in `~/.config/herdr-app/mods/workspace-chrome/`), which Eli's `eli-chrome` and
  `eli-agents` draw. Herdr app is Apache-2.0
  ([../programs/LICENSE-herdr-app.txt](../programs/LICENSE-herdr-app.txt)); the Lucide glyphs among
  them are ISC ([../icons/LICENSE-Lucide.txt](../icons/LICENSE-Lucide.txt)).
- `git-pull-request.svg`: Lucide's (ISC), as Eli's `eli-agents` mod ships it.
