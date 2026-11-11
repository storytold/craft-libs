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
    /// The width in points.
    pub width: f32,
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
    let thin = tokens.stroke.thin;
    match kind {
        RuleKind::Divider => RuleLook { color: color(c.line.divider), width: thin },
        RuleKind::Edge => RuleLook { color: color(c.line.border), width: thin },
        RuleKind::Grid => RuleLook { color: color(c.overlay.grid), width: thin },
        RuleKind::DropMark => RuleLook { color: color(c.accent.solid), width: tokens.stroke.medium },
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

/// A scroll bar's track and thumb.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScrollBarLook {
    /// The track fill per state.
    pub track: Stateful<StyleColor>,
    /// The thumb fill per state.
    pub thumb: Stateful<StyleColor>,
    /// The bar's thickness.
    pub thickness: f32,
    /// The shortest the thumb gets along the bar.
    pub min_thumb: f32,
    /// The length of the fade at an edge past which content is hidden.
    pub fade: f32,
}

pub(crate) fn scroll_bar_look(tokens: &UiTokens) -> ScrollBarLook {
    let c = &tokens.color;
    let clear = style_color(Rgba::TRANSPARENT);
    let track = Stateful::new(clear).off(craft_theme::States::new(clear, style_color(c.state.hover), style_color(c.state.hover), clear));
    let thumb = Stateful::new(style_color(c.text.faint)).off(craft_theme::States::new(
        style_color(c.text.faint),
        style_color(c.text.muted),
        style_color(c.text.normal),
        style_color(c.state.disabled_fill),
    ));
    ScrollBarLook { track, thumb, thickness: tokens.size.handle.sm, min_thumb: tokens.size.handle.md * 2.0, fade: tokens.size.scroll_fade }
}

/// The reset dot's colour and diameter.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResetMarkLook {
    /// The dot colour.
    pub color: StyleColor,
    /// The dot diameter.
    pub diameter: f32,
}

pub(crate) fn reset_mark_look(tokens: &UiTokens) -> ResetMarkLook {
    ResetMarkLook { color: style_color(tokens.color.accent.solid), diameter: tokens.size.handle.sm / 2.0 }
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
