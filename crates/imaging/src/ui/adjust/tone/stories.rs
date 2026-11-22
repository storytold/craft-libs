//! Stories of the point picker row: one picker armed, and the row in a colour mode without pickers.

use super::{TonePicker, TonePickerRow};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(TonePickerRow<'static>, covers = [Default], drive = Drive::Never)]
fn tone_picker_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (armed, armed_at) = data.value_at("armed", Some(TonePicker::Gray));
    TonePickerRow::new(armed_at.bind(armed)).show(ui).into()
}

#[story(TonePickerRow<'static>, covers = [], drive = Drive::Never)]
fn tone_picker_row_unavailable(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (armed, armed_at) = data.value_at("armed", None::<TonePicker>);
    TonePickerRow::new(armed_at.bind(armed)).unavailable().show(ui).into()
}
