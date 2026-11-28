//! [`ActionList`]: the flat list of actions with an optional set each, and every edit the panel
//! and the app's commands need.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::file::{ActionOccurrence, ActionsFile};
use super::model::{Action, ActionColor, ActionInclude, ActionStep};

/// Why an [`ActionList`] edit was refused. The app shows the message; nothing changed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionListError {
    /// No action at this index.
    NoSuchAction(usize),
    /// No step at this index of the action.
    NoSuchStep {
        /// The action.
        action: usize,
        /// The step.
        step: usize,
    },
    /// A destination past the end.
    OutOfRange,
    /// No set of this name.
    NoSuchSet(String),
    /// Another action (or set) already has the name.
    NameTaken(String),
    /// The name is empty or only white space.
    EmptyName,
    /// No Clear All, Reset or Replace to undo: none was made, or an edit since ended its undo.
    NothingToUndo,
}

impl std::fmt::Display for ActionListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionListError::NoSuchAction(_) => f.write_str("no such action"),
            ActionListError::NoSuchStep { .. } => f.write_str("no such step"),
            ActionListError::NoSuchSet(_) => f.write_str("no such set"),
            ActionListError::OutOfRange => f.write_str("destination is out of range"),
            ActionListError::NameTaken(_) => f.write_str("an action with this name already exists"),
            ActionListError::EmptyName => f.write_str("\"name\" is empty"),
            ActionListError::NothingToUndo => f.write_str("no Actions library change is available to undo"),
        }
    }
}

impl std::error::Error for ActionListError {}

/// What [`ActionList::merge_file`] added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MergedFile {
    /// The indices of the actions added.
    pub added: Vec<usize>,
    /// The (name in the file, name given) pair of each action renamed to stay unique.
    pub renames: Vec<(String, String)>,
    /// The (stored id, name given) pair of each action a PhotoCraft file gave an `action_id`, so
    /// the app moves what it keyed by the id (`actions.playById:<id>` shortcuts, calls by
    /// `actionId`) to the name. An id the file repeats is left out, as PhotoCraft ignores it.
    pub ids: Vec<(String, String)>,
}

/// Which whole-list command an undo reverts: the Actions panel's Clear All Actions, Reset
/// Actions… or Replace Actions….
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LibraryEdit {
    /// [`ActionList::clear_all`].
    ClearAll,
    /// [`ActionList::reset`].
    Reset,
    /// [`ActionList::replace_all`].
    Replace,
}

/// The list before the last whole-list command.
#[derive(Clone, Debug, PartialEq)]
struct LibrarySnapshot {
    edit: LibraryEdit,
    actions: Vec<Action>,
    sets: Vec<String>,
}

/// The recorded actions: one flat list (PhotoCraft's engine, journal and file are flat), each
/// action with an optional set name, plus the order of every set, with or without actions, so New
/// Set works before anything is recorded into it and a set keeps its place as it fills and empties
/// (VectorCraft's nested sets map onto it).
///
/// The list is app data shared by the panel, the app's commands, the CLI and the control channel:
/// the panel only reads it and reports events, and the app applies each event with these calls
/// (owner question 1, recommended answer). Every successful edit bumps [`revision`](Self::revision),
/// so a store can skip unchanged writes; the revision only moves forward, also when the whole list
/// is [replaced](Self::replace). Indices are into [`actions`](Self::actions).
///
/// Action names are unique across the whole list, whatever their sets: the app finds an action
/// by its name (calls, function keys, scripts). An edit that would repeat a name is refused, and
/// a loaded file's repeated names are renamed (`Foo`, `Foo 2`). Set names are unique likewise.
///
/// Clear All, Reset and Replace keep one undo: [`undo_library`](Self::undo_library) restores the
/// list as it was before the last of them, until any other edit ends it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionList {
    pub(super) actions: Vec<Action>,
    pub(super) sets: Vec<String>,
    pub(super) revision: u64,
    undo: Option<Box<LibrarySnapshot>>,
}

impl ActionList {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// The actions in list order.
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// The action at `index`.
    pub fn action(&self, index: usize) -> Option<&Action> {
        self.actions.get(index)
    }

    /// Every set name in display order: the order sets were made or loaded, with or without actions.
    /// A set an action names that the order lacks follows, by first appearance.
    pub fn sets(&self) -> Vec<&str> {
        let mut sets: Vec<&str> = self.sets.iter().map(String::as_str).collect();
        for set in self.actions.iter().filter_map(|action| action.set.as_deref()) {
            if !sets.contains(&set) {
                sets.push(set);
            }
        }
        sets
    }

    /// The set of the action at `index`.
    pub fn set_of(&self, index: usize) -> Option<&str> {
        self.actions.get(index).and_then(|action| action.set.as_deref())
    }

    /// The panel's order of action indices: loose actions first, then each set's actions, sets
    /// in [`sets`](Self::sets) order.
    pub fn display_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.actions.len()).filter(|&i| self.actions[i].set.is_none()).collect();
        for set in self.sets() {
            order.extend((0..self.actions.len()).filter(|&i| self.actions[i].set.as_deref() == Some(set)));
        }
        order
    }

    /// The number of steps of the action at `index` (0 when there is none).
    pub fn step_count(&self, index: usize) -> usize {
        self.actions.get(index).map_or(0, |action| action.steps.len())
    }

    /// `base` if no action has it, else `base 2`, `base 3` … the first free one. With an empty
    /// `base`, PhotoCraft's `Action n` from the list's length + 1 up.
    pub fn unique_name(&self, base: &str) -> String {
        let mut names = Names::of(&self.actions);
        if base.is_empty() {
            let mut n = self.actions.len() + 1;
            loop {
                let name = format!("Action {n}");
                if !names.taken.contains(&name) {
                    return name;
                }
                n += 1;
            }
        }
        names.claim(base)
    }

    /// Adds an empty action named `name` at the end (in `set` when given); returns its index.
    pub fn new_action(&mut self, name: &str, set: Option<String>) -> Result<usize, ActionListError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ActionListError::EmptyName);
        }
        let set = set.map(|set| set.trim().to_owned()).filter(|set| !set.is_empty());
        if self.name_in_use(name) {
            return Err(ActionListError::NameTaken(name.to_owned()));
        }
        if let Some(set) = &set {
            self.record_set(set);
        }
        self.actions.push(Action { set, ..Action::new(name, Vec::new()) });
        self.touch();
        Ok(self.actions.len() - 1)
    }

    /// Adds an empty set named `name`; refuses a taken or empty name.
    pub fn new_set(&mut self, name: &str) -> Result<(), ActionListError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ActionListError::EmptyName);
        }
        if self.sets().contains(&name) {
            return Err(ActionListError::NameTaken(name.to_owned()));
        }
        self.sets.push(name.to_owned());
        self.touch();
        Ok(())
    }

    /// Renames the action at `index` to `name` (trimmed name not empty, not taken; the same name
    /// is a success that changes nothing). `calls` sees every step of every action with the old
    /// and new names, so the app rewrites its own calls (PhotoCraft: `actions.play`'s `action`,
    /// `actions.conditional`'s `then` and `else`); the app also moves its shortcut override and
    /// rewrites its pending journal itself.
    pub fn rename(&mut self, index: usize, name: &str, mut calls: impl FnMut(&mut ActionStep, &str, &str)) -> Result<(), ActionListError> {
        let name = name.trim();
        let old = self.actions.get(index).ok_or(ActionListError::NoSuchAction(index))?.name.clone();
        if name.is_empty() {
            return Err(ActionListError::EmptyName);
        }
        if old == name {
            return Ok(());
        }
        if self.name_in_use(name) {
            return Err(ActionListError::NameTaken(name.to_owned()));
        }
        self.actions[index].name = name.to_owned();
        for step in self.actions.iter_mut().flat_map(|action| action.steps.iter_mut()) {
            calls(step, &old, name);
        }
        self.touch();
        Ok(())
    }

    /// Removes and returns the action at `index`; the set it leaves empty stays, in its place.
    pub fn delete_action(&mut self, index: usize) -> Result<Action, ActionListError> {
        if index >= self.actions.len() {
            return Err(ActionListError::NoSuchAction(index));
        }
        let action = self.actions.remove(index);
        self.touch();
        Ok(action)
    }

    /// Removes and returns step `step` of action `action`; `disabled` and `dialogs` follow (the
    /// removed index is dropped, later ones move down).
    pub fn delete_step(&mut self, action: usize, step: usize) -> Result<ActionStep, ActionListError> {
        let target = self.actions.get_mut(action).ok_or(ActionListError::NoSuchAction(action))?;
        if step >= target.steps.len() {
            return Err(ActionListError::NoSuchStep { action, step });
        }
        let removed = target.steps.remove(step);
        target.remap_steps(|i| match i.cmp(&step) {
            std::cmp::Ordering::Less => Some(i),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(i - 1),
        });
        self.touch();
        Ok(removed)
    }

    /// Moves the action at `from` to `to` (an index in the list after the removal, PhotoCraft's
    /// `actions.move`).
    pub fn move_action(&mut self, from: usize, to: usize) -> Result<(), ActionListError> {
        if from >= self.actions.len() {
            return Err(ActionListError::NoSuchAction(from));
        }
        if to >= self.actions.len() {
            return Err(ActionListError::OutOfRange);
        }
        let moved = self.actions.remove(from);
        self.actions.insert(to, moved);
        self.touch();
        Ok(())
    }

    /// Puts the action at `index` into `set` (`None`: out of any set), as a drop into or out of a
    /// set folder does. A new set goes at the end of the order; the set the action leaves stays, in
    /// its place, even when it is now empty.
    pub fn assign_set(&mut self, index: usize, set: Option<String>) -> Result<(), ActionListError> {
        if index >= self.actions.len() {
            return Err(ActionListError::NoSuchAction(index));
        }
        let set = match set {
            None => None,
            Some(set) if set.trim().is_empty() => return Err(ActionListError::EmptyName),
            Some(set) => Some(set.trim().to_owned()),
        };
        if let Some(set) = &set {
            self.record_set(set);
        }
        self.actions[index].set = set;
        self.touch();
        Ok(())
    }

    /// Moves step `step` of action `from` to position `to` of action `into` (the same action or
    /// another). Within one action `disabled` and `dialogs` follow [`moved_index`]; between
    /// actions the step takes its include and dialog flags along. A set never changes: one whose last
    /// step moves away keeps its place.
    pub fn move_step(&mut self, from: usize, step: usize, into: usize, to: usize) -> Result<(), ActionListError> {
        let source = self.actions.get(from).ok_or(ActionListError::NoSuchAction(from))?;
        if step >= source.steps.len() {
            return Err(ActionListError::NoSuchStep { action: from, step });
        }
        let target_len = self.actions.get(into).ok_or(ActionListError::NoSuchAction(into))?.steps.len();
        if from == into {
            if to >= target_len {
                return Err(ActionListError::OutOfRange);
            }
            let action = &mut self.actions[from];
            let moved = action.steps.remove(step);
            action.steps.insert(to, moved);
            action.remap_steps(|i| Some(moved_index(i, step, to)));
        } else {
            if to > target_len {
                return Err(ActionListError::OutOfRange);
            }
            let (enabled, dialog) = (source.step_enabled(step), source.dialogs.contains(&step));
            let source = &mut self.actions[from];
            let moved = source.steps.remove(step);
            source.remap_steps(|i| match i.cmp(&step) {
                std::cmp::Ordering::Less => Some(i),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(i - 1),
            });
            let target = &mut self.actions[into];
            target.steps.insert(to, moved);
            target.remap_steps(|i| Some(if i >= to { i + 1 } else { i }));
            if !enabled {
                target.disabled.insert(to);
            }
            if dialog {
                target.dialogs.insert(to);
            }
        }
        self.touch();
        Ok(())
    }

    /// Switches step `step` of `action` on or off for playback; returns its new state.
    pub fn toggle_step(&mut self, action: usize, step: usize) -> Result<bool, ActionListError> {
        let on = !self.action(action).ok_or(ActionListError::NoSuchAction(action))?.step_enabled(step);
        self.set_step(action, step, on)?;
        Ok(on)
    }

    /// Sets whether playback runs step `step` of `action`: the panel's include cell on a step
    /// row, as a [`Toggle`](crate::ui::actions::ActionsEvent::Toggle) event reports it.
    pub fn set_step(&mut self, action: usize, step: usize, on: bool) -> Result<(), ActionListError> {
        let target = self.actions.get_mut(action).ok_or(ActionListError::NoSuchAction(action))?;
        if step >= target.steps.len() {
            return Err(ActionListError::NoSuchStep { action, step });
        }
        let changed = if on { target.disabled.remove(&step) } else { target.disabled.insert(step) };
        if changed {
            self.touch();
        }
        Ok(())
    }

    /// The include column on an action row: every step on unless all already are, in which case
    /// every step off. Returns whether the steps are now on.
    pub fn toggle_all(&mut self, action: usize) -> Result<bool, ActionListError> {
        let target = self.actions.get(action).ok_or(ActionListError::NoSuchAction(action))?;
        if target.steps.is_empty() {
            return Ok(false);
        }
        let enable = target.include() != ActionInclude::All;
        self.set_all(action, enable)?;
        Ok(enable)
    }

    /// Switches every step of `action` on or off: the panel's include cell on an action row, as
    /// a [`Toggle`](crate::ui::actions::ActionsEvent::Toggle) event reports it. An action with
    /// no steps stays as it is.
    pub fn set_all(&mut self, action: usize, on: bool) -> Result<(), ActionListError> {
        let target = self.actions.get_mut(action).ok_or(ActionListError::NoSuchAction(action))?;
        let disabled = if on { BTreeSet::new() } else { (0..target.steps.len()).collect() };
        if target.disabled != disabled {
            target.disabled = disabled;
            self.touch();
        }
        Ok(())
    }

    /// Switches the dialog flag of step `step` of `action`; returns its new state.
    pub fn toggle_dialog(&mut self, action: usize, step: usize) -> Result<bool, ActionListError> {
        let on = !self.action(action).ok_or(ActionListError::NoSuchAction(action))?.dialogs.contains(&step);
        self.set_dialog(action, step, on)?;
        Ok(on)
    }

    /// Sets the dialog flag of step `step` of `action`: the panel's dialog cell, as a
    /// [`Toggle`](crate::ui::actions::ActionsEvent::Toggle) event reports it.
    pub fn set_dialog(&mut self, action: usize, step: usize, on: bool) -> Result<(), ActionListError> {
        let target = self.actions.get_mut(action).ok_or(ActionListError::NoSuchAction(action))?;
        if step >= target.steps.len() {
            return Err(ActionListError::NoSuchStep { action, step });
        }
        let changed = if on { target.dialogs.insert(step) } else { target.dialogs.remove(&step) };
        if changed {
            self.touch();
        }
        Ok(())
    }

    /// Sets the Button Mode colour of the action at `index`.
    pub fn set_color(&mut self, index: usize, color: ActionColor) -> Result<(), ActionListError> {
        self.actions.get_mut(index).ok_or(ActionListError::NoSuchAction(index))?.color = color;
        self.touch();
        Ok(())
    }

    /// Replaces the app's own options of the action at `index` ([`Action::options`]). Setting the options it holds
    /// changes nothing, so the revision stays.
    pub fn set_options(&mut self, index: usize, options: serde_json::Map<String, serde_json::Value>) -> Result<(), ActionListError> {
        let action = self.actions.get_mut(index).ok_or(ActionListError::NoSuchAction(index))?;
        if action.options != options {
            action.options = options;
            self.touch();
        }
        Ok(())
    }

    /// Appends a whole action and returns its index; an empty or taken name is refused.
    pub fn push(&mut self, action: Action) -> Result<usize, ActionListError> {
        if action.name.trim().is_empty() {
            return Err(ActionListError::EmptyName);
        }
        if self.name_in_use(&action.name) {
            return Err(ActionListError::NameTaken(action.name));
        }
        if let Some(set) = &action.set {
            self.record_set(set);
        }
        self.actions.push(action);
        self.touch();
        Ok(self.actions.len() - 1)
    }

    /// Inserts `step` at `at` in action `action`; `disabled` and `dialogs` at or after `at` move
    /// up.
    pub fn insert_step(&mut self, action: usize, at: usize, step: ActionStep) -> Result<(), ActionListError> {
        let target = self.actions.get_mut(action).ok_or(ActionListError::NoSuchAction(action))?;
        if at > target.steps.len() {
            return Err(ActionListError::OutOfRange);
        }
        target.steps.insert(at, step);
        target.remap_steps(|i| Some(if i >= at { i + 1 } else { i }));
        self.touch();
        Ok(())
    }

    /// Inserts a copy of action `index` below itself under a unique name and returns its index;
    /// `shift(step, at, count)` sees every step of every action so the app moves its calls by
    /// index.
    pub fn duplicate_action(&mut self, index: usize, mut shift: impl FnMut(&mut ActionStep, usize, usize)) -> Result<usize, ActionListError> {
        let mut copy = self.actions.get(index).ok_or(ActionListError::NoSuchAction(index))?.clone();
        copy.name = self.unique_name(&format!("{} copy", copy.name));
        let at = index + 1;
        self.actions.insert(at, copy);
        self.shift_calls(at, 1, &mut shift);
        self.touch();
        Ok(at)
    }

    /// Inserts a copy of step `step` of action `action` after itself, with its include and dialog
    /// flags.
    pub fn duplicate_step(&mut self, action: usize, step: usize) -> Result<(), ActionListError> {
        let source = self.actions.get(action).ok_or(ActionListError::NoSuchAction(action))?;
        let copied = source.steps.get(step).cloned().ok_or(ActionListError::NoSuchStep { action, step })?;
        let (disabled, dialog) = (!source.step_enabled(step), source.dialogs.contains(&step));
        let at = step + 1;
        self.insert_step(action, at, copied)?;
        let target = &mut self.actions[action];
        if disabled {
            target.disabled.insert(at);
        }
        if dialog {
            target.dialogs.insert(at);
        }
        Ok(())
    }

    /// Renames the set `set` to `name` (trimmed) in the order and in every member's `set`: [`ActionListError::EmptyName`]
    /// for an empty name, [`ActionListError::NameTaken`] for a name another set has, [`ActionListError::NoSuchSet`] for an
    /// unknown set. The same name succeeds and changes nothing.
    pub fn rename_set(&mut self, set: &str, name: &str) -> Result<(), ActionListError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ActionListError::EmptyName);
        }
        let order: Vec<String> = self.sets().into_iter().map(str::to_owned).collect();
        let at = order.iter().position(|each| each == set).ok_or_else(|| ActionListError::NoSuchSet(set.to_owned()))?;
        if name == set {
            return Ok(());
        }
        if order.iter().any(|each| each == name) {
            return Err(ActionListError::NameTaken(name.to_owned()));
        }
        self.sets = order;
        self.sets[at] = name.to_owned();
        for action in self.actions.iter_mut().filter(|action| action.set.as_deref() == Some(set)) {
            action.set = Some(name.to_owned());
        }
        self.touch();
        Ok(())
    }

    /// Removes the set `set` from the order and its actions from the list, and returns the actions in list order:
    /// [`ActionListError::NoSuchSet`] for an unknown set.
    pub fn delete_set(&mut self, set: &str) -> Result<Vec<Action>, ActionListError> {
        if !self.sets().contains(&set) {
            return Err(ActionListError::NoSuchSet(set.to_owned()));
        }
        let (removed, kept): (Vec<Action>, Vec<Action>) =
            std::mem::take(&mut self.actions).into_iter().partition(|action| action.set.as_deref() == Some(set));
        self.actions = kept;
        self.sets.retain(|each| each != set);
        self.touch();
        Ok(removed)
    }

    /// Duplicates the set `set` under a unique name and returns that name; `shift` is as for
    /// [`duplicate_action`](Self::duplicate_action). The new set goes right after `set` in the order;
    /// the copies of its actions go after its last action, and a set with none gives an empty set.
    pub fn duplicate_set(&mut self, set: &str, mut shift: impl FnMut(&mut ActionStep, usize, usize)) -> Result<String, ActionListError> {
        let mut order: Vec<String> = self.sets().into_iter().map(str::to_owned).collect();
        let position = order.iter().position(|each| each == set).ok_or_else(|| ActionListError::NoSuchSet(set.to_owned()))?;
        let name = Names::new(order.iter().cloned()).claim(&format!("{set} copy"));
        order.insert(position + 1, name.clone());
        self.sets = order;
        let members: Vec<usize> = (0..self.actions.len()).filter(|&i| self.actions[i].set.as_deref() == Some(set)).collect();
        if let Some(&last) = members.last() {
            let at = last + 1;
            // Each copy's name is claimed before the next, so the copies never repeat one.
            let mut names = Names::of(&self.actions);
            for (offset, &source) in members.iter().enumerate() {
                let mut copy = self.actions[source].clone();
                copy.set = Some(name.clone());
                copy.name = names.claim(&format!("{} copy", copy.name));
                self.actions.insert(at + offset, copy);
            }
            self.shift_calls(at, members.len(), &mut shift);
        }
        self.touch();
        Ok(name)
    }

    /// Replaces step `step` of action `action` with `with` and returns the step it replaced.
    pub fn replace_step(&mut self, action: usize, step: usize, with: ActionStep) -> Result<ActionStep, ActionListError> {
        let target = self.actions.get_mut(action).ok_or(ActionListError::NoSuchAction(action))?;
        let slot = target.steps.get_mut(step).ok_or(ActionListError::NoSuchStep { action, step })?;
        let replaced = std::mem::replace(slot, with);
        self.touch();
        Ok(replaced)
    }

    /// Marks action `index` as written (or not) by an automation client. Marking it as it already
    /// is changes nothing, so the revision stays.
    pub fn set_automation(&mut self, index: usize, on: bool) -> Result<(), ActionListError> {
        let action = self.actions.get_mut(index).ok_or(ActionListError::NoSuchAction(index))?;
        if action.automation != on {
            action.automation = on;
            self.touch();
        }
        Ok(())
    }

    /// Adds the actions of `file`, each under a name unique in the list; with `replace`, the
    /// actions of every set the file names are dropped first. The file's empty sets are added when
    /// absent. `calls(step, &renames)` runs once on every step of every action just added, after
    /// all renames are known (also when there are none).
    ///
    /// A PhotoCraft file's folder occurrences become sets: the first occurrence of a set name
    /// keeps it (and joins a set of that name already in the list, as a plain file's set does),
    /// each later one gets a unique name (`Set 2`), and an empty one becomes an empty set. Of the
    /// actions sharing a name, the file's shortcut owner keeps the name. The stored ids come back
    /// in [`MergedFile::ids`].
    pub fn merge_file(&mut self, file: ActionsFile, replace: bool, mut calls: impl FnMut(&mut ActionStep, &[(String, String)])) -> MergedFile {
        let ActionsFile { mut actions, sets, set_order, occurrences, .. } = file;
        if replace {
            let named: BTreeSet<&str> = actions
                .iter()
                .filter_map(|action| action.set.as_deref())
                .chain(sets.iter().map(String::as_str))
                .chain(set_order.iter().map(String::as_str))
                .chain(occurrences.set_rows.iter().map(String::as_str))
                .collect();
            self.actions.retain(|action| !action.set.as_deref().is_some_and(|set| named.contains(set)));
        }
        let fields = |index: usize| occurrences.actions.get(index).cloned().unwrap_or_default();
        let empty_sets = self.fold_occurrences(&mut actions, sets, &occurrences.set_rows, &fields);
        // The shortcut owners claim first, so each keeps its name over an earlier namesake.
        let owners = (0..actions.len()).filter(|&i| fields(i).legacy_shortcut_owner);
        let order: Vec<usize> = owners.chain((0..actions.len()).filter(|&i| !fields(i).legacy_shortcut_owner)).collect();
        let claimed = Names::of(&self.actions).claim_all(order.iter().map(|&i| actions[i].name.as_str()));
        let mut given = vec![String::new(); actions.len()];
        for (&i, name) in order.iter().zip(claimed) {
            given[i] = name;
        }
        for set in &set_order {
            self.record_set(set);
        }
        let mut merged = MergedFile::default();
        let ids = file_ids(&occurrences.actions);
        for (index, (mut action, name)) in actions.into_iter().zip(given).enumerate() {
            if name != action.name {
                merged.renames.push((action.name.clone(), name.clone()));
            }
            if let Some(Some(id)) = ids.get(index) {
                merged.ids.push((id.clone(), name.clone()));
            }
            action.name = name;
            if let Some(set) = &action.set {
                self.record_set(set);
            }
            self.actions.push(action);
            merged.added.push(self.actions.len() - 1);
        }
        for &index in &merged.added {
            for step in &mut self.actions[index].steps {
                calls(step, &merged.renames);
            }
        }
        for set in empty_sets {
            self.record_set(&set);
        }
        self.touch();
        merged
    }

    /// Gives each action of a file the set name of its folder occurrence and returns the file's
    /// empty sets: the plain ones, then each occurrence that holds no action. The first
    /// occurrence of a name keeps it; a later one, or one whose name a plain set of the file has,
    /// claims a name unique among the list's sets and the file's.
    fn fold_occurrences(
        &self,
        actions: &mut [Action],
        sets: Vec<String>,
        set_rows: &[String],
        fields: &dyn Fn(usize) -> ActionOccurrence,
    ) -> Vec<String> {
        if set_rows.is_empty() {
            return sets;
        }
        // An action's slot counts only where the file's occurrence still has its set name.
        let slot_of =
            |index: usize, action: &Action| fields(index).set_slot.filter(|&slot| set_rows.get(slot).map(String::as_str) == action.set.as_deref());
        let plain: HashSet<&str> = actions
            .iter()
            .enumerate()
            .filter(|(index, action)| slot_of(*index, action).is_none())
            .filter_map(|(_, action)| action.set.as_deref())
            .chain(sets.iter().map(String::as_str))
            .collect();
        let mut names = Names::new(self.sets().into_iter().chain(plain.iter().copied()).map(str::to_owned));
        let mut first: HashSet<&str> = HashSet::new();
        let slot_names: Vec<String> = set_rows
            .iter()
            .map(|caption| {
                if !plain.contains(caption.as_str()) && first.insert(caption.as_str()) {
                    names.taken.insert(caption.clone());
                    caption.clone()
                } else {
                    names.claim(caption)
                }
            })
            .collect();
        let mut filled = vec![false; set_rows.len()];
        let slots: Vec<Option<usize>> = actions.iter().enumerate().map(|(index, action)| slot_of(index, action)).collect();
        for (action, slot) in actions.iter_mut().zip(slots) {
            if let Some(slot) = slot {
                action.set = Some(slot_names[slot].clone());
                filled[slot] = true;
            }
        }
        let mut empty = sets;
        empty.extend(slot_names.into_iter().zip(filled).filter(|(_, filled)| !filled).map(|(name, _)| name));
        empty
    }

    /// The list a file holds (its empty sets included), named as [`merge_file`](Self::merge_file)
    /// names a file's actions and sets in an empty list: a name the file repeats is renamed
    /// (`Foo`, `Foo 2`), and a call by that name keeps reaching the first action that has it. The
    /// app that keys anything by a PhotoCraft file's stored ids uses [`load`](Self::load).
    pub fn from_file(file: ActionsFile) -> Self {
        Self::load(file).0
    }

    /// [`from_file`](Self::from_file) with what loading renamed and the stored ids.
    pub fn load(file: ActionsFile) -> (Self, MergedFile) {
        let mut list = Self::new();
        let merged = list.merge_file(file, false, |_, _| {});
        list.revision = 0;
        (list, merged)
    }

    /// Clear All Actions: no action and no set. [`undo_library`](Self::undo_library) restores
    /// them.
    pub fn clear_all(&mut self) {
        self.library_edit(LibraryEdit::ClearAll, |list| {
            list.actions.clear();
            list.sets.clear();
        });
    }

    /// Reset Actions…: the app's `defaults` (its starting sets), added to the list with `append`
    /// (renamed where a name is taken, as [`merge_file`](Self::merge_file) does), else in place
    /// of everything. `calls` is as for `merge_file`.
    pub fn reset(&mut self, defaults: ActionsFile, append: bool, calls: impl FnMut(&mut ActionStep, &[(String, String)])) -> MergedFile {
        self.library_edit(LibraryEdit::Reset, |list| {
            if !append {
                list.actions.clear();
                list.sets.clear();
            }
            list.merge_file(defaults, false, calls)
        })
    }

    /// Replace Actions…: the list becomes what `file` holds, named as [`load`](Self::load) names
    /// it. `calls` is as for [`merge_file`](Self::merge_file).
    pub fn replace_all(&mut self, file: ActionsFile, calls: impl FnMut(&mut ActionStep, &[(String, String)])) -> MergedFile {
        self.library_edit(LibraryEdit::Replace, |list| {
            list.actions.clear();
            list.sets.clear();
            list.merge_file(file, false, calls)
        })
    }

    /// The whole-list command [`undo_library`](Self::undo_library) would revert, for the app's
    /// Undo label; `None` when an edit since ended it.
    pub fn library_undo(&self) -> Option<LibraryEdit> {
        self.undo.as_ref().map(|snapshot| snapshot.edit)
    }

    /// Restores the list as it was before the last Clear All, Reset or Replace and returns which
    /// one it reverted.
    pub fn undo_library(&mut self) -> Result<LibraryEdit, ActionListError> {
        let snapshot = self.undo.take().ok_or(ActionListError::NothingToUndo)?;
        let LibrarySnapshot { edit, actions, sets } = *snapshot;
        self.actions = actions;
        self.sets = sets;
        self.touch();
        Ok(edit)
    }

    /// Runs a whole-list command and keeps the list from before it as the one undo.
    fn library_edit<R>(&mut self, edit: LibraryEdit, apply: impl FnOnce(&mut Self) -> R) -> R {
        let snapshot = LibrarySnapshot { edit, actions: self.actions.clone(), sets: self.sets.clone() };
        let result = apply(self);
        self.touch();
        self.undo = Some(Box::new(snapshot));
        result
    }

    /// Replaces the whole list with `with` (a list loaded or restored from elsewhere): its actions
    /// and sets, under this list's revision bumped once, so the revision only moves forward and a
    /// store that compares revisions sees the change.
    pub fn replace(&mut self, with: ActionList) {
        let revision = self.revision;
        *self = Self { revision, ..with };
        self.touch();
    }

    /// The list as a file (version [`ActionsFile::VERSION`]; the empty sets in `sets`, the set order in `set_order` when it is not the natural one).
    pub fn to_file(&self) -> ActionsFile {
        let empty = self.sets().into_iter().filter(|set| !self.actions.iter().any(|action| action.set.as_deref() == Some(*set)));
        let mut file = ActionsFile::new(self.actions.clone(), empty.map(str::to_owned).collect());
        let mut natural: Vec<&str> = Vec::new();
        for set in self.actions.iter().filter_map(|action| action.set.as_deref()).chain(file.sets.iter().map(String::as_str)) {
            if !natural.contains(&set) {
                natural.push(set);
            }
        }
        if natural != self.sets() {
            file.set_order = self.sets().into_iter().map(str::to_owned).collect();
        }
        file
    }

    /// Puts `set` at the end of the set order when the order lacks it.
    fn record_set(&mut self, set: &str) {
        if !self.sets.iter().any(|each| each == set) {
            self.sets.push(set.to_owned());
        }
    }

    /// Whether an action has `name`.
    fn name_in_use(&self, name: &str) -> bool {
        self.actions.iter().any(|action| action.name == name)
    }

    /// Calls `shift(step, at, count)` on every step of every action, after `count` actions were
    /// inserted at `at`.
    fn shift_calls(&mut self, at: usize, count: usize, shift: &mut impl FnMut(&mut ActionStep, usize, usize)) {
        for step in self.actions.iter_mut().flat_map(|action| action.steps.iter_mut()) {
            shift(step, at, count);
        }
    }

    /// Bumped by every successful edit.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Marks the list changed by an edit made outside these calls (the app replaced an action's
    /// steps). Every edit ends the undo of the last Clear All, Reset or Replace.
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.undo = None;
    }
}

/// Each action's stored id, where it is PhotoCraft's form (32 lowercase hex digits) and no other
/// action of the file has it.
fn file_ids(occurrences: &[ActionOccurrence]) -> Vec<Option<String>> {
    let valid = |id: &&String| id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let mut count: HashMap<&str, usize> = HashMap::new();
    for id in occurrences.iter().filter_map(|occurrence| occurrence.action_id.as_ref()).filter(valid) {
        *count.entry(id).or_default() += 1;
    }
    occurrences.iter().map(|occurrence| occurrence.action_id.clone().filter(|id| valid(&id) && count.get(id.as_str()) == Some(&1))).collect()
}

/// The names taken so far and, per base name, the next suffix to try, so claiming one base many
/// times (a file that repeats a name) stays linear.
#[derive(Default)]
struct Names {
    taken: HashSet<String>,
    next: HashMap<String, u32>,
}

impl Names {
    fn new(taken: impl IntoIterator<Item = String>) -> Self {
        Self { taken: taken.into_iter().collect(), next: HashMap::new() }
    }

    /// The names of `actions`.
    fn of(actions: &[Action]) -> Self {
        Self::new(actions.iter().map(|action| action.name.clone()))
    }

    /// The names for `bases`, in order. Every base that is free keeps its name, even when an
    /// earlier repeat of another base would have picked it as a suffix; only the repeats and the
    /// clashes with taken names go through [`claim`](Self::claim).
    fn claim_all<'a>(&mut self, bases: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let bases: Vec<&str> = bases.into_iter().collect();
        let free: Vec<bool> = bases.iter().map(|base| self.taken.insert((*base).to_owned())).collect();
        bases.iter().zip(free).map(|(base, free)| if free { (*base).to_owned() } else { self.claim(base) }).collect()
    }

    /// `base` if it is free, else `base 2`, `base 3` … the first free one; the name returned is
    /// taken from then on.
    fn claim(&mut self, base: &str) -> String {
        if self.taken.insert(base.to_owned()) {
            return base.to_owned();
        }
        let mut n = self.next.get(base).copied().unwrap_or(2);
        loop {
            let name = format!("{base} {n}");
            n += 1;
            if self.taken.insert(name.clone()) {
                self.next.insert(base.to_owned(), n);
                return name;
            }
        }
    }
}

/// Where the item at `index` ends up after the item at `from` moved to `to`: the moved item goes
/// to `to`, items between shift by one toward `from`, others stay. A selection or the recording
/// index follows a move with it.
pub fn moved_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}
