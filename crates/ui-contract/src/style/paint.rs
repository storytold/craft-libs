//! What the small painters (rules, handles, scroll bars, the reset mark) draw, read from tokens.

use super::tokens::UiTokens;
use craft_theme::{Rgba, Stateful};
use egui::Color32;

use super::look::StyleColor;

#[doc(hidden)]
pub fn color(c: Rgba) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn style_color(c: Rgba) -> StyleColor {
    StyleColor(color(c))
}

/// The line a rule draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RuleLook {
    /// The colour.
    pub color: Color32,
    /// The width.
    pub width: RuleWidth,
}

/// The width of a rule.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum RuleWidth {
    /// One physical pixel at every scale.
    #[default]
    Hairline,
    /// A width in points, rounded to whole physical pixels and never thinner than one.
    Points(f32),
}

impl RuleWidth {
    /// The width in whole physical pixels on a display of `pixels_per_point`.
    pub fn pixels(self, pixels_per_point: f32) -> f32 {
        match self {
            RuleWidth::Hairline => 1.0,
            RuleWidth::Points(width) => (width * pixels_per_point).round().max(1.0),
        }
    }

    /// The width in points on a display of `pixels_per_point`.
    pub fn points(self, pixels_per_point: f32) -> f32 {
        self.pixels(pixels_per_point) / pixels_per_point.max(f32::EPSILON)
    }
}

/// Which rule a look is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuleKind {
    /// A separator between neighbours.
    Divider,
    /// The edge of a surface or region.
    Edge,
    /// A grid line.
    Grid,
    /// The insertion mark of a drop.
    DropMark,
}

pub(crate) fn rule_look(tokens: &UiTokens, kind: RuleKind) -> RuleLook {
    let c = &tokens.color;
    let hairline = RuleWidth::Hairline;
    match kind {
        RuleKind::Divider => RuleLook { color: color(c.line.divider), width: hairline },
        RuleKind::Edge => RuleLook { color: color(c.line.border), width: hairline },
        RuleKind::Grid => RuleLook { color: color(c.overlay.grid), width: hairline },
        RuleKind::DropMark => RuleLook { color: color(c.accent.solid), width: RuleWidth::Points(tokens.stroke.thick) },
    }
}

/// A canvas handle's fills and stroke.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HandleLook {
    /// The side or diameter.
    pub side: f32,
    /// The fill per state.
    pub fill: Stateful<StyleColor>,
    /// The stroke colour.
    pub stroke: StyleColor,
    /// The stroke width.
    pub stroke_width: f32,
}

pub(crate) fn handle_look(tokens: &UiTokens) -> HandleLook {
    let c = &tokens.color;
    let fill = Stateful::new(style_color(c.overlay.handle_fill)).off(craft_theme::States::new(
        style_color(c.overlay.handle_fill),
        style_color(c.accent.hover),
        style_color(c.accent.pressed),
        style_color(c.state.disabled_fill),
    ));
    HandleLook { side: tokens.size.handle.md, fill, stroke: style_color(c.overlay.handle_stroke), stroke_width: tokens.stroke.thin }
}

/// Where a scroll bar sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScrollBarKind {
    /// Over the content at its end edge: a clear track that tints under the pointer, a thumb as thick as the track.
    Overlay,
    /// Beside the view along its bottom and right, as a document window's bars: a solid chrome track with an edge rule
    /// on the side facing the view, and a thumb inset from the track's edges.
    Docked,
}

/// A scroll bar's track and thumb.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScrollBarLook {
    /// Where the bar sits.
    pub kind: ScrollBarKind,
    /// The track fill per state.
    pub track: Stateful<StyleColor>,
    /// The thumb fill per state.
    pub thumb: Stateful<StyleColor>,
    /// The bar's thickness.
    pub thickness: f32,
    /// The shortest the thumb gets along the bar.
    pub min_thumb: f32,
    /// Between the track's edges and the painted thumb.
    pub inset: f32,
    /// The rule along the side of the track that faces the view.
    pub edge: Option<RuleLook>,
    /// The length of the fade at an edge past which content is hidden.
    pub fade: f32,
}

/// The shortest a scroll bar's thumb gets along the bar, whatever the bar's kind.
pub(crate) fn min_thumb(tokens: &UiTokens) -> f32 {
    tokens.size.handle.md * 2.0
}

pub(crate) fn scroll_bar_look(tokens: &UiTokens, kind: ScrollBarKind) -> ScrollBarLook {
    let c = &tokens.color;
    let clear = style_color(Rgba::TRANSPARENT);
    let thumb = Stateful::new(style_color(c.text.faint)).off(craft_theme::States::new(
        style_color(c.text.faint),
        style_color(c.text.muted),
        style_color(c.text.normal),
        style_color(c.state.disabled_fill),
    ));
    let (track, inset, edge) = match kind {
        ScrollBarKind::Overlay => {
            let hover = style_color(c.state.hover);
            (Stateful::new(clear).off(craft_theme::States::new(clear, hover, hover, clear)), 0.0, None)
        }
        ScrollBarKind::Docked => (Stateful::new(style_color(c.surface.chrome)), tokens.space.gap.tight, Some(rule_look(tokens, RuleKind::Edge))),
    };
    ScrollBarLook { kind, track, thumb, thickness: tokens.size.handle.sm, min_thumb: min_thumb(tokens), inset, edge, fade: tokens.size.scroll_fade }
}

/// The reset dot's colours and diameter.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResetMarkLook {
    /// The dot colour, while the value differs from its reset value.
    pub color: StyleColor,
    /// The ring colour, while the value is at its reset value: the slot shows before anything changes.
    pub ring: StyleColor,
    /// The ring's line width.
    pub ring_width: f32,
    /// The dot diameter.
    pub diameter: f32,
}

/// The state layer: the theme's hover and press fills, clear at rest and while disabled. An opaque state fill takes
/// the place of the rest fill of a part without a surface of its own.
pub(crate) fn state_layer_look(tokens: &UiTokens) -> Stateful<StyleColor> {
    let (clear, state) = (style_color(Rgba::TRANSPARENT), &tokens.color.state);
    Stateful::new(clear).off(craft_theme::States::new(clear, style_color(state.hover), style_color(state.pressed), clear))
}

pub(crate) fn reset_mark_look(tokens: &UiTokens) -> ResetMarkLook {
    ResetMarkLook {
        color: style_color(tokens.color.accent.solid),
        ring: style_color(tokens.color.text.disabled),
        ring_width: tokens.stroke.thin,
        diameter: tokens.size.handle.sm / 2.0,
    }
}

/// The colour of `ink` in `tokens`.
#[doc(hidden)]
pub fn ink_color(tokens: &UiTokens, ink: super::key::Ink) -> Color32 {
    let text = &tokens.color.text;
    color(match ink {
        super::key::Ink::OnPanel => text.normal,
        super::key::Ink::OnDark => text.on_strong,
        super::key::Ink::Accent => tokens.color.accent.text,
    })
}

/// The colour of the scrim behind a modal layer.
pub(crate) fn scrim_color(tokens: &UiTokens) -> Color32 {
    color(tokens.color.surface.scrim)
}
