# Research: Rune for a terminal multiplexer desktop app

How to build herdr-frontend on [Rune](https://github.com/anomalyco/rune): window chrome, sidebars,
tabs, split panes, overlays, keyboard-driven UX and GPU terminal panes.

**Sources and revisions.** Rune `dev` at [`4918969f`][rune] (2026-11-25), and
[opencode-rune][ocr] at `a1d6a19c` (2026-11-25), a large app on Rune that pins Rune
`5342f14d`, 51 commits behind the Rune rev above. Every claim cites a file at those revisions.
Claims marked **verified** were checked with a throwaway hello-world build (Apple M1 Pro, macOS,
Rust 1.99.1) that pinned Rune `4918969f`. It used `rune::grid`, a background feeder and a headless
test. The app code was not committed.

Terminal emulation (`libghostty-vt`) belongs to
[`terminal-rendering.md`](terminal-rendering.md), and the herdr-core socket to
[`herdr-core-protocol.md`](herdr-core-protocol.md). This doc covers the Rune side of both.

## Summary

- **Draw terminals with `rune::grid`.** It is Rune's built-in terminal-screen element, and
  opencode-rune's Ghostty terminal uses it. Rows are `Rc<GridRow>`: the app hands back the same
  `Rc` for a row that didn't change, so Rune neither reshapes nor re-emits it. The cost for a
  200×60 screen is about 0.4 ms per frame when one row changes, well inside a 120 Hz budget. A
  custom wgpu pass is possible through the `Compositor` hook, but it loses accessibility, the CPU
  painter and headless parity. It also gets no public access to Rune's glyph atlas. Don't start
  there.
- **Threading.** Run each terminal's VT emulator on its own thread, as opencode-rune does. The
  worker publishes at most one snapshot per drawn frame and wakes the UI through `rune::Waker`.
  Run the herdr-core client on a thread too. It sends events over a channel, and `AppSpec::poll`
  drains that channel.
- **Keyboard.** App keybindings and modes (a tmux-style prefix) go in one keymap function, called
  from a root `El::on_key_preview` and from `AppSpec::on_key`. The preview sees every key before the
  focused terminal (**verified**) but runs only while something has focus; `AppSpec::on_key`
  covers the case where nothing does. The terminal grid is
  `focusable` and takes everything else in `El::on_key`, Tab and Ctrl chords included, ahead of
  Rune's focus navigation (**verified**). `tx.physical_key()`, `tx.key_base()`,
  `tx.key_repeat()` and `on_key_up` supply what a key encoder needs. Rune has no multi-key
  sequences: build modes as an explicit `rune::machine` state machine.
- **Shell.** `rune-kit` gives the sidebar (`shell::SplitView`, `Sidebar`, `PaneState`), tab strips
  (`TabStrip`: reorder, tear-out, middle-click close), and the overlays: `Menu`, `Popover`,
  `Dialog`, `Combobox`/`PopupList` for a palette, `ToastStack` and `Tooltip`. Nested terminal
  splits are not a component: build them from `on_drag` dividers, as opencode-rune does.
- **Testing.** Every debug build accepts `--snap`, `--tree` and `--script`. `cargo test` can drive
  the real runtime with `App::headless_runtime` (**verified**). Workers should check UI changes
  that way.
- **Biggest gaps.** In-app drag and drop has no payloads (dragging a tab onto a pane is hand-rolled,
  rune#357). A panic in a view kills the process (rune#1003). Linux has no native pop-up menus
  (use the in-window fallback hook), and on GNOME Wayland `App::titlebar` shows two title bars
  (rune#591). Rune lands hundreds of commits a day on `dev`, so pin a rev and bump deliberately.

## 1. Pinning, dependencies, toolchain, build times

### Pin a `dev` rev, and patch the same rev

Rune is pre-release. Pin a `dev` rev; `dev` is fast-forward only and never rewritten
([`README.md`][rune-readme], [`AGENTS.md`][rune-agents] "Changes"). Cargo does not inherit
`[patch.crates-io]` from a dependency. The app's workspace root therefore patches every crate in
Rune's own `[patch.crates-io]` list, at the same rev and the same URL
([`crates/rune/README.md`][rune-crate-readme] "Depending on Rune").

At `4918969f`, Rune's root [`Cargo.toml`][rune-cargo] patches eight crates. This manifest builds
(**verified**):

```toml
[dependencies]
rune     = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }
rune-kit = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }

[patch.crates-io]
winit                  = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }
sctk-adwaita           = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }
parley_engine          = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }
wgpu-hal               = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }
naga                   = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }
accesskit_windows      = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }
accesskit_macos        = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }
accesskit_atspi_common = { git = "https://github.com/anomalyco/rune", rev = "4918969f04dce020f63b34e024368a86e5e1857d" }
```

- The patches are required. `winit` and `sctk-adwaita` must come from the same rev, or the
  workspace fails to compile ([`crates/rune/README.md`][rune-crate-readme]). Without the winit
  patch, Rune doesn't build on macOS at all (rune#850). Without `accesskit_macos`, VoiceOver
  misses Rune's headings.
- To bump: read Rune's root `Cargo.toml` at the new rev and match its patch list. Then run
  `cargo check` and treat every ``Patch `…` was not used in the crate graph`` warning as a wrong
  URL, a wrong rev, or a patch Rune no longer needs ([`crates/rune/README.md`][rune-crate-readme]
  "Bumping Rune"). Rune ships [`scripts/check_rune_pins.py`][rune-pins] for CI to check this
  offline. Copy it in; note that rune#2439 says it misses workspace-member manifests.
- Use one Rune git URL everywhere. A mirror goes through `git config url.<mirror>.insteadOf`
  plus `[net] git-fetch-with-cli = true`, not through edited manifests.
- Rune moves fast: `5342f14d..4918969f` is 51 commits in about nine hours. Rune doesn't take
  pull requests; problems go upstream as issues ([`README.md`][rune-readme] "Contributing").
  Expect API churn between bumps, and read [`CHANGELOG.md`][rune-changelog] (it lists changes
  that move output without a code change) plus `changelog/unreleased/` at each bump.

### Features

The defaults are `accessibility`, `dbus`, `embedded-fonts`, `hunspell`, `metal-backend`,
`ogg-audio` and `svg` ([`crates/rune/Cargo.toml`][rune-crate-cargo]). opencode-rune turns off
`embedded-fonts` (it registers its own faces, which saves 1.25 MB) and `ogg-audio`
([opencode-rune `Cargo.toml`][ocr-cargo]). Add `tooling` if a release build should keep
`--snap`, `--tree` and `--script`; debug builds always have them. Add `native-file-dialog` for
Open and Save panels.

### Toolchain and platforms

- Rune pins Rust `1.99.1` with edition 2024 ([`rust-toolchain.toml`][rune-toolchain]).
  opencode-rune pins the same and needs Cargo 1.99 for `default-features = false` on a workspace
  dependency ([opencode-rune `rust-toolchain.toml`][ocr-toolchain]). Pin `1.99.1` here.
- Linux builds need `libxkbcommon-dev` and `libwayland-dev` (CI included). Windowed runs also need
  `libxkbcommon-x11-0`. A GPU-less CI box runs `--snap` without a display, and runs windows
  through `mesa-vulkan-drivers` (lavapipe) ([`crates/rune/README.md`][rune-crate-readme] "Linux").

### Build times (verified, M1 Pro, 8 cores)

| Build | Wall time |
| --- | --- |
| Cold `cargo build`, dev profile (overrides below), `rune` + `rune-kit`, git fetch included | 2 m 05 s |
| Incremental rebuild after editing the app crate | 0.8–2.7 s |
| `cargo test` of the app crate, warm | 1.9 s |
| Cold `cargo build --release` (deps already fetched) | 1 m 43 s |

The dev builds above used `[profile.dev.package."*"] opt-level = 1, debug = false` with
`opt-level = 3` for `rune` and `rune-text`. That is close to opencode-rune's profile below, not
Cargo's default dev profile. The release build used Cargo's default release profile.

The release binary was 31 MB unstripped, with default features and `rune-kit`. Rune itself reports
10.9 MB (9.3 MB stripped) for its settings example ([`crates/rune/README.md`][rune-crate-readme]
"Measured").

Build Rune optimized inside dev builds, as opencode-rune does: `opt-level = 1` for dependencies
and `opt-level = 3` for `rune`, `rune-text`, `rune-image` and `parley_engine`
([opencode-rune `Cargo.toml`][ocr-cargo]). Otherwise text shaping and painting make debug runs
sluggish.

## 2. App architecture on Rune

### State model and view functions

- An app is plain state `S` plus `fn view(&S, &mut Cx<S>) -> El`. Handlers such as
  `.on_press(|s: &mut S| ..)` and `.on_key(|s, ev, tx| ..)` are queued and run between frames
  with `&mut S`. The view then re-runs and Rune diffs the result. Hover and press only retarget
  animation tracks and never re-run the view ([`crates/rune/README.md`][rune-crate-readme]
  "What exists").
- `AppSpec`'s `init`, `view`, `setup`, `poll` and `on_key` are **`fn` pointers, not closures**
  ([`crates/rune/src/tooling.rs`][rune-appspec]). Use `App::new(spec).init(move || ..)` to build
  first state from captured values such as the socket path or CLI arguments
  ([`crates/rune/README.md`][rune-crate-readme] "App lifetime").
- `AppSpec::new(title, size, init, view)` fills every other field; literals end with
  `..AppSpec::new(..)`. Rune adds builder methods rather than pub fields, so literals keep
  compiling ([`AGENTS.md`][rune-agents] "Changes").
- **Keyed trees.** `.key("…")` keeps an element's identity, animation tracks, Taffy cache and
  focus across commits. Key every pane, tab and row by its herdr id, never by index.
  `rt.find("row-3/more")` and script `"key"` targets resolve key paths.
- **Skip work.** `memo(id, version, || ..)` reuses a subtree while a small version (an id plus an
  edit counter) holds. `region(id, || ..)` with `rune::Tracked<T>` fields reuses a subtree while
  the tracked cells it read keep their versions
  ([`crates/rune-core/src/tracked.rs`][rune-tracked], [`crates/rune/src/memo.rs`][rune-memo]).
  Long lists use `virtual_rows(count, estimate, key, build)`, which builds only mounted rows. Rune's
  "Fast apps" page ([`site/src/FastApps.tsx`][rune-fast]) is the checklist. `poll` must be cheap
  and return true only on visible change. Read `Cx::layout_size`, not `window_layout`. Hover is a
  style (`.hover(color)`), not state. Bound every cache. Debug builds warn on each of these
  mistakes.
- **Modes are explicit state machines.** Rune's own rule, which opencode-rune enforces too: one
  `enum State` whose variants carry only valid data, and one
  `fn step(&mut State, Event) -> Effects<Effect>` (`rune::machine`), with timers arriving as
  events ([`AGENTS.md`][rune-agents] "State"; [opencode-rune `AGENTS.md`][ocr-agents]). Use this
  for connection and reconnect, the prefix-key mode, drags, palette open and close, and focus
  modes.

### Background work: the herdr-core socket client feeding state

Nothing blocking runs on the UI thread. Debug builds warn when a handler, `poll` or `init` waits
for more than 8 ms ([`site/src/FastApps.tsx`][rune-fast]). The supported shapes:

- **Channel plus waker (recommended for the core client).** `AppSpec::setup(&mut S, Waker)` runs
  once. `rune::Waker` is `Arc<dyn Fn() + Send + Sync>`
  ([`crates/rune-core/src/lib.rs`][rune-waker]). A client thread reads the socket, sends decoded
  events into an `mpsc` channel and calls `waker()`. Then `poll(&mut S, clock, &mut Tx) -> bool`
  drains the channel with `try_recv` and returns true only if visible state changed
  (**verified**: lines a thread sent appeared in a headless `--snap`).
- **`AppHandle<S>`** (`Clone + Send + Sync`, from `app.handle()`). `handle.update(|s, tx| ..)`
  runs a closure on the UI thread, in call order, even while no window shows
  ([`crates/rune/README.md`][rune-crate-readme] "App lifetime"). It suits rare events. For a
  firehose, prefer a channel, so one frame applies a batch.
- **`rune::task::spawn` / `serve`** run work on Rune's shared pool or on a dedicated service
  thread draining a `Receiver` ([`crates/rune-core/src/task.rs`][rune-task]).
  **`rune::Latest`** runs debounced latest-wins requests, such as palette search
  ([`crates/rune/README.md`][rune-crate-readme] "Requests off the main thread").
- Start network work in `App::on_ready`, after the first frame
  ([`AGENTS.md`][rune-agents] "Performance").

### Timers

`tx.wake_after(Duration)` and `tx.wake_at(t)` schedule a `poll` on the app clock. Deadlines
coalesce and the shell sleeps until the earliest one, with no timer thread. A loop must have an end
condition: an idle window draws nothing, and debug builds warn when a window keeps drawing with
nothing changing ([`crates/rune/README.md`][rune-crate-readme] "App hooks";
[`site/src/FastApps.tsx`][rune-fast]). A cursor blink is therefore a wake loop that stops while
the terminal is unfocused or the window hidden. Hidden and covered windows run `poll` and timers
but not view, layout or drawing.

### Multiple windows

`rune::run_windows(|displays| Vec<WindowSpec<S>>)` opens several windows on one event loop with
one GPU device. Each window has its own retained tree, renderer and text atlas, and windows can
have different state types (`AnyWindowSpec`). `App::window(WindowSpec)` adds a titled window to a
running app; `rune::open_window` opens one later. Share data between windows through handles or
services; each window's state is its own ([`crates/rune/README.md`][rune-crate-readme] "Multiple
windows and overlays"). For herdr-frontend: one `S` per window, with the core client as a shared
service that fans events out to every window's channel.

## 3. Drawing a terminal grid at 120 Hz

### Recommended: `rune::grid`

[`crates/rune/src/el/grid.rs`][rune-grid] is "a terminal's screen as one element". It was added in
`06b93dc7` (#813, 2026-11-12).

```rust
pub fn grid(cols: u16, cell_w: f32, cell_h: f32, rows: Vec<Rc<GridRow>>) -> El
GridRow::new(text, spans, style)   // the narrow cells as one shaped line, styled by Span runs
    .cell(col, width, text, style, color) // wide or fallback glyphs pinned to their column
    .fill(start_col, end_col, color)      // background runs and selection under the text
    .shape(Shape)                         // vector strokes over the text: box drawing, curly underlines
    .wraps(bool)                          // the row continues into the next (soft wrap)
El::grid_caret(col, row)                  // the cursor, as the screen reader's caret (draws nothing)
```

- **Cost.** A row is shaped once while the app hands back the same `Rc<GridRow>`, and Rune redraws
  its last instances while nothing it depends on changed. Rune's numbers for a 200×60 screen,
  headless, thread CPU per frame at p50: full redraw 62.5 → 2.46 ms, one row changed
  9.5 → 0.40 ms, nothing changed 8.0 → 0.30 ms
  ([`changelog/unreleased/2026-11-13-grid.md`][rune-grid-changelog]). A 120 Hz frame has 8.3 ms.
- **Fidelity.** It draws the same pixels as the equivalent `rich` line, per-cell `text`, fills and
  `canvas`, on both the GPU and the CPU painter: 0 pixels differ on 32 screens. Accessibility
  reads it as role `terminal`, a text line per row, with the caret at `grid_caret`
  ([`changelog/unreleased/2026-11-14-a11y-grid-lines.md`][rune-grid-a11y],
  [`…-grid-caret.md`][rune-grid-caret]). **Verified:** `--tree` reports the element with role
  `terminal`.
- **Not included.** The cursor shape and blink, find highlights, hovered-link underlines and IME
  preedit are drawn as sibling elements absolutely positioned over the grid. So are mouse
  handlers on the viewport, which map pixels to cells.

### How opencode-rune draws its Ghostty terminal

- **The VT core runs off the UI thread.** [`crates/terminal`][ocr-terminal] wraps `libghostty-vt`
  (built by `build.rs` with Zig, or taken from `GHOSTTY_VT_LIB`). Each session's worker feeds PTY
  bytes to the emulator. A lock-free three-state `Handoff` publishes a `TerminalSnapshot` only
  after the UI has read the previous one: at most one extraction per drawn frame, and none for a
  hidden tab. Synchronized output (`?2026`) holds the frame for up to 1 s
  ([`publish.rs`][ocr-publish]). Output is coalesced for at most 2 ms or 256 KiB before the worker
  looks at its commands again ([`pty.rs`][ocr-pty]). The worker calls `rune::Waker` when there is
  something to draw.
- **The UI builds rows with a cache.** [`grid/rows.rs`][ocr-rows] keeps a per-tab
  `HashMap<(row id, content hash, font/cell key, theme), Rc<GridRow>>`. Unchanged rows are handed
  back as the same `Rc`, so only changed rows are reshaped. Box drawing, strikethrough and curly or
  double underlines go into `GridRow::shape` ([`draw.rs`][ocr-draw]). Wide, emoji and other-script
  cells go into `GridRow::cell`.
- **Cell metrics.** The cell is the advance of "M" by the face's ascent plus descent, each rounded
  to whole device pixels so the grid stays integral at any scale. Tracking is set so glyphs land
  exactly one cell apart, and `calt`/`liga` are off so ligatures can't shift cells
  ([`grid/mod.rs`][ocr-grid] `cell_metrics_at`, `terminal_font_style`).
- **Overlays.** [`grid/mod.rs`][ocr-grid] builds a `canvas` cursor (block, bar, underline or hollow)
  kept mounted with `.instant_changes()` and `Transition::none()`. It also builds find-hit fills,
  link underlines, OSC 133 prompt marks in the margin, an `.ime_cursor()` anchor element, and the
  preedit drawn as `rich` text. [`grid/pointer.rs`][ocr-pointer] attaches `on_drag` (selection,
  word and line clicks via `Drag::clicks`), `on_wheel`, `on_pointer_move_local`, a context menu,
  and a drag-able scrollback bar.
- **Resize.** The view computes `cols × rows` from the pane's size and tells the worker
  (`tab.ensure_fit`), which debounces the PTY resize ([`grid/mod.rs`][ocr-grid],
  [`pty.rs`][ocr-pty]).

### Alternative: a custom wgpu pass (not recommended to start)

`App::compositor` / `rune::run_with(spec, Compositor)` hands each frame's device, queue, encoder,
target and Rune's `Renderer` to the app. `El::effect(mark)` reserves a box in painter order where
the compositor paints with its own pipeline, and `render::CLIP_WGSL` clips it like Rune's own
primitives ([`crates/rune/README.md`][rune-crate-readme] "Compositor hook"). A cell-instanced
terminal shader could live there, but:

- I found no public API to sample Rune's glyph atlas from a compositor, so the pass needs its own
  rasterizer and atlas.
- It gives up the grid's accessibility tree, the CPU painter, and pixel parity in
  `--snap`/`--tree`.
- On macOS, Rune's default native Metal backend supports only a subset of wgpu. Engines needing
  more must opt into `GpuBackend::Wgpu`.

Move to this only if profiling shows `grid` can't keep up (many panes scrolling at once, say). The
numbers above suggest it can.

## 4. Keyboard

### Routing order

A window key goes to the first of these that takes it, after global shortcuts and native menu
equivalents ([`crates/rune/README.md`][rune-crate-readme] "Key routing"):

1. Return or Escape during an IME composition.
2. **`El::on_key_preview`** on the focused element's ancestors, outermost first, down to the
   focused element. This is the capture phase, and runs even while a text field has focus.
3. **`El::on_key`** on the focused element and its ancestors, innermost first. Skipped while a text
   field has focus.
4. Return to the default button.
5. With nothing focused, the window's `AppSpec::on_key`.
6. The focused field's `on_input_key`.
7. Focus navigation: Tab and Shift-Tab, Escape closing a modal, arrows inside a `focus_group`,
   Space and Return pressing controls.
8. The text field's editor.
9. `AppSpec::on_key` again, for keys the focused control passed on.
10. Copy keys on selected text.
11. Page Up/Down, Home and End scroll the nearest scroll view.

A focused element's `on_physical_input` (raw physical key, press/release, repeat, produced text,
consumed modifiers) runs **before all of the logical routing**. If it consumes a key, that key never
reaches `on_key_preview` ([`crates/rune/src/runtime/input/physical.rs`][rune-physical]
`dispatch_physical`).

### Recommended wiring for herdr-frontend

- **App keymap and modes: one function, called from two places.** Call it from a root
  `on_key_preview`, which sees every key before the focused terminal. A prefix chord such as
  `ctrl+b` enters a mode, and the next key runs a command (**verified**: a root `on_key_preview`
  caught `ctrl+b` and then `%` while a focused grid's `on_key` was installed). Preview handlers
  run only on the focused element's ancestors, though. With nothing focused they never run
  ([`crates/rune/src/runtime/input/keys.rs`][rune-input-keys] `element_key_preview`;
  **verified**: with nothing focused, `ctrl+b` skipped the root preview and `AppSpec::on_key`
  heard Escape), and focus is
  easy to lose: a click on plain content blurs, as do startup before a pane takes focus and closing
  the last pane. Call the same keymap from `AppSpec::on_key`, where step 5 delivers keys while
  `tx.focused()` is `None`. Preview handlers also stop at an active modal, so an open palette or
  dialog owns its keys. Rune has no multi-stroke chords (`Chord` and `Keybind` match one chord;
  [`crates/rune/src/input/chord.rs`][rune-chord]). Model the modes, including any Vim-like
  normal/insert modes, as a `rune::machine` with a timeout event.
- **Bind with `Chord` and `Keybind`, not raw `Mods`.** `Mods::cmd` is ⌘ on macOS and Ctrl elsewhere,
  so Ctrl+W on Windows is `{cmd, ctrl}`. `Chord::parse("mod+shift+t")` resolves per platform;
  `matches_tx(ev, tx)` follows the layout through `tx.key_base()`, and falls back to the physical
  key on non-ASCII layouts. `Keybind` holds alternatives and per-platform overrides and writes
  canonical strings back for a settings file ([`crates/rune/README.md`][rune-crate-readme]).
- **The terminal takes the remaining keys in `on_key` on its focusable grid** (opencode-rune:
  [`grid/pointer.rs`][ocr-pointer], [`input.rs`][ocr-input], [`side_ops.rs`][ocr-side-ops]). Step 3
  comes before focus navigation, so Tab, Escape and Ctrl chords all reach the terminal
  (**verified**: Tab and Ctrl+C reached the grid). Return true to consume. Return false for keys
  the app owns, such as ⌘-chords on macOS, so they reach `AppSpec::on_key`. opencode-rune leaves F6
  as an escape hatch so the keyboard is never trapped in the shell.
- **Raw key data for the encoder.** `KeyEvent { key: Key, mods: Mods { shift, cmd, alt, ctrl,
  logo } }`. `Key` is `Text`, `Enter`, `Backspace`, `Delete`, arrows,
  `Home`/`End`/`PageUp`/`PageDown`, `Insert`, `PrintScreen`, `ScrollLock`, `Pause`, `Escape`, `Tab`,
  `Command(char)`, `Function(1..=24)`, `ContextMenu`, `Modifier { .. }` (opt-in) and `Media(_)`
  (opt-in) ([`crates/rune-edit/src/keys.rs`][rune-keys]). Off macOS a Ctrl chord arrives as
  `Key::Command(c)`. In a handler, `tx.physical_key()`, `tx.key_base()` and `tx.key_repeat()` give
  the physical key, the unmodified character and auto-repeat
  ([`crates/rune/src/el/tx/keys.rs`][rune-tx-keys]); `El::on_key_up` delivers releases. Together
  these cover the kitty keyboard protocol's press/repeat/release reporting without
  `on_physical_input`. `App::modifier_keys()` opts into lone modifier presses. Windows Ctrl+Break
  arrives as `Key::Pause` with Ctrl.
- **IME and the Option key.** Mark an element at the cursor cell `.ime_cursor()` and handle
  `App::on_ime(|s, ime, tx| ..)`. `Ime::Preedit` is drawn, not inserted; `Ime::Commit` carries only
  composed text, while plain keys arrive as `Key::Text`
  ([`crates/rune/README.md`][rune-crate-readme] "Text fields"). On macOS,
  `.ime_option_as_alt_mode(OptionAsAlt::Left | Right | Both)` beside `.ime_cursor()` makes Option
  act as Alt (Meta) for the terminal. opencode-rune exposes it as a setting and keeps the IME off
  at password prompts ([`grid/mod.rs`][ocr-grid]).
- **Focus.** Use `.focusable(true)`, `.tab_index(n)`, `tx.focus(key, ring)`, `tx.blur()`,
  `.on_focus_change` and `.focus_group()` (arrow-key roving for the sidebar list and tab strips),
  and `.modal(click_outside, dismiss)`, which traps Tab, handles Escape and restores the opener.
  `tx.focused()` returns only the focused element's own key, not its key path
  ([`crates/rune/src/el/tx/keys.rs`][rune-tx-keys]). So if every pane's grid is keyed `screen`
  under a `pane-<id>` parent, `tx.focused()` reads `"screen"` for all of them. To find the pane
  that has the keyboard, use `tx.focus_within("pane-<id>")` (it takes keys and `/`-separated key
  paths), or track the focused pane in `S` through `.on_focus_change`. opencode-rune compares
  `tx.focused()` against a single constant, which tells its one grid from other controls
  ([`side_ops.rs`][ocr-side-ops]). `/` separates key paths, so don't use it inside a single key.
- **Global (system-wide) shortcuts** come from `App::global_shortcut("cmd+shift+space", ..)` or
  `handle.register_shortcut`. The backends are Carbon on macOS, `RegisterHotKey` on Windows, X11
  grabs, and the xdg-desktop-portal on Wayland ([`crates/rune/src/shortcuts/README.md`][rune-shortcuts]).

## 5. Shell UI building blocks

| Need | Rune / rune-kit API | Notes |
| --- | --- | --- |
| Custom title bar with tabs in it | `App::titlebar(Titlebar::new(h))`, `.window_drag()`, `Cx::chrome_insets()`, `Cx::titlebar_inset()`; `Chrome { full_size_content, traffic_lights }` | macOS: full-size content and placed traffic lights. Windows: Rune draws the caption buttons and keeps snap layouts. Wayland (no SSD): Rune draws the buttons. On GNOME, `App::titlebar` can leave two bars (rune#591). Borderless resize grips: `El::window_resize(edge)` ([`crates/rune/README.md`][rune-crate-readme] "Window chrome") |
| Sidebar, resizable and collapsible | `rune_kit::shell::{SplitView, Sidebar, SidebarRow, PaneState, Toolbar}` | Resizing, double-click reset, collapse below half the minimum, a peek on edge hover, one Tab stop with arrow and type-select navigation, `virtual_rows` for long lists ([`crates/rune-kit/README.md`][kit-readme] "App shell") |
| Tabs | `rune_kit::TabStrip` (horizontal or vertical) | Spring open and close, shrink then scroll on overflow, drag to reorder, tear out, middle-click close ([`crates/rune-kit/src/shell/tab_strip.rs`][kit-tabs]) |
| Split panes with drag-to-resize | Hand-built: `el().cursor(Cursor::ResizeColumn).on_drag(..)` dividers that update ratios in state | No generic nested splitter in rune-kit. opencode-rune builds its split and bottom panels this way ([opencode-rune `features/panels/src/view/mod.rs`][ocr-panels-view]) |
| Command palette, pickers | `.modal(..)` container with `rune_kit::Combobox` or `PopupList::build_list` inside; `rune::Latest` for search | `Combobox` filters, loops with Up/Down and Ctrl-N/P, and ignores keys during IME composition ([`crates/rune-kit/README.md`][kit-readme] "Components") |
| Popovers, anchored overlays | `El::anchored(key, side, align, gap)`, `.above(key, gutter)`, `.raise()`; `rune_kit::{Popover, HoverCard, Tooltip, Select}` built into a `layer` passed to `rune_kit::root(body, layer)` | Anchored content flips and clamps to the window. Popups stay inside the window (rune#1369 tracks real popup windows) |
| Context menus | `tx.popup_menu(PopupMenu, anchor, on_pick)` | Native on macOS (`NSMenu`) and Windows (`TrackPopupMenuEx`). On Linux, `popup_menu_supported()` is false: handle `App::on_popup_menu` and render `rune_kit::Menu::from_popup_menu` ([`crates/rune/README.md`][rune-crate-readme] "Pop-up and context menus") |
| Toasts | `rune_kit::ToastStack` | Stacked, coalescing, timed ([`crates/rune-kit/src/overlays/toast/stack.rs`][kit-toast]) |
| Dialogs | `rune_kit::Dialog`, `.modal(..)` / `.modal_trap()` | Nested modals restore focus in stack order |
| Scroll | `.scroll(ScrollSpec::default())`, `stick_bottom()`, `scroll_x`, `scroll_xy`, `tx.scroll_into_view`, `tx.scroll_to_with` | Rune owns scrolling: no view re-run while scrolling. The terminal's scrollback is the emulator's, not a Rune scroll view (opencode-rune draws its own scrollbar) |
| Theming tokens | `rune::tok` (OKLCH palettes, spacing, radii, type, motion); `Theme::from_json`; `rune_kit::Kit::from_palette(PaletteSeed { bg, fg, accent })` | Colours animate in OKLab on theme change. `Cx::system_dark()` follows the OS ([`crates/rune/README.md`][rune-crate-readme], [`crates/rune-kit/README.md`][kit-readme]) |
| Custom fonts | `AppSpec { fonts: &[bytes], families: (ui, mono) }`, `Family::Named("…")`, `rune::text::add_font(bytes)` at runtime, `TextStyle.features = "'calt' 0, 'liga' 0"` | Per-span font families are not supported yet (rune#1093). Use `GridRow::cell` with its own `TextStyle` for glyphs from another face |
| SVG icons | `rune::Icon { name, view, body }` + `icon(&ICON, size)`; `Shape::fill_svg`, `Path::from_svg` | Icons are rasterized into the glyph atlas |
| Native menu bar | `App::menu_item(MenuItem::new(..).menu("File").key_equivalent(..))`, `handle.set_menu`, `App::standard_menus()` | macOS only; Windows and Linux return `MenuError::Unsupported` ([`crates/rune/src/shortcuts/README.md`][rune-shortcuts]) |
| System notifications | `rune-notify`: `register`, `post`, `on_click` | macOS `UNUserNotificationCenter`, Windows toasts, Linux `org.freedesktop.Notifications`. On Linux, call after the first frame ([`crates/rune-notify/README.md`][rune-notify]) |
| Clipboard | `tx.copy`, `copy_rich`, `copy_secret`, `tx.request_paste` + `.on_paste`, `tx.read_clipboard(|s, content, tx| ..)` | Asynchronous reads. Headless runs never touch the system clipboard |
| Multiple windows, tray, single instance | `run_windows`, `App::window`, `rune::open_window`, `Tray`, `rune::single_instance` | See §2 |

## 6. Headless tooling and how workers should test UI

Every debug build of an app run through `rune::run`, `App::run` or `run_windows` accepts Rune's
headless flags. They use the same renderer, offscreen, on a fixed virtual clock
([`crates/rune/README.md`][rune-crate-readme] "Agent tooling"):

```sh
B=target/debug/herdr-frontend
$B --snap out.png [--scene NAME] [--at 0.4] [--settle] [--size 1200x800] [--scale 2] [--theme dark]
$B --tree tree.json [--at T] [--settle]     # every element: key, path, role, label, text, rect, colours, focus
$B --script flow.json --tree end.json --at mark:end
$B --script flow.json --frames 30 --out frames/  # or --strip / --video
$B --list-scenes ; $B --window KEY ...          # pick a window of a multi-window app
```

- **Scripts** are `{"events": [{"t": 0.1, "type": "tap", "key": "screen"}, …]}`. Events include
  `move`, `down`, `up`, `tap`, `double-click`, `drag`, `key` (`name`, `cmd`/`shift`/`alt`/`ctrl`,
  optional physical `code`), `key-up`, `type`, `paste`, `wheel`, `resize`, `shortcut` (a global
  chord), `menu-pick`/`menu-cancel`, and `mark` (to name times). **Verified:** a script that tapped
  the grid, then pressed Tab and Ctrl+C, left the expected text in `--tree`.
- **Scenes** (`AppSpec::scenes: &[(name, fn(&mut S))]`) put the app in a named state for `--snap`.
  Give each worker-visible state a scene: empty core, a connected core, a split layout, the palette
  open, a disconnect banner.
- **Unit tests** drive the real runtime with no window (**verified**):

  ```rust
  let mut rt = App::new(spec()).headless_runtime((1200.0, 800.0), Env::new(true, 2.0));
  rt.frame(0.0);
  let (x, y) = rt.find("pane-1/screen").unwrap();
  rt.pointer_down(x, y); rt.pointer_up(x, y); rt.frame(0.0);
  assert_eq!(rt.presented_focus_key(), Some("screen")); // the focused element's own key, not "pane-1/screen"
  rt.key(KeyEvent { key: Key::Tab, mods: Mods::default() }); rt.frame(0.0); // reaches the grid's on_key
  let status = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("status"));
  ```

  `headless_runtime` runs `init`, `setup` (with a no-op waker), `poll` and `on_key`, but never
  `on_ready`. Drive input with `pointer_*`, `key`, `paste`, `ime` and `wheel_xy`, and step time with
  `rt.frame(dt)`. Inspect `rt.state`, `rt.elements()`, `rt.accessibility_json()` and
  `rt.popup_menu()`, and answer menus with `rt.resolve_popup(..)`
  ([`crates/rune/README.md`][rune-crate-readme] "Testing an app headless").
- **System fonts differ by OS**, so `--tree` metrics and `--snap` text differ between macOS and
  Linux CI. Set `RUNE_SYSTEM_FONTS=off` and bundle every face the app shows to make them match.
- **Worker rule of thumb:** for any UI change, add or extend a scene and a headless test, and look
  at a `--snap` PNG before opening the PR. Never test against the user's real herdr-core: give the
  app a fixture mode, or point it at an isolated core (see [`AGENTS.md`](../../AGENTS.md)
  "Safety").
- Live Rust editing (`scripts/rune-dev`, `hot` feature) exists, but it is built around Rune
  maintainers' remote build workers; `--local` runs it on one machine. Not verified here
  ([`crates/rune/README.md`][rune-crate-readme] "Live Rust editing").

## 7. Gaps and bugs that affect this app

| Gap | Impact | Workaround |
| --- | --- | --- |
| No in-app drag and drop with payloads ([rune#357](https://github.com/anomalyco/rune/issues/357)) | Dragging a tab onto a pane or into a split | The dragged element's `on_drag` hit-tests the pointer against rects read with `Cx::layout` / `Tx::read_layout` (or `rt.element_at_with` + `El::data`), and the app draws the drop highlight. `TabStrip` already reorders and tears out within one strip |
| A panic in a view or handler ends the process ([rune#1003](https://github.com/anomalyco/rune/issues/1003)) | One rendering bug closes every window | Keep views panic-free (no `unwrap` on core data). Herdr state lives in herdr-core, so a restart loses nothing durable |
| Keys for an open popup while another element keeps focus ([rune#1365](https://github.com/anomalyco/rune/issues/1365)) | Autocomplete over a focused field | Give the palette or picker focus as a `.modal`, or forward keys from the field's `on_key` |
| `on_focus_lost` can't tell window deactivation from an in-window focus move ([rune#2449](https://github.com/anomalyco/rune/issues/2449)) | Rename-in-place fields commit on Alt-Tab | Poll `TextEdit::focused` |
| Linux: no native pop-up menus | Context menus | `App::on_popup_menu` + `rune_kit::Menu::from_popup_menu` |
| Linux GNOME Wayland: `App::titlebar` shows two bars, or no controls ([rune#591](https://github.com/anomalyco/rune/issues/591)) | Custom title-bar tabs on GNOME | Accept the system bar on GNOME, or put tabs below it |
| Linux: no system drag and drop ([rune#171](https://github.com/anomalyco/rune/issues/171), [#526](https://github.com/anomalyco/rune/issues/526)); wheel and trackpad quirks ([#898](https://github.com/anomalyco/rune/issues/898)) | Dropping files onto a terminal on Linux | Feature-gate, and test on macOS first |
| No per-span font family or OpenType features ([rune#1093](https://github.com/anomalyco/rune/issues/1093)) | Mixing a Nerd Font symbol face with the main face in one row | `GridRow::cell` with its own `TextStyle`, or a family list such as `"User Font, Symbols Nerd Font Mono"` |
| Native menu bar is macOS only | Windows and Linux menus | In-window menus (`rune_kit::Menu`), or a keymap with the palette |
| No platform message dialog ([rune#637](https://github.com/anomalyco/rune/issues/637)) | Confirming before a window exists or while quitting | In-window `Dialog` with `on_close_request` / `on_quit_request` (macOS) |
| Displays changing at runtime are not reported | Moving a window between monitors | `App::on_window_geometry` reports the scale; `rune::displays()` re-reads on demand |
| Very high churn | Bumps can break builds | Pin; bump on purpose with `check_rune_pins.py`; read `CHANGELOG.md` and `changelog/unreleased/` |

## 8. Recommended app skeleton

### Crates

```text
Cargo.toml                 # workspace; Rune pin + the [patch.crates-io] block; dev opt-levels (§1)
rust-toolchain.toml        # 1.99.1
crates/
  core-client/             # herdr-core socket client: framing, typed requests/events, reconnect machine.
                           # No Rune dependency; runs on its own thread; tested against an isolated core.
  terminal/                # libghostty-vt wrapper + per-terminal worker thread: feed bytes, key/mouse
                           # encoding, snapshot extraction, Handoff, Waker. Depends on rune only for Waker
                           # and Instant (see terminal-rendering.md).
  model/                   # Pure state + rune::machine machines: workspaces, spaces, tabs, pane tree,
                           # focus, keymap modes, palette. No I/O; exhaustively unit-tested.
  app/                     # The binary: AppSpec/App wiring, views, rune-kit shell, scenes, headless tests.
    src/main.rs            # build App, init from args, run_windows
    src/state.rs           # S: model + channels + terminal handles + UI-only state (Tracked fields)
    src/poll.rs            # drain core events -> model::step; read terminal snapshots; timers
    src/keys.rs            # one keymap + mode machine, called from the root on_key_preview and from
                           # AppSpec::on_key (nothing focused); the focused pane via tx.focus_within
    src/view/{shell,sidebar,tabs,panes,terminal,palette,toasts,titlebar}.rs
    src/scenes.rs          # named states for --snap
    tests/                 # headless_runtime tests
```

Keep `model` free of Rune types where practical, so core logic tests run without a GPU and
`core-client` can be shared with other tools. Put the `rune::machine` state machines in `model`
(`rune-core` alone is enough if Rune's types are wanted there).

### State and event flow

```text
 herdr-core socket ──► core-client thread ──mpsc──┐
                                                  ├─► waker() ─► next frame: poll(&mut S)
 terminal worker (libghostty-vt) ─ Handoff ───────┘                 │
        ▲ bytes / key / mouse / resize                              ├─ drain core events → model::step → Effects
        │                                                           │      (send request, open terminal worker, toast…)
        │                                                           ├─ read snapshots of visible terminals
        │                                                           └─ return true only if something visible changed
        │
 view(&S, cx) ─► rune-kit shell (titlebar · sidebar · tab strip · pane tree)
        │          └─ terminal pane: grid(cols, cw, ch, cached Rc<GridRow>) + cursor/IME/find overlays
        │
 input ─► root on_key_preview / AppSpec::on_key (one keymap + prefix-mode machine) ─► model::step
        └► focused grid on_key / on_key_up / IME ─► terminal worker (encode → core)
```

- **One source of truth.** herdr-core owns workspaces, terminals and labels; the app mirrors them
  from core events. UI-only state (pane ratios, palette query, focus) lives in `S`. Never write
  shared layout documents or labels in a form Herdr app can't read (see
  [`AGENTS.md`](../../AGENTS.md)).
- **Effects, not I/O, in handlers.** Handlers and `poll` call `model::step` and run the returned
  effects by sending on channels. Nothing blocks the UI thread.
- **Only visible terminals cost frames.** Read a terminal's snapshot only when its pane is shown,
  as opencode-rune does with `snapshot()` against `peek()`. Hidden terminals keep parsing output on
  their worker but build no rows and never wake the UI.
- **Keys are stable herdr ids** (`space-<id>`, `tab-<id>`, and each grid `screen` under its
  `pane-<id>`, addressed as the path `pane-<id>/screen`), so scripts, tests
  and focus all address the same elements.

### `model` as built

`crates/model` depends on `herdr-proto` and `herdr-shared` only, with no Rune types. Its shape:

- **One transition.** `Model::step(now, Event | Action) -> Effects`. `now` is the app clock
  (Rune's `Tx::clock`), passed with every input so deadlines never run on a stale time. Events
  are core news (`Connected`, `Lost`, `Terminals(TerminalUpdate)` built with
  `TerminalUpdate::from_changes`, `Machines`), replies (`DocRead`, `DocWritten`,
  `Reply { token }`, `StreamClosed`), input (`Key { chord, terminal }`), and `Tick` when a
  `WakeAt` comes due.
  `Effects` lists what to do (`Call` with a `Token`, `Doc` rounds, `Terminal` commands, `App`,
  `Focus`, `Toast`, `Alert`, `Pick`, `NotYet`, `WakeAt`, `SaveView`, `Answer`), plus `handled`:
  whether a key was used, so it must not reach the terminal.
- **Control calls.** A `do` from the control socket is a third input, `Call { id, name, args }`,
  answered by exactly one `Effect::Answer { id, reply }`, in the same step or once a launch
  returns. `Model::ls` and `Model::status` are read-only (`docs/cli.md`).
- **World and view.** `World` holds one `Mirror` per machine (this one first, then by name;
  temporary machines left out) and the terminal records. `View` holds this screen's choices by
  id and falls back to the first space, tab or pane. `View::reconcile` forgets ids a read layout
  lost, keeps a machine's ids until its layout is read, and replaces a closed space or tab with
  the one that slides into its place. Eli's close rule (left first, skipping hidden and settled
  tabs; an agent's tab hands over to the next agent) runs on top for the shown space. After each
  step the shown space, tab and pane are written down, so a swap or a tab moved first can't move
  what is shown. `View::to_json` is the saved file; the jump list is in memory only.
- **Launches are two-phase.** `tab.new`, splits and new spaces make their ids, send
  `terminal.run` with `herdr.space`/`herdr.tab`/`herdr.seen=0`, `keep` and herdr-app's `TERM`
  env, and put the pane in the layout only when the reply brings the address. A labeled terminal
  whose tab has a launch pending is not adopted meanwhile. A launch on a machine whose layout
  can't take an edit is refused before any terminal starts. The model also adopts labeled
  orphans, removes panes whose stream ended or whose terminal the core no longer has, and backs a
  failing machine's layout reads off for `RETRY` (2 s).
- **Keys.** `Keymap::eli()` is §22's effective table as data (`Chord` in Eli's syntax); the key
  file and live reload are ticket 016's. `Mode` is `Normal`, `Prefix` (dropped after 1 s, GPUI's
  pending-key timeout; an unbound sequence is swallowed whole), `Popup` or `Copy`. Popups and
  copy mode read their own keys (§22), then fall back to one-key global bindings, as Herdr app
  runs window keys over a dialog; `tab.close`/`pane.close` close the popup or leave copy mode
  instead. List popups keep a query and a wrapping cursor; the feature that fills them reports
  `PopupRows` and resolves `Effect::Pick`. Prompts, confirms and the agent-jump bar resolve in the model.
- **Not built here** (they return `Effect::NotYet`): Neovim-aware Ctrl+H/J/K/L where the editor
  label is set, `pane.resize`, the review view, peek, menus, sub-agent folds, links and pane drag. Writing
  `herdr.seen` labels (`SeenTracker`) and drawing alerts are ticket 025's; the model keeps
  `Attention` for displayed status and marks what the person looks at as seen.

### The app shell as built

`crates/app` wires the model to a live core and draws it (ticket 014):

- **`link`.** A `Link` trait the UI thread drives without blocking and drains once a frame in
  `AppSpec::poll`. `Live` runs herdr-client's `Supervisor` and `MachineWatcher`, makes calls and
  document rounds whose `Pending`s it `try_take`s, and reads every attach stream on one thread
  (`hc-streams`), which writes a terminal's output straight into its terminal worker through a
  `terminal::Writer`. The worker returns the credit once it parsed the bytes (`write_then`), so
  a parser that falls behind holds the stream to the core's 256 KiB window. The UI hears only
  when a stream starts or ends, so a hidden terminal's output never wakes it. `Fake` is an in-process core for scenes and tests:
  it answers on the next poll, paints each terminal's recorded screen on attach, and keeps
  every frame sent.
- **`State::pump`.** Link news goes through `Model::step`; effects become link calls, focus
  (`Tx::focus_when_shown` on `pane-<id>`), wakes and view saves. One `Viewer` per terminal the
  shown tab draws: a `Terminal` worker and the `TerminalPane` drawing its snapshots. Viewers
  stay attached while their pane is in the layout, shown or not.
- **Viewer policy** (`herdr-core-protocol.md` §4). Every terminal attaches `resumed: true` at
  the record's current grid, with `history: 2000` and this window's `display`. The pane in use,
  the focused pane while the window is focused, sends FACTS, then RESIZE with its grid (Ghostty's
  whole-pixel cell, `terminal::Size::report`, no padding, grid at the top left), then FOCUS(1);
  RESIZE again on each grid change at most every 25 ms; FOCUS(0) when it stops being in use. The
  local terminal follows the record's `cols` × `rows` (kept by `size` events), never the pane.
  After a reconnect or a `restarted` event, viewers reattach `resumed` and reset their screen
  for the repaint.
- **Window.** Borderless (`WindowOptions::borderless`): AppKit's borderless style has square
  corners, no title bar and no traffic lights. `Chrome { drag_regions: true }` makes only the
  38 pt sidebar top row and tab strip (`.window_drag()`) move it. The view is the sidebar
  placeholder (366 pt), the tab strip placeholder, and the tab area, where panes sit at the
  model's `pane_rects` (`Geometry { gap: 1 }`) over a border-colored fill. The tab area's
  `on_layout_tx` reports the geometry; before the first layout it comes from the window size
  (`App::on_window_geometry`, `SIZE` headless).
- **Input.** Plain typing only (`keys::encode`): text, Return, Backspace, Tab, Escape, arrows
  and editing keys, and Ctrl with a letter, encoded by the terminal's worker for the program's
  modes and sent as DATA. The focused pane's element takes keys in `El::on_key`; with nothing
  focused, `AppSpec::on_key` types into the model's focused pane and gives it focus back. A click
  focuses a pane (`Action::FocusPane`).
- **Home** (`$HERDR_FRONTEND_HOME`, else `$XDG_CONFIG_HOME/herdr-frontend`, else
  `~/.config/herdr-frontend`): `view.json` (the model's saved view) and `window.json` (position,
  physical size, maximized, full screen, display). Writes go through a saver thread, written
  whole and renamed; close and quit write at once. A saved place whose display is gone opens
  centered.
- **Headless.** `--snap`, `--tree` and `--script` never connect: `main` runs the app with no
  core. The `shell*` scenes run on `Fake`; `State::settle` makes every pump wait on the
  terminals (`Terminal::snapshot_now`) so a frame never races a worker. While the screen is
  locked, macOS reports the real window covered and Rune draws no frames.

[rune]: https://github.com/anomalyco/rune/tree/4918969f04dce020f63b34e024368a86e5e1857d
[rune-readme]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/README.md
[rune-agents]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/AGENTS.md
[rune-cargo]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/Cargo.toml
[rune-toolchain]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/rust-toolchain.toml
[rune-changelog]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/CHANGELOG.md
[rune-pins]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/scripts/check_rune_pins.py
[rune-crate-readme]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/README.md
[rune-crate-cargo]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/Cargo.toml
[rune-appspec]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/src/tooling.rs
[rune-tracked]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune-core/src/tracked.rs
[rune-memo]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/src/memo.rs
[rune-fast]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/site/src/FastApps.tsx
[rune-waker]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune-core/src/lib.rs
[rune-task]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune-core/src/task.rs
[rune-grid]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/src/el/grid.rs
[rune-grid-changelog]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/changelog/unreleased/2026-11-13-grid.md
[rune-grid-a11y]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/changelog/unreleased/2026-11-14-a11y-grid-lines.md
[rune-grid-caret]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/changelog/unreleased/2026-11-13-grid-caret.md
[rune-physical]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/src/runtime/input/physical.rs
[rune-input-keys]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/src/runtime/input/keys.rs
[rune-chord]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/src/input/chord.rs
[rune-keys]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune-edit/src/keys.rs
[rune-tx-keys]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/src/el/tx/keys.rs
[rune-shortcuts]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune/src/shortcuts/README.md
[rune-notify]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune-notify/README.md
[kit-readme]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune-kit/README.md
[kit-tabs]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune-kit/src/shell/tab_strip.rs
[kit-toast]: https://github.com/anomalyco/rune/blob/4918969f04dce020f63b34e024368a86e5e1857d/crates/rune-kit/src/overlays/toast/stack.rs
[ocr]: https://github.com/anomalyco/opencode-rune/tree/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c
[ocr-cargo]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/Cargo.toml
[ocr-toolchain]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/rust-toolchain.toml
[ocr-agents]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/AGENTS.md
[ocr-terminal]: https://github.com/anomalyco/opencode-rune/tree/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/crates/terminal
[ocr-publish]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/crates/terminal/src/publish.rs
[ocr-pty]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/crates/terminal/src/pty.rs
[ocr-grid]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/features/terminal-surface/src/grid/mod.rs
[ocr-rows]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/features/terminal-surface/src/grid/rows.rs
[ocr-draw]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/features/terminal-surface/src/draw.rs
[ocr-pointer]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/features/terminal-surface/src/grid/pointer.rs
[ocr-input]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/features/terminal-surface/src/input.rs
[ocr-side-ops]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/features/terminal-surface/src/side_ops.rs
[ocr-panels-view]: https://github.com/anomalyco/opencode-rune/blob/a1d6a19ce1f9b136de916b7a7e6393ffb10f956c/features/panels/src/view/mod.rs
