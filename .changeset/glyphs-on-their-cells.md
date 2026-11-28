---
"herdr-frontend": patch
---

Terminal text stays on its cells at every zoom. When zoomed (1.1 by default), each glyph sat a fraction of a pixel further off its column than the last. By the end of a long line the cursor, the selection and copy mode's cursor covered the wrong letters, and text crowded the Nerd Font icons beside it. Each glyph now sits on its own cell across the whole line, under the cursor and selection that mark it.
