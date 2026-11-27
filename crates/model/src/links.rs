//! What a Cmd-click on a link in a terminal opens: Eli's `nvim-links` (`cfg/mods/nvim-links/`,
//! `feature-inventory.md` §17) over Herdr app's own link handling (`app:src/links.rs`).
//!
//! - A URL (anything with a scheme) goes to the system's opener, except a `file:` URI: Herdr
//!   hands those to link handlers as file links, and Pi marks every path it prints with one
//!   (OSC 8). It names the path on the pane's machine (no host, or `localhost`) and carries no
//!   place.
//! - A path is looked up on the clicked pane's machine with [`FIND`]: `~/`, a leading `$NAME`
//!   (read with `printenv`, never evaluated), `$PWD`, absolute, or relative to the pane's live
//!   folder; first as written, then without a sentence's period and a `:42` or `:42:7` place.
//! - A file Neovim edits ([`is_neovim_file`]) opens in Neovim in a split to the right of the
//!   clicked pane, on its machine, at the place it names. Anything else here (folders, images,
//!   PDFs) goes to the system's opener; there is no opener on another machine. A path found
//!   nowhere opens nothing.

/// The files Neovim edits: ones without an extension (Makefile, Dockerfile, `.gitignore`), and
/// the document types Eli's `install-neovim-app` gives the Neovim app
/// (`cfg/mods/nvim-links/open.js` `NEOVIM_EXTENSIONS`).
pub const NEOVIM_EXTENSIONS: &[&str] = &[
    "txt", "text", "md", "markdown", "json", "jsonc", "yaml", "yml", "toml", "xml", "csv", "log",
    "conf", "config", "ini", "env", "c", "h", "cc", "cpp", "hpp", "m", "mm", "swift", "go", "rs",
    "py", "rb", "php", "java", "kt", "kts", "scala", "js", "jsx", "ts", "tsx", "vue", "svelte",
    "sh", "bash", "zsh", "fish", "lua", "vim", "sql", "html", "htm", "css", "scss", "sass", "less",
];

/// How long the lookup may take on its machine, in seconds (`nvim-links`' 10 s).
pub const LOOKUP_TIMEOUT: f64 = 10.0;

/// A place in a file: `:42` or `:42:7`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub line: u32,
    pub col: Option<u32>,
}

/// Whether Neovim is the editor for the file at `path`: no extension, or one on the list.
pub fn is_neovim_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    // A leading dot starts a name (.gitignore), not an extension.
    let name = name.trim_start_matches('.');
    match name.rfind('.') {
        None => true,
        Some(dot) => NEOVIM_EXTENSIONS.contains(&name[dot + 1..].to_ascii_lowercase().as_str()),
    }
}

/// `raw` without a sentence's period and a `:42` or `:42:7` place, and that place.
pub fn split_place(raw: &str) -> (&str, Option<Place>) {
    let text = raw.strip_suffix('.').unwrap_or(raw);
    let number = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    if let Some((head, last)) = text.rsplit_once(':')
        && let Some(n) = number(last)
    {
        if let Some((bare, first)) = head.rsplit_once(':')
            && let Some(line) = number(first)
        {
            return (bare, Some(Place { line, col: Some(n) }));
        }
        return (head, Some(Place { line: n, col: None }));
    }
    (text, None)
}

/// The Neovim command opening `path` at `place`: `nvim +42 -- path`, or with a column
/// `nvim "+call cursor(42, 7)" -- path`.
pub fn neovim(path: &str, place: Option<Place>) -> Vec<String> {
    let mut argv = vec!["nvim".to_owned()];
    match place {
        Some(Place {
            line,
            col: Some(col),
        }) => argv.push(format!("+call cursor({line}, {col})")),
        Some(Place { line, col: None }) => argv.push(format!("+{line}")),
        None => {}
    }
    argv.extend(["--".to_owned(), path.to_owned()]);
    argv
}

/// Whether `link` starts with a URL scheme (`https:`, `mailto:`, `file:`), as Herdr app tells
/// URLs from paths.
pub fn has_scheme(link: &str) -> bool {
    link.split_once(':').is_some_and(|(s, _)| {
        s.len() > 1
            && s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    })
}

/// The path a `file:` URI names on its own machine (no host, or `localhost`), as Herdr reads
/// it; `None` for another host or a URI that isn't a path.
pub fn file_uri_path(raw: &str) -> Option<String> {
    let rest = raw.get(..5).filter(|s| s.eq_ignore_ascii_case("file:"))?;
    let mut rest = &raw[rest.len()..];
    rest = rest.split(['?', '#']).next().unwrap_or("");
    if let Some(after) = rest.strip_prefix("//") {
        let (host, path) = after.split_at(after.find('/').unwrap_or(after.len()));
        if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
            return None;
        }
        rest = path;
    }
    if !rest.starts_with('/') {
        return None;
    }
    percent_decode(rest)
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The lookup, a POSIX `sh` script run on the clicked pane's machine as
/// `sh -c FIND sh <cwd> <spelling>...`. It resolves each spelling as Herdr does and prints the
/// first that exists, `<index>\t<f|d|o>\t<path>` (a regular file, a folder or something else),
/// exiting 1 when none does. It never evaluates the clicked text: a leading `$NAME` is read with
/// `printenv` (`nvim-links`' `FIND`, which looks only for regular files; folders here go to the
/// opener).
pub const FIND: &str = r#"cwd=$1; shift; i=0
for p in "$@"; do
  case $p in
    "~/"*) p="$HOME/${p#\~/}" ;;
    '$'*)
      name=${p#?}; name=${name%%/*}; rest=${p#?"$name"}
      case $name in
        ''|[!A-Za-z_]*|*[!A-Za-z0-9_]*) p= ;;
        PWD) p=${cwd:+$cwd$rest} ;;
        *) value=$(printenv "$name") && [ -n "$value" ] && p=$value$rest || p= ;;
      esac ;;
    /*) ;;
    *) p="${cwd:+$cwd/$p}" ;;
  esac
  if [ -n "$p" ] && [ -e "$p" ]; then
    if [ -f "$p" ]; then k=f; elif [ -d "$p" ]; then k=d; else k=o; fi
    printf '%s\t%s\t%s\n' "$i" "$k" "$p"; exit 0
  fi
  i=$((i + 1))
done
exit 1"#;

/// What a lookup is for: the spellings it tries, each with the place in the file it names.
#[derive(Clone, Debug, PartialEq)]
pub struct Lookup {
    pub spellings: Vec<String>,
    pub places: Vec<Option<Place>>,
}

impl Lookup {
    /// The spellings of a clicked path: as written, then without its period and place. A
    /// `file:` URI is the one path it names, or `None` for another host.
    pub fn of(raw: &str) -> Option<Lookup> {
        if raw
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("file:"))
        {
            return Some(Lookup {
                spellings: vec![file_uri_path(raw)?],
                places: vec![None],
            });
        }
        let (bare, place) = split_place(raw);
        let mut lookup = Lookup {
            spellings: vec![raw.to_owned()],
            places: vec![None],
        };
        if bare != raw {
            lookup.spellings.push(bare.to_owned());
            lookup.places.push(place);
        }
        Some(lookup)
    }

    /// `sh`'s arguments on the pane's machine, with the pane's live folder (empty if unknown).
    pub fn argv(&self, cwd: &str) -> Vec<String> {
        let mut argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            FIND.to_owned(),
            "sh".to_owned(),
            cwd.to_owned(),
        ];
        argv.extend(self.spellings.iter().cloned());
        argv
    }
}

/// What the lookup found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: String,
    /// A regular file (not a folder, a device or a socket).
    pub file: bool,
    pub place: Option<Place>,
}

/// Reads what [`FIND`] printed for `lookup`.
pub fn found(lookup: &Lookup, stdout: &str) -> Option<Found> {
    let line = stdout.strip_suffix('\n').unwrap_or(stdout);
    let mut parts = line.splitn(3, '\t');
    let index: usize = parts.next()?.parse().ok()?;
    let kind = parts.next()?;
    let path = parts
        .next()
        .filter(|p| !p.is_empty() && !p.contains('\n'))?;
    Some(Found {
        path: path.to_owned(),
        file: kind == "f",
        place: lookup.places.get(index).copied().flatten(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neovim_edits_files_without_an_extension_and_listed_ones() {
        for path in [
            "/a/Makefile",
            "Dockerfile",
            "/a/.gitignore",
            "src/main.rs",
            "README.MD",
            "x.tsx",
        ] {
            assert!(is_neovim_file(path), "{path}");
        }
        for path in ["/a/photo.png", "doc.pdf", "/a/movie.mp4", "a.b/c.zip"] {
            assert!(!is_neovim_file(path), "{path}");
        }
    }

    #[test]
    fn places_come_off_the_end() {
        let at = |line, col| Some(Place { line, col });
        assert_eq!(split_place("src/a.rs:42"), ("src/a.rs", at(42, None)));
        assert_eq!(split_place("src/a.rs:42:7."), ("src/a.rs", at(42, Some(7))));
        assert_eq!(split_place("src/a.rs."), ("src/a.rs", None));
        assert_eq!(split_place("src/a.rs"), ("src/a.rs", None));
        assert_eq!(split_place("x:y"), ("x:y", None));
        assert_eq!(
            neovim("/a.rs", at(42, Some(7))),
            ["nvim", "+call cursor(42, 7)", "--", "/a.rs"]
        );
        assert_eq!(
            neovim("/a.rs", at(42, None)),
            ["nvim", "+42", "--", "/a.rs"]
        );
        assert_eq!(neovim("/a.rs", None), ["nvim", "--", "/a.rs"]);
    }

    #[test]
    fn schemes_and_file_uris() {
        assert!(has_scheme("https://a.com"));
        assert!(has_scheme("mailto:x@y"));
        assert!(has_scheme("file:///tmp/x"));
        assert!(!has_scheme("src/a.rs:42"));
        assert!(!has_scheme("/tmp/a"));
        assert!(!has_scheme("C:/x"), "one letter is a drive, not a scheme");
        assert_eq!(file_uri_path("file:///tmp/a%20b"), Some("/tmp/a b".into()));
        assert_eq!(
            file_uri_path("file://localhost/etc/hosts"),
            Some("/etc/hosts".into())
        );
        assert_eq!(file_uri_path("file://other/etc/hosts"), None);
        assert_eq!(file_uri_path("file:relative"), None);
    }

    #[test]
    fn a_lookup_tries_the_path_as_written_then_bare() {
        let lookup = Lookup::of("src/a.rs:42:7").unwrap();
        assert_eq!(lookup.spellings, ["src/a.rs:42:7", "src/a.rs"]);
        let argv = lookup.argv("/work");
        assert_eq!(&argv[..2], ["sh", "-c"]);
        assert_eq!(&argv[3..], ["sh", "/work", "src/a.rs:42:7", "src/a.rs"]);
        assert_eq!(
            found(&lookup, "1\tf\t/work/src/a.rs\n"),
            Some(Found {
                path: "/work/src/a.rs".into(),
                file: true,
                place: Some(Place {
                    line: 42,
                    col: Some(7)
                }),
            })
        );
        assert_eq!(
            found(&lookup, "0\td\t/work/src/a.rs:42:7\n").map(|f| (f.file, f.place)),
            Some((false, None))
        );
        assert_eq!(found(&lookup, ""), None);
    }

    /// The script, run here by `sh`, resolves every spelling as Herdr does.
    #[test]
    fn the_lookup_script_resolves_spellings() {
        let dir = std::env::temp_dir().join(format!("herdr-frontend-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "").unwrap();
        let cwd = dir.to_str().unwrap();
        let run = |raw: &str, folder: &str| {
            let lookup = Lookup::of(raw).unwrap();
            let argv = lookup.argv(folder);
            let out = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .env("FROM_TEST", cwd)
                .output()
                .unwrap();
            found(&lookup, &String::from_utf8_lossy(&out.stdout))
        };
        let file = format!("{cwd}/src/a.rs");
        assert_eq!(run("src/a.rs:3", cwd).map(|f| f.path), Some(file.clone()));
        assert_eq!(
            run("$PWD/src/a.rs", cwd).map(|f| f.path),
            Some(file.clone())
        );
        assert_eq!(
            run("$FROM_TEST/src/a.rs.", "").map(|f| f.path),
            Some(file.clone())
        );
        assert_eq!(run(&file, "").map(|f| f.file), Some(true));
        assert_eq!(run("src", cwd).map(|f| f.file), Some(false), "a folder");
        assert_eq!(run("src/gone.rs", cwd), None);
        assert_eq!(run("src/a.rs", ""), None, "relative, with no folder");
        assert_eq!(run("$(touch x)/a", cwd), None, "never evaluated");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
