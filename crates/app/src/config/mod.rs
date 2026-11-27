//! The look and preferences the window reads from files, and reads again when they change
//! (`docs/settings.md`):
//!
//! - [`settings`]: `settings.json` in herdr-frontend's home, defaulting to Eli's values.
//! - [`theme`]: the theme it names, arctic built in or the home's `themes/<name>.json`.
//! - [`programs`]: program names, icons and colors: the built-in table, then the home's
//!   `programs.json` and icon files.
//! - [`fonts`]: the terminal and panel font families, found in the home's `fonts/` and then
//!   Herdr app's (read only).
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

use self::fonts::FamilyFiles;
use self::programs::Programs;
use self::settings::Settings;
use self::theme::Theme;

pub const SETTINGS: &str = "settings.json";
pub const PROGRAMS: &str = "programs.json";
pub const THEMES: &str = "themes";
pub const FONTS: &str = "fonts";
/// The home's icon folders, as `programs.json` names their files (`icons/pi.svg`).
pub const ICON_DIRS: [&str; 2] = ["icons", "program-icons"];

/// How often the watcher looks at the files.
pub const WATCH_EVERY: Duration = Duration::from_millis(500);

/// Where the files are: herdr-frontend's home, and Herdr app's, whose fonts it also uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub home: Option<PathBuf>,
    /// Herdr app's home (`$HERDR_APP_HOME`, else `$XDG_CONFIG_HOME/herdr-app`, else
    /// `~/.config/herdr-app`): read only, never written. Its `fonts/`, and on a home's first run
    /// its `shared.json`, its layout's sidebar state and the `eli-agents` store
    /// ([`crate::shared`]).
    pub herdr_app: Option<PathBuf>,
}

impl Sources {
    /// herdr-frontend's home and the Herdr app home the environment names.
    pub fn new(home: Option<PathBuf>) -> Sources {
        Sources {
            home,
            herdr_app: Some(herdr_app_home()),
        }
    }

    /// No files at all: the built-in defaults (scenes and tests).
    pub fn none() -> Sources {
        Sources::default()
    }

    /// The folders fonts are looked for in, the home's first.
    pub fn font_dirs(&self) -> Vec<PathBuf> {
        [&self.home, &self.herdr_app]
            .into_iter()
            .flatten()
            .map(|dir| dir.join(FONTS))
            .collect()
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
        for dir in self.font_dirs() {
            paths.push(dir.clone());
            paths.extend(fonts::files(&[dir]));
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

/// Herdr app's home, as herdr-app finds it (`app:src/mods.rs` `app_dir`).
pub fn herdr_app_home() -> PathBuf {
    let var = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    var("HERDR_APP_HOME").unwrap_or_else(|| {
        var("XDG_CONFIG_HOME")
            .unwrap_or_else(|| var("HOME").unwrap_or_default().join(".config"))
            .join("herdr-app")
    })
}

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
    pub programs: Programs,
    /// The families `font` and `ui_font` name, when the font folders have them.
    pub font: Option<FamilyFiles>,
    pub ui_font: Option<FamilyFiles>,
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

        let theme = load_theme(home, &settings, &mut problems);

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

        let dirs = sources.font_dirs();
        let font = fonts::find(&settings.font, &dirs);
        if font.is_none() && !is_bundled(&settings.font) {
            problems.push(format!(
                "font {:?} isn't in {}; terminals use JetBrains Mono",
                settings.font,
                folders(&dirs)
            ));
        }
        let ui_font = fonts::find(&settings.ui_font, &dirs);
        if ui_font.is_none() && !is_bundled(&settings.ui_font) {
            problems.push(format!(
                "ui_font {:?} isn't in {}; the panels use the system font",
                settings.ui_font,
                folders(&dirs)
            ));
        }

        Config {
            settings,
            settings_text,
            theme,
            programs,
            font,
            ui_font,
            problems,
        }
    }
}

/// The bundled family, which needs no file.
fn is_bundled(family: &str) -> bool {
    family
        .trim()
        .eq_ignore_ascii_case(crate::view::terminal::font::JETBRAINS_MONO_FAMILY)
}

fn folders(dirs: &[PathBuf]) -> String {
    if dirs.is_empty() {
        return "any font folder".into();
    }
    dirs.iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(" or ")
}

/// The theme `settings` names: the home's file first (so a home can change arctic too), then
/// the built-in arctic; a theme that can't be had falls back to arctic, with the problem said.
/// The settings' `colors` go over it, each bad one skipped and said.
fn load_theme(home: Option<&Path>, settings: &Settings, problems: &mut Vec<String>) -> Theme {
    let name = settings.theme.trim();
    let file = home
        .and_then(|h| std::fs::read_to_string(h.join(THEMES).join(format!("{name}.json"))).ok());
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
    theme
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
        Sources {
            home: Some(home.to_owned()),
            herdr_app: Some(home.join("herdr-app")),
        }
    }

    #[test]
    fn an_empty_home_is_elis_look_with_the_missing_font_said() {
        let home = scratch("empty");
        let config = Config::load(&sources(&home), None);
        assert_eq!(config.settings, Settings::default());
        assert_eq!(config.theme, Theme::arctic());
        assert!(config.settings_text.is_none());
        assert!(config.font.is_none());
        assert_eq!(config.problems.len(), 2, "{:?}", config.problems);
        assert!(config.problems[0].starts_with("font \"Operator Mono Book Herdr\" isn't in"));
        let _ = std::fs::remove_dir_all(&home);
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
        // Herdr app's fonts are found too.
        let herdr_fonts = home.join("herdr-app").join(FONTS);
        std::fs::create_dir_all(&herdr_fonts).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/fonts/JetBrainsMono-Variable.ttf"),
            herdr_fonts.join("JetBrainsMono-Variable.ttf"),
        )
        .unwrap();

        let config = Config::load(&sources(&home), None);
        assert!(config.problems.is_empty(), "{:?}", config.problems);
        assert_eq!(config.theme.name, "mine");
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
        assert_eq!(
            config.font.as_ref().map(|f| f.faces[0].path.clone()),
            Some(herdr_fonts.join("JetBrainsMono-Variable.ttf"))
        );
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
