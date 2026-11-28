---
"herdr-frontend": patch
---

A pane herdr-frontend makes (a new space, tab or split) starts its program at the pane's own grid, with the window's colors and cell size, instead of the core's default 200 × 120. Neovim's first paint no longer comes out laid out for that default size, with each row spilling onto the next and panels and floats out of place, when another app such as Herdr app is attached to the same core.
