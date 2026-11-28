---
"herdr-frontend": patch
---

A trackpad (and its momentum) or a free-spinning wheel scrolls a terminal's scrollback smoothly, by the pixel, instead of a line at a time. Distances are the same as before and the same as Herdr app's: a line per cell height of travel, times `terminal_scroll_speed`. The screen moves in the frame the scroll arrives, rather than after the terminal's worker answers. A notched mouse wheel still moves three whole lines a notch, and programs that track the mouse still get whole wheel events.
