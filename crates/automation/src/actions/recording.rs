//! [`Recording`]: steps the app's journal gives, appended to an action.

use super::list::{ActionList, ActionListError};
use super::model::ActionStep;

/// A recording under way: the action it records into and where the app's journal stood when it
/// started. The app keeps it (PhotoCraft's `ActionState::recording`), filters its journal past
/// `journal_start` for replayable commands (its own rule: not `actions.*` edits, not
/// `edit.transform.again`), and appends them with [`ActionList::append_recorded`] on Stop or
/// before an edit of that action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Recording {
    /// The action recorded into.
    pub action: usize,
    /// The app's journal length when the recording started or last flushed.
    pub journal_start: usize,
}

impl ActionList {
    /// Starts a recording: into the action at `into`, or into a new action with a
    /// [unique name](ActionList::unique_name) (in `set` when given) when `into` is `None`.
    /// `journal_start` is the app's journal length now.
    pub fn begin_recording(&mut self, into: Option<usize>, set: Option<String>, journal_start: usize) -> Result<Recording, ActionListError> {
        let index = match into {
            Some(index) => {
                if index >= self.actions.len() {
                    return Err(ActionListError::NoSuchAction(index));
                }
                index
            }
            None => {
                let name = self.unique_name("");
                self.new_action(&name, set)?
            }
        };
        Ok(Recording { action: index, journal_start })
    }

    /// Appends `steps` to the action at `index` (a flush of the journal while recording).
    pub fn append_recorded(&mut self, index: usize, steps: impl IntoIterator<Item = ActionStep>) -> Result<(), ActionListError> {
        let action = self.actions.get_mut(index).ok_or(ActionListError::NoSuchAction(index))?;
        let before = action.steps.len();
        action.steps.extend(steps);
        if action.steps.len() != before {
            self.touch();
        }
        Ok(())
    }
}
