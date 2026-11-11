//! `PresetLibrary::apply`: the browser's Rename, Delete, MoveTo and Reorder events carried out on the library.

use super::library::{GroupPath, PresetChange, PresetError, PresetGroup, PresetId, PresetKey, PresetLibrary, same_name};
use super::{PresetRow, children_end};
use crate::contract::ReorderMove;
use crate::patterns::presets::PresetEvent;
use craft_behavior::EditEvent;

/// Where moved rows land relative to the row they are placed against.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Before,
    After,
}

/// Rows of `group`'s deepest chain, counting `group` itself.
fn height<T>(group: &PresetGroup<T>) -> usize {
    1 + group.groups.iter().map(height).max().unwrap_or(0)
}

type Key = PresetKey<PresetId>;

impl<T> PresetLibrary<T> {
    /// Carries out one browser event and returns what changed (empty when nothing did):
    ///
    /// - `Rename` of a preset or group ([`rename`](Self::rename), [`rename_group`](Self::rename_group));
    ///   a refused name changes nothing.
    /// - `Delete` of a preset or group.
    /// - `MoveTo { key, group }` with a preset key and a group key.
    /// - `Reorder` when it is a `Commit` or a `Set` ([`reorder`](Self::reorder)); a copy is ignored, and so are
    ///   `Begin`, `Update` and `Cancel`.
    /// - Every other event returns nothing.
    ///
    /// The browser's reorder rule already refused what the library refuses; the library checks again and ignores
    /// a refused move.
    pub fn apply(&mut self, event: &PresetEvent<Key>) -> Vec<PresetChange> {
        let change = match event {
            PresetEvent::Rename { key: PresetKey::Item(id), text } => self.rename(*id, text),
            PresetEvent::Rename { key: PresetKey::Group(path), text } => self.rename_group(path, text),
            PresetEvent::Delete(PresetKey::Item(id)) => self.delete(*id),
            PresetEvent::Delete(PresetKey::Group(path)) => self.delete_group(path),
            PresetEvent::MoveTo { key: PresetKey::Item(id), group: PresetKey::Group(path) } => self.move_to(*id, path),
            PresetEvent::Reorder(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
                return self.reorder_move(after).unwrap_or_default();
            }
            _ => return Vec::new(),
        };
        change.into_iter().collect()
    }

    /// [`apply`](Self::apply) for each event in order, the changes concatenated.
    pub fn apply_all(&mut self, events: &[PresetEvent<Key>]) -> Vec<PresetChange> {
        events.iter().flat_map(|event| self.apply(event)).collect()
    }

    /// [`reorder`](Self::reorder) for the move a browser reports.
    pub fn reorder_move(&mut self, mv: &ReorderMove<Key>) -> Result<Vec<PresetChange>, PresetError> {
        if mv.copy {
            return Ok(Vec::new());
        }
        match &mv.parent {
            Some(PresetKey::Group(parent)) => self.reorder_into(&mv.keys, parent, mv.to),
            _ => self.reorder(&mv.keys, mv.to),
        }
    }

    /// [`reorder`](Self::reorder) for a move into the group `parent`: the moved rows go before the row at `to` when
    /// that row is a child of `parent`, else after the last child of `parent`.
    pub fn reorder_into(&mut self, keys: &[Key], parent: &GroupPath, to: usize) -> Result<Vec<PresetChange>, PresetError> {
        self.group(parent).ok_or(PresetError::NoGroup)?;
        let rows = self.rows();
        let mut gone = vec![false; rows.len()];
        for at in keys.iter().filter_map(|key| rows.iter().position(|row| row.key == *key)) {
            gone[at..children_end(&rows, at)].fill(true);
        }
        let rest: Vec<&PresetRow<Key>> = rows.iter().zip(&gone).filter(|(_, gone)| !**gone).map(|(row, _)| row).collect();
        let child_of_parent = |key: &Key| match key {
            PresetKey::Item(id) => self.group_of(*id).as_ref() == Some(parent),
            PresetKey::Group(path) => path.parent().as_ref() == Some(parent),
        };
        match rest.get(to).filter(|row| child_of_parent(&row.key)) {
            Some(row) => self.reorder_against(keys, &row.key.clone(), Place::Before),
            None => {
                let group = self.group(parent).ok_or(PresetError::NoGroup)?;
                let (items, groups) = (group.items.len(), group.groups.len());
                self.reorder_to(keys, parent.clone(), items, groups)
            }
        }
    }

    /// Moves `keys` so that they sit together at display position `to`, counted in the display order that remains once
    /// the moved rows (and the rows inside moved groups) are taken out; they keep their display order. They go before
    /// the row now at `to`, or after the last row when `to` is past the end. Presets next to a preset of another
    /// group change group (`Moved`), groups next to another group change parent (`GroupMoved`), and the order inside
    /// each group touched changes (`Reordered`). A group moved into itself or one of its folders, or deeper than
    /// the nesting, is refused ([`PresetError::TooDeep`]), and so is a group whose name a new sibling already has
    /// ([`PresetError::NameTaken`]); nothing changes then.
    pub fn reorder(&mut self, keys: &[Key], to: usize) -> Result<Vec<PresetChange>, PresetError> {
        let rows = self.rows();
        let moved: Vec<usize> = keys.iter().filter_map(|key| rows.iter().position(|row| row.key == *key)).collect();
        let mut gone = vec![false; rows.len()];
        for &at in &moved {
            for slot in &mut gone[at..children_end(&rows, at)] {
                *slot = true;
            }
        }
        let rest: Vec<&PresetRow<Key>> = rows.iter().zip(&gone).filter(|(_, gone)| !**gone).map(|(row, _)| row).collect();
        let (target, place) = match rest.get(to) {
            Some(row) => (row.key.clone(), Place::Before),
            None => (rest.last().ok_or(PresetError::NoGroup)?.key.clone(), Place::After),
        };
        self.reorder_against(keys, &target, place)
    }

    fn reorder_against(&mut self, items: &[Key], target: &Key, place: Place) -> Result<Vec<PresetChange>, PresetError> {
        // The destination group and the slots in it, as indices into its lists before any row is
        // taken out. Items sit before groups, so a preset next to a group goes to the end of the
        // items and a group next to a preset to the start of the groups.
        let (dest, item_slot, group_slot) = match target {
            PresetKey::Item(id) => {
                let dest = self.group_of(*id).ok_or(PresetError::NoPreset)?;
                let at = self.group(&dest).and_then(|g| g.items.iter().position(|i| i.id == *id)).ok_or(PresetError::NoPreset)?;
                match place {
                    Place::Before => (dest, at, 0),
                    Place::After => (dest, at + 1, 0),
                }
            }
            PresetKey::Group(path) => {
                self.group(path).filter(|_| path.depth() > 0).ok_or(PresetError::NoGroup)?;
                let parent = path.parent().ok_or(PresetError::NoGroup)?;
                let siblings = self.group(&parent).ok_or(PresetError::NoGroup)?;
                let at = siblings.groups.iter().position(|g| g.name == path.name()).ok_or(PresetError::NoGroup)?;
                let at = if place == Place::After { at + 1 } else { at };
                let items = siblings.items.len();
                (parent, items, at)
            }
        };

        self.reorder_to(items, dest, item_slot, group_slot)
    }

    /// Moves `items` into `dest`, their presets at `item_slot` of its items and their groups at `group_slot` of its
    /// groups, both counted before any row is taken out.
    fn reorder_to(&mut self, items: &[Key], dest: GroupPath, item_slot: usize, group_slot: usize) -> Result<Vec<PresetChange>, PresetError> {
        let nesting_limit = self.nesting + 1;

        // The rows moved: a row inside another moved group goes with that group.
        let mut group_paths: Vec<GroupPath> = Vec::new();
        for key in items {
            if let PresetKey::Group(path) = key {
                self.group(path).filter(|_| path.depth() > 0).ok_or(PresetError::NoGroup)?;
                if !group_paths.contains(path) {
                    group_paths.push(path.clone());
                }
            }
        }
        let inside_moved = |path: &GroupPath| group_paths.iter().any(|g| path.names().starts_with(g.names()) && path != g);
        let top_paths: Vec<GroupPath> = group_paths.iter().filter(|g| !inside_moved(g)).cloned().collect();
        let mut preset_ids: Vec<PresetId> = Vec::new();
        for key in items {
            if let PresetKey::Item(id) = key {
                let from = self.group_of(*id).ok_or(PresetError::NoPreset)?;
                let goes_with_group = top_paths.iter().any(|g| from.names().starts_with(g.names()));
                if !goes_with_group && !preset_ids.contains(id) {
                    preset_ids.push(*id);
                }
            }
        }

        // Refuse before changing anything.
        let dest_group = self.group(&dest).ok_or(PresetError::NoGroup)?;
        for (n, path) in top_paths.iter().enumerate() {
            if dest.names().starts_with(path.names()) {
                return Err(PresetError::TooDeep);
            }
            let source = self.group(path).ok_or(PresetError::NoGroup)?;
            let new_deepest = dest.depth() + height(source);
            let old_deepest = path.depth() + height(source) - 1;
            if new_deepest > nesting_limit && new_deepest > old_deepest {
                return Err(PresetError::TooDeep);
            }
            let moving_out = |sibling: &PresetGroup<T>| top_paths.iter().any(|p| p.parent().as_ref() == Some(&dest) && p.name() == sibling.name);
            let clash = dest_group.groups.iter().any(|g| !moving_out(g) && same_name(&g.name, path.name()));
            let doubled = top_paths[..n].iter().any(|p| same_name(p.name(), path.name()));
            if clash || doubled {
                return Err(PresetError::NameTaken);
            }
        }

        let order = |group: &PresetGroup<T>| {
            (group.items.iter().map(|i| i.id).collect::<Vec<_>>(), group.groups.iter().map(|g| g.name.clone()).collect::<Vec<_>>())
        };
        let before = order(dest_group);
        let item_slot = item_slot - dest_group.items.iter().take(item_slot).filter(|i| preset_ids.contains(&i.id)).count();
        let group_slot = group_slot
            - dest_group
                .groups
                .iter()
                .take(group_slot)
                .filter(|g| top_paths.iter().any(|p| p.parent().as_ref() == Some(&dest) && p.name() == g.name))
                .count();

        let mut changes = Vec::new();
        let mut taken_items = Vec::new();
        for id in &preset_ids {
            let from = self.group_of(*id).ok_or(PresetError::NoPreset)?;
            let source = self.group_mut(&from).ok_or(PresetError::NoGroup)?;
            let at = source.items.iter().position(|i| i.id == *id).ok_or(PresetError::NoPreset)?;
            taken_items.push(source.items.remove(at));
            if from != dest {
                changes.push(PresetChange::Moved { id: *id, to: dest.clone() });
            }
        }
        let mut taken_groups = Vec::new();
        for path in &top_paths {
            let parent = path.parent().ok_or(PresetError::NoGroup)?;
            let source = self.group_mut(&parent).ok_or(PresetError::NoGroup)?;
            let at = source.groups.iter().position(|g| g.name == path.name()).ok_or(PresetError::NoGroup)?;
            taken_groups.push(source.groups.remove(at));
            if parent != dest {
                changes.push(PresetChange::GroupMoved { from: path.clone(), to: dest.child(path.name()) });
            }
        }
        let target = self.group_mut(&dest).ok_or(PresetError::NoGroup)?;
        let at = item_slot.min(target.items.len());
        target.items.splice(at..at, taken_items);
        let at = group_slot.min(target.groups.len());
        target.groups.splice(at..at, taken_groups);
        if order(target) != before {
            changes.push(PresetChange::Reordered(dest));
        }
        Ok(changes)
    }
}
