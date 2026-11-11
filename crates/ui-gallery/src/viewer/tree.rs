//! The component tree: one group per level, the components of each under it, narrowed by the filter.

use super::catalog::{ComponentInfo, fold_filter};
use craft_behavior::{DisplayOrder, ItemSource};
use craft_ui::contract::Level;
use craft_ui::list::OpenSet;
use std::borrow::Cow;

/// The key of a tree row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TreeKey {
    /// A level group.
    Level(Level),
    /// A component, by its registered type name.
    Component(&'static str),
}

/// One shown row of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TreeRow {
    /// The row's key.
    pub key: TreeKey,
    /// The text the row shows.
    pub label: String,
    /// 0 for a level, 1 for a component.
    pub depth: usize,
    /// Whether the row holds components.
    pub group: bool,
    /// Whether a group shows its components.
    pub open: bool,
}

/// The slug a level is remembered by.
pub fn slug(level: Level) -> String {
    level.title().to_lowercase()
}

/// The level whose slug is `slug`.
pub fn level_named(slug_text: &str) -> Option<Level> {
    Level::ALL.into_iter().find(|level| slug(*level) == slug_text)
}

/// Opens the level of `selected` when the selection is not the one `seen` records, then records it, so a selection
/// that moves into a closed level shows its row. The levels the user opened or closed otherwise stay as they are.
pub fn open_level_of_selection(
    open: &mut OpenSet<TreeKey>,
    components: &[ComponentInfo],
    selected: Option<&'static str>,
    seen: &mut Option<Option<&'static str>>,
) {
    if *seen == Some(selected) {
        return;
    }
    *seen = Some(selected);
    if let Some(component) = selected.and_then(|type_name| components.iter().find(|component| component.type_name == type_name)) {
        open.set(&TreeKey::Level(component.level), true);
    }
}

/// The rows of the tree for `filter`: the levels in order, each with the components that match, sorted by name
/// ignoring case. A filter opens every level that holds a match, so the matches show; the open set stays as it is.
pub fn rows(components: &[ComponentInfo], filter: &str, open: &OpenSet<TreeKey>) -> Vec<TreeRow> {
    let filter = fold_filter(filter);
    let filtering = !filter.is_empty();
    let mut rows = Vec::new();
    for level in Level::ALL {
        let mut members: Vec<&ComponentInfo> = components.iter().filter(|component| component.level == level && component.matches(&filter)).collect();
        if members.is_empty() {
            continue;
        }
        members.sort_by_cached_key(|component| component.name.to_lowercase());
        let key = TreeKey::Level(level);
        let shown = filtering || open.is_open(&key);
        rows.push(TreeRow { key, label: level.title().to_owned(), depth: 0, group: true, open: shown });
        if shown {
            rows.extend(members.into_iter().map(|component| TreeRow {
                key: TreeKey::Component(component.type_name),
                label: component.name.clone(),
                depth: 1,
                group: false,
                open: false,
            }));
        }
    }
    rows
}

/// The groups of `rows`, for an Alt-click that opens or closes every group.
pub fn groups(rows: &[TreeRow]) -> Vec<TreeKey> {
    rows.iter().filter(|row| row.group).map(|row| row.key.clone()).collect()
}

/// `rows` as the display order of the list.
pub struct TreeRows<'a>(&'a [TreeRow]);

impl<'a> TreeRows<'a> {
    /// The display order of `rows`.
    pub fn new(rows: &'a [TreeRow]) -> Self {
        TreeRows(rows)
    }
}

impl DisplayOrder<TreeKey> for TreeRows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> TreeKey {
        self.0[at].key.clone()
    }

    fn depth(&self, at: usize) -> usize {
        self.0[at].depth
    }

    fn has_children(&self, at: usize) -> bool {
        self.0[at].group
    }

    fn open(&self, at: usize) -> bool {
        self.0[at].open
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.0[at].label.as_str()))
    }
}

impl ItemSource<TreeKey> for TreeRows<'_> {}
