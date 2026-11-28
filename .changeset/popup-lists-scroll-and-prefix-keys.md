---
"herdr-frontend": patch
---

Popup lists (the command palette, the pickers, the settings and machine dialogs) are one flat list that the mouse wheel and trackpad scroll, leaving the selection where it is. A list that fits doesn't scroll, and the palette keeps its height as you move through it. Key hints in the palette, menus and keys help show your prefix key when a command has one (`ctrl-g t` for New tab, `ctrl-g shift-q` for Close tab), and another key only when it has none. The palette no longer offers "Reset to the default keys", and the tab strip no longer shows a `ctrl-g …` indicator while a prefix is pending.
