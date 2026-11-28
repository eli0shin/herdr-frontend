//! The look and settings through Rune's real runtime with no window (`docs/settings.md`): Eli's
//! defaults, the arctic theme on the shell, zoom and its keys (saved as state, never in
//! `settings.json`), and live reload from an isolated home.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use herdr_frontend::config::settings::Settings;
use herdr_frontend::config::theme::Theme;
use herdr_frontend::fixtures::demo_core;
use herdr_frontend::home::Home;
use herdr_frontend::link::Offline;
use herdr_frontend::window::Keeper;
use herdr_frontend::{SIZE, State, app};
use model::{Action, AppCommand};
use rune::{ElementInfo, Env, Runtime};

fn frames(rt: &mut Runtime<State>, n: usize) {
    for _ in 0..n {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }
}

fn element(rt: &Runtime<State>, key: &str) -> ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

/// A scratch herdr-frontend home, never the real one.
fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("herdr-frontend-look-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The app on the demo core with its home at `home`, as `live` makes it but on a fake core.
fn runtime_at(home: &Path) -> Runtime<State> {
    runtime_in(Home::at(home))
}

fn runtime_in(home: Home) -> Runtime<State> {
    let mut rt = app()
        .init(move || State::new(Box::new(Offline), Some(home), Keeper::default()))
        .headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.connect(Box::new(demo_core()));
    rt.state.settle();
    frames(&mut rt, 3);
    rt
}

/// Within half a device pixel at zoom 1.1 on a 2x display.
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.25
}

fn until(rt: &mut Runtime<State>, what: &str, done: impl Fn(&Runtime<State>) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done(rt) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
        frames(rt, 1);
    }
    frames(rt, 2);
}

#[test]
fn with_no_settings_file_the_window_is_elis_arctic_at_zoom_1_1() {
    let home = scratch("defaults");
    let rt = runtime_at(&home);
    assert_eq!(rt.state.config.settings, Settings::default());
    assert_eq!(rt.zoom(), 1.1);

    // Rune's zoom: the view lays out in 1 / 1.1 as many points, and the layout's sizes stay as
    // designed in them.
    let root = element(&rt, "root");
    assert!(
        (root.rect[2] - SIZE.0 / 1.1).abs() < 0.01,
        "{:?}",
        root.rect
    );
    // (Rune snaps them to the zoomed scale's device pixels.)
    let brand = element(&rt, "brand");
    assert!(
        near(brand.rect[2], 366.0) && near(brand.rect[3], 38.0),
        "{:?}",
        brand.rect
    );

    // Arctic's panels and terminals.
    let arctic = Theme::arctic().chrome();
    assert_eq!(element(&rt, "root").fill, arctic.canvas);
    assert_eq!(element(&rt, "sidebar").fill, arctic.chrome);
    // The tab area is the terminals' background; only the gaps between split panes are the
    // border color.
    assert_eq!(element(&rt, "main").fill, rt.state.background());
    assert_eq!(element(&rt, "gap-0").fill, arctic.border);
    assert_eq!(element(&rt, "brand-title").text_color, Some(arctic.muted));
    let colors = rt.state.viewers["t1"]
        .pane
        .as_ref()
        .unwrap()
        .snapshot
        .colors;
    assert_eq!(colors.background, terminal::Rgb::new(0x1e, 0x1e, 0x1e));
    assert_eq!(colors.foreground, terminal::Rgb::new(0xd4, 0xd4, 0xd4));

    // Operator Mono at 17 pt where it's installed; JetBrains Mono, with status saying why, where
    // it isn't.
    assert_eq!(rt.state.font.points, 17.0);
    let status = rt.state.window_status();
    assert_eq!(status["zoom"], 1.1);
    assert_eq!(status["font_size"], 17.0);
    assert_eq!(status["ui_font_size"], 13.0);
    match &rt.state.config.font {
        Some(family) => {
            assert_eq!(rt.state.font.family(), family.private);
            assert_eq!(status["errors"], serde_json::json!([]));
        }
        None => {
            assert_eq!(rt.state.font.family(), "JetBrains Mono");
            assert_eq!(
                status["errors"][0],
                "font \"Operator Mono\" isn't installed; terminals use JetBrains Mono"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_font_that_isnt_installed_falls_back_to_jetbrains_mono_and_says_so() {
    let home = scratch("missing-font");
    std::fs::write(home.join("settings.json"), r#"{"font": "No Such Mono"}"#).unwrap();
    let rt = runtime_at(&home);
    assert_eq!(rt.state.font.family(), "JetBrains Mono");
    assert_eq!(
        rt.state.window_status()["errors"][0],
        "font \"No Such Mono\" isn't installed; terminals use JetBrains Mono"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_zoom_keys_zoom_everything_and_save_it_as_state() {
    let dir = scratch("zoom");
    let (config, state) = (dir.join("config"), dir.join("state"));
    std::fs::create_dir_all(&config).unwrap();
    let settings = r#"{"zoom": 1.1, "font_size": 17, "theme": "arctic"}"#;
    std::fs::write(config.join("settings.json"), settings).unwrap();
    let mut rt = runtime_in(Home::split(&config, &state));
    let cell_before = rt.state.display().cell;

    rt.state.step(1.0, Action::App(AppCommand::ZoomIn));
    frames(&mut rt, 3);
    assert_eq!(rt.zoom(), 1.2);
    let root = element(&rt, "root");
    assert!(
        (root.rect[2] - SIZE.0 / 1.2).abs() < 0.01,
        "{:?}",
        root.rect
    );
    // The terminals' cell follows the zoom (in whole device pixels), and the core hears it.
    let cell_after = rt.state.display().cell;
    assert!(cell_after.unwrap()[1] > cell_before.unwrap()[1]);

    for (command, zoom) in [
        (AppCommand::ZoomOut, 1.1),
        (AppCommand::ZoomOut, 1.0),
        (AppCommand::ZoomIn, 1.1),
        (AppCommand::ZoomIn, 1.2),
        (AppCommand::ZoomReset, 1.0),
        (AppCommand::ZoomIn, 1.1),
        (AppCommand::ZoomIn, 1.2),
        (AppCommand::ZoomIn, 1.3),
    ] {
        rt.state.step(2.0, Action::App(command));
        frames(&mut rt, 2);
        assert_eq!(rt.zoom(), zoom, "{command:?}");
    }
    // Saved in the state folder, over the settings' zoom it stepped from.
    until(&mut rt, "zoom.json to be written", |_| {
        std::fs::read_to_string(state.join("zoom.json")).is_ok_and(|t| t.contains("1.3"))
    });
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(state.join("zoom.json")).unwrap()).unwrap();
    assert_eq!(saved, serde_json::json!({"zoom": 1.3, "settings": 1.1}));
    // The config folder is never written: settings.json is as it was, and nothing is added.
    std::thread::sleep(Duration::from_millis(1500));
    frames(&mut rt, 3);
    assert_eq!(rt.zoom(), 1.3);
    assert_eq!(
        std::fs::read_to_string(config.join("settings.json")).unwrap(),
        settings
    );
    let mut files: Vec<String> = std::fs::read_dir(&config)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        ["keys.json", "settings.json"],
        "keys.json: the defaults, made once"
    );

    // The next run opens at the saved zoom.
    drop(rt);
    let mut rt = runtime_in(Home::split(&config, &state));
    assert_eq!(rt.zoom(), 1.3);

    // An edited settings.json zoom is the zoom again, and the next run starts there.
    std::fs::write(config.join("settings.json"), r#"{"zoom": 0.9}"#).unwrap();
    until(&mut rt, "the new settings zoom", |rt| rt.zoom() == 0.9);
    drop(rt);
    let mut rt = runtime_in(Home::split(&config, &state));
    assert_eq!(rt.zoom(), 0.9);

    // Edited between runs too: a zoom set over 0.9, then settings.json says 1.1.
    rt.state.step(3.0, Action::App(AppCommand::ZoomIn));
    assert_eq!(rt.state.zoom(), 1.0);
    let saved = |zoom: f64, settings: f64| {
        let want = serde_json::json!({"zoom": zoom, "settings": settings});
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let text = std::fs::read_to_string(state.join("zoom.json")).unwrap_or_default();
            if serde_json::from_str::<serde_json::Value>(&text).is_ok_and(|v| v == want) {
                return;
            }
            assert!(Instant::now() < deadline, "zoom.json isn't {want}: {text}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    saved(1.0, 0.9);
    drop(rt);
    std::fs::write(config.join("settings.json"), r#"{"zoom": 1.1}"#).unwrap();
    let rt = runtime_in(Home::split(&config, &state));
    assert_eq!(rt.zoom(), 1.1);
    // The stale zoom is gone, so going back to 0.9 doesn't bring its 1.0 back.
    saved(1.1, 1.1);
    drop(rt);
    std::fs::write(config.join("settings.json"), r#"{"zoom": 0.9}"#).unwrap();
    let rt = runtime_in(Home::split(&config, &state));
    assert_eq!(rt.zoom(), 0.9);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_and_theme_files_reload_live() {
    let home = scratch("reload");
    let mut rt = runtime_at(&home);
    assert_eq!(rt.state.font.points, 17.0);

    std::fs::write(
        home.join("settings.json"),
        r#"{"font_size": 20, "zoom": 1.0, "terminal_line_height": 1.2, "minimum_contrast": 4.5}"#,
    )
    .unwrap();
    until(&mut rt, "the new settings", |rt| {
        rt.state.config.settings.font_size == 20.0
    });
    assert_eq!(rt.zoom(), 1.0);
    assert_eq!(rt.state.font.points, 20.0);
    assert_eq!(rt.state.look.minimum_contrast, 4.5);
    let face = rt.state.face(2.0);
    let plain = herdr_frontend::view::terminal::font::Font::jetbrains_mono(20.0)
        .at(2.0)
        .unwrap();
    assert!(
        face.metrics.cell_height > plain.metrics.cell_height,
        "taller rows"
    );

    // A theme of the home's own, and an override from the settings.
    std::fs::create_dir_all(home.join("themes")).unwrap();
    std::fs::write(
        home.join("themes/mine.json"),
        herdr_frontend::config::theme::ARCTIC_JSON.replace("\"#1e1e1e\"", "\"#101010\""),
    )
    .unwrap();
    std::fs::write(
        home.join("settings.json"),
        r##"{"theme": "mine", "colors": {"chrome": "#202020"}}"##,
    )
    .unwrap();
    until(&mut rt, "the new theme", |rt| {
        rt.state.config.theme.name == "mine"
    });
    let chrome = rt.state.config.theme.chrome();
    assert_eq!(element(&rt, "root").fill, chrome.canvas);
    assert_eq!(element(&rt, "sidebar").fill.hex(), "#202020ff");
    until(&mut rt, "the terminals' new background", |rt| {
        rt.state.viewers["t1"]
            .pane
            .as_ref()
            .is_some_and(|p| p.snapshot.colors.background == terminal::Rgb::new(0x10, 0x10, 0x10))
    });

    // A broken file keeps what the window has, and says so.
    std::fs::write(home.join("settings.json"), "{ broken").unwrap();
    until(&mut rt, "the broken file", |rt| {
        rt.state
            .config
            .problems
            .iter()
            .any(|p| p.starts_with("settings.json:"))
    });
    assert_eq!(rt.state.config.theme.name, "mine");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn tabs_show_their_programs_icon() {
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.connect(Box::new(demo_core()));
    rt.state.settle();
    frames(&mut rt, 3);
    let icons: Vec<ElementInfo> = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref() == Some("icon") && e.path.contains("tab-"))
        .collect();
    assert_eq!(icons.len(), 2, "one per tab");
    assert!(near(icons[0].rect[2], 14.0) && near(icons[0].rect[3], 14.0));
}
