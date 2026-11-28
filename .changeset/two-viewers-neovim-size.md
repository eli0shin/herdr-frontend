---
"herdr-frontend": patch
---

Neovim keeps herdr-frontend's grid with Herdr app open on the same core: a new pane's terminal starts at that pane's grid, and focusing a pane tells the program herdr-frontend's size again, so its first frame no longer comes out cut up and wrapped.
