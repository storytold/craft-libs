//! The fit rule: how much of each group shows at each width.

use super::GroupLevel;

/// The widths of one group at each of its levels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroupWidths {
    /// The group at `GroupLevel::Full`.
    pub full: f32,
    /// The group at `GroupLevel::Compact`.
    pub compact: f32,
    /// The group at `GroupLevel::Collapsed`.
    pub collapsed: f32,
}

/// The level of each group for `available` width, by production's fit rule.
pub fn plan(widths: &[GroupWidths], available: f32) -> Vec<GroupLevel> {
    let _ = (widths, available);
    todo!("rb.R12")
}
