//! Replays recorded program output through a [`Vt`] and checks the screen and the replies.
//!
//! Each stream in `streams/` was recorded with `streams/record.py` at 80x24, from the
//! `streams/` directory, with the script shown next to its test (`\x1b` is Esc). The recorder
//! answered the startup queries itself, so the replies checked here are our terminal's own.

use std::sync::Arc;

use terminal::{
    ActiveScreen, CellFlags, Config, CursorStyle, Dirty, Rgb, Snapshot, Theme, Underline, Vt, Width,
};

fn stream(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/streams/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// A meter row as `meters.py` draws it: `load * 0.4` bars padded to 40 columns, then the load.
fn meter(n: usize, load: usize) -> String {
    let bar = load * 4 / 10;
    format!(
        "{n:>2}[{}{} {load:>2}%]",
        "|".repeat(bar),
        " ".repeat(40 - bar)
    )
}

/// The offset of the first `needle` in `bytes`.
fn find(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap_or_else(|| panic!("{:?} not in the stream", String::from_utf8_lossy(needle)))
}

fn vt() -> Vt {
    Vt::new(&Config {
        cell_width: 10,
        cell_height: 22,
        ..Config::default()
    })
}

fn replies(vt: &mut Vt) -> String {
    String::from_utf8(vt.take_replies()).expect("utf-8 replies")
}

/// `python3 -I record.py nvim-startup.bin 80 24 -- nvim --clean sample.txt`, NVIM v0.12.6:
///
/// ```text
/// sleep 1.5
/// send "jjj"
/// sleep 0.5
/// send "A // edited"
/// sleep 0.5
/// send "\x1b"
/// sleep 0.5
/// send ":q!\r"
/// sleep 1
/// ```
#[test]
fn neovim_startup_edit_and_quit() {
    let bytes = stream("nvim-startup.bin");
    let mut vt = Vt::new(&Config {
        cell_width: 10,
        cell_height: 22,
        theme: Theme {
            background: Some(Rgb::new(0x11, 0x22, 0x33)),
            ..Theme::default()
        },
        ..Config::default()
    });

    // Up to insert mode: the file is drawn, and Neovim has turned on its modes.
    let insert = find(&bytes, b"-- INSERT --");
    vt.write(&bytes[..insert]);
    let s = vt.snapshot();
    assert_eq!(s.active_screen, ActiveScreen::Alternate);
    assert_eq!((s.cols, s.rows), (80, 24));
    assert_eq!(s.line(0), "# A sample file for the replay tests");
    assert_eq!(s.line(3), r#"    let greeting = "hello, terminal";"#);
    assert_eq!(
        s.line(7),
        "Wide: 日本語 and an emoji 🎉 sit in two cells each."
    );
    assert_eq!(s.line(8), "Box: ┌──┐ │ok│ └──┘");
    assert_eq!(s.line(9), "~");
    assert_eq!(
        s.line(22),
        "sample.txt                                                    4,1            All"
    );
    assert!(s.modes.bracketed_paste);
    assert!(s.modes.mouse_tracking);
    assert!(s.modes.focus_reporting);

    // 日 at column 6 takes columns 6 and 7; the emoji sits after "and an emoji ".
    let wide = &s.screen[7].cells;
    assert_eq!((wide[6].text.as_str(), wide[6].width), ("日", Width::Wide));
    assert_eq!(wide[7].width, Width::SpacerTail);
    assert_eq!(
        (wide[26].text.as_str(), wide[26].width),
        ("🎉", Width::Wide)
    );
    // The status line is inverse.
    assert!(s.screen[22].cells[0].flags.contains(CellFlags::INVERSE));

    // Neovim's startup probes, answered in order. The recorder answered the same queries
    // itself, so Neovim's later behavior matches what it would do on our terminal.
    let answered = replies(&mut vt);
    let expected = [
        // DECRQM for modes 69, 2026, 2027, 2031 and 2048: 2027 is on from the start.
        "\x1b[?69;2$y",
        "\x1b[?2026;2$y",
        "\x1b[?2027;1$y",
        "\x1b[?2031;2$y",
        "\x1b[?2048;2$y",
        // DECRQSS of the SGR after `CSI 4:3 m`: undercurl is supported.
        "\x1bP1$r0;4:3m\x1b\\",
        // Kitty keyboard flags, primary device attributes, then OSC 11 (our theme's
        // background) and device status, twice.
        "\x1b[?0u",
        "\x1b[?62;22c",
        "\x1b]11;rgb:1111/2222/3333\x07",
        "\x1b[0n",
        "\x1b]11;rgb:1111/2222/3333\x07",
        "\x1b[0n",
        // XTGETTCAP: Tc (a flag, so no value), RGB=8, and the truecolor setrgbf/setrgbb strings.
        "\x1bP1+r5463\x1b\\",
        "\x1bP1+r524742=38\x1b\\",
        "\x1bP1+r73657472676266=5C455B33383A323A25703125643A25703225643A25703325646D\x1b\\",
        "\x1bP1+r73657472676262=5C455B34383A323A25703125643A25703225643A25703325646D\x1b\\",
        // DECRQSS after a truecolor background: Neovim checks the color round-trips.
        "\x1bP1$r0;48:2::1:2:3m\x1b\\",
        "\x1b[?62;22c",
        // XTGETTCAP Ms, the OSC 52 clipboard string.
        "\x1bP1+r4D73=5C455D35323B25703125733B25703225735C303037\x1b\\",
    ];
    assert_eq!(answered, expected.concat());

    // Insert mode, once Neovim has shown its cursor after the text: the text is in and the
    // cursor is a bar.
    let shown = b"\x1b[4;48H\x1b[?25h";
    let typed = find(&bytes, shown) + shown.len();
    vt.write(&bytes[insert..typed]);
    let s = vt.snapshot();
    assert_eq!(
        s.line(3),
        r#"    let greeting = "hello, terminal"; // edited"#
    );
    assert_eq!(s.line(23), "-- INSERT --");
    assert_eq!(s.cursor.style, CursorStyle::Bar);
    assert_eq!(s.cursor.position, Some((47, 3)));
    assert!(s.cursor.visible);
    assert_eq!(replies(&mut vt), "");

    // Esc and :q! leaves the alternate screen: the primary screen is back, empty, and every mode
    // Neovim turned on is off again.
    vt.write(&bytes[typed..]);
    let s = vt.snapshot();
    assert_eq!(s.active_screen, ActiveScreen::Primary);
    assert_eq!(s.text(), "");
    assert!(s.cursor.visible);
    assert!(!s.modes.bracketed_paste && !s.modes.mouse_tracking && !s.modes.focus_reporting);
    assert_eq!(s.dirty, Dirty::Full);
    // The final device-attributes query on the way out.
    assert_eq!(replies(&mut vt), "\x1b[?62;22c");
}

/// `python3 -I record.py less-alt-screen.bin 80 24 -- less pager.txt`, macOS `/usr/bin/less`:
///
/// ```text
/// sleep 1
/// send " "
/// sleep 0.5
/// send "q"
/// sleep 0.5
/// ```
#[test]
fn less_pages_on_the_alternate_screen_and_restores_the_shell() {
    let bytes = stream("less-alt-screen.bin");
    let mut vt = vt();
    // Some shell output first, so there's a primary screen to come back to.
    vt.write(b"$ less pager.txt\r\n");

    let first_page = find(&bytes, b"\x1b[7mpager.txt");
    vt.write(&bytes[..first_page]);
    let s = vt.snapshot();
    assert_eq!(s.active_screen, ActiveScreen::Alternate);
    assert_eq!(s.line(0), "line 1 of the pager fixture");
    assert_eq!(s.line(22), "line 23 of the pager fixture");

    let prompt = find(&bytes, b":\x1b[K");
    vt.write(&bytes[first_page..prompt]);
    let s = vt.snapshot();
    assert_eq!(s.line(0), "line 24 of the pager fixture");
    assert_eq!(s.line(22), "line 46 of the pager fixture");
    // The alternate screen keeps no scrollback.
    assert_eq!(s.scrollbar.total, 24);
    assert!(s.scrollbar.at_bottom);

    vt.write(&bytes[prompt..]);
    let s = vt.snapshot();
    assert_eq!(s.active_screen, ActiveScreen::Primary);
    assert_eq!(s.text(), "$ less pager.txt");
    assert_eq!(s.cursor.position, Some((0, 1)));
    // less asks nothing.
    assert_eq!(replies(&mut vt), "");
    assert_eq!(vt.take_events(), []);
}

/// An htop-style curses screen (`streams/meters.py`: colored meters redrawn in place, a boxed
/// table, a status line), since htop's own output depends on the machine.
/// `python3 -I record.py meters.bin 80 24 -- python3 -I meters.py`, ncurses 6:
///
/// ```text
/// sleep 1.2
/// send "q"
/// sleep 0.5
/// ```
#[test]
fn curses_meters_redraw_in_place() {
    let bytes = stream("meters.bin");
    let mut vt = vt();

    // Frame one. curses draws the table window, then clears the screen under it (`CSI 2J`) and
    // draws the meters, so the table first shows in frame two.
    let second = find(&bytes, b"\r\x1b[7d");
    vt.write(&bytes[..second]);
    let one = vt.snapshot();
    assert_eq!(one.active_screen, ActiveScreen::Alternate);
    assert_eq!(one.line(0), meter(1, 12));
    assert_eq!(one.line(1), meter(2, 55));
    assert_eq!(one.line(2), meter(3, 90));
    assert_eq!(one.line(4), "Tasks: 40, load average: 0.02");
    assert_eq!(one.line(6), "");
    assert_eq!(one.line(23), " F10 Quit");
    assert!(!one.cursor.visible);

    // Colors resolve through the default palette: bold green on black under 70%, red above.
    let palette = one.colors.palette;
    let green = &one.screen[0].cells[3];
    assert_eq!(green.text.as_str(), "|");
    assert_eq!((green.fg, green.bg), (Some(palette[2]), Some(palette[0])));
    assert!(green.flags.contains(CellFlags::BOLD));
    assert_eq!(one.screen[2].cells[3].fg, Some(palette[1]));
    // `use_default_colors`: cyan on the default background, and black on cyan.
    let tasks = &one.screen[4].cells[0];
    assert_eq!((tasks.fg, tasks.bg), (Some(palette[6]), None));
    let quit = &one.screen[23].cells[1];
    assert_eq!((quit.fg, quit.bg), (Some(palette[0]), Some(palette[6])));

    // Frame two draws the table, rewrites the meters and patches the task count. The rows it
    // didn't touch come back as the same `Arc`s, except the status line: the cursor left it and
    // came back, and libghostty marks the rows a cursor leaves and enters dirty.
    let third = find(&bytes, b"\x1b[8;6H3");
    vt.write(&bytes[second..third]);
    let two = vt.snapshot();
    assert_eq!(two.dirty, Dirty::Partial);
    assert_eq!(two.line(0), meter(1, 42));
    assert_eq!(two.line(4), "Tasks: 41, load average: 0.12");
    // DEC special graphics come out as box drawing; the title is inverse.
    assert_eq!(
        two.line(6),
        "┌─ PID  NAME ────────────────────────────────────┐"
    );
    assert!(two.screen[6].cells[3].flags.contains(CellFlags::INVERSE));
    assert_eq!(two.screen[6].cells[3].underline, Underline::None);
    assert_eq!(
        two.line(7),
        "│  102  alpha      3%                            │"
    );
    assert_eq!(
        two.line(11),
        "└────────────────────────────────────────────────┘"
    );
    let kept: Vec<usize> = (0..24)
        .filter(|&y| Arc::ptr_eq(&one.screen[y], &two.screen[y]))
        .collect();
    assert_eq!(kept, [3, 5, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22]);
    for &y in &kept {
        assert_eq!(one.screen[y].id, two.screen[y].id);
    }

    // A snapshot with nothing new reuses every row.
    let again = vt.snapshot();
    assert_eq!(again.dirty, Dirty::Clean);
    assert!((0..24).all(|y| Arc::ptr_eq(&two.screen[y], &again.screen[y])));
    assert_eq!(again.generation, two.generation + 1);

    // The last frame, then `q`: back on the empty primary screen.
    let exit = find(&bytes, b"\x1b[?1049l");
    vt.write(&bytes[third..exit]);
    let three = vt.snapshot();
    assert_eq!(three.line(0), meter(1, 72));
    assert_eq!(three.line(4), "Tasks: 42, load average: 0.22");
    assert_eq!(
        three.line(9),
        "│  305  gamma      8%                            │"
    );
    vt.write(&bytes[exit..]);
    let done = vt.snapshot();
    assert_eq!(done.active_screen, ActiveScreen::Primary);
    assert_eq!(done.text(), "");
    assert!(done.cursor.visible);
    assert_eq!(replies(&mut vt), "");
}

/// The snapshot the replays build is self-contained: it outlives the `Vt` and moves to another
/// thread, as the worker's handoff needs.
#[test]
fn snapshots_outlive_their_terminal() {
    let mut vt = vt();
    vt.write(b"kept\r\n");
    let s: Snapshot = vt.snapshot();
    drop(vt);
    let line = std::thread::spawn(move || s.line(0)).join().unwrap();
    assert_eq!(line, "kept");
}
