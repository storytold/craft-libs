//! The start screen's data: recent files, templates, the view and sort the recent files show in, and the sort.

use craft_i18n::Collator;
use std::cmp::Ordering;
use std::path::PathBuf;

/// The key a start screen's thumbnail has. The screen asks `Thumbnails::state` for the key at the painted cell, and
/// the app's `ThumbSource` answers its revision and renders the picture.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ThumbKey {
    /// The thumbnail of the recent file at this path.
    Recent(PathBuf),
    /// The preview of the template with this id.
    Template(String),
}

/// A recent file as the start screen lists it. The app keeps the list and passes it every pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RecentFile {
    /// The file's full path; also the identity of the entry.
    pub path: PathBuf,
    /// The name shown: the file name without its folder.
    pub name: String,
    /// Last opened, in seconds since the Unix epoch, for sorting and the age line.
    pub opened: Option<i64>,
    /// Created, in seconds since the Unix epoch, for sorting by Created.
    pub created: Option<i64>,
    /// A note the app gives ("12 pages · 2.4 MB"), shown in place of the age line.
    pub note: Option<String>,
}

impl RecentFile {
    /// A recent file at `path` shown as `name`, with no dates.
    pub fn new(path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        RecentFile { path: path.into(), name: name.into(), opened: None, created: None, note: None }
    }

    /// This file last opened at `opened`, in seconds since the Unix epoch.
    #[must_use]
    pub fn opened(mut self, opened: i64) -> Self {
        self.opened = Some(opened);
        self
    }

    /// This file created at `created`, in seconds since the Unix epoch.
    #[must_use]
    pub fn created(mut self, created: i64) -> Self {
        self.created = Some(created);
        self
    }

    /// This file with `note` shown in place of its age line.
    #[must_use]
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// A template card: a named document size or starter file.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Template {
    /// The app's identifier, returned in [`StartAction::UseTemplate`](super::StartAction::UseTemplate).
    pub id: String,
    /// The card's name, translated by the app.
    pub name: String,
    /// The line under the name: "1920 × 1080 px".
    pub detail: String,
    /// The width and height of the page the template makes, in any one unit: its card shows the page
    /// (`Tile::page`), as New Document's presets do, in place of the template's thumbnail.
    pub page: Option<(f64, f64)>,
}

impl Template {
    /// A template `id` named `name`, with `detail` under the name.
    pub fn new(id: impl Into<String>, name: impl Into<String>, detail: impl Into<String>) -> Self {
        Template { id: id.into(), name: name.into(), detail: detail.into(), page: None }
    }

    /// This template makes a `width` by `height` page, which its card shows.
    #[must_use]
    pub fn page(mut self, width: f64, height: f64) -> Self {
        self.page = Some((width, height));
        self
    }
}

/// How the recent files show. Saved as `"grid"` or `"list"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum RecentView {
    /// Cards with thumbnails, as many columns as fit.
    #[default]
    Grid,
    /// One row per file: icon, name, folder, age.
    List,
}

/// The recent files' order. Saved as `"opened"`, `"name"` or `"created"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum RecentSort {
    /// Most recently opened first.
    #[default]
    Opened,
    /// By file name, runs of digits by value.
    Name,
    /// Most recently created first.
    Created,
}

/// Newest first; an entry with no date comes after every dated one.
fn newest_first(a: Option<i64>, b: Option<i64>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// The recent files in `sort` order. Opened and Created put the newest first; Name follows `collator`, with a path
/// breaking ties so the order does not depend on the input's.
pub fn sort_recent(files: &mut [RecentFile], sort: RecentSort, collator: &Collator) {
    match sort {
        RecentSort::Opened => files.sort_by(|a, b| newest_first(a.opened, b.opened)),
        RecentSort::Created => files.sort_by(|a, b| newest_first(a.created, b.created)),
        RecentSort::Name => files.sort_by(|a, b| collator.compare(&a.name, &b.name).then_with(|| a.path.cmp(&b.path))),
    }
}
