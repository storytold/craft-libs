//! Stories of `CompareSwatch`: an opaque pair with both warnings, and a translucent pair.

use craft_theme::Srgba;
use craft_ui::contract::Show as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::CompareSwatch;
use crate::ui::ColorWarning;

/// The colour being edited over the one the picker started from, with the out-of-gamut and web-colour warnings.
#[story(CompareSwatch<'static>, covers = [Default], drive = Drive::Never)]
fn compare_swatch(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let warnings = [
        ColorWarning::new("Out of gamut for printing", Srgba::rgb(0.9, 0.1, 0.9)),
        ColorWarning::web_colors("Not a web color", Srgba::rgb(1.0, 0.0, 0.8)),
    ];
    CompareSwatch::new("New color", Srgba::rgb(1.0, 0.0, 1.0), Srgba::rgb(0.9, 0.2, 0.1)).warnings(&warnings).show(ui).into()
}

/// Translucent colours over the checkerboard.
#[story(CompareSwatch<'static>, covers = [], drive = Drive::Never)]
fn compare_swatch_translucent(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let new = Srgba { r: 0.2, g: 0.5, b: 0.8, a: 0.5 };
    let current = Srgba { r: 0.9, g: 0.6, b: 0.1, a: 0.25 };
    CompareSwatch::new("New color", new, current).show(ui).into()
}
