//! Recorded actions (F13.06): the action list, its file form, playback and recording. Builds on
//! every target, the browser included.
//!
//! | Part | Names |
//! |---|---|
//! | The model | [`Action`], [`ActionStep`], [`ActionColor`], [`ActionInclude`] |
//! | The list and its edits | [`ActionList`], [`ActionListError`], [`moved_index`] |
//! | The file | [`ActionsFile`] (PhotoCraft's `actions.json`) |
//! | Playback | [`ActionPlayer`], [`ActionRunner`], [`StepOutcome`], [`PlaybackBudget`], [`PlaybackReport`], [`StepFailure`], [`PlaybackRefusal`], [`MAX_PLAYBACK_STEPS`], [`MAX_PLAYBACK_DEPTH`] |
//! | Recording | [`Recording`], [`ActionList::begin_recording`], [`ActionList::append_recorded`] |
//!
//! The app keeps the command table: what a step id means, which journal entries are replayable,
//! `.atn` import and export, shortcuts, and the pause-for-dialog resume. The Actions panel
//! ([`crate::ui::actions`]) reads an [`ActionList`] and reports events; the app applies them with
//! these calls, so its CLI and control channel share them.

mod file;
mod list;
mod model;
mod player;
mod recording;
#[cfg(test)]
mod tests;

pub use file::ActionsFile;
pub use list::{ActionList, ActionListError, moved_index};
pub use model::{Action, ActionColor, ActionInclude, ActionStep};
pub use player::{
    ActionPlayer, ActionRunner, MAX_PLAYBACK_DEPTH, MAX_PLAYBACK_STEPS, PlaybackBudget, PlaybackRefusal, PlaybackReport, StepFailure, StepOutcome,
};
pub use recording::Recording;
