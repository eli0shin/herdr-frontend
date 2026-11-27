//! The new-space folder picker (`cmd-n`, `ctrl-g ctrl-s`; Eli's `new-space` mod,
//! `feature-inventory.md` §11): 640 wide, at most 0.6 of the window, 40 from the top, titled "New
//! space" (or "New space on <machine>") with a "New space in…" field. Rows: the folder's name
//! (semibold), its parent (dim, `~` for home) and its tag ("here", "repo", "path"). Twelve show at
//! once, with "↓ N more"; "Loading folders…" until the space's machine answers.
//!
//! The folders and their ranking are [`model::folders`]; Enter makes the space there
//! ([`model::Action::NewSpaceIn`]).

use model::folders::Folder;
use model::mode::ListPopup;
use rune::{El, col, row, text};

use crate::State;
use crate::view::list::{self, Item, More};
use crate::view::overlay::{self, Place};

pub const PLACE: Place = Place::Top {
    offset: 40.0,
    width: 640.0,
    height: 0.6,
};

/// Rows shown at once.
pub const SHOWN: usize = 12;

pub fn rows(s: &State, list: &ListPopup) -> Vec<Folder> {
    model::popups::new_space_rows(&s.model, &list.query)
}

pub fn view(s: &State, list: &ListPopup) -> El {
    let c = &s.chrome;
    let picker = s.model.folder_picker();
    let home = picker.and_then(|p| p.home.as_deref());
    let loading = picker.is_some_and(|p| p.loading());
    let rows = rows(s, list);
    let note = |key: &str, t: &str| overlay::note(s, t).key(key).px(14.0).h(22.0).items_center();
    let items: Vec<Item> = rows
        .iter()
        .enumerate()
        .map(|(i, folder)| {
            let (name, parent) = folder.parts(home);
            let small = |t: String| text(t).font(s.ui_font(11.0, 400.0)).color(c.dim);
            Item::row(
                row()
                    .key(format!("folder-{i}"))
                    .mx(4.0)
                    .px(10.0)
                    .h(32.0)
                    .gap(10.0)
                    .items_center()
                    .radius(7.0)
                    .fill(if i == list.cursor {
                        c.selected
                    } else {
                        c.popover
                    })
                    .child(
                        text(name)
                            .key(format!("folder-name-{i}"))
                            .font(s.ui_font(13.0, 600.0))
                            .color(c.foreground),
                    )
                    .child(
                        small(parent)
                            .key(format!("folder-parent-{i}"))
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .children(
                        folder
                            .source
                            .tag()
                            .map(|t| small(t.into()).key(format!("folder-tag-{i}"))),
                    ),
                i,
            )
        })
        .collect();
    let title = match picker.and_then(|p| p.machine.as_deref()) {
        Some(machine) => format!("New space on {machine}"),
        None => "New space".into(),
    };
    let loading_note = loading.then(|| note("folders-loading", "Loading folders…"));
    let empty = (!loading && rows.is_empty()).then(|| {
        note(
            "folders-empty",
            "No folder matches; Enter starts where you are",
        )
    });
    col()
        .w_full()
        .min_h(0.0)
        .padding(14.0)
        .gap(8.0)
        .child(overlay::title(
            s,
            "new-space-title",
            &title,
            Some(overlay::note(s, "↑/↓ · Enter · Esc")),
        ))
        .child(overlay::field(s, "New space in…", None, None))
        .child(
            col()
                .w_full()
                .min_h(0.0)
                .child(list::view(s, "folder-rows", items, SHOWN, More::Below))
                .children(loading_note)
                .children(empty),
        )
}
