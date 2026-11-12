//! Stories of `Icon`.

use super::Icon;
use crate::contract::{Props as _, Show as _, ShowContainer as _};
use crate::layout_components::Grid;
use crate::primitives::IconSpec;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::{ColumnWidth, IconId};
use craft_theme::ControlSize;
use craft_theme::variant::Tone;
use egui::Ui;

#[story(Icon<'static>, covers = [Default], drive = Drive::Never)]
fn icon(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Icon::new(IconSpec::svg(IconId::new("settings"))).label("Settings").show(ui).into()
}

/// Six icons at each control size (an icon's side is its control size's icon side), then one in each tone.
#[story(Icon<'static>, covers = [], drive = Drive::Never)]
fn icon_sizes(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    const ICONS: [&str; 6] = ["settings", "undo", "redo", "copy", "trash", "eye"];
    const COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content; 6];
    let sizes = [("Sm", ControlSize::Sm), ("Md", ControlSize::Md)];
    let tones = [Tone::Neutral, Tone::Accent, Tone::Info, Tone::Good, Tone::Warn, Tone::Bad];
    Grid::new(COLUMNS)
        .show(ui, |ui, _| {
            for (name, size) in sizes {
                for icon in ICONS {
                    Icon::new(IconSpec::svg(IconId::new(icon))).size(size).id_salt((name, icon)).show(ui);
                }
            }
            for (tone, icon) in tones.into_iter().zip(ICONS) {
                Icon::new(IconSpec::svg(IconId::new(icon))).tone(tone).id_salt(("tone", icon)).show(ui);
            }
        })
        .into()
}
