//! `Surface`: the pure shapes of a surface kind.

use crate::style::{Corners, SurfaceEdge, SurfaceKind, SurfaceStyle, UiTokens, surface_style};
use craft_i18n::Direction;
use egui::emath::GuiRounding as _;
use egui::epaint::{Mesh, RectShape};
use egui::layers::ShapeIdx;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, StrokeKind, Ui, Vec2, vec2};

/// The surface painter: shadow, fill, then edge.
pub struct Surface;

impl Surface {
    /// The shapes of `style` over `rect`, snapped to the pixel grid: shadow, fill, edge. `fill` replaces the
    /// kind's fill (the state fill), so washes never stack. A bevel is whole physical pixels wide.
    pub fn shapes(
        style: &SurfaceStyle,
        fill: Option<Color32>,
        rect: Rect,
        corners: Corners,
        direction: Direction,
        pixels_per_point: f32,
    ) -> Vec<Shape> {
        let rect = rect.round_to_pixels(pixels_per_point);
        let radius = corners.apply(style.radius, direction);
        let mut out = Vec::with_capacity(3);
        if let Some(shadow) = style.shadow {
            out.push(Shape::Rect(shadow.as_shape(rect, radius)));
        }
        let fill = fill.unwrap_or(style.fill);
        if fill != Color32::TRANSPARENT {
            out.push(Shape::Rect(RectShape::filled(rect, radius, fill)));
        }
        match style.edge {
            SurfaceEdge::Border(stroke) => out.push(Shape::Rect(RectShape::stroke(rect, radius, stroke, StrokeKind::Inside))),
            SurfaceEdge::Bevel { top_left, bottom_right, width } => {
                let width = (width * pixels_per_point).round().max(1.0) / pixels_per_point;
                out.push(Shape::mesh(bevel(rect, radius, width, top_left, bottom_right)));
            }
            _ => {}
        }
        out
    }
}

/// The rounded frames surfaces keep to, at most one per layer: a floating window's body ([`frame_surfaces`]).
#[derive(Default)]
pub(crate) struct Frames(Vec<(crate::runtime::egui::areas::Layer, Rect, CornerRadius)>);

/// The most layers that keep a frame at once: the frame a closed window left gives way to newer ones.
const MOST_FRAMES: usize = 32;

/// How far a surface's edge may lie from the frame's and still meet it, in points: the rounding to the pixel grid.
const FRAME_SLACK: f32 = 0.75;

/// Rounds the surfaces painted from here on in `ui`'s layer that meet a corner of `rect`: such a surface takes
/// `radius` at that corner where its own is smaller. A floating window sets its body's rect and its own lower corners,
/// so the square surfaces of the panels and rails inside it (a dock column, a tool rail's chrome) keep to its rounded
/// corners instead of covering them.
pub fn frame_surfaces(ui: &Ui, rect: Rect, radius: CornerRadius) {
    // A measuring run paints nothing.
    if crate::runtime::measure::active(ui.ctx()) {
        return;
    }
    let layer = ui.layer_id();
    crate::runtime::pass_state::with::<Frames, _>(ui.ctx(), |frames| {
        frames.0.retain(|(at, ..)| *at != layer);
        if frames.0.len() >= MOST_FRAMES {
            frames.0.remove(0);
        }
        frames.0.push((layer, rect, radius));
    });
}

/// Fits `shapes`, painted in `layer`, to the layer's frame ([`frame_surfaces`]); a layer without one keeps them.
pub(crate) fn fit_to_frame(ctx: &egui::Context, layer: crate::runtime::egui::areas::Layer, shapes: &mut [Shape]) {
    if crate::runtime::measure::active(ctx) {
        return;
    }
    let frame = crate::runtime::pass_state::with::<Frames, _>(ctx, |frames| {
        frames.0.iter().find(|(at, ..)| *at == layer).map(|(_, rect, radius)| (*rect, *radius))
    });
    let Some((rect, radius)) = frame else { return };
    for shape in shapes {
        fit_shape(shape, rect, radius);
    }
}

/// Rounds each corner of a rect `shape` that meets the same corner of `frame` to at least the frame's radius there.
fn fit_shape(shape: &mut Shape, frame: Rect, radius: CornerRadius) {
    match shape {
        Shape::Rect(rect) => {
            let near = |a: f32, b: f32| (a - b).abs() <= FRAME_SLACK;
            let at = rect.rect;
            let (left, right) = (near(at.left(), frame.left()), near(at.right(), frame.right()));
            let (top, bottom) = (near(at.top(), frame.top()), near(at.bottom(), frame.bottom()));
            let corners = &mut rect.corner_radius;
            if top && left {
                corners.nw = corners.nw.max(radius.nw);
            }
            if top && right {
                corners.ne = corners.ne.max(radius.ne);
            }
            if bottom && left {
                corners.sw = corners.sw.max(radius.sw);
            }
            if bottom && right {
                corners.se = corners.se.max(radius.se);
            }
        }
        Shape::Vec(shapes) => {
            for shape in shapes {
                fit_shape(shape, frame, radius);
            }
        }
        _ => {}
    }
}

/// The surface under content that runs before its rect is known: [`reserve`](SurfaceSlot::reserve) takes its place in
/// the paint order, [`paint`](SurfaceSlot::paint) fills it at the rect the content took.
pub(crate) struct SurfaceSlot {
    painter: Painter,
    slot: ShapeIdx,
}

impl SurfaceSlot {
    /// The slot under what `ui` paints next. A layer's surface casts its shadow past the layer's own clip, so on a
    /// `layer` the slot paints under the whole window.
    pub(crate) fn reserve(ui: &Ui, layer: bool) -> Self {
        let mut painter = ui.painter().clone();
        if layer {
            painter.set_clip_rect(ui.ctx().content_rect());
        }
        let slot = painter.add(Shape::Noop);
        SurfaceSlot { painter, slot }
    }

    /// Paints the surface `kind` over `rect`, kept to its layer's frame ([`frame_surfaces`]).
    pub(crate) fn paint(&self, tokens: &UiTokens, kind: SurfaceKind, rect: Rect, direction: Direction) {
        let mut shapes = Surface::shapes(&surface_style(tokens, kind), None, rect, Corners::All, direction, self.painter.pixels_per_point());
        fit_to_frame(self.painter.ctx(), self.painter.layer_id(), &mut shapes);
        self.painter.set(self.slot, Shape::Vec(shapes));
    }
}

/// The bevel inside `rect`: two strips between the outline and the outline inset by `width`, split at 45 degrees on
/// the top-right and bottom-left corners. Both outlines are sampled at the same angles, so square corners get
/// mitred joins and round ones follow the radius.
fn bevel(rect: Rect, radius: CornerRadius, width: f32, top_left: Color32, bottom_right: Color32) -> Mesh {
    let width = width.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let inner = rect.shrink(width);
    let max_r = rect.width().min(rect.height()) / 2.0;
    let corner = |point: Pos2, inner_point: Pos2, into: Vec2, radius: u8, start: f32| {
        let outer_r = f32::from(radius).min(max_r);
        let inner_r = (outer_r - width).max(0.0);
        (point + into * outer_r, outer_r, inner_point + into * inner_r, inner_r, start)
    };
    let corners = [
        corner(rect.left_top(), inner.left_top(), Vec2::RIGHT + Vec2::DOWN, radius.nw, 180.0),
        corner(rect.right_top(), inner.right_top(), Vec2::LEFT + Vec2::DOWN, radius.ne, 270.0),
        corner(rect.right_bottom(), inner.right_bottom(), Vec2::LEFT + Vec2::UP, radius.se, 0.0),
        corner(rect.left_bottom(), inner.left_bottom(), Vec2::RIGHT + Vec2::UP, radius.sw, 90.0),
    ];
    let at = |centre: Pos2, r: f32, degrees: f32| centre + r * vec2(degrees.to_radians().cos(), degrees.to_radians().sin());
    let arc = |i: usize, from: f32, to: f32, out: &mut Vec<(Pos2, Pos2)>| {
        let (oc, or, ic, ir, start) = corners[i];
        let steps = ((or * (to - from) / 90.0 / 2.0).ceil() as usize).clamp(1, 16);
        for s in 0..=steps {
            let a = start + from + (to - from) * s as f32 / steps as f32;
            out.push((at(oc, or, a), at(ic, ir, a)));
        }
    };
    let mut lit = Vec::new();
    arc(3, 45.0, 90.0, &mut lit);
    arc(0, 0.0, 90.0, &mut lit);
    arc(1, 0.0, 45.0, &mut lit);
    let mut shaded = Vec::new();
    arc(1, 45.0, 90.0, &mut shaded);
    arc(2, 0.0, 90.0, &mut shaded);
    arc(3, 0.0, 45.0, &mut shaded);
    let mut mesh = Mesh::default();
    for (strip, color) in [(lit, top_left), (shaded, bottom_right)] {
        let base = mesh.vertices.len() as u32;
        for (outer, inner) in &strip {
            mesh.colored_vertex(*outer, color);
            mesh.colored_vertex(*inner, color);
        }
        for i in 0..strip.len().saturating_sub(1) as u32 {
            let (o0, i0, o1, i1) = (base + 2 * i, base + 2 * i + 1, base + 2 * i + 2, base + 2 * i + 3);
            mesh.add_triangle(o0, i0, o1);
            mesh.add_triangle(i0, i1, o1);
        }
    }
    mesh
}
