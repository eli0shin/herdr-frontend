//! herdr-frontend's app-local shared state (`feature-inventory.md` §0 "App-local shared state"),
//! in its state folder (`crate::home`): the Agents list's sort, published order, space colors
//! and hidden machines, and the sidebar's width and visibility. The keys have the names Eli's mods gave them
//! (`space.colors`, `agents.sort`, `dotfiles.agents.order`, `dotfiles.agents.hidden-machines`),
//! plus `sidebar.px` and `sidebar.hidden`. Everything starts from the defaults and lives only
//! here: herdr-frontend is a client of herdr-core and reads nothing of Herdr app's (`AGENTS.md`).
//! The Agents list's times are in `agents.json` beside it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use model::agents::{self, Changed};
use serde_json::{Map, Value, json};

use crate::home::Home;

/// The shared state's file in the state folder.
pub const SHARED: &str = "shared.json";
/// The Agents list's times in the state folder (`eli-agents`' `changed`).
pub const TIMES: &str = "agents.json";

/// The sidebar's width, as Eli's saved layout has it (`feature-inventory.md` §1).
pub const SIDEBAR_PX: f32 = 366.0;
/// The range a divider drag keeps it in (`cfg/layouts/eli.json` `min`, `max`).
pub const SIDEBAR_MIN: f32 = 200.0;
pub const SIDEBAR_MAX: f32 = 600.0;
pub const SIDEBAR_PX_KEY: &str = "sidebar.px";
pub const SIDEBAR_HIDDEN_KEY: &str = "sidebar.hidden";

/// The values, as JSON, by key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shared {
    values: Map<String, Value>,
}

fn object(text: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str(text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

impl Shared {
    /// The state folder's `shared.json`; the defaults when there is none (or no home).
    pub fn load(home: Option<&Path>) -> Shared {
        let values = home
            .and_then(|home| std::fs::read_to_string(home.join(SHARED)).ok())
            .and_then(|text| object(&text))
            .unwrap_or_default();
        Shared { values }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// Sets a key (`null` too, as Herdr app's `setShared` keeps it). True when it changed.
    pub fn set(&mut self, key: &str, value: Value) -> bool {
        if self.values.get(key) == Some(&value) {
            return false;
        }
        self.values.insert(key.to_owned(), value);
        true
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.values).expect("JSON values always serialize")
    }

    /// How the Agents list sorts.
    pub fn sort(&self) -> agents::Sort {
        agents::Sort::parse(self.str(agents::SORT_KEY))
    }

    /// The machines whose agents the list leaves out (`""` is this computer).
    pub fn hidden_machines(&self) -> BTreeSet<String> {
        self.get(agents::HIDDEN_MACHINES_KEY)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Each space's color, by id.
    pub fn space_colors(&self) -> BTreeMap<String, String> {
        self.get(agents::SPACE_COLORS_KEY)
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The sidebar's width, kept in range.
    pub fn sidebar_px(&self) -> f32 {
        self.get(SIDEBAR_PX_KEY)
            .and_then(Value::as_f64)
            .map_or(SIDEBAR_PX, |px| clamp_sidebar(px as f32))
    }

    pub fn sidebar_hidden(&self) -> bool {
        self.get(SIDEBAR_HIDDEN_KEY)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

/// A sidebar width in its range.
pub fn clamp_sidebar(px: f32) -> f32 {
    if px.is_finite() {
        px.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
    } else {
        SIDEBAR_PX
    }
}

/// The Agents list's times: the state folder's `agents.json`, else none.
pub fn load_times(home: Option<&Path>) -> BTreeMap<String, Changed> {
    home.and_then(|home| std::fs::read_to_string(home.join(TIMES)).ok())
        .and_then(|text| object(&text))
        .and_then(|o| serde_json::from_value(o.get("changed")?.clone()).ok())
        .unwrap_or_default()
}

/// `agents.json`'s text.
pub fn times_json(times: &BTreeMap<String, Changed>) -> String {
    serde_json::to_string_pretty(&json!({ "changed": times })).expect("times always serialize")
}

/// Writes the shared state to the state folder in the background.
pub fn save(home: Option<&Home>, shared: &Shared) {
    if let Some(home) = home {
        home.save(SHARED, shared.to_json());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-frontend-shared-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_homes_files_are_read_and_kept_in_range() {
        let dir = scratch("home");
        std::fs::write(
            dir.join(SHARED),
            r##"{"agents.sort": "grouped", "sidebar.px": 9000, "sidebar.hidden": true,
                "dotfiles.agents.hidden-machines": ["devbox", ""],
                "space.colors": {"sp1": "ansi6", "sp2": "#123456"}}"##,
        )
        .unwrap();
        let shared = Shared::load(Some(&dir));
        assert_eq!(shared.sort(), agents::Sort::Grouped);
        assert_eq!(
            shared.hidden_machines(),
            ["".to_owned(), "devbox".to_owned()].into()
        );
        assert_eq!(shared.space_colors()["sp2"], "#123456");
        assert_eq!(shared.sidebar_px(), SIDEBAR_MAX, "kept in range");
        assert!(shared.sidebar_hidden());
        let mut times = BTreeMap::new();
        times.insert(
            "t1".to_owned(),
            Changed {
                s: "idle".into(),
                at: 1794116789525,
            },
        );
        std::fs::write(dir.join(TIMES), times_json(&times)).unwrap();
        assert_eq!(load_times(Some(&dir)), times);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_anywhere_is_the_defaults() {
        let dir = scratch("none");
        let shared = Shared::load(Some(&dir.join("home")));
        assert_eq!(shared, Shared::load(None));
        assert_eq!(shared.sort(), agents::Sort::Priority);
        assert_eq!(shared.sidebar_px(), SIDEBAR_PX);
        assert!(!shared.sidebar_hidden());
        assert!(shared.space_colors().is_empty());
        assert!(load_times(Some(&dir.join("home"))).is_empty());
        assert!(shared.hidden_machines().is_empty());
        let mut shared = shared;
        assert!(shared.set("agents.sort", json!("grouped")));
        assert!(!shared.set("agents.sort", json!("grouped")));
        assert_eq!(shared.sort(), agents::Sort::Grouped);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
