//! Fixture screens: recorded or generated program output (`fixtures/`), replayed through a real
//! terminal into the snapshot a pane draws. The scenes, the headless tests and the Ghostty pixel
//! diff all draw these.

use std::sync::Arc;

use terminal::{Config, Row, Snapshot, Theme, Vt};

/// A fixture: its name and the bytes a program wrote to a `COLS` × `ROWS` terminal.
pub struct Fixture {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

/// Every fixture is 80 × 30, the size it was recorded or generated at.
pub const COLS: u16 = 80;
pub const ROWS: u16 = 30;

/// 16, 256 and 24-bit colors, SGR styles, wide characters and combining marks (`make.py`).
pub const COLORS: Fixture = Fixture {
    name: "colors",
    bytes: include_bytes!("../fixtures/colors.bin"),
};

/// Box drawing: lines, corners, tees and crosses in light, heavy and double; arcs, dashes,
/// blocks, braille and Powerline (`make.py`).
pub const BOXES: Fixture = Fixture {
    name: "boxes",
    bytes: include_bytes!("../fixtures/boxes.bin"),
};

/// Neovim 0.12 with three splits, line numbers, a cursorline and a visual selection
/// (`fixtures/README.md` has the command line).
pub const NVIM_SPLITS: Fixture = Fixture {
    name: "nvim-splits",
    bytes: include_bytes!("../fixtures/nvim-splits.bin"),
};

/// The Neovim checklist (`nvim-checklist.lua`): splits, a rounded float, undercurls in their
/// colors, a statusline with Nerd Font icons and Powerline separators, CJK, emoji, and the
/// cmdline with the cursor in it (`fixtures/README.md` has the command line).
pub const NVIM_CHECKLIST: Fixture = Fixture {
    name: "nvim-checklist",
    bytes: include_bytes!("../fixtures/nvim-checklist.bin"),
};

pub const ALL: [&Fixture; 4] = [&COLORS, &BOXES, &NVIM_SPLITS, &NVIM_CHECKLIST];

impl Fixture {
    /// The fixture's screen, replayed into a fresh terminal with `theme`'s colors.
    pub fn snapshot(&self, theme: &Theme) -> Snapshot {
        replay(self.bytes, COLS, ROWS, theme)
    }
}

/// `bytes` written to a fresh `cols` × `rows` terminal with `theme`'s colors.
pub fn replay(bytes: &[u8], cols: u16, rows: u16, theme: &Theme) -> Snapshot {
    let mut vt = Vt::new(&Config {
        cols,
        rows,
        theme: theme.clone(),
        ..Config::default()
    });
    vt.write(bytes);
    vt.snapshot()
}

/// The screen file and grid the `screen-file` scene shows, from `HERDR_FRONTEND_SCREEN` (a file
/// of program output) and `HERDR_FRONTEND_GRID` (`COLSxROWS`, 80x30 if unset): how the Ghostty
/// pixel diff draws what Ghostty drew.
pub fn screen_file() -> Option<(Vec<u8>, u16, u16)> {
    let path = std::env::var_os("HERDR_FRONTEND_SCREEN")?;
    let bytes = std::fs::read(path).ok()?;
    let grid = std::env::var("HERDR_FRONTEND_GRID").unwrap_or_default();
    let (cols, rows) = grid
        .split_once('x')
        .and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)))
        .unwrap_or((COLS, ROWS));
    Some((bytes, cols, rows))
}

/// `snapshot` with columns `start..=end` of rows `rows` selected, as the terminal reports a
/// selection it holds.
pub fn select(
    snapshot: &mut Snapshot,
    rows: std::ops::RangeInclusive<usize>,
    start: u16,
    end: u16,
) {
    for y in rows {
        if let Some(row) = snapshot.screen.get_mut(y) {
            *row = Arc::new(Row {
                selection: Some((start, end)),
                ..(**row).clone()
            });
        }
    }
}

/// A pane showing `terminal`, as a `herdr-app` document writes one.
pub fn pane(id: &str, terminal: &str) -> serde_json::Value {
    serde_json::json!({"id": id, "type": "block", "content": {"terminal": terminal}})
}

/// A split of `children` along `axis` (`"row"` or `"column"`).
pub fn split(id: &str, axis: &str, children: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({"id": id, "type": axis, "children": children})
}

/// A tab named `name` whose root is `root`.
pub fn tab(id: &str, name: &str, root: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"id": id, "name": name, "root": root})
}

/// A live shell terminal at `cols` × `rows`.
pub fn terminal(address: &str, cols: u16, rows: u16) -> herdr_proto::Terminal {
    herdr_proto::Terminal {
        address: address.into(),
        cwd: "/tmp".into(),
        keep: true,
        cols,
        rows,
        ..Default::default()
    }
}

/// The demo core the shell scenes and tests run on: one space with two tabs. The first splits
/// Neovim beside the colors and box-drawing screens (`t1 | t2 / t3`), the second holds one
/// more shell. Each terminal shows its fixture screen, at the fixtures' 80 × 30.
pub fn demo_core() -> crate::link::Fake {
    let work = tab(
        "tb-work",
        "work",
        split(
            "tb-work",
            "row",
            vec![
                pane("pn-1", "t1"),
                split(
                    "c-1",
                    "column",
                    vec![pane("pn-2", "t2"), pane("pn-3", "t3")],
                ),
            ],
        ),
    );
    let logs = tab(
        "tb-logs",
        "logs",
        split("tb-logs", "row", vec![pane("pn-4", "t4")]),
    );
    let doc = serde_json::json!({"spaces": [{
        "id": "sp-1",
        "name": "herdr-frontend",
        "folder": "/tmp",
        "tabs": [work, logs],
    }]});
    // What runs in each, for the tabs' program icons: Neovim, then shells.
    let terminals = [("t1", "nvim"), ("t2", "zsh"), ("t3", "zsh"), ("t4", "zsh")]
        .into_iter()
        .map(|(a, program)| herdr_proto::Terminal {
            argv: vec![program.into()],
            ..terminal(a, COLS, ROWS)
        })
        .collect();
    let fake = crate::link::Fake::new(doc, terminals);
    {
        let mut core = fake.core();
        for (address, fixture) in [("t1", &NVIM_SPLITS), ("t2", &COLORS), ("t3", &BOXES)] {
            core.screens.insert(address.into(), fixture.bytes.to_vec());
        }
        core.screens.insert("t4".into(), b"$ ".to_vec());
    }
    fake
}

/// The tab strip's core: one space whose tabs show each kind of pill. `work` splits Neovim
/// beside a shell; an unnamed Pi tab goes by its session; an agent's tab (Claude, working) is
/// left out of the strip; an unnamed shell is numbered among the shown tabs (3, not its own 4);
/// a long name is cut short at 170 pt.
pub fn tab_strip_core() -> crate::link::Fake {
    let work = tab(
        "tb-work",
        "work",
        split(
            "tb-work",
            "row",
            vec![pane("pn-1", "t1"), pane("pn-2", "t2")],
        ),
    );
    let one = |id: &str, name: &str, pane_id: &str, address: &str| {
        tab(id, name, split(id, "row", vec![pane(pane_id, address)]))
    };
    let doc = serde_json::json!({"spaces": [{
        "id": "sp-1",
        "name": "herdr-frontend",
        "folder": "/tmp",
        "tabs": [
            work,
            one("tb-pi", "", "pn-3", "t3"),
            one("tb-agent", "", "pn-4", "t4"),
            one("tb-shell", "", "pn-5", "t5"),
            one("tb-long", "a tab whose name is longer than the strip lets a pill be", "pn-6", "t6"),
        ],
    }]});
    let mut terminals: Vec<herdr_proto::Terminal> =
        [("t1", "nvim"), ("t2", "zsh"), ("t5", "zsh"), ("t6", "zsh")]
            .into_iter()
            .map(|(a, program)| herdr_proto::Terminal {
                argv: vec![program.into()],
                ..terminal(a, COLS, ROWS)
            })
            .collect();
    terminals.push(herdr_proto::Terminal {
        argv: vec!["pi".into()],
        title: "π - tab strip - herdr-frontend".into(),
        cwd: "/code/herdr-frontend".into(),
        ..terminal("t3", COLS, ROWS)
    });
    terminals.push(herdr_proto::Terminal {
        argv: vec!["claude".into()],
        status: Some(herdr_proto::Status {
            state: herdr_proto::StatusState::Working,
            app: "claude".into(),
            ..Default::default()
        }),
        ..terminal("t4", COLS, ROWS)
    });
    let fake = crate::link::Fake::new(doc, terminals);
    {
        let mut core = fake.core();
        core.screens.insert("t1".into(), NVIM_SPLITS.bytes.to_vec());
        for address in ["t2", "t3", "t4", "t5", "t6"] {
            core.screens.insert(address.into(), b"$ ".to_vec());
        }
    }
    fake
}
