//! The chrome's one-color glyphs, tinted with `.color()`: the same shapes as the icons Herdr
//! app's shipped mods draw (`mods/workspace-chrome/*.svg`, `mods/pane/close.svg`, Lucide-style
//! glyphs, `assets/icons/LICENSES.md`).

use rune::Icon;

/// Working: a ring at a quarter's strength with a quarter arc on it, turned once per 900 ms
/// (`workspace-chrome/spinner.svg`).
pub static SPINNER: Icon = Icon {
    name: "herdr-spinner",
    view: 24.0,
    body: r#"<circle cx="12" cy="12" r="9.4" fill="none" stroke="currentColor" stroke-width="3.2" stroke-opacity="0.24"/><path d="M12 2.6a9.4 9.4 0 0 1 9.4 9.4" fill="none" stroke="currentColor" stroke-width="3.2" stroke-linecap="round"/>"#,
};

/// A ×: closes (`pane/close.svg`).
pub static CLOSE: Icon = Icon {
    name: "herdr-close",
    view: 16.0,
    body: r#"<path d="M4 4l8 8M12 4l-8 8" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#,
};

/// This computer (`workspace-chrome/monitor.svg`).
pub static MONITOR: Icon = Icon {
    name: "herdr-monitor",
    view: 24.0,
    body: r#"<rect x="3" y="4" width="18" height="12" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M8.5 20h7M12 16v4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

/// Another machine (`workspace-chrome/server.svg`).
pub static SERVER: Icon = Icon {
    name: "herdr-server",
    view: 24.0,
    body: r#"<rect x="3.5" y="4" width="17" height="7" rx="2" fill="none" stroke="currentColor" stroke-width="1.5"/><rect x="3.5" y="13" width="17" height="7" rx="2" fill="none" stroke="currentColor" stroke-width="1.5"/>"#,
};

/// Lucide's stroke, as the Lucide glyphs below draw.
macro_rules! lucide {
    ($paths:literal) => {
        concat!(
            r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#,
            $paths,
            "</g>"
        )
    };
}

/// The Agents list sorted by priority (`workspace-chrome/clock.svg`).
pub static CLOCK: Icon = Icon {
    name: "herdr-clock",
    view: 24.0,
    body: lucide!(r#"<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>"#),
};

/// The Agents list grouped by space (`workspace-chrome/layers.svg`).
pub static LAYERS: Icon = Icon {
    name: "herdr-layers",
    view: 24.0,
    body: lucide!(
        r#"<path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z"/><path d="M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12"/><path d="M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17"/>"#
    ),
};

/// The current choice in a menu (`workspace-chrome/check.svg`).
pub static CHECK: Icon = Icon {
    name: "herdr-check",
    view: 24.0,
    body: lucide!(r#"<path d="M20 6 9 17l-5-5"/>"#),
};

/// Open: sub-agents shown (`workspace-chrome/chevron-down.svg`).
pub static CHEVRON_DOWN: Icon = Icon {
    name: "herdr-chevron-down",
    view: 24.0,
    body: lucide!(r#"<path d="m6 9 6 6 6-6"/>"#),
};

/// Folded: sub-agents hidden (`workspace-chrome/chevron-right.svg`).
pub static CHEVRON_RIGHT: Icon = Icon {
    name: "herdr-chevron-right",
    view: 24.0,
    body: lucide!(r#"<path d="m9 18 6-6-6-6"/>"#),
};

/// A watched pull request (Lucide's `git-pull-request`, as `eli-agents` ships it).
pub static GIT_PULL_REQUEST: Icon = Icon {
    name: "herdr-git-pull-request",
    view: 24.0,
    body: lucide!(
        r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><path d="M6 9v12"/>"#
    ),
};

/// Renaming (`workspace-chrome/square-pen.svg`).
pub static RENAME: Icon = Icon {
    name: "herdr-square-pen",
    view: 24.0,
    body: lucide!(
        r#"<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/>"#
    ),
};

/// Something new (`workspace-chrome/plus.svg`).
pub static PLUS: Icon = Icon {
    name: "herdr-plus",
    view: 24.0,
    body: lucide!(r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#),
};

/// A split to the right (`workspace-chrome/panel-right.svg`).
pub static PANEL_RIGHT: Icon = Icon {
    name: "herdr-panel-right",
    view: 24.0,
    body: lucide!(r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M15 3v18"/>"#),
};

/// A split below (`workspace-chrome/panel-bottom.svg`).
pub static PANEL_BOTTOM: Icon = Icon {
    name: "herdr-panel-bottom",
    view: 24.0,
    body: lucide!(r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 15h18"/>"#),
};

/// Zoom (`workspace-chrome/square.svg`).
pub static SQUARE: Icon = Icon {
    name: "herdr-square",
    view: 24.0,
    body: lucide!(r#"<rect width="18" height="18" x="3" y="3" rx="2"/>"#),
};

/// A terminal (`workspace-chrome/terminal.svg`).
pub static TERMINAL: Icon = Icon {
    name: "herdr-terminal",
    view: 24.0,
    body: lucide!(r#"<path d="M12 19h8"/><path d="m4 17 6-6-6-6"/>"#),
};

/// Closing, in menus (`workspace-chrome/x.svg`).
pub static MENU_CLOSE: Icon = Icon {
    name: "herdr-menu-x",
    view: 24.0,
    body: lucide!(r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#),
};

/// A search field (`workspace-chrome/search.svg`).
pub static SEARCH: Icon = Icon {
    name: "herdr-search",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="6.5"/><path d="m20 20-4-4"/></g>"#,
};

/// The palette's field (`frame/palette/command.svg`).
pub static COMMAND: Icon = Icon {
    name: "herdr-command",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"/></g>"#,
};

/// A file (`workspace-chrome/file-text.svg`).
pub static FILE: Icon = Icon {
    name: "herdr-file-text",
    view: 24.0,
    body: lucide!(
        r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#
    ),
};

/// A folder (`workspace-chrome/folder.svg`).
pub static FOLDER: Icon = Icon {
    name: "herdr-folder",
    view: 24.0,
    body: lucide!(
        r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#
    ),
};

/// A menu item's icon (`cfg/mods/menu/main.js` `menuIcon`).
/// Copying a command (Lucide `copy`).
pub static COPY: Icon = Icon {
    name: "herdr-copy",
    view: 24.0,
    body: lucide!(
        r#"<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#
    ),
};

pub fn menu_icon(icon: model::MenuIcon) -> &'static Icon {
    use model::MenuIcon as M;
    match icon {
        M::Close => &MENU_CLOSE,
        M::Rename => &RENAME,
        M::New => &PLUS,
        M::SplitRight => &PANEL_RIGHT,
        M::SplitDown => &PANEL_BOTTOM,
        M::Zoom => &SQUARE,
        M::Terminal => &TERMINAL,
        M::Machine => &SERVER,
        M::Copy => &COPY,
    }
}
