//! `ScrollBar`: the track and thumb of a scroll part, with state.

use super::rule::Rule;
use crate::style::{ScrollBarKind, ScrollBarLook, StateSet};
use craft_behavior::{Axis, ScrollSpan};
use egui::{Color32, CornerRadius, Direction, Rect, Shape, Vec2, pos2};

/// The scroll bar painter.
pub struct ScrollBar;

/// The opacity of an edge fade at the view's edge, once at least the fade's length of content is hidden past it.
const FADE_STRENGTH: f32 = 0.5;

/// How far a scroll view's content runs past each of its edges, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct HiddenContent {
    /// Past the left edge.
    pub left: f32,
    /// Past the right edge.
    pub right: f32,
    /// Past the top edge.
    pub top: f32,
    /// Past the bottom edge.
    pub bottom: f32,
}

impl HiddenContent {
    /// The content of `content` points past a view of `view` points scrolled by `offset`, with x counted from the
    /// reading start (the right edge when `rtl`). An axis whose content fits hides nothing.
    pub fn new(content: Vec2, view: Vec2, offset: Vec2, rtl: bool) -> Self {
        let before = offset.max(Vec2::ZERO);
        let after = (content - view - offset).max(Vec2::ZERO);
        let (left, right) = if rtl { (after.x, before.x) } else { (before.x, after.x) };
        HiddenContent { left, right, top: before.y, bottom: after.y }
    }
}

impl ScrollBar {
    /// The thumb inside `track`, a bar along `axis`, for `span`: its length is the shown share of the track, at least
    /// `min_thumb`, and it sits at the visible range's share of the travel, from the right under right-to-left
    /// on the x axis. This is the thumb a press takes; the painted one is inset by its look's `inset`.
    pub fn thumb(min_thumb: f32, track: Rect, axis: Axis, rtl: bool, span: &ScrollSpan) -> Rect {
        let length = if axis == Axis::X { track.width() } else { track.height() };
        let (at, len) = craft_behavior::scroll::thumb(span, length, min_thumb);
        match axis {
            Axis::X if rtl => Rect::from_min_size(pos2(track.max.x - at - len, track.min.y), Vec2::new(len, track.height())),
            Axis::X => Rect::from_min_size(pos2(track.min.x + at, track.min.y), Vec2::new(len, track.height())),
            _ => Rect::from_min_size(pos2(track.min.x, track.min.y + at), Vec2::new(track.width(), len)),
        }
    }

    /// The fades over `view` at each edge past which content is hidden: `look.fade` long, from `background` at the
    /// edge to clear inward, the more opaque the more is hidden, up to `FADE_STRENGTH` once a fade's length is.
    pub fn fades(look: &ScrollBarLook, view: Rect, hidden: HiddenContent, background: Color32) -> Vec<Shape> {
        let across = |hidden: f32, extent: f32, edge: fn(Rect, f32) -> Rect, direction: Direction| {
            let length = look.fade.min(extent);
            (hidden > 0.0 && length > 0.0).then(|| {
                let at = background.gamma_multiply((hidden / look.fade).clamp(0.0, 1.0) * FADE_STRENGTH);
                Shape::gradient_rect(edge(view, length), direction, [at, Color32::TRANSPARENT])
            })
        };
        let (width, height) = (view.width(), view.height());
        [
            across(hidden.left, width, |v, l| Rect::from_min_max(v.min, pos2(v.min.x + l, v.max.y)), Direction::LeftToRight),
            across(hidden.right, width, |v, l| Rect::from_min_max(pos2(v.max.x - l, v.min.y), v.max), Direction::RightToLeft),
            across(hidden.top, height, |v, l| Rect::from_min_max(v.min, pos2(v.max.x, v.min.y + l)), Direction::TopDown),
            across(hidden.bottom, height, |v, l| Rect::from_min_max(pos2(v.min.x, v.max.y - l), v.max), Direction::BottomUp),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// The shapes of the bar in `track` along `axis` for `span`, in `state`: the track, a docked bar's rule along the
    /// side facing the view (its top for a horizontal bar, its left for a vertical one), and the fully round thumb.
    pub fn shapes(
        look: &ScrollBarLook,
        track: Rect,
        axis: Axis,
        rtl: bool,
        span: &ScrollSpan,
        state: &StateSet,
        pixels_per_point: f32,
    ) -> Vec<Shape> {
        let thumb = Self::thumb(look.min_thumb, track, axis, rtl, span).shrink(look.inset);
        let round = |rect: Rect| CornerRadius::from(rect.width().min(rect.height()) / 2.0);
        let track_round = match look.kind {
            ScrollBarKind::Overlay => round(thumb),
            ScrollBarKind::Docked => CornerRadius::ZERO,
        };
        let mut shapes = vec![Shape::rect_filled(track, track_round, state.resolve(&look.track).egui())];
        if let Some(edge) = &look.edge {
            let (from, to) = if axis == Axis::X { (track.left_top(), track.right_top()) } else { (track.left_top(), track.left_bottom()) };
            shapes.extend(Rule::shapes(edge, from, to, pixels_per_point));
        }
        shapes.push(Shape::rect_filled(thumb, round(thumb), state.resolve(&look.thumb).egui()));
        shapes
    }
}
