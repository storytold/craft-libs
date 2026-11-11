//! `IconButton`: a square ghost button that shows only an icon.

use super::button::ButtonOutput;
use super::face::{self, tinted};
use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, PopupKind, Props, Role, Styled as _};
use crate::primitives::{IconSpec, PaintShape, StrokeWidth};
use crate::runtime::commands::CommandRef;
use crate::style::{ComponentStyle, DataColor, FocusLook, IconLook, StyleColor, StyleCx, SurfaceLook};
use craft_behavior::{Metrics, Sense};
use craft_theme::{
    Srgba, Variant,
    variant::{Emphasis, Tone},
};
use egui::{Color32, Pos2, Vec2};

/// The style of an icon button.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[style(variants = [
    Variant::GHOST,
    Variant::look(Emphasis::Ghost, Tone::Accent),
    Variant::look(Emphasis::Ghost, Tone::Info),
    Variant::look(Emphasis::Ghost, Tone::Good),
    Variant::look(Emphasis::Ghost, Tone::Warn),
    Variant::look(Emphasis::Ghost, Tone::Bad),
])]
#[non_exhaustive]
pub struct IconButtonStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The glyph: its side, and its colour per state (the on look takes the accent text).
    #[style(icon)]
    glyph: IconLook,
    /// The marker dot's diameter.
    marker: f32,
    /// The marker dot of a marked command.
    marker_color: StyleColor,
    /// The ring around the marker dot.
    marker_edge: StyleColor,
    /// The popup mark at the end-bottom corner: its side, which is also its inset from the corner.
    mark: f32,
    /// The popup mark's colour.
    mark_color: StyleColor,
}

/// A square button showing only an icon. Its label is its accessible name; a tooltip is the caller's. A selected
/// button (`selected(true)`) shows the on look: the host owns the state and a click is reported in the response.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct IconButton<'a> {
    #[common]
    common: Common,
    icon: IconSpec<'a>,
    /// Whether the button is a Tab stop; off for a button that repeats a key of the control it sits in, as a search
    /// field's clear button repeats Escape.
    #[prop]
    focusable: bool,
    /// A small dot of the colour at the end-bottom corner of the glyph, ringed so it shows on any fill. The colour
    /// is the user's content, so it has no contrast pair. It takes the place of the dot a marked command shows in the
    /// accent.
    #[prop(into)]
    badge: Option<Srgba>,
    /// The kind of popup the button opens: AccessKit reports it, and a mark shows at the end-bottom corner unless
    /// [`popup_mark`](Self::popup_mark) is off.
    #[prop(into)]
    popup: Option<PopupKind>,
    /// Whether the popup mark shows; off where the glyph itself names the popup, as a panel menu's "…" does.
    #[prop]
    popup_mark: bool,
    /// Whether the popup the button opens is open, as the host knows it.
    #[prop(into)]
    expanded: Option<bool>,
}

impl<'a> IconButton<'a> {
    /// An icon button showing `icon`; `label` it with the accessible name.
    pub fn new(icon: impl Into<IconSpec<'a>>) -> Self {
        IconButton { common: Common::new(), icon: icon.into(), focusable: true, badge: None, popup: None, popup_mark: true, expanded: None }
    }

    /// An icon button bound to `command`: its label, enabled state and activation follow the command.
    pub fn command(icon: impl Into<IconSpec<'a>>, command: impl Into<CommandRef>) -> Self {
        <IconButton as Props>::command(IconButton::new(icon), command)
    }
}

impl Component for IconButton<'_> {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = IconButtonStyle;
    type Output = ButtonOutput;

    fn style(cx: &StyleCx<'_>, _: IconButtonStyleKey) -> IconButtonStyle {
        let face = face::icon_face(cx);
        IconButtonStyle {
            surface: face.surface,
            metrics: face.metrics,
            focus: face::focus(cx),
            glyph: face.glyph,
            marker: cx.tokens.size.handle.sm,
            marker_color: cx.color(cx.tokens.color.accent.solid),
            marker_edge: cx.color(cx.tokens.color.line.mark),
            mark: cx.tokens.space.gap.tight,
            mark_color: cx.color(cx.tokens.color.text.faint),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.glyph.side)
    }

    fn sense(&self) -> Sense {
        if self.focusable { Sense::CLICK | Sense::FOCUS } else { Sense::CLICK }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let glyph = egui::Rect::from_center_size(cx.rect.center(), Vec2::splat(cx.style.glyph.side));
        let draw = ui.draw();
        draw.icon(glyph, &tinted(&self.icon, cx.resolve(&cx.style.glyph.color)));
        if self.popup.is_some() && self.popup_mark {
            let (end, bottom) = if cx.flow.is_rtl() {
                (cx.rect.left() + cx.style.mark, cx.rect.bottom() - cx.style.mark)
            } else {
                (cx.rect.right() - cx.style.mark, cx.rect.bottom() - cx.style.mark)
            };
            let reach = if cx.flow.is_rtl() { cx.style.mark } else { -cx.style.mark };
            let corner = Pos2::new(end, bottom);
            let triangle = PaintShape::Polygon(vec![corner, Pos2::new(end, bottom - cx.style.mark), Pos2::new(end + reach, bottom)]);
            draw.fill(&triangle, cx.style.mark_color);
        }
        let marked = cx.command.as_ref().is_some_and(|command| command.marked);
        if self.badge.is_some() || marked {
            let radius = cx.style.marker / 2.0;
            let x = if cx.flow.is_rtl() { glyph.left() + radius } else { glyph.right() - radius };
            let dot = PaintShape::Circle(Pos2::new(x, glyph.bottom() - radius), radius);
            match self.badge {
                Some(badge) => draw.data_fill(&dot, DataColor::new(Color32::from(badge))),
                None => draw.fill(&dot, cx.style.marker_color),
            }
            draw.stroke(&dot, StrokeWidth::Thin, cx.style.marker_edge);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        if let Some(kind) = self.popup {
            access.has_popup(kind);
        }
        if let Some(open) = self.expanded {
            access.expanded(open);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ButtonOutput {
        ButtonOutput::of(&cx, self.activated(&cx))
    }
}
