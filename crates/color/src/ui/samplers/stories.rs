//! Stories of `ColorSamplerMarks` and `SamplerOptions`.

use super::{ColorSamplerMarks, ColorSamplers, SamplerOptions};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The document pixels of the story's canvas.
const SIZE: [u32; 2] = [300, 200];

#[story(ColorSamplerMarks<'static>, covers = [Default], drive = Drive::Never)]
fn three_points(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut samplers = ColorSamplers::new();
    for at in [[40.0, 30.0], [120.0, 90.0], [200.0, 60.0]] {
        let _ = samplers.add(at, SIZE);
    }
    let at = ui.available_rect_before_wrap();
    let center = Point::new(f64::from(at.width()) / 2.0, f64::from(at.height()) / 2.0);
    let view = CanvasView::new(at, center, 1.0);
    let shown = Canvas::new("Document").view(view).show(ui, |ui, body| ColorSamplerMarks::new("Color samplers", &samplers).show_at(ui, body.rect));
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}

#[story(SamplerOptions, covers = [], drive = Drive::Never)]
fn options(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Bar::new("Color sampler options", BarKind::Options).show(ui, |ui, _| SamplerOptions::new(3).show(ui));
    match shown.inner {
        Some(output) => StoryOutput::from(output),
        None => StoryOutput::from(shown.output),
    }
}
