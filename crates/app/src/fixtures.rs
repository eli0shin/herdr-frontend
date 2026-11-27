//! Fixture screens: recorded or generated program output (`fixtures/`), replayed through a real
//! terminal into the snapshot a pane draws. The scenes, the headless tests and the Ghostty pixel
//! diff all draw these.

use std::sync::Arc;

use terminal::{Config, Row, Snapshot, Theme, Vt};

use crate::config::settings::Settings;
use crate::view::terminal::font::Font;

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

/// The settings the `screen-file` scene draws with, from `HERDR_FRONTEND_SETTINGS` (a
/// `settings.json`), `None` when it's unset or empty: how the Ghostty pixel diff draws a screen in another
/// font than Ghostty's default (`scripts/ghostty-diff styles`).
pub fn screen_settings() -> Result<Option<Settings>, String> {
    let Some(path) = std::env::var_os("HERDR_FRONTEND_SETTINGS").filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e}", std::path::Path::new(&path).display()))?;
    let (settings, problems) = Settings::parse(&text)?;
    match problems.is_empty() {
        true => Ok(Some(settings)),
        false => Err(problems.join("; ")),
    }
}

/// The terminal font the `screen-file` scene draws with: [`screen_settings`]' installed family,
/// else Ghostty's default, JetBrains Mono at 14 pt.
pub fn screen_font() -> Result<Font, String> {
    let Some(settings) = screen_settings()? else {
        return Ok(Font::jetbrains_mono(crate::TERMINAL_POINTS));
    };
    let config = crate::config::Config::with_fonts(settings);
    let s = &config.settings;
    let family = config
        .font
        .as_ref()
        .ok_or_else(|| config.problems.join("; "))?;
    let font = Font::from_family(family, s.font_size).map_err(|e| e.to_string())?;
    Ok(font.adjusted(s.terminal_line_height, s.terminal_cell_width))
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

/// An agent: a terminal whose program `app` reports `state` (OSC 7501), titled `title`.
pub fn agent(
    address: &str,
    app: &str,
    state: herdr_proto::StatusState,
    title: &str,
) -> herdr_proto::Terminal {
    herdr_proto::Terminal {
        title: title.into(),
        status: Some(herdr_proto::Status {
            state,
            app: app.into(),
            ..Default::default()
        }),
        ..terminal(address, COLS, ROWS)
    }
}

/// A Pi session with Eli's PR-watch label (`dotfiles.pi_pr_watch`).
pub fn pi(
    address: &str,
    state: herdr_proto::StatusState,
    title: &str,
    watch: serde_json::Value,
) -> herdr_proto::Terminal {
    let mut t = agent(address, "pi", state, title);
    t.labels
        .insert(model::agents::WATCH_LABEL.into(), watch.to_string());
    t
}

/// The agents core the sidebar scenes and tests run on: the demo core's `work` tab, then agents
/// in three spaces.
///
/// - `herdr-frontend`: Claude working on the sidebar (`t10`) with two sub-agents (`t20`
///   exploring, `t21` idle under it with `t22`), and Codex needing you (`t11`, tab "review").
/// - `dotfiles`: a Pi orchestrator (`t12`, ticket HF-23, watching #21) and its two workers,
///   `t13` working (#22, #23) and `t14` done.
/// - `notes`: a pinned Claude, idle (`t15`).
pub fn agents_core() -> crate::link::Fake {
    use herdr_proto::StatusState::*;
    let one = |tab_id: &str, name: &str, pane_id: &str, address: &str| {
        tab(
            tab_id,
            name,
            split(tab_id, "row", vec![pane(pane_id, address)]),
        )
    };
    let work = tab(
        "tb-work",
        "work",
        split("tb-work", "row", vec![pane("pn-1", "t1")]),
    );
    let doc = serde_json::json!({"spaces": [
        {"id": "sp-1", "name": "herdr-frontend", "folder": "/tmp", "tabs": [
            work,
            one("tb-a1", "", "pn-10", "t10"),
            one("tb-a2", "review", "pn-11", "t11"),
        ]},
        {"id": "sp-2", "name": "dotfiles", "folder": "/tmp", "tabs": [
            one("tb-o", "orchestrate", "pn-12", "t12"),
            one("tb-w1", "", "pn-13", "t13"),
            one("tb-w2", "fonts", "pn-14", "t14"),
        ]},
        {"id": "sp-3", "name": "notes", "folder": "/tmp", "tabs": [
            one("tb-n", "", "pn-15", "t15"),
        ]},
    ]});
    let sub = |address: &str, parent: &str, name: &str, state, created| herdr_proto::Terminal {
        parent: Some(parent.into()),
        name: Some(name.into()),
        created_ms: created,
        ..agent(address, "claude", state, "")
    };
    let mut done = pi(
        "t14",
        Idle,
        "π - font weights - dotfiles",
        serde_json::json!({"role": "worker", "orchestration": "o1"}),
    );
    // Finished, and no client has looked since: done.
    done.turn = 2;
    done.labels.insert("herdr.seen".into(), "1".into());
    let mut pinned = agent("t15", "claude", Idle, "✳ Meeting notes");
    pinned.labels.insert("herdr.pinned".into(), "1".into());
    let terminals = vec![
        herdr_proto::Terminal {
            argv: vec!["nvim".into()],
            ..terminal("t1", COLS, ROWS)
        },
        agent("t10", "claude", Working, "✳ Port the agents sidebar"),
        agent("t11", "codex", Blocked, "codex"),
        pi(
            "t12",
            Idle,
            "π - orchestrate - dotfiles",
            serde_json::json!({"role": "orchestrator", "orchestration": "o1", "ticket": "HF-23", "prs": ["#21"]}),
        ),
        pi(
            "t13",
            Working,
            "π - sidebar rows - dotfiles",
            serde_json::json!({"role": "worker", "orchestration": "o1", "prs": ["#22", "#23"]}),
        ),
        done,
        pinned,
        sub("t20", "t10", "explore", Working, 1),
        sub("t21", "t10", "tests", Idle, 2),
        sub("t22", "t21", "fixtures", Blocked, 3),
    ];
    let fake = crate::link::Fake::new(doc, terminals);
    {
        let mut core = fake.core();
        core.screens.insert("t1".into(), NVIM_SPLITS.bytes.to_vec());
        for t in ["t10", "t11", "t12", "t13", "t14", "t15"] {
            core.screens.insert(t.into(), b"$ ".to_vec());
        }
    }
    fake
}

/// A Pi reporting `state`, run as `pi`.
fn pi_agent(address: &str, state: herdr_proto::StatusState) -> herdr_proto::Terminal {
    herdr_proto::Terminal {
        argv: vec!["pi".into()],
        ..agent(address, "pi", state, "")
    }
}

/// The demo core with work in other spaces, for the pickers: `dotfiles` holds a Pi that needs
/// you, `repos` a working Pi with a sub-agent and a finished one, and `scratch` is quiet.
/// The demo's `herdr-frontend` space stays shown.
pub fn busy_core() -> crate::link::Fake {
    use herdr_proto::StatusState;
    let demo = demo_core();
    let mut doc = demo.core().docs[&None].0.clone();
    let space = |id: &str, name: &str, folder: &str, tabs: Vec<serde_json::Value>| serde_json::json!({"id": id, "name": name, "folder": folder, "tabs": tabs});
    let one = |tab_id: &str, name: &str, pane_id: &str, address: &str| {
        tab(
            tab_id,
            name,
            split(tab_id, "row", vec![pane(pane_id, address)]),
        )
    };
    let spaces = doc["spaces"].as_array_mut().expect("the demo's spaces");
    spaces.push(space(
        "sp-2",
        "dotfiles",
        "/tmp/dotfiles",
        vec![
            one("tb-review", "review the stow", "pn-a1", "a1"),
            one("tb-dot", "shell", "pn-5", "t5"),
        ],
    ));
    spaces.push(space(
        "sp-3",
        "repos",
        "/tmp/repos",
        vec![
            one("tb-fix", "fix the rebase", "pn-a2", "a2"),
            one("tb-docs", "docs", "pn-a3", "a3"),
        ],
    ));
    spaces.push(space(
        "sp-4",
        "scratch",
        "/tmp/scratch",
        vec![one("tb-scratch", "", "pn-6", "t6")],
    ));
    let mut terminals: Vec<herdr_proto::Terminal> =
        demo.core().terminals.values().cloned().collect();
    let mut kid = pi_agent("a4", StatusState::Working);
    kid.parent = Some("a2".into());
    kid.keep = false;
    terminals.extend([
        pi_agent("a1", StatusState::Blocked),
        terminal("t5", COLS, ROWS),
        pi_agent("a2", StatusState::Working),
        pi_agent("a3", StatusState::Done),
        kid,
        terminal("t6", COLS, ROWS),
    ]);
    let fake = crate::link::Fake::new(doc, terminals);
    {
        let mut core = fake.core();
        let screens = demo.core().screens.clone();
        core.screens = screens;
    }
    fake
}
