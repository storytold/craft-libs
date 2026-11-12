//! `Switch`: an on/off setting that applies at once, a track with a thumb that slides to the on end. Its text is the
//! label step's, as the check box's is.

use crate::contract::{
    Animate as _, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, PaintUi, Pointer as _, Role, Styled as _, spec,
};

use crate::style::{ComponentStyle, FocusLook, StyleCx, SurfaceKind, SurfaceLook, SurfaceStyle};
use craft_behavior::{Flow, Metrics, Sense};
use craft_theme::{Stateful, States};
use egui::{CornerRadius, Rect, Vec2};

/// Where the thumb rests while the value is mixed: the middle of its travel.
const MIXED_AT: f32 = 0.5;
/// The track's length in track heights (the old `SWITCH_ASPECT`).
const LENGTH_IN_HEIGHTS: f32 = 2.0;

/// The style of a switch.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SwitchStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The track.
    track: SurfaceLook,
    /// The thumb: a round knob surface, with its outline and shadow.
    thumb: Stateful<SurfaceStyle>,
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
    const PARTS: bool = false;
    const LABELLED_AFTER: bool = true;
    const OVERLAYS: bool = false;
    type Style = SwitchStyle;
    type Output = EditResponse<bool>;

    fn style(cx: &StyleCx<'_>, _: SwitchStyleKey) -> SwitchStyle {
        let control = cx.control();
        let colors = &cx.tokens.color;
        let radius = CornerRadius::same((control.icon / 2.0).round() as u8);
        let thumb = SurfaceStyle { radius: CornerRadius::same(u8::MAX), ..cx.surface(SurfaceKind::Knob) };
        SwitchStyle {
            metrics: Metrics::new().min_size(Vec2::new(0.0, control.height)),
            focus: cx.focus_look(),
            track: super::indicator(cx, radius),
            thumb: Stateful::new(thumb).off(States::new(
                thumb,
                thumb,
                thumb,
                thumb.disabled(cx.color(colors.state.disabled_fill).egui(), cx.color(colors.line.divider).egui()),
            )),
            width: control.icon * LENGTH_IN_HEIGHTS,
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
        draw.surface_look(track, &style.track, cx.state());
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
