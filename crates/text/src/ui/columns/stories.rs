//! Stories of the Columns dialog and the page preview.

use super::{ColumnsApply, ColumnsPreview, TextColumns, TextColumnsDialog};
use craft_ui::contract::Show as _;
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The text width of the Word story, points: a 6.5 in text width.
const TEXT_WIDTH: f32 = 468.0;

#[story(TextColumnsDialog, covers = [Default], drive = Drive::Never)]
fn three_columns(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let columns = TextColumns { count: Some(3), spacing: Some(36.0), equal_width: Some(true), line_between: Some(true), ..TextColumns::default() };
    let mut dialog = TextColumnsDialog::new(columns)
        .word(TEXT_WIDTH)
        .apply_to(&[ColumnsApply::Section, ColumnsApply::Document, ColumnsApply::Forward], ColumnsApply::Document);
    let _ = dialog.show(ui, &DialogStack::default());
    StoryOutput::default()
}

#[story(ColumnsPreview, covers = [Default], drive = Drive::Never)]
fn preview(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let rows = vec![(140.0, 24.0), (140.0, 24.0), (140.0, 0.0)];
    ColumnsPreview::new("Preview", rows, TEXT_WIDTH, true).show(ui).into()
}
