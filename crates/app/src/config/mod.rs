//! The look and preferences the window reads from files, and reads again when they change
//! (`docs/settings.md`):
//!
//! - [`settings`]: `settings.json` in herdr-frontend's home, defaulting to Eli's values.
//! - [`theme`]: the theme it names, arctic built in or the home's `themes/<name>.json`.
//! - [`programs`]: program names, icons and colors: the built-in table, then the home's
//!   `programs.json` and icon files.
//! - [`fonts`]: the terminal and panel font families, found among the installed fonts as Ghostty
//!   finds its `font-family`.
//!
//! [`Config::load`] reads them all; it does file I/O, so the window calls it at start and then
//! only on the [`Watcher`]'s thread, which looks for changes twice a second and hands each new
//! [`Config`] to the UI thread.

pub mod fonts;
pub mod programs;
pub mod settings;
pub mod theme;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime};

use self::fonts::{Family, Slot};
use self::programs::Programs;
use self::settings::Settings;
use self::theme::Theme;

pub const SETTINGS: &str = "settings.json";
pub const PROGRAMS: &str = "programs.json";
pub const THEMES: &str = "themes";
/// The home's icon folders, as `programs.json` names their files (`icons/pi.svg`).
pub const ICON_DIRS: [&str; 2] = ["icons", "program-icons"];

/// How often the watcher looks at the files.
pub const WATCH_EVERY: Duration = Duration::from_millis(500);

/// Where the files are: herdr-frontend's home, the only folder they're read from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub home: Option<PathBuf>,
}

impl Sources {
    /// herdr-frontend's home.
    pub fn new(home: Option<PathBuf>) -> Sources {
        Sources { home }
    }

    /// No files at all: the built-in defaults (scenes and tests).
    pub fn none() -> Sources {
        Sources::default()
    }

    /// What the watcher compares: every file a load reads (or would read, were it there), with
    /// its modification time and length. Every theme file counts, so the stamp doesn't depend on
    /// which theme the settings name. Take it before a load: a file written during the load then
    /// differs from it, and is loaded next time.
    pub fn stamp(&self) -> Stamp {
        let mut paths = Vec::new();
        if let Some(home) = &self.home {
            paths.push(home.join(SETTINGS));
            paths.push(home.join(PROGRAMS));
            paths.push(home.join(THEMES));
            paths.extend(listing(&home.join(THEMES)));
            for dir in ICON_DIRS {
                paths.extend(listing(&home.join(dir)));
            }
        }
        paths
            .into_iter()
            .map(|p| {
                let meta = std::fs::metadata(&p).ok();
                let stamp = meta.and_then(|m| Some((m.modified().ok()?, m.len())));
                (p, stamp)
            })
            .collect()
    }
}

/// [`Sources::stamp`]: each watched path with its modification time and length.
pub type Stamp = Vec<(PathBuf, Option<(SystemTime, u64)>)>;

/// A folder's files, sorted; none when it isn't there.
fn listing(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    files
}

/// Everything the files say, resolved.
#[derive(Clone, Debug)]
pub struct Config {
    pub settings: Settings,
    /// `settings.json` as read, which the zoom keys edit; `None` with no file.
    pub settings_text: Option<String>,
    pub theme: Theme,
    /// The home's theme file the theme came from (`themes/<name>.json`, home-relative), when it
    /// came from one rather than the built-in arctic.
    pub theme_file: Option<String>,
    pub programs: Programs,
    /// The families `font` and `ui_font` name, when they're installed.
    pub font: Option<Family>,
    pub ui_font: Option<Family>,
    /// What's wrong with the files, for `status`'s `errors`.
    pub problems: Vec<String>,
}

impl Default for Config {
    /// The built-in defaults: Eli's settings, arctic, the built-in programs, no font files.
    fn default() -> Config {
        Config::from_settings(Settings::default())
    }
}

impl Config {
    /// `settings` with the built-in theme and programs and no font files.
    pub fn from_settings(settings: Settings) -> Config {
        let mut theme = Theme::arctic();
        let problems = theme.apply(&settings.colors);
        Config {
            settings,
            settings_text: None,
            theme,
            theme_file: None,
            programs: Programs::default(),
            font: None,
            ui_font: None,
            problems,
        }
    }

    /// Reads every file. `previous` is what the window has now: a settings file that can't be
    /// read keeps its settings.
    pub fn load(sources: &Sources, previous: Option<&Settings>) -> Config {
        let mut problems = Vec::new();
        let home = sources.home.as_deref();
        let read = |name: &str| home.and_then(|h| std::fs::read_to_string(h.join(name)).ok());

        let settings_text = read(SETTINGS);
        let settings = match settings_text.as_deref().map(Settings::parse) {
            None => Settings::default(),
            Some(Ok((settings, found))) => {
                problems.extend(found);
                settings
            }
            Some(Err(e)) => {
                problems.push(e);
                previous.cloned().unwrap_or_default()
            }
        };

        let (theme, theme_file) = load_theme(home, &settings, &mut problems);

        let programs_text = read(PROGRAMS);
        // The home's icons by home-relative path: every file in its icon folders (so one named
        // like a bundled icon replaces it), and any other path its own table names.
        let mut icons = BTreeMap::new();
        if let Some(home) = home {
            let mut paths: Vec<String> = ICON_DIRS
                .iter()
                .flat_map(|dir| {
                    listing(&home.join(dir)).into_iter().filter_map(move |p| {
                        let name = p.file_name()?.to_str()?.to_owned();
                        Some(format!("{dir}/{name}"))
                    })
                })
                .collect();
            paths.extend(Programs::icon_paths(programs_text.as_deref()));
            for path in paths {
                if let Ok(bytes) = std::fs::read(home.join(&path)) {
                    icons.insert(path, Arc::from(bytes));
                }
            }
        }
        let programs = match Programs::new(programs_text.as_deref(), icons) {
            Ok(programs) => programs,
            Err((programs, e)) => {
                problems.push(e);
                programs
            }
        };

        let mut config = Config {
            settings,
            settings_text,
            theme,
            theme_file,
            programs,
            font: None,
            ui_font: None,
            problems,
        };
        config.find_fonts();
        config
    }

    /// `settings` with the built-in theme and programs, and its fonts looked up among the
    /// installed ones as [`Config::load`] does (the Ghostty pixel diff's screens).
    pub fn with_fonts(settings: Settings) -> Config {
        let mut config = Config::from_settings(settings);
        config.find_fonts();
        config
    }

    /// Looks up the families `font` and `ui_font` name, saying in [`Config::problems`] what
    /// can't be had. The panels' Operator Mono is its Medium and Bold ([`settings::PANEL_STYLES`];
    /// Book and Bold where Medium isn't installed), by style name since, found by traits, its
    /// Bold would be taken for its regular face. Another panel family takes the terminal's faces
    /// when it's the same one, and is found by traits otherwise.
    fn find_fonts(&mut self) {
        let s = &self.settings;
        let found = fonts::find(&s.font, s.font_styles());
        say_missing(
            "font",
            &s.font,
            &found,
            "terminals use JetBrains Mono",
            &mut self.problems,
        );
        let named = |family: &str| s.ui_font.trim().eq_ignore_ascii_case(family.trim());
        let same = named(&s.font);
        let ui = if named(settings::FONT) {
            let medium = fonts::find(&s.ui_font, settings::PANEL_STYLES);
            if medium.family.is_some() {
                medium
            } else {
                fonts::find(&s.ui_font, Settings::default().font_styles())
            }
        } else if same {
            found.clone()
        } else {
            fonts::find(&s.ui_font, [None; 4])
        };
        if !same {
            say_missing(
                "ui_font",
                &s.ui_font,
                &ui,
                "the panels use the system font",
                &mut self.problems,
            );
        }
        self.font = found.family;
        self.ui_font = ui.family;
    }
}

/// What `key`'s family `name` lacks: the whole family, which `fallback` replaces, or a face a
/// style name asks for.
fn say_missing(
    key: &str,
    name: &str,
    found: &fonts::Found,
    fallback: &str,
    problems: &mut Vec<String>,
) {
    if found.family.is_none() {
        if !is_bundled(name) {
            problems.push(format!("{key} {name:?} isn't installed; {fallback}"));
        }
        return;
    }
    for slot in &found.unmatched {
        let instead = match slot {
            Slot::Regular => "the family's regular face is used",
            Slot::Bold => "bold text draws the regular face",
            Slot::Italic => "italic text leans the regular face",
            Slot::BoldItalic => "bold italic text draws the italic or regular face",
        };
        problems.push(format!(
            "{} has no face of {name:?} with that style name; {instead}",
            slot.key()
        ));
    }
}

/// The bundled family, which needs no file.
fn is_bundled(family: &str) -> bool {
    family
        .trim()
        .eq_ignore_ascii_case(crate::view::terminal::font::JETBRAINS_MONO_FAMILY)
}

/// The theme `settings` names: the home's file first (so a home can change arctic too), then
/// the built-in arctic; a theme that can't be had falls back to arctic, with the problem said.
/// The settings' `colors` go over it, each bad one skipped and said. Also the home-relative
/// path of the home's file it came from, if it did.
fn load_theme(
    home: Option<&Path>,
    settings: &Settings,
    problems: &mut Vec<String>,
) -> (Theme, Option<String>) {
    let name = settings.theme.trim();
    let path = format!("{THEMES}/{name}.json");
    let file = home.and_then(|h| std::fs::read_to_string(h.join(&path)).ok());
    let from_file = file.is_some().then_some(path);
    let loaded = match (file, name == theme::ARCTIC) {
        (Some(text), _) => Theme::from_file(name, &text),
        (None, true) => Theme::from_file(name, theme::ARCTIC_JSON),
        (None, false) => Err(format!(
            "theme {name:?} is not arctic and there is no themes/{name}.json; using arctic"
        )),
    };
    let mut theme = loaded.unwrap_or_else(|e| {
        problems.push(e);
        Theme::arctic()
    });
    problems.extend(theme.apply(&settings.colors));
    (theme, from_file)
}

/// Looks for changed files on its own thread and loads them there.
pub struct Watcher {
    configs: Receiver<Config>,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    /// Watches `sources` from `seen`, the stamp taken just before `settings` were loaded, so a
    /// file written since is loaded on the first look. `wake` wakes the UI thread when a new
    /// [`Config`] is ready.
    pub fn start(
        sources: Sources,
        seen: Stamp,
        settings: Settings,
        wake: impl Fn() + Send + 'static,
    ) -> Watcher {
        let (tx, configs) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let mut seen = seen;
        let mut settings = settings;
        std::thread::Builder::new()
            .name("herdr-frontend-config".into())
            .spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    std::thread::sleep(WATCH_EVERY);
                    let Some(config) = look(&sources, &mut seen, &mut settings, Config::load)
                    else {
                        continue;
                    };
                    if tx.send(config).is_err() {
                        break;
                    }
                    wake();
                }
            })
            .expect("spawning the config watcher");
        Watcher { configs, stop }
    }

    /// The newest config loaded since the last call.
    pub fn latest(&self) -> Option<Config> {
        self.configs.try_iter().last()
    }
}

/// One look at the files: when they changed since `seen`, loads them with `load`. `seen`
/// becomes the stamp taken before the load, so whatever is written during it differs from it.
fn look(
    sources: &Sources,
    seen: &mut Stamp,
    settings: &mut Settings,
    load: impl FnOnce(&Sources, Option<&Settings>) -> Config,
) -> Option<Config> {
    let now = sources.stamp();
    if now == *seen {
        return None;
    }
    *seen = now;
    let config = load(sources, Some(settings));
    *settings = config.settings.clone();
    Some(config)
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-frontend-config-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sources(home: &Path) -> Sources {
        Sources::new(Some(home.to_owned()))
    }

    #[test]
    fn an_empty_home_is_elis_look() {
        let home = scratch("empty");
        let config = Config::load(&sources(&home), None);
        assert_eq!(config.settings, Settings::default());
        assert_eq!(config.theme, Theme::arctic());
        assert_eq!(config.theme_file, None, "the built-in arctic");
        assert!(config.settings_text.is_none());
        // Operator Mono, if this Mac has it; status says so if it doesn't.
        match &config.font {
            Some(font) => {
                assert_eq!(font.name, "Operator Mono");
                let ui = config.ui_font.as_ref().expect("the panels' Operator Mono");
                assert_eq!(ui.name, "Operator Mono");
                assert_ne!(
                    ui.private, font.private,
                    "the panels have faces of their own"
                );
                assert!(config.problems.is_empty(), "{:?}", config.problems);
            }
            None => assert_eq!(
                config.problems,
                ["font \"Operator Mono\" isn't installed; terminals use JetBrains Mono"]
            ),
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_family_that_isnt_installed_is_said_and_jetbrains_mono_needs_none() {
        let home = scratch("missing");
        std::fs::write(
            home.join(SETTINGS),
            r#"{"font": "No Such Mono", "ui_font": "No Such Sans"}"#,
        )
        .unwrap();
        let config = Config::load(&sources(&home), None);
        assert!(config.font.is_none() && config.ui_font.is_none());
        assert_eq!(
            config.problems,
            [
                "font \"No Such Mono\" isn't installed; terminals use JetBrains Mono",
                "ui_font \"No Such Sans\" isn't installed; the panels use the system font"
            ]
        );
        // The bundled JetBrains Mono needs no install.
        std::fs::write(
            home.join(SETTINGS),
            r#"{"font": "JetBrains Mono", "ui_font": "JetBrains Mono"}"#,
        )
        .unwrap();
        let config = Config::load(&sources(&home), None);
        assert!(config.problems.is_empty(), "{:?}", config.problems);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The panels' Operator Mono is Medium where the terminals' is Book, beside it or beside
    /// another terminal family, found by style name: by traits, its Bold would be taken for its
    /// regular face.
    #[cfg(target_os = "macos")]
    #[test]
    fn operator_mono_for_the_panels_is_medium() {
        let menlo = Settings {
            font: "Menlo".into(),
            ..Settings::fixture()
        };
        for terminals in [Settings::default(), menlo] {
            let font = terminals.font.clone();
            let config = Config::with_fonts(Settings {
                ui_font: settings::FONT.into(),
                ..terminals
            });
            let Some(ui) = config.ui_font else {
                eprintln!("skipping: Operator Mono isn't installed");
                return;
            };
            let names: Vec<&str> = fonts::Slot::ALL
                .iter()
                .map(|s| ui.face(*s).map_or("", |f| f.postscript.as_str()))
                .collect();
            assert_eq!(
                names,
                [
                    "OperatorMono-Medium",
                    "OperatorMono-Bold",
                    "OperatorMono-MediumItalic",
                    "OperatorMono-BoldItalic"
                ],
                "beside {font}"
            );
            assert!(config.problems.is_empty(), "{:?}", config.problems);
        }
        // The terminals keep Book.
        let config = Config::with_fonts(Settings::default());
        let font = config.font.expect("Operator Mono, as the panels found it");
        assert_eq!(font.regular().postscript, "OperatorMono-Book");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_style_name_the_family_lacks_is_said() {
        let config = Config::with_fonts(Settings {
            font: "Menlo".into(),
            font_style_bold: Some("Heavy".into()),
            ui_font: "Menlo".into(),
            ..Settings::fixture()
        });
        assert!(config.font.is_some());
        assert_eq!(
            config.problems,
            [
                "font_style_bold has no face of \"Menlo\" with that style name; bold text draws \
                 the regular face"
            ]
        );
    }

    #[test]
    fn the_homes_files_are_read() {
        let home = scratch("files");
        std::fs::write(
            home.join(SETTINGS),
            r##"{"font": "JetBrains Mono", "ui_font": "JetBrains Mono", "theme": "mine",
                "colors": {"accent": "#123456"}}"##,
        )
        .unwrap();
        std::fs::create_dir_all(home.join(THEMES)).unwrap();
        std::fs::write(
            home.join(THEMES).join("mine.json"),
            theme::ARCTIC_JSON.replace("#1e1e1e", "#101010"),
        )
        .unwrap();
        std::fs::write(
            home.join(PROGRAMS),
            r#"{"programs": {"mine": {"icon": "program-icons/mine.svg"}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.join("program-icons")).unwrap();
        std::fs::write(home.join("program-icons/mine.svg"), "<svg/>").unwrap();
        let config = Config::load(&sources(&home), None);
        assert!(config.problems.is_empty(), "{:?}", config.problems);
        assert_eq!(config.theme.name, "mine");
        assert_eq!(config.theme_file.as_deref(), Some("themes/mine.json"));
        assert_eq!(config.theme.get("canvas").unwrap().hex(), "#101010");
        assert_eq!(config.theme.get("accent").unwrap().hex(), "#123456");
        assert!(config.programs.icon("program-icons/mine.svg").is_some());
        // An icon named like a bundled one replaces it, with no programs.json entry.
        let neovim = config.programs.icon("program-icons/neovim.svg").unwrap();
        assert_ne!(&neovim[..], b"<svg/>");
        std::fs::write(home.join("program-icons/neovim.svg"), "<svg/>").unwrap();
        let config = Config::load(&sources(&home), None);
        assert_eq!(
            config
                .programs
                .icon("program-icons/neovim.svg")
                .map(|b| &b[..]),
            Some(&b"<svg/>"[..])
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_homes_arctic_file_replaces_the_built_in_one_and_is_said() {
        let home = scratch("arctic");
        std::fs::create_dir_all(home.join(THEMES)).unwrap();
        std::fs::write(
            home.join(THEMES).join("arctic.json"),
            theme::ARCTIC_JSON.replace("#1e1e1e", "#101010"),
        )
        .unwrap();
        let config = Config::load(&sources(&home), None);
        assert_eq!(config.theme.get("canvas").unwrap().hex(), "#101010");
        assert_eq!(config.theme_file.as_deref(), Some("themes/arctic.json"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn broken_files_keep_what_was_there_and_say_why() {
        let home = scratch("broken");
        std::fs::write(home.join(SETTINGS), "{ nope").unwrap();
        std::fs::write(home.join(PROGRAMS), r#"{"programs": 3}"#).unwrap();
        let before = Settings {
            zoom: 1.5,
            ..Settings::default()
        };
        let config = Config::load(&sources(&home), Some(&before));
        assert_eq!(config.settings, before);
        assert!(
            config
                .problems
                .iter()
                .any(|p| p.starts_with("settings.json:"))
        );
        assert!(
            config
                .problems
                .iter()
                .any(|p| p.starts_with("programs.json:"))
        );

        std::fs::write(home.join(SETTINGS), r#"{"theme": "nothing"}"#).unwrap();
        let config = Config::load(&sources(&home), None);
        assert_eq!(config.theme, Theme::arctic());
        assert!(
            config
                .problems
                .iter()
                .any(|p| p.contains("no themes/nothing.json"))
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_file_written_during_a_load_is_loaded_next_time() {
        let home = scratch("midload");
        let sources = sources(&home);
        let mut seen = sources.stamp();
        let mut settings = Settings::default();
        // `dot stow`: settings.json names a theme, and the theme file lands while that load runs.
        std::fs::write(home.join(SETTINGS), r#"{"theme": "mine"}"#).unwrap();
        let first = look(&sources, &mut seen, &mut settings, |sources, previous| {
            let config = Config::load(sources, previous);
            std::fs::create_dir_all(home.join(THEMES)).unwrap();
            std::fs::write(
                home.join(THEMES).join("mine.json"),
                theme::ARCTIC_JSON.replace("#1e1e1e", "#101010"),
            )
            .unwrap();
            config
        })
        .expect("settings.json changed");
        assert_eq!(
            first.theme.name,
            theme::ARCTIC,
            "the theme wasn't there yet"
        );
        let next = look(&sources, &mut seen, &mut settings, Config::load)
            .expect("the theme file was written after the stamp");
        assert_eq!(next.theme.name, "mine");
        assert_eq!(next.theme.get("canvas").unwrap().hex(), "#101010");
        assert!(look(&sources, &mut seen, &mut settings, Config::load).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_watcher_loads_a_changed_file() {
        let home = scratch("watch");
        let sources = sources(&home);
        let seen = sources.stamp();
        let first = Config::load(&sources, None);
        let watcher = Watcher::start(sources, seen, first.settings, || {});
        std::fs::write(home.join(SETTINGS), r#"{"zoom": 1.5}"#).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let config = loop {
            if let Some(config) = watcher.latest() {
                break config;
            }
            assert!(std::time::Instant::now() < deadline, "never reloaded");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(config.settings.zoom, 1.5);
        assert_eq!(config.settings_text.as_deref(), Some(r#"{"zoom": 1.5}"#));
        let _ = std::fs::remove_dir_all(&home);
    }
}
