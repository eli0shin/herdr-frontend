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
