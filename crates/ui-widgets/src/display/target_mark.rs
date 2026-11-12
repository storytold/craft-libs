//! `TargetMark`: a pulsing accent ring around a part of the window, on a passive layer above the content, so a guide
//! (a tour step) shows the part it explains.

use crate::contract::overlay::OverlayFit;
use crate::contract::{
    Animate as _, Bind, BindMut, Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Pad, PaintUi, Role, ShowContainer as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::Env;
use crate::style::{ComponentStyle, StyleColor, StyleCx, UiTokens};
use craft_behavior::{Metrics, Sense};
use craft_theme::Mix as _;
use egui::{CornerRadius, Rect, Response, Ui};
use std::f32::consts::TAU;

#[cfg(feature = "testing")]
mod stories;

/// The period of the pulse in seconds: a sine of four radians per second.
const PULSE_PERIOD: f64 = std::f64::consts::FRAC_PI_2;
/// The ring's opacity at the low point of its pulse; it reaches full opacity at the top.
const DIMMEST: f32 = 0.6;

/// How the ring stands off its target: the gap left around the target, how far the ring grows at the top of its pulse,
/// and its line width.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reach {
    gap: f32,
    pulse: f32,
    width: f32,
}

impl Reach {
    fn of(tokens: &UiTokens) -> Self {
        Reach { gap: tokens.space.gap.tight, pulse: tokens.space.gap.tight, width: tokens.stroke.thick }
    }

    /// The rect the ring covers at the top of its pulse: the layer's rect.
    fn outer(self, target: Rect) -> Rect {
        target.expand(self.gap + self.pulse + self.width)
    }

    /// The line of the ring at `pulse` (0 to 1) of its growth, the stroke lying outside the gap.
    fn line(self, target: Rect, pulse: f32) -> Rect {
        target.expand(self.gap + self.pulse * pulse + self.width / 2.0)
    }
}

/// The look of a target mark.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TargetMarkStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The ring at the top of its pulse.
    ring: StyleColor,
    /// The ring at the low point of its pulse.
    ring_dim: StyleColor,
    /// The ring's corner: a control's.
    radius: CornerRadius,
    reach: Reach,
}

/// A ring around `target` that pulses in the accent colour, still where motion is off. It sits on a layer above the
/// content that takes no presses, no hover and no focus, so the part it marks stays usable. Show it with
/// [`show_mark`](TargetMark::show_mark) in every pass the mark shows.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct TargetMark {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    target: Rect,
    open: Bind<'static, bool>,
}

impl TargetMark {
    /// A mark around `target`, a rect in window coordinates.
    pub fn new(target: Rect) -> Self {
        TargetMark { common: Common::new(), props: ContainerProps::new().pad(Pad::None), target, open: Bind::given(true) }
    }

    /// Shows the mark around its target.
    pub fn show_mark(mut self, ui: &mut Ui) -> Response {
        let outer = Reach::of(&Env::of(ui).tokens).outer(self.target);
        let layer = Overlay::tooltip().fit(OverlayFit::Content).placed(outer).body_scroll(false);
        self.props = self.props.mount(Mount::Layer(layer));
        self.show(ui, |_, _| ()).output
    }
}

impl Component for TargetMark {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = TargetMarkStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: TargetMarkStyleKey) -> TargetMarkStyle {
        let accent = cx.tokens.color.accent.solid;
        TargetMarkStyle {
            metrics: Metrics::new(),
            ring: cx.color(accent),
            ring_dim: cx.color(accent.faded(DIMMEST)),
            radius: UiTokens::corner(cx.tokens.shape.radius.control),
            reach: Reach::of(cx.tokens),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        // Still, the ring stands at the top of its pulse.
        let pulse = cx.phase(PULSE_PERIOD, None).map_or(1.0, |phase| 0.5 + 0.5 * (phase * TAU).sin());
        let line = style.reach.line(self.target, pulse);
        ui.draw().stroke(&PaintShape::Rect(line, style.radius), StrokeWidth::Thick, style.ring_dim.mix(style.ring, pulse));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for TargetMark {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }
}
