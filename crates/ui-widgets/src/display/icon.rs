//! `Icon`: an icon of the catalog or a value glyph at the icon side of the control size.

use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Role, Styled as _};
use crate::primitives::IconSpec;
use crate::style::{ComponentStyle, IconLook, Ink, StyleCx};
use craft_behavior::{Metrics, Sense};
use craft_theme::variant::Tone;
use craft_theme::{Stateful, States};
use egui::{Response, Vec2};

#[cfg(feature = "testing")]
mod stories;

/// The style of an icon.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct IconStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The side and the ink colour.
    #[style(icon)]
    icon: IconLook,
}

/// An icon. It mirrors under right-to-left when it has a reading direction. It is decoration unless labelled: only
/// a labelled icon is in the AccessKit tree, as an image, and its label shows as its tooltip.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Icon<'a> {
    #[common]
    common: Common,
    /// The ink, which is part of the style key.
    #[style_key]
    ink: Ink,
    /// The tone the icon takes its colour from, in place of the ink.
    #[style_key]
    #[prop]
    tone: Option<Tone>,
    spec: IconSpec<'a>,
}

impl<'a> Icon<'a> {
    /// An icon of `spec`, at the icon side of the control size.
    pub fn new(spec: IconSpec<'a>) -> Self {
        Icon { common: Common::new(), ink: spec.ink, tone: None, spec }
    }
}

impl Component for Icon<'_> {
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = IconStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: IconStyleKey) -> IconStyle {
        let color = key.tone.map_or_else(|| cx.ink(key.ink), |tone| cx.tone(tone));
        let off = States::new(color, color, color, cx.color(cx.tokens.color.text.disabled));
        IconStyle { metrics: Metrics::new(), icon: IconLook::new(cx.control().icon, Stateful::new(color).off(off)) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.icon.side)
    }

    fn sense(&self) -> Sense {
        super::display_sense(&self.common, true)
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if cx.resolved.label.is_empty() {
            access.role(Role::None);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.spec.color.is_none() {
            self.spec.color = Some(cx.resolve(&cx.style.icon.color));
        }
        ui.draw().icon(cx.content_rect, &self.spec);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
