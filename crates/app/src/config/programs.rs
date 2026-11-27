// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! What runs in a terminal, as the panels show it: a name, an icon and colors (herdr-app's
//! `programs.json`, `app:src/programs.rs`).
//!
//! The table is herdr-app's built-in one (`assets/programs/herdr.json`, with its icons in
//! `assets/icons/`), then Eli's `programs.json` (`assets/programs/eli.json`: Pi in purple, Neovim
//! with its full-color logo), then the home's `programs.json`, each entry by entry. An icon path
//! is relative to the home (`icons/pi.svg`, `program-icons/neovim.svg`): a file there wins, else
//! the bundled one by the same path.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;

const HERDR: &str = include_str!("../../../../assets/programs/herdr.json");
const ELI: &str = include_str!("../../../../assets/programs/eli.json");

// `ICONS`: every bundled icon as (`icons/<file>` or `program-icons/<file>`, bytes), by build.rs.
include!(concat!(env!("OUT_DIR"), "/icons.rs"));

#[derive(Clone, Debug, Default, Deserialize)]
struct Table {
    #[serde(default)]
    shells: Vec<String>,
    #[serde(default)]
    interpreters: Vec<String>,
    #[serde(default)]
    programs: BTreeMap<String, Entry>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Entry {
    #[serde(default)]
    name: Option<String>,
    /// A path relative to the home.
    #[serde(default)]
    icon: Option<String>,
    /// A one-color glyph, drawn in the program's color; else a picture in its own colors.
    #[serde(default)]
    tint: Option<bool>,
    #[serde(default)]
    tile: Option<String>,
    #[serde(default)]
    color: Option<String>,
    /// Other process names (or agents' OSC 7501 app names) for this entry.
    #[serde(default, rename = "match")]
    also: Vec<String>,
}

impl Entry {
    fn over(&mut self, mine: Entry) {
        self.name = mine.name.or(self.name.take());
        self.icon = mine.icon.or(self.icon.take());
        self.tint = mine.tint.or(self.tint);
        self.tile = mine.tile.or(self.tile.take());
        self.color = mine.color.or(self.color.take());
        if !mine.also.is_empty() {
            self.also = mine.also;
        }
    }
}

impl Table {
    fn over(&mut self, mine: Table) {
        self.shells.extend(mine.shells);
        self.interpreters.extend(mine.interpreters);
        for (key, entry) in mine.programs {
            self.programs.entry(key).or_default().over(entry);
        }
    }
}

/// How one terminal shows (herdr-app's `program`: `{ key, name, icon, tint, tile, color, shell,
/// agent }`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// The table's entry: `pi`, `nvim`, `shell`, `program`, …
    pub key: String,
    pub name: String,
    /// The icon's path relative to the home.
    pub icon: Option<String>,
    /// A one-color glyph drawn in [`Program::color`] (or the caller's); false: a picture.
    pub tint: bool,
    pub tile: Option<String>,
    pub color: Option<String>,
    pub shell: bool,
    /// Named by the agent's own status report, not by the process.
    pub agent: bool,
}

/// The table, and the icon files it can draw.
#[derive(Clone, Debug)]
pub struct Programs {
    table: Table,
    /// Icons by home-relative path: the home's own files over the bundled ones.
    icons: BTreeMap<String, Arc<[u8]>>,
}

impl Default for Programs {
    fn default() -> Programs {
        Programs::new(None, BTreeMap::new()).expect("the built-in tables parse")
    }
}

impl Programs {
    /// The built-in table with the home's `programs.json` text on top, and the home's icon
    /// files (by home-relative path) over the bundled ones. `Err` says what is wrong with the
    /// home's file; the built-in table is used then.
    pub fn new(
        mine: Option<&str>,
        home_icons: BTreeMap<String, Arc<[u8]>>,
    ) -> Result<Programs, (Programs, String)> {
        let mut table: Table = serde_json::from_str(HERDR).expect("assets/programs/herdr.json");
        table.over(serde_json::from_str(ELI).expect("assets/programs/eli.json"));
        let mut icons: BTreeMap<String, Arc<[u8]>> = ICONS
            .iter()
            .map(|(path, bytes)| ((*path).to_owned(), Arc::from(*bytes)))
            .collect();
        icons.extend(home_icons);
        let error = match mine.map(serde_json::from_str::<Table>) {
            None => None,
            Some(Ok(mine)) => {
                table.over(mine);
                None
            }
            Some(Err(e)) => Some(format!("programs.json: {e}")),
        };
        let programs = Programs { table, icons };
        match error {
            None => Ok(programs),
            Some(e) => Err((programs, e)),
        }
    }

    /// The home-relative icon paths the table names that aren't bundled: the files the home
    /// must bring.
    pub fn icon_paths(mine: Option<&str>) -> Vec<String> {
        let mine: Table = mine
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default();
        mine.programs
            .values()
            .filter_map(|e| e.icon.clone())
            .collect()
    }

    /// An icon's file, by its home-relative path.
    pub fn icon(&self, path: &str) -> Option<&Arc<[u8]>> {
        self.icons.get(path)
    }

    /// How a terminal shows: from the agent's own report (`app`, its OSC 7501 name), else the
    /// program it runs (`argv`).
    pub fn of(&self, app: Option<&str>, argv: &[String]) -> Program {
        resolve(&self.table, app, argv)
    }

    /// A terminal record's program: its status's `app`, else its foreground process's argv, else
    /// the argv it was started with (herdr-app's `program_of` and `running_argv`).
    pub fn of_terminal(&self, record: &herdr_proto::Terminal) -> Program {
        let app = record.status.as_ref().map(|s| s.app.as_str());
        let argv = record
            .foreground
            .as_ref()
            .map(|f| &f.argv)
            .filter(|a| !a.is_empty())
            .unwrap_or(&record.argv);
        self.of(app, argv)
    }
}

/// A command's name: its path's last part, without a login shell's `-` or `.exe`.
fn base(s: &str) -> &str {
    let s = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let s = s.trim_start_matches('-');
    match s.len().checked_sub(4) {
        Some(i) if s.is_char_boundary(i) && s[i..].eq_ignore_ascii_case(".exe") => &s[..i],
        _ => s,
    }
}

fn find<'a>(t: &'a Table, name: &str) -> Option<(&'a str, &'a Entry)> {
    let lower = name.to_lowercase();
    t.programs
        .iter()
        .find(|(k, _)| k.as_str() == name || k.to_lowercase() == lower)
        .or_else(|| {
            t.programs.iter().find(|(_, e)| {
                e.also
                    .iter()
                    .any(|a| a == name || a.to_lowercase() == lower)
            })
        })
        .map(|(k, e)| (k.as_str(), e))
}

fn resolve(t: &Table, app: Option<&str>, argv: &[String]) -> Program {
    let entry = |key: &str| t.programs.get(key).cloned().unwrap_or_default();
    if let Some(app) = app.filter(|a| !a.is_empty()) {
        return match find(t, app) {
            Some((key, e)) => shown(key, e, None, false, true),
            None => shown("agent", &entry("agent"), Some(app), false, true),
        };
    }
    // Herdr app starts commands in the person's shell: `zsh -ic '"$@"; exec "$0" -i' zsh <cmd>…`,
    // or fish's `command $argv[2..]; exec $argv[1] -i`; on macOS as login shells (`-lic`, `-li`).
    if argv.len() > 4
        && t.shells.iter().any(|s| s == base(&argv[0]))
        && argv[1].starts_with('-')
        && argv[1].ends_with('c')
        && (argv[2].contains("\"$@\"")
            || matches!(
                argv[2].as_str(),
                "command $argv[2..]; exec $argv[1] -i" | "command $argv[2..]; exec $argv[1] -li"
            ))
    {
        return resolve(t, None, &argv[4..]);
    }
    let Some(first) = argv.first().map(|a| base(a)) else {
        return shown("shell", &entry("shell"), None, true, false);
    };
    // `node …/vite.js dev`, `npx vite`, `uv run pytest`: the program they run, when it's known.
    if t.interpreters.iter().any(|i| i == first) {
        for arg in argv[1..].iter().filter(|a| !a.starts_with('-')) {
            // A script's name without its extension, never a module path (`http.server`).
            let name = base(arg);
            let stem = [".js", ".mjs", ".cjs", ".ts", ".py", ".rb"]
                .iter()
                .find_map(|x| name.strip_suffix(x))
                .unwrap_or(name);
            if let Some((key, e)) =
                find(t, stem).filter(|(k, _)| !t.interpreters.iter().any(|i| i == k))
            {
                return shown(key, e, None, false, false);
            }
        }
    }
    if let Some((key, e)) = find(t, first) {
        return shown(key, e, None, false, false);
    }
    if t.shells.iter().any(|s| s == first) {
        return shown("shell", &entry("shell"), Some(first), true, false);
    }
    let mut e = entry("program");
    e.tile = Some(hash_color(first).into());
    e.color = Some("#ffffff".into());
    shown("program", &e, Some(first), false, false)
}

fn shown(key: &str, e: &Entry, name: Option<&str>, shell: bool, agent: bool) -> Program {
    Program {
        key: key.to_owned(),
        name: name
            .map(str::to_owned)
            .or_else(|| e.name.clone())
            .unwrap_or_else(|| key.to_owned()),
        icon: e.icon.clone(),
        tint: e.tint.unwrap_or(true),
        tile: e.tile.clone(),
        color: e.color.clone(),
        shell,
        agent,
    }
}

/// A color for a program nobody listed, the same every time (herdr-app's `hash_color`).
fn hash_color(s: &str) -> &'static str {
    const PALETTE: [&str; 8] = [
        "#6d5bd0", "#2f80ed", "#0f9d8a", "#d9822b", "#c2417d", "#3a7d44", "#8a5a44", "#4e5d94",
    ];
    let h = s.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    });
    PALETTE[(h % PALETTE.len() as u32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(app: Option<&str>, argv: &[&str]) -> Program {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        Programs::default().of(app, &argv)
    }

    fn key(app: Option<&str>, argv: &[&str]) -> String {
        of(app, argv).key
    }

    #[test]
    fn agents_shells_tools_and_what_interpreters_run() {
        assert_eq!(key(Some("claude-code"), &["/bin/zsh"]), "claude");
        assert_eq!(key(Some("newagent"), &[]), "agent");
        assert_eq!(of(Some("newagent"), &[]).name, "newagent");
        assert_eq!(key(None, &["/bin/zsh"]), "shell");
        assert_eq!(key(None, &["-zsh"]), "shell");
        assert_eq!(key(None, &[]), "shell");
        assert_eq!(key(None, &["nvim", "x.rs"]), "nvim");
        assert_eq!(key(None, &["vi"]), "vim");
        assert_eq!(
            key(None, &["node", "/x/node_modules/.bin/vite", "dev"]),
            "vite"
        );
        assert_eq!(key(None, &["npx", "vitest"]), "vitest");
        assert_eq!(key(None, &["node", "server.js"]), "node");
        assert_eq!(
            key(None, &["python3", "-m", "http.server", "8799"]),
            "python"
        );
        assert_eq!(key(None, &["uv", "run", "pytest"]), "pytest");
        assert_eq!(
            key(
                None,
                &[
                    "/bin/zsh",
                    "-ic",
                    "\"$@\"; exec \"$0\" -i",
                    "/bin/zsh",
                    "vi",
                    "x"
                ]
            ),
            "vim"
        );
        assert_eq!(
            key(
                None,
                &[
                    "/opt/homebrew/bin/fish",
                    "-lic",
                    "command $argv[2..]; exec $argv[1] -li",
                    "fish",
                    "pi"
                ]
            ),
            "pi"
        );
        let unknown = of(None, &["mytool"]);
        assert_eq!(
            (unknown.key.as_str(), unknown.name.as_str()),
            ("program", "mytool")
        );
        assert_eq!(unknown.tile.as_deref(), Some(hash_color("mytool")));
        assert_eq!(unknown.color.as_deref(), Some("#ffffff"));
    }

    #[test]
    fn elis_programs_are_built_in() {
        let pi = of(Some("pi"), &[]);
        assert_eq!(pi.color.as_deref(), Some("#a78bfa"));
        assert_eq!(pi.icon.as_deref(), Some("icons/pi.svg"));
        assert!(pi.tint && pi.agent);
        let nvim = of(None, &["nvim"]);
        assert_eq!(nvim.icon.as_deref(), Some("program-icons/neovim.svg"));
        assert!(!nvim.tint, "Neovim's logo keeps its colors");
        assert_eq!(nvim.name, "Neovim");
    }

    #[test]
    fn every_entry_has_its_icon_bundled() {
        let programs = Programs::default();
        for (k, e) in &programs.table.programs {
            let icon = e
                .icon
                .as_deref()
                .unwrap_or_else(|| panic!("{k} has no icon"));
            assert!(programs.icon(icon).is_some(), "{k}: {icon} isn't bundled");
        }
    }

    #[test]
    fn the_homes_table_and_icons_go_on_top() {
        let mine = r##"{"programs": {"pi": {"color": "#00ff00"}, "mytool": {"icon": "program-icons/mine.svg", "tint": false}}}"##;
        let icons = BTreeMap::from([(
            "program-icons/mine.svg".to_owned(),
            Arc::from(&b"<svg/>"[..]),
        )]);
        let programs = Programs::new(Some(mine), icons).unwrap();
        let argv = |a: &str| vec![a.to_owned()];
        let pi = programs.of(Some("pi"), &[]);
        assert_eq!(pi.color.as_deref(), Some("#00ff00"));
        assert_eq!(pi.icon.as_deref(), Some("icons/pi.svg"), "the rest stays");
        let mine = programs.of(None, &argv("mytool"));
        assert_eq!(mine.key, "mytool");
        assert!(!mine.tint);
        assert_eq!(
            programs.icon("program-icons/mine.svg").map(|b| &b[..]),
            Some(&b"<svg/>"[..])
        );
        assert_eq!(
            Programs::icon_paths(Some(r#"{"programs": {"a": {"icon": "x/a.svg"}}}"#)),
            ["x/a.svg"]
        );

        let (fallback, error) = Programs::new(Some("{"), BTreeMap::new()).unwrap_err();
        assert!(error.starts_with("programs.json:"));
        assert_eq!(
            fallback.of(Some("pi"), &[]).color.as_deref(),
            Some("#a78bfa")
        );
    }

    #[test]
    fn a_terminal_record_shows_its_agent_else_what_runs_in_it() {
        let programs = Programs::default();
        let mut record = herdr_proto::Terminal {
            argv: vec!["/bin/zsh".into()],
            ..Default::default()
        };
        assert_eq!(programs.of_terminal(&record).key, "shell");
        record.foreground = Some(herdr_proto::types::Foreground {
            argv: vec!["nvim".into(), "x".into()],
            ..Default::default()
        });
        assert_eq!(programs.of_terminal(&record).key, "nvim");
        record.status = Some(herdr_proto::types::Status {
            app: "pi".into(),
            ..Default::default()
        });
        assert_eq!(programs.of_terminal(&record).key, "pi");
    }
}
