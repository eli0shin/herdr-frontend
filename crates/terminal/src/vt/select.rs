//! Selecting with the mouse, while the program doesn't take it: libghostty-vt's selection
//! gestures (`guides/selection.md`), which turn presses, drags and releases into the
//! terminal's selection, with Ghostty's click counts and autoscroll
//! (`docs/research/terminal-rendering.md` §3, "Mouse").
//!
//! A click selects nothing, a double click a word, a triple click a line, and dragging after
//! any of them extends by cells, words or lines. Shift+press moves the end of the selection
//! there. A drag held past the top or bottom edge scrolls one row every [`AUTOSCROLL_EVERY`],
//! as Ghostty's `selection_scroll_ms` does, while the worker keeps time
//! ([`Vt::autoscroll_deadline`]).

use std::ptr;
use std::time::{Duration, Instant};

use ghostty_vt_sys::*;

use super::{Vt, ok, set};
use crate::input::{SelectAction, SelectEvent};

/// How often a drag held past the grid's edge scrolls a row: Ghostty's `selection_scroll_ms`
/// (`Thread.zig:41`).
pub const AUTOSCROLL_EVERY: Duration = Duration::from_millis(15);

/// The gesture and its reusable events. Owned by a [`Vt`], freed before its terminal.
pub(super) struct Selector {
    gesture: GhosttySelectionGesture,
    press: GhosttySelectionGestureEvent,
    drag: GhosttySelectionGestureEvent,
    release: GhosttySelectionGestureEvent,
    tick: GhosttySelectionGestureEvent,
    /// A Shift+press moved the end of an existing selection; drags keep moving it.
    extending: bool,
    /// The last drag, for the autoscroll ticks that continue it.
    last: Option<SelectEvent>,
    /// When the next autoscroll tick is due, while a drag is held past an edge.
    tick_at: Option<Instant>,
}

impl Selector {
    pub(super) fn new() -> Selector {
        // SAFETY: each handle is created here, checked, and freed once in `free`.
        unsafe {
            let mut gesture: GhosttySelectionGesture = ptr::null_mut();
            ok(
                ghostty_selection_gesture_new(ptr::null(), &mut gesture),
                "ghostty_selection_gesture_new",
            );
            let event = |kind| {
                let mut event: GhosttySelectionGestureEvent = ptr::null_mut();
                ok(
                    ghostty_selection_gesture_event_new(ptr::null(), &mut event, kind),
                    "ghostty_selection_gesture_event_new",
                );
                event
            };
            Selector {
                gesture,
                press: event(GHOSTTY_SELECTION_GESTURE_EVENT_TYPE_PRESS),
                drag: event(GHOSTTY_SELECTION_GESTURE_EVENT_TYPE_DRAG),
                release: event(GHOSTTY_SELECTION_GESTURE_EVENT_TYPE_RELEASE),
                tick: event(GHOSTTY_SELECTION_GESTURE_EVENT_TYPE_AUTOSCROLL_TICK),
                extending: false,
                last: None,
                tick_at: None,
            }
        }
    }

    /// Frees the gesture's tracked references with `term`, then the handles.
    ///
    /// # Safety
    ///
    /// `term` is the live terminal the gesture was used with, and nothing uses `self` after.
    pub(super) unsafe fn free(&mut self, term: GhosttyTerminal) {
        // SAFETY: the caller's contract; each handle was made in `new`.
        unsafe {
            ghostty_selection_gesture_event_free(self.tick);
            ghostty_selection_gesture_event_free(self.release);
            ghostty_selection_gesture_event_free(self.drag);
            ghostty_selection_gesture_event_free(self.press);
            ghostty_selection_gesture_free(self.gesture, term);
        }
    }
}

impl Vt {
    /// One step of a mouse selection. A press with no selection to make (a single click)
    /// clears the old one.
    pub fn select(&mut self, event: &SelectEvent, now: Instant) {
        let Some((col, row)) = self.cell_at(event.x, event.y) else {
            return;
        };
        match event.action {
            SelectAction::Press if event.extend && self.has_selection() => {
                self.selector.extending = true;
                self.move_selection_end(col, row);
            }
            SelectAction::Drag if self.selector.extending => self.move_selection_end(col, row),
            SelectAction::Press => {
                self.selector.extending = false;
                self.selector.last = None;
                self.selector.tick_at = None;
                let behavior = match event.clicks {
                    0 | 1 => GHOSTTY_SELECTION_GESTURE_BEHAVIOR_CELL,
                    2 => GHOSTTY_SELECTION_GESTURE_BEHAVIOR_WORD,
                    _ => GHOSTTY_SELECTION_GESTURE_BEHAVIOR_LINE,
                };
                // The click count is Rune's (the system's double-click interval and slop), so
                // the gesture's own timing is left unset: an untimed press uses the
                // single-click behavior, set here to the count's.
                let behaviors = GhosttySelectionGestureBehaviors {
                    single_click: behavior,
                    double_click: behavior,
                    triple_click: behavior,
                };
                let e = self.selector.press;
                // SAFETY: `e` is live and each value has its option's type.
                let selected = unsafe {
                    self.set_ref(e, col, row);
                    self.set_position(e, event);
                    ghostty_selection_gesture_event_set(
                        e,
                        GHOSTTY_SELECTION_GESTURE_EVENT_OPT_BEHAVIORS,
                        (&raw const behaviors).cast(),
                    );
                    self.apply(e)
                };
                if !selected {
                    self.clear_selection();
                }
            }
            SelectAction::Drag => {
                let e = self.selector.drag;
                // SAFETY: as above.
                unsafe {
                    self.set_ref(e, col, row);
                    self.set_position(e, event);
                    self.set_geometry(e);
                    self.apply(e);
                }
                self.selector.last = Some(*event);
                self.follow_autoscroll(now);
            }
            SelectAction::Release => {
                self.selector.extending = false;
                self.selector.last = None;
                self.selector.tick_at = None;
                let e = self.selector.release;
                // SAFETY: as above. A release makes no selection.
                unsafe {
                    self.set_ref(e, col, row);
                    self.apply(e);
                }
            }
        }
    }

    /// When the next autoscroll tick is due: a drag is held past the grid's top or bottom.
    pub fn autoscroll_deadline(&self) -> Option<Instant> {
        self.selector.tick_at
    }

    /// Scrolls a row toward the edge the drag is held past and extends the selection there,
    /// if a tick is due. True if it did.
    pub fn autoscroll(&mut self, now: Instant) -> bool {
        let (Some(due), Some(last)) = (self.selector.tick_at, self.selector.last) else {
            return false;
        };
        if now < due {
            return false;
        }
        let Some((col, row)) = self.cell_at(last.x, last.y) else {
            self.selector.tick_at = None;
            return false;
        };
        let e = self.selector.tick;
        let coordinate = GhosttyPointCoordinate {
            x: col,
            y: u32::from(row),
        };
        // SAFETY: `e` is live and each value has its option's type.
        let ticked = unsafe {
            ghostty_selection_gesture_event_set(
                e,
                GHOSTTY_SELECTION_GESTURE_EVENT_OPT_VIEWPORT,
                (&raw const coordinate).cast(),
            );
            self.set_position(e, &last);
            self.set_geometry(e);
            self.apply(e)
        };
        if ticked {
            self.selector.tick_at = None;
            self.follow_autoscroll(now);
        } else {
            // Nothing to scroll toward, or the press's cell left the screen.
            self.selector.tick_at = None;
        }
        ticked
    }

    /// Whether anything is selected.
    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    pub fn clear_selection(&mut self) {
        // SAFETY: `term` is live; null clears the option.
        unsafe { set(self.term, GHOSTTY_TERMINAL_OPT_SELECTION, ptr::null()) };
    }

    /// The selected text as Ghostty copies it: plain, soft-wrapped lines joined, trailing
    /// spaces trimmed. `None` with nothing selected.
    pub fn selection_text(&self) -> Option<String> {
        let selection = self.selection()?;
        let yes = true;
        // SAFETY: the formatter borrows the live terminal and is freed before returning; each
        // option value has its type; the allocated buffer is copied, then freed.
        unsafe {
            let mut formatter: GhosttyFormatter = ptr::null_mut();
            ok(
                ghostty_formatter_terminal_new(ptr::null(), &mut formatter, self.term),
                "ghostty_formatter_terminal_new",
            );
            ok(
                ghostty_formatter_set(
                    formatter,
                    GHOSTTY_FORMATTER_OPT_SELECTION,
                    (&raw const selection).cast(),
                ),
                "FORMATTER_OPT_SELECTION",
            );
            ok(
                ghostty_formatter_set(
                    formatter,
                    GHOSTTY_FORMATTER_OPT_UNWRAP,
                    (&raw const yes).cast(),
                ),
                "FORMATTER_OPT_UNWRAP",
            );
            ok(
                ghostty_formatter_set(
                    formatter,
                    GHOSTTY_FORMATTER_OPT_TRIM,
                    (&raw const yes).cast(),
                ),
                "FORMATTER_OPT_TRIM",
            );
            let (mut out, mut len) = (ptr::null_mut::<u8>(), 0usize);
            let result = ghostty_formatter_format_alloc(formatter, ptr::null(), &mut out, &mut len);
            ghostty_formatter_free(formatter);
            ok(result, "ghostty_formatter_format_alloc");
            let text = if out.is_null() || len == 0 {
                String::new()
            } else {
                String::from_utf8_lossy(std::slice::from_raw_parts(out, len)).into_owned()
            };
            ghostty_free(ptr::null(), out, len);
            (!text.is_empty()).then_some(text)
        }
    }

    /// The terminal's selection, as a snapshot valid until the next change.
    fn selection(&self) -> Option<GhosttySelection> {
        let mut selection = GhosttySelection::default();
        // SAFETY: `term` is live and the key's output type is `GhosttySelection`.
        let result = unsafe {
            ghostty_terminal_get(
                self.term,
                GHOSTTY_TERMINAL_DATA_SELECTION,
                (&raw mut selection).cast(),
            )
        };
        match result {
            GHOSTTY_SUCCESS => Some(selection),
            GHOSTTY_NO_VALUE => None,
            other => panic!("TERMINAL_DATA_SELECTION returned {other}"),
        }
    }

    /// The viewport cell under a pixel position, clamped into the grid.
    fn cell_at(&self, x: f32, y: f32) -> Option<(u16, u16)> {
        let size = self.effects().size;
        if size.cell_width == 0 || size.cell_height == 0 {
            return None;
        }
        let along = |px: f32, cell: u32, cells: u16| {
            let n = (px.max(0.0) / cell as f32).floor() as u32;
            n.min(u32::from(cells.max(1)) - 1) as u16
        };
        Some((
            along(x, size.cell_width, size.columns),
            along(y, size.cell_height, size.rows),
        ))
    }

    /// The viewport cell as a grid reference.
    fn grid_ref(&self, col: u16, row: u16) -> Option<GhosttyGridRef> {
        let point = GhosttyPoint {
            tag: GHOSTTY_POINT_TAG_VIEWPORT,
            value: GhosttyPointValue {
                coordinate: GhosttyPointCoordinate {
                    x: col,
                    y: u32::from(row),
                },
            },
        };
        let mut out = GhosttyGridRef::default();
        // SAFETY: `term` is live and `out` is writable.
        (unsafe { ghostty_terminal_grid_ref(self.term, point, &mut out) } == GHOSTTY_SUCCESS)
            .then_some(out)
    }

    /// Shift+click: the selection keeps its start and ends at the cell.
    fn move_selection_end(&mut self, col: u16, row: u16) {
        let (Some(mut selection), Some(end)) = (self.selection(), self.grid_ref(col, row)) else {
            return;
        };
        selection.end = end;
        // SAFETY: `term` is live and the value is a fresh `GhosttySelection` from it.
        unsafe {
            set(
                self.term,
                GHOSTTY_TERMINAL_OPT_SELECTION,
                (&raw const selection).cast(),
            )
        };
    }

    /// Runs a gesture event and installs the selection it makes. True if it made one.
    ///
    /// # Safety
    ///
    /// `e` is one of this `Vt`'s gesture events, with its required options set.
    unsafe fn apply(&mut self, e: GhosttySelectionGestureEvent) -> bool {
        let mut selection = GhosttySelection::default();
        // SAFETY: the caller's contract; the gesture and terminal are live.
        unsafe {
            match ghostty_selection_gesture_event(
                self.selector.gesture,
                self.term,
                e,
                &mut selection,
            ) {
                GHOSTTY_SUCCESS => {
                    set(
                        self.term,
                        GHOSTTY_TERMINAL_OPT_SELECTION,
                        (&raw const selection).cast(),
                    );
                    true
                }
                _ => false,
            }
        }
    }

    /// # Safety
    ///
    /// `e` is a live gesture event.
    unsafe fn set_ref(&self, e: GhosttySelectionGestureEvent, col: u16, row: u16) {
        let r = self.grid_ref(col, row);
        let value = r.as_ref().map_or(ptr::null(), |r| ptr::from_ref(r).cast());
        // SAFETY: the caller's contract; the value is a `GhosttyGridRef` or null (unset).
        unsafe {
            ghostty_selection_gesture_event_set(e, GHOSTTY_SELECTION_GESTURE_EVENT_OPT_REF, value)
        };
    }

    /// # Safety
    ///
    /// `e` is a live press, drag or tick event.
    unsafe fn set_position(&self, e: GhosttySelectionGestureEvent, event: &SelectEvent) {
        let position = GhosttySurfacePosition {
            x: f64::from(event.x),
            y: f64::from(event.y),
        };
        let rectangle = event.rectangle;
        // SAFETY: the caller's contract; each value has its option's type.
        unsafe {
            ghostty_selection_gesture_event_set(
                e,
                GHOSTTY_SELECTION_GESTURE_EVENT_OPT_POSITION,
                (&raw const position).cast(),
            );
            if e != self.selector.press {
                ghostty_selection_gesture_event_set(
                    e,
                    GHOSTTY_SELECTION_GESTURE_EVENT_OPT_RECTANGLE,
                    (&raw const rectangle).cast(),
                );
            }
        }
    }

    /// # Safety
    ///
    /// `e` is a live drag or tick event.
    unsafe fn set_geometry(&self, e: GhosttySelectionGestureEvent) {
        let size = self.effects().size;
        let geometry = GhosttySelectionGestureGeometry {
            columns: u32::from(size.columns.max(1)),
            cell_width: size.cell_width.max(1),
            padding_left: 0,
            screen_height: (u32::from(size.rows.max(1)) * size.cell_height).max(1),
        };
        // SAFETY: the caller's contract; the value is a `GhosttySelectionGestureGeometry`.
        unsafe {
            ghostty_selection_gesture_event_set(
                e,
                GHOSTTY_SELECTION_GESTURE_EVENT_OPT_GEOMETRY,
                (&raw const geometry).cast(),
            )
        };
    }

    /// Starts or stops the autoscroll ticks as the gesture asks.
    fn follow_autoscroll(&mut self, now: Instant) {
        let mut direction = GHOSTTY_SELECTION_GESTURE_AUTOSCROLL_NONE;
        // SAFETY: the gesture and terminal are live; the output type is the autoscroll enum.
        let result = unsafe {
            ghostty_selection_gesture_get(
                self.selector.gesture,
                self.term,
                GHOSTTY_SELECTION_GESTURE_DATA_AUTOSCROLL,
                (&raw mut direction).cast(),
            )
        };
        let scrolling =
            result == GHOSTTY_SUCCESS && direction != GHOSTTY_SELECTION_GESTURE_AUTOSCROLL_NONE;
        self.selector.tick_at = match (scrolling, self.selector.tick_at) {
            (false, _) => None,
            (true, Some(due)) => Some(due),
            (true, None) => Some(now + AUTOSCROLL_EVERY),
        };
    }
}
