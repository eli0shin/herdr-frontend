//! What one frame of a terminal looks like: the immutable [`Snapshot`] a worker hands the UI.
//!
//! A snapshot owns everything it describes, so the UI reads it without touching the emulator.
//! Rows are `Arc`s. A row libghostty kept clean since the last snapshot comes back as the same
//! `Arc`, so a renderer can cache work per row by [`Row::id`] or by pointer.

use std::fmt;
use std::sync::Arc;

/// A 24-bit color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }
}

/// One frame of a terminal, as the program left it (or as it was when a render hold began).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Counts the snapshots this terminal has published, starting at 1.
    pub generation: u64,
    pub cols: u16,
    pub rows: u16,
    /// The cell size passed to the last resize, in whole device pixels (0 before one).
    pub cell_width: u32,
    pub cell_height: u32,
    /// The rows of the viewport, top to bottom. There are always `rows` of them.
    pub screen: Vec<Arc<Row>>,
    /// How much changed since the previous snapshot.
    pub dirty: Dirty,
    pub cursor: Cursor,
    pub colors: Colors,
    pub scrollbar: Scrollbar,
    /// Which screen the program is on.
    pub active_screen: ActiveScreen,
    pub modes: Modes,
    /// The title from OSC 0 or 2, empty if none.
    pub title: String,
    /// The working directory from OSC 7 (raw, often a `file://` URI), empty if none.
    pub pwd: String,
    /// The find bar's search, while it has a query ([`Row::matches`] marks its hits).
    pub find: Option<Found>,
    /// The link under the pointer while Cmd is held, for its underline: one span per row it
    /// covers, in the viewport.
    pub link: Vec<Span>,
}

/// Where a search over the screen and scrollback stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Matches found so far on the active screen, scrollback included.
    pub total: usize,
    /// The selected match, counted from the newest (0 is the one nearest the bottom).
    pub selected: Option<usize>,
    /// The search has caught up with the terminal; `total` is final until more output comes.
    pub complete: bool,
}

/// Columns `start..=end` of viewport row `row`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub row: u16,
    pub start: u16,
    pub end: u16,
}

/// A search hit on a row: columns `start..=end`, and whether it is the selected one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: u16,
    pub end: u16,
    pub selected: bool,
}

impl Snapshot {
    /// The text of row `y`, trimmed at the end.
    pub fn line(&self, y: usize) -> String {
        self.screen
            .get(y)
            .map(|row| row.text().trim_end().to_owned())
            .unwrap_or_default()
    }

    /// The viewport as text: one trimmed line per row, without trailing empty lines.
    pub fn text(&self) -> String {
        let mut lines: Vec<String> = (0..self.screen.len()).map(|y| self.line(y)).collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }
}

/// How much of a snapshot changed since the previous one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dirty {
    /// Nothing in the rows changed (the cursor, scrollbar or title may have).
    Clean,
    /// Some rows changed: the rest are the previous snapshot's `Arc`s.
    Partial,
    /// Everything changed, such as after a resize, a screen switch or new colors.
    #[default]
    Full,
}

/// A row's identity across snapshots. Ids are never reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RowId(pub u128);

/// One row of cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: RowId,
    /// The row soft-wraps into the next one.
    pub wrapped: bool,
    /// One cell per column.
    pub cells: Vec<Cell>,
    /// The selected columns, inclusive, if the selection touches this row.
    pub selection: Option<(u16, u16)>,
    /// The find bar's hits on this row, left to right.
    pub matches: Vec<Match>,
    /// The columns, inclusive, of the link under the pointer while Cmd is held, if it's on this
    /// row ([`Snapshot::link`]).
    pub link: Option<(u16, u16)>,
}

impl Row {
    /// The row's text: each cell's grapheme, a space for an empty cell, nothing for the spacer
    /// half of a wide character.
    pub fn text(&self) -> String {
        let mut text = String::with_capacity(self.cells.len());
        for cell in &self.cells {
            match cell.width {
                Width::SpacerTail | Width::SpacerHead => {}
                Width::Narrow | Width::Wide if cell.text.is_empty() => text.push(' '),
                Width::Narrow | Width::Wide => text.push_str(cell.text.as_str()),
            }
        }
        text
    }
}

/// One cell, with its colors resolved through the palette.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub text: Grapheme,
    pub width: Width,
    /// The foreground, or `None` for the default foreground. Bold-as-bright isn't applied.
    pub fg: Option<Rgb>,
    /// The background, or `None` for the default background.
    pub bg: Option<Rgb>,
    pub underline: Underline,
    /// The underline color, or `None` to use the foreground.
    pub underline_color: Option<Rgb>,
    pub flags: CellFlags,
    /// The cell is part of an OSC 8 hyperlink.
    pub hyperlink: bool,
}

/// How many columns a cell's character takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Width {
    #[default]
    Narrow,
    /// The first of a wide character's two cells. It holds the text.
    Wide,
    /// The second cell of a wide character. It holds no text.
    SpacerTail,
    /// The last column of a row whose wide character wrapped to the next row.
    SpacerHead,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// SGR attributes other than colors and the underline.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CellFlags(u8);

impl CellFlags {
    pub const BOLD: CellFlags = CellFlags(1 << 0);
    pub const ITALIC: CellFlags = CellFlags(1 << 1);
    pub const FAINT: CellFlags = CellFlags(1 << 2);
    pub const BLINK: CellFlags = CellFlags(1 << 3);
    pub const INVERSE: CellFlags = CellFlags(1 << 4);
    pub const INVISIBLE: CellFlags = CellFlags(1 << 5);
    pub const STRIKETHROUGH: CellFlags = CellFlags(1 << 6);
    pub const OVERLINE: CellFlags = CellFlags(1 << 7);

    /// The flags from libghostty's style flag bits (the low 8 bits of `GhosttyStyleFlags` use
    /// the same layout).
    pub(crate) const fn from_style_bits(bits: u16) -> CellFlags {
        CellFlags((bits & 0xff) as u8)
    }

    pub const fn empty() -> CellFlags {
        CellFlags(0)
    }

    pub const fn contains(self, other: CellFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for CellFlags {
    type Output = CellFlags;

    fn bitor(self, rhs: CellFlags) -> CellFlags {
        CellFlags(self.0 | rhs.0)
    }
}

impl fmt::Debug for CellFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; 8] = [
            "BOLD",
            "ITALIC",
            "FAINT",
            "BLINK",
            "INVERSE",
            "INVISIBLE",
            "STRIKETHROUGH",
            "OVERLINE",
        ];
        let set: Vec<&str> = (0..8)
            .filter(|bit| self.0 & (1 << bit) != 0)
            .map(|bit| NAMES[bit])
            .collect();
        write!(f, "CellFlags({})", set.join(" | "))
    }
}

/// A cell's grapheme cluster: empty, or the base character and its combining marks. Short
/// clusters (nearly all of them) are stored inline.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Grapheme(Repr);

#[derive(Clone, PartialEq, Eq, Hash)]
enum Repr {
    Inline {
        len: u8,
        bytes: [u8; Grapheme::INLINE],
    },
    Heap(Box<str>),
}

impl Default for Repr {
    fn default() -> Repr {
        Repr::Inline {
            len: 0,
            bytes: [0; Grapheme::INLINE],
        }
    }
}

impl Grapheme {
    const INLINE: usize = 14;

    pub fn new(text: &str) -> Grapheme {
        if text.len() <= Self::INLINE {
            let mut bytes = [0; Self::INLINE];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            Grapheme(Repr::Inline {
                len: text.len() as u8,
                bytes,
            })
        } else {
            Grapheme(Repr::Heap(text.into()))
        }
    }

    pub fn as_str(&self) -> &str {
        match &self.0 {
            Repr::Inline { len, bytes } => {
                std::str::from_utf8(&bytes[..*len as usize]).expect("built from a str")
            }
            Repr::Heap(text) => text,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }
}

impl fmt::Debug for Grapheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl From<&str> for Grapheme {
    fn from(text: &str) -> Grapheme {
        Grapheme::new(text)
    }
}

/// The cursor as the program left it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// The cursor's cell in the viewport, or `None` when it's scrolled out of view.
    pub position: Option<(u16, u16)>,
    /// The cursor sits on the spacer half of a wide character (its `position` is that half).
    pub wide_tail: bool,
    /// The program shows the cursor (DECTCEM).
    pub visible: bool,
    pub blinking: bool,
    /// The cursor is at a password prompt.
    pub password_input: bool,
    pub style: CursorStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorStyle {
    Bar,
    #[default]
    Block,
    Underline,
    BlockHollow,
}

/// The terminal's current colors: defaults, then anything the program set (OSC 4, 10, 11, 12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub foreground: Rgb,
    pub background: Rgb,
    /// The cursor color, if one is set.
    pub cursor: Option<Rgb>,
    pub palette: [Rgb; 256],
}

impl Default for Colors {
    fn default() -> Colors {
        Colors {
            foreground: Rgb::default(),
            background: Rgb::default(),
            cursor: None,
            palette: [Rgb::default(); 256],
        }
    }
}

/// Where the viewport is in the scrollback, in rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scrollbar {
    /// Rows in the scrollback and the active area together.
    pub total: u64,
    /// The viewport's first row, counted from the top of the scrollback.
    pub offset: u64,
    /// Rows in the viewport.
    pub len: u64,
    /// The viewport follows the active area (it isn't scrolled into history).
    pub at_bottom: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActiveScreen {
    #[default]
    Primary,
    Alternate,
}

/// The modes a view reads to decide what input does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// Some mouse tracking mode is on: clicks go to the program unless Shift is held.
    pub mouse_tracking: bool,
    /// DEC mode 2004.
    pub bracketed_paste: bool,
    /// DEC mode 1004. A herdr-core terminal sends focus through the core, never itself.
    pub focus_reporting: bool,
    /// The kitty keyboard protocol flags in effect.
    pub kitty_keyboard: u8,
    /// A render hold (synchronized output, mode 2026) is in effect: this snapshot is the frame
    /// from when it began.
    pub held: bool,
}
