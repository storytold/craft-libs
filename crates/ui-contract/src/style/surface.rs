//! What each surface kind looks like in a token set: fill, corners, edge and shadow.

use super::look::SurfaceKind;
use super::tokens::UiTokens;
use craft_theme::{Edge, Elevation, Rgba, Stroke as StrokeStep};
use egui::{Color32, CornerRadius, Shadow, Stroke};

/// A surface's edge, resolved from the theme's edge style.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum SurfaceEdge {
    /// Nothing around the fill.
    None,
    /// An outline inside the rect.
    Border(Stroke),
    /// A two-tone bevel: `top_left` on the top and left sides, `bottom_right` on the bottom and right. Lighting
    /// does not mirror under right-to-left.
    #[non_exhaustive]
    Bevel {
        /// The top and left sides.
        top_left: Color32,
        /// The bottom and right sides.
        bottom_right: Color32,
        /// The width in points.
        width: f32,
    },
}

/// Everything a surface of one kind draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceStyle {
    /// The fill; a state fill replaces it.
    pub fill: Color32,
    /// The corner radii before `Corners` applies.
    pub radius: CornerRadius,
    /// The border or bevel.
    pub edge: SurfaceEdge,
    /// Drawn under the fill.
    pub shadow: Option<Shadow>,
}

impl SurfaceStyle {
    /// This look while disabled: opaque, the fill moved toward `panel` (the disabled fill) by `Opacity::DISABLED`, the edge `edge`, and no
    /// shadow.
    pub fn disabled(&self, panel: Color32, edge: Color32) -> SurfaceStyle {
        let edge = match self.edge {
            SurfaceEdge::Border(stroke) => SurfaceEdge::Border(Stroke::new(stroke.width, edge)),
            SurfaceEdge::Bevel { width, .. } => SurfaceEdge::Bevel { top_left: edge, bottom_right: edge, width },
            other => other,
        };
        SurfaceStyle { fill: craft_theme::Mix::mix(self.fill, panel, craft_theme::Opacity::DISABLED), edge, shadow: None, ..*self }
    }
}

impl craft_theme::Mix for SurfaceStyle {
    /// The fill blends; radius, edge and shadow switch to `to` from the halfway point.
    fn mix(self, to: SurfaceStyle, t: f32) -> SurfaceStyle {
        if t >= 0.5 {
            SurfaceStyle { fill: craft_theme::Mix::mix(self.fill, to.fill, t), ..to }
        } else {
            SurfaceStyle { fill: craft_theme::Mix::mix(self.fill, to.fill, t), ..self }
        }
    }
}

fn color(c: Rgba) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn shadow(e: Elevation) -> Option<Shadow> {
    let [x, y] = e.offset;
    let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    let shadow = Shadow {
        offset: [x.round().clamp(-128.0, 127.0) as i8, y.round().clamp(-128.0, 127.0) as i8],
        blur: byte(e.blur),
        spread: byte(e.spread),
        color: Color32::from_black_alpha(byte(e.alpha * 255.0)),
    };
    (shadow.color.a() > 0).then_some(shadow)
}

/// The look of `kind` in `tokens`.
pub(crate) fn surface_style(tokens: &UiTokens, kind: SurfaceKind) -> SurfaceStyle {
    use SurfaceKind as K;
    let c = &tokens.color;
    let r = &tokens.shape.radius;
    let fill = match kind {
        K::Background => c.surface.background,
        K::Canvas => c.surface.canvas,
        K::Chrome => c.surface.chrome,
        K::Panel | K::Popup | K::Dialog | K::Floating => c.surface.panel,
        K::Raised => c.surface.raised,
        K::Sunken | K::Well => c.surface.sunken,
        K::Pressed => c.state.pressed,
        K::Knob => c.surface.knob,
        K::Scrim => c.surface.scrim,
        K::Frame => Rgba::TRANSPARENT,
    };
    let radius = match kind {
        K::Background | K::Canvas | K::Chrome | K::Scrim => 0.0,
        K::Panel | K::Frame => r.panel,
        K::Raised | K::Pressed | K::Sunken | K::Well | K::Knob => r.control,
        K::Popup | K::Dialog => r.surface,
        K::Floating => r.window,
    };
    let shadow = match kind {
        K::Popup | K::Dialog => shadow(tokens.elevation.popup),
        K::Floating => shadow(tokens.elevation.floating),
        K::Knob => shadow(tokens.elevation.knob),
        _ => None,
    };
    let width = StrokeStep::W1.width(1.0);
    let stroke = |c: Rgba| SurfaceEdge::Border(Stroke::new(width, color(c)));
    let edge = match (tokens.shape.edge, kind) {
        (_, K::Background | K::Canvas | K::Chrome | K::Panel | K::Scrim) => SurfaceEdge::None,
        (_, K::Popup | K::Dialog | K::Floating | K::Frame) => stroke(c.line.border),
        (Edge::Flat, K::Raised) => stroke(c.line.border),
        (Edge::Flat, K::Sunken | K::Pressed | K::Well) => stroke(c.line.control_edge),
        (Edge::Flat, K::Knob) => stroke(c.line.knob_edge),
        (Edge::Bevel { light, dark, width }, K::Raised | K::Knob) => SurfaceEdge::Bevel { top_left: color(light), bottom_right: color(dark), width },
        (Edge::Bevel { light, dark, width }, K::Sunken | K::Pressed | K::Well) => {
            SurfaceEdge::Bevel { top_left: color(dark), bottom_right: color(light), width }
        }
    };
    SurfaceStyle { fill: color(fill), radius: UiTokens::corner(radius), edge, shadow }
}
