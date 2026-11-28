---
"herdr-frontend": patch
---

Every pane shown in the window in front now keeps its terminal at the pane's size, not just the focused one. After a split, a close, a divider drag, a sidebar toggle, a window resize or a zoom, each pane redraws at its new size at once, without a click. Split panes are sized in whole terminal cells, so each terminal's grid reaches the divider. Divider drags snap to cell boundaries, and the resize keys (`alt-shift-h/j/k/l`) move a divider 5 columns across or 2 rows down per press. A grid that doesn't fill its pane's height sits at the pane's bottom, with the leftover above it. Space a pane hasn't filled yet shows the terminal's background, not the grey split-gap color.
