//! `Checkbox`: a box with a check mark, a dash while the value is mixed. Its text is the label step's (a `Field`'s,
//! `PaintUi::labelled`'s, or the one `show` runs for a labelled instance); the control draws only the box.

use crate::contract::{Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, MeasureCx, PaintUi, Pointer as _, Role, Styled as _};
use crate::primitives::{PaintShape, StrokeWidth, glyph};
use crate::runtime::focus::{ActivateKeys, KeyUse};
use crate::style::{ComponentStyle, Flags, FocusLook, StyleColor, StyleCx, SurfaceKind};
use craft_behavior::{Metrics, Sense};
use craft_theme::{Stateful, States, variant::Tone};
use egui::{CornerRadius, Rect, Vec2};

/// The mixed dash's half-length as a share of the box's side.
const DASH: f32 = 0.25;

/// The style of a check box.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CheckboxStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The box fill.
    fill: Stateful<StyleColor>,
    /// The box edge.
    edge: Stateful<StyleColor>,
    /// The check mark and the dash.
    mark: Stateful<StyleColor>,
    radius: CornerRadius,
    side: f32,
}

/// A check box over a bool; a `Mixed` binding shows the mixed state and a click makes it checked.
#[derive(crate::contract::Component)]
pub struct Checkbox<'a> {
    #[common]
    common: Common,
    value: Bind<'a, bool>,
}

impl<'a> Checkbox<'a> {
    /// A check box of `value`.
    pub fn new(value: Bind<'a, bool>) -> Self {
        Checkbox { common: Common::new(), value }
    }
}

impl Component for Checkbox<'_> {
    const ROLE: Role = Role::CHECK_BOX;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Space);
    const PARTS: bool = false;
    const LABELLED_AFTER: bool = true;
    const OVERLAYS: bool = false;
    type Style = CheckboxStyle;
    type Output = EditResponse<bool>;

    fn style(cx: &StyleCx<'_>, _: CheckboxStyleKey) -> CheckboxStyle {
        let control = cx.control();
        let colors = &cx.tokens.color;
        let solid = cx.color(colors.accent.solid);
        let disabled = cx.color(colors.text.disabled);
        let fill = super::well(cx).on(States::new(solid, solid, solid, disabled));
        let edge = super::outline(cx).on(States::new(solid, solid, solid, disabled)).when(Flags::INVALID, cx.tone(Tone::Bad));
        CheckboxStyle {
            metrics: Metrics::new().min_size(Vec2::new(0.0, control.height)),
            focus: cx.focus_look(),
            fill,
            edge,
            mark: super::mark(cx),
            radius: cx.surface(SurfaceKind::Well).radius,
            side: control.icon,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.icon())
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
        let square = Rect::from_center_size(cx.rect.center(), Vec2::splat(cx.style.side));
        let draw = ui.draw();
        let shape = PaintShape::Rect(square, cx.style.radius);
        draw.fill(&shape, cx.resolve(&cx.style.fill));
        draw.stroke(&shape, StrokeWidth::Medium, cx.resolve(&cx.style.edge));
        if cx.mixed {
            let c = square.center();
            let half = Vec2::new(cx.style.side * DASH, 0.0);
            draw.stroke(&PaintShape::Path(vec![c - half, c + half]), StrokeWidth::Medium, cx.resolve(&cx.style.mark));
        } else if cx.value().copied().unwrap_or(false) {
            draw.stroke(&glyph::check(square), StrokeWidth::Medium, cx.resolve(&cx.style.mark));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<bool> {
        cx.edit_response()
    }
}

#[cfg(feature = "testing")]
mod stories;
