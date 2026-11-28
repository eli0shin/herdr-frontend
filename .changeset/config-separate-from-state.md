---
"herdr-frontend": patch
---

The app never writes its config folder (`~/.config/herdr-frontend`) any more, so `settings.json`, `keys.json`, themes and programs can live in dotfiles without diffs. Everything it writes now goes in `~/.local/state/herdr-frontend`: the window's place, the view, the sidebar's shared state, the Agents list's times, `keys.error`, the control socket, and the zoom. The zoom keys no longer change `settings.json`. Its `zoom` is the starting value, and editing it still changes the zoom. State files left in the config folder by older versions are moved over at the first launch.
