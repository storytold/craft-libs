//! Stories of `ColorWarningChip`: its two arrangements.

use std::sync::LazyLock;

use craft_theme::Srgba;
use craft_ui::contract::Show as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{ColorWarning, ColorWarningArrangement, ColorWarningChip};

static WARNING: LazyLock<ColorWarning> = LazyLock::new(|| ColorWarning::new("Out of gamut for printing", Srgba::rgb(0.9, 0.1, 0.9)));

/// The icon over the chip, as the Color Picker's warnings column shows it.
#[story(ColorWarningChip<'static>, covers = [Default], drive = Drive::Never)]
fn color_warning_chip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ColorWarningChip::new(&WARNING).show(ui).into()
}

/// The icon beside the chip, as the Color panel shows it under its wells.
#[story(ColorWarningChip<'static>, covers = [], drive = Drive::Never)]
fn color_warning_chip_beside(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ColorWarningChip::new(&WARNING).arrangement(ColorWarningArrangement::Beside).show(ui).into()
}
