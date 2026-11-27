//! What tabs are called on screen (`cfg/mods/eli-tabs/names.js`, identical in `eli-agents` and
//! `pr-finder`, `feature-inventory.md` §4 "Tab names from Pi").
//!
//! A named tab keeps its name. An unnamed tab whose first Pi pane's title reads
//! `π - <session> - <folder>` goes by `<session>`; duplicates of such names in a space gain
//! ` · <tab number>` (then `.2`, `.3` while that clashes too). Split tabs use their first Pi pane
//! in tree order, never the focused one, so names don't change with focus. Anything else goes by
//! [`crate::tab_name`]: its place among the space's tabs.
//!
//! Which terminals run Pi is the program table's call (herdr-app's `program.key == "pi"`), so the
//! caller passes it in.

use std::collections::{BTreeMap, BTreeSet};

use herdr_proto::Terminal;
use herdr_shared::{Displayed, Space};

use crate::model::Model;
use crate::world::{World, tab_name};

/// One pill of the tab bar (`cfg/mods/eli-tabs/main.js` `item`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPill {
    pub tab: String,
    /// What it reads: its name or Pi session ([`tab_names`]), else its number among the shown
    /// tabs, as Cmd+1-9 count them.
    pub label: String,
    /// Its space's selected tab: raised, and the only one with a close button.
    pub active: bool,
    /// A pane of it is zoomed: the label reads `<label> Z`.
    pub zoomed: bool,
    /// The most urgent status among its terminals: a spinner while working, a dot when blocked
    /// or done.
    pub status: Option<Displayed>,
}

impl Model {
    /// The tab bar's pills for `space`: its shown tabs ([`Model::shown_tabs`]), in order.
    /// `is_pi`: whether a terminal runs Pi, for [`tab_names`].
    pub fn tab_pills(&self, space: &Space, is_pi: impl Fn(&Terminal) -> bool) -> Vec<TabPill> {
        let names: BTreeMap<String, String> =
            tab_names(&self.world, space, is_pi).into_iter().collect();
        let selected = self
            .view
            .selected_tab(&self.world, space)
            .map(|t| t.id.as_str());
        self.shown_tabs(space)
            .into_iter()
            .enumerate()
            .map(|(i, tab)| {
                let name = names.get(&tab.id).cloned().unwrap_or_default();
                // Its own number counts every tab, hidden agents' too; the bar numbers the
                // shown ones.
                let label = if tab.name.is_empty() && name == tab_name(space, tab) {
                    (i + 1).to_string()
                } else {
                    name
                };
                let addresses = tab.root.addresses();
                TabPill {
                    tab: tab.id.clone(),
                    label,
                    active: selected == Some(tab.id.as_str()),
                    zoomed: self.view.zoomed_in(tab).is_some(),
                    status: self.attention.rollup(addresses.iter().map(String::as_str)),
                }
            })
            .collect()
    }
}

/// The session name a Pi terminal's title carries: `π - <session> - <folder>` gives `<session>`.
/// `cwd` is the terminal's live folder; an unnamed Pi session titles itself `π - <folder>`, which
/// gives nothing (`names.js` `piSessionName`).
pub fn pi_session_name(title: &str, cwd: Option<&str>) -> Option<String> {
    let rest = title.strip_prefix("π - ")?;
    let folder = cwd.map(|c| c.trim_end_matches('/').rsplit('/').next().unwrap_or(""));
    if Some(rest) == folder {
        return None;
    }
    let suffix = folder.filter(|f| !f.is_empty()).map(|f| format!(" - {f}"));
    let end = match &suffix {
        Some(suffix) if rest.ends_with(suffix.as_str()) => rest.len() - suffix.len(),
        _ => rest.rfind(" - ")?,
    };
    if end == 0 {
        return None;
    }
    let name: String = rest[..end]
        .chars()
        .filter(|c| !matches!(*c as u32, 0x00..=0x1f | 0x7f))
        .collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Every tab of `space` with what to call it, in order (`names.js` `tabNames`). `is_pi`: whether
/// a terminal runs Pi.
pub fn tab_names(
    world: &World,
    space: &Space,
    is_pi: impl Fn(&Terminal) -> bool,
) -> Vec<(String, String)> {
    let mut automatic = BTreeSet::new();
    let mut names: Vec<(String, String)> = space
        .tabs
        .iter()
        .map(|tab| {
            if tab.name.is_empty() {
                let lead = tab
                    .root
                    .addresses()
                    .into_iter()
                    .find_map(|a| world.terminal(&a).filter(|t| is_pi(t)));
                let name =
                    lead.and_then(|t| pi_session_name(&t.title, world.folder_of(&t.address)));
                if let Some(name) = name {
                    automatic.insert(tab.id.clone());
                    return (tab.id.clone(), name);
                }
            }
            (tab.id.clone(), tab_name(space, tab))
        })
        .collect();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, name) in &names {
        *counts.entry(name.clone()).or_default() += 1;
    }
    // Every original name is taken too: a person may already have used the suffix.
    let mut used: BTreeSet<String> = names.iter().map(|(_, n)| n.clone()).collect();
    for (i, (id, name)) in names.iter_mut().enumerate() {
        if !automatic.contains(id) || counts[name.as_str()] < 2 {
            continue;
        }
        let suffix = format!("{name} \u{b7} {}", i + 1);
        let mut unique = suffix.clone();
        let mut attempt = 2;
        while used.contains(&unique) {
            unique = format!("{suffix}.{attempt}");
            attempt += 1;
        }
        used.insert(unique.clone());
        *name = unique;
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_titles_give_the_session_name_without_the_folder() {
        let name = |title: &str, cwd: Option<&str>| pi_session_name(title, cwd);
        assert_eq!(
            name("π - fix the bar - herdr", Some("/code/herdr/")).as_deref(),
            Some("fix the bar")
        );
        assert_eq!(name("π - herdr", Some("/code/herdr")), None, "unnamed");
        // Another folder than the live one: the last ` - ` splits.
        assert_eq!(
            name("π - a - b - elsewhere", Some("/code/herdr")).as_deref(),
            Some("a - b")
        );
        assert_eq!(name("π - a - b", None).as_deref(), Some("a"));
        assert_eq!(name("π - solo", None), None, "no folder part");
        assert_eq!(name("π -  - herdr", Some("/x/herdr")), None, "blank");
        assert_eq!(
            name("π - \u{1b}[1mbold\u{7} - x", Some("/x")).as_deref(),
            Some("[1mbold")
        );
        assert_eq!(name("vim - herdr", Some("/x/herdr")), None, "not Pi's");
        assert_eq!(name("π - - herdr", Some("/x/herdr")), None);
    }
}
