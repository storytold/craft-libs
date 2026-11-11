//! `ToneRamp`: a gradient bar through evenly spaced stops.

use craft_behavior::{Axis, Input, Metrics, Sense};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Role};
use craft_ui::style::{ComponentStyle, DataColor, StyleCx};
use egui::{Response, Vec2};

/// The style of a tone ramp: the thickness it asks for when nothing sizes it.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ToneRampStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The side of the square a ramp asks for.
    thickness: f32,
}

/// A gradient through `stops`, evenly spaced from the start of the bar to its end: two stops for the tone bars of a
/// curve, many for a hue spectrum. It runs along the width, or bottom to top when `vertical`. A ramp is a face: a
/// slider shows it as its track, and a curve graph shows one under the plot and one beside it. Shown on its own in a
/// flow, it spans the room along its axis. It paints through the content painter's one ramp painter.
#[derive(Clone, Component, craft_ui::contract::Props)]
#[component(content_painter, sample = ToneRamp::sample)]
pub struct ToneRamp<'a> {
    #[common]
    common: Common,
    stops: &'a [DataColor],
    /// Whether the ramp runs from the bottom edge to the top edge instead of along the width.
    #[prop(flag)]
    vertical: bool,
}

impl<'a> ToneRamp<'a> {
    /// A ramp through `stops`.
    pub fn new(stops: &'a [DataColor]) -> Self {
        ToneRamp { common: Common::new(), stops, vertical: false }
    }
}

impl ToneRamp<'static> {
    /// A black to white ramp, for the suite.
    fn sample() -> Self {
        const BLACK_TO_WHITE: [DataColor; 2] = [DataColor::new(egui::Color32::from_gray(0)), DataColor::new(egui::Color32::from_gray(u8::MAX))];
        ToneRamp::new(&BLACK_TO_WHITE)
    }
}

impl Component for ToneRamp<'_> {
    const ROLE: Role = Role::None;
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = ToneRampStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ToneRampStyleKey) -> ToneRampStyle {
        ToneRampStyle { metrics: Metrics::new(), thickness: cx.tokens.size.icon.sm }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.thickness)
    }

    fn fills(&self) -> Option<Axis> {
        Some(if self.vertical { Axis::Y } else { Axis::X })
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let last = self.stops.len().saturating_sub(1).max(1) as f32;
        let at = |k: usize| k as f32 / last;
        let (stops, axis): (Vec<(f32, DataColor)>, Axis) = if self.vertical {
            (self.stops.iter().rev().enumerate().map(|(k, color)| (at(k), *color)).collect(), Axis::Y)
        } else {
            (self.stops.iter().enumerate().map(|(k, color)| (at(k), *color)).collect(), Axis::X)
        };
        ui.content_painter(cx).ramp(cx.rect, &stops, axis);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
