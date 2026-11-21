//! Stories of the pattern fill editor: the bitmap mode, the two-colour preset mode and the editor in Photoshop's
//! dialog. Each tile shows a texture the story makes, as a host hands in its patterns' pictures.

use super::{PatternColors, PatternFillEditor, PatternFillOptions, PatternSwatch};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::primitives::textures::TextureCache;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, ColorImage, Ui};

/// The library, each tile a checker texture: the stand-in for the host's pattern bitmaps.
fn swatches(ctx: &egui::Context) -> Vec<PatternSwatch> {
    let picture = TextureCache::texture(ctx, "pattern-story-tile", || {
        let (light, dark) = (Color32::from_gray(225), Color32::from_gray(150));
        ColorImage::new([8, 8], (0..64).map(|at| if (at % 8 < 4) == (at / 8 < 4) { light } else { dark }).collect())
    });
    ["Stripes", "Checks", "Dots", "Grid", "Waves", "Bubbles"]
        .iter()
        .map(|name| PatternSwatch::new(name.to_lowercase(), *name).with_picture(picture))
        .collect()
}

fn editor(ui: &mut Ui, data: &mut StoryData, start: PatternFillOptions, library: &[PatternSwatch]) -> StoryOutput {
    let (options, options_at) = data.value_at("options", start);
    let out = PatternFillEditor::new("Pattern fill", options_at.bind(options), library).show(ui);
    StoryOutput::from(out)
}

#[story(PatternFillEditor<'static>, covers = [Default], drive = Drive::Never)]
fn pattern_fill_bitmap(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    editor(ui, data, PatternFillOptions::bitmap("bubbles"), &swatches(ui.ctx()))
}

#[story(PatternFillEditor<'static>, covers = [Default], drive = Drive::Never)]
fn pattern_fill_preset(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let colors = PatternColors::new(Srgba::rgb(0.1, 0.1, 0.1), Srgba::rgb(0.95, 0.95, 0.9));
    editor(ui, data, PatternFillOptions::preset("checks", colors), &swatches(ui.ctx()))
}

#[story(PatternFillEditor<'static>, covers = [Default], drive = Drive::Never)]
fn pattern_fill_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let library = swatches(ui.ctx());
    let ((options, options_at), (open, open_at)) = data.pair_at(("options", PatternFillOptions::bitmap("bubbles")), ("open", true));
    FormDialog::new("Pattern Fill", options_at.bind(options)).opened(open_at.bind(open)).show(ui, |ui, _, value| {
        PatternFillEditor::new("Pattern fill", Bind::bound(value), &library).show(ui);
        true
    });
    StoryOutput::default()
}
