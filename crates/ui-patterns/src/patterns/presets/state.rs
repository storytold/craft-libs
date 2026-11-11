//! `PresetState`: the view settings of a preset browser that the host stores: grid or list, thumbnail zoom, which
//! groups differ from the default open state, and the search text.

use super::model::PresetRow;
use crate::contract::OpenChange;
use crate::tile_grid::TileZoom;
use std::collections::HashSet;
use std::hash::Hash;

/// How the presets are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PresetView {
    /// Thumbnails in a wrapped grid.
    #[default]
    Grid,
    /// One row per preset with its name.
    List,
}

/// Which groups are open before the user toggles any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DefaultOpen {
    /// Every group.
    #[default]
    All,
    /// The first group in the list only.
    First,
    /// No group.
    None,
}

/// The view settings of a preset browser. Plain data: the host keeps it (and may save it); the browser and a
/// `PresetFooter` write the same value.
///
/// A group is open when its default (from [`DefaultOpen`]) differs from membership in `toggled`. While the search
/// text is not empty, every group is open.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, bound(serialize = "K: serde::Serialize", deserialize = "K: serde::Deserialize<'de> + Eq + std::hash::Hash"))]
#[non_exhaustive]
pub struct PresetState<K> {
    /// Grid or list. Independent of the zoom: a small thumbnail stays a grid.
    pub view: PresetView,
    /// The thumbnail zoom, a factor of the grid's large thumbnail that both views follow.
    pub zoom: f64,
    /// The groups whose open state differs from the default.
    pub toggled: HashSet<K>,
    /// The search text; the browser shows presets whose name matches it.
    pub filter: String,
}

impl<K> Default for PresetState<K> {
    fn default() -> Self {
        Self { view: PresetView::Grid, zoom: 1.0, toggled: HashSet::new(), filter: String::new() }
    }
}

impl<K: Eq + Hash> PartialEq for PresetState<K> {
    fn eq(&self, other: &Self) -> bool {
        self.view == other.view && self.zoom == other.zoom && self.toggled == other.toggled && self.filter == other.filter
    }
}

impl<K> PresetState<K> {
    /// A grid at the medium thumbnail, no group toggled, no filter.
    pub fn new() -> Self {
        Self::default()
    }

    /// The zoom clamped to the range of `zoom` (a NaN gives its default).
    pub fn zoom_in(&self, zoom: &TileZoom) -> f64 {
        let range = zoom.range();
        if self.zoom.is_nan() { zoom.default_value() } else { self.zoom.clamp(*range.start(), range.end().max(*range.start())) }
    }

    /// Whether the search text filters the list (it is not blank).
    pub fn filtering(&self) -> bool {
        !self.filter.trim().is_empty()
    }
}

impl<K: Clone + Eq + Hash> PresetState<K> {
    /// Whether the group at `at` is open for the user: its default, flipped by `toggled`. Every group is open while
    /// the list is filtered.
    pub fn is_open(&self, rows: &[PresetRow<K>], at: usize, default_open: DefaultOpen) -> bool {
        self.open_test(rows, default_open)(at)
    }

    /// [`is_open`](PresetState::is_open) for many rows: a test that finds the first group once.
    pub fn open_test<'a>(&'a self, rows: &'a [PresetRow<K>], default_open: DefaultOpen) -> impl Fn(usize) -> bool + 'a {
        let first = rows.iter().position(|r| r.is_group());
        let filtering = self.filtering();
        move |at| {
            let Some(row) = rows.get(at) else {
                return false;
            };
            let by_default = match default_open {
                DefaultOpen::All => true,
                DefaultOpen::None => false,
                DefaultOpen::First => first == Some(at),
            };
            filtering || by_default != self.toggled.contains(&row.key)
        }
    }

    /// Opens or closes the group at `at` (toggling its key as needed).
    pub fn set_open(&mut self, rows: &[PresetRow<K>], at: usize, default_open: DefaultOpen, open: bool) {
        let Some(row) = rows.get(at).filter(|r| r.is_group()) else {
            return;
        };
        if default_is_open(rows, at, default_open) == open {
            self.toggled.remove(&row.key);
        } else {
            self.toggled.insert(row.key.clone());
        }
    }

    /// Flips the group at `at`.
    pub fn toggle(&mut self, rows: &[PresetRow<K>], at: usize, default_open: DefaultOpen) {
        let open = !self.is_open(rows, at, default_open);
        self.set_open(rows, at, default_open, open);
    }

    /// Applies a disclosure's [`OpenChange`]: the group it names, or every group when `all`.
    pub fn apply_open(&mut self, rows: &[PresetRow<K>], change: &OpenChange<K>, default_open: DefaultOpen) {
        if change.all {
            for at in (0..rows.len()).filter(|&at| rows[at].is_group()) {
                self.set_open(rows, at, default_open, change.open);
            }
        } else if let Some(at) = rows.iter().position(|r| r.is_group() && r.key == change.key) {
            self.set_open(rows, at, default_open, change.open);
        }
    }

    /// Forgets toggled keys that are no longer groups of `rows`, so a deleted group's key does not flip a new one
    /// that reuses it.
    pub fn retain_groups(&mut self, rows: &[PresetRow<K>]) {
        let groups: HashSet<&K> = rows.iter().filter(|r| r.is_group()).map(|r| &r.key).collect();
        self.toggled.retain(|k| groups.contains(k));
    }
}

/// Whether the group at `at` is open before any toggle.
pub fn default_is_open<K>(rows: &[PresetRow<K>], at: usize, default_open: DefaultOpen) -> bool {
    match default_open {
        DefaultOpen::All => true,
        DefaultOpen::None => false,
        DefaultOpen::First => rows.iter().position(|r| r.is_group()) == Some(at),
    }
}
