//! Stories of the Brush Preset picker.

use craft_behavior::Mixed;
use craft_ui::patterns::presets::{PresetFns, PresetRow, PresetSource};
use craft_ui::primitives::FacePaint;
use craft_ui::runtime::env::Env;
use craft_ui::testing::pictures::{SampleTip, paint_tip, paint_wave};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use craft_ui::patterns::presets::PresetState;

use super::{BrushCardParts, BrushPickerState, BrushPickerValues, BrushPresetPicker, BrushRowPaint, BrushesPanel, brush_row};
use crate::ui::brush_settings::TipAngle;

const SOFT: u32 = 1;
const HARD: u32 = 2;
const SOFT_SIZE: f64 = 45.0;
const HARD_SIZE: f64 = 12.0;
const MAX_SIZE: f64 = 5000.0;

/// The tip and stroke of a preset with `tip`, in the ink of the surface; a soft tip's stroke is wider and half
/// opaque.
fn paint(tip: SampleTip) -> BrushRowPaint {
    let ((width, alpha), (tip_key, stroke_key)) = if tip == SampleTip::Soft { ((0.19, 0.5), (1, 2)) } else { ((0.12, 1.0), (3, 4)) };
    BrushRowPaint::new(
        FacePaint::new(tip_key, move |cx, rect| paint_tip(cx, rect, tip, cx.ink())),
        FacePaint::new(stroke_key, move |cx, rect| paint_wave(cx, rect, width, cx.ink().gamma_multiply(alpha))),
    )
}

fn rows(parts: &BrushCardParts, env: &Env) -> Vec<PresetRow<u32>> {
    let parts = *parts;
    vec![
        brush_row(SOFT, "Soft Round", 0, SOFT_SIZE, parts, env, &paint(SampleTip::Soft)),
        brush_row(HARD, "Hard Round", 0, HARD_SIZE, parts, env, &paint(SampleTip::Hard)),
    ]
}

#[story(BrushPresetPicker<'static, u32>, covers = [Default], drive = Drive::Never)]
fn picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((current, state), brush_at) = data.value_at("brush", (Some(SOFT), BrushPickerState::<u32>::new()));
    let source = PresetSource::new(state.parts, PresetFns::new(rows));
    let values = BrushPickerValues::new(SOFT_SIZE, Some(0.8));
    BrushPresetPicker::new("Brush Preset picker", brush_at.bind(current), &source, brush_at.bind(state), values)
        .max_diameter(MAX_SIZE)
        .import()
        .show(ui)
        .into()
}

/// The open picker of a round brush tilted 30° and 60 % round, as the app opens it at a point: the angle widget, Size
/// and Hardness in the head.
#[story(BrushPresetPicker<'static, u32>, covers = [Open], drive = Drive::Never)]
fn picker_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let at = ui.max_rect().min;
    let ((current, state, open), open_at2) = data.value_at("open", (Some(SOFT), BrushPickerState::<u32>::new(), true));
    let source = PresetSource::new(state.parts, PresetFns::new(rows));
    let values = BrushPickerValues::new(SOFT_SIZE, Some(0.8)).with_tip(TipAngle::new(30.0, 0.6));
    BrushPresetPicker::new("Brush Preset picker", open_at2.bind(current), &source, open_at2.bind(state), values)
        .max_diameter(MAX_SIZE)
        .import()
        .open_at(at, open_at2.bind(open))
        .show(ui)
        .into()
}

#[story(BrushesPanel<'static, u32>, covers = [Default], drive = Drive::Never)]
fn panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((state, diameter), panel_at) = data.value_at("panel", (PresetState::<u32>::new(), SOFT_SIZE));
    let source = PresetSource::new(BrushCardParts::ALL, PresetFns::new(rows));
    BrushesPanel::new("Brushes", &source, panel_at.bind(state), panel_at.bind(diameter), MAX_SIZE).selected(SOFT).import(true).show(ui).into()
}

#[story(BrushPresetPicker<'static, u32>, covers = [Mixed], drive = Drive::Never)]
fn picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((current, state), mixed_at) = data.value_at("mixed", (Mixed::<Option<u32>>::Mixed, BrushPickerState::<u32>::new()));
    let source = PresetSource::new(state.parts, PresetFns::new(rows));
    let values = BrushPickerValues::new(HARD_SIZE, None);
    BrushPresetPicker::new("Brush Preset picker", mixed_at.bind_mixed(current), &source, mixed_at.bind(state), values).show(ui).into()
}

#[story(BrushPresetPicker<'static, u32>, covers = [], drive = Drive::Never)]
fn picker_sampled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((current, state), sampled_at) = data.value_at("sampled", (Some(HARD), BrushPickerState::<u32>::new()));
    let source = PresetSource::new(state.parts, PresetFns::new(rows));
    BrushPresetPicker::new("Brush Preset picker", sampled_at.bind(current), &source, sampled_at.bind(state), BrushPickerValues::new(HARD_SIZE, None))
        .show(ui)
        .into()
}

#[story(BrushPresetPicker<'static, u32>, covers = [], drive = Drive::Never)]
fn picker_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((current, state), empty_at) = data.value_at("empty", (None::<u32>, BrushPickerState::<u32>::new()));
    let source = PresetSource::new((), PresetFns::new(|_, _| Vec::new()));
    BrushPresetPicker::new("Brush Preset picker", empty_at.bind(current), &source, empty_at.bind(state), BrushPickerValues::new(SOFT_SIZE, Some(1.0)))
        .show(ui)
        .into()
}
