//! Stories of `ColorSamplerMarks` and `SamplerOptions`.

use super::{ColorSamplerMarks, ColorSamplers, SamplerOptions};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_theme::{Hsb, Srgba};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::canvas_story::{StoryPicture, show_document};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, Ui};

/// The document pixels of the story's picture.
const SIZE: [u32; 2] = [240, 150];

/// The picture the samplers sit on: hue across, brightness down.
fn picture() -> StoryPicture<impl Fn(Point) -> Color32> {
    let [width, height] = SIZE.map(f64::from);
    StoryPicture::new(SIZE[0], SIZE[1], move |p| {
        Color32::from(Srgba::from_hsb(Hsb { h: (p.x / width * 360.0) as f32, s: 1.0, b: (1.0 - p.y / height * 0.85) as f32 }))
    })
}

#[story(ColorSamplerMarks<'static>, covers = [Default], drive = Drive::Never)]
fn three_points(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut samplers = ColorSamplers::new();
    for at in [[40.0, 30.0], [120.0, 90.0], [200.0, 60.0]] {
        let _ = samplers.add(at, SIZE);
    }
    let at = ui.available_rect_before_wrap();
    let center = Point::new(f64::from(at.width()) / 2.0, f64::from(at.height()) / 2.0);
    let view = CanvasView::new(at, center, 1.0);
    let picture = picture();
    let shown = Canvas::new("Document").view(view).show(ui, |ui, body| {
        show_document(ui, body.rect, &picture);
        ColorSamplerMarks::new("Color samplers", &samplers).show_at(ui, body.rect)
    });
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
