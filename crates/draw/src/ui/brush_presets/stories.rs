//! Stories of the Brush Preset picker.

use craft_behavior::Mixed;
use craft_ui::contract::Bind;
use craft_ui::patterns::presets::PresetRow;
use craft_ui::runtime::env::Env;
use craft_ui::testing::pictures::{SampleTip, paint_tip, paint_wave};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{BrushCardParts, BrushPickerState, BrushPickerValues, BrushPresetPicker, BrushRowPaint, brush_row};

const SOFT: u32 = 1;
const HARD: u32 = 2;
const SOFT_SIZE: f64 = 45.0;
const HARD_SIZE: f64 = 12.0;
const MAX_SIZE: f64 = 5000.0;

/// The tip and stroke of a preset with `tip`, in the ink of the surface; a soft tip's stroke is wider and half
/// opaque.
fn paint(tip: SampleTip) -> BrushRowPaint {
    let (width, alpha) = if tip == SampleTip::Soft { (0.19, 0.5) } else { (0.12, 1.0) };
    BrushRowPaint::new(
        move |cx, rect| paint_tip(cx, rect, tip, cx.ink()),
        move |cx, rect| paint_wave(cx, rect, width, cx.ink().gamma_multiply(alpha)),
    )
}

fn rows(env: &Env, parts: BrushCardParts) -> Vec<PresetRow<u32>> {
    vec![
        brush_row(SOFT, "Soft Round", 0, SOFT_SIZE, parts, env, &paint(SampleTip::Soft)),
        brush_row(HARD, "Hard Round", 0, HARD_SIZE, parts, env, &paint(SampleTip::Hard)),
    ]
}

#[story(BrushPresetPicker<'static, u32>, covers = [Default], drive = Drive::Never)]
fn picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let env = Env::of(ui);
    let (current, state) = data.value("brush", (Some(SOFT), BrushPickerState::<u32>::new()));
    let rows = rows(&env, state.parts);
    let values = BrushPickerValues::new(SOFT_SIZE, Some(0.8));
    BrushPresetPicker::new("Brush Preset picker", Bind::bound(current), &rows, Bind::bound(state), values)
        .max_diameter(MAX_SIZE)
        .import()
        .show(ui)
        .into()
}

#[story(BrushPresetPicker<'static, u32>, covers = [Mixed], drive = Drive::Never)]
fn picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let env = Env::of(ui);
    let (current, state) = data.value("mixed", (Mixed::<Option<u32>>::Mixed, BrushPickerState::<u32>::new()));
    let rows = rows(&env, state.parts);
    let values = BrushPickerValues::new(HARD_SIZE, None);
    BrushPresetPicker::new("Brush Preset picker", Bind::bound_mixed(current), &rows, Bind::bound(state), values).show(ui).into()
}

#[story(BrushPresetPicker<'static, u32>, covers = [], drive = Drive::Never)]
fn picker_sampled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let env = Env::of(ui);
    let (current, state) = data.value("sampled", (Some(HARD), BrushPickerState::<u32>::new()));
    let rows = rows(&env, state.parts);
    BrushPresetPicker::new("Brush Preset picker", Bind::bound(current), &rows, Bind::bound(state), BrushPickerValues::new(HARD_SIZE, None))
        .show(ui)
        .into()
}

#[story(BrushPresetPicker<'static, u32>, covers = [], drive = Drive::Never)]
fn picker_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (current, state) = data.value("empty", (None::<u32>, BrushPickerState::<u32>::new()));
    BrushPresetPicker::new("Brush Preset picker", Bind::bound(current), &[], Bind::bound(state), BrushPickerValues::new(SOFT_SIZE, Some(1.0)))
        .show(ui)
        .into()
}
