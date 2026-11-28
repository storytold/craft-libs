//! The fit rule: how much of each group shows at each width.

use super::GroupLevel;
use super::group::LEVELS;

/// The widths of one group at each of its levels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GroupWidths {
    /// The group at `GroupLevel::Full`.
    pub full: f32,
    /// The group at `GroupLevel::Compact`.
    pub compact: f32,
    /// The group at `GroupLevel::Collapsed`.
    pub collapsed: f32,
}

impl GroupWidths {
    /// The widths of a group at its full, compact and collapsed levels.
    pub const fn new(full: f32, compact: f32, collapsed: f32) -> Self {
        GroupWidths { full, compact, collapsed }
    }
}

/// The level of each group for `available` width, by the rule of a row's levelled units
/// (`craft_ui::layout::plan_levels`), which the ribbon's row applies to its groups. Groups shrink from the end: first
/// each to compact, then each to collapsed, stopping as soon as the row fits. Then, from the start, each group takes
/// back a level while the row still fits: shrinking a wide group can free room an earlier group no longer needed to
/// give up. A width that is not a number never fits, so it gives nothing back.
pub fn plan(widths: &[GroupWidths], available: f32) -> Vec<GroupLevel> {
    let units: Vec<Vec<f32>> = widths.iter().map(|group| vec![group.full, group.compact, group.collapsed]).collect();
    craft_ui::layout::plan_levels(&units, available).into_iter().map(|at| LEVELS.get(at).copied().unwrap_or(GroupLevel::Collapsed)).collect()
}
