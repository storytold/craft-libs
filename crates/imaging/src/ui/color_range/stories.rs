//! Stories of the Color Range dialog: the body at Sampled Colors with two samples, at Midtones, with a failed
//! preview, and the hosted dialog opened from a button.

use std::sync::Arc;

use craft_ui::dialog::stories::hosted;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{ColorRangeDialog, ColorRangeSelect, ColorRangeValue, HostedColorRange, SampleSign};
use crate::ui::mock_image::MockImage;

/// The proxy's size and the factor it is reduced by.
const PROXY: [u32; 2] = [120, 80];
const SCALE: u32 = 4;

fn sampled() -> ColorRangeValue {
    let mut value = ColorRangeValue::default();
    value.samples.add = vec![[62.0, 42.0], [200.0, 150.0]];
    value.samples.order = vec![SampleSign::Add, SampleSign::Add];
    value
}

fn document() -> [u32; 2] {
    [PROXY[0] * SCALE, PROXY[1] * SCALE]
}

#[story(ColorRangeDialog<'static>, covers = [Default], drive = Drive::Never)]
fn color_range_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, value_at) = data.value_at("value", sampled());
    let picture = MockImage::new(PROXY);
    ColorRangeDialog::new("Color Range", value_at.bind(value))
        .mask(&picture)
        .image(&picture)
        .picture_scale(SCALE)
        .document(document())
        .show(ui)
        .into()
}

#[story(ColorRangeDialog<'static>, covers = [], drive = Drive::Never)]
fn color_range_dialog_midtones(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, value_at) = data.value_at("value", ColorRangeValue { select: ColorRangeSelect::Midtones, ..ColorRangeValue::default() });
    let picture = MockImage::new(PROXY);
    ColorRangeDialog::new("Color Range", value_at.bind(value))
        .mask(&picture)
        .image(&picture)
        .picture_scale(SCALE)
        .document(document())
        .show(ui)
        .into()
}

#[story(ColorRangeDialog<'static>, covers = [], drive = Drive::Never)]
fn color_range_dialog_failed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, value_at) = data.value_at("value", ColorRangeValue::default());
    ColorRangeDialog::new("Color Range", value_at.bind(value))
        .picture_scale(SCALE)
        .document(document())
        .error(Label::from("The preview could not be rendered."))
        .show(ui)
        .into()
}

#[story(ColorRangeDialog<'static>, covers = [], drive = Drive::Never, title = "In its dialog")]
fn hosted_color_range(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || {
        let mut dialog = HostedColorRange::new("Color Range", sampled(), document());
        let picture = Arc::new(MockImage::new(PROXY));
        dialog.set_preview(Some(picture.clone()), Some(picture), SCALE, None);
        dialog
    })
}
