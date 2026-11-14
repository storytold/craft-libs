//! `Badge`: a small label in a tone, soft or solid, and the count form.

use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Styled as _};
use crate::primitives::{TextRun, Wrap};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx, SurfaceLook, TextLook, TypeRole};
use craft_behavior::{Flow, Insets, Metrics, Sense};
use craft_theme::{
    Variant,
    variant::{Emphasis, Tone},
};
use egui::{Response, Vec2};
use std::borrow::Cow;

#[cfg(feature = "testing")]
mod stories;

/// The largest count a count badge shows; more reads `99+`.
pub const MAX_COUNT: u32 = 99;

/// The style of a badge.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[style(variants = [
    Variant::look(Emphasis::Soft, Tone::Neutral),
    Variant::look(Emphasis::Soft, Tone::Accent),
    Variant::look(Emphasis::Soft, Tone::Info),
    Variant::look(Emphasis::Soft, Tone::Good),
    Variant::look(Emphasis::Soft, Tone::Warn),
    Variant::look(Emphasis::Soft, Tone::Bad),
    Variant::look(Emphasis::Solid, Tone::Neutral),
    Variant::look(Emphasis::Solid, Tone::Accent),
    Variant::look(Emphasis::Solid, Tone::Info),
    Variant::look(Emphasis::Solid, Tone::Good),
    Variant::look(Emphasis::Solid, Tone::Warn),
    Variant::look(Emphasis::Solid, Tone::Bad),
    Variant::look(Emphasis::Outline, Tone::Neutral),
])]
#[non_exhaustive]
pub struct BadgeStyle {
    /// The pill: the variant's fill and edge.
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    /// The label.
    #[style(text)]
    text: TextLook,
    /// The label of a muted badge: the variant's secondary ink.
    #[style(text)]
    muted: TextLook,
}

/// A small label in a tone: a soft tint by default, a solid fill with a solid variant, an edge without fill with the
/// secondary variant (a chip). The tone is the variant
/// (`Variant::look(Emphasis::Soft, Tone::Good)`); the text always says what the tone means. A count reads `99+`
/// above `MAX_COUNT`. A label longer than the badge's room is cut at its end.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Badge {
    #[common]
    common: Common,
    /// The label in the muted text colour: what only some of a selection carries (a partial chip).
    #[prop(flag)]
    muted: bool,
    /// The pill's end holds a square as high as the pill, in place of its end padding, for a mark the host draws over
    /// it (a chip's remove button).
    #[prop(flag)]
    end_mark: bool,
}

impl Badge {
    /// A neutral, soft badge showing `label`, which is also its accessible name.
    pub fn new(label: impl Into<Label>) -> Self {
        Badge { common: Common::new(), muted: false, end_mark: false }.label(label)
    }

    /// A count: `99+` above `MAX_COUNT`.
    pub fn count(n: u32) -> Self {
        let text = if n > MAX_COUNT { format!("{MAX_COUNT}+") } else { n.to_string() };
        Badge::new(Label::from_cow(Cow::Owned(text)))
    }
}

impl Badge {
    /// The width the end mark adds to the content `height` high: the pill's height less the end padding it replaces.
    fn mark_room(&self, style: &BadgeStyle, height: f32) -> f32 {
        let padding = style.metrics.padding;
        if self.end_mark { (height + padding.top + padding.bottom - padding.end).max(0.0) } else { 0.0 }
    }
}

impl Component for Badge {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const OVERLAYS: bool = false;
    type Style = BadgeStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: BadgeStyleKey) -> BadgeStyle {
        let parts = cx.variant_look();
        let control = cx.control();
        let pad_y = cx.tokens.space.pad.chip;
        let mut surface = SurfaceLook::new(parts.fill.clone()).radius(crate::style::UiTokens::corner(cx.tokens.shape.radius.pill));
        if let Some(edge) = &parts.edge {
            surface = surface.border(edge.clone(), cx.tokens.stroke.thin);
        }
        BadgeStyle {
            surface,
            metrics: Metrics::new().padding(Insets::new(control.pad_x, control.pad_x, pad_y, pad_y)),
            text: TextLook::new(TypeRole::Caption, parts.text),
            muted: TextLook::new(TypeRole::Caption, parts.muted),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let text = cx.layout(&TextRun::new(cx.resolved.label).role(cx.style.text.role)).size();
        Vec2::new(text.x + self.mark_room(cx.style, text.y), text.y)
    }

    fn sense(&self) -> Sense {
        super::display_sense(&self.common, false)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let text = if self.muted { &cx.style.muted } else { &cx.style.text };
        let run = TextRun::new(cx.resolved.label).role(text.role).color(cx.resolve(&text.color)).wrap(Wrap::Elide);
        let content = cx.content_rect;
        let room = self.mark_room(cx.style, content.height());
        let (label, _) = Flow::new(content, cx.flow.is_rtl()).take_start((content.width() - room).max(0.0));
        let layout = draw.layout(&run, label.width());
        draw.galley(label.center() - layout.size() / 2.0, &layout);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
