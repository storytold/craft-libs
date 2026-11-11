//! The stroke preview: Photoshop's S-curve test stroke as data ([`PreviewStroke`]) and the strip
//! that shows the app's rendering of it ([`BrushStrokeStrip`]).

use std::sync::Arc;

use craft_behavior::{Axis, Metrics, Sense};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use craft_ui::primitives::{Revision, TextureStore, TextureStoreOptions};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx, SurfaceLook};
use egui::{ColorImage, Id, Rect, Response, Vec2, pos2};

/// One point of the test stroke, in pixels of the strip image.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PreviewSample {
    /// Across, from the image's left edge.
    pub x: f64,
    /// Down, from the image's top edge.
    pub y: f64,
    /// The pen pressure, `PreviewStroke::PRESSURE_FLOOR` to 1.
    pub pressure: f32,
}

/// Photoshop's preview stroke for an image `width` × `height` pixels: an S-curve with the pressure
/// tapering in and out, so every engine draws the same test stroke.
///
/// [`SAMPLES`](Self::SAMPLES) points at `s = i / (SAMPLES − 1)`: `x = MARGIN + s × (width − 2 ×
/// MARGIN)`, `y = height / 2 − sin(2π s) × height × AMPLITUDE`, pressure `max(sin(π s),
/// PRESSURE_FLOOR)`. The engine draws it with the brush scaled by [`fit`](Self::fit), Build-up off
/// and a fixed seed (PhotoCraft 7), so the strip changes only when the settings do.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PreviewStroke {
    /// The image width in pixels.
    pub width: u32,
    /// The image height in pixels.
    pub height: u32,
}

impl PreviewStroke {
    /// The number of samples.
    pub const SAMPLES: usize = 65;
    /// Pixels kept clear at each end.
    pub const MARGIN: f64 = 10.0;
    /// The curve's swing up and down, as a fraction of the height.
    pub const AMPLITUDE: f64 = 0.22;
    /// The share of the height the brush diameter may take.
    pub const FIT: f64 = 0.55;
    /// The least pressure, at the stroke's ends.
    pub const PRESSURE_FLOOR: f32 = 0.05;

    /// The stroke for an image of this size.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// The samples, in drawing order.
    pub fn samples(&self) -> Vec<PreviewSample> {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let last = (Self::SAMPLES - 1) as f64;
        (0..Self::SAMPLES)
            .map(|i| {
                let s = i as f64 / last;
                PreviewSample {
                    x: Self::MARGIN + s * (w - 2.0 * Self::MARGIN),
                    y: h / 2.0 - (s * std::f64::consts::TAU).sin() * h * Self::AMPLITUDE,
                    pressure: ((s * std::f64::consts::PI).sin() as f32).max(Self::PRESSURE_FLOOR),
                }
            })
            .collect()
    }

    /// The scale for a brush `diameter` pixels across: `min(1, FIT × height / max(diameter, 1))`.
    /// The engine multiplies the brush size (and a dual brush's) by it, at least 1 pixel.
    pub fn fit(&self, diameter: f64) -> f64 {
        (Self::FIT * f64::from(self.height) / diameter.max(1.0)).min(1.0)
    }
}

/// The app's rendering of the preview stroke: it gets the strip's size in physical pixels and returns the image
/// (the app's engine draws [`PreviewStroke`] into it). A pass may show the strip more than once, so the rendering
/// is a `Fn`; it runs only when the strip's revision or pixel size changes.
#[derive(Clone)]
pub struct StripImage<'a>(Arc<dyn Fn([usize; 2]) -> ColorImage + 'a>);

impl<'a> StripImage<'a> {
    /// The rendering that runs `make`.
    pub fn new(make: impl Fn([usize; 2]) -> ColorImage + 'a) -> Self {
        StripImage(Arc::new(make))
    }
}

/// The style of the stroke strip: the well and its height.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct BrushStrokeStripStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
}

/// The stroke preview strip: the app's rendering of the test stroke, kept as a texture by its [`Revision`]. The
/// render closure runs only when the revision or the strip's size in pixels changes.
///
/// - **Look.** The field well, the full width it is offered and the large thumbnail side high; the image fills it.
/// - **AccessKit.** An image named by the label.
#[derive(Clone, craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct BrushStrokeStrip<'a> {
    #[common]
    common: Common,
    revision: Revision,
    make: StripImage<'a>,
}

impl<'a> BrushStrokeStrip<'a> {
    /// A strip named `label` whose picture is at `revision` (`Revision::of(&settings_signature)`), rendered by `make`.
    pub fn new(label: impl Into<Label>, revision: Revision, make: StripImage<'a>) -> Self {
        BrushStrokeStrip { common: Common::new(), revision, make }.label(label)
    }
}

impl Component for BrushStrokeStrip<'_> {
    const ROLE: Role = Role::IMAGE;
    const OVERLAYS: bool = false;
    type Style = BrushStrokeStripStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: BrushStrokeStripStyleKey) -> BrushStrokeStripStyle {
        BrushStrokeStripStyle { surface: cx.text_well(), metrics: Metrics::new().min_size(Vec2::new(0.0, cx.tokens.size.thumb.lg)) }
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let painter = ui.content_painter(cx);
        let rect = cx.rect;
        if !painter.painter.clip_rect().is_positive() {
            return;
        }
        let ppp = painter.pixels_per_point();
        let pixels = [rect.width(), rect.height()].map(|side| (side * ppp).round().max(1.0) as usize);
        let ctx = painter.painter.ctx();
        let store = TextureStore::<Id>::in_context(ctx, "brush-stroke-strip", TextureStoreOptions::frames());
        let make = &self.make.0;
        let picture = store.get_or_put(ctx, cx.id, Revision::of(&(self.revision, pixels)), || make(pixels));
        painter.picture(picture.id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), 0.0);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
