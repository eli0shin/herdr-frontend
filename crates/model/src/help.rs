//! The keys help (`cmd-/`, `feature-inventory.md` §10): every binding that can run, grouped as
//! Herdr app's help groups them (spaces, tabs, panes, agents, terminal, window), with the keys
//! of non-global sections labelled, and filtered by typing. Built from the [`Keymap`], so a key
//! file's changes show at once.

use crate::action::{
    Action, AppCommand, CopyAction, Direction, Popup, PopupAction, Split, TerminalAction,
};
use crate::keys::{Chord, Context, Keymap};

/// The help's groups, in the order they show.
pub const GROUPS: [&str; 8] = [
    "spaces",
    "tabs",
    "panes",
    "agents",
    "terminal",
    "window",
    "popups",
    "copy mode",
];

/// One action and the keys that run it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpRow {
    /// One of [`GROUPS`].
    pub group: &'static str,
    /// The key file section, for keys outside `global` and `layout:eli`: `terminal`, `popup`,
    /// `copy`.
    pub section: Option<&'static str>,
    pub title: String,
    /// The command name ([`Action::name`]).
    pub name: String,
    /// Each sequence, in Eli's syntax.
    pub keys: Vec<String>,
}

impl HelpRow {
    /// Whether the row shows for a typed filter: it is in the title, the command name, the
    /// group, the section or one of the keys (any case), as Herdr app's help filters.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        let has = |field: &str| field.to_lowercase().contains(&query);
        has(&self.title)
            || has(&self.name)
            || has(self.group)
            || self.section.is_some_and(has)
            || self.keys.iter().any(|k| has(k))
    }
}

/// The rows for every action a binding runs, in [`GROUPS`] order. Bindings to `"terminal"` and
/// `global` keys `layout:eli` overrides are left out.
pub fn rows(keymap: &Keymap) -> Vec<HelpRow> {
    let mut rows: Vec<HelpRow> = Vec::new();
    for b in keymap.effective() {
        let Some(action) = &b.action else { continue };
        let Some(name) = action.name() else { continue };
        let section = match b.context {
            Context::Global | Context::Layout => None,
            other => Some(other.section()),
        };
        let keys = sequence(&b.keys);
        match rows
            .iter_mut()
            .find(|r| r.name == name && r.section == section)
        {
            Some(row) => {
                if !row.keys.contains(&keys) {
                    row.keys.push(keys);
                }
            }
            None => rows.push(HelpRow {
                group: group(action, b.context),
                section,
                title: title(action),
                name,
                keys: vec![keys],
            }),
        }
    }
    for row in &mut rows {
        row.keys.sort_by_key(|k| (k.contains(' '), k.clone()));
    }
    rows.sort_by(|a, b| order(a).cmp(&order(b)));
    rows
}

/// The rows a filter leaves.
pub fn filtered<'a>(rows: &'a [HelpRow], query: &str) -> Vec<&'a HelpRow> {
    rows.iter().filter(|r| r.matches(query)).collect()
}

/// The key a context menu shows beside an item that runs `action`: its first sequence in `global`
/// (`cfg/mods/menu/main.js` `shortcut`). Herdr app lists a section's keys sorted, as the key file
/// reads them, so `ctrl-g r` comes before `f2`.
pub fn hint(keymap: &Keymap, action: &Action) -> Option<String> {
    keymap
        .bindings()
        .iter()
        .find(|b| b.context == Context::Global && b.action.as_ref() == Some(action))
        .map(|b| sequence(&b.keys))
}

/// A sequence in Eli's syntax: `ctrl-g s`.
pub fn sequence(keys: &[Chord]) -> String {
    keys.iter()
        .map(Chord::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rows sort by group, section, then command name with its number read as a number
/// (`tab.select:2` before `tab.select:10`).
fn order(row: &HelpRow) -> (usize, Option<&'static str>, String, u32) {
    let group = GROUPS.iter().position(|g| *g == row.group).unwrap_or(0);
    let split = row.name.trim_end_matches(|c: char| c.is_ascii_digit());
    let number = row.name[split.len()..].parse().unwrap_or(0);
    (group, row.section, split.to_owned(), number)
}

/// Which help group an action is in.
fn group(action: &Action, context: Context) -> &'static str {
    match context {
        Context::Popup => return "popups",
        Context::Copy => return "copy mode",
        Context::Terminal => return "terminal",
        Context::Global | Context::Layout => {}
    }
    match action {
        Action::SpaceNext
        | Action::SpacePrev
        | Action::SpaceSelect(_)
        | Action::SpaceRename
        | Action::SpaceClose
        | Action::Open(Popup::SpacePicker | Popup::NewSpace) => "spaces",
        Action::TabNew
        | Action::TabClose
        | Action::TabNext
        | Action::TabPrev
        | Action::TabSelect(_)
        | Action::TabRename
        | Action::JumpBack
        | Action::JumpForward
        | Action::Open(Popup::Picker) => "tabs",
        Action::PaneSplit(_)
        | Action::PaneClose
        | Action::PaneZoom
        | Action::PaneGo(_)
        | Action::PaneNavigate(_)
        | Action::PaneSwap(_)
        | Action::PaneResize(_) => "panes",
        Action::AgentNext
        | Action::AgentPrev
        | Action::AgentSelect(_)
        | Action::Review
        | Action::Open(
            Popup::AgentJump | Popup::AgentMachines | Popup::PrFinder | Popup::SortMenu,
        ) => "agents",
        Action::Terminal(_) | Action::CopyMode => "terminal",
        Action::Popup(_) | Action::AgentJumpKey(_) => "popups",
        Action::Copy(_) => "copy mode",
        _ => "window",
    }
}

/// What an action does, in a few words (§22's "What" column).
pub fn title(action: &Action) -> String {
    let dir = |d: &Direction| d.as_str();
    match action {
        Action::App(command) => match command {
            AppCommand::Quit => "Quit",
            AppCommand::Hide => "Hide app",
            AppCommand::HideOthers => "Hide others",
            AppCommand::Reset => "Escape hatch: default keys",
            AppCommand::ZoomIn => "Zoom in",
            AppCommand::ZoomOut => "Zoom out",
            AppCommand::ZoomReset => "Reset zoom",
            AppCommand::ToggleSidebar => "Toggle sidebar",
            AppCommand::LayoutSwitch => "Next layout",
            AppCommand::CloseWindow => "Close window (terminals keep running)",
            AppCommand::Maximize => "Maximize window",
        }
        .into(),
        Action::Open(popup) => match popup {
            Popup::Palette => "Command palette",
            Popup::Settings => "Settings",
            Popup::Help => "Keys help",
            Popup::Picker => "Tab and agent picker",
            Popup::SpacePicker => "Space picker",
            Popup::NewSpace => "New space in a picked folder",
            Popup::AgentMachines => "Machine filter",
            Popup::PrFinder => "Find a PR's agent",
            Popup::AgentJump => "Jump to an agent by label",
            Popup::SortMenu => "Sort agents",
        }
        .into(),
        Action::CopyMode => "Copy mode".into(),
        Action::Review => "Review view / back".into(),
        Action::TabNew | Action::NewTabIn(_) => "New tab".into(),
        Action::TabMenu(_) => "Tab menu".into(),
        Action::TabClose => "Close tab".into(),
        Action::TabNext => "Next tab / next agent on an agent's tab".into(),
        Action::TabPrev => "Previous tab / previous agent".into(),
        Action::TabSelect(n) => format!("Tab {n}"),
        Action::TabRename => "Rename tab".into(),
        Action::JumpBack => "Back in the tab jump list".into(),
        Action::JumpForward => "Forward in the tab jump list".into(),
        Action::SpaceNext => "Next space".into(),
        Action::SpacePrev => "Previous space".into(),
        Action::SpaceSelect(n) => format!("Space {n}"),
        Action::SpaceRename => "Rename space".into(),
        Action::SpaceClose => "Close space".into(),
        Action::AgentNext => "Next agent".into(),
        Action::AgentPrev => "Previous agent".into(),
        Action::AgentSelect(n) => format!("Agent row {n}"),
        Action::PaneSplit(Split::Right) => "Split right".into(),
        Action::PaneSplit(Split::Down) => "Split down".into(),
        Action::PaneClose => "Close pane".into(),
        Action::PaneZoom => "Zoom pane".into(),
        Action::PaneGo(d) => format!("Focus the pane {}", dir(d)),
        Action::PaneNavigate(d) => format!("Focus the pane {} (Neovim-aware)", dir(d)),
        Action::PaneSwap(d) => format!("Swap the pane {}", dir(d)),
        Action::PaneResize(d) => format!("Resize the pane {}", dir(d)),
        Action::Terminal(t) => match t {
            TerminalAction::Search => "Find in screen and scrollback".into(),
            TerminalAction::Copy => "Copy the selection".into(),
            TerminalAction::Paste => "Paste".into(),
            TerminalAction::PasteImage => "Paste an image".into(),
            TerminalAction::SelectAll => "Select all".into(),
            TerminalAction::Send(text) => format!("Send {}", text.escape_debug()),
        },
        Action::Popup(p) => match p {
            PopupAction::Up => "Up",
            PopupAction::Down => "Down",
            PopupAction::Choose => "Choose",
            PopupAction::Close => "Close",
            PopupAction::Toggle => "Show or hide",
            PopupAction::CloseItem => "Close the selected item",
            PopupAction::Query(_) => "Filter",
        }
        .into(),
        Action::Copy(c) => match c {
            CopyAction::Escape => "Drop the selection, else leave".into(),
            CopyAction::Yank => "Copy and leave".into(),
            CopyAction::Quit => "Leave".into(),
            other => other.as_str().replace('_', " "),
        },
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(rows: &'a [HelpRow], name: &str, section: Option<&str>) -> &'a HelpRow {
        rows.iter()
            .find(|r| r.name == name && r.section == section)
            .unwrap_or_else(|| panic!("no row {name} {section:?}"))
    }

    #[test]
    fn menu_hints_are_the_first_global_key() {
        let keys = Keymap::eli();
        assert_eq!(hint(&keys, &Action::TabRename).as_deref(), Some("ctrl-g r"));
        assert_eq!(hint(&keys, &Action::TabNew).as_deref(), Some("cmd-t"));
        assert_eq!(hint(&keys, &Action::TabClose).as_deref(), Some("cmd-w"));
        assert_eq!(hint(&keys, &Action::RenameTabOf("tb1".into())), None);
    }

    #[test]
    fn rows_list_each_action_once_with_all_its_keys() {
        let rows = rows(&Keymap::eli());
        let new_tab = row(&rows, "tab.new", None);
        assert_eq!(new_tab.keys, ["cmd-t", "ctrl-g t"]);
        assert_eq!(new_tab.group, "tabs");
        assert_eq!(new_tab.title, "New tab");
        // `global` and `layout:eli` both bind ctrl-tab to the next tab: one row, one key.
        let next = row(&rows, "tab.next", None);
        assert_eq!(next.keys, ["cmd-shift-]", "ctrl-tab", "ctrl-g n"]);
        let help = row(&rows, "popup.help", None);
        assert_eq!(help.keys, ["cmd-/", "cmd-shift-/"]);
        assert_eq!(help.group, "window");
    }

    #[test]
    fn keys_layout_eli_overrides_show_only_under_its_action() {
        let rows = rows(&Keymap::eli());
        assert_eq!(row(&rows, "pane.navigate:left", None).keys, ["ctrl-h"]);
        assert_eq!(row(&rows, "pane.go:left", None).keys, ["cmd-alt-left"]);
    }

    #[test]
    fn other_sections_are_labelled_and_grouped_by_their_section() {
        let rows = rows(&Keymap::eli());
        let clear = row(&rows, "send:\u{c}", Some("terminal"));
        assert_eq!(clear.keys, ["ctrl-g ctrl-l"]);
        assert_eq!(clear.group, "terminal");
        assert_eq!(clear.title, "Send \\u{c}");
        let escape = row(&rows, "copy_mode.escape", Some("copy"));
        assert_eq!(escape.group, "copy mode");
        assert_eq!(escape.keys, ["escape"]);
    }

    #[test]
    fn rows_come_in_group_order_and_numbers_count() {
        let rows = rows(&Keymap::eli());
        let groups: Vec<usize> = rows
            .iter()
            .map(|r| GROUPS.iter().position(|g| *g == r.group).unwrap())
            .collect();
        assert!(groups.windows(2).all(|w| w[0] <= w[1]), "{groups:?}");
        let agents: Vec<&str> = rows
            .iter()
            .filter(|r| r.name.starts_with("agent.select:"))
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(agents.first(), Some(&"agent.select:1"));
        assert_eq!(agents.last(), Some(&"agent.select:10"));
    }

    #[test]
    fn typing_filters_by_every_word_over_titles_names_and_keys() {
        let rows = rows(&Keymap::eli());
        let names = |q: &str| -> Vec<String> {
            filtered(&rows, q)
                .into_iter()
                .map(|r| r.name.clone())
                .collect()
        };
        assert_eq!(names("ctrl-g /"), ["popup.agent-machines"]);
        assert_eq!(names("Space PICKER"), ["popup.space-picker"]);
        assert_eq!(names("split:right"), ["pane.split:right"]);
        // A key's start finds every key it begins.
        let prefix = names("ctrl-g s");
        assert!(prefix.contains(&"popup.space-picker".to_owned()));
        assert!(prefix.contains(&"tab.close".to_owned()), "ctrl-g shift-q");
        assert!(names("tab").len() > 10);
        assert!(names("nothing like this").is_empty());
        assert_eq!(filtered(&rows, "").len(), rows.len());
    }

    #[test]
    fn a_key_file_change_shows_in_the_rows() {
        let k = Keymap::from_json(
            r#"{"global": {"cmd-k": "tab.new"}, "terminal": {"ctrl-h": "terminal"}}"#,
        )
        .unwrap();
        let rows = rows(&k);
        assert_eq!(row(&rows, "tab.new", None).keys, ["cmd-k"]);
        // The built-in escape hatch is listed; the pass-through isn't.
        assert_eq!(
            row(&rows, "app.reset", None).keys,
            ["ctrl-alt-shift-escape"]
        );
        assert_eq!(rows.len(), 2);
    }
}
