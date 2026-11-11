//! `ScrollBar`: the track and thumb of a scroll part, with state.

use crate::style::{ScrollBarLook, StateSet};
use egui::{Color32, Direction, Rect, Shape, Vec2, pos2, vec2};

/// Which way a bar runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BarAxis {
    /// Along x.
    Horizontal,
    /// Along y.
    Vertical,
}

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
    /// The thumb inside `track` for `viewport` points of `content` scrolled by `offset`: its length is the shown
    /// share of the track, at least `look.min_thumb`, and it sits at the offset's share of the travel.
    pub fn thumb(look: &ScrollBarLook, axis: BarAxis, track: Rect, content: f32, viewport: f32, offset: f32) -> Rect {
        let length = match axis {
            BarAxis::Horizontal => track.width(),
            BarAxis::Vertical => track.height(),
        };
        let share = if content > 0.0 { (viewport / content).clamp(0.0, 1.0) } else { 1.0 };
        let thumb = (length * share).max(look.min_thumb.min(length));
        let travel = (length - thumb).max(0.0);
        let range = (content - viewport).max(f32::EPSILON);
        let at = travel * (offset / range).clamp(0.0, 1.0);
        match axis {
            BarAxis::Horizontal => Rect::from_min_size(pos2(track.min.x + at, track.min.y), vec2(thumb, track.height())),
            BarAxis::Vertical => Rect::from_min_size(pos2(track.min.x, track.min.y + at), vec2(track.width(), thumb)),
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

    /// The shapes of the track and the thumb in `state`.
    pub fn shapes(look: &ScrollBarLook, track: Rect, thumb: Rect, state: &StateSet) -> Vec<Shape> {
        let radius = thumb.width().min(thumb.height()) / 2.0;
        vec![
            Shape::rect_filled(track, radius, state.resolve(&look.track).egui()),
            Shape::rect_filled(thumb, radius, state.resolve(&look.thumb).egui()),
        ]
    }
}
