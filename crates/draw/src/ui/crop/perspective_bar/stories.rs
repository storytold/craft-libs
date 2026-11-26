//! Stories of the Perspective Crop options bar.

use super::super::options::{CropLength, CropSizeFields};
use super::super::session::{CropDocKey, CropDocument};
use super::PerspectiveCropBar;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::ShowContainer as _;
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use kurbo::Size;

#[story(PerspectiveCropBar<'static>, covers = [], drive = Drive::Never)]
fn perspective_crop_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let document = CropDocument::new(CropDocKey::new(1), Size::new(2400.0, 1500.0), 72.0);
    let fields = CropSizeFields {
        width: Some(CropLength::new(1200.0, UnitKind::Pixels)),
        height: Some(CropLength::new(900.0, UnitKind::Pixels)),
        resolution: "300".into(),
        ..CropSizeFields::default()
    };
    let ((size, grid), bar_at) = data.value_at("bar", (fields, true));
    let mut output = None;
    Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
        output = Some(PerspectiveCropBar::new(&mut *size, &document, bar_at.bind(&mut *grid)).show(ui));
    });
    output.map(StoryOutput::from).unwrap_or_default()
}
