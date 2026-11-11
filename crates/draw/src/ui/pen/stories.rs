//! Stories of the Path Options popover.

use super::{PathOptionsPopover, PathOptionsSettings};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The colour the popover's swatch shows while the settings hold none.
fn line_color() -> Srgba {
    Srgba::rgb(0.0, 0.5, 1.0)
}

#[story(PathOptionsPopover<'static>, covers = [Default], drive = Drive::Never)]
fn path_options_defaults(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut open = *data.value("open", true);
    let settings = data.value("defaults", PathOptionsSettings::default());
    let shown = PathOptionsPopover::new("Path Options", Bind::bound(&mut *settings)).default_color(line_color()).show(ui, &mut open);
    *data.value("open", true) = open;
    shown.into()
}

#[story(PathOptionsPopover<'static>, covers = [], drive = Drive::Never)]
fn path_options_with_a_color(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut open = *data.value("open-set", true);
    let settings = data.value("set", PathOptionsSettings { rubber_band: true, color: Some([200, 40, 40]), thickness: Some(2.0) });
    let shown = PathOptionsPopover::new("Path Options", Bind::bound(&mut *settings)).show(ui, &mut open);
    *data.value("open-set", true) = open;
    shown.into()
}
