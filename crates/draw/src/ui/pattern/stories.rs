//! Stories of the pattern fill editor: the bitmap mode, the two-colour preset mode and the editor in Photoshop's
//! dialog. The tiles are painted from the theme's colours by a closure, as a host paints its patterns.

use super::{PatternColors, PatternFillEditor, PatternFillOptions, PatternSwatch};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::primitives::ContentPainterCx;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rect, Ui};

fn swatches() -> Vec<PatternSwatch> {
    ["Stripes", "Checks", "Dots", "Grid", "Waves", "Bubbles"].iter().map(|name| PatternSwatch::new(name.to_lowercase(), *name)).collect()
}

/// A tile: a plain fill over the whole rect, the stand-in for the host's pattern bitmap.
fn tile(cx: &ContentPainterCx<'_>, rect: Rect, _: &PatternSwatch) {
    cx.checker(rect);
}

fn editor(ui: &mut Ui, data: &mut StoryData, start: PatternFillOptions, library: &[PatternSwatch]) -> StoryOutput {
    let (options, options_at) = data.value_at("options", start);
    let out = PatternFillEditor::new("Pattern fill", options_at.bind(options), library).thumbnails(tile).show(ui);
    StoryOutput::from(out)
}

#[story(PatternFillEditor<'static>, covers = [Default], drive = Drive::Never)]
fn pattern_fill_bitmap(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    editor(ui, data, PatternFillOptions::bitmap("bubbles"), &swatches())
}

#[story(PatternFillEditor<'static>, covers = [Default], drive = Drive::Never)]
fn pattern_fill_preset(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let colors = PatternColors::new(Srgba::rgb(0.1, 0.1, 0.1), Srgba::rgb(0.95, 0.95, 0.9));
    editor(ui, data, PatternFillOptions::preset("checks", colors), &swatches())
}

#[story(PatternFillEditor<'static>, covers = [Default], drive = Drive::Never)]
fn pattern_fill_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let library = swatches();
    let ((options, options_at), (open, open_at)) = data.pair_at(("options", PatternFillOptions::bitmap("bubbles")), ("open", true));
    FormDialog::new("Pattern Fill", options_at.bind(options)).opened(open_at.bind(open)).show(ui, |ui, _, value| {
        PatternFillEditor::new("Pattern fill", Bind::bound(value), &library).thumbnails(tile).show(ui);
        true
    });
    StoryOutput::default()
}
