//! Relation assertions between rects and lengths: the tests compare layouts to each other and to tokens, never to
//! literal sizes.

use egui::Rect;

/// Slack for float rounding in layout, in points.
const EPS: f32 = 0.5;

/// `a` ends where `b` begins or earlier.
pub fn left_of(a: Rect, b: Rect) -> bool {
    a.max.x <= b.min.x + EPS
}

/// `a` ends above where `b` begins or earlier.
pub fn above(a: Rect, b: Rect) -> bool {
    a.max.y <= b.min.y + EPS
}

/// `lower` starts below where `upper` ends, with a gap between them.
pub fn apart_above(upper: Rect, lower: Rect) -> bool {
    lower.min.y > upper.max.y + EPS
}

/// `inner` ends above the foot of `outer`, not on it.
pub fn off_foot(inner: Rect, outer: Rect) -> bool {
    inner.max.y < outer.max.y - EPS
}

/// `a` and `b` start at the same left edge.
pub fn aligned_left(a: Rect, b: Rect) -> bool {
    (a.min.x - b.min.x).abs() <= EPS
}

/// `a` and `b` stand in one column: the same left edge and the same width.
pub fn same_column(a: Rect, b: Rect) -> bool {
    aligned_left(a, b) && (a.width() - b.width()).abs() <= EPS
}

/// `inner` lies within `outer`.
pub fn inside(inner: Rect, outer: Rect) -> bool {
    inner.min.x >= outer.min.x - EPS && inner.min.y >= outer.min.y - EPS && inner.max.x <= outer.max.x + EPS && inner.max.y <= outer.max.y + EPS
}

/// `a` and `b` are equally tall.
pub fn same_height(a: Rect, b: Rect) -> bool {
    (a.height() - b.height()).abs() <= EPS
}

/// `right_to_left` is `left_to_right` reflected across the vertical middle of `within`, with the same size and height.
pub fn mirrored(left_to_right: Rect, right_to_left: Rect, within: Rect) -> bool {
    let reflected_min = within.min.x + (within.max.x - left_to_right.max.x);
    (right_to_left.min.x - reflected_min).abs() <= EPS
        && (right_to_left.width() - left_to_right.width()).abs() <= EPS
        && (right_to_left.min.y - left_to_right.min.y).abs() <= EPS
        && same_height(left_to_right, right_to_left)
}

/// `grown` is at least `base` along one axis: a measure under a larger text size grows or stays.
pub fn grows_with(base: f32, grown: f32) -> bool {
    grown >= base - EPS
}

/// `measured` equals the token value `token`.
pub fn equals_token(measured: f32, token: f32) -> bool {
    (measured - token).abs() <= EPS
}
