---
"herdr-frontend": patch
---

A program that redraws when its pane changes size (Pi, Neovim without in-band size reports, any full-screen program) no longer ends up torn, wrapped or shifted after the sidebar closes, a zoom, a window resize or a new pane taking its size. A shown pane now resizes its terminal as it sends its new size, so the program's redraw is read at the right grid.
