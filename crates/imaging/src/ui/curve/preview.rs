//! `CurvePreview`: a curve drawn read-only in the editor's look.

use super::graph::{CurveSampler, Map, PlotLook, outputs, paint_line};
use super::model::Curve;
use crate::ui::histogram::ChannelColor;
use craft_behavior::{Input, Metrics, Sense};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Response, Vec2};

/// The style of a curve preview.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CurvePreviewStyle {
    #[style(metrics)]
    metrics: Metrics,
    look: PlotLook,
    /// The side of the preview.
    side: f32,
}

/// A curve drawn read-only: the plot, the quarter grid, the diagonal and the curve in its channel colour, without
/// points, tone bars or fields (a preset or contour thumbnail). AccessKit reports an image named by the label.
#[derive(Component, craft_ui::contract::Props)]
#[component(content_painter, sample = CurvePreview::sample)]
pub struct CurvePreview<'a> {
    #[common]
    common: Common,
    curve: &'a Curve,
    levels: f32,
    /// The curve's channel colour.
    #[prop]
    color: ChannelColor,
    /// The interpolation of the curve through its points.
    #[prop(func = "Fn(&[[f32; 2]]) -> Vec<f32>")]
    sampler: Option<CurveSampler<'a>>,
}

impl<'a> CurvePreview<'a> {
    /// A preview of `curve` named `label`.
    pub fn new(label: impl Into<Label>, curve: &'a Curve) -> Self {
        CurvePreview { common: Common::new(), curve, levels: 255.0, color: ChannelColor::Neutral, sampler: None }.label(label)
    }

    /// The greatest level of the curve.
    #[must_use]
    pub fn levels(mut self, levels: u32) -> Self {
        self.levels = levels.max(1) as f32;
        self
    }
}

impl CurvePreview<'static> {
    /// A preview of a diagonal, for the suite.
    fn sample() -> Self {
        static IDENTITY: std::sync::OnceLock<Curve> = std::sync::OnceLock::new();
        CurvePreview::new("Curve", IDENTITY.get_or_init(|| Curve::identity(255.0)))
    }
}

impl Component for CurvePreview<'_> {
    const ROLE: Role = Role::IMAGE;
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = CurvePreviewStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: CurvePreviewStyleKey) -> CurvePreviewStyle {
        CurvePreviewStyle { metrics: Metrics::new(), look: PlotLook::new(cx), side: cx.tokens.size.plot.width.min(cx.tokens.size.plot.height) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let look = &cx.style.look;
        let painter = ui.content_painter(cx);
        look.backdrop(ui, &painter, cx.rect, None);
        let map = Map { area: cx.rect.shrink(look.pad()), levels: self.levels };
        paint_line(&ui.draw(), &outputs(self.curve, self.sampler.as_ref(), self.levels), map, look.curve(self.color));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
