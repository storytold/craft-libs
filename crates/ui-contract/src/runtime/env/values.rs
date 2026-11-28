//! The library values the env carries: scale, motion policy, arena id and the component set.

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

/// The user's wheel multipliers: `line` for notches, `point` for touchpad deltas. Distinct from
/// [`WheelScope`](super::WheelScope), which says who takes the wheel, not how far it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WheelSpeed {
    /// The multiplier for line (notch) scrolling.
    pub line: f32,
    /// The multiplier for point (touchpad) scrolling.
    pub point: f32,
}

impl WheelSpeed {
    /// The multipliers for line (notch) and point (touchpad) scrolling.
    pub const fn new(line: f32, point: f32) -> Self {
        WheelSpeed { line, point }
    }
}

impl Default for WheelSpeed {
    fn default() -> Self {
        WheelSpeed { line: 1.0, point: 1.0 }
    }
}

/// Code pages that decode Alt+numpad codes; the app sets them from the system locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AltCodePages {
    /// The ANSI code page, for codes typed with a leading zero.
    pub ansi: u16,
    /// The OEM code page, for codes typed without one.
    pub oem: u16,
}

impl AltCodePages {
    /// The ANSI and OEM code pages.
    pub const fn new(ansi: u16, oem: u16) -> Self {
        AltCodePages { ansi, oem }
    }
}

impl Default for AltCodePages {
    fn default() -> Self {
        AltCodePages { ansi: 1252, oem: 437 }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::contract::MemoProps)]
#[doc(hidden)]
pub struct InBar(pub bool);

impl EnvValue for InBar {
    const AFFECTS_STYLE: bool = true;

    fn root() -> Self {
        InBar(false)
    }

    fn style_hash(&self) -> Option<u64> {
        use std::hash::{Hash as _, Hasher as _};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        Some(hasher.finish())
    }
}

impl super::StyleValue for InBar {}

/// The component set: an app sets it on its root (`RootEnv::set`) and a container may set another for its subtree
/// (`Overrides::values`); every window, layer and popup opened inside takes the scope's. A component with variants
/// reads it in `style` (`StyleCx::set`) and maps it to one of them. It is in the style key, so a change restyles as a
/// theme or density change does.
impl EnvValue for craft_theme::ComponentSet {
    const AFFECTS_STYLE: bool = true;

    fn root() -> Self {
        craft_theme::ComponentSet::default()
    }

    fn style_hash(&self) -> Option<u64> {
        use std::hash::Hasher as _;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        Some(hasher.finish())
    }
}

impl super::StyleValue for craft_theme::ComponentSet {}

/// Where a shell pane sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, crate::contract::MemoProps)]
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
