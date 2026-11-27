//! The settings dialog (`cmd-,`, `ctrl-g c`, the palette's "Settings"). herdr-frontend is set up
//! by files (`docs/settings.md`, `docs/keys.md`), so this lists them instead of editing values:
//! each file of the home with its path, what is wrong with the files now, and a row that shows
//! the home folder in Finder. Enter (or a click) on a file opens it in a new tab running
//! `$EDITOR`, else Neovim ([`model::Action::EditFile`]); the window follows the saved file live.

use std::path::PathBuf;

use model::mode::ListPopup;
use rune::{Cx, El, col, row, text};

use crate::State;
use crate::config::{PROGRAMS, SETTINGS, THEMES};
use crate::view::list::{self, Item};
use crate::view::overlay::{self, Place};

pub const PLACE: Place = Place::Dialog {
    width: 520.0,
    height: None,
};

/// One row: a file to edit, or the home folder to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    File {
        path: PathBuf,
        name: String,
        about: &'static str,
    },
    Reveal(PathBuf),
}

/// The home's files: settings, keys, programs, and the theme's file when the theme came from
/// one, or the settings name a theme that isn't built in (its file is to be made).
pub fn rows(s: &State) -> Vec<Row> {
    let home = s.home_dir();
    let mut files = vec![
        (SETTINGS.to_owned(), "Zoom, fonts, theme and colors"),
        (model::KEY_FILE.to_owned(), "The keymap"),
        (PROGRAMS.to_owned(), "Program names, icons and colors"),
    ];
    let theme = s.config.settings.theme.trim();
    let theme_file = s.config.theme_file.clone().or_else(|| {
        (theme != crate::config::theme::ARCTIC).then(|| format!("{THEMES}/{theme}.json"))
    });
    if let Some(file) = theme_file {
        files.push((file, "The theme"));
    }
    let mut out: Vec<Row> = files
        .into_iter()
        .map(|(name, about)| Row::File {
            path: home.join(&name),
            name,
            about,
        })
        .collect();
    out.push(Row::Reveal(home));
    out
}

pub fn view(s: &State, list: &ListPopup, cx: &Cx<State>) -> El {
    let c = &s.chrome;
    let rows = rows(s);
    let shown =
        |p: &std::path::Path| model::filter::display_path(&p.to_string_lossy(), s.model.home());
    let items: Vec<Item> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let on = i == list.cursor;
            let (title, about, icon) = match r {
                Row::File { path, name, about } => (
                    name.clone(),
                    format!("{about} · {}", shown(path)),
                    "ui/file-text.svg",
                ),
                Row::Reveal(home) => (
                    "Show the home folder".to_owned(),
                    shown(home),
                    "ui/folder.svg",
                ),
            };
            Item::row(
                row()
                    .key(format!("settings-row-{i}"))
                    .mx(4.0)
                    .px(10.0)
                    .h(44.0)
                    .gap(10.0)
                    .items_center()
                    .radius(7.0)
                    .fill(if on { c.selected } else { c.popover })
                    .children(overlay::glyph(s, icon, 15.0, c.muted, cx))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(
                                text(title)
                                    .key(format!("settings-name-{i}"))
                                    .font(s.ui_font(13.0, 500.0))
                                    .color(c.foreground)
                                    .truncate(),
                            )
                            .child(overlay::note(s, about).truncate()),
                    ),
                i,
            )
        })
        .collect();
    let problems: Vec<El> = s
        .file_problems()
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            text(p)
                .key(format!("settings-problem-{i}"))
                .font(s.ui_font(12.0, 400.0))
                .color(c.blocked)
                .w_full()
        })
        .collect();
    col()
        .w_full()
        .padding(18.0)
        .gap(12.0)
        .child(overlay::title(
            s,
            "settings-title",
            "Settings",
            Some(overlay::note(s, "Enter: edit · Esc")),
        ))
        .child(
            overlay::note(
                s,
                "herdr-frontend reads these files and follows each change live.",
            )
            .font(s.ui_font(12.0, 400.0)),
        )
        .child(list::view(s, "settings-rows", items, rows.len(), false))
        .children((!problems.is_empty()).then(|| col().w_full().gap(4.0).children(problems)))
}
