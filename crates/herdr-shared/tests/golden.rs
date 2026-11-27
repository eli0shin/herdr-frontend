//! Real `herdr-app` documents, read from the live core by `scripts/record_shared_fixtures.py`,
//! each byte for byte as the core prints it (only names and folders replaced).

use std::path::PathBuf;

use herdr_proto::methods::DocGetResult;
use herdr_shared::{Document, Intent, Mirror, Round};
use serde_json::Value;

fn fixtures() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/documents");
    let mut found: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                text.trim_end().to_owned(),
            )
        })
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no fixtures in {}", dir.display());
    found
}

fn decode(text: &str) -> Document {
    Document::from_value(serde_json::from_str(text).unwrap()).unwrap()
}

#[test]
fn decode_and_encode_give_back_the_cores_exact_bytes() {
    for (name, text) in fixtures() {
        let doc = decode(&text);
        let again = serde_json::to_string(&doc.to_value()).unwrap();
        assert_eq!(again, text, "{name}");
        assert!(
            !doc.spaces.is_empty(),
            "{name}: an empty document proves little"
        );
    }
}

#[test]
fn qualifying_for_a_machine_and_back_changes_nothing() {
    for (name, text) in fixtures() {
        let doc = decode(&text);
        let remote = doc.qualified(Some("devbox"));
        for address in remote.addresses() {
            assert!(address.starts_with("devbox/"), "{name}: {address}");
            assert!(remote.pane_of(&address).is_some(), "{name}: {address}");
        }
        let back = remote.localized(Some("devbox"));
        assert_eq!(
            serde_json::to_string(&back.to_value()).unwrap(),
            text,
            "{name}"
        );
    }
}

#[test]
fn every_tab_root_carries_its_tabs_id() {
    for (name, text) in fixtures() {
        for tab in decode(&text).tabs() {
            assert_eq!(tab.root.id, tab.id, "{name}");
            assert!(!tab.root.panes().is_empty(), "{name}: {}", tab.id);
        }
    }
}

fn mirror_of(text: &str) -> Mirror {
    let mut mirror = Mirror::new(None);
    assert!(matches!(mirror.next_round(), Some(Round::Read(_))));
    mirror.finish_read(Ok(DocGetResult {
        value: serde_json::from_str(text).unwrap(),
        version: 1,
        ..Default::default()
    }));
    mirror
}

#[test]
fn reading_a_real_document_never_writes_it() {
    for (name, text) in fixtures() {
        let mut mirror = mirror_of(&text);
        assert_eq!(mirror.next_round(), None, "{name}");
    }
}

#[test]
fn an_edit_writes_only_what_it_changed() {
    for (name, text) in fixtures() {
        let mut mirror = mirror_of(&text);
        let tab = decode(&text).spaces[0].tabs[0].id.clone();
        mirror
            .edit(Intent::RenameTab {
                tab: tab.clone(),
                name: "renamed".into(),
            })
            .unwrap();
        let Some(Round::Write(set)) = mirror.next_round() else {
            panic!("{name}: expected a write");
        };
        let mut expected: Value = serde_json::from_str(&text).unwrap();
        expected["spaces"][0]["tabs"][0]["name"] = "renamed".into();
        assert_eq!(
            serde_json::to_string(&set.value).unwrap(),
            serde_json::to_string(&expected).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn a_pane_rename_writes_only_its_title_and_clearing_it_gives_the_document_back() {
    for (name, text) in fixtures() {
        let mut mirror = mirror_of(&text);
        let pane = decode(&text).tabs().next().unwrap().root.panes()[0]
            .id
            .clone();
        mirror
            .edit(Intent::RenamePane {
                pane: pane.clone(),
                name: "server".into(),
            })
            .unwrap();
        let Some(Round::Write(set)) = mirror.next_round() else {
            panic!("{name}: expected a write");
        };
        let mut expected: Value = serde_json::from_str(&text).unwrap();
        let mut named = false;
        visit(&mut expected, &mut |node| {
            if node["id"] == pane.as_str() && node["type"] == "block" {
                node["title"] = "server".into();
                named = true;
            }
        });
        assert!(named, "{name}");
        assert_eq!(
            serde_json::to_string(&set.value).unwrap(),
            serde_json::to_string(&expected).unwrap(),
            "{name}"
        );

        // Unnamed again, the bytes are the core's own.
        let mut doc = Document::from_value(set.value).unwrap();
        assert!(doc.apply(&Intent::RenamePane {
            pane,
            name: String::new(),
        }));
        assert_eq!(
            serde_json::to_string(&doc.to_value()).unwrap(),
            text,
            "{name}"
        );
    }
}

/// Every layout node of a raw document.
fn visit(value: &mut Value, f: &mut impl FnMut(&mut Value)) {
    for space in value["spaces"].as_array_mut().into_iter().flatten() {
        for tab in space["tabs"].as_array_mut().into_iter().flatten() {
            visit_node(&mut tab["root"], f);
        }
    }
}

fn visit_node(node: &mut Value, f: &mut impl FnMut(&mut Value)) {
    f(node);
    let children = node.get_mut("children").and_then(Value::as_array_mut);
    for child in children.into_iter().flatten() {
        visit_node(child, f);
    }
}
