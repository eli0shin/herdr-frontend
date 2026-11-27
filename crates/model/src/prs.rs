//! The PR finder (`ctrl-g f`; Eli's `pr-finder` mod, `feature-inventory.md` §12): which agent
//! watches a PR. A port of `cfg/mods/pr-finder/match.js` and its `rows`; its tests follow
//! `dot/tests/herdr-app-pr-finder.mjs`.
//!
//! The rows are the Agents list's agents that watch at least one PR (their Pi
//! `dotfiles.pi_pr_watch` label), in the list's order, minus hidden machines (the list already
//! leaves those out). A typed query keeps the agents with a PR every term matches, best first,
//! each with the PRs that matched.

use crate::agents::{Card, List, Watch};

/// One watched PR: its label as the Agents list shows it (`#N` in the session's repo, `repo#N`
/// elsewhere), and its repo and number in full.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pr {
    pub label: String,
    pub repo: String,
    pub number: String,
}

/// One PR finder row: an agent and the PRs it watches that matched (all of them with no query).
#[derive(Clone, Debug, PartialEq)]
pub struct PrRow {
    pub card: Card,
    pub matched: Vec<Pr>,
}

/// An agent's watched PRs (`watchedPrs`): from the label's `refs` (`owner/repo#N`); a Pi from
/// before `refs` names its own repo's PRs `#N` only, so `fallback` (its space's repo) stands in.
/// A sha watch on a merge (`merge of #N`) counts as that PR when no PR is watched.
pub fn watched(watch: Option<&Watch>, fallback: &str) -> Vec<Pr> {
    let Some(watch) = watch else {
        return Vec::new();
    };
    let mut prs: Vec<Pr> = watch
        .prs
        .iter()
        .enumerate()
        .map(|(i, label)| split(label, watch.refs.get(i).cloned().flatten(), fallback))
        .collect();
    if prs.is_empty()
        && let Some(sha) = &watch.sha
        && let Some(number) = merge_number(sha)
    {
        prs.push(Pr {
            label: sha.clone(),
            repo: fallback.to_owned(),
            number,
        });
    }
    prs
}

/// The first `#<digits>` in a sha watch's text.
fn merge_number(sha: &str) -> Option<String> {
    sha.match_indices('#').find_map(|(at, _)| {
        let digits: String = sha[at + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        (!digits.is_empty()).then_some(digits)
    })
}

fn split(label: &str, reference: Option<String>, fallback: &str) -> Pr {
    let full = reference.unwrap_or_else(|| {
        if label.starts_with('#') && !fallback.is_empty() {
            format!("{fallback}{label}")
        } else {
            label.to_owned()
        }
    });
    let (repo, number) = match full.rfind('#') {
        Some(at) => (full[..at].to_owned(), full[at + 1..].to_owned()),
        None => (String::new(), String::new()),
    };
    Pr {
        label: label.to_owned(),
        repo,
        number,
    }
}

/// The repo a space's name stands for, for PRs named `#N` only: the name before any `@`, without
/// a leading `.`.
fn space_repo(name: &str) -> &str {
    let name = name.split('@').next().unwrap_or("");
    name.strip_prefix('.').unwrap_or(name)
}

/// The PR finder's rows for `query`: the agents the Agents list holds that watch a PR, in its
/// published row order (agents it doesn't show, such as folded workers, after, in layout
/// order), then ranked by `query` ([`rank`]).
pub fn rows(list: &List, query: &str) -> Vec<PrRow> {
    let place = |address: &str| {
        list.rows_order
            .iter()
            .position(|a| a == address)
            .unwrap_or(usize::MAX)
    };
    let mut cards: Vec<&Card> = list.cards.iter().collect();
    cards.sort_by_key(|c| place(&c.address));
    let all: Vec<(Card, Vec<Pr>)> = cards
        .into_iter()
        .map(|c| {
            let prs = watched(c.watch.as_ref(), space_repo(&c.space_name));
            (c.clone(), prs)
        })
        .filter(|(_, prs)| !prs.is_empty())
        .collect();
    rank(query, all)
}

/// `rankAgents`: terms split on whitespace and `#`, and every term must match one PR. Each row
/// keeps the PRs that matched, best first; rows rank by their best PR, ties keeping the given
/// order. An empty query keeps every row and its PRs.
pub fn rank(query: &str, rows: Vec<(Card, Vec<Pr>)>) -> Vec<PrRow> {
    let query = query.to_lowercase();
    let terms: Vec<&str> = query
        .split_whitespace()
        .flat_map(|t| t.split('#'))
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        return rows
            .into_iter()
            .map(|(card, matched)| PrRow { card, matched })
            .collect();
    }
    let mut scored: Vec<(u32, usize, PrRow)> = Vec::new();
    for (index, (card, prs)) in rows.into_iter().enumerate() {
        let mut matched: Vec<(u32, Pr)> = prs
            .into_iter()
            .filter_map(|pr| {
                let total: Option<u32> = terms.iter().map(|t| score(t, &pr)).sum();
                total.map(|t| (t, pr))
            })
            .collect();
        let Some(best) = matched.iter().map(|(t, _)| *t).max() else {
            continue;
        };
        matched.sort_by_key(|m| std::cmp::Reverse(m.0));
        let matched = matched.into_iter().map(|(_, pr)| pr).collect();
        scored.push((best, index, PrRow { card, matched }));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, row)| row).collect()
}

/// One term against one PR (`scoreTerm`). A number matches the PR's number: exact, then a
/// prefix, then inside it. A word matches the repo: its name exact, then its start, then
/// anywhere in `owner/repo` (better at a word start), then its letters in order ("fmr" finds
/// ffs-mono-repo).
fn score(term: &str, pr: &Pr) -> Option<u32> {
    let len = |s: &str| s.encode_utf16().count() as u32;
    if term.chars().all(|c| c.is_ascii_digit()) {
        let n = pr.number.as_str();
        if n == term {
            return Some(1000);
        }
        if n.starts_with(term) {
            return Some(600 - (len(n) - len(term)).min(600));
        }
        return n.contains(term).then_some(300);
    }
    let repo = pr.repo.to_lowercase();
    let name = &repo[repo.rfind('/').map_or(0, |i| i + 1)..];
    if name == term {
        return Some(900);
    }
    if name.starts_with(term) {
        return Some(700 - (len(name) - len(term)).min(700));
    }
    if let Some(at) = repo.find(term) {
        let start = repo[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_lowercase() && !c.is_ascii_digit());
        return Some(400 + if start { 50 } else { 0 });
    }
    // In-order letters.
    let chars: Vec<char> = repo.chars().collect();
    let mut total = 0;
    let mut last: Option<usize> = None;
    for c in term.chars() {
        let from = last.map_or(0, |l| l + 1);
        let next = from + chars.get(from..)?.iter().position(|&p| p == c)?;
        total += if next == from { 3 } else { 1 };
        last = Some(next);
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_shared::Displayed;

    fn watch(prs: &[&str], refs: &[&str], sha: Option<&str>) -> Watch {
        Watch {
            prs: prs.iter().map(|p| (*p).to_owned()).collect(),
            refs: refs.iter().map(|r| Some((*r).to_owned())).collect(),
            sha: sha.map(str::to_owned),
            ..Watch::default()
        }
    }

    fn pr(label: &str, repo: &str, number: &str) -> Pr {
        Pr {
            label: label.into(),
            repo: repo.into(),
            number: number.into(),
        }
    }

    fn card(name: &str) -> Card {
        Card {
            address: name.into(),
            title: name.into(),
            status: Displayed::Idle,
            focused: false,
            space_name: String::new(),
            space_color: None,
            machine: None,
            workers: 0,
            watch: None,
        }
    }

    fn ffs(n: u32) -> Pr {
        pr(&format!("#{n}"), "org/ffs-mono-repo", &n.to_string())
    }

    fn fixture() -> Vec<(Card, Vec<Pr>)> {
        vec![
            (
                card("orchestrator"),
                vec![
                    ffs(2031),
                    ffs(2032),
                    ffs(2040),
                    pr("tool#7", "another/tool", "7"),
                ],
            ),
            (card("worker"), vec![ffs(2032)]),
            (card("other"), vec![pr("#20320", "org/other", "20320")]),
        ]
    }

    fn names(query: &str) -> Vec<String> {
        rank(query, fixture())
            .into_iter()
            .map(|r| r.card.title)
            .collect()
    }

    #[test]
    fn watched_prs_come_from_refs_else_the_space_repo_stands_in_for_hash_n() {
        let w = watch(
            &["#104", "tool#7"],
            &["eli0shin/repos#104", "another/tool#7"],
            None,
        );
        assert_eq!(
            watched(Some(&w), "ignored"),
            [
                pr("#104", "eli0shin/repos", "104"),
                pr("tool#7", "another/tool", "7")
            ]
        );
        let old = watch(&["#104", "tool#7"], &[], None);
        assert_eq!(
            watched(Some(&old), "repos"),
            [pr("#104", "repos", "104"), pr("tool#7", "tool", "7")]
        );
        let merge = watch(&[], &[], Some("merge of #9"));
        assert_eq!(
            watched(Some(&merge), "repos"),
            [pr("merge of #9", "repos", "9")]
        );
        assert!(watched(Some(&watch(&[], &[], Some("SHA abc1234"))), "repos").is_empty());
        assert!(watched(None, "repos").is_empty());
    }

    #[test]
    fn an_empty_query_keeps_every_row_and_its_prs() {
        let all = rank("  ", fixture());
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].matched.len(), 4);
        assert_eq!(names(""), ["orchestrator", "worker", "other"]);
    }

    #[test]
    fn a_pr_number_finds_every_agent_watching_it_exact_before_a_longer_number() {
        let found = rank("2032", fixture());
        let titles: Vec<&str> = found.iter().map(|r| r.card.title.as_str()).collect();
        assert_eq!(titles, ["orchestrator", "worker", "other"]);
        let labels: Vec<&str> = found[0].matched.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["#2032"]);
        assert_eq!(names("#2032"), ["orchestrator", "worker", "other"]);
        assert!(names("999").is_empty());
    }

    #[test]
    fn repo_words_and_repo_hash_n_narrow_to_one_pr_and_terms_must_match_the_same_pr() {
        assert_eq!(names("tool"), ["orchestrator"]);
        assert_eq!(names("tool#7"), ["orchestrator"]);
        assert_eq!(names("ffs 2040"), ["orchestrator"]);
        assert!(names("tool 2032").is_empty());
        assert_eq!(names("fmr 2032"), ["orchestrator", "worker"]);
    }

    #[test]
    fn the_space_name_stands_for_its_repo() {
        assert_eq!(space_repo("repos@main"), "repos");
        assert_eq!(space_repo(".dotfiles"), "dotfiles");
        assert_eq!(space_repo(""), "");
    }
}
