//! Basic shapes recovered from a scene path. usvg turns `rect`, `circle` and `ellipse` into
//! paths; an app that edits them as shapes (a rectangle with corner radii, an ellipse) gets them
//! back here from the segments alone.
//!
//! A path is a primitive when it is one closed contour whose points all lie on the outline of
//! an axis-aligned (rounded) rectangle or ellipse and that encloses its area. The check works on
//! the segments as given (a [`Path`](super::document::Path)'s are after its absolute transform), so
//! a rotated or skewed shape is not recognised.

use super::document::Segment;

/// An axis-aligned basic shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    /// A rectangle, with elliptical corners of radii `rx`, `ry` (both 0 for square corners).
    Rect { x: f64, y: f64, width: f64, height: f64, rx: f64, ry: f64 },
    /// An ellipse (a circle when `rx == ry`).
    Ellipse { cx: f64, cy: f64, rx: f64, ry: f64 },
}

/// Most segments a primitive's contour has: usvg writes a rounded rectangle as 4 lines and up to
/// 4 curves per corner.
const MAX_SEGMENTS: usize = 32;
/// Points sampled on each curve, besides its ends: enough that the polygon through them
/// encloses a curved outline's area to well within the area check's 1%.
const SAMPLES: usize = 31;

/// The curve parameters sampled between a curve's ends.
fn samples() -> impl Iterator<Item = f64> {
    (1..=SAMPLES).map(|i| i as f64 / (SAMPLES + 1) as f64)
}

/// The basic shape `segments` draw, if they draw one.
pub fn recognize(segments: &[Segment]) -> Option<Primitive> {
    let (ends, points) = contour(segments)?;
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in &points {
        for axis in 0..2 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    let (width, height) = (max[0] - min[0], max[1] - min[1]);
    if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let tol = 1e-3 * width.max(height);
    // The straight part of the top edge starts `rx` in; of the left edge, `ry` down.
    let rx = ends.iter().filter(|p| (p[1] - min[1]).abs() <= tol).map(|p| p[0] - min[0]).fold(f64::INFINITY, f64::min);
    let ry = ends.iter().filter(|p| (p[0] - min[0]).abs() <= tol).map(|p| p[1] - min[1]).fold(f64::INFINITY, f64::min);
    let rx = snap(rx, width / 2.0, tol)?;
    let ry = snap(ry, height / 2.0, tol)?;
    if (rx == 0.0) != (ry == 0.0) {
        return None;
    }
    let inner = ([min[0] + rx, min[1] + ry], [max[0] - rx, max[1] - ry]);
    let on_outline = |p: &[f64; 2]| {
        let q = [p[0].clamp(inner.0[0], inner.1[0]), p[1].clamp(inner.0[1], inner.1[1])];
        let d = [p[0] - q[0], p[1] - q[1]];
        if rx == 0.0 {
            // Square corners: on an edge of the box.
            (p[0] - min[0]).abs() <= tol || (p[0] - max[0]).abs() <= tol || (p[1] - min[1]).abs() <= tol || (p[1] - max[1]).abs() <= tol
        } else if d == [0.0, 0.0] {
            false
        } else {
            let e = (d[0] / rx).hypot(d[1] / ry);
            (e - 1.0).abs() * rx.min(ry) <= tol || (d[0] == 0.0 && (d[1].abs() - ry).abs() <= tol) || (d[1] == 0.0 && (d[0].abs() - rx).abs() <= tol)
        }
    };
    if !points.iter().all(on_outline) {
        return None;
    }
    // Every point is on the outline; it also has to go all the way round once.
    let expected = width * height - (4.0 - std::f64::consts::PI) * rx * ry;
    if (area(&points).abs() - expected).abs() > 0.01 * expected {
        return None;
    }
    if (rx - width / 2.0).abs() <= tol && (ry - height / 2.0).abs() <= tol {
        return Some(Primitive::Ellipse { cx: min[0] + width / 2.0, cy: min[1] + height / 2.0, rx: width / 2.0, ry: height / 2.0 });
    }
    Some(Primitive::Rect { x: min[0], y: min[1], width, height, rx, ry })
}

/// `value` within `tol` of 0 or `half` is snapped there; past `half` it isn't a corner radius.
fn snap(value: f64, half: f64, tol: f64) -> Option<f64> {
    if !value.is_finite() || value < -tol || value > half + tol {
        return None;
    }
    Some(if value <= tol {
        0.0
    } else if (value - half).abs() <= tol {
        half
    } else {
        value
    })
}

/// Points of a contour.
type Points = Vec<[f64; 2]>;

/// The segment ends of one closed contour, and all its points (ends and curve samples), or
/// `None` for anything else: several contours, an open one, or too many segments.
fn contour(segments: &[Segment]) -> Option<(Points, Points)> {
    if segments.len() > MAX_SEGMENTS + 2 {
        return None;
    }
    let (first, rest) = segments.split_first()?;
    let Segment::MoveTo(start) = *first else { return None };
    let mut points = vec![start];
    let mut ends = vec![start];
    let mut at = start;
    let mut closed = false;
    for segment in rest {
        if closed {
            return None;
        }
        match *segment {
            Segment::MoveTo(_) => return None,
            Segment::LineTo(p) => {
                points.push(p);
                ends.push(p);
                at = p;
            }
            Segment::QuadTo(c, p) => {
                for t in samples() {
                    let u = 1.0 - t;
                    points.push([u * u * at[0] + 2.0 * u * t * c[0] + t * t * p[0], u * u * at[1] + 2.0 * u * t * c[1] + t * t * p[1]]);
                }
                points.push(p);
                ends.push(p);
                at = p;
            }
            Segment::CubicTo(c1, c2, p) => {
                for t in samples() {
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    points.push([a * at[0] + b * c1[0] + c * c2[0] + d * p[0], a * at[1] + b * c1[1] + c * c2[1] + d * p[1]]);
                }
                points.push(p);
                ends.push(p);
                at = p;
            }
            Segment::Close => closed = true,
        }
    }
    let ends_at_start = (at[0] - start[0]).abs() < 1e-9 && (at[1] - start[1]).abs() < 1e-9;
    (closed || ends_at_start).then_some((ends, points))
}

/// The signed area of the polygon through `points`.
fn area(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

#[cfg(test)]
#[path = "primitive_tests.rs"]
mod tests;
