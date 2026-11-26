//! [`Vt`]: one libghostty-vt terminal with its effects, its render state and its encoders, used
//! from one thread.
//!
//! A [`crate::Terminal`] runs a `Vt` on its worker thread. A `Vt` can also be driven directly,
//! with no thread at all, as the replay tests do.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ghostty_vt_sys::*;

use crate::input::{KeyAction, KeyEvent, MouseAction, MouseEvent, OptionAsAlt, PasteRejected};
use crate::snapshot::{
    ActiveScreen, Cell, CellFlags, Colors, Cursor, CursorStyle, Dirty, Grapheme, Modes, Rgb, Row,
    RowId, Scrollbar, Snapshot, Underline, Width,
};

/// How a terminal starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub cols: u16,
    pub rows: u16,
    /// The cell size in whole device pixels, for size reports and the mouse. Zero until the
    /// metrics are known: size queries then go unanswered.
    pub cell_width: u32,
    pub cell_height: u32,
    pub scrollback: Scrollback,
    pub theme: Theme,
    /// What `CSI ? 996 n` answers. `None` leaves the query unanswered.
    pub color_scheme: Option<ColorScheme>,
    /// Let programs write the clipboard (OSC 52 and the like), up to `clipboard_max_bytes`.
    /// Programs can never read it.
    pub clipboard_write: bool,
    pub clipboard_max_bytes: usize,
    pub option_as_alt: OptionAsAlt,
    /// How long a program may hold its frame (synchronized output) before it shows anyway.
    pub render_hold_timeout: Duration,
}

impl Default for Config {
    /// Herdr app's terminal settings: 10,000 lines or 10 MB of scrollback, and clipboard
    /// writes up to 1 MiB (`docs/research/feature-inventory.md` §17).
    fn default() -> Config {
        Config {
            cols: 80,
            rows: 24,
            cell_width: 0,
            cell_height: 0,
            scrollback: Scrollback::default(),
            theme: Theme::default(),
            color_scheme: None,
            clipboard_write: true,
            clipboard_max_bytes: 1 << 20,
            option_as_alt: OptionAsAlt::default(),
            render_hold_timeout: Duration::from_secs(1),
        }
    }
}

/// Scrollback limits. History is pruned in whole pages, so both are approximate (a little over
/// or under), and whichever is reached first applies. libghostty's own default is only 10,000
/// bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scrollback {
    /// `None` for no line limit.
    pub max_lines: Option<usize>,
    /// `None` for no byte limit. Zero turns scrollback off.
    pub max_bytes: Option<usize>,
}

impl Default for Scrollback {
    fn default() -> Scrollback {
        Scrollback {
            max_lines: Some(10_000),
            max_bytes: Some(10_000_000),
        }
    }
}

/// The colors a terminal starts with, before a program changes them. `None` keeps
/// libghostty's default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Theme {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    /// Palette entries to replace, such as the 16 ANSI colors.
    pub palette: Vec<(u8, Rgb)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
}

/// Where the viewport goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    Top,
    Bottom,
    /// Rows; up is negative.
    Delta(isize),
    /// The first visible row, counted from the top of the scrollback (`Scrollbar::offset`).
    Row(usize),
    /// Shell prompts (OSC 133); back is negative.
    Prompts(isize),
}

/// Something the program asked for that isn't a reply: the app decides what it means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Bell,
    Title(String),
    /// The raw OSC 7 value, often a `file://` URI.
    Pwd(String),
    /// The program wrote the clipboard. `None` asks to clear it.
    Clipboard {
        location: ClipboardLocation,
        text: Option<String>,
    },
    /// OSC 9 or 777. Shown in the app only, never as an OS notification.
    Notification {
        title: String,
        body: String,
    },
    Progress(Progress),
    /// The program fully reset the terminal (RIS).
    Reset,
    /// A paste sent to the worker was refused; see [`PasteRejected`].
    PasteRejected(PasteRejected),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardLocation {
    Standard,
    Selection,
    Primary,
}

/// An OSC 9;4 progress report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Remove,
    Set(u8),
    Error(Option<u8>),
    Indeterminate,
    Pause(Option<u8>),
}

/// DEC private mode 2026.
const SYNCHRONIZED_OUTPUT: GhosttyMode = 2026;
/// DEC private mode 2027, on from the start as in Ghostty and Herdr app: VS16 makes an emoji
/// two cells wide.
const GRAPHEME_CLUSTER: GhosttyMode = 2027;
const BRACKETED_PASTE: GhosttyMode = 2004;
const FOCUS_EVENT: GhosttyMode = 1004;

/// The state callbacks write into, behind the terminal's userdata pointer.
struct Effects {
    /// Bytes for the program, from WRITE_PTY.
    pty: Vec<u8>,
    events: Vec<Event>,
    /// What the SIZE callback reports.
    size: GhosttySizeReportSize,
    color_scheme: Option<ColorScheme>,
    clipboard_write: bool,
    /// The render state, so a hold can capture the frame the moment it begins.
    state: GhosttyRenderState,
    /// What the terminal said when `state` was last updated.
    meta: Meta,
    /// When the current render hold began.
    hold: Option<Instant>,
    /// `state` holds a frame captured at the start of a hold that no snapshot has read yet.
    captured: bool,
}

/// Terminal data read together with each render-state update, so it describes the same
/// moment.
#[derive(Clone, Copy, Debug, Default)]
struct Meta {
    scrollbar: Scrollbar,
    active_screen: ActiveScreen,
    mouse_tracking: bool,
    bracketed_paste: bool,
    focus_reporting: bool,
    kitty_keyboard: u8,
}

/// One libghostty-vt terminal. Not `Send`: it stays on the thread that made it.
pub struct Vt {
    term: GhosttyTerminal,
    /// The terminal's userdata, owned by the `Vt` and freed after the terminal. A raw pointer
    /// rather than a `Box`, so the library's copy stays valid while the `Vt` moves and uses it:
    /// every access goes through it ([`Vt::effects`]).
    effects: NonNull<Effects>,
    rows_iter: GhosttyRenderStateRowIterator,
    cells: GhosttyRenderStateRowCells,
    keys: GhosttyKeyEncoder,
    key_event: GhosttyKeyEvent,
    mouse: GhosttyMouseEncoder,
    mouse_event: GhosttyMouseEvent,
    option_as_alt: OptionAsAlt,
    hold_timeout: Duration,
    /// Rows of the last snapshot by id.
    rows_by_id: HashMap<RowId, Arc<Row>>,
    /// What the last snapshot was built with: rows from other colors or widths are stale.
    built_with: Option<(Colors, u16)>,
    generation: u64,
}

impl Vt {
    pub fn new(config: &Config) -> Vt {
        let cols = config.cols.max(1);
        let rows = config.rows.max(1);
        // SAFETY: every handle is created here, checked, and freed once in `Drop`. The userdata
        // pointer is the boxed `Effects`, whose address is stable for the terminal's life, and
        // each callback's signature matches its option's documented type.
        unsafe {
            let mut term: GhosttyTerminal = ptr::null_mut();
            ok(
                ghostty_terminal_new(ptr::null(), &mut term, cols, rows),
                "ghostty_terminal_new",
            );
            let mut state: GhosttyRenderState = ptr::null_mut();
            ok(
                ghostty_render_state_new(ptr::null(), &mut state),
                "ghostty_render_state_new",
            );
            let mut rows_iter: GhosttyRenderStateRowIterator = ptr::null_mut();
            ok(
                ghostty_render_state_row_iterator_new(ptr::null(), &mut rows_iter),
                "ghostty_render_state_row_iterator_new",
            );
            let mut cells: GhosttyRenderStateRowCells = ptr::null_mut();
            ok(
                ghostty_render_state_row_cells_new(ptr::null(), &mut cells),
                "ghostty_render_state_row_cells_new",
            );
            let mut keys: GhosttyKeyEncoder = ptr::null_mut();
            ok(
                ghostty_key_encoder_new(ptr::null(), &mut keys),
                "ghostty_key_encoder_new",
            );
            let mut key_event: GhosttyKeyEvent = ptr::null_mut();
            ok(
                ghostty_key_event_new(ptr::null(), &mut key_event),
                "ghostty_key_event_new",
            );
            let mut mouse: GhosttyMouseEncoder = ptr::null_mut();
            ok(
                ghostty_mouse_encoder_new(ptr::null(), &mut mouse),
                "ghostty_mouse_encoder_new",
            );
            let mut mouse_event: GhosttyMouseEvent = ptr::null_mut();
            ok(
                ghostty_mouse_event_new(ptr::null(), &mut mouse_event),
                "ghostty_mouse_event_new",
            );
            let track_last_cell = true;
            ok(
                ghostty_mouse_encoder_set(
                    mouse,
                    GHOSTTY_MOUSE_ENCODER_OPT_TRACK_LAST_CELL,
                    (&raw const track_last_cell).cast(),
                ),
                "mouse TRACK_LAST_CELL",
            );

            let effects = NonNull::from(Box::leak(Box::new(Effects {
                pty: Vec::new(),
                events: Vec::new(),
                size: GhosttySizeReportSize {
                    rows,
                    columns: cols,
                    cell_width: config.cell_width,
                    cell_height: config.cell_height,
                },
                color_scheme: config.color_scheme,
                clipboard_write: config.clipboard_write,
                state,
                meta: Meta::default(),
                hold: None,
                captured: false,
            })));
            let userdata: *mut Effects = effects.as_ptr();
            set(
                term,
                GHOSTTY_TERMINAL_OPT_USERDATA,
                userdata.cast_const().cast(),
            );
            set_callbacks(term);

            let grapheme = GhosttyTerminalModeConfig {
                mode: GRAPHEME_CLUSTER,
                value: true,
            };
            set(
                term,
                GHOSTTY_TERMINAL_OPT_MODE_DEFAULT,
                (&raw const grapheme).cast(),
            );
            let clipboard_max = config.clipboard_max_bytes;
            set(
                term,
                GHOSTTY_TERMINAL_OPT_CLIPBOARD_WRITE_MAX_BYTES,
                (&raw const clipboard_max).cast(),
            );

            let mut vt = Vt {
                term,
                effects,
                rows_iter,
                cells,
                keys,
                key_event,
                mouse,
                mouse_event,
                option_as_alt: config.option_as_alt,
                hold_timeout: config.render_hold_timeout,
                rows_by_id: HashMap::new(),
                built_with: None,
                generation: 0,
            };
            vt.set_scrollback(config.scrollback);
            vt.set_theme(&config.theme);
            if config.cell_width > 0 && config.cell_height > 0 {
                vt.resize(cols, rows, config.cell_width, config.cell_height);
            }
            vt
        }
    }

    /// Feeds the program's output.
    pub fn write(&mut self, bytes: &[u8]) {
        // SAFETY: `term` is live and `bytes` is readable for its length. Callbacks run during
        // the call and only touch `effects`.
        unsafe { ghostty_terminal_vt_write(self.term, bytes.as_ptr(), bytes.len()) };
    }

    /// The bytes the terminal wrote back to the program since the last call: query replies,
    /// and the mode 2048 size report a resize sends. A herdr-core viewer sends them as
    /// AUTOMATIC_INPUT.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.effects_mut().pty)
    }

    /// The effects since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.effects_mut().events)
    }

    /// Resizes the grid. The cell size, in whole device pixels, is what size reports and the
    /// mouse use. Ends a render hold.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u32, cell_height: u32) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        // Set first: a program with mode 2048 on gets a size report from inside the resize.
        self.effects_mut().size = GhosttySizeReportSize {
            rows,
            columns: cols,
            cell_width,
            cell_height,
        };
        // SAFETY: `term` is live.
        let result =
            unsafe { ghostty_terminal_resize(self.term, cols, rows, cell_width, cell_height) };
        ok(result, "ghostty_terminal_resize");
    }

    pub fn set_scrollback(&mut self, limits: Scrollback) {
        let lines = limits.max_lines.unwrap_or(usize::MAX);
        let bytes = limits.max_bytes.unwrap_or(usize::MAX);
        // SAFETY: `term` is live and each value is the `size_t` its option takes.
        unsafe {
            set(
                self.term,
                GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES,
                (&raw const lines).cast(),
            );
            set(
                self.term,
                GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
                (&raw const bytes).cast(),
            );
        }
    }

    /// Sets the default colors. Colors a program set itself (OSC 4, 10, 11, 12) still win.
    pub fn set_theme(&mut self, theme: &Theme) {
        // SAFETY: `term` is live; each value is the type its option takes, or null to restore
        // the library default.
        unsafe {
            let color = |rgb: Option<Rgb>| {
                rgb.map(|c| GhosttyColorRgb {
                    r: c.r,
                    g: c.g,
                    b: c.b,
                })
            };
            let set_color = |option, rgb: Option<GhosttyColorRgb>| match rgb {
                Some(c) => set(self.term, option, (&raw const c).cast()),
                None => set(self.term, option, ptr::null()),
            };
            set_color(
                GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND,
                color(theme.foreground),
            );
            set_color(
                GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND,
                color(theme.background),
            );
            set_color(GHOSTTY_TERMINAL_OPT_COLOR_CURSOR, color(theme.cursor));
            // Back to the built-in palette first: the default palette is whatever the last
            // theme set, and a theme only lists the entries it replaces.
            set(self.term, GHOSTTY_TERMINAL_OPT_COLOR_PALETTE, ptr::null());
            if !theme.palette.is_empty() {
                let mut palette = [GhosttyColorRgb::default(); 256];
                ok(
                    ghostty_terminal_get(
                        self.term,
                        GHOSTTY_TERMINAL_DATA_COLOR_PALETTE_DEFAULT,
                        palette.as_mut_ptr().cast(),
                    ),
                    "COLOR_PALETTE_DEFAULT",
                );
                for &(index, c) in &theme.palette {
                    palette[usize::from(index)] = GhosttyColorRgb {
                        r: c.r,
                        g: c.g,
                        b: c.b,
                    };
                }
                set(
                    self.term,
                    GHOSTTY_TERMINAL_OPT_COLOR_PALETTE,
                    palette.as_ptr().cast(),
                );
            }
        }
    }

    pub fn scroll(&mut self, scroll: Scroll) {
        let (tag, value) = match scroll {
            Scroll::Top => (
                GHOSTTY_TERMINAL_SCROLL_VIEWPORT_TOP,
                GhosttyTerminalScrollViewportValue { delta: 0 },
            ),
            Scroll::Bottom => (
                GHOSTTY_TERMINAL_SCROLL_VIEWPORT_BOTTOM,
                GhosttyTerminalScrollViewportValue { delta: 0 },
            ),
            Scroll::Delta(delta) => (
                GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA,
                GhosttyTerminalScrollViewportValue { delta },
            ),
            Scroll::Row(row) => (
                GHOSTTY_TERMINAL_SCROLL_VIEWPORT_ROW,
                GhosttyTerminalScrollViewportValue { row },
            ),
            Scroll::Prompts(delta_prompt) => (
                GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA_PROMPT,
                GhosttyTerminalScrollViewportValue { delta_prompt },
            ),
        };
        // SAFETY: `term` is live, and the union field matches the tag.
        let result = unsafe {
            ghostty_terminal_scroll_viewport(
                self.term,
                GhosttyTerminalScrollViewport { tag, value },
            )
        };
        ok(result, "ghostty_terminal_scroll_viewport");
    }

    /// Resets the terminal as RIS would. Ends a render hold.
    pub fn reset(&mut self) {
        // SAFETY: `term` is live.
        unsafe { ghostty_terminal_reset(self.term) };
    }

    /// When the current render hold began, if the program is holding its frame.
    pub fn hold_started(&self) -> Option<Instant> {
        self.effects().hold
    }

    /// When the current hold must end, if there is one.
    pub fn hold_deadline(&self) -> Option<Instant> {
        self.effects().hold.map(|since| since + self.hold_timeout)
    }

    /// Ends a hold that lasted past its timeout, as Ghostty does after one second: the program
    /// may have crashed or forgotten to release it. True if it ended one.
    pub fn expire_hold(&mut self, now: Instant) -> bool {
        match self.hold_deadline() {
            Some(deadline) if now >= deadline => {
                let mode = GhosttyTerminalModeConfig {
                    mode: SYNCHRONIZED_OUTPUT,
                    value: false,
                };
                // SAFETY: `term` is live and the value is a `GhosttyTerminalModeConfig`. This
                // doesn't invoke the render hold callback.
                unsafe {
                    set(
                        self.term,
                        GHOSTTY_TERMINAL_OPT_MODE,
                        (&raw const mode).cast(),
                    )
                };
                self.effects_mut().hold = None;
                self.effects_mut().captured = false;
                true
            }
            _ => false,
        }
    }

    /// A snapshot taken now would show a frame the last one didn't: always outside a hold, and
    /// during one only until the frame captured when it began has been read.
    pub fn can_show(&self) -> bool {
        self.effects().hold.is_none() || self.effects().captured
    }

    /// The current frame. Outside a render hold that's the terminal as it is now; during one,
    /// the frame from when the hold began.
    pub fn snapshot(&mut self) -> Snapshot {
        // SAFETY: `term`, the render state, the iterators and the encoders are live. Row and
        // cell data are read before the next update, and every out pointer has its key's type.
        unsafe {
            let state = self.effects().state;
            if self.effects().hold.is_none() {
                self.effects_mut().meta = update(state, self.term);
            }
            self.effects_mut().captured = false;
            let meta = self.effects().meta;

            let cols: u16 = state_get(state, GHOSTTY_RENDER_STATE_DATA_COLS);
            let rows: u16 = state_get(state, GHOSTTY_RENDER_STATE_DATA_ROWS);
            let mut raw_colors = GhosttyRenderStateColors {
                size: size_of::<GhosttyRenderStateColors>(),
                ..Default::default()
            };
            ok(
                ghostty_render_state_get(
                    state,
                    GHOSTTY_RENDER_STATE_DATA_COLORS,
                    (&raw mut raw_colors).cast(),
                ),
                "RENDER_STATE_DATA_COLORS",
            );
            let colors = Colors {
                foreground: rgb(raw_colors.foreground),
                background: rgb(raw_colors.background),
                cursor: raw_colors.cursor_has_value.then(|| rgb(raw_colors.cursor)),
                palette: raw_colors.palette.map(rgb),
            };
            let dirty: GhosttyRenderStateDirty = state_get(state, GHOSTTY_RENDER_STATE_DATA_DIRTY);
            let full =
                dirty == GHOSTTY_RENDER_STATE_DIRTY_FULL || self.built_with != Some((colors, cols));

            let mut screen = Vec::with_capacity(rows as usize);
            let mut rows_by_id = HashMap::with_capacity(rows as usize);
            let mut rebuilt = false;
            ok(
                ghostty_render_state_get(
                    state,
                    GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                    (&raw mut self.rows_iter).cast(),
                ),
                "RENDER_STATE_DATA_ROW_ITERATOR",
            );
            while ghostty_render_state_row_iterator_next(self.rows_iter) {
                let mut raw_id = GhosttyRenderStateRowId::default();
                let has_id = ghostty_render_state_row_get(
                    self.rows_iter,
                    GHOSTTY_RENDER_STATE_ROW_DATA_ID,
                    (&raw mut raw_id).cast(),
                ) == GHOSTTY_SUCCESS;
                let id = RowId(u128::from(raw_id.bits[0]) << 64 | u128::from(raw_id.bits[1]));
                let row_dirty: bool = row_get(self.rows_iter, GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY);
                let mut range = GhosttyRenderStateRowSelection {
                    size: size_of::<GhosttyRenderStateRowSelection>(),
                    ..Default::default()
                };
                let selection = (ghostty_render_state_row_get(
                    self.rows_iter,
                    GHOSTTY_RENDER_STATE_ROW_DATA_SELECTION,
                    (&raw mut range).cast(),
                ) == GHOSTTY_SUCCESS)
                    .then_some((range.start_x, range.end_x));

                let kept = (has_id && !full && !row_dirty)
                    .then(|| self.rows_by_id.get(&id))
                    .flatten()
                    .filter(|row| row.selection == selection)
                    .cloned();
                let row = match kept {
                    Some(row) => row,
                    None => {
                        rebuilt = true;
                        Arc::new(self.read_row(id, selection, &colors))
                    }
                };
                if has_id {
                    rows_by_id.insert(id, row.clone());
                }
                screen.push(row);
            }
            self.rows_by_id = rows_by_id;
            self.built_with = Some((colors, cols));

            let mut raw_cursor = GhosttyRenderStateCursor {
                size: size_of::<GhosttyRenderStateCursor>(),
                ..Default::default()
            };
            ok(
                ghostty_render_state_get(
                    state,
                    GHOSTTY_RENDER_STATE_DATA_CURSOR,
                    (&raw mut raw_cursor).cast(),
                ),
                "RENDER_STATE_DATA_CURSOR",
            );
            let cursor = Cursor {
                position: raw_cursor
                    .viewport_has_value
                    .then_some((raw_cursor.viewport_x, raw_cursor.viewport_y)),
                wide_tail: raw_cursor.viewport_has_value && raw_cursor.wide_tail,
                visible: raw_cursor.visible,
                blinking: raw_cursor.blinking,
                password_input: raw_cursor.password_input,
                style: match raw_cursor.visual_style {
                    GHOSTTY_CURSOR_STYLE_BAR => CursorStyle::Bar,
                    GHOSTTY_CURSOR_STYLE_UNDERLINE => CursorStyle::Underline,
                    GHOSTTY_CURSOR_STYLE_BLOCK_HOLLOW => CursorStyle::BlockHollow,
                    _ => CursorStyle::Block,
                },
            };
            ok(
                ghostty_render_state_clean(state),
                "ghostty_render_state_clean",
            );

            self.generation += 1;
            Snapshot {
                generation: self.generation,
                cols,
                rows,
                cell_width: self.effects().size.cell_width,
                cell_height: self.effects().size.cell_height,
                screen,
                dirty: if full {
                    Dirty::Full
                } else if rebuilt {
                    Dirty::Partial
                } else {
                    Dirty::Clean
                },
                cursor,
                colors,
                scrollbar: meta.scrollbar,
                active_screen: meta.active_screen,
                modes: Modes {
                    mouse_tracking: meta.mouse_tracking,
                    bracketed_paste: meta.bracketed_paste,
                    focus_reporting: meta.focus_reporting,
                    kitty_keyboard: meta.kitty_keyboard,
                    held: self.effects().hold.is_some(),
                },
                // Borrowed strings, copied before the next terminal call.
                title: terminal_string(self.term, GHOSTTY_TERMINAL_DATA_TITLE),
                pwd: terminal_string(self.term, GHOSTTY_TERMINAL_DATA_PWD),
            }
        }
    }

    /// Reads the row under the row iterator.
    unsafe fn read_row(
        &mut self,
        id: RowId,
        selection: Option<(u16, u16)>,
        colors: &Colors,
    ) -> Row {
        // SAFETY: the caller positioned `rows_iter` on a row of the current render state.
        unsafe {
            let raw_row: GhosttyRow = row_get(self.rows_iter, GHOSTTY_RENDER_STATE_ROW_DATA_RAW);
            let mut wrapped = false;
            ghostty_row_get(raw_row, GHOSTTY_ROW_DATA_WRAP, (&raw mut wrapped).cast());
            ok(
                ghostty_render_state_row_get(
                    self.rows_iter,
                    GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                    (&raw mut self.cells).cast(),
                ),
                "RENDER_STATE_ROW_DATA_CELLS",
            );
            let mut cells = Vec::new();
            while ghostty_render_state_row_cells_next(self.cells) {
                cells.push(read_cell(self.cells, colors));
            }
            Row {
                id,
                wrapped,
                cells,
                selection,
            }
        }
    }

    /// The bytes a key event sends, with the program's keyboard modes as they are now. Empty
    /// for a key that sends nothing, such as a lone modifier.
    pub fn encode_key(&mut self, event: &KeyEvent) -> Vec<u8> {
        let text = event.text.as_deref().unwrap_or("");
        // Text with control characters (Ctrl+C's "\x03") isn't printable: the encoder builds
        // those sequences from the key and mods.
        let printable = !text.is_empty() && !text.chars().any(char::is_control);
        // SAFETY: the encoder, event and terminal are live; `text` outlives the encode call;
        // `buf` and `written` are valid for writing.
        unsafe {
            ok(
                ghostty_key_encoder_set_from_terminal(self.keys, self.term),
                "ghostty_key_encoder_set_from_terminal",
            );
            // The sync resets Option-as-Alt, so set it again every time (guides/input.md).
            let option_as_alt = self.option_as_alt.raw();
            ok(
                ghostty_key_encoder_set(
                    self.keys,
                    GHOSTTY_KEY_ENCODER_OPT_MACOS_OPTION_AS_ALT,
                    (&raw const option_as_alt).cast(),
                ),
                "KEY_ENCODER_OPT_MACOS_OPTION_AS_ALT",
            );
            let e = self.key_event;
            ghostty_key_event_set_action(
                e,
                match event.action {
                    KeyAction::Press => GHOSTTY_KEY_ACTION_PRESS,
                    KeyAction::Repeat => GHOSTTY_KEY_ACTION_REPEAT,
                    KeyAction::Release => GHOSTTY_KEY_ACTION_RELEASE,
                },
            );
            ghostty_key_event_set_key(e, event.key);
            ghostty_key_event_set_mods(e, event.mods.0);
            ghostty_key_event_set_consumed_mods(
                e,
                if printable { event.consumed_mods.0 } else { 0 },
            );
            ghostty_key_event_set_composing(e, event.composing);
            let utf8 = if printable { text } else { "" };
            ghostty_key_event_set_utf8(
                e,
                GhosttyString {
                    ptr: utf8.as_ptr(),
                    len: utf8.len(),
                },
            );
            ghostty_key_event_set_unshifted_codepoint(e, event.unshifted.map_or(0, u32::from));
            let mut buf = vec![0u8; 128];
            loop {
                let mut written = 0usize;
                match ghostty_key_encoder_encode(
                    self.keys,
                    e,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut written,
                ) {
                    GHOSTTY_SUCCESS => {
                        buf.truncate(written);
                        break;
                    }
                    GHOSTTY_OUT_OF_SPACE if written > buf.len() => buf.resize(written, 0),
                    other => panic!("ghostty_key_encoder_encode returned {other}"),
                }
            }
            // Don't keep a pointer to `text` in the event past this call.
            ghostty_key_event_set_utf8(
                e,
                GhosttyString {
                    ptr: ptr::null(),
                    len: 0,
                },
            );
            buf
        }
    }

    /// The bytes a mouse event sends to a program that tracks the mouse, in its format. Empty
    /// when the program doesn't track the mouse, or for motion within the last reported cell.
    pub fn encode_mouse(&mut self, event: &MouseEvent) -> Vec<u8> {
        let size = self.effects().size;
        let (cell_width, cell_height) = (size.cell_width.max(1), size.cell_height.max(1));
        let geometry = GhosttyMouseEncoderSize {
            size: size_of::<GhosttyMouseEncoderSize>(),
            screen_width: u32::from(size.columns) * cell_width,
            screen_height: u32::from(size.rows) * cell_height,
            cell_width,
            cell_height,
            ..Default::default()
        };
        // SAFETY: the encoder, event and terminal are live, each option value has its type,
        // and `buf` and `written` are valid for writing.
        unsafe {
            ok(
                ghostty_mouse_encoder_set_from_terminal(self.mouse, self.term),
                "ghostty_mouse_encoder_set_from_terminal",
            );
            ok(
                ghostty_mouse_encoder_set(
                    self.mouse,
                    GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
                    (&raw const geometry).cast(),
                ),
                "MOUSE_ENCODER_OPT_SIZE",
            );
            let pressed = event.any_button_pressed;
            ok(
                ghostty_mouse_encoder_set(
                    self.mouse,
                    GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
                    (&raw const pressed).cast(),
                ),
                "MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED",
            );
            let e = self.mouse_event;
            ghostty_mouse_event_set_action(
                e,
                match event.action {
                    MouseAction::Press => GHOSTTY_MOUSE_ACTION_PRESS,
                    MouseAction::Release => GHOSTTY_MOUSE_ACTION_RELEASE,
                    MouseAction::Motion => GHOSTTY_MOUSE_ACTION_MOTION,
                },
            );
            match event.button {
                Some(button) => ghostty_mouse_event_set_button(e, button.raw()),
                None => ghostty_mouse_event_clear_button(e),
            };
            ghostty_mouse_event_set_mods(e, event.mods.0);
            ghostty_mouse_event_set_position(
                e,
                GhosttyMousePosition {
                    x: event.x.max(0.0),
                    y: event.y.max(0.0),
                },
            );
            let mut buf = vec![0u8; 128];
            loop {
                let mut written = 0usize;
                match ghostty_mouse_encoder_encode(
                    self.mouse,
                    e,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut written,
                ) {
                    GHOSTTY_SUCCESS => {
                        buf.truncate(written);
                        return buf;
                    }
                    GHOSTTY_OUT_OF_SPACE if written > buf.len() => buf.resize(written, 0),
                    other => panic!("ghostty_mouse_encoder_encode returned {other}"),
                }
            }
        }
    }

    /// The bytes pasting `text` sends, framed for the program's modes (bracketed paste) and
    /// with unsafe control bytes replaced. Text that could run commands is refused unless
    /// `allow_unsafe`.
    pub fn paste(&mut self, text: &str, allow_unsafe: bool) -> Result<Vec<u8>, PasteRejected> {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        // The pasted bytes come out of WRITE_PTY, mixed with nothing else: set any replies
        // aside first.
        let replies = std::mem::take(&mut self.effects_mut().pty);
        let mime = "text/plain";
        let mimes = [GhosttyString {
            ptr: mime.as_ptr(),
            len: mime.len(),
        }];
        let mut source: &str = text;
        let paste = GhosttyPaste {
            size: size_of::<GhosttyPaste>(),
            location: GHOSTTY_CLIPBOARD_LOCATION_STANDARD,
            source: GHOSTTY_PASTE_SOURCE_CLIPBOARD,
            mimes: mimes.as_ptr(),
            mimes_len: mimes.len(),
            reader: GhosttyMimeReader {
                read: Some(read_paste),
                userdata: (&raw mut source).cast(),
            },
            allow_unsafe,
        };
        let mut pasted = false;
        // SAFETY: `term` is live; `paste`, `mimes` and `source` outlive the call, and the reader
        // reads `source` as the `&str` it points to.
        let result = unsafe { ghostty_terminal_paste(self.term, &paste, &mut pasted) };
        let bytes = std::mem::replace(&mut self.effects_mut().pty, replies);
        match result {
            GHOSTTY_SUCCESS => Ok(bytes),
            GHOSTTY_REJECTED => Err(PasteRejected {
                text: text.to_owned(),
            }),
            other => panic!("ghostty_terminal_paste returned {other}"),
        }
    }

    /// The modes as they are now (a snapshot's modes are from its frame).
    pub fn modes(&self) -> Modes {
        // SAFETY: `term` is live.
        let meta = unsafe { read_meta(self.term) };
        Modes {
            mouse_tracking: meta.mouse_tracking,
            bracketed_paste: meta.bracketed_paste,
            focus_reporting: meta.focus_reporting,
            kitty_keyboard: meta.kitty_keyboard,
            held: self.effects().hold.is_some(),
        }
    }
}

impl Vt {
    fn effects(&self) -> &Effects {
        // SAFETY: `effects` is live until `Drop`, and callbacks only borrow it during library
        // calls, which never run while this borrow is held.
        unsafe { self.effects.as_ref() }
    }

    fn effects_mut(&mut self) -> &mut Effects {
        // SAFETY: as in `effects`, and `&mut self` makes this the only borrow from Rust.
        unsafe { self.effects.as_mut() }
    }
}

impl Drop for Vt {
    fn drop(&mut self) {
        // SAFETY: each handle was created in `new` and is freed exactly once, the terminal last
        // of those it refers to.
        unsafe {
            ghostty_mouse_event_free(self.mouse_event);
            ghostty_mouse_encoder_free(self.mouse);
            ghostty_key_event_free(self.key_event);
            ghostty_key_encoder_free(self.keys);
            ghostty_render_state_row_cells_free(self.cells);
            ghostty_render_state_row_iterator_free(self.rows_iter);
            ghostty_render_state_free(self.effects().state);
            ghostty_terminal_free(self.term);
            // No callback can run once the terminal is gone.
            drop(Box::from_raw(self.effects.as_ptr()));
        }
    }
}

/// Reads the cell under `cells`.
unsafe fn read_cell(cells: GhosttyRenderStateRowCells, colors: &Colors) -> Cell {
    // SAFETY: the caller positioned `cells` on a cell; out pointers have their keys' types.
    unsafe {
        let raw: GhosttyCell = cell_get(cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW);
        let mut wide: GhosttyCellWide = GHOSTTY_CELL_WIDE_NARROW;
        ghostty_cell_get(raw, GHOSTTY_CELL_DATA_WIDE, (&raw mut wide).cast());
        let mut hyperlink = false;
        ghostty_cell_get(
            raw,
            GHOSTTY_CELL_DATA_HAS_HYPERLINK,
            (&raw mut hyperlink).cast(),
        );
        let style: GhosttyStyle = cell_get(cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE);
        Cell {
            text: grapheme(cells),
            width: match wide {
                GHOSTTY_CELL_WIDE_WIDE => Width::Wide,
                GHOSTTY_CELL_WIDE_SPACER_TAIL => Width::SpacerTail,
                GHOSTTY_CELL_WIDE_SPACER_HEAD => Width::SpacerHead,
                _ => Width::Narrow,
            },
            fg: cell_color(cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR),
            bg: cell_color(cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR),
            underline: match u32::from(style.flags) >> GHOSTTY_STYLE_UNDERLINE_SHIFT & 0x7 {
                GHOSTTY_SGR_UNDERLINE_SINGLE => Underline::Single,
                GHOSTTY_SGR_UNDERLINE_DOUBLE => Underline::Double,
                GHOSTTY_SGR_UNDERLINE_CURLY => Underline::Curly,
                GHOSTTY_SGR_UNDERLINE_DOTTED => Underline::Dotted,
                GHOSTTY_SGR_UNDERLINE_DASHED => Underline::Dashed,
                _ => Underline::None,
            },
            underline_color: style_color(style.underline_color, colors),
            flags: CellFlags::from_style_bits(style.flags),
            hyperlink,
        }
    }
}

unsafe fn grapheme(cells: GhosttyRenderStateRowCells) -> Grapheme {
    let mut bytes = [0u8; 64];
    let mut buf = GhosttyBuffer {
        ptr: bytes.as_mut_ptr(),
        cap: bytes.len(),
        len: 0,
    };
    // SAFETY: `buf` describes writable memory of `cap` bytes.
    let result = unsafe {
        ghostty_render_state_row_cells_get(
            cells,
            GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
            (&raw mut buf).cast(),
        )
    };
    match result {
        GHOSTTY_SUCCESS => Grapheme::new(&String::from_utf8_lossy(&bytes[..buf.len])),
        GHOSTTY_OUT_OF_SPACE => {
            let mut heap = vec![0u8; buf.len];
            let mut buf = GhosttyBuffer {
                ptr: heap.as_mut_ptr(),
                cap: heap.len(),
                len: 0,
            };
            // SAFETY: as above, with a buffer of the size the library asked for.
            ok(
                unsafe {
                    ghostty_render_state_row_cells_get(
                        cells,
                        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
                        (&raw mut buf).cast(),
                    )
                },
                "ROW_CELLS_DATA_GRAPHEMES_UTF8",
            );
            Grapheme::new(&String::from_utf8_lossy(&heap[..buf.len]))
        }
        other => panic!("ROW_CELLS_DATA_GRAPHEMES_UTF8 returned {other}"),
    }
}

unsafe fn cell_color(
    cells: GhosttyRenderStateRowCells,
    key: GhosttyRenderStateRowCellsData,
) -> Option<Rgb> {
    let mut color = GhosttyColorRgb::default();
    // SAFETY: the key's output type is `GhosttyColorRgb`.
    match unsafe { ghostty_render_state_row_cells_get(cells, key, (&raw mut color).cast()) } {
        GHOSTTY_SUCCESS => Some(rgb(color)),
        GHOSTTY_NO_VALUE => None,
        other => panic!("row cell color {key} returned {other}"),
    }
}

fn style_color(color: GhosttyStyleColor, colors: &Colors) -> Option<Rgb> {
    match u32::from(color.tag) {
        // SAFETY: the tag says which union field is set.
        GHOSTTY_STYLE_COLOR_PALETTE => {
            Some(colors.palette[unsafe { color.value.palette } as usize])
        }
        GHOSTTY_STYLE_COLOR_RGB => Some(rgb(unsafe { color.value.rgb })),
        _ => None,
    }
}

/// Updates the render state from the terminal and reads the terminal data that goes with it.
unsafe fn update(state: GhosttyRenderState, term: GhosttyTerminal) -> Meta {
    // SAFETY: the caller passes live handles.
    unsafe {
        ok(
            ghostty_render_state_update(state, term),
            "ghostty_render_state_update",
        );
        read_meta(term)
    }
}

unsafe fn read_meta(term: GhosttyTerminal) -> Meta {
    // SAFETY: the caller passes a live terminal; each out pointer has its key's type.
    unsafe {
        let scrollbar: GhosttyTerminalScrollbar =
            terminal_get(term, GHOSTTY_TERMINAL_DATA_SCROLLBAR);
        let at_bottom: bool = terminal_get(term, GHOSTTY_TERMINAL_DATA_VIEWPORT_ACTIVE);
        let screen: GhosttyTerminalScreen = terminal_get(term, GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN);
        Meta {
            scrollbar: Scrollbar {
                total: scrollbar.total,
                offset: scrollbar.offset,
                len: scrollbar.len,
                at_bottom,
            },
            active_screen: if screen == GHOSTTY_TERMINAL_SCREEN_ALTERNATE {
                ActiveScreen::Alternate
            } else {
                ActiveScreen::Primary
            },
            mouse_tracking: terminal_get(term, GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING),
            bracketed_paste: mode(term, BRACKETED_PASTE),
            focus_reporting: mode(term, FOCUS_EVENT),
            kitty_keyboard: terminal_get(term, GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS),
        }
    }
}

unsafe fn mode(term: GhosttyTerminal, mode: GhosttyMode) -> bool {
    let mut config = GhosttyTerminalModeConfig { mode, value: false };
    // SAFETY: the key's in/out type is `GhosttyTerminalModeConfig`.
    ok(
        unsafe { ghostty_terminal_get(term, GHOSTTY_TERMINAL_DATA_MODE, (&raw mut config).cast()) },
        "TERMINAL_DATA_MODE",
    );
    config.value
}

/// A terminal string (the title or the pwd), copied. Empty when the program never set one.
unsafe fn terminal_string(term: GhosttyTerminal, key: GhosttyTerminalData) -> String {
    let s: GhosttyString = unsafe { terminal_get(term, key) };
    // SAFETY: the library returned a string readable for `len` bytes until the next mutating
    // call, and it's copied now.
    unsafe { string(s) }
}

/// Copies a borrowed `GhosttyString`.
unsafe fn string(s: GhosttyString) -> String {
    if s.ptr.is_null() || s.len == 0 {
        return String::new();
    }
    // SAFETY: the caller passes a string the library says is readable for `len` bytes.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(s.ptr, s.len) }).into_owned()
}

fn rgb(c: GhosttyColorRgb) -> Rgb {
    Rgb::new(c.r, c.g, c.b)
}

#[track_caller]
fn ok(result: GhosttyResult, what: &str) {
    assert_eq!(result, GHOSTTY_SUCCESS, "{what} returned {result}");
}

#[track_caller]
unsafe fn set(term: GhosttyTerminal, option: GhosttyTerminalOption, value: *const c_void) {
    // SAFETY: the caller passes a value of the option's type.
    ok(
        unsafe { ghostty_terminal_set(term, option, value) },
        "ghostty_terminal_set",
    );
}

unsafe fn terminal_get<T: Default>(term: GhosttyTerminal, key: GhosttyTerminalData) -> T {
    let mut out = T::default();
    // SAFETY: the caller picks `T` as the key's output type.
    ok(
        unsafe { ghostty_terminal_get(term, key, (&raw mut out).cast()) },
        "ghostty_terminal_get",
    );
    out
}

unsafe fn state_get<T: Default>(state: GhosttyRenderState, key: GhosttyRenderStateData) -> T {
    let mut out = T::default();
    // SAFETY: the caller picks `T` as the key's output type.
    ok(
        unsafe { ghostty_render_state_get(state, key, (&raw mut out).cast()) },
        "ghostty_render_state_get",
    );
    out
}

unsafe fn row_get<T: Default>(
    rows: GhosttyRenderStateRowIterator,
    key: GhosttyRenderStateRowData,
) -> T {
    let mut out = T::default();
    // SAFETY: the caller picks `T` as the key's output type.
    ok(
        unsafe { ghostty_render_state_row_get(rows, key, (&raw mut out).cast()) },
        "ghostty_render_state_row_get",
    );
    out
}

unsafe fn cell_get<T: Default>(
    cells: GhosttyRenderStateRowCells,
    key: GhosttyRenderStateRowCellsData,
) -> T {
    let mut out = T::default();
    // SAFETY: the caller picks `T` as the key's output type.
    ok(
        unsafe { ghostty_render_state_row_cells_get(cells, key, (&raw mut out).cast()) },
        "ghostty_render_state_row_cells_get",
    );
    out
}

// Effects. Each runs inside `ghostty_terminal_vt_write` (or a resize, reset or paste) on the
// `Vt`'s thread, with `userdata` the `Vt`'s `Effects`. None may write to the terminal.

unsafe fn effects<'a>(userdata: *mut c_void) -> &'a mut Effects {
    // SAFETY: userdata is the `Vt`'s `Effects`, alive as long as the terminal, and nothing else
    // borrows it while the library is calling back.
    unsafe { &mut *userdata.cast::<Effects>() }
}

unsafe fn set_callbacks(term: GhosttyTerminal) {
    type Bare = unsafe extern "C" fn(GhosttyTerminal, *mut c_void);
    // SAFETY: each function has the signature of its option's callback type.
    unsafe {
        set(
            term,
            GHOSTTY_TERMINAL_OPT_WRITE_PTY,
            (on_write_pty as unsafe extern "C" fn(_, _, _, _)) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_SIZE,
            (on_size as unsafe extern "C" fn(_, _, _) -> _) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_RENDER_HOLD,
            (on_render_hold as unsafe extern "C" fn(_, _, _)) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_BELL,
            (on_bell as Bare) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_TITLE_CHANGED,
            (on_title as Bare) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_PWD_CHANGED,
            (on_pwd as Bare) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_RESET,
            (on_reset as Bare) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_COLOR_SCHEME,
            (on_color_scheme as unsafe extern "C" fn(_, _, _) -> _) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_CLIPBOARD_WRITE,
            (on_clipboard_write as unsafe extern "C" fn(_, _, _)) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_DESKTOP_NOTIFICATION,
            (on_notification as unsafe extern "C" fn(_, _, _)) as *const c_void,
        );
        set(
            term,
            GHOSTTY_TERMINAL_OPT_PROGRESS_REPORT,
            (on_progress as unsafe extern "C" fn(_, _, _)) as *const c_void,
        );
    }
}

unsafe extern "C" fn on_write_pty(
    _: GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    // SAFETY: the library passes `len` readable bytes, valid for this call.
    unsafe {
        effects(userdata)
            .pty
            .extend_from_slice(std::slice::from_raw_parts(data, len))
    };
}

unsafe extern "C" fn on_size(
    _: GhosttyTerminal,
    userdata: *mut c_void,
    out: *mut GhosttySizeReportSize,
) -> bool {
    // SAFETY: `out` is writable for one `GhosttySizeReportSize`.
    unsafe {
        let size = effects(userdata).size;
        // Before the metrics are known there's no true pixel size to report.
        if size.cell_width == 0 || size.cell_height == 0 {
            return false;
        }
        *out = size;
    }
    true
}

unsafe extern "C" fn on_render_hold(term: GhosttyTerminal, userdata: *mut c_void, held: bool) {
    // SAFETY: `effects.state` is live; updating it reads the terminal, which the library allows
    // from this callback (guides/rendering.md "Render holds").
    unsafe {
        let effects = effects(userdata);
        if held {
            // The terminal holds exactly the frame the program wants left on screen.
            effects.meta = update(effects.state, term);
            effects.hold = Some(Instant::now());
            effects.captured = true;
        } else {
            effects.hold = None;
            effects.captured = false;
        }
    }
}

unsafe extern "C" fn on_bell(_: GhosttyTerminal, userdata: *mut c_void) {
    unsafe { effects(userdata).events.push(Event::Bell) };
}

unsafe extern "C" fn on_title(term: GhosttyTerminal, userdata: *mut c_void) {
    // SAFETY: reading the title is allowed in this callback (terminal.h, TitleChangedFn).
    unsafe {
        let title = terminal_string(term, GHOSTTY_TERMINAL_DATA_TITLE);
        effects(userdata).events.push(Event::Title(title));
    }
}

unsafe extern "C" fn on_pwd(term: GhosttyTerminal, userdata: *mut c_void) {
    // SAFETY: as in `on_title` (terminal.h, PwdChangedFn).
    unsafe {
        let pwd = terminal_string(term, GHOSTTY_TERMINAL_DATA_PWD);
        effects(userdata).events.push(Event::Pwd(pwd));
    }
}

unsafe extern "C" fn on_reset(_: GhosttyTerminal, userdata: *mut c_void) {
    unsafe { effects(userdata).events.push(Event::Reset) };
}

unsafe extern "C" fn on_color_scheme(
    _: GhosttyTerminal,
    userdata: *mut c_void,
    out: *mut GhosttyColorScheme,
) -> bool {
    // SAFETY: `out` is writable for one `GhosttyColorScheme`.
    unsafe {
        match effects(userdata).color_scheme {
            Some(scheme) => {
                *out = match scheme {
                    ColorScheme::Light => GHOSTTY_COLOR_SCHEME_LIGHT,
                    ColorScheme::Dark => GHOSTTY_COLOR_SCHEME_DARK,
                };
                true
            }
            None => false,
        }
    }
}

unsafe extern "C" fn on_clipboard_write(
    _: GhosttyTerminal,
    userdata: *mut c_void,
    write: *const GhosttyClipboardWrite,
) {
    // SAFETY: `write` and everything it points to are valid for this call, and the reply is
    // made before returning.
    unsafe {
        let effects = effects(userdata);
        let write = &*write;
        let result = if effects.clipboard_write {
            let contents: &[GhosttyClipboardContent] = if write.contents_len == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(write.contents, write.contents_len)
            };
            let text = contents
                .iter()
                .find(|c| string(c.mime).starts_with("text/plain"))
                .or_else(|| {
                    contents
                        .iter()
                        .find(|c| string(c.mime).starts_with("text/"))
                })
                .map(|c| string(c.data));
            if write.contents_len > 0 && text.is_none() {
                GHOSTTY_CLIPBOARD_WRITE_RESULT_UNSUPPORTED
            } else {
                effects.events.push(Event::Clipboard {
                    location: match write.location {
                        GHOSTTY_CLIPBOARD_LOCATION_SELECTION => ClipboardLocation::Selection,
                        GHOSTTY_CLIPBOARD_LOCATION_PRIMARY => ClipboardLocation::Primary,
                        _ => ClipboardLocation::Standard,
                    },
                    text,
                });
                GHOSTTY_CLIPBOARD_WRITE_RESULT_SUCCESS
            }
        } else {
            GHOSTTY_CLIPBOARD_WRITE_RESULT_DENIED
        };
        if let Some(reply) = write.reply {
            let answer = GhosttyClipboardWriteReply {
                size: size_of::<GhosttyClipboardWriteReply>(),
                result,
                remember: false,
            };
            reply(write, &answer);
        }
    }
}

unsafe extern "C" fn on_notification(
    _: GhosttyTerminal,
    userdata: *mut c_void,
    notification: *const GhosttyTerminalDesktopNotification,
) {
    // SAFETY: `notification` and its strings are valid for this call.
    unsafe {
        let n = &*notification;
        effects(userdata).events.push(Event::Notification {
            title: string(n.title),
            body: string(n.body),
        });
    }
}

unsafe extern "C" fn on_progress(
    _: GhosttyTerminal,
    userdata: *mut c_void,
    report: *const GhosttyTerminalProgressReport,
) {
    // SAFETY: `report` is valid for this call.
    unsafe {
        let report = &*report;
        let percent = u8::try_from(report.progress).ok().map(|p| p.min(100));
        let progress = match report.state {
            GHOSTTY_TERMINAL_PROGRESS_STATE_REMOVE => Progress::Remove,
            GHOSTTY_TERMINAL_PROGRESS_STATE_SET => Progress::Set(percent.unwrap_or(0)),
            GHOSTTY_TERMINAL_PROGRESS_STATE_ERROR => Progress::Error(percent),
            GHOSTTY_TERMINAL_PROGRESS_STATE_PAUSE => Progress::Pause(percent),
            _ => Progress::Indeterminate,
        };
        effects(userdata).events.push(Event::Progress(progress));
    }
}

/// The paste's MIME reader: writes the `&str` behind `userdata`.
unsafe extern "C" fn read_paste(
    userdata: *mut c_void,
    _mime: GhosttyString,
    writer: GhosttyWriter,
) -> bool {
    // SAFETY: `userdata` is the `&str` `Vt::paste` set, alive for the paste call.
    unsafe {
        let text: &str = *userdata.cast::<&str>();
        match writer.write {
            Some(write) if !text.is_empty() => write(writer.userdata, text.as_ptr(), text.len()),
            Some(_) => true,
            None => false,
        }
    }
}
