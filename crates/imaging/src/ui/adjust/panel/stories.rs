//! Stories of the Adjustments panel: every kind, and an app's subset in its own order.

use super::{AdjustmentKind, AdjustmentsPanel};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(AdjustmentsPanel<'static>, covers = [Default], drive = Drive::Never, title = "Every adjustment")]
fn adjustments_panel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    AdjustmentsPanel::new("Add an adjustment").show(ui).into()
}

/// An app that offers six adjustments, in its own order.
#[story(AdjustmentsPanel<'static>, covers = [], drive = Drive::Never, title = "A subset in the app's order")]
fn adjustments_panel_subset(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    const SUBSET: [AdjustmentKind; 6] = [
        AdjustmentKind::Curves,
        AdjustmentKind::Levels,
        AdjustmentKind::Invert,
        AdjustmentKind::HueSaturation,
        AdjustmentKind::Threshold,
        AdjustmentKind::BrightnessContrast,
    ];
    AdjustmentsPanel::new("Add an adjustment").kinds(&SUBSET).show(ui).into()
}
