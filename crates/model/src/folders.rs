//! The new-space folder picker (`cmd-n`, `ctrl-g ctrl-s`; Eli's `new-space` mod,
//! `feature-inventory.md` §11): the folders a new space can start in, and how a typed filter ranks
//! them. A port of `cfg/mods/new-space/folders.js`; its tests follow
//! `dot/tests/herdr-app-new-space.mjs`.
//!
//! The folders, best first: the focused terminal's ("here", so Enter right away starts where you
//! are, as plain `space.new` does), then `zoxide query -l` in frecency order, then the `repos`
//! config's paths ("repo") zoxide hasn't seen. Nothing is walked, so the list is there at once.
//! [`crate::Model`] asks the space's machine for zoxide's list, the repos config and (on another
//! machine) `$HOME` when the popup opens ([`NewSpace`]).

use serde_json::Value;

/// How long each of the picker's lookups may take, in seconds (the mod's 5 s).
pub const LOOKUP_TIMEOUT: f64 = 5.0;

/// The repos config, as the picker reads it on the space's machine.
pub const REPOS_CONFIG: &str = r#"cat "${XDG_CONFIG_HOME:-$HOME/.config}/repos/config.json""#;

/// Where a folder came from: its tag in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The focused terminal's folder: "here".
    Here,
    /// zoxide's list: no tag.
    Zoxide,
    /// The repos config: "repo".
    Repo,
    /// A typed `/…`, `~` or `~/…` path: "path".
    Typed,
}

impl Source {
    /// The tag the row shows.
    pub fn tag(self) -> Option<&'static str> {
        match self {
            Source::Here => Some("here"),
            Source::Zoxide => None,
            Source::Repo => Some("repo"),
            Source::Typed => Some("path"),
        }
    }
}

/// One folder a new space can start in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub path: String,
    pub source: Source,
}

impl Folder {
    /// The row's two parts: the folder's name and its parent, both as shown (`~` for home).
    pub fn parts(&self, home: Option<&str>) -> (String, String) {
        let shown = display_path(&self.path, home);
        match shown.rfind('/') {
            None => (shown, String::new()),
            Some(_) if shown == "/" => (shown, String::new()),
            Some(0) => (shown[1..].to_owned(), String::new()),
            Some(cut) => (shown[cut + 1..].to_owned(), shown[..cut].to_owned()),
        }
    }
}

/// One of the picker's lookups on the space's machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// `$HOME` on another machine.
    Home,
    /// `zoxide query -l`.
    Zoxide,
    /// The repos config.
    Repos,
}

impl Lookup {
    /// What runs for it.
    pub fn argv(self) -> Vec<String> {
        let words: &[&str] = match self {
            Lookup::Home => &["sh", "-c", r#"printf %s "$HOME""#],
            Lookup::Zoxide => &["zoxide", "query", "-l"],
            Lookup::Repos => &["sh", "-c", REPOS_CONFIG],
        };
        words.iter().map(|w| (*w).to_owned()).collect()
    }
}

/// The open picker: the machine its space goes on, where you are, and what its lookups said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSpace {
    /// Which opening this is, so a late answer for an earlier one is dropped.
    pub opening: u64,
    /// The shown space's machine (`None`: this computer). The space is made there.
    pub machine: Option<String>,
    /// The focused terminal's folder.
    pub focused: Option<String>,
    /// The machine's home, for `~`: this computer's at once; another machine's once it answers.
    pub home: Option<String>,
    pub zoxide: Option<String>,
    pub repos: Option<String>,
    /// Lookups not answered yet. While any is, the list says "Loading folders…".
    pub waiting: Vec<Lookup>,
}

impl NewSpace {
    pub fn loading(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// A lookup answered: its output, or `""` when it failed (as the mod's `output`).
    pub fn answered(&mut self, lookup: Lookup, output: String) {
        let Some(at) = self.waiting.iter().position(|l| *l == lookup) else {
            return;
        };
        self.waiting.remove(at);
        match lookup {
            Lookup::Home => {
                if !output.is_empty() {
                    self.home = Some(output);
                }
            }
            Lookup::Zoxide => self.zoxide = Some(output),
            Lookup::Repos => self.repos = Some(output),
        }
    }

    /// The folders gathered so far: only where you are until the lookups are all in.
    pub fn folders(&self) -> Vec<Folder> {
        if self.loading() {
            return gather(self.focused.as_deref(), None, &[]);
        }
        let repos = repos_paths(self.repos.as_deref().unwrap_or(""));
        gather(self.focused.as_deref(), self.zoxide.as_deref(), &repos)
    }

    /// The rows for `query`: a typed path first (`path`), then the matching folders, best first.
    pub fn rows(&self, query: &str) -> Vec<Folder> {
        let home = self.home.as_deref();
        let ranked = rank(query, self.folders(), home);
        let Some(typed) = typed_folder(query, home) else {
            return ranked;
        };
        let mut out = vec![Folder {
            path: typed.clone(),
            source: Source::Typed,
        }];
        out.extend(ranked.into_iter().filter(|f| f.path != typed));
        out
    }
}

/// The folders, best first: `focused`, then zoxide's lines, then `repos` (`gatherFolders`). Each
/// once, trailing slashes trimmed.
pub fn gather(focused: Option<&str>, zoxide: Option<&str>, repos: &[String]) -> Vec<Folder> {
    let mut out: Vec<Folder> = Vec::new();
    let mut add = |path: &str, source: Source| {
        let clean = trim_slash(path);
        if clean.is_empty() || out.iter().any(|f| f.path == clean) {
            return;
        }
        out.push(Folder {
            path: clean,
            source,
        });
    };
    if let Some(f) = focused {
        add(f, Source::Here);
    }
    for line in zoxide.unwrap_or("").split('\n') {
        add(line.trim(), Source::Zoxide);
    }
    for path in repos {
        add(path, Source::Repo);
    }
    out
}

/// The paths in a repos `config.json` (`{repos: [{path}]}`); none when it is unreadable.
pub fn repos_paths(config: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(config) else {
        return Vec::new();
    };
    match value.get("repos") {
        Some(Value::Array(repos)) => repos
            .iter()
            .filter_map(|r| r.get("path")?.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// `path` with home as `~`.
pub fn display_path(path: &str, home: Option<&str>) -> String {
    match home {
        Some(home) if !home.is_empty() => match path.strip_prefix(home) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
            _ => path.to_owned(),
        },
        _ => path.to_owned(),
    }
}

/// A typed path (`/…`, `~`, `~/…`) as a folder, so any folder stays reachable (`typedFolder`).
/// `~` needs the machine's home.
pub fn typed_folder(query: &str, home: Option<&str>) -> Option<String> {
    let q = query.trim();
    if q.starts_with('/') {
        let trimmed = trim_slash(q);
        return Some(if trimmed.is_empty() {
            "/".into()
        } else {
            trimmed
        });
    }
    let home = home.filter(|h| !h.is_empty())?;
    (q == "~" || q.starts_with("~/")).then(|| trim_slash(&format!("{home}{}", &q[1..])))
}

/// fzf-like (`rankFolders`): every space-separated term must match somewhere in the shown path,
/// in any order, a term's letters in order. A term inside the last segment beats one elsewhere,
/// a segment start beats mid-word, shorter names win, and ties keep the gathered order.
pub fn rank(query: &str, folders: Vec<Folder>, home: Option<&str>) -> Vec<Folder> {
    let query = query.trim().to_lowercase();
    let terms: Vec<&str> = query.split_whitespace().collect();
    if terms.is_empty() {
        return folders;
    }
    let mut scored: Vec<(f64, usize, Folder)> = Vec::new();
    for (index, folder) in folders.into_iter().enumerate() {
        let shown = display_path(&folder.path, home).to_lowercase();
        let total: Option<f64> = terms.iter().map(|t| score_term(t, &shown)).sum();
        if let Some(total) = total {
            scored.push((total, index, folder));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, f)| f).collect()
}

/// One term's score in a lowercased shown path, `None` when it doesn't match (`scoreTerm`).
/// Positions and lengths count UTF-16 units, as the mod's do.
fn score_term(term: &str, path: &str) -> Option<f64> {
    let units = |s: &str| s.encode_utf16().count();
    let base = &path[path.rfind('/').map_or(0, |i| i + 1)..];
    if let Some(at) = base.find(term) {
        // Shorter names win: "repos" over "repos-clean-index-keeps-focus".
        let start = if at == 0 { 200.0 } else { 0.0 };
        let whole = if units(base) == units(term) {
            300.0
        } else {
            0.0
        };
        return Some(1000.0 + start + whole - units(base) as f64);
    }
    if let Some(at) = path.find(term) {
        let start = if path[..at].ends_with('/') {
            100.0
        } else {
            0.0
        };
        return Some(500.0 + start - units(path) as f64 / 100.0);
    }
    // In-order letters: reward runs and segment starts.
    let chars: Vec<char> = path.chars().collect();
    let mut score = 0.0;
    let mut at: Option<usize> = None;
    let mut run = 0.0;
    for c in term.chars() {
        let from = at.map_or(0, |a| a + 1);
        let next = from + chars.get(from..)?.iter().position(|&p| p == c)?;
        run = if at.is_some_and(|a| next == a + 1) || (at.is_none() && next == 0) {
            run + 1.0
        } else {
            0.0
        };
        let boundary = next > 0 && matches!(chars[next - 1], '/' | '-');
        score += 1.0 + run * 2.0 + if boundary { 3.0 } else { 0.0 };
        at = Some(next);
    }
    Some(score)
}

/// Trailing slashes go, but `/` stays `/` (`trimSlash`).
fn trim_slash(path: &str) -> String {
    if path.chars().count() > 1 {
        path.trim_end_matches('/').to_owned()
    } else {
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: Option<&str> = Some("/Users/eli");

    fn paths(folders: &[Folder]) -> Vec<&str> {
        folders.iter().map(|f| f.path.as_str()).collect()
    }

    fn zoxide(lines: &[&str]) -> Vec<Folder> {
        gather(None, Some(&lines.join("\n")), &[])
    }

    #[test]
    fn the_focused_folder_comes_first_then_zoxide_then_repos_zoxide_has_not_seen() {
        let folders = gather(
            Some("/Users/eli/code/repos/"),
            Some("/Users/eli/code/repos\n/Users/eli/.dotfiles\n\n"),
            &[
                "/Users/eli/.dotfiles".into(),
                "/Users/eli/code/ffs-mono-repo".into(),
            ],
        );
        let tagged: Vec<(&str, Source)> = folders
            .iter()
            .map(|f| (f.path.as_str(), f.source))
            .collect();
        assert_eq!(
            tagged,
            [
                ("/Users/eli/code/repos", Source::Here),
                ("/Users/eli/.dotfiles", Source::Zoxide),
                ("/Users/eli/code/ffs-mono-repo", Source::Repo),
            ]
        );
    }

    #[test]
    fn without_sources_yet_only_the_focused_folder_is_offered() {
        assert!(gather(None, None, &[]).is_empty());
        assert_eq!(paths(&gather(Some("/"), None, &[])), ["/"]);
    }

    #[test]
    fn repos_config_paths_ignore_anything_unreadable() {
        assert_eq!(
            repos_paths(r#"{"repos": [{"name": "a", "path": "/x/a"}, {"name": "b"}]}"#),
            ["/x/a"]
        );
        assert!(repos_paths("").is_empty());
        assert!(repos_paths(r#"{"repos": 3}"#).is_empty());
    }

    #[test]
    fn an_empty_query_keeps_the_gathered_order() {
        let folders = zoxide(&["/b", "/a"]);
        assert_eq!(rank("  ", folders.clone(), HOME), folders);
    }

    #[test]
    fn the_folder_named_like_the_query_beats_longer_names_and_parent_matches() {
        let folders = zoxide(&[
            "/Users/eli/code/repos-clean-index-keeps-focus",
            "/Users/eli/code/repos/src",
            "/Users/eli/code/repos",
            "/Users/eli/code/ffs-mono-repo",
        ]);
        assert_eq!(
            paths(&rank("repos", folders, HOME)),
            [
                "/Users/eli/code/repos",
                "/Users/eli/code/repos-clean-index-keeps-focus",
                "/Users/eli/code/repos/src",
            ]
        );
    }

    #[test]
    fn scattered_letters_match_in_order_and_every_term_must_match() {
        let folders = zoxide(&[
            "/Users/eli/code/ffs-mono-repo",
            "/Users/eli/code/forgecad-work",
            "/Users/eli/.dotfiles",
        ]);
        assert_eq!(
            paths(&rank("fmr", folders.clone(), HOME)),
            ["/Users/eli/code/ffs-mono-repo"]
        );
        assert_eq!(
            paths(&rank("code work", folders.clone(), HOME)),
            ["/Users/eli/code/forgecad-work"]
        );
        assert!(rank("zzz", folders, HOME).is_empty());
    }

    #[test]
    fn home_shows_as_tilde_and_the_query_matches_what_is_shown() {
        assert_eq!(display_path("/Users/eli/code", HOME), "~/code");
        assert_eq!(display_path("/Users/eli", HOME), "~");
        assert_eq!(display_path("/Users/elise", HOME), "/Users/elise");
        assert_eq!(display_path("/Users/eli", None), "/Users/eli");
        let folders = zoxide(&["/Users/eli/code/x", "/opt/users/eli"]);
        assert_eq!(paths(&rank("~/c", folders, HOME)), ["/Users/eli/code/x"]);
    }

    #[test]
    fn a_typed_absolute_or_home_path_is_a_folder_of_its_own() {
        assert_eq!(typed_folder("/tmp/x/", HOME).as_deref(), Some("/tmp/x"));
        assert_eq!(typed_folder("/", HOME).as_deref(), Some("/"));
        assert_eq!(typed_folder("//", HOME).as_deref(), Some("/"));
        assert_eq!(typed_folder("~", HOME).as_deref(), Some("/Users/eli"));
        assert_eq!(
            typed_folder(" ~/code/new ", HOME).as_deref(),
            Some("/Users/eli/code/new")
        );
        assert_eq!(typed_folder("repos", HOME), None);
        assert_eq!(typed_folder("~/x", None), None);
    }

    #[test]
    fn rows_show_name_and_parent_as_shown() {
        let f = |p: &str| Folder {
            path: p.into(),
            source: Source::Zoxide,
        };
        let parts = |p: &str| f(p).parts(HOME);
        assert_eq!(
            parts("/Users/eli/code/repos"),
            ("repos".into(), "~/code".into())
        );
        assert_eq!(parts("/Users/eli"), ("~".into(), String::new()));
        assert_eq!(parts("/Users/eli/x"), ("x".into(), "~".into()));
        assert_eq!(parts("/"), ("/".into(), String::new()));
        assert_eq!(parts("/opt"), ("opt".into(), String::new()));
        assert_eq!(parts("/opt/a"), ("a".into(), "/opt".into()));
    }

    #[test]
    fn a_typed_path_leads_the_rows_and_is_not_listed_twice() {
        let mut ns = NewSpace {
            opening: 1,
            machine: None,
            focused: Some("/Users/eli/code/repos".into()),
            home: HOME.map(str::to_owned),
            zoxide: None,
            repos: None,
            waiting: vec![Lookup::Zoxide, Lookup::Repos],
        };
        assert!(ns.loading());
        assert_eq!(paths(&ns.rows("")), ["/Users/eli/code/repos"]);
        ns.answered(
            Lookup::Zoxide,
            "/Users/eli/code/api\n/Users/eli/code/repos\n".into(),
        );
        assert!(ns.loading(), "the repos config is still out");
        ns.answered(
            Lookup::Repos,
            r#"{"repos":[{"path":"/Users/eli/code/web"}]}"#.into(),
        );
        assert!(!ns.loading());
        assert_eq!(
            paths(&ns.rows("")),
            [
                "/Users/eli/code/repos",
                "/Users/eli/code/api",
                "/Users/eli/code/web"
            ]
        );
        let rows = ns.rows("~/code/api");
        assert_eq!(rows[0].source, Source::Typed);
        assert_eq!(
            paths(&rows),
            ["/Users/eli/code/api"],
            "listed once, as typed"
        );
        // Another machine's `~` waits for its home.
        ns.home = None;
        assert!(typed_folder("~", ns.home.as_deref()).is_none());
        ns.waiting = vec![Lookup::Home];
        ns.answered(Lookup::Home, String::new());
        assert_eq!(ns.home, None, "a failed lookup leaves it unknown");
    }
}
