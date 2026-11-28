---
"herdr-frontend": patch
---

Neovim started in the focused pane keeps herdr-frontend's grid with Herdr app open on the same core: when a program there turns on in-band size reports, herdr-frontend tells it its size again a moment later, after Herdr app has answered, so Neovim no longer lays out at Herdr app's grid.
