//! `PresetReorder`: which presets and groups the browser lets the user drag, and where they may land.

use crate::contract::{ListDrag, Props};
use crate::list::DropTest;
use crate::patterns::presets::model::{PresetRow, children_end, nest_allowed};

/// What may be dragged and how deep groups may nest. The browser always keeps these rules: a preset lands next to
/// another preset or on a group, which it goes into; a group lands next to a group, where `nest` allows its depth.
///
/// ```ignore
/// let reorder = PresetReorder::<u32>::items_and_groups().nest(1).accepts(|drag, over, _| drag.keys != [7]);
/// ```
#[derive(Props)]
pub struct PresetReorder<'a, K> {
    /// Groups drag as well as presets.
    groups: bool,
    /// The deepest level a group may sit at: 0 lets no group into another, 1 lets groups hold folders, and so on.
    #[prop(quantity = "levels")]
    nest: usize,
    /// Refuses the landings for which the test is false: a refused spot shows no drop mark. It gets the drag, the row
    /// under the pointer (none over an empty browser) and the position before which the dragged rows would go.
    #[prop(func = "Fn(&ListDrag<K>, Option<&K>, usize) -> bool")]
    accepts: Option<DropTest<'a, K>>,
}

impl<K> Default for PresetReorder<'_, K> {
    fn default() -> Self {
        Self::items()
    }
}

impl<'a, K> PresetReorder<'a, K> {
    /// Presets move, within and between groups; groups stay where they are.
    pub fn items() -> Self {
        PresetReorder { groups: false, nest: 0, accepts: None }
    }

    /// Presets and groups move.
    pub fn items_and_groups() -> Self {
        PresetReorder { groups: true, nest: 0, accepts: None }
    }

    /// Whether groups may be dragged.
    pub fn moves_groups(&self) -> bool {
        self.groups
    }
}

impl<K: PartialEq> PresetReorder<'_, K> {
    /// Whether dragging `drag` over the row `over` (`None` over an empty browser), before position `insert`, lands
    /// where the rules allow over `rows`.
    pub fn allows(&self, rows: &[PresetRow<K>], drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        let at = |key: &K| rows.iter().position(|row| row.key == *key);
        let Some(over) = over else { return self.accepts.as_ref().is_none_or(|test| test.passes(drag, None, insert)) };
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
        self.accepts.as_ref().is_none_or(|test| test.passes(drag, Some(over), insert))
    }
}
