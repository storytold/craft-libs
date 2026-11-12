//! Stories of `ColorSamplerMarks` and `SamplerOptions`.

use super::{ColorSamplerMarks, ColorSamplers, SamplerCache, SamplerOptions, SamplerReadouts};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_theme::{Hsb, Srgba};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::runtime::env::Env;
use craft_ui::testing::canvas_story::{StoryPicture, show_document};
use craft_ui::testing::{Drive, StoryData, StoryOutput, stage_fitting, story};
use egui::{Color32, Ui};

craft_ui::words! {
    /// The readout of the sampler story.
    #[non_exhaustive]
    pub(crate) struct SamplerStoryWords {
        /// One point's line: its number, the pixel it is on, its RGB and its CMYK.
        point: template "samplerStory.point" = "#{n}  {x}, {y}   R {r}  G {g}  B {b}   C {c}  M {m}  Y {y_ink}  K {k}";
    }
}

/// The document pixels of the story's picture.
const SIZE: [u32; 2] = [240, 150];

/// The colour of the story's picture at `p`: hue across, brightness down.
fn colour_at(p: [f64; 2]) -> Srgba {
    let [width, height] = SIZE.map(f64::from);
    Srgba::from_hsb(Hsb { h: (p[0] / width * 360.0) as f32, s: 1.0, b: (1.0 - p[1] / height * 0.85) as f32 })
}

/// The picture the samplers sit on.
fn picture() -> StoryPicture<impl Fn(Point) -> Color32> {
    StoryPicture::new(SIZE[0], SIZE[1], |p| Color32::from(colour_at([p.x, p.y])))
}

/// Four numbered points on the picture, with each point's pixel, RGB and CMYK under it as a panel reads them.
#[story(ColorSamplerMarks<'static>, covers = [Default], drive = Drive::Never)]
fn sample_points(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<SamplerStoryWords>();
    let mut samplers = ColorSamplers::new();
    for at in [[40.0, 30.0], [120.0, 75.0], [200.0, 50.0], [80.0, 120.0]] {
        let _ = samplers.add(at, SIZE);
    }
    // The canvas is as tall as the picture, which sits at its top left at one point per pixel.
    let room = stage_fitting(ui, |_| SIZE[1] as f32);
    let center = Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0);
    let view = CanvasView::new(room, center, 1.0);
    let picture = picture();
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &picture);
        ColorSamplerMarks::new("Color samplers", &samplers).show_at(ui, body.rect)
    });
    let readouts = SamplerReadouts::new(&samplers, colour_at).get(data.note("readouts", SamplerCache::default()));
    let lines: Vec<String> = readouts
        .iter()
        .map(|point| {
            let [r, g, b] = point.rgb.map(|v| v.to_string());
            let [c, m, y, k] = point.cmyk.map(|v| v.to_string());
            let [x, y_at] = point.position.map(|v| v.to_string());
            words.point.fill(&[
                ("n", point.number.to_string().as_str()),
                ("x", x.as_str()),
                ("y", y_at.as_str()),
                ("r", r.as_str()),
                ("g", g.as_str()),
                ("b", b.as_str()),
                ("c", c.as_str()),
                ("m", m.as_str()),
                ("y_ink", y.as_str()),
                ("k", k.as_str()),
            ])
        })
        .collect();
    shown.inner.map(StoryOutput::from).unwrap_or_default().readout(lines.join("\n"))
}

#[story(SamplerOptions, covers = [], drive = Drive::Never)]
fn options(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Bar::new("Color sampler options", BarKind::Options).show(ui, |ui, _| SamplerOptions::new(3).show(ui));
    match shown.inner {
        Some(output) => StoryOutput::from(output),
        None => StoryOutput::from(shown.output),
    }
}
