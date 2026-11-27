// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! Links in terminal text: Ghostty's URL and file path pattern (`ghostty:src/config/url.zig`),
//! as Herdr app uses it unchanged (`app:src/links.rs`), and the link under a column of a line.
//!
//! The pattern finds scheme URLs (`https://`, `mailto:`, `file:`, ...), rooted paths (`/`,
//! `./`, `../`, `~/`, `$VAR/`, `.dir/`) and bare relative paths with a slash (`src/main.rs`,
//! `lib/x.zig:42:10`). Whether a path exists is for whoever opens it to find out.
//!
//! [`word`] is the run of text around a column that could name a terminal (`app:src/links.rs`
//! `word`): whoever knows which terminals are live decides whether it does.

use std::ops::Range;
use std::sync::LazyLock;

use fancy_regex::Regex;

static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    let schemes = r"https?://|mailto:|ftp://|file:|ssh:|git://|ssh://|tel:|magnet:|ipfs://|ipns://|gemini://|gopher://|news:";
    let ipv6 = r"(?:\[[:0-9a-fA-F]+(?:[:0-9a-fA-F]*)+\](?::[0-9]+)?)";
    let scheme_chars = r"[\w\-.~:/?#@!$&*+,;=%]";
    let path_chars = r"[\w\-.~:\/?#@!$&*+;=%]";
    let bracketed = r"(?:[\(\[]\w*[\)\]])?";
    let no_trailing_punctuation = r"(?<![,.])";
    let no_trailing_colon = r"(?<!:)";
    let dotted = r"(?=[\w\-.~:\/?#@!$&*+;=%]*\.)";
    let undotted = r"(?![\w\-.~:\/?#@!$&*+;=%]*\.)";
    let dotted_spaces = r"(?:(?<!:) (?!\w+:\/\/)(?!\.{0,2}\/)(?!~\/)[\w\-.~:\/?#@!$&*+;=%]*[\/.])*";
    let any_spaces = r"(?:(?<!:) (?!\w+:\/\/)(?!\.{0,2}\/)(?!~\/)[\w\-.~:\/?#@!$&*+;=%]+)*";
    let rooted = r"(?:\.\.\/|\.\/|(?<!\w)~\/|(?:[\w][\w\-.]*\/)*(?<!\w)\$[A-Za-z_]\w*\/|\.[\w][\w\-.]*\/|(?<![\w~\/])\/(?!\/))";
    let scheme_url =
        format!("(?:{schemes})(?:{ipv6}|{scheme_chars}+{bracketed})+{no_trailing_punctuation}");
    let rooted_path = format!(
        "{rooted}(?:{dotted}{path_chars}+{dotted_spaces}{no_trailing_colon}|{undotted}{path_chars}+{any_spaces}{no_trailing_colon})"
    );
    let bare_path =
        format!(r"{dotted}(?<!\$\d*)(?<!\w)[\w][\w\-.]*\/{path_chars}+{no_trailing_colon}");
    Regex::new(&format!("{scheme_url}|{rooted_path}|{bare_path}")).expect("the link pattern")
});

/// The link in `text` that covers byte `at`, as a byte range.
pub fn find(text: &str, at: usize) -> Option<Range<usize>> {
    for m in PATTERN.find_iter(text) {
        let m = m.ok()?;
        if m.start() > at {
            return None;
        }
        if at < m.end() {
            return Some(m.range());
        }
    }
    None
}

/// The word around byte `at` of `text` that could be a terminal address: letters, digits, `-`,
/// `_`, `/` and `.`, without dots at its ends. `None` when `at` isn't in one
/// (`app:src/links.rs` `word`).
pub fn word(text: &str, at: usize) -> Option<Range<usize>> {
    let allowed = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.');
    if !text.get(at..)?.chars().next().is_some_and(allowed) {
        return None;
    }
    let start = text[..at]
        .char_indices()
        .rev()
        .find(|(_, c)| !allowed(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| !allowed(*c))
        .map_or(text.len(), |(i, _)| at + i);
    let found = &text[start..end];
    let start = start + found.len() - found.trim_start_matches('.').len();
    let end = start + found.trim_matches('.').len();
    (start <= at && at < end).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(text: &str) -> Option<&str> {
        PATTERN.find(text).unwrap().map(|m| m.as_str())
    }

    /// Ghostty's own cases (`ghostty:src/config/url.zig`), as Herdr app keeps them.
    #[test]
    fn matches_like_ghostty() {
        let cases = [
            ("hello https://example.com world", "https://example.com"),
            (
                "https://example.com/foo(bar) more",
                "https://example.com/foo(bar)",
            ),
            (
                "https://example.com/foo(bar)baz more",
                "https://example.com/foo(bar)baz",
            ),
            (
                "Link inside (https://example.com) parens",
                "https://example.com",
            ),
            (
                "Link period https://example.com. More text.",
                "https://example.com",
            ),
            (
                "Link trailing comma https://example.com, more text.",
                "https://example.com",
            ),
            (
                "Link in double quotes \"https://example.com\" and more",
                "https://example.com",
            ),
            (
                "also match http://example.com non-secure links",
                "http://example.com",
            ),
            (
                "match with query url https://example.com?query=1&other=2 and more text.",
                "https://example.com?query=1&other=2",
            ),
            ("dot.http://example.com", "http://example.com"),
            ("match file://example.com file links", "file://example.com"),
            ("/tmp/test.txt http://www.google.com", "/tmp/test.txt"),
            (
                "Serving HTTP on :: port 8000 (http://[::]:8000/)",
                "http://[::]:8000/",
            ),
            ("../example.py", "../example.py"),
            ("first time ../example.py contributor ", "../example.py"),
            (
                "[link](/home/user/ghostty.user/example)",
                "/home/user/ghostty.user/example",
            ),
            ("./spaces-end.   ", "./spaces-end."),
            ("../test folder/file.txt", "../test folder/file.txt"),
            ("/tmp/test  folder/file.txt", "/tmp/test"),
            (
                "diff --git a/src/font/shaper/harfbuzz.zig b/src/font/shaper/harfbuzz.zig",
                "a/src/font/shaper/harfbuzz.zig",
            ),
            ("/tmp/foo.txt /tmp/bar.txt", "/tmp/foo.txt"),
            ("src/config/url.zig", "src/config/url.zig"),
            ("app/folder/file.rb:1", "app/folder/file.rb:1"),
            ("modified:   src/config/url.zig", "src/config/url.zig"),
            (
                "lib/ghostty/terminal.zig:42:10",
                "lib/ghostty/terminal.zig:42:10",
            ),
            ("src/foo.c,baz.txt", "src/foo.c"),
            ("open ~/Documents/notes.md please", "~/Documents/notes.md"),
            ("~/.config/ghostty/config", "~/.config/ghostty/config"),
            ("$HOME/src/config/url.zig", "$HOME/src/config/url.zig"),
            (
                "space dir: $PWD/src/ghostty/main.zig",
                "$PWD/src/ghostty/main.zig",
            ),
            (".config/ghostty/config", ".config/ghostty/config"),
            (
                "  - shared/src/foo/SomeItem.m:12, shared/src/",
                "shared/src/foo/SomeItem.m:12",
            ),
            ("2024/report.txt", "2024/report.txt"),
            ("./Downloads: Operation not permitted", "./Downloads"),
        ];
        for (input, expect) in cases {
            assert_eq!(first(input), Some(expect), "{input:?}");
        }
        for input in [
            "input/output",
            "foo/bar",
            "$10/bar",
            "foo/bar,baz.txt",
            "foo$BAR/baz.txt",
            "// foo bar",
            "//foo",
        ] {
            assert_eq!(first(input), None, "{input:?}");
        }
    }

    fn at(text: &str, i: usize) -> Option<&str> {
        word(text, i).map(|r| &text[r])
    }

    #[test]
    fn a_word_is_letters_digits_dashes_slashes_and_inner_dots() {
        let text = "see t12, or devbox/t3. (x.y)";
        assert_eq!(at(text, 4), Some("t12"));
        assert_eq!(at(text, 6), Some("t12"));
        assert_eq!(at(text, 7), None, "the comma");
        assert_eq!(at(text, 12), Some("devbox/t3"));
        assert_eq!(at(text, 21), None, "a trailing dot isn't in it");
        assert_eq!(at(text, 24), Some("x.y"));
        assert_eq!(at(text, 3), None, "a space");
        assert_eq!(at(text, 99), None);
        assert_eq!(at("..t4..", 0), None);
        assert_eq!(at("..t4..", 2), Some("t4"));
        assert_eq!(at("é-t5", 3), Some("é-t5"), "not ASCII only");
    }

    #[test]
    fn finds_the_link_under_a_byte() {
        let text = "see https://a.com and /tmp/x.txt.";
        assert_eq!(find(text, 0), None);
        assert_eq!(find(text, 4).map(|r| &text[r]), Some("https://a.com"));
        assert_eq!(find(text, 16).map(|r| &text[r]), Some("https://a.com"));
        assert_eq!(find(text, 17), None);
        assert_eq!(find(text, 25).map(|r| &text[r]), Some("/tmp/x.txt."));
    }
}
