//! Stories of the Crop options bar and its menus.

use super::super::menus::{CropOptionsMenu, CropOverlayMenu};
use super::super::options::{CropLength, CropRatio, CropSettings, CropSizeFields, CropUserPreset};
use super::super::session::{CropDocKey, CropDocument};
use super::CropBar;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::ShowContainer as _;
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use kurbo::Size;

fn document() -> CropDocument {
    CropDocument::new(CropDocKey::new(1), Size::new(2400.0, 1500.0), 72.0)
}

#[story(CropBar<'static>, covers = [], drive = Drive::Never)]
fn crop_bar_ratio(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let document = document();
    let settings = data.value("settings", CropSettings::default());
    let mut output = None;
    Bar::new("Options", BarKind::Options).show(ui, |ui, _| output = Some(CropBar::new(settings, &document).angle(0.0).show(ui)));
    output.map(StoryOutput::from).unwrap_or_default()
}

#[story(CropBar<'static>, covers = [], drive = Drive::Never)]
fn crop_bar_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let document = document();
    let size = CropSizeFields {
        width: Some(CropLength::new(4.0, UnitKind::Inches)),
        height: Some(CropLength::new(5.0, UnitKind::Inches)),
        resolution: "300".into(),
        ..CropSizeFields::default()
    };
    let settings = CropSettings { ratio: CropRatio::Size, size, ..CropSettings::default() };
    let settings = data.value("size", settings);
    let mut output = None;
    Bar::new("Options", BarKind::Options).show(ui, |ui, _| output = Some(CropBar::new(settings, &document).show(ui)));
    output.map(StoryOutput::from).unwrap_or_default()
}

#[story(CropBar<'static>, covers = [], drive = Drive::Never)]
fn crop_bar_with_user_presets(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let document = document();
    let presets = [
        CropUserPreset::new("3 : 2", CropRatio::Sides { width: 3.0, height: 2.0 }, CropSizeFields::default()),
        CropUserPreset::new("Poster", CropRatio::Sides { width: 2.0, height: 3.0 }, CropSizeFields::default()),
    ];
    let settings = data.value("presets", CropSettings::default());
    let mut output = None;
    Bar::new("Options", BarKind::Options).show(ui, |ui, _| output = Some(CropBar::new(settings, &document).user_presets(&presets).show(ui)));
    output.map(StoryOutput::from).unwrap_or_default()
}

#[story(CropOverlayMenu<'static>, covers = [], drive = Drive::Never)]
fn crop_overlay_menu_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = data.value("overlay", CropSettings::default());
    CropOverlayMenu::new(settings).opened().show(ui).into()
}

#[story(CropOptionsMenu<'static>, covers = [], drive = Drive::Never)]
fn crop_options_menu_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = data.value("options", CropSettings::default());
    CropOptionsMenu::new(settings).opened().show(ui).into()
}

#[story(CropOptionsMenu<'static>, covers = [], drive = Drive::Never)]
fn crop_options_menu_classic_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut settings = CropSettings::default();
    settings.shield.classic_mode = true;
    let settings = data.value("classic", settings);
    CropOptionsMenu::new(settings).opened().show(ui).into()
}
