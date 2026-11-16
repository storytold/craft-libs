//! The library values the env carries: scale, motion policy and arena id.

use super::EnvValue;
use craft_behavior::{ColumnKey, MotionMode};
use craft_theme::MotionTokens;
use egui::Id;
use std::hash::Hash;

/// The interface scale: pixels per point.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Scale(pub f32);

impl Scale {
    /// `pixels_per_point` pixels per point.
    pub const fn new(pixels_per_point: f32) -> Self {
        Scale(pixels_per_point)
    }
}

impl Default for Scale {
    fn default() -> Self {
        Scale(1.0)
    }
}

/// The motion mode in effect, carried in `Env.motion`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MotionPolicy {
    /// The level animations run at.
    pub mode: MotionMode,
    /// The durations and curves of the theme's motions; reduced motion keeps them.
    pub tokens: MotionTokens,
}

impl MotionPolicy {
    /// The policy that runs animations at `mode` with `tokens`.
    pub const fn new(mode: MotionMode, tokens: MotionTokens) -> Self {
        Self { mode, tokens }
    }
}

/// The id of an arena, carried in `Env.arena`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ArenaId(pub Id);

impl ArenaId {
    /// The arena whose id is `id`.
    pub const fn new(id: Id) -> Self {
        ArenaId(id)
    }
}

/// Where a column's content sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnAlign {
    /// At the start edge.
    #[default]
    Start,
    /// At the end edge.
    End,
}

/// Where the collection holds a primary press on a row, within slop, or the press a part of the row holds.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowPressed {
    /// On the row's own area.
    Row,
    /// On the cell of this toggle column, which the press flips.
    Cell(ColumnKey),
    /// On a part of the item that took the press itself (a row's trailing button).
    #[non_exhaustive]
    Part {
        /// The row held the lead when the press began: a press on a part moves the lead to its row.
        was_lead: bool,
    },
}

/// Set by a bar for its body: its fields put `space.gap.item` between a caption and the control, a vertical divider
/// takes a box wider than its line, a select fits its widest entry up to a field length, a rule shows only with the
/// member after it, and a row too long for its room narrows its last units first. A layer or pane opened from the bar
/// starts without it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct InBar(pub bool);

impl EnvValue for InBar {
    fn root() -> Self {
        InBar(false)
    }
}

/// Where a shell pane sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PaneEdge {
    /// What the edge panes leave.
    #[default]
    Center,
    /// The start edge: left in reading order, right under right-to-left.
    Start,
    /// The end edge.
    End,
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
}
