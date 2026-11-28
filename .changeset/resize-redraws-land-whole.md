---
"herdr-frontend": patch
---

A program that redraws when its pane changes size (Pi, Neovim without in-band size reports, any full-screen program) no longer ends up torn, wrapped or shifted after the sidebar closes, a zoom, a window resize or a new pane taking its size. The pane in use now resizes its terminal when it sends the new size, so the program's redraw is read at the right grid.
