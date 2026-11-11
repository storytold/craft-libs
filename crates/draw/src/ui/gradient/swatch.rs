//! [`GradientSwatch`]: a well that shows a gradient.

use craft_behavior::{Axis, CursorIcon, CursorPriority, Metrics, Sense};
use craft_theme::{Stateful, States};
use craft_ui::choice::SwatchMetrics;
use craft_ui::contract::{Access, Common, Component, Cx, MeasureCx, Outcome, PaintUi, Pointer as _, PopupKind, Props as _, Role, Styled as _};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx};
use egui::{CornerRadius, Response, Vec2};

use super::ramp::GradientRamp;
use super::stops::{Sampler, default_sampler};
use super::value::{GradientStops, StopRows};

#[cfg(feature = "screen-testing")]
mod stories;

/// The style of a gradient swatch: the colour swatch's side, corners and ring.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GradientSwatchStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The side of the square well.
    side: f32,
    /// The well's corners.
    radius: CornerRadius,
    /// The ring on the well's outline, which separates the well from the panel.
    edge: Stateful<StyleColor>,
}

/// What a gradient swatch reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GradientSwatchOutput {
    /// The swatch's response.
    pub response: Response,
    /// A primary click, Space, Enter or an AccessKit click reached the enabled swatch.
    pub clicked: bool,
}

impl Outcome for GradientSwatchOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A well that shows a gradient and reports a click, as a colour swatch does for a colour:
/// `if GradientSwatch::new("Gradient", &stops).show(ui).clicked { open_editor }`.
///
/// The ramp is drawn over the checkerboard where it is translucent, inside the swatch's ring. The
/// swatch edits nothing; a click is the host's cue. `fill` takes the width the layout offers (an
/// options bar cell), else the well is square. AccessKit sees a button named by the label, with
/// `has_popup` when the host opens a dialog on the click. A disabled swatch keeps its gradient and
/// fades its ring.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct GradientSwatch<'a> {
    #[common]
    common: Common,
    stops: &'a GradientStops,
    /// The row layout, for the default sampler.
    #[prop]
    rows: StopRows,
    /// Samples the gradient instead of [`default_sampler`].
    #[prop]
    sampler: Option<&'a Sampler>,
    /// Takes the width the layout offers, at the control height.
    #[prop(flag)]
    fill: bool,
    /// Tells AccessKit that a click opens a dialog.
    #[prop(flag)]
    has_popup: bool,
    clicked: bool,
}

impl<'a> GradientSwatch<'a> {
    /// A swatch of `stops`; `label` names it for a screen reader.
    pub fn new(label: impl Into<craft_ui::runtime::env::Label>, stops: &'a GradientStops) -> Self {
        GradientSwatch { common: Common::new(), stops, rows: StopRows::Split, sampler: None, fill: false, has_popup: false, clicked: false }
            .label(label)
    }
}

impl Component for GradientSwatch<'_> {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = GradientSwatchStyle;
    type Output = GradientSwatchOutput;

    fn style(cx: &StyleCx<'_>, _: GradientSwatchStyleKey) -> GradientSwatchStyle {
        let metrics = SwatchMetrics::of(cx);
        let colors = &cx.tokens.color;
        let rest = cx.color(colors.line.control_edge);
        let disabled = cx.color(colors.text.disabled);
        GradientSwatchStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            side: metrics.side,
            radius: metrics.radius,
            edge: Stateful::new(rest).off(States::new(rest, rest, rest, disabled)),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let side = cx.style.side;
        let width = if self.fill { cx.room.filter(|room| room.is_finite()).map_or(side, |room| room.max(side)) } else { side };
        Vec2::new(width, side)
    }

    fn fills(&self) -> Option<Axis> {
        self.fill.then_some(Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.clicked = cx.activation().is_some() && cx.resolved.enabled;
        if cx.resolved.enabled && cx.hit() {
            cx.cursor(CursorIcon::PointingHand, CursorPriority::Part);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        if self.has_popup {
            access.has_popup(PopupKind::Dialog);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let default = default_sampler(self.rows);
        let sampler: &Sampler = self.sampler.unwrap_or(&default);
        let colors = GradientRamp::colors(self.stops, sampler);
        GradientRamp::paint(&ui.content_painter(cx), cx.rect, &colors);
        ui.draw().stroke(&PaintShape::Rect(cx.rect, style.radius), StrokeWidth::Thin, cx.resolve(&style.edge));
    }

    fn finish(self, cx: Cx<'_, Self>) -> GradientSwatchOutput {
        GradientSwatchOutput { response: cx.response.clone(), clicked: self.clicked }
    }
}
