//! Stories of the paint bar, one per tool and Eraser mode.

use super::{EraserMode, MixerLoad, PaintBar, PaintTool, PaintValues};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::choice::SwatchImage;
use craft_ui::contract::{Entry, ShowContainer as _};
use craft_ui::testing::pictures::sample_picture;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The bar of `tool` in a bar strip, over values the story keeps, starting from `start`.
fn show(ui: &mut Ui, data: &mut StoryData, tool: PaintTool, eraser: EraserMode) -> StoryOutput {
    show_from(ui, data, tool, eraser, PaintValues::new(0u32))
}

fn show_from(ui: &mut Ui, data: &mut StoryData, tool: PaintTool, eraser: EraserMode, start: PaintValues<u32>) -> StoryOutput {
    let state = data.value("state", (start, eraser));
    let (values, eraser_mode) = (&mut state.0, &mut state.1);
    let modes = [Entry::new(0u32, "Normal"), Entry::new(1u32, "Multiply"), Entry::new(2u32, "Screen")];
    Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
        let mut bar = PaintBar::new(tool, &mut *values).modes(&modes);
        if tool == PaintTool::Eraser {
            bar = bar.eraser_mode(&mut *eraser_mode);
        }
        let _ = bar.show(ui);
    });
    StoryOutput::default()
}

#[story(PaintBar<'static, u32>, covers = [], drive = Drive::Never)]
fn brush(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, PaintTool::Brush, EraserMode::Brush)
}

#[story(PaintBar<'static, u32>, covers = [], drive = Drive::Never)]
fn pencil(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, PaintTool::Pencil, EraserMode::Brush)
}

#[story(PaintBar<'static, u32>, covers = [], drive = Drive::Never)]
fn eraser_brush_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, PaintTool::Eraser, EraserMode::Brush)
}

#[story(PaintBar<'static, u32>, covers = [], drive = Drive::Never)]
fn eraser_block_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, PaintTool::Eraser, EraserMode::Block)
}

#[story(PaintBar<'static, u32>, covers = [], drive = Drive::Never)]
fn mixer_brush(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, PaintTool::MixerBrush, EraserMode::Brush)
}

/// The Mixer Brush after an Alt-click: the load swatch shows the sampled tip image.
#[story(PaintBar<'static, u32>, covers = [], drive = Drive::Never)]
fn mixer_brush_sampled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = PaintValues::new(0u32);
    start.mixer.current_load = MixerLoad::Image(SwatchImage::new(sample_picture(11, [64, 64])));
    show_from(ui, data, PaintTool::MixerBrush, EraserMode::Brush, start)
}
