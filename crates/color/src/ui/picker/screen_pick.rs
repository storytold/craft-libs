//! The screen-pick button: an icon button that also reports the press and drag that follow it, for Photoshop's
//! press-and-drag sampling.

use craft_behavior::{IconId, Metrics, PressPhase, Sense};
use craft_theme::{Stateful, States, variant::Tone};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, NoValue, Outcome, PaintUi, Pointer as _, Role, Styled as _};
use craft_ui::primitives::IconSpec;
use craft_ui::style::{ComponentStyle, FocusLook, IconLook, Ink, StyleCx};
use egui::{Rect, Response, Vec2};

use super::ScreenPickGesture;

/// The style of the screen-pick button: the glyph and the focus ring.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(super) struct ScreenPickButtonStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The glyph: its side, and its colour per state.
    #[style(icon)]
    glyph: IconLook,
}

/// What the button reports.
#[derive(Debug)]
#[non_exhaustive]
pub(super) struct ScreenPickOutput {
    pub response: Response,
    /// The press of this pass.
    pub gesture: Option<ScreenPickGesture>,
}

impl Outcome for ScreenPickOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The eyedropper button; its label is its accessible name.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub(super) struct ScreenPickButton {
    #[common]
    common: Common,
    gesture: Option<ScreenPickGesture>,
}

impl ScreenPickButton {
    pub(super) fn new() -> Self {
        ScreenPickButton { common: Common::new(), gesture: None }
    }
}

impl Component for ScreenPickButton {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = ScreenPickButtonStyle;
    type Output = ScreenPickOutput;

    fn style(cx: &StyleCx<'_>, _: ScreenPickButtonStyleKey) -> ScreenPickButtonStyle {
        let control = cx.control();
        let (rest, accent) = (cx.ink(Ink::OnPanel), cx.tone(Tone::Accent));
        let disabled = cx.color(cx.tokens.color.text.disabled);
        ScreenPickButtonStyle {
            metrics: Metrics::new().min_size(Vec2::splat(control.height)),
            focus: cx.focus_look(),
            glyph: IconLook::new(control.icon, Stateful::new(rest).off(States::new(rest, accent, accent, disabled))),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.glyph.side)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::DRAG | Sense::FOCUS
    }

    /// A drag past the slop is the held press; its release outside a click ends it; a click or Enter is a click.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.gesture = match cx.press().phase {
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => Some(ScreenPickGesture::Held { at: pos }),
            PressPhase::Release { pos, click: false, .. } => Some(ScreenPickGesture::Released { at: pos }),
            _ => cx.activation().map(|_| ScreenPickGesture::Clicked),
        };
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let glyph = Rect::from_center_size(cx.rect.center(), Vec2::splat(cx.style.glyph.side));
        let icon = IconSpec::svg(IconId::new("eyedropper")).color(cx.resolve(&cx.style.glyph.color));
        ui.draw().icon(glyph, &icon);
    }

    fn finish(self, cx: Cx<'_, Self>) -> ScreenPickOutput {
        ScreenPickOutput { response: cx.response.clone(), gesture: self.gesture }
    }
}
