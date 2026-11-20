//! `EditHistory<T>`: the undo and redo of a dialog, a tool session or the keymap editor, fed by edit events.

use crate::value::{EditEvent, GestureId, Mixed, Value};
use std::sync::Arc;

/// Most steps a history keeps.
const LIMIT: usize = 100;

/// Which way [`EditHistory::step`] goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Step {
    /// Back to the value before the latest step.
    Undo,
    /// Forward again after an undo.
    Redo,
}

/// One step: the value it leaves and the name the undo entry shows.
#[derive(Clone, Debug)]
struct Entry<T> {
    value: T,
    name: Arc<str>,
}

/// The gesture under way: its name, taken from its `Begin`.
#[derive(Clone, Debug)]
struct Open {
    id: GestureId,
    name: Arc<str>,
}

/// Undo and redo stacks of a value `T`, at most 100 steps. A gesture is one step: `Begin` opens it, `Commit` closes it,
/// `Cancel` drops it, and a `Set` is one step by itself. A commit that leaves the value as it was records nothing.
#[derive(Clone, Debug)]
pub struct EditHistory<T> {
    undo: Vec<Entry<T>>,
    redo: Vec<Entry<T>>,
    /// The value as of the latest step.
    settled: T,
    open: Option<Open>,
}

impl<T: Value> EditHistory<T> {
    /// A history that starts at `now`.
    pub fn new(now: &T) -> Self {
        EditHistory { undo: Vec::new(), redo: Vec::new(), settled: now.clone(), open: None }
    }

    /// Takes in one event. Events of a gesture other than the open one are ignored. A concrete
    /// before value captures changes made outside history since the preceding edit.
    pub fn record(&mut self, event: &EditEvent<T>) {
        match event {
            EditEvent::Begin { id, label, .. } => self.open = Some(Open { id: *id, name: label.clone() }),
            EditEvent::Update { .. } => {}
            EditEvent::Commit { id, before, after } => {
                if let Some(open) = self.open.take_if(|open| open.id == *id) {
                    self.push(before, after, open.name);
                }
            }
            EditEvent::Cancel { id, .. } => {
                if self.open.as_ref().is_some_and(|open| open.id == *id) {
                    self.open = None;
                }
            }
            EditEvent::Set { before, after, label, .. } => {
                self.open = None;
                self.push(before, after, label.clone());
            }
        }
    }

    /// A gesture is open: a caller that restores a draft waits for it to end.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    fn push(&mut self, before: &Mixed<T>, after: &T, name: Arc<str>) {
        let before = before.same().unwrap_or(&self.settled).clone();
        self.settled = after.clone();
        if *after == before {
            return;
        }
        if self.undo.len() >= LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(Entry { value: before, name });
        self.redo.clear();
    }

    /// Steps `now` back or forward. False when there is nowhere to go. An open gesture is abandoned.
    pub fn step(&mut self, now: &mut T, step: Step) -> bool {
        self.open = None;
        let (from, to) = match step {
            Step::Undo => (&mut self.undo, &mut self.redo),
            Step::Redo => (&mut self.redo, &mut self.undo),
        };
        let Some(entry) = from.pop() else { return false };
        to.push(Entry { value: std::mem::replace(now, entry.value.clone()), name: entry.name });
        self.settled = entry.value;
        true
    }

    /// Takes back the newest undo step without a redo entry, for an edit that was refused: `settled` returns to the
    /// value before the step, which is returned for the caller to apply, and the redo steps stay as they were. `None`
    /// when there is no step.
    pub fn take_back(&mut self) -> Option<T> {
        self.open = None;
        let entry = self.undo.pop()?;
        self.settled = entry.value.clone();
        Some(entry.value)
    }

    /// An undo would change something.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// A redo would change something.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// How many steps a redo can go forward.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Mixed;

    fn set(before: i32, after: i32) -> EditEvent<i32> {
        EditEvent::set(Mixed::Same(before), after, "Fill".into(), None)
    }

    #[test]
    fn undo_restores_selection_changed_between_edits() {
        let mut history = EditHistory::new(&(0, 0));
        history.record(&EditEvent::set(Mixed::Same((0, 1)), (1, 0), "Remove".into(), None));
        let mut now = (1, 0);
        assert!(history.step(&mut now, Step::Undo));
        assert_eq!(now, (0, 1));
        assert!(history.step(&mut now, Step::Redo));
        assert_eq!(now, (1, 0));
    }

    #[test]
    fn a_take_back_returns_the_earlier_value_keeps_redo_and_settles() {
        let mut history = EditHistory::new(&0);
        history.record(&set(0, 1));
        history.record(&set(1, 2));
        history.record(&set(2, 5));
        let mut now = 5;
        assert!(history.step(&mut now, Step::Undo));
        assert_eq!(now, 2);
        assert!(history.can_redo());

        assert_eq!(history.take_back(), Some(1), "the value before the newest step");
        assert!(history.can_redo(), "redo is as it was");
        let mut now = 1;
        assert!(history.step(&mut now, Step::Undo));
        assert_eq!(now, 0, "the undo takes the step before the one taken back");
    }

    #[test]
    fn redo_len_counts_the_undone_steps_until_a_new_step() {
        let mut history = EditHistory::new(&0);
        history.record(&set(0, 1));
        history.record(&set(1, 2));
        let mut now = 2;
        assert!(history.step(&mut now, Step::Undo));
        assert!(history.step(&mut now, Step::Undo));
        assert_eq!(history.redo_len(), 2);
        assert!(history.step(&mut now, Step::Redo));
        assert_eq!(history.redo_len(), 1);
        history.record(&set(1, 7));
        assert_eq!(history.redo_len(), 0, "a new step drops the redo steps");
    }
}
