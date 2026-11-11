//! `Avatar`: a person's round face, the initials of their name on their colour.

use craft_behavior::{Metrics, Sense};
use craft_theme::variant::Variant;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, NoValue, Outcome, PaintUi, Props as _, Role, clicked};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth, TextRun};
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, StyleColor, StyleCx, TypeRole};
use egui::{Color32, Rect, Response, Vec2};

use crate::collab::initials;

#[cfg(feature = "screen-testing")]
mod stories;

/// Paints a round face filling `rect`: `fill` with the initials of `name` centred on it, in the ink that reads on `fill`.
pub fn paint_avatar(ui: &PaintUi<'_>, rect: Rect, name: &str, fill: DataColor) {
    let draw = ui.draw();
    draw.data_fill(&circle(rect), fill);
    let ink = ui.with_style(Variant::SECONDARY, |cx| cx.ink_on(fill));
    paint_initials(&draw, rect, name, ink);
}

/// Paints a round face in a theme colour `fill` with the initials of `name` in `ink`.
pub(super) fn paint_face(draw: &Draw<'_>, rect: Rect, name: &str, fill: StyleColor, ink: StyleColor) {
    draw.fill(&circle(rect), fill);
    paint_initials(draw, rect, name, ink);
}

fn circle(rect: Rect) -> PaintShape {
    PaintShape::Circle(rect.center(), rect.width().min(rect.height()) / 2.0)
}

fn paint_initials(draw: &Draw<'_>, rect: Rect, name: &str, ink: StyleColor) {
    let run = TextRun::new(initials(name)).role(TypeRole::Caption).color(ink);
    let size = draw.layout(&run, f32::INFINITY).size();
    let size = Vec2::new(size.x.min(rect.width()), size.y);
    draw.text(Rect::from_center_size(rect.center(), size), &run);
}

/// The style of an avatar.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct AvatarStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The fill of an avatar without a colour of its own.
    accent: StyleColor,
    /// The initials on the accent fill.
    on_accent: StyleColor,
    /// The ring of a selected or hovered avatar.
    ring: StyleColor,
    side: f32,
}

/// What an avatar returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct AvatarOutput {
    /// The avatar's response.
    pub response: Response,
    /// The avatar was pressed by pointer or key.
    pub clicked: bool,
}

impl Outcome for AvatarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A round well at the control height with the initials of a person's name, named by the name. `fill` gives it the
/// person's colour, else it takes the accent; `selected` draws a ring round it. A pressable avatar is a button.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct Avatar {
    #[common]
    common: Common,
    /// The person's colour.
    #[prop]
    fill: Option<[u8; 3]>,
    /// The avatar is a button that reports a press.
    #[prop]
    pressable: bool,
}

impl Avatar {
    /// An avatar of the person `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Avatar { common: Common::new(), fill: None, pressable: false }.label(name.into())
    }
}

impl Component for Avatar {
    const ROLE: Role = Role::BUTTON;
    const ALT_ROLES: &'static [Role] = &[Role::IMAGE];
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = AvatarStyle;
    type Output = AvatarOutput;

    fn style(cx: &StyleCx<'_>, _: AvatarStyleKey) -> AvatarStyle {
        let colors = &cx.tokens.color;
        let control = cx.control();
        AvatarStyle {
            metrics: Metrics::new().min_size(Vec2::splat(control.height)),
            focus: cx.focus_look(),
            accent: cx.color(colors.accent.solid),
            on_accent: cx.color(colors.text.on_strong),
            ring: cx.color(colors.accent.solid),
            side: control.height,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn sense(&self) -> Sense {
        if self.pressable { Sense::CLICK | Sense::FOCUS } else { Sense::HOVER }
    }

    fn access(&self, access: &mut craft_ui::contract::Access, _: &Cx<'_, Self>) {
        if !self.pressable {
            access.role(Role::IMAGE);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let side = cx.content_rect.width().min(cx.content_rect.height());
        let face = Rect::from_center_size(cx.content_rect.center(), Vec2::splat(side));
        match self.fill {
            Some([r, g, b]) => paint_avatar(ui, face, cx.resolved.label, DataColor::new(Color32::from_rgb(r, g, b))),
            None => paint_face(&ui.draw(), face, cx.resolved.label, style.accent, style.on_accent),
        }
        let draw = ui.draw();
        if cx.resolved.selected == Some(true) || (self.pressable && cx.response.hovered()) {
            draw.stroke(&PaintShape::Circle(face.center(), side / 2.0), StrokeWidth::Medium, style.ring);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> AvatarOutput {
        AvatarOutput { clicked: clicked(&cx.response), response: cx.response.clone() }
    }
}
