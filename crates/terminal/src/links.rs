//! Links in terminal text: Ghostty's URL and file path pattern (`ghostty:src/config/url.zig`),
//! as Herdr app uses it unchanged (`app:src/links.rs`), and the link under a column of a line.
//!
//! The pattern finds scheme URLs (`https://`, `mailto:`, `file:`, ...), rooted paths (`/`,
//! `./`, `../`, `~/`, `$VAR/`, `.dir/`) and bare relative paths with a slash (`src/main.rs`,
//! `lib/x.zig:42:10`). Whether a path exists is for whoever opens it to find out.

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
