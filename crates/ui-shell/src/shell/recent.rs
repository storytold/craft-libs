//! The start screen's data: recent files, templates, the view and sort the recent files show in, and the sort.

use craft_i18n::Collator;
use std::cmp::Ordering;
use std::path::PathBuf;

use crate::primitives::Revision;

/// The key a start screen's thumbnail has in the app's texture store. The app loads the picture for the key and
/// revision a card names; the screen only reads the store.
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
    /// The revision of the picture stored under [`ThumbKey::Recent`] with this path; `None` for a file with no
    /// thumbnail.
    pub thumbnail: Option<Revision>,
}

impl RecentFile {
    /// A recent file at `path` shown as `name`, with no dates and no thumbnail.
    pub fn new(path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        RecentFile { path: path.into(), name: name.into(), opened: None, created: None, thumbnail: None }
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
    /// The revision of the picture stored under [`ThumbKey::Template`] with this id; `None` for a card with no
    /// preview.
    pub thumbnail: Option<Revision>,
}

impl Template {
    /// A template `id` named `name`, with `detail` under the name and no preview.
    pub fn new(id: impl Into<String>, name: impl Into<String>, detail: impl Into<String>) -> Self {
        Template { id: id.into(), name: name.into(), detail: detail.into(), thumbnail: None }
    }
}

/// How the recent files show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RecentView {
    /// Cards with thumbnails, as many columns as fit.
    #[default]
    Grid,
    /// One row per file: icon, name, folder, age.
    List,
}

/// The recent files' order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
