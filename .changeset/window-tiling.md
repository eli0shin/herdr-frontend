---
"herdr-frontend": patch
---

The window no longer fights a tiling window manager. It saves its size and position only when it closes or quits, not on every resize, so it no longer records the corner AeroSpace parks hidden windows in or a frame AeroSpace gave it, and a saved place that's mostly off every display reopens centered. It never reopens zoomed or full screen. A double click on the tab strip or the sidebar's top row no longer zooms the window over the whole display and under AeroSpace's bar. Resizing no longer briefly gives the borderless window a title bar on every size change, which used up about half the UI thread while AeroSpace resized it.
