//! herdr-frontend's app-local shared state (`feature-inventory.md` §0 "App-local shared state"):
//! what Herdr app keeps in its `shared.json`, here in herdr-frontend's own home. Keys keep Herdr
//! app's names, so the values mean the same: `space.colors`, `agents.sort`,
//! `dotfiles.agents.order`, `dotfiles.agents.hidden-machines` and the rest, and two of
//! herdr-frontend's own for the sidebar, `sidebar.px` and `sidebar.hidden` (what Herdr app keeps
//! in its layout's state, `layouts/.state/<layout>.json` `nodes.sidebar`; its own `sidebar.width`
//! is the narrow rail's way back, which herdr-frontend doesn't have).
//!
//! On the first run (no `shared.json` in the home) Eli's current values come from Herdr app's
//! home, read only: its `shared.json`, and the sidebar's width and visibility from its layout's
//! state. The Agents list's times come along the same way, from the `eli-agents` mod's store, into
//! `agents.json`. Herdr app's files are never written.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use model::agents::{self, Changed};
use serde_json::{Map, Value, json};

use crate::home::Home;

/// The shared state's file in the home.
pub const SHARED: &str = "shared.json";
/// The Agents list's times in the home (`eli-agents`' `changed`).
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

/// What [`Shared::load`] found.
#[derive(Clone, Debug, PartialEq)]
pub enum Loaded {
    /// The home's own file.
    Home,
    /// Imported from Herdr app's home: the home has no file yet, so this is written there.
    Imported,
    /// Nothing anywhere: the defaults.
    Defaults,
}

fn object(text: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str(text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

impl Shared {
    /// The home's `shared.json`, else Herdr app's values from `herdr_app`, else nothing.
    pub fn load(home: Option<&Path>, herdr_app: Option<&Path>) -> (Shared, Loaded) {
        if let Some(home) = home
            && let Ok(text) = std::fs::read_to_string(home.join(SHARED))
        {
            return (
                Shared {
                    values: object(&text).unwrap_or_default(),
                },
                Loaded::Home,
            );
        }
        let Some(app) = herdr_app else {
            return (Shared::default(), Loaded::Defaults);
        };
        let file = std::fs::read_to_string(app.join(SHARED))
            .ok()
            .and_then(|t| object(&t));
        let Some(mut values) = file else {
            return (Shared::default(), Loaded::Defaults);
        };
        let layout = values
            .get("layout")
            .and_then(Value::as_str)
            .unwrap_or("eli")
            .to_owned();
        let state =
            std::fs::read_to_string(app.join("layouts/.state").join(format!("{layout}.json")))
                .ok()
                .and_then(|t| object(&t));
        if let Some(sidebar) = state
            .as_ref()
            .and_then(|s| s.get("nodes"))
            .and_then(|n| n.get("sidebar"))
        {
            if let Some(px) = sidebar.pointer("/size/px").and_then(Value::as_f64) {
                values.insert(SIDEBAR_PX_KEY.into(), json!(px));
            }
            if let Some(hidden) = sidebar.get("hidden").and_then(Value::as_bool) {
                values.insert(SIDEBAR_HIDDEN_KEY.into(), json!(hidden));
            }
        }
        (Shared { values }, Loaded::Imported)
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

/// The Agents list's times: the home's `agents.json`, else the `eli-agents` mod's store in Herdr
/// app's home (`data/eli-agents/store.json`, whose `changed` is a JSON string), else none. True
/// when imported, so they are written to the home.
pub fn load_times(
    home: Option<&Path>,
    herdr_app: Option<&Path>,
) -> (BTreeMap<String, Changed>, bool) {
    let parse = |v: &Value| serde_json::from_value::<BTreeMap<String, Changed>>(v.clone()).ok();
    if let Some(home) = home
        && let Ok(text) = std::fs::read_to_string(home.join(TIMES))
    {
        let times = object(&text)
            .and_then(|o| o.get("changed").and_then(parse))
            .unwrap_or_default();
        return (times, false);
    }
    let imported = herdr_app
        .and_then(|app| std::fs::read_to_string(app.join("data/eli-agents/store.json")).ok())
        .and_then(|text| object(&text))
        .and_then(|store| {
            let changed = store.get("changed")?;
            match changed {
                Value::String(inner) => serde_json::from_str::<Value>(inner)
                    .ok()
                    .and_then(|v| parse(&v)),
                other => parse(other),
            }
        });
    match imported {
        Some(times) => (times, true),
        None => (BTreeMap::new(), false),
    }
}

/// `agents.json`'s text.
pub fn times_json(times: &BTreeMap<String, Changed>) -> String {
    serde_json::to_string_pretty(&json!({ "changed": times })).expect("times always serialize")
}

/// Writes the shared state to the home in the background.
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
    fn the_first_run_imports_herdr_apps_values_and_its_sidebar() {
        let dir = scratch("import");
        let (home, app) = (dir.join("home"), dir.join("herdr-app"));
        std::fs::create_dir_all(app.join("layouts/.state")).unwrap();
        std::fs::create_dir_all(app.join("data/eli-agents")).unwrap();
        std::fs::write(
            app.join(SHARED),
            r##"{"layout":"eli","agents.sort":"grouped","sidebar.width":268,
                "dotfiles.agents.hidden-machines":["devbox",""],
                "space.colors":{"sp1":"ansi6","sp2":"#123456"}}"##,
        )
        .unwrap();
        std::fs::write(
            app.join("layouts/.state/eli.json"),
            r#"{"nodes":{"sidebar":{"size":{"px":420.0},"hidden":true}}}"#,
        )
        .unwrap();
        std::fs::write(
            app.join("data/eli-agents/store.json"),
            r#"{"changed":"{\"t1\":{\"s\":\"idle\",\"at\":1794116789525}}"}"#,
        )
        .unwrap();
        let (shared, loaded) = Shared::load(Some(&home), Some(&app));
        assert_eq!(loaded, Loaded::Imported);
        assert_eq!(shared.sort(), agents::Sort::Grouped);
        assert_eq!(
            shared.hidden_machines(),
            ["".to_owned(), "devbox".to_owned()].into()
        );
        assert_eq!(shared.space_colors()["sp2"], "#123456");
        assert_eq!(
            shared.sidebar_px(),
            420.0,
            "the layout's, not the rail's 268"
        );
        assert!(shared.sidebar_hidden());
        let (times, imported) = load_times(Some(&home), Some(&app));
        assert!(imported);
        assert_eq!(times["t1"].at, 1794116789525);

        // Once the home has its own, Herdr app's files are not read again.
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(SHARED), r#"{"sidebar.px": 9000}"#).unwrap();
        std::fs::write(home.join(TIMES), times_json(&BTreeMap::new())).unwrap();
        let (shared, loaded) = Shared::load(Some(&home), Some(&app));
        assert_eq!(loaded, Loaded::Home);
        assert_eq!(shared.sort(), agents::Sort::Priority);
        assert_eq!(shared.sidebar_px(), SIDEBAR_MAX, "kept in range");
        assert!(!shared.sidebar_hidden());
        assert_eq!(
            load_times(Some(&home), Some(&app)),
            (BTreeMap::new(), false)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_anywhere_is_the_defaults() {
        let dir = scratch("none");
        let (shared, loaded) = Shared::load(Some(&dir.join("home")), Some(&dir.join("app")));
        assert_eq!(loaded, Loaded::Defaults);
        assert_eq!(shared.sidebar_px(), SIDEBAR_PX);
        assert!(shared.hidden_machines().is_empty());
        let mut shared = shared;
        assert!(shared.set("agents.sort", json!("grouped")));
        assert!(!shared.set("agents.sort", json!("grouped")));
        assert_eq!(shared.sort(), agents::Sort::Grouped);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
