//! `Spinner`: the busy mark, an arc that turns once a period and stands still where motion is off. It shows in
//! the flow, or as the face of a part (`ui.face(part, Spinner::new())`) where a component takes a busy state.

use crate::contract::{Access, Animate as _, Common, Component, Cx, MeasureCx, PaintUi, Role, Styled as _};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::style::{ComponentStyle, Flags, IconLook, StyleCx};
use craft_behavior::{Metrics, Sense};
use craft_theme::Stateful;
use egui::{Pos2, Response, Vec2, vec2};
use std::f32::consts::{PI, TAU};

#[cfg(feature = "testing")]
mod stories;

/// How long the spinner takes for a turn, in seconds.
const PERIOD: f64 = 1.0;
/// The arc the spinner draws, in radians.
const SWEEP: f32 = PI * 4.0 / 3.0;
/// The points of the arc.
const POINTS: usize = 24;

/// The style of a spinner. Shown as the face of a part it reads in the host's ink, so it follows any button
/// variant or row fill.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SpinnerStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The arc: the icon side of the control and the host's ink.
    #[style(icon)]
    arc: IconLook,
    /// How far the arc sits inside the square.
    inset: f32,
}

/// The side of a spinner, as the icon sizes and the control height name it. A spinner without one follows the
/// control size around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpinnerSize {
    /// The small icon side.
    Sm,
    /// The medium icon side.
    Md,
    /// The large icon side.
    Lg,
    /// The height of a medium control.
    Control,
}

/// The busy mark. Named by its label it is a busy progress indicator; without a label it is decoration, as inside
/// a button that reports busy itself.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Spinner {
    #[common]
    common: Common,
    /// The side, which is part of the style key.
    #[style_key]
    #[prop]
    extent: Option<SpinnerSize>,
}

impl Spinner {
    /// A spinner at the icon side of the control size.
    pub fn new() -> Self {
        Spinner { common: Common::new(), extent: None }
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Spinner::new()
    }
}

impl Component for Spinner {
    const ROLE: Role = Role::PROGRESS_INDICATOR;
    const PARTS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = SpinnerStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: SpinnerStyleKey) -> SpinnerStyle {
        let ink = Stateful::new(cx.color(cx.tokens.color.text.normal));
        let (icon, control) = (&cx.tokens.size.icon, &cx.tokens.size.control);
        let side = match key.extent {
            None => cx.control().icon,
            Some(SpinnerSize::Sm) => icon.sm,
            Some(SpinnerSize::Md) => icon.md,
            Some(SpinnerSize::Lg) => icon.lg,
            Some(SpinnerSize::Control) => control.md,
        };
        SpinnerStyle { metrics: Metrics::new(), arc: IconLook::new(side, cx.host_ink(ink)), inset: cx.tokens.space.gap.tight }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.arc.side)
    }

    fn sense(&self) -> Sense {
        super::display_sense(&self.common, true)
    }

    fn flags(&self) -> Flags {
        Flags::BUSY
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if cx.resolved.label.is_empty() {
            access.role(Role::None);
        } else {
            access.busy(true);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let centre = cx.rect.center();
        let radius = (cx.style.arc.side / 2.0 - cx.style.inset).max(0.0);
        let start = cx.phase(PERIOD, None).map_or(0.0, |turn| turn * TAU);
        let points: Vec<Pos2> = (0..=POINTS)
            .map(|at| {
                let angle = start + SWEEP * at as f32 / POINTS as f32;
                centre + radius * vec2(angle.cos(), angle.sin())
            })
            .collect();
        ui.draw().stroke(&PaintShape::Path(points), StrokeWidth::Heavy, cx.resolve(&cx.style.arc.color));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
