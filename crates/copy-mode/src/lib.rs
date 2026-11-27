//! Copy mode (`ctrl-g [`, `docs/research/feature-inventory.md` §16): Vim's motions, visual
//! modes, text objects, marks and search over a terminal's screen and scrollback
//! (`docs/research/terminal-rendering.md` §6). Pure: no terminal, no I/O.
//!
//! Two halves, each used where its inputs live:
//!
//! - **[`Vim`], the grammar.** Turns keys into [`Command`]s: counts, `g`, `f`/`t` and mark
//!   prefixes, the `y` operator, text objects after `i`/`a`, and the `/`/`?` prompt. It needs no
//!   text, so the model runs it as keys arrive and knows at once when copy mode ends.
//! - **[`Session`], the engine.** Runs a command over a [`Buffer`]: the terminal's rows, read
//!   lazily. It moves the cursor, keeps the visual anchor, marks and the last search, scrolls the
//!   view to follow the cursor and says what to yank. The terminal's worker runs it against the
//!   live buffer; tests run it against [`fixture::Fixture`].
//!
//! Positions are [`Pos`]: rows from the oldest scrollback row (0), columns from the left. Rows
//! are the screen's rows. `j`/`k`, `0`/`^`/`$`, `H`/`M`/`L` and the page keys move over them;
//! words, `f`/`t`, `%`, paragraphs and text objects read a soft-wrapped row and the rows it
//! continues on as one line.

mod buffer;
mod command;
mod engine;
pub mod fixture;
mod grammar;
mod motion;
mod object;

pub use buffer::{Buffer, Pos, Row};
pub use command::{Command, FindKind, Motion, Object, Visual};
pub use engine::{Outcome, Range, Session, Want};
pub use grammar::{Key, Prompt, Step, Vim};
