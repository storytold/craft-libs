//! Stories of the chord recorder.

use super::{ChordRecorder, Recorder};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(Recorder, covers = [Default], drive = Drive::Never)]
fn chord_recorder(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ChordRecorder::new("Shortcut", false, "Cmd+K").id_salt("recorder").show(ui).response.into()
}

#[story(Recorder, covers = [Default], drive = Drive::Never)]
fn chord_recorder_recording(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ChordRecorder::new("Shortcut", true, "press keys").id_salt("recording").show(ui).response.into()
}
