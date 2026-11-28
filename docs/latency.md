# Input latency: from a key or click to the frame

Eli saw about a second between a mouse-down in a terminal and the cursor moving there, and some
typing lag (ticket 056, from his v0.1.1 recording, #2 at 3:58 and #5). He suspected his USB hub
or a monitor running at 30 FPS. This page is how herdr-frontend measures that path, what the
measurements show, and what was ours to fix.

## The path

A key or a click a program tracks goes through these stages. Each stage is a hop between
threads or processes:

| Stage | Thread | What happens |
| --- | --- | --- |
| `os` | macOS | the event's own time (`NSEvent` timestamp, Rune's `Tx::input_time`) |
| `handler` | UI | Rune runs the pane's handler, at the start of a frame |
| `encoded` | terminal worker | the key or mouse report is encoded for the program's modes |
| `sent` | UI | `poll` drains the worker's output and sends it to the core as DATA |
| `echo` | hc-streams | the program's first output after it arrives on the attach stream |
| `parsed` | terminal worker | libghostty-vt parses it |
| `published` | terminal worker | a snapshot with it goes into the one-slot handoff, and the waker runs |
| `taken` | UI | the next frame's `poll` takes the snapshot |
| `built` | UI | that frame's view builds the pane's rows from it |
| `drawn` | UI | the frame's layout and paint are encoded and submitted |
| `presented` | macOS | the frame reaches the screen (Metal's `presentedTime`) |

DATA goes through `hc connect`, a child process piping to the core's socket, and the echo
comes back the same way. `sent` to `echo` is herdr-core, the program and the pty: not ours.

## Measuring it

`HERDR_FRONTEND_LATENCY=<file>` turns on the tracer (`crates/terminal/src/latency.rs`). It
follows one interaction at a time through every stage. A stage counts only after the stage
before it, on the same terminal, so output from another terminal or a timer isn't taken for
the echo. It also writes every frame: when it started and was drawn, the display link's target
for it, when it was shown, its update and encode times, the display's refresh rate, and the
frames macOS skipped. With the variable unset, each stage costs one atomic load.

`just latency` builds the release app, opens it on an isolated, seeded core with an isolated
home (as `just e2e` does), raises its window, and types into the split tab's shell through the
control socket: 60 keys spaced like typing, then 10 more while `yes | head` floods the
terminal. Then it prints each stage's p50, p90 and max, and the frame pacing. It sends no
OS-level input, so its keys have no `os` stage.

To time real keys and clicks, start the app with the variable set, use it, quit it, and read the
trace:

```sh
HERDR_FRONTEND_LATENCY=/tmp/hfe-latency.jsonl ~/Applications/herdr-frontend.app/Contents/MacOS/herdr-frontend
just latency --summarize /tmp/hfe-latency.jsonl
```

Real input carries the `os` stage, so that report also has "OS event to on screen". A hub or a
keyboard adds its delay before the OS stamps the event, so the trace leaves that out.

macOS draws nothing for a covered window or a locked screen. Those frames show up as skipped,
and nothing reaches the screen.

## What the recording shows

The recording logs each click's time, and the video shows when the nvim cursor moved. The two
clocks disagree by about 170 ms (fitting the logged pointer track to the pointer in the video
gives that), so a click-to-frame time read across them isn't reliable. Taken from the video
alone, the 14 clicks in the "laggy" part (1:02 to 1:22) moved nvim's cursor 17 to 300 ms after
the pointer came to rest, and most within 200 ms. That bound includes the person's own time
between stopping the pointer and pressing. None of the clicks took anything near a second on
screen.

The screen recording captured about 56 frames a second (91,177 frames in 26:55) and the display
reports 60 Hz, so nothing suggests the monitor was held at 30 FPS. That's the capture's rate, not
the window's own frames, which the tracer reports. The typing in #5 shows 278 updates of Pi's input line
in 95 s, a median of 67 ms apart, with 2 to 4 characters in about a quarter of them. That's fast
typing. Without each key's time it can't say more.

The keyboard (a Dactyl Manuform, 12 Mb/s) is four USB 2 hubs deep, behind the Dell dock and
two generic hubs, and the mouse's Logitech receiver is two deep. Hubs add microseconds; a hub
dropping or retrying would show as the pointer itself lagging, which nothing here measures. The
main display (Samsung LS27D80) is behind the dock's MST hub at 60 Hz. DisplayLink drives only
the small Logitech panel ("FLARE"), not the main display.

## Where a key's time goes

`just latency` on this Mac (M1 Pro, release build, the window on screen on the 60 Hz Samsung
display), 73 keys typed into `sh` that reached the screen:

| keys, ms | p50 | p90 | max |
| --- | --- | --- | --- |
| handler to the worker's encoding | 0.0 | 0.0 | 1.3 |
| encoded to DATA on the connection | 0.1 | 0.6 | 1.7 |
| DATA to the echo arriving (core, `hc`, pty: not ours) | 0.5 | 3.3 | 28.8 |
| echo to parsed | 0.0 | 0.0 | 0.8 |
| parsed to the snapshot published | 0.0 | 0.0 | 16.0 |
| published to the UI taking it | 14.8 | 20.2 | 27.3 |
| taken to the pane's rows built | 0.2 | 0.6 | 3.1 |
| built to the frame encoded | 0.8 | 2.8 | 10.3 |
| frame encoded to on screen | 56.2 | 56.8 | 64.5 |
| **handler to on screen** | **72.8** | **81.6** | **107.3** |

Two A/B runs of the same build earlier the same morning (100 keys each, 113 on screen) put
handler to on screen at 57.5 and 59.5 ms p50: the presentation step moves by a refresh or so
from run to run.

Almost all of the time is waiting for the display, not work:

- **Published to taken, about one refresh.** The waker asks for a frame, and Rune draws it on
  the display link's next tick. The worker's work before it and the UI's after it (rows, layout,
  paint, encode) are each about a millisecond.
- **Frame encoded to on screen, 2 to 3.5 refreshes.** Rune aims each frame at a refresh about
  19 ms out (p50; the display link's `target`), and macOS shows it 16.7 to 33.3 ms after that
  target. That second part is the window server's queue, after the frame is submitted.

The window draws at the display's 60 Hz: no timer, `ambient_fps` cap or render hold paces it,
and of the 2,500 frames in these runs macOS skipped none while the window was uncovered.
Under a flood (`yes | head -n 400000`), though, the longest runs of back-to-back frames came at
38 fps, mostly 33.3 ms apart, while each frame's update and encode took about a millisecond.
Something between the snapshot handoff and the display link draws every other refresh while a
terminal is busy. That doesn't touch a key typed into a quiet terminal; it's for a follow-up.

### Rune's swap chain

`App::frame_latency(1)` gives a window two drawables instead of Rune's default three, so a
frame waits less behind the ones before it. One run each, same build:

| frames, ms | drawables | p50 | p90 |
| --- | --- | --- | --- |
| frame encoded to on screen | 3 (default) | 39.7 | 56.5 |
| | 2 | 31.5 | 38.3 |
| handler to on screen | 3 (default) | 57.5 | 73.3 |
| | 2 | 59.5 | 65.0 |

Two drawables cut the tail by about a refresh, but the median didn't move. The same setting
measured 39.7 and 56.2 ms p50 for the presentation step in two runs, a bigger spread than the
gain. So it isn't shipped: it needs repeated runs to show a real gain first.

### What the recording leaves for the hardware

On screen, a key in a quiet terminal takes about 60 to 80 ms from the app's handler to the
screen, about 1 ms of which is herdr-frontend's own work. The rest is the core's echo, the
display link and the window server. A second between a click and the cursor isn't anywhere on
this path; the recording itself shows clicks landing within about 200 ms. What the trace can't
see is before the OS stamps the event: the USB hubs, and the monitor's own processing after the
frame is shown. A trace of real clicks (`HERDR_FRONTEND_LATENCY`, above) adds the OS-event stage.

## What was ours, and the fix

Rune polls once before a frame's input handlers run and once right after them. The worker
encodes a key on its own thread a few microseconds after the handler hands it over, which races
that second poll. When the poll won, the bytes waited for the next frame's poll, a full refresh
later (16.7 ms at 60 Hz), before going out as DATA. Each key press, IME commit, and click
or release a program tracks now waits for the worker to encode it (`Terminal::handled`), for at
most 2 ms, so the poll right after the handler sends it in the same frame. An idle worker
answers in microseconds. Behind a flood of output the wait times out and the bytes go out a frame
later, as before.

Measured with the window on screen and visible part of the time, the same release binary with
the wait off and on, 100 keys each:

| encoded to DATA on the connection, ms | p50 | p90 | max |
| --- | --- | --- | --- |
| without the wait | 0.3 | 9.7 | 58.0 |
| with the wait | 0.1 | 0.6 | 5.8 |

Nothing else on a key's path into a quiet terminal waits, throttles or batches (the flood's 38
fps, above, is the exception to follow up):

- The one-slot handoff waits only while the UI hasn't taken the last snapshot, and the UI takes
  one every frame.
- Synchronized output (mode 2026) holds a frame only while the program asks it to, for at most
  one second, as Ghostty does (`Vt::expire_hold`).
- No timer or `ambient_fps` cap paces the window. Rune draws on the display link's refresh, and
  the first frame after the window rests draws at once.

Scrolling the scrollback had the same shape: UI, then the worker, then the handoff, then the
next frame. The smooth-scrolling ticket (051) moved the screen in the frame the scroll arrives
and measures it with `just scroll-latency`. The tracer follows keys, text and clicks, not the
wheel.
