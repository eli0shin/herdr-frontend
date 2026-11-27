//! What the grammar asks the engine to do.

/// What a visual selection covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Visual {
    /// `v`: from the anchor to the cursor, as text flows.
    Chars,
    /// `V`: whole rows.
    Lines,
    /// `Ctrl-V`: the rectangle with the anchor and the cursor at its corners.
    Block,
}

/// `f`, `F`, `t` or `T`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FindKind {
    /// `f`: onto the next one.
    Onto,
    /// `F`: onto the previous one.
    OntoBack,
    /// `t`: just before the next one.
    Till,
    /// `T`: just after the previous one.
    TillBack,
}

impl FindKind {
    /// The other direction, for `,`.
    pub fn reversed(self) -> FindKind {
        match self {
            FindKind::Onto => FindKind::OntoBack,
            FindKind::OntoBack => FindKind::Onto,
            FindKind::Till => FindKind::TillBack,
            FindKind::TillBack => FindKind::Till,
        }
    }

    pub fn forward(self) -> bool {
        matches!(self, FindKind::Onto | FindKind::Till)
    }
}

/// Where a motion goes. A count, where there is one, comes with the [`Command`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    /// `h`, ←, Backspace.
    Left,
    /// `l`, →.
    Right,
    /// `k`, ↑, Ctrl-P.
    Up,
    /// `j`, ↓, Ctrl-N.
    Down,
    /// `0`, Home: the row's first column.
    LineStart,
    /// `^`: the row's first text.
    FirstText,
    /// `$`, End: the row's last text (with a count, on the row count − 1 below).
    LineEnd,
    /// `|`: the count's column.
    Column,
    /// `+` and `-`: the first text a row (count rows) down or up.
    NextLine,
    PrevLine,
    /// `w`, `W`: the start of the next word (`big`: of the next run of non-blanks).
    WordForward {
        big: bool,
    },
    /// `b`, `B`: the start of this word or the one before.
    WordBack {
        big: bool,
    },
    /// `e`, `E`: the end of this word or the next.
    WordEnd {
        big: bool,
    },
    /// `ge`, `gE`: the end of the word before.
    WordEndBack {
        big: bool,
    },
    /// `gg`: the first row, or the count's row.
    Top,
    /// `G`: the last row with text, or the count's row.
    Bottom,
    /// `H`, `M`, `L`: the view's top (count rows down), middle and bottom (count rows up).
    ViewTop,
    ViewMiddle,
    ViewBottom,
    /// `f`, `F`, `t`, `T` with their character, along the line. `repeat` is `;` or `,`: `t` and
    /// `T` then skip a match right next to the cursor, so they don't stay put.
    Find {
        kind: FindKind,
        ch: char,
        repeat: bool,
    },
    /// `%`: the bracket matching the next one on the line; with a count, that percentage of the
    /// way down.
    Match,
    /// `}` and `{`: the next or previous empty line.
    ParagraphForward,
    ParagraphBack,
    /// Ctrl-D and Ctrl-U: the view and the cursor half a view (or count rows) down or up.
    HalfPageDown,
    HalfPageUp,
    /// Ctrl-F / Page Down and Ctrl-B / Page Up: a view (less two rows) down or up.
    PageDown,
    PageUp,
    /// Ctrl-E and Ctrl-Y: the view a row (or count rows) down or up; the cursor stays on its
    /// row while that's in view.
    ScrollDown,
    ScrollUp,
    /// `/` and `?`: the next match for `query` (empty: the last search's), down or up.
    Search {
        query: String,
        backward: bool,
    },
    /// `n`, `N`: the last search again, the same way or the other way.
    SearchNext {
        reverse: bool,
    },
    /// `*` and `#`: the next or previous whole-word match for the word under the cursor.
    SearchWord {
        backward: bool,
    },
    /// `` `x `` (`exact`) and `'x`: mark `x`'s cell, or its row's first text. `'` and `` ` ``
    /// name the place before the last jump.
    Mark {
        name: char,
        exact: bool,
    },
}

impl Motion {
    /// What yanking over the motion takes.
    pub(crate) fn kind(&self) -> Reach {
        use Motion as M;
        match self {
            M::Up
            | M::Down
            | M::NextLine
            | M::PrevLine
            | M::Top
            | M::Bottom
            | M::ViewTop
            | M::ViewMiddle
            | M::ViewBottom
            | M::HalfPageDown
            | M::HalfPageUp
            | M::PageDown
            | M::PageUp
            | M::ScrollDown
            | M::ScrollUp => Reach::Lines,
            M::Mark { exact: false, .. } => Reach::Lines,
            M::LineEnd
            | M::WordEnd { .. }
            | M::WordEndBack { .. }
            | M::Match
            | M::Find {
                kind: FindKind::Onto | FindKind::Till,
                ..
            } => Reach::Inclusive,
            _ => Reach::Exclusive,
        }
    }

    /// A jump: the place before it is remembered as mark `'`.
    pub(crate) fn jumps(&self) -> bool {
        use Motion as M;
        matches!(
            self,
            M::Top
                | M::Bottom
                | M::ViewTop
                | M::ViewMiddle
                | M::ViewBottom
                | M::Match
                | M::ParagraphForward
                | M::ParagraphBack
                | M::Search { .. }
                | M::SearchNext { .. }
                | M::SearchWord { .. }
                | M::Mark { .. }
        )
    }

    /// Scrolls the view rather than moving over text: no operator takes it.
    pub fn scrolls(&self) -> bool {
        use Motion as M;
        matches!(
            self,
            M::HalfPageDown | M::HalfPageUp | M::PageDown | M::PageUp | M::ScrollDown | M::ScrollUp
        )
    }
}

/// How far yanking over a motion reaches (`:help exclusive`, `:help linewise`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Up to the cell before the motion's end.
    Exclusive,
    /// Up to and with the motion's end.
    Inclusive,
    /// Every row from the cursor's to the end's.
    Lines,
}

/// A text object, after `i` (inner) or `a` (around) in visual mode or after `y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Object {
    /// `iw`, `aw`, `iW`, `aW`.
    Word { big: bool },
    /// `ip`, `ap`: a run of lines with text, or of empty lines.
    Paragraph,
    /// `i"`, `a'`, …: the quoted text on the line, with or without its quotes.
    Quote(char),
    /// `i(`, `a]`, `iB`, …: inside the brackets around the cursor; `open` is `(`, `[`, `{`
    /// or `<`.
    Bracket(char),
}

impl Object {
    /// Whether it selects whole rows.
    pub fn linewise(self) -> bool {
        self == Object::Paragraph
    }
}

/// One thing for the engine to do.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    /// Move the cursor, `count` times where the motion repeats (`None`: no count was typed).
    Move {
        motion: Motion,
        count: Option<usize>,
    },
    /// Start selecting from the cursor, switch what the selection covers, or (`None`) drop it.
    Visual(Option<Visual>),
    /// `o`: the cursor goes to the selection's other end.
    SwapEnds,
    /// Select a text object in visual mode, `inner` (`iw`, `i(`) or around it (`aw`, `a(`):
    /// from its start to its end.
    Select {
        object: Object,
        inner: bool,
        count: usize,
    },
    /// Set mark `name` at the cursor (`m`).
    Mark(char),
    /// Yank the selection.
    Yank,
    /// `y` and a motion: from the cursor to where the motion goes.
    YankMotion {
        motion: Motion,
        count: Option<usize>,
    },
    /// `y` and a text object.
    YankObject {
        object: Object,
        inner: bool,
        count: usize,
    },
    /// `yy`, `Y`: count rows from the cursor's.
    YankLines { count: usize },
}

impl Command {
    /// Whether it copies text.
    pub fn yanks(&self) -> bool {
        matches!(
            self,
            Command::Yank
                | Command::YankMotion { .. }
                | Command::YankObject { .. }
                | Command::YankLines { .. }
        )
    }
}
