//! `PresetReorder`: which presets and groups the browser lets the user drag, and where they may land.

use crate::contract::{ListDrag, Props};
use crate::patterns::presets::model::{PresetRow, children_end, nest_allowed};
use crate::patterns::presets::source::PresetSource;

/// What may be dragged and how deep groups may nest. The browser always keeps these rules: a preset lands next to
/// another preset or on a group, which it goes into; a group lands next to a group, where `nest` allows its depth. The
/// host refuses further landings with its source's `accepts` test ([`PresetFns::accepts`](super::super::PresetFns::accepts)).
///
/// ```ignore
/// let reorder = PresetReorder::items_and_groups().nest(1);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Props, crate::contract::MemoProps)]
pub struct PresetReorder {
    /// Groups drag as well as presets.
    groups: bool,
    /// The deepest level a group may sit at: 0 lets no group into another, 1 lets groups hold folders, and so on.
    #[prop(quantity = "levels")]
    nest: usize,
}

impl Default for PresetReorder {
    fn default() -> Self {
        Self::items()
    }
}

impl PresetReorder {
    /// Presets move, within and between groups; groups stay where they are.
    pub fn items() -> Self {
        PresetReorder { groups: false, nest: 0 }
    }

    /// Presets and groups move.
    pub fn items_and_groups() -> Self {
        PresetReorder { groups: true, nest: 0 }
    }

    /// Whether groups may be dragged.
    pub fn moves_groups(&self) -> bool {
        self.groups
    }
}

impl PresetReorder {
    /// Whether dragging `drag` over the row `over` (`None` over an empty browser), before position `insert`, lands
    /// where the rules allow over `rows` and `source` accepts.
    pub fn allows<K: Clone + PartialEq + Send + Sync + 'static>(
        &self,
        source: &PresetSource<'_, K>,
        rows: &[PresetRow<K>],
        drag: &ListDrag<K>,
        over: Option<&K>,
        insert: usize,
    ) -> bool {
        let at = |key: &K| rows.iter().position(|row| row.key == *key);
        let Some(over) = over else { return source.accepts(drag, None, insert) };
        let Some(target) = at(over) else { return false };
        for key in &drag.keys {
            let Some(from) = at(key) else { return false };
            if (from..children_end(rows, from)).contains(&target) {
                return false;
            }
            let fits = !rows[from].is_group()
                || (self.groups && rows[target].is_group() && (rows[target].depth == 0 || nest_allowed(rows, from, target, self.nest + 1)));
            if !fits {
                return false;
            }
        }
        source.accepts(drag, Some(over), insert)
    }
}
