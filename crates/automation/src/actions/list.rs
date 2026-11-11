//! [`ActionList`]: the flat list of actions with an optional set each, and every edit the panel
//! and the app's commands need.

use std::collections::BTreeSet;

use super::file::ActionsFile;
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
    /// Another action (or set) already has the name.
    NameTaken(String),
    /// The name is empty or only white space.
    EmptyName,
}

impl std::fmt::Display for ActionListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionListError::NoSuchAction(_) => f.write_str("no such action"),
            ActionListError::NoSuchStep { .. } => f.write_str("no such step"),
            ActionListError::OutOfRange => f.write_str("destination is out of range"),
            ActionListError::NameTaken(_) => f.write_str("an action with this name already exists"),
            ActionListError::EmptyName => f.write_str("\"name\" is empty"),
        }
    }
}

impl std::error::Error for ActionListError {}

/// The recorded actions: one flat list (PhotoCraft's engine, journal and file are flat), each
/// action with an optional set name, plus the names of sets that hold no action yet, so New Set
/// works before anything is recorded into it (VectorCraft's nested sets map onto
/// it).
///
/// The list is app data shared by the panel, the app's commands, the CLI and the control channel:
/// the panel only reads it and reports events, and the app applies each event with these calls
/// (owner question 1, recommended answer). Every successful edit bumps [`revision`](Self::revision),
/// so a store can skip unchanged writes. Indices are into [`actions`](Self::actions).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionList {
    pub(super) actions: Vec<Action>,
    pub(super) sets: Vec<String>,
    pub(super) revision: u64,
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

    /// Every set name in display order: sets of actions in order of first appearance, then the
    /// empty sets in the order they were made.
    pub fn sets(&self) -> Vec<&str> {
        let mut sets: Vec<&str> = Vec::new();
        let named = self.actions.iter().filter_map(|action| action.set.as_deref());
        for set in named.chain(self.sets.iter().map(String::as_str)) {
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
        if base.is_empty() {
            let mut n = self.actions.len() + 1;
            loop {
                let name = format!("Action {n}");
                if !self.name_in_use(&name) {
                    return name;
                }
                n += 1;
            }
        }
        if !self.name_in_use(base) {
            return base.to_owned();
        }
        (2..).map(|n| format!("{base} {n}")).find(|name| !self.name_in_use(name)).unwrap_or_else(|| base.to_owned())
    }

    /// Adds an empty action named `name` at the end (in `set` when given); returns its index.
    pub fn new_action(&mut self, name: &str, set: Option<String>) -> Result<usize, ActionListError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ActionListError::EmptyName);
        }
        if self.name_in_use(name) {
            return Err(ActionListError::NameTaken(name.to_owned()));
        }
        let set = set.map(|set| set.trim().to_owned()).filter(|set| !set.is_empty());
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

    /// Removes and returns the action at `index`.
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
    /// set folder does.
    pub fn assign_set(&mut self, index: usize, set: Option<String>) -> Result<(), ActionListError> {
        let target = self.actions.get_mut(index).ok_or(ActionListError::NoSuchAction(index))?;
        let set = match set {
            None => None,
            Some(set) if set.trim().is_empty() => return Err(ActionListError::EmptyName),
            Some(set) => Some(set.trim().to_owned()),
        };
        target.set = set;
        self.touch();
        Ok(())
    }

    /// Moves step `step` of action `from` to position `to` of action `into` (the same action or
    /// another). Within one action `disabled` and `dialogs` follow [`moved_index`]; between
    /// actions the step takes its include and dialog flags along.
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

    /// The list a file holds (its empty sets included).
    pub fn from_file(file: ActionsFile) -> Self {
        Self { actions: file.actions, sets: file.sets, revision: 0 }
    }

    /// The list as a file (version [`ActionsFile::VERSION`]; the empty sets in `sets`).
    pub fn to_file(&self) -> ActionsFile {
        let empty = self.sets().into_iter().filter(|set| !self.actions.iter().any(|action| action.set.as_deref() == Some(*set)));
        ActionsFile { version: ActionsFile::VERSION, actions: self.actions.clone(), sets: empty.map(str::to_owned).collect() }
    }

    fn name_in_use(&self, name: &str) -> bool {
        self.actions.iter().any(|action| action.name == name)
    }

    /// Bumped by every successful edit.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Marks the list changed by an edit made outside these calls (the app replaced an action's
    /// steps).
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
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
