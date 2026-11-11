//! The data the host supplies to a preset browser: its flat display list of groups and items, the shape of a
//! cell, and what the thumbnail painter is told.

mod apply;
mod library;

pub use library::{GroupPath, PresetChange, PresetError, PresetGroup, PresetId, PresetItem, PresetKey, PresetLibrary};

use crate::list::ThumbAspect;
use crate::primitives::ContentPainterCx;
use craft_behavior::IconId;
use egui::Rect;
use std::fmt;
use std::sync::Arc;

/// The shape of a grid cell's thumbnail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CellShape {
    /// As wide as high.
    #[default]
    Square,
    /// `wide` parts of width to `high` parts of height: 32:18 for gradients.
    #[non_exhaustive]
    Ratio {
        /// Parts of width.
        wide: u16,
        /// Parts of height.
        high: u16,
    },
}

impl CellShape {
    /// A thumbnail `wide` parts wide to `high` parts high.
    pub const fn ratio(wide: u16, high: u16) -> Self {
        CellShape::Ratio { wide, high }
    }

    /// The thumbnail's aspect.
    pub fn aspect(self) -> ThumbAspect {
        match self {
            CellShape::Square => ThumbAspect::SQUARE,
            CellShape::Ratio { wide, high } => ThumbAspect::new(wide, high),
        }
    }
}

/// What the thumbnail painter is told about the cell it paints.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ThumbInfo<'a, K> {
    /// The preset's key.
    pub key: &'a K,
    /// Whether the preset is the selected one.
    pub selected: bool,
    /// Whether the pointer is over the cell.
    pub hovered: bool,
}

impl<'a, K> ThumbInfo<'a, K> {
    /// The facts about the cell of `key`.
    pub const fn new(key: &'a K, selected: bool, hovered: bool) -> Self {
        ThumbInfo { key, selected, hovered }
    }
}

/// Paints a [`Preview`] band into its rect.
pub type PreviewPaint = Arc<dyn Fn(&ContentPainterCx, Rect) + Send + Sync>;

/// A second band of a list row, in list view only: the brush's stroke preview.
#[derive(Clone)]
#[non_exhaustive]
pub struct Preview {
    /// Width over height of the band.
    pub aspect: ThumbAspect,
    /// Paints the band into the rect, clipped to it.
    pub paint: PreviewPaint,
}

impl fmt::Debug for Preview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Preview").field("aspect", &self.aspect).finish_non_exhaustive()
    }
}

/// Whether a row is a group (or a folder inside one) or a preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetKind {
    /// A group or folder: a header row with children.
    Group,
    /// A preset: a thumbnail cell or list row.
    Item,
}

/// One entry of the host's flat display list: a group (or folder) or a preset. `K` is one type for both, so a
/// canvas that takes drops matches the variant it wants.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PresetRow<K> {
    /// The key events name the row by.
    pub key: K,
    /// The name, shown on a header and in a list row, and read by a screen reader.
    pub label: String,
    /// The nesting level: 0 at the top. A row is inside the nearest row above it of lower depth.
    pub depth: usize,
    /// Group or item.
    pub kind: PresetKind,
    /// The number shown at the trailing end of a header.
    pub count: Option<usize>,
    /// A short text under a grid thumbnail and as the detail line of a list row.
    pub caption: Option<String>,
    /// An icon shown instead of a thumbnail.
    pub icon: Option<IconId>,
    /// The stroke preview band of a list row.
    pub preview: Option<Preview>,
    /// Drawn faded.
    pub dimmed: bool,
    /// A tooltip other than the label.
    pub tooltip: Option<String>,
    /// The name a screen reader reads, instead of the label (and caption): the label stays what a rename starts from.
    pub accessible_name: Option<String>,
}

impl<K> PresetRow<K> {
    fn new(key: K, label: impl Into<String>, depth: usize, kind: PresetKind) -> Self {
        Self {
            key,
            label: label.into(),
            depth,
            kind,
            count: None,
            caption: None,
            icon: None,
            preview: None,
            dimmed: false,
            tooltip: None,
            accessible_name: None,
        }
    }

    /// A group or folder at `depth`.
    pub fn group(key: K, label: impl Into<String>, depth: usize) -> Self {
        Self::new(key, label, depth, PresetKind::Group)
    }

    /// A preset at `depth`.
    pub fn item(key: K, label: impl Into<String>, depth: usize) -> Self {
        Self::new(key, label, depth, PresetKind::Item)
    }

    /// Shows `count` at the trailing end of the header.
    #[must_use]
    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    /// Shows `caption` under the thumbnail.
    #[must_use]
    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = Some(caption.into());
        self
    }

    /// Shows `icon` instead of a thumbnail.
    #[must_use]
    pub fn icon(mut self, icon: IconId) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Adds a band of `aspect` in list view, painted by `paint`.
    #[must_use]
    pub fn preview(mut self, aspect: ThumbAspect, paint: impl Fn(&ContentPainterCx, Rect) + Send + Sync + 'static) -> Self {
        self.preview = Some(Preview { aspect, paint: Arc::new(paint) });
        self
    }

    /// Draws the row faded.
    #[must_use]
    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    /// Sets the tooltip, instead of the label.
    #[must_use]
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Sets the name a screen reader reads, instead of the label.
    #[must_use]
    pub fn accessible_name(mut self, name: impl Into<String>) -> Self {
        self.accessible_name = Some(name.into());
        self
    }

    /// Whether the row is a group or folder.
    pub fn is_group(&self) -> bool {
        self.kind == PresetKind::Group
    }
}

/// Whether any row has a caption, so that every grid cell makes room for one line.
pub fn has_captions<K>(rows: &[PresetRow<K>]) -> bool {
    rows.iter().any(|r| !r.is_group() && r.caption.is_some())
}

/// Whether a row's children are in the list after it: the rows below it that are deeper.
pub fn children_end<K>(rows: &[PresetRow<K>], at: usize) -> usize {
    let depth = rows.get(at).map_or(0, |r| r.depth);
    rows.iter().skip(at + 1).position(|r| r.depth <= depth).map_or(rows.len(), |n| at + 1 + n)
}

/// How many levels of groups the subtree at `at` has below its own row (0 for a group with no folder inside).
fn group_levels_below<K>(rows: &[PresetRow<K>], at: usize) -> usize {
    let base = rows.get(at).map_or(0, |r| r.depth);
    rows[at + 1..children_end(rows, at)].iter().filter(|r| r.is_group()).map(|r| r.depth - base).max().unwrap_or(0)
}

/// Whether the group at `dragged` may nest into the group at `target` when no group may sit deeper than `max_nest`
/// (0 lets no group into another), counting the folders the dragged group carries. Nothing nests into itself or its
/// own folders.
pub fn nest_allowed<K>(rows: &[PresetRow<K>], dragged: usize, target: usize, max_nest: usize) -> bool {
    let (Some(from), Some(into)) = (rows.get(dragged), rows.get(target)) else {
        return false;
    };
    if !from.is_group() || !into.is_group() || (dragged..children_end(rows, dragged)).contains(&target) {
        return false;
    }
    into.depth + 1 + group_levels_below(rows, dragged) <= max_nest
}
