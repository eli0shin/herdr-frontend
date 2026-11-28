//! Every frame a terminal shows, compared with a fresh terminal's render of the same output.
//!
//! A [`Vt`] builds each snapshot from the last one: libghostty-vt's render state rebuilds only the
//! rows it marks dirty, and the snapshot hands back a row it kept clean as the same `Arc`. A row
//! kept when its cells changed would show stale text until something redraws it. So these tests
//! feed recorded programs in small writes, as output arrives from the core, and after each write
//! that leaves a frame to show compare every row of the snapshot with a terminal that never drew
//! an earlier frame: a fresh one, fed everything so far in one write.
//!
//! The recordings are the replay tests' (`streams/`, recorded with `streams/record.py`), with
//! the command line of each next to the test that uses it.

use std::time::Duration;

use terminal::{Cell, Config, Snapshot, Vt};

fn stream(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/streams/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn vt(cols: u16, rows: u16) -> Vt {
    Vt::new(&Config {
        cols,
        rows,
        cell_width: 10,
        cell_height: 22,
        // Time doesn't pass in a replay: a render hold ends only when the program ends it.
        render_hold_timeout: Duration::from_secs(3600),
        ..Config::default()
    })
}

/// What a row shows: everything but its identity.
fn shown(s: &Snapshot) -> Vec<(bool, &[Cell])> {
    s.screen
        .iter()
        .map(|row| (row.wrapped, row.cells.as_slice()))
        .collect()
}

/// Replays `bytes` at `cols` × `rows` in writes of `chunk` bytes, comparing each frame with a
/// fresh terminal's. Returns how many frames it compared.
fn every_frame_matches_a_fresh_render(
    name: &str,
    bytes: &[u8],
    (cols, rows): (u16, u16),
    chunk: usize,
) -> usize {
    let mut live = vt(cols, rows);
    let mut compared = 0;
    let mut end = 0;
    for write in bytes.chunks(chunk) {
        live.write(write);
        end += write.len();
        // A program holding its frame (synchronized output) shows nothing new until it lets go,
        // as the worker publishes nothing then.
        if !live.can_show() {
            continue;
        }
        let frame = live.snapshot();
        let mut fresh = vt(cols, rows);
        fresh.write(&bytes[..end]);
        let expected = fresh.snapshot();
        assert_eq!(frame.modes.held, expected.modes.held, "{name} at {end}");
        compared += 1;
        let (got, want) = (shown(&frame), shown(&expected));
        for y in 0..got.len().max(want.len()) {
            assert!(
                got.get(y) == want.get(y),
                "{name}, {chunk}-byte writes, frame at byte {end}: row {y} is stale\n  \
                 shown: {:?}\n  fresh: {:?}",
                frame.line(y),
                expected.line(y),
            );
        }
        assert_eq!(frame.cursor, expected.cursor, "{name} at {end}");
    }
    compared
}

/// [`every_frame_matches_a_fresh_render`] in writes of a few sizes. The smallest must give at
/// least `min_frames` frames, or the recording no longer shows what the test is for.
fn replay(name: &str, size: (u16, u16), min_frames: usize) {
    let bytes = stream(name);
    for chunk in [13, 256, 4096] {
        let compared = every_frame_matches_a_fresh_render(name, &bytes, size, chunk);
        assert!(
            chunk > 13 || compared >= min_frames,
            "{name}, {chunk}-byte writes: only {compared} frames to compare"
        );
    }
}

/// Neovim 0.12.6 in diff mode, as diffview draws: two windows of filler lines and changes, then
/// scrolling them (left and right margins and scroll regions), a bordered float opened and
/// closed over them, splits, a tab and back. Recorded at 100 × 30 with Neovim's synchronized
/// output on, as it is with our terminal:
///
/// ```sh
/// cd crates/terminal/tests/streams
/// python3 -I record.py nvim-floats.bin 100 30 -- \
///     nvim -n -i NONE -u "$PWD/nvim-floats.lua" -d sample.txt pager.txt < steps
/// ```
///
/// where `steps` is (`\x17` is Ctrl-W, `\x05` Ctrl-E, `\x04` Ctrl-D, `\x19` Ctrl-Y):
///
/// ```text
/// sleep 1.5
/// send "\x17l"
/// sleep 0.3
/// send "20j"
/// sleep 0.3
/// send "\x05\x05\x05"
/// sleep 0.3
/// send "\x04"
/// sleep 0.3
/// send ":lua Float()\r"
/// sleep 0.5
/// send "jj"
/// sleep 0.3
/// send ":close\r"
/// sleep 0.3
/// send ":vsplit\r"
/// sleep 0.3
/// send ":split\r"
/// sleep 0.3
/// send "\x19\x19"
/// sleep 0.3
/// send ":only\r"
/// sleep 0.3
/// send ":tabnew sample.txt\r"
/// sleep 0.3
/// send ":tabclose\r"
/// sleep 0.3
/// send "gg"
/// sleep 0.3
/// send ":qa!\r"
/// sleep 0.5
/// ```
#[test]
fn neovim_diff_floats_and_splits_repaint_every_row() {
    replay("nvim-floats.bin", (100, 30), 20);
}

/// Neovim's startup, an edit and `:q!` (`replay.rs` has its command line).
#[test]
fn neovim_startup_repaints_every_row() {
    replay("nvim-startup.bin", (80, 24), 3);
}

/// `less` paging on the alternate screen and back (`replay.rs`).
#[test]
fn less_repaints_every_row() {
    replay("less-alt-screen.bin", (80, 24), 3);
}

/// Meters redrawn in place, as htop does (`replay.rs`).
#[test]
fn meters_repaint_every_row() {
    replay("meters.bin", (80, 24), 3);
}
