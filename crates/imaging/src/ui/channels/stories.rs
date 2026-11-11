//! Stories of `ChannelsPanel`: alpha and spot channels; Quick Mask and a layer mask; a multichannel document.

use craft_theme::Srgba;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{ChannelKind, ChannelRow, ChannelsPanel, ColorIndicates};

type Rows = Vec<ChannelRow<u32>>;

fn colour_rows() -> Rows {
    let mut rows = vec![ChannelRow { targeted: true, ..ChannelRow::new(0, ChannelKind::Composite, "RGB") }];
    for (key, name) in [(1, "Red"), (2, "Green"), (3, "Blue")] {
        rows.push(ChannelRow { targeted: true, ..ChannelRow::new(key, ChannelKind::Color, name) });
    }
    rows
}

fn alpha_and_spot_rows() -> Rows {
    let mut rows = colour_rows();
    rows.push(ChannelRow::new(10, ChannelKind::Alpha(ColorIndicates::MaskedAreas), "Alpha 1"));
    rows.push(ChannelRow { visible: false, ..ChannelRow::new(11, ChannelKind::Alpha(ColorIndicates::SelectedAreas), "Alpha 2") });
    rows.push(ChannelRow::new(12, ChannelKind::Spot(Srgba::rgb(0.88, 0.1, 0.2)), "PANTONE 185 C"));
    rows
}

fn mask_rows() -> Rows {
    let mut rows = colour_rows();
    for row in &mut rows {
        row.targeted = false;
    }
    rows.push(ChannelRow { targeted: true, ..ChannelRow::new(20, ChannelKind::LayerMask(true), "Sky Mask") });
    rows.push(ChannelRow::new(10, ChannelKind::Alpha(ColorIndicates::MaskedAreas), "Alpha 1"));
    rows.push(ChannelRow::new(30, ChannelKind::QuickMask(ColorIndicates::MaskedAreas), "Quick Mask"));
    rows
}

/// A Multichannel document lists its channels as spot channels, with no composite.
fn multichannel_rows() -> Rows {
    let inks = [("Cyan", [0.0, 0.7, 0.9]), ("Magenta", [0.9, 0.0, 0.6]), ("Yellow", [0.95, 0.85, 0.0])];
    let mut rows: Rows =
        inks.iter().zip(40..).map(|((name, [r, g, b]), key)| ChannelRow::new(key, ChannelKind::Spot(Srgba::rgb(*r, *g, *b)), *name)).collect();
    rows[0].targeted = true;
    rows
}

#[story(ChannelsPanel<'static, u32>, covers = [Default], drive = Drive::Never)]
fn channels_panel_alpha_and_spot(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = data.value("rows", alpha_and_spot_rows()).clone();
    ChannelsPanel::new(&rows).show(ui).into()
}

#[story(ChannelsPanel<'static, u32>, covers = [], drive = Drive::Never)]
fn channels_panel_quick_mask_and_layer_mask(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = data.value("rows", mask_rows()).clone();
    ChannelsPanel::new(&rows).show(ui).into()
}

#[story(ChannelsPanel<'static, u32>, covers = [], drive = Drive::Never)]
fn channels_panel_multichannel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = data.value("rows", multichannel_rows()).clone();
    ChannelsPanel::new(&rows).show(ui).into()
}
