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
