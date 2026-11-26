//! The `herdr-app` layout document, one per machine, under herdr-app's exact schema
//! (`app:src/mirror.rs` `Document`, `app:src/layout.rs` `Node`; `herdr-core-protocol.md` §3).
//!
//! Lossless: each level types only what herdr-frontend reads and keeps every other field in
//! `extra`, as read. Node kinds and block contents it doesn't know survive too, so a newer
//! herdr-app's additions outlive herdr-frontend's writes. The fields herdr-app always writes
//! (`spaces`; a space's `id`, `name`, `folder`, `tabs`; a tab's `id`, `name`, `root`; a node's
//! `id`, `type`, and `children` or `content`) are always written, filled with herdr-app's defaults
//! when a document left them out, so a document herdr-app wrote encodes back to the same JSON.

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// Fields a level doesn't type, kept as read.
pub type Extra = Map<String, Value>;

/// The name of the layout document on each machine's core.
pub const DOCUMENT: &str = "herdr-app";

/// One machine's spaces, in their shared order. Ids and terminal addresses are unqualified.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub spaces: Vec<Space>,
    /// herdr-app drops these on its own writes; herdr-frontend keeps them.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A place to work.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Space {
    /// `sp<32 hex>`.
    pub id: String,
    /// Empty: named after its first terminal's checkout, per screen, never written.
    pub name: String,
    /// Where new terminals in the space start.
    pub folder: String,
    pub tabs: Vec<Tab>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A named piece of work in a space: its panes and how they are split.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tab {
    /// `tb<32 hex>`, also the root node's id.
    pub id: String,
    /// Empty: called by its position (1, 2, ...).
    pub name: String,
    pub root: Node,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Tab {
    /// herdr-app's default for a tab without a root: an empty row with an empty id.
    fn default() -> Self {
        Tab {
            id: String::new(),
            name: String::new(),
            root: Node::container("", Axis::Row, Vec::new()),
            extra: Extra::new(),
        }
    }
}

/// A row or column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    Row,
    Column,
}

/// A layout node: a block or a container. Its `size`, a stack's `active`, `floating`, looks and
/// every other field stay in `extra` (read them with the accessors).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub kind: Kind,
    pub extra: Extra,
}

/// A node's `type`, with what it holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Block {
        content: Content,
    },
    Row {
        children: Vec<Node>,
    },
    Column {
        children: Vec<Node>,
    },
    Stack {
        children: Vec<Node>,
    },
    /// A `type` this crate doesn't know. Its `children`, when they read as nodes, are walked like
    /// any container's (so a terminal inside is still found); everything else stays in `extra`.
    Other {
        kind: String,
        children: Option<Vec<Node>>,
    },
}

/// A block's `content`, kept whole. herdr-app's variants are untagged and tried in order:
/// `{terminal}`, `{mod, props?}`, `{tab, pane?}` (or 0.1.0's `{context}`), `{agent}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Content(pub Extra);

/// What a block shows, as herdr-app reads its content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shows<'a> {
    Terminal(&'a str),
    /// A herdr-app JS mod: drawn as a placeholder, never dropped or rewritten.
    Mod {
        name: &'a str,
        props: Option<&'a Value>,
    },
    /// A chrome layout's tab slot.
    Tab(&'a str),
    Agent(&'a str),
    /// None of herdr-app's variants.
    Other,
}

impl Content {
    /// A terminal block's content.
    pub fn terminal(address: impl Into<String>) -> Self {
        let mut map = Extra::new();
        map.insert("terminal".into(), Value::String(address.into()));
        Content(map)
    }

    /// A mod block's content. `props` are stored as given.
    pub fn mod_block(name: impl Into<String>, props: Option<Value>) -> Self {
        let mut map = Extra::new();
        map.insert("mod".into(), Value::String(name.into()));
        if let Some(props) = props.filter(|p| !p.is_null()) {
            map.insert("props".into(), props);
        }
        Content(map)
    }

    pub fn shows(&self) -> Shows<'_> {
        let text = |key: &str| self.0.get(key).and_then(Value::as_str);
        if let Some(address) = text("terminal") {
            Shows::Terminal(address)
        } else if let Some(name) = text("mod") {
            Shows::Mod {
                name,
                props: self.0.get("props").filter(|p| !p.is_null()),
            }
        } else if let Some(tab) = text("tab").or_else(|| text("context")) {
            Shows::Tab(tab)
        } else if let Some(agent) = text("agent") {
            Shows::Agent(agent)
        } else {
            Shows::Other
        }
    }

    /// The terminal address, when this is a terminal block.
    pub fn address(&self) -> Option<&str> {
        match self.shows() {
            Shows::Terminal(address) => Some(address),
            _ => None,
        }
    }

    /// Rewrites the terminal address of a terminal block; other contents are left alone.
    pub(crate) fn map_address(&mut self, map: &impl Fn(&str) -> String) {
        if let Some(Value::String(address)) = self.0.get_mut("terminal") {
            *address = map(address);
        }
    }
}

/// A node's `size`: herdr-app's untagged `{px}` (tried first) or `{weight}`, both `f32`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Px(f32),
    Weight(f32),
}

impl Size {
    /// How herdr-app writes it: through `serde_json::to_value`, which widens the `f32` to `f64`.
    fn to_value(self) -> Value {
        let (key, n) = match self {
            Size::Px(px) => ("px", px),
            Size::Weight(weight) => ("weight", weight),
        };
        let mut map = Extra::new();
        map.insert(key.into(), Value::from(n));
        Value::Object(map)
    }

    fn from_value(value: &Value) -> Option<Self> {
        let number = |key| value.get(key).and_then(Value::as_f64).map(|n| n as f32);
        number("px")
            .map(Size::Px)
            .or_else(|| number("weight").map(Size::Weight))
    }
}

impl Node {
    pub fn new(id: impl Into<String>, kind: Kind) -> Node {
        Node {
            id: id.into(),
            kind,
            extra: Extra::new(),
        }
    }

    pub fn block(id: impl Into<String>, content: Content) -> Node {
        Node::new(id, Kind::Block { content })
    }

    /// A terminal pane.
    pub fn pane(id: impl Into<String>, address: impl Into<String>) -> Node {
        Node::block(id, Content::terminal(address))
    }

    pub fn container(id: impl Into<String>, axis: Axis, children: Vec<Node>) -> Node {
        Node::new(
            id,
            match axis {
                Axis::Row => Kind::Row { children },
                Axis::Column => Kind::Column { children },
            },
        )
    }

    /// The `type` word.
    pub fn type_name(&self) -> &str {
        match &self.kind {
            Kind::Block { .. } => "block",
            Kind::Row { .. } => "row",
            Kind::Column { .. } => "column",
            Kind::Stack { .. } => "stack",
            Kind::Other { kind, .. } => kind,
        }
    }

    pub fn size(&self) -> Option<Size> {
        self.extra.get("size").and_then(Size::from_value)
    }

    pub fn set_size(&mut self, size: Option<Size>) {
        match size {
            Some(size) => self.extra.insert("size".into(), size.to_value()),
            None => self.extra.remove("size"),
        };
    }

    /// `hidden`, as herdr-app reads it (absent is false).
    pub fn hidden(&self) -> bool {
        self.extra.get("hidden").and_then(Value::as_bool) == Some(true)
    }

    /// A stack's selected child.
    pub fn active(&self) -> Option<&str> {
        match self.kind {
            Kind::Stack { .. } => self.extra.get("active").and_then(Value::as_str),
            _ => None,
        }
    }

    pub(crate) fn set_active(&mut self, active: Option<String>) {
        match active {
            Some(active) => self.extra.insert("active".into(), Value::String(active)),
            None => self.extra.remove("active"),
        };
    }

    fn from_map(mut map: Extra) -> Result<Node, String> {
        let id = match map.remove("id") {
            None => String::new(),
            Some(Value::String(id)) => id,
            Some(other) => return Err(format!("node id is not a string: {other}")),
        };
        let kind = match map.remove("type") {
            Some(Value::String(kind)) => kind,
            Some(other) => {
                return Err(format!(
                    "node {id:?} has a type that is not a string: {other}"
                ));
            }
            None => return Err(format!("node {id:?} has no type")),
        };
        let children = |map: &mut Extra| -> Result<Vec<Node>, String> {
            match map.remove("children") {
                Some(Value::Array(items)) => items.into_iter().map(Node::from_value).collect(),
                Some(other) => Err(format!("node {id:?} children are not a list: {other}")),
                None => Err(format!("node {id:?} has no children")),
            }
        };
        let kind = match kind.as_str() {
            "block" => match map.remove("content") {
                Some(Value::Object(content)) => Kind::Block {
                    content: Content(content),
                },
                Some(other) => {
                    return Err(format!("node {id:?} content is not an object: {other}"));
                }
                None => return Err(format!("block {id:?} has no content")),
            },
            "row" => Kind::Row {
                children: children(&mut map)?,
            },
            "column" => Kind::Column {
                children: children(&mut map)?,
            },
            "stack" => Kind::Stack {
                children: children(&mut map)?,
            },
            _ => {
                // Walk an unknown container's children only when every one reads as a node;
                // otherwise keep them as read.
                let nodes = match map.get("children") {
                    Some(Value::Array(items)) => items
                        .iter()
                        .map(|item| Node::from_value(item.clone()))
                        .collect::<Result<Vec<_>, _>>()
                        .ok(),
                    _ => None,
                };
                if nodes.is_some() {
                    map.remove("children");
                }
                Kind::Other {
                    kind,
                    children: nodes,
                }
            }
        };
        Ok(Node {
            id,
            kind,
            extra: map,
        })
    }

    fn from_value(value: Value) -> Result<Node, String> {
        match value {
            Value::Object(map) => Node::from_map(map),
            other => Err(format!("node is not an object: {other}")),
        }
    }

    fn to_map(&self) -> Extra {
        let mut map = self.extra.clone();
        map.insert("id".into(), Value::String(self.id.clone()));
        map.insert("type".into(), Value::String(self.type_name().to_owned()));
        let nodes = |children: &[Node]| {
            Value::Array(children.iter().map(|c| Value::Object(c.to_map())).collect())
        };
        match &self.kind {
            Kind::Block { content } => {
                map.insert("content".into(), Value::Object(content.0.clone()));
            }
            Kind::Row { children } | Kind::Column { children } | Kind::Stack { children } => {
                map.insert("children".into(), nodes(children));
            }
            Kind::Other {
                children: Some(children),
                ..
            } => {
                map.insert("children".into(), nodes(children));
            }
            Kind::Other { children: None, .. } => {}
        }
        map
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_map().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Node::from_map(Extra::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// Why a document can't be read. herdr-app marks such a machine's layout unavailable and stops
/// writing it; so does the `Mirror`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Undecodable(pub String);

impl fmt::Display for Undecodable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot read this machine's layout: {}", self.0)
    }
}

impl std::error::Error for Undecodable {}

impl Document {
    /// Reads a `doc.get` value. An unwritten document (`null`) is empty, as in herdr-app.
    pub fn from_value(value: Value) -> Result<Document, Undecodable> {
        if value.is_null() {
            return Ok(Document::default());
        }
        serde_json::from_value(value).map_err(|e| Undecodable(e.to_string()))
    }

    /// The value for `doc.set`.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("a document always encodes")
    }
}

/// Qualifies an id or address for a machine this core added: `devbox/sp…`. This machine's stay
/// bare (`app:src/mirror.rs` `qualify`).
pub fn qualify(id: &str, machine: Option<&str>) -> String {
    machine.map_or_else(|| id.to_owned(), |m| format!("{m}/{id}"))
}

/// The id on its own machine: `devbox/sp…` → `sp…`. An id without that machine's prefix is
/// returned as it is (`app:src/mirror.rs` `local`).
pub fn local<'a>(id: &'a str, machine: Option<&str>) -> &'a str {
    machine
        .and_then(|m| id.strip_prefix(m).and_then(|s| s.strip_prefix('/')))
        .unwrap_or(id)
}

/// The machine of a qualified address or id: `devbox/t4` → `devbox`, `t4` → this one.
pub fn machine_of(id: &str) -> Option<&str> {
    id.split_once('/').map(|(m, _)| m)
}

/// A fresh id: the prefix (`sp`, `tb`, `p`, `s`) and 128 random bits in hex. Never a per-screen
/// counter, since two screens make ids for the same document (`app:src/model.rs` `fresh_id`).
pub fn fresh_id(prefix: &str) -> String {
    use std::hash::{BuildHasher, Hasher};
    let random = || {
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    };
    format!("{prefix}{:016x}{:016x}", random(), random())
}

/// A fresh id, qualified for its machine.
pub fn fresh_id_on(prefix: &str, machine: Option<&str>) -> String {
    qualify(&fresh_id(prefix), machine)
}

/// Rewrites a node's identity: its id, a terminal block's address, a stack's `active`, and its
/// floating blocks' ids and addresses. Mod props are stored as given (`app:src/mirror.rs`
/// `map_node`).
fn map_node(node: &mut Node, map: &impl Fn(&str) -> String) {
    node.id = map(&node.id);
    if let Kind::Block { content } = &mut node.kind {
        content.map_address(map);
    }
    if let Kind::Stack { .. } = node.kind
        && let Some(Value::String(active)) = node.extra.get_mut("active")
    {
        *active = map(active);
    }
    if let Some(Value::Array(floats)) = node.extra.get_mut("floating") {
        for float in floats.iter_mut().filter_map(Value::as_object_mut) {
            if let Some(Value::String(id)) = float.get_mut("id") {
                *id = map(id);
            }
            if let Some(Value::Object(content)) = float.get_mut("content")
                && let Some(Value::String(address)) = content.get_mut("terminal")
            {
                *address = map(address);
            }
        }
    }
    for child in node.children_mut().into_iter().flatten() {
        map_node(child, map);
    }
}

impl Document {
    fn mapped(&self, map: impl Fn(&str) -> String) -> Document {
        let mut doc = self.clone();
        for space in &mut doc.spaces {
            space.id = map(&space.id);
            for tab in &mut space.tabs {
                tab.id = map(&tab.id);
                map_node(&mut tab.root, &map);
            }
        }
        doc
    }

    /// As the app holds it: ids and addresses qualified for `machine` (`None`: this one).
    pub fn qualified(&self, machine: Option<&str>) -> Document {
        if machine.is_none() {
            return self.clone();
        }
        self.mapped(|id| qualify(id, machine))
    }

    /// As the machine's core stores it: the `machine/` prefixes taken off again.
    pub fn localized(&self, machine: Option<&str>) -> Document {
        if machine.is_none() {
            return self.clone();
        }
        self.mapped(|id| local(id, machine).to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip(value: Value) {
        let doc = Document::from_value(value.clone()).unwrap();
        assert_eq!(doc.to_value(), value);
        // The core stores documents with sorted keys, as `Value` prints them.
        assert_eq!(
            serde_json::to_string(&doc.to_value()).unwrap(),
            serde_json::to_string(&value).unwrap()
        );
    }

    fn eli_tab() -> Value {
        json!({"id": "tb1", "name": "", "root": {"id": "tb1", "type": "row", "children": [
            {"id": "p1", "type": "block", "content": {"terminal": "t6p"}},
            {"id": "s1", "type": "column", "size": {"weight": 1.0}, "children": [
                {"id": "p2", "type": "block", "content": {"terminal": "t6q"}}]}]}})
    }

    #[test]
    fn a_document_herdr_app_wrote_encodes_back_to_the_same_json() {
        round_trip(
            json!({"spaces": [{"id": "sp1", "name": "", "folder": "/Users/me", "tabs": [eli_tab()]}]}),
        );
    }

    #[test]
    fn unknown_fields_survive_at_every_level() {
        round_trip(
            json!({"later": 1, "spaces": [{"id": "sp1", "name": "work", "folder": "/w", "color": "teal",
            "tabs": [{"id": "tb1", "name": "", "pinned": [1, 2],
                "floating": [{"pane": {"id": "q", "type": "block", "content": {"terminal": "t2"}}, "rect": [0.2, 0.2, 0.6, 0.6]}],
                "root": {"id": "tb1", "type": "row", "gap": 4, "brand_new": {"x": null},
                    "children": [{"id": "p1", "type": "block", "title": "x", "hidden": false,
                        "content": {"terminal": "t1", "unknown": true}}]}}]}]}),
        );
    }

    #[test]
    fn unknown_kinds_and_contents_survive() {
        round_trip(
            json!({"spaces": [{"id": "sp1", "name": "", "folder": "", "tabs": [{"id": "tb1", "name": "", "root":
            {"id": "tb1", "type": "row", "children": [
                {"id": "g1", "type": "grid", "cols": 2, "children": [
                    {"id": "p1", "type": "block", "content": {"terminal": "t1"}}]},
                {"id": "w1", "type": "web", "url": "https://example.com", "children": "not nodes"},
                {"id": "w2", "type": "canvas", "children": [{"no": "type"}]},
                {"id": "m1", "type": "block", "content": {"mod": "board", "props": {"terminal": "t9", "deep": [1.5]}}},
                {"id": "x1", "type": "block", "content": {"browser": "https://example.com"}},
                {"id": "a1", "type": "block", "content": {"agent": "focused"}},
                {"id": "c1", "type": "block", "content": {"context": "active"}}]}}]}]}),
        );
        let doc = Document::from_value(json!({"spaces": [{"id": "s", "tabs": [{"id": "t", "root":
            {"id": "t", "type": "row", "children": [{"id": "g", "type": "grid", "children": [
                {"id": "p", "type": "block", "content": {"terminal": "t1"}}]}]}}]}]}))
        .unwrap();
        assert_eq!(
            doc.pane_of("t1").as_deref(),
            Some("p"),
            "terminals inside unknown containers are found"
        );
    }

    #[test]
    fn numbers_keep_their_exact_text() {
        // herdr-app writes f32 sizes widened to f64; integers stay integers.
        round_trip(
            json!({"spaces": [{"id": "s", "name": "", "folder": "", "tabs": [{"id": "t", "name": "", "root":
            {"id": "t", "type": "row", "children": [
                {"id": "a", "type": "block", "size": {"weight": 0.3333333432674408}, "min": 120, "content": {"terminal": "t1"}},
                {"id": "b", "type": "block", "size": {"px": 260}, "content": {"terminal": "t2"}},
                {"id": "c", "type": "block", "size": {"weight": 1}, "content": {"terminal": "t3"}}]}}]}]}),
        );
    }

    #[test]
    fn strings_survive_escapes_and_unicode() {
        round_trip(
            json!({"spaces": [{"id": "s", "name": "naïve \"quotes\" \\ 🦀 \u{0001}", "folder": "/tmp/ä b", "tabs": []}]}),
        );
    }

    #[test]
    fn omitted_shared_fields_take_herdr_apps_defaults() {
        let doc =
            Document::from_value(json!({"spaces": [{"id": "s", "tabs": [{"id": "t"}]}]})).unwrap();
        assert_eq!(doc.spaces[0].name, "");
        assert_eq!(doc.spaces[0].folder, "");
        assert!(doc.spaces[0].tabs[0].root.children().is_empty());
        // Written as herdr-app writes it.
        assert_eq!(
            doc.to_value(),
            json!({"spaces": [{"id": "s", "name": "", "folder": "", "tabs": [{"id": "t", "name": "",
                "root": {"id": "", "type": "row", "children": []}}]}]})
        );
        assert_eq!(
            Document::from_value(json!({})).unwrap(),
            Document::default()
        );
        assert_eq!(
            Document::from_value(Value::Null).unwrap(),
            Document::default()
        );
        assert_eq!(Document::default().to_value(), json!({"spaces": []}));
    }

    #[test]
    fn malformed_documents_are_undecodable() {
        for bad in [
            json!([]),
            json!({"spaces": {}}),
            json!({"spaces": [{"id": 5}]}),
            json!({"spaces": [{"id": "s", "tabs": [{"id": "t", "root": {"id": "t", "children": []}}]}]}),
            json!({"spaces": [{"id": "s", "tabs": [{"id": "t", "root": {"id": "t", "type": "row", "children": [1]}}]}]}),
            json!({"spaces": [{"id": "s", "tabs": [{"id": "t", "root": {"id": "t", "type": "block"}}]}]}),
            json!({"spaces": [{"id": "s", "tabs": [{"id": "t", "root": {"id": "t", "type": "row"}}]}]}),
        ] {
            assert!(Document::from_value(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn contents_read_in_herdr_apps_order() {
        let content = |v: Value| Content(v.as_object().unwrap().clone());
        assert_eq!(
            content(json!({"terminal": "t1", "mod": "x"})).shows(),
            Shows::Terminal("t1")
        );
        assert_eq!(
            content(json!({"mod": "x", "props": {"a": 1}})).shows(),
            Shows::Mod {
                name: "x",
                props: Some(&json!({"a": 1}))
            }
        );
        assert_eq!(
            content(json!({"mod": "x", "props": null})).shows(),
            Shows::Mod {
                name: "x",
                props: None
            }
        );
        assert_eq!(
            content(json!({"tab": "active"})).shows(),
            Shows::Tab("active")
        );
        assert_eq!(
            content(json!({"context": "active"})).shows(),
            Shows::Tab("active")
        );
        assert_eq!(
            content(json!({"agent": "focused"})).shows(),
            Shows::Agent("focused")
        );
        assert_eq!(content(json!({"terminal": 5})).shows(), Shows::Other);
        assert_eq!(
            Content::mod_block("x", Some(Value::Null)).0,
            *json!({"mod": "x"}).as_object().unwrap()
        );
    }

    #[test]
    fn sizes_read_px_first_and_write_as_herdr_app_does() {
        let mut node = Node::pane("p", "t1");
        assert_eq!(node.size(), None);
        node.extra
            .insert("size".into(), json!({"px": 260, "weight": 2}));
        assert_eq!(node.size(), Some(Size::Px(260.0)));
        node.set_size(Some(Size::Weight(1.0 / 3.0)));
        // f32 → f64, as `serde_json::to_value` widens herdr-app's f32.
        assert_eq!(node.extra["size"], json!({"weight": 0.3333333432674408}));
        node.set_size(None);
        assert!(!node.extra.contains_key("size"));
    }

    #[test]
    fn remote_documents_qualify_identity_but_not_mod_props() {
        let raw = json!({"spaces": [{"id": "sp1", "name": "", "folder": "", "tabs": [{"id": "tb1", "name": "", "root":
            {"id": "tb1", "type": "row", "children": [
                {"id": "p1", "type": "block", "content": {"terminal": "t1"},
                    "floating": [{"id": "f1", "content": {"terminal": "t2"}, "edge": "left"}]},
                {"id": "k1", "type": "stack", "active": "p3", "children": [
                    {"id": "p3", "type": "block", "content": {"mod": "board", "props": {"terminal": "t1"}}}]}]}}]}]});
        let doc = Document::from_value(raw.clone()).unwrap();
        let mac = doc.qualified(Some("mac"));
        let v = mac.to_value();
        assert_eq!(v["spaces"][0]["id"], "mac/sp1");
        assert_eq!(v["spaces"][0]["tabs"][0]["id"], "mac/tb1");
        let root = &v["spaces"][0]["tabs"][0]["root"];
        assert_eq!(root["id"], "mac/tb1");
        assert_eq!(root["children"][0]["content"]["terminal"], "mac/t1");
        assert_eq!(root["children"][0]["floating"][0]["id"], "mac/f1");
        assert_eq!(
            root["children"][0]["floating"][0]["content"]["terminal"],
            "mac/t2"
        );
        assert_eq!(root["children"][1]["active"], "mac/p3");
        assert_eq!(
            root["children"][1]["children"][0]["content"]["props"]["terminal"],
            "t1"
        );
        assert_eq!(mac.localized(Some("mac")).to_value(), raw);
        assert_eq!(doc.qualified(None), doc);
        assert_eq!(doc.localized(None), doc);
    }

    #[test]
    fn qualify_and_local_follow_herdr_app() {
        assert_eq!(qualify("t1", None), "t1");
        assert_eq!(qualify("t1", Some("mac")), "mac/t1");
        assert_eq!(local("mac/t1", Some("mac")), "t1");
        assert_eq!(local("t1", Some("mac")), "t1");
        assert_eq!(local("macx/t1", Some("mac")), "macx/t1");
        assert_eq!(local("mac/t1", None), "mac/t1");
        assert_eq!(machine_of("mac/t1"), Some("mac"));
        assert_eq!(machine_of("t1"), None);
    }

    #[test]
    fn fresh_ids_are_prefixed_random_128_bits() {
        let a = fresh_id("sp");
        assert_eq!(a.len(), 2 + 32);
        assert!(a.starts_with("sp") && a[2..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(fresh_id("p"), fresh_id("p"));
        assert!(fresh_id_on("tb", Some("mac")).starts_with("mac/tb"));
    }
}
