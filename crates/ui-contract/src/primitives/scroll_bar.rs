//! `ScrollBar`: the track and thumb of a scroll part, with state.

use super::rule::RectSide;
use crate::style::{ScrollBarLook, StateSet};
use craft_behavior::{Axis, ScrollSpan};
use egui::{Color32, CornerRadius, Direction, Rect, Shape, Vec2, pos2};

/// The scroll bar painter.
pub struct ScrollBar;

/// The opacity of an edge fade at the view's edge, once at least the fade's length of content is hidden past it.
const FADE_STRENGTH: f32 = 0.5;

/// The length of an edge fade, in points, in every density.
const FADE_LENGTH: f32 = 20.0;

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

/// A scroll strip's arrow as shares of the strip's height: its width and its height.
const STRIP_ARROW: (f32, f32) = (0.75, 0.28125);

impl ScrollBar {
    /// The filled arrow of a scroll strip, centred in `strip`: pointing up for the strip before the view (`back`),
    /// down for the one after it.
    pub fn strip_arrow(strip: Rect, back: bool, color: Color32) -> Shape {
        let (half, depth) = (strip.height() * STRIP_ARROW.0 / 2.0, strip.height() * STRIP_ARROW.1 / 2.0);
        let c = strip.center();
        let (tip, base) = if back { (c.y - depth, c.y + depth) } else { (c.y + depth, c.y - depth) };
        Shape::convex_polygon(vec![pos2(c.x - half, base), pos2(c.x + half, base), pos2(c.x, tip)], color, egui::Stroke::NONE)
    }

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

    /// The fades over `view` at each edge past which content is hidden: `FADE_LENGTH` long, from `background` at the
    /// edge to clear inward, the more opaque the more is hidden, up to `FADE_STRENGTH` once a fade's length is.
    pub fn fades(view: Rect, hidden: HiddenContent, background: Color32) -> Vec<Shape> {
        let across = |hidden: f32, extent: f32, edge: fn(Rect, f32) -> Rect, direction: Direction| {
            let length = FADE_LENGTH.min(extent);
            (hidden > 0.0 && length > 0.0).then(|| {
                let at = background.gamma_multiply((hidden / FADE_LENGTH).clamp(0.0, 1.0) * FADE_STRENGTH);
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

    /// The shapes of the bar in `track` along `axis` for `span`, in `state` (the bar's: hovered while the pointer is
    /// anywhere on it): the track and the fully round thumb, `look.thumb_width` thick along the track's outer edge (its
    /// bottom, or its end side in the reading direction). No shapes when the track has no room: a view shorter or
    /// narrower than its padding gives its bar a track of no length, or one whose ends cross.
    pub fn shapes(look: &ScrollBarLook, track: Rect, axis: Axis, rtl: bool, span: &ScrollSpan, state: &StateSet) -> Vec<Shape> {
        if !track.is_positive() {
            return Vec::new();
        }
        let full = Self::thumb(look.min_thumb, track, axis, rtl, span);
        let thumb = Self::along_edge(full, state.resolve(&look.thumb_width), axis, rtl);
        let round = |rect: Rect| CornerRadius::from(rect.width().min(rect.height()) / 2.0);
        vec![
            Shape::rect_filled(track, round(full), state.resolve(&look.track).egui()),
            Shape::rect_filled(thumb, round(thumb), state.resolve(&look.thumb).egui()),
        ]
    }

    /// The thumb `width` thick along the outer edge of the full thumb `full`, at most `full`.
    fn along_edge(full: Rect, width: f32, axis: Axis, rtl: bool) -> Rect {
        let side = if axis == Axis::X { RectSide::Bottom } else { RectSide::end(rtl) };
        let width = width.min(if axis == Axis::X { full.height() } else { full.width() });
        match side {
            RectSide::Bottom => Rect::from_min_max(pos2(full.min.x, full.max.y - width), full.max),
            RectSide::Left => Rect::from_min_max(full.min, pos2(full.min.x + width, full.max.y)),
            _ => Rect::from_min_max(pos2(full.max.x - width, full.min.y), full.max),
        }
    }
}
