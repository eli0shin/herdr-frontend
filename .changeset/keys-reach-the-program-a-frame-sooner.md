---
"herdr-frontend": patch
---

Keys, typed text and clicks a program tracks go out to the program in the frame that handled them, instead of sometimes waiting for the next one. `HERDR_FRONTEND_LATENCY=<file>` traces where each key's and click's time goes on its way to the screen, and `just latency` measures it on an isolated core (`docs/latency.md`).
