//! `Switch`: an on/off setting that applies at once, a track with a thumb that slides to the on end. Its text is the
//! label step's, as the check box's is.

use crate::contract::{
    Animate as _, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, MeasureCx, PaintUi, Pointer as _, Role, Styled as _, spec,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::focus::{ActivateKeys, KeyUse};
use crate::style::{ComponentStyle, Flags, FocusLook, StyleColor, StyleCx, SurfaceKind, SurfaceStyle};
use craft_behavior::{Flow, Metrics, Sense};
use craft_theme::{Stateful, States, variant::Tone};
use egui::{CornerRadius, Rect, Vec2};

/// Where the thumb rests while the value is mixed: the middle of its travel.
const MIXED_AT: f32 = 0.5;

/// The style of a switch.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SwitchStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The track fill.
    track: Stateful<StyleColor>,
    /// The track edge.
    edge: Stateful<StyleColor>,
    /// The thumb: a round knob surface, with its outline and shadow.
    thumb: Stateful<SurfaceStyle>,
    radius: CornerRadius,
    width: f32,
    height: f32,
    /// Between the track's edge and the thumb.
    inset: f32,
}

/// A switch over a bool; a `Mixed` binding rests the thumb in the middle and a click turns it on.
#[derive(crate::contract::Component)]
pub struct Switch<'a> {
    #[common]
    common: Common,
    value: Bind<'a, bool>,
}

impl<'a> Switch<'a> {
    /// A switch of `value`.
    pub fn new(value: Bind<'a, bool>) -> Self {
        Switch { common: Common::new(), value }
    }
}

impl Component for Switch<'_> {
    const ROLE: Role = Role::SWITCH;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Space);
    const PARTS: bool = false;
    const LABELLED_AFTER: bool = true;
    const OVERLAYS: bool = false;
    type Style = SwitchStyle;
    type Output = EditResponse<bool>;

    fn style(cx: &StyleCx<'_>, _: SwitchStyleKey) -> SwitchStyle {
        let control = cx.control();
        let colors = &cx.tokens.color;
        let on = cx.color(colors.accent.solid);
        let disabled = cx.color(colors.text.disabled);
        let track = super::well(cx).on(States::new(on, on, on, disabled));
        let edge = super::outline(cx).on(States::new(on, on, on, disabled)).when(Flags::INVALID, cx.tone(Tone::Bad));
        let thumb = SurfaceStyle { radius: CornerRadius::same(u8::MAX), ..cx.surface(SurfaceKind::Knob) };
        SwitchStyle {
            metrics: Metrics::new().min_size(Vec2::new(0.0, control.height)),
            focus: cx.focus_look(),
            track,
            edge,
            thumb: Stateful::new(thumb).off(States::new(
                thumb,
                thumb,
                thumb,
                thumb.disabled(cx.color(colors.state.disabled_fill).egui(), cx.color(colors.line.divider).egui()),
            )),
            radius: CornerRadius::same((control.icon / 2.0).round() as u8),
            width: cx.tokens.size.toggle,
            height: control.icon,
            inset: cx.tokens.space.gap.tight,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.style.width, cx.style.height)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn bind(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if cx.activation().is_some() {
            let on = cx.value().copied().unwrap_or(false);
            cx.set(!on);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let on = cx.value().copied().unwrap_or(false);
        let at = if cx.mixed { MIXED_AT } else { cx.anim_bool("on", on, spec::CHECK) };
        let track = Rect::from_center_size(cx.rect.center(), Vec2::new(style.width, style.height));
        let draw = ui.draw();
        let shape = PaintShape::Rect(track, style.radius);
        draw.fill(&shape, cx.resolve(&style.track));
        draw.stroke(&shape, StrokeWidth::Medium, cx.resolve(&style.edge));
        let slot = Flow::new(track, cx.flow.is_rtl()).span(at * (style.width - style.height), style.height);
        let size = style.height - style.inset * 2.0;
        ui.draw().surface_styled(Rect::from_center_size(slot.center(), Vec2::splat(size)), &cx.resolve(&style.thumb));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<bool> {
        cx.edit_response()
    }
}

#[cfg(feature = "testing")]
mod stories;
