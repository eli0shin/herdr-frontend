//! Smoke test of the linked library: write VT bytes, resize, and read the render state back.

use std::ptr;

use ghostty_vt_sys::*;

#[test]
fn library_is_the_pinned_release() {
    let version = check_version().expect("linked library matches the bindings");
    assert_eq!(version.full, PINNED_VERSION);
    assert_eq!(header_version(), PINNED_VERSION);
    assert_eq!(
        (version.major, version.minor, version.patch),
        (
            GHOSTTY_VT_VERSION_MAJOR as usize,
            GHOSTTY_VT_VERSION_MINOR as usize,
            GHOSTTY_VT_VERSION_PATCH as usize
        )
    );
}

#[test]
fn write_resize_and_read_render_state() {
    // SAFETY: every handle is created before use and freed once, every out pointer matches the
    // output type its key documents, and row and cell data are read before the next update.
    unsafe {
        let mut term: GhosttyTerminal = ptr::null_mut();
        ok(ghostty_terminal_new(ptr::null(), &mut term, 80, 24));

        let bytes = b"hello\r\n\x1b[31mworld\x1b[0m";
        ghostty_terminal_vt_write(term, bytes.as_ptr(), bytes.len());
        ok(ghostty_terminal_resize(term, 100, 30, 17, 37));

        let width_px: u32 = terminal_get(term, GHOSTTY_TERMINAL_DATA_WIDTH_PX);
        let height_px: u32 = terminal_get(term, GHOSTTY_TERMINAL_DATA_HEIGHT_PX);
        assert_eq!((width_px, height_px), (1700, 1110));

        let mut state: GhosttyRenderState = ptr::null_mut();
        ok(ghostty_render_state_new(ptr::null(), &mut state));
        ok(ghostty_render_state_update(state, term));

        let cols: u16 = state_get(state, GHOSTTY_RENDER_STATE_DATA_COLS);
        let rows: u16 = state_get(state, GHOSTTY_RENDER_STATE_DATA_ROWS);
        assert_eq!((cols, rows), (100, 30));
        let dirty: GhosttyRenderStateDirty = state_get(state, GHOSTTY_RENDER_STATE_DATA_DIRTY);
        assert_eq!(dirty, GHOSTTY_RENDER_STATE_DIRTY_FULL);

        let mut colors = GhosttyRenderStateColors {
            size: size_of::<GhosttyRenderStateColors>(),
            ..Default::default()
        };
        ok(ghostty_render_state_get(
            state,
            GHOSTTY_RENDER_STATE_DATA_COLORS,
            (&raw mut colors).cast(),
        ));

        let mut iter: GhosttyRenderStateRowIterator = ptr::null_mut();
        ok(ghostty_render_state_row_iterator_new(
            ptr::null(),
            &mut iter,
        ));
        ok(ghostty_render_state_get(
            state,
            GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
            (&raw mut iter).cast(),
        ));
        let mut cells: GhosttyRenderStateRowCells = ptr::null_mut();
        ok(ghostty_render_state_row_cells_new(ptr::null(), &mut cells));

        let mut lines = Vec::new();
        let mut first_fg = Vec::new();
        while ghostty_render_state_row_iterator_next(iter) {
            ok(ghostty_render_state_row_get(
                iter,
                GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                (&raw mut cells).cast(),
            ));
            let mut line = String::new();
            let mut fg = None;
            let mut x = 0;
            while ghostty_render_state_row_cells_next(cells) {
                line.push_str(&cell_text(cells));
                if x == 0 {
                    fg = cell_fg(cells);
                }
                x += 1;
            }
            assert_eq!(x, 100, "every row has the resized width");
            lines.push(line.trim_end().to_owned());
            first_fg.push(fg);
        }

        assert_eq!(lines.len(), 30);
        assert_eq!(lines[0], "hello");
        assert_eq!(lines[1], "world");
        assert!(lines[2..].iter().all(String::is_empty), "{lines:?}");
        assert_eq!(first_fg[0], None, "default foreground");
        assert_eq!(
            first_fg[1],
            Some(rgb(colors.palette[1])),
            "SGR 31 is palette red"
        );

        ghostty_render_state_row_cells_free(cells);
        ghostty_render_state_row_iterator_free(iter);
        ghostty_render_state_free(state);
        ghostty_terminal_free(term);
    }
}

fn ok(result: GhosttyResult) {
    assert_eq!(result, GHOSTTY_SUCCESS);
}

unsafe fn terminal_get<T: Default>(term: GhosttyTerminal, key: GhosttyTerminalData) -> T {
    let mut out = T::default();
    ok(unsafe { ghostty_terminal_get(term, key, (&raw mut out).cast()) });
    out
}

unsafe fn state_get<T: Default>(state: GhosttyRenderState, key: GhosttyRenderStateData) -> T {
    let mut out = T::default();
    ok(unsafe { ghostty_render_state_get(state, key, (&raw mut out).cast()) });
    out
}

/// The current cell's grapheme cluster, or a space for an empty cell.
unsafe fn cell_text(cells: GhosttyRenderStateRowCells) -> String {
    let mut bytes = [0u8; 32];
    let mut buf = GhosttyBuffer {
        ptr: bytes.as_mut_ptr(),
        cap: bytes.len(),
        len: 0,
    };
    ok(unsafe {
        ghostty_render_state_row_cells_get(
            cells,
            GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
            (&raw mut buf).cast(),
        )
    });
    if buf.len == 0 {
        " ".to_owned()
    } else {
        String::from_utf8(bytes[..buf.len].to_vec()).unwrap()
    }
}

unsafe fn cell_fg(cells: GhosttyRenderStateRowCells) -> Option<(u8, u8, u8)> {
    let mut color = GhosttyColorRgb::default();
    let result = unsafe {
        ghostty_render_state_row_cells_get(
            cells,
            GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
            (&raw mut color).cast(),
        )
    };
    match result {
        GHOSTTY_SUCCESS => Some(rgb(color)),
        GHOSTTY_NO_VALUE => None,
        other => panic!("FG_COLOR returned {other}"),
    }
}

fn rgb(c: GhosttyColorRgb) -> (u8, u8, u8) {
    (c.r, c.g, c.b)
}
