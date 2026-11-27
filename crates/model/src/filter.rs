//! Eli's filter matching, shared by every popup that filters as you type: a port of
//! `cfg/mods/space-picker/match.js` (`rankSpaces`, `scoreTerm`, `scoreField`, `wordStart`,
//! `displayPath`).
//!
//! fzf-like: every space-separated term must match somewhere, in any order, and a term's letters
//! match in order. The item's name counts most, then its other fields. Within a field a whole
//! word beats scattered letters, a match at a word start beats one mid-word, and shorter fields
//! win. Ties keep the items' order.

/// The query's terms: lowercased, split on whitespace.
pub fn terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// One item's score for every term (higher is better), or `None` when a term matches nowhere.
/// `name` counts above `others`.
pub fn score(terms: &[String], name: &str, others: &[&str]) -> Option<i64> {
    let name = name.to_lowercase();
    let others: Vec<String> = others
        .iter()
        .filter(|f| !f.is_empty())
        .map(|f| f.to_lowercase())
        .collect();
    let mut total = 0;
    for term in terms {
        total += score_term(term, &name, &others)?;
    }
    Some(total)
}

/// The indices of the items every term matches, best first; ties keep their order. With no
/// terms, every item in order. `fields` gives an item's name and its other fields.
pub fn rank<T>(
    query: &str,
    items: &[T],
    fields: impl Fn(&T) -> (String, Vec<String>),
) -> Vec<usize> {
    let terms = terms(query);
    if terms.is_empty() {
        return (0..items.len()).collect();
    }
    let mut scored: Vec<(i64, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let (name, others) = fields(item);
            let others: Vec<&str> = others.iter().map(String::as_str).collect();
            score(&terms, &name, &others).map(|s| (s, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// A folder as the popups show it: `~` for the home folder and what is under it.
pub fn display_path(path: &str, home: &str) -> String {
    if !home.is_empty() {
        if path == home {
            return "~".into();
        }
        if let Some(rest) = path.strip_prefix(home)
            && rest.starts_with('/')
        {
            return format!("~{rest}");
        }
    }
    path.to_owned()
}

fn score_term(term: &str, name: &str, others: &[String]) -> Option<i64> {
    if let Some(s) = score_field(term, name) {
        return Some(2000 + s);
    }
    others.iter().filter_map(|f| score_field(term, f)).max()
}

/// Higher is better; `None` when the term's letters aren't all in the field, in order.
fn score_field(term: &str, field: &str) -> Option<i64> {
    let chars: Vec<char> = field.chars().collect();
    let len = chars.len() as i64;
    let term_chars: Vec<char> = term.chars().collect();
    if let Some(at) = find(&chars, &term_chars, 0) {
        let place = if at == 0 {
            200
        } else if word_start(&chars, at) {
            100
        } else {
            0
        };
        let whole = if chars.len() == term_chars.len() {
            300
        } else {
            0
        };
        return Some(1000 + place + whole - len.min(100));
    }
    // In-order letters: runs and word starts score higher ("pf" finds photo-frame).
    let mut score = 0;
    let mut last: Option<usize> = None;
    let mut run = 0;
    for c in term_chars {
        let from = last.map_or(0, |l| l + 1);
        let next = (from..chars.len()).find(|&i| chars[i] == c)?;
        run = if last.is_some_and(|l| next == l + 1) {
            run + 1
        } else {
            0
        };
        score += 1 + run * 2 + if word_start(&chars, next) { 3 } else { 0 };
        last = Some(next);
    }
    Some(score)
}

/// Where `needle` first starts in `hay` at or after `from`, in characters.
fn find(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    (from..=hay.len().checked_sub(needle.len())?).find(|&i| hay[i..i + needle.len()] == *needle)
}

fn word_start(text: &[char], at: usize) -> bool {
    at == 0 || !text[at - 1].is_ascii_lowercase() && !text[at - 1].is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(query: &str, names: &[&str]) -> Vec<String> {
        rank(query, names, |n| (n.to_string(), Vec::new()))
            .into_iter()
            .map(|i| names[i].to_owned())
            .collect()
    }

    #[test]
    fn every_term_must_match_in_any_order() {
        let names = ["herdr-frontend", "dotfiles", "herdr-app"];
        assert_eq!(ranked("app herdr", &names), ["herdr-app"]);
        assert_eq!(ranked("zzz", &names), Vec::<String>::new());
        assert_eq!(ranked("  ", &names), names);
    }

    #[test]
    fn a_whole_name_beats_a_word_start_beats_mid_word_beats_scattered_letters() {
        assert_eq!(
            ranked("app", &["xappx", "my-app-two", "app-x", "app", "a-p-p"]),
            ["app", "app-x", "my-app-two", "xappx", "a-p-p"]
        );
        // Scattered letters at word starts: "pf" finds photo-frame.
        assert_eq!(
            ranked("pf", &["profile", "photo-frame"]),
            ["photo-frame", "profile"]
        );
    }

    #[test]
    fn shorter_names_win_and_ties_keep_the_order() {
        assert_eq!(
            ranked("web", &["web-tools", "web-a"]),
            ["web-a", "web-tools"]
        );
        assert_eq!(ranked("x", &["ax", "bx"]), ["ax", "bx"]);
    }

    #[test]
    fn the_name_counts_above_the_other_fields() {
        let items = [("api", "~/code/web"), ("web", "~/code/api")];
        let order = rank("web", &items, |(n, f)| (n.to_string(), vec![f.to_string()]));
        assert_eq!(order, [1, 0]);
        // A term found only in another field still matches.
        assert_eq!(
            rank("code", &items, |(n, f)| (
                n.to_string(),
                vec![f.to_string()]
            )),
            [0, 1]
        );
    }

    #[test]
    fn matching_ignores_case_and_handles_non_ascii() {
        assert_eq!(ranked("NAÏ", &["naïve"]), ["naïve"]);
        assert_eq!(ranked("🦀", &["🦀 crab"]), ["🦀 crab"]);
    }

    #[test]
    fn folders_under_home_show_with_a_tilde() {
        assert_eq!(display_path("/home/me", "/home/me"), "~");
        assert_eq!(display_path("/home/me/code/x", "/home/me"), "~/code/x");
        assert_eq!(display_path("/home/meow", "/home/me"), "/home/meow");
        assert_eq!(display_path("/tmp", ""), "/tmp");
    }
}
