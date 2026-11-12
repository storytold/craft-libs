//! [`GradientSwatch`]: a well that shows a gradient.

use craft_behavior::{CursorIcon, CursorPriority, Metrics, Sense};
use craft_ui::choice::SwatchMetrics;
use craft_ui::contract::{
    Access, Common, Component, Cx, DisabledLook, MeasureCx, Outcome, PaintUi, Pointer as _, PopupKind, Props as _, Role, Sizing,
};
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx};
use egui::{CornerRadius, Response, Vec2};

use super::ramp::GradientRamp;
use super::stops::{Sampler, default_sampler};
use super::value::{GradientStops, StopRows};

#[cfg(feature = "screen-testing")]
mod stories;

/// The style of a gradient swatch: the colour swatch's height, corners and ring.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GradientSwatchStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The height of the well: the colour swatch's side.
    side: f32,
    /// The well's corners.
    radius: CornerRadius,
    /// The ring on the well's outline, which separates the well from the panel.
    edge: StyleColor,
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
/// swatch edits nothing; a click is the host's cue. The well has the gradient chip aspect
/// (`GradientRamp::CHIP_ASPECT`); with `Sizing::FillWidth` it takes the wider width the layout
/// offers (an options bar cell). AccessKit sees a button named by the label, with `has_popup`
/// when the host opens a dialog on the click.
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
    /// Tells AccessKit that a click opens a dialog.
    #[prop(flag)]
    has_popup: bool,
    clicked: bool,
}

impl<'a> GradientSwatch<'a> {
    /// A swatch of `stops`; `label` names it for a screen reader.
    pub fn new(label: impl Into<craft_ui::runtime::env::Label>, stops: &'a GradientStops) -> Self {
        GradientSwatch { common: Common::new(), stops, rows: StopRows::Split, sampler: None, has_popup: false, clicked: false }.label(label)
    }
}

impl Component for GradientSwatch<'_> {
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = GradientSwatchStyle;
    type Output = GradientSwatchOutput;

    fn style(cx: &StyleCx<'_>, _: GradientSwatchStyleKey) -> GradientSwatchStyle {
        let (radius, edge) = GradientRamp::chip_look(cx);
        GradientSwatchStyle { metrics: Metrics::new(), focus: cx.focus_look(), side: SwatchMetrics::of(cx).side, radius, edge }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let height = cx.style.side;
        let (wide, high) = GradientRamp::CHIP_ASPECT;
        let chip = height * f32::from(wide) / f32::from(high);
        let width = if cx.resolved.sizing() == Some(Sizing::FillWidth) {
            cx.room.filter(|room| room.is_finite()).map_or(chip, |room| room.max(chip))
        } else {
            chip
        };
        Vec2::new(width, height)
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
        GradientRamp::paint_chip(&ui.content_painter(cx), cx.rect, style.radius, &colors, style.edge);
    }

    fn finish(self, cx: Cx<'_, Self>) -> GradientSwatchOutput {
        GradientSwatchOutput { response: cx.response.clone(), clicked: self.clicked }
    }
}
