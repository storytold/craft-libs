//! `IconButton`: a square ghost button that shows only an icon.

use super::button::ButtonOutput;
use super::face::{self, tinted};
use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, PopupKind, Props, Role, Styled as _};
use crate::controls::disclosure::DisclosureGlyph;
use crate::primitives::{Draw, IconSpec, PaintShape, StrokeWidth};
use crate::runtime::commands::{CommandRef, ResolvedCommand};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, FocusLook, IconLook, StyleColor, StyleCx, SurfaceLook};
use craft_behavior::{Metrics, Sense};
use craft_i18n::Direction;
use craft_theme::{
    Variant,
    variant::{Emphasis, Tone},
};
use egui::{Pos2, Vec2};

/// The marker dot's look: the one reading of the tokens that `IconButton` and `Tab` paint it from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MarkerStyle {
    /// The dot's diameter.
    pub(crate) side: f32,
    /// The dot's fill.
    pub(crate) color: StyleColor,
    /// The dot's edge.
    pub(crate) edge: StyleColor,
}

impl MarkerStyle {
    /// The marker look from the tokens.
    pub(crate) fn of(cx: &StyleCx<'_>) -> MarkerStyle {
        MarkerStyle { side: cx.tokens.size.handle.sm, color: cx.color(cx.tokens.color.accent.solid), edge: cx.color(cx.tokens.color.line.mark) }
    }
}

/// Paints the marker dot centred on `center`.
pub(crate) fn paint_marker(draw: &Draw<'_>, center: Pos2, style: &MarkerStyle) {
    let dot = PaintShape::Circle(center, style.side / 2.0);
    draw.fill(&dot, style.color);
    draw.stroke(&dot, StrokeWidth::Thin, style.edge);
}

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
    /// The tool group's triangle at the end-bottom corner: its side, which is also its inset from the corner.
    mark: f32,
    /// The side of the square the menu arrow is drawn in.
    arrow: f32,
    /// The colour of the triangle and the menu arrow.
    mark_color: StyleColor,
}

/// The mark an icon button shows for the popup it opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PopupMark {
    /// No mark.
    #[default]
    None,
    /// A rail button that stands for a group of tools: a filled triangle at its end-bottom corner.
    ToolGroup,
    /// A button that opens a menu: the dropdown arrow under the end edge of its glyph.
    Menu,
}

/// How an icon button's pointer press affects the focused control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PressFocus {
    /// Follow the standard focus policy.
    #[default]
    Standard,
    /// Leave focus with the current control on press.
    Keep,
}

/// A square button showing only an icon. Its label is its accessible name; a tooltip is the caller's. A selected
/// button (`selected(true)`) shows the on look: the host owns the state and a click is reported in the response.
/// [`ButtonOutput::held`] reports the press from its start to its end. A button whose menu picks one of its rows (an
/// Align To button) gives the picked row as its [`choice`](Self::choice): it then reads as a combo box named by its
/// label, with the choice as its value.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct IconButton<'a> {
    #[common]
    common: Common,
    icon: IconSpec<'a>,
    /// Whether the button is a Tab stop; off for a button that repeats a key of the control it sits in, as a search
    /// field's clear button repeats Escape.
    #[prop]
    focusable: bool,
    /// The focus policy for a pointer press.
    #[prop]
    press_focus: PressFocus,
    /// The kind of popup the button opens: AccessKit reports it, and [`popup_mark`](Self::popup_mark) shows it.
    #[prop(into)]
    popup: Option<PopupKind>,
    /// The mark that tells the button opens more than it shows: a rail button with a flyout and a panel footer button
    /// that opens a menu have one.
    #[prop]
    popup_mark: PopupMark,
    /// Whether the popup the button opens is open, as the host knows it. A button with a [`choice`](Self::choice)
    /// reports, without it, whether the container hung from it is open.
    #[prop(into)]
    expanded: Option<bool>,
    /// The row the menu it opens has picked, for a button that picks one of its menu's rows.
    #[prop(into)]
    choice: Option<Label>,
}

impl<'a> IconButton<'a> {
    /// An icon button showing `icon`; `label` it with the accessible name.
    pub fn new(icon: impl Into<IconSpec<'a>>) -> Self {
        IconButton {
            common: Common::new(),
            icon: icon.into(),
            focusable: true,
            press_focus: PressFocus::Standard,
            popup: None,
            popup_mark: PopupMark::None,
            expanded: None,
            choice: None,
        }
    }

    /// An icon button bound to `command`: its label, enabled state and activation follow the command.
    pub fn command(icon: impl Into<IconSpec<'a>>, command: impl Into<CommandRef>) -> Self {
        <IconButton as Props>::command(IconButton::new(icon), command)
    }

    /// Whether `command`'s marker shows: it is marked and its content is not shown. A mark tells that the command's
    /// content has something to show, so it shows while that content is not shown: a rail's panel that is closed.
    pub fn shows_marker(command: &ResolvedCommand) -> bool {
        command.marked && command.checked != Some(true)
    }
}

impl Component for IconButton<'_> {
    const ROLE: Role = Role::BUTTON;
    const ALT_ROLES: &'static [Role] = &[Role::COMBO_BOX];
    const PARTS: bool = false;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = IconButtonStyle;
    type Output = ButtonOutput;

    fn style(cx: &StyleCx<'_>, _: IconButtonStyleKey) -> IconButtonStyle {
        let face = face::icon_face(cx);
        let marker = MarkerStyle::of(cx);
        let (mut surface, mut glyph) = (face.surface, face.glyph);
        // On a side menu's brand surface the glyph reads in the menu's ink and its bands lie over the brand colour, as a
        // `Button` row's do.
        if cx.env::<super::ControlPlace>() == super::ControlPlace::SideMenu {
            let (ink, fill) = super::button::side_menu_ink(cx);
            glyph.color = ink;
            surface.fill = fill;
        }
        IconButtonStyle {
            surface,
            metrics: face.metrics,
            focus: face::focus(cx),
            glyph,
            marker: marker.side,
            marker_color: marker.color,
            marker_edge: marker.edge,
            mark: cx.tokens.space.gap.tight,
            arrow: cx.tokens.size.icon.glyph_sm,
            mark_color: cx.color(cx.tokens.color.text.faint),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.glyph.side)
    }

    fn sense(&self) -> Sense {
        let sense = if self.focusable { Sense::CLICK | Sense::FOCUS } else { Sense::CLICK };
        if self.press_focus == PressFocus::Keep { sense | Sense::KEEP_FOCUS } else { sense }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let glyph = egui::Rect::from_center_size(cx.rect.center(), Vec2::splat(cx.style.glyph.side));
        let draw = ui.draw();
        draw.icon(glyph, &tinted(&self.icon, cx.resolve(&cx.style.glyph.color)));
        let rtl = cx.flow.is_rtl();
        match self.popup.map(|_| self.popup_mark) {
            Some(PopupMark::ToolGroup) => {
                let (end, bottom) = if rtl {
                    (cx.rect.left() + cx.style.mark, cx.rect.bottom() - cx.style.mark)
                } else {
                    (cx.rect.right() - cx.style.mark, cx.rect.bottom() - cx.style.mark)
                };
                let reach = if rtl { cx.style.mark } else { -cx.style.mark };
                let corner = Pos2::new(end, bottom);
                let triangle = PaintShape::Polygon(vec![corner, Pos2::new(end, bottom - cx.style.mark), Pos2::new(end + reach, bottom)]);
                draw.fill(&triangle, cx.style.mark_color);
            }
            Some(PopupMark::Menu) => {
                // The arrow hangs from the glyph's bottom edge, centred on its end edge, so it never covers the glyph.
                let end = if rtl { glyph.left() } else { glyph.right() };
                let square = egui::Rect::from_center_size(Pos2::new(end, glyph.bottom()), Vec2::splat(cx.style.arrow));
                let arrow = DisclosureGlyph::dropdown_form();
                let points = arrow.path(square, if rtl { Direction::Rtl } else { Direction::Ltr });
                let top = points.iter().map(|point| point.y).fold(f32::INFINITY, f32::min);
                let points = points.into_iter().map(|point| point + Vec2::new(0.0, glyph.bottom() - top)).collect();
                draw.stroke(&PaintShape::Path(points), StrokeWidth::Thin, cx.style.mark_color);
            }
            _ => {}
        }
        if cx.command.as_ref().is_some_and(|command| IconButton::shows_marker(command)) {
            let marker = MarkerStyle { side: cx.style.marker, color: cx.style.marker_color, edge: cx.style.marker_edge };
            let radius = marker.side / 2.0;
            let x = if rtl { glyph.left() + radius } else { glyph.right() - radius };
            paint_marker(&draw, Pos2::new(x, glyph.bottom() - radius), &marker);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if let Some(kind) = self.popup {
            access.has_popup(kind);
        }
        if self.choice.is_some() {
            access.role(Role::COMBO_BOX);
        }
        let shown = || self.choice.is_some().then(|| crate::contract::overlay::opened_from(&cx.ctx, cx.response.id));
        if let Some(open) = self.expanded.or_else(shown) {
            access.expanded(open);
        }
    }

    fn value_text(&self, cx: &Cx<'_, Self>) -> Option<String> {
        self.choice.as_ref().map(|choice| cx.env.resolve(choice).as_str().to_owned())
    }

    fn finish(self, cx: Cx<'_, Self>) -> ButtonOutput {
        ButtonOutput::of(&cx, self.activated(&cx))
    }
}
