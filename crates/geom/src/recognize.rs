//! Shape recognition for the Shaper tool: a rough freehand stroke becomes a clean shape.
//!
//! The stroke is classified by how closed it is, how many corners survive a coarse polyline
//! simplification independent of where drawing began, its edge/angle fit and how well it fills
//! its bounding box. Samples are spaced by distance so pointer speed does not bias the fit.
//! A zig-zag stroke with several reversals is a scribble (the Shaper deletes what it covers).

use crate::{Affine, Point, Rect, Vec2};

/// What the Shaper recognised.
#[derive(Clone, Debug, PartialEq)]
pub enum Recognized {
    Line {
        a: Point,
        b: Point,
    },
    /// Rectangle at a 45° step (squares snap when the sides are within 10%).
    Rectangle {
        rect: Rect,
        rotation: f64,
    },
    /// Ellipse at a 45° step (circles snap when the axes are within 10%).
    Ellipse {
        rect: Rect,
        rotation: f64,
    },
    /// Regular polygon (triangle, pentagon, hexagon…): centre, radius, side count and the rotation
    /// (degrees) of the first vertex from straight up.
    Polygon {
        center: Point,
        radius: f64,
        sides: u32,
        rotation: f64,
    },
    /// A zig-zag over existing art: delete what it covers.
    Scribble(Rect),
}

fn bbox(pts: &[Point]) -> Rect {
    pts.iter().fold(Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN), |r, p| r.union_pt(*p))
}

fn path_len(pts: &[Point]) -> f64 {
    pts.windows(2).map(|w| w[0].distance(w[1])).sum()
}

fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let l2 = ab.hypot2();
    if l2 < 1e-12 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Douglas–Peucker simplification with bounded stack use, even for noisy strokes.
fn simplify(pts: &[Point], tol: f64) -> Vec<Point> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    if let Some(first) = keep.first_mut() {
        *first = true;
    }
    if let Some(last) = keep.last_mut() {
        *last = true;
    }
    let mut pending = vec![(0, pts.len() - 1)];
    while let Some((start, end)) = pending.pop() {
        let Some(window) = pts.get(start..=end) else { continue };
        let (Some(&a), Some(&b)) = (window.first(), window.last()) else { continue };
        let (offset, deviation) = window
            .iter()
            .enumerate()
            .skip(1)
            .take(window.len().saturating_sub(2))
            .map(|(i, p)| (i, dist_to_segment(*p, a, b)))
            .fold((0, 0.0), |best, candidate| if candidate.1 > best.1 { candidate } else { best });
        if deviation > tol {
            let split = start + offset;
            if let Some(k) = keep.get_mut(split) {
                *k = true;
            }
            if split > start + 1 {
                pending.push((start, split));
            }
            if end > split + 1 {
                pending.push((split, end));
            }
        }
    }
    pts.iter().zip(keep).filter_map(|(p, keep)| keep.then_some(*p)).collect()
}

/// Sample by distance, so pointer speed/density cannot bias the shape fit. Bounds and length
/// still come from the complete stroke, but simplification works on at most 256 samples.
fn resample(pts: &[Point], length: f64) -> Vec<Point> {
    const SAMPLES: usize = 256;
    let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { return Vec::new() };
    let mut out = vec![first];
    let mut walked = 0.0;
    for (&a, &b) in pts.iter().zip(pts.iter().skip(1)) {
        let segment = a.distance(b);
        if segment <= 0.0 {
            continue;
        }
        while out.len() < SAMPLES - 1 {
            let target = length * (out.len() as f64 / (SAMPLES - 1) as f64);
            if target > walked + segment {
                break;
            }
            out.push(a + (b - a) * ((target - walked) / segment).clamp(0.0, 1.0));
        }
        walked += segment;
    }
    out.push(last);
    out
}

/// A closed stroke has no special starting corner. Split at distant points and simplify both
/// halves, then discard collinear seam points instead of counting them as polygon vertices.
fn closed_corners(pts: &[Point], tol: f64) -> Vec<Point> {
    let Some(&first) = pts.first() else { return Vec::new() };
    let start = pts.iter().enumerate().max_by(|(_, a), (_, b)| a.distance(first).total_cmp(&b.distance(first))).map_or(0, |(i, _)| i);
    let ring: Vec<Point> = pts.iter().skip(start).chain(pts.iter().take(start)).copied().collect();
    let Some(&first) = ring.first() else { return Vec::new() };
    let split = ring.iter().enumerate().max_by(|(_, a), (_, b)| a.distance(first).total_cmp(&b.distance(first))).map_or(0, |(i, _)| i);
    let (Some(left), Some(right)) = (ring.get(..=split), ring.get(split..)) else { return Vec::new() };
    let mut corners = simplify(left, tol);
    corners.pop();
    let other: Vec<Point> = right.iter().copied().chain(std::iter::once(first)).collect();
    let mut other = simplify(&other, tol);
    other.pop();
    corners.extend(other);
    while corners.len() > 3 {
        let n = corners.len();
        let redundant = corners.iter().enumerate().find_map(|(i, p)| {
            let (&a, &b) = (corners.get((i + n - 1) % n)?, corners.get((i + 1) % n)?);
            (dist_to_segment(*p, a, b) <= tol).then_some(i)
        });
        let Some(i) = redundant else { break };
        corners.remove(i);
    }
    corners
}

fn area(poly: &[Point]) -> f64 {
    let Some(&origin) = poly.first() else { return 0.0 };
    // Translating to the first point avoids cancellation far from the document origin.
    poly.iter().zip(poly.iter().skip(1)).map(|(&a, &b)| (a - origin).cross(b - origin)).sum::<f64>().abs() / 2.0
}

/// Four substantial, approximately perpendicular edges, even with uneven sides and corners.
fn is_rectangle(pts: &[Point], corners: &[Point], size: f64) -> bool {
    if corners.len() != 4 || !is_polygonal(pts, corners, size, 0.055) {
        return false;
    }
    right_angles(corners, size)
}

fn right_angles(corners: &[Point], size: f64) -> bool {
    let around: Vec<Point> = corners.iter().cycle().take(corners.len() + 2).copied().collect();
    around.windows(3).all(|w| {
        let [a, b, c] = w else { return false };
        let (u, v) = (*b - *a, *c - *b);
        let (lu, lv) = (u.hypot(), v.hypot());
        lu > 0.03 * size && lv > 0.03 * size && (u.dot(v) / (lu * lv)).abs() < 0.45
    })
}

/// Fit all vertices, so neither the starting point nor drawing direction picks the orientation.
fn polygon_rotation(corners: &[Point], step: f64) -> f64 {
    let Some(&origin) = corners.first() else { return 0.0 };
    let n = corners.len() as f64;
    let center = origin + corners.iter().fold(Vec2::ZERO, |sum, p| sum + (*p - origin) / n);
    let period = 360.0 / n;
    let error = |rotation: f64| {
        corners
            .iter()
            .map(|p| {
                let angle = (*p - center).atan2().to_degrees() + 90.0;
                let delta = (angle - rotation).rem_euclid(period);
                delta.min(period - delta).powi(2)
            })
            .sum::<f64>()
    };
    // A candidate a whole number of periods from an earlier one is the same polygon (a hexagon at 270°
    // is the one at 90°) and scores the same up to rounding, so it is dropped: otherwise the platform's
    // `atan2` rounding picks between them (FreeBSD's libm gave 270° where Linux gives 90°).
    let candidates = [0.0, 90.0, 180.0, 270.0].into_iter().filter(|r| r % step == 0.0);
    let distinct = candidates.clone().enumerate().filter(|&(i, r)| !candidates.clone().take(i).any(|q| (r - q) % period == 0.0));
    distinct.map(|(_, r)| r).min_by(|a, b| error(*a).total_cmp(&error(*b))).unwrap_or(0.0)
}

fn rectangle_rotation(corners: &[Point]) -> f64 {
    let (sin, cos) = corners.iter().zip(corners.iter().cycle().skip(1)).fold((0.0, 0.0), |(sin, cos), (&a, &b)| {
        let angle = (b - a).atan2() * 4.0;
        (sin + angle.sin(), cos + angle.cos())
    });
    (sin.atan2(cos).to_degrees() / 4.0 / 45.0).round().mul_add(45.0, 0.0).rem_euclid(180.0)
}

fn ellipse_rotation(pts: &[Point]) -> f64 {
    let Some(&origin) = pts.first() else { return 0.0 };
    let n = pts.len() as f64;
    let center = origin + pts.iter().fold(Vec2::ZERO, |sum, p| sum + (*p - origin) / n);
    let (xx, yy, xy) = pts.iter().fold((0.0, 0.0, 0.0), |(xx, yy, xy), p| {
        let v = *p - center;
        (xx + v.x * v.x, yy + v.y * v.y, xy + v.x * v.y)
    });
    // A near-circle has no useful orientation; keep it upright rather than following noise.
    if (xx - yy).hypot(2.0 * xy) < 0.1 * (xx + yy) {
        return 0.0;
    }
    ((2.0 * xy).atan2(xx - yy).to_degrees() / 2.0 / 45.0).round().mul_add(45.0, 0.0).rem_euclid(180.0)
}

/// Dimensions in the fitted frame, but the centre stays in document coordinates.
fn oriented_bounds(pts: &[Point], rotation: f64) -> Rect {
    if rotation == 0.0 {
        return bbox(pts);
    }
    let origin = pts.first().copied().unwrap_or(Point::ZERO);
    let rotate = Affine::rotate(rotation.to_radians());
    let inverse = Affine::rotate(-rotation.to_radians());
    let local: Vec<Point> = pts.iter().map(|p| inverse * (*p - origin).to_point()).collect();
    let b = bbox(&local);
    Rect::from_center_size(origin + (rotate * b.center()).to_vec2(), (b.width(), b.height()))
}

/// Number of sharp direction reversals (turns of more than 120°) along the stroke.
fn reversals(pts: &[Point], min_seg: f64) -> usize {
    let s = simplify(pts, min_seg);
    s.windows(3)
        .filter(|w| {
            let (u, v): (Vec2, Vec2) = (w[1] - w[0], w[2] - w[1]);
            u.hypot() > min_seg && v.hypot() > min_seg && u.dot(v) / (u.hypot() * v.hypot()) < -0.5
        })
        .count()
}

/// Recognise a freehand stroke (document points, in drawing order).
pub fn recognize(pts: &[Point]) -> Option<Recognized> {
    if pts.len() < 2 || pts.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    let b = bbox(pts);
    let size = b.width().max(b.height());
    if size < 2.0 || !size.is_finite() {
        return None;
    }
    let len = path_len(pts);
    if !len.is_finite() || len <= 0.0 {
        return None;
    }
    let original = pts;
    let samples = resample(pts, len);
    let pts = samples.as_slice();
    let (&first, &last) = (pts.first()?, pts.last()?);
    let chord = first.distance(last);
    // Scribble: several sharp reversals packed into a small area relative to the stroke length.
    if reversals(pts, size * 0.08) >= 2 && len > 1.5 * size {
        // An equilateral triangle turns 120° at its vertices: drawing noise can make all
        // three turns just exceed the reversal threshold. Its closed, straight edges and
        // substantial area distinguish it from a deletion scribble.
        let corners = closed_corners(pts, size * 0.12);
        let triangle = chord <= 0.25 * size && corners.len() == 3 && area(pts) > 0.2 * size * size && is_polygonal(pts, &corners, size, 0.055);
        if !triangle {
            return Some(Recognized::Scribble(b));
        }
    }
    // Line: every point near the chord.
    let max_dev = pts.iter().map(|p| dist_to_segment(*p, first, last)).fold(0.0, f64::max);
    if chord > 0.5 * size && max_dev < 0.08 * chord.max(1.0) && len < 1.3 * chord {
        return Some(Recognized::Line { a: first, b: last });
    }
    // Closed shapes: the ends meet (within a fifth of the size).
    if chord > 0.25 * size {
        return None;
    }
    let corners = closed_corners(pts, size * 0.12);
    let fill = area(pts) / (b.width() * b.height()).max(1e-9);
    if !fill.is_finite() {
        return None;
    }
    let center = b.center();
    let square = |r: Rect| {
        let (w, h) = (r.width(), r.height());
        if (w - h).abs() < 0.1 * w.max(h) {
            let s = (w + h) / 2.0;
            Rect::from_center_size(r.center(), (s, s))
        } else {
            r
        }
    };
    match corners.len() {
        3 => {
            let radius = corners.iter().map(|p| p.distance(center)).sum::<f64>() / 3.0;
            Some(Recognized::Polygon { center, radius, sides: 3, rotation: polygon_rotation(&corners, 180.0) })
        }
        4 if is_rectangle(pts, &corners, size) => {
            let rotation = rectangle_rotation(&corners);
            Some(Recognized::Rectangle { rect: square(oriented_bounds(original, rotation)), rotation })
        }
        n @ 5..=8 if fill > 0.6 && fill < 0.85 && is_polygonal(pts, &corners, size, 0.02) => {
            let radius = corners.iter().map(|p| p.distance(center)).sum::<f64>() / n as f64;
            Some(Recognized::Polygon { center, radius, sides: n as u32, rotation: if n == 6 { polygon_rotation(&corners, 90.0) } else { 0.0 } })
        }
        _ => {
            let rotation = ellipse_rotation(pts);
            let rect = oriented_bounds(original, rotation);
            (area(pts) / (rect.width() * rect.height()).max(1e-9) > 0.6).then(|| Recognized::Ellipse { rect: square(rect), rotation })
        }
    }
}

/// Do the stroke's points hug the straight edges between `corners` (rather than bulging like an
/// ellipse)?
fn is_polygonal(pts: &[Point], corners: &[Point], size: f64, tolerance: f64) -> bool {
    if pts.is_empty() || corners.len() < 3 {
        return false;
    }
    let dev: f64 = pts
        .iter()
        .map(|p| corners.iter().zip(corners.iter().cycle().skip(1)).map(|(&a, &b)| dist_to_segment(*p, a, b)).fold(f64::MAX, f64::min))
        .sum::<f64>()
        / pts.len() as f64;
    dev < tolerance * size
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wobbly stroke through `corners` (closed), `k` samples per edge.
    fn wobbly(corners: &[Point], k: usize, wobble: f64) -> Vec<Point> {
        let mut out = vec![];
        let n = corners.len();
        for i in 0..n {
            let (a, b) = (corners[i], corners[(i + 1) % n]);
            for j in 0..k {
                let t = j as f64 / k as f64;
                let w = ((i * k + j) as f64 * 1.7).sin() * wobble;
                out.push(a + (b - a) * t + Vec2::new(w, -w));
            }
        }
        out.push(corners[0] + Vec2::new(3.0, 2.0));
        out
    }

    fn circle(c: Point, rx: f64, ry: f64, k: usize) -> Vec<Point> {
        (0..=k)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / k as f64;
                c + Vec2::new(rx * a.cos() + (a * 5.0).sin(), ry * a.sin())
            })
            .collect()
    }

    #[test]
    fn rough_rectangle_and_square() {
        let r = wobbly(&[Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(200.0, 100.0), Point::new(0.0, 100.0)], 20, 2.0);
        let Some(Recognized::Rectangle { rect: b, .. }) = recognize(&r) else { panic!("{:?}", recognize(&r)) };
        assert!((b.width() - 200.0).abs() < 8.0 && (b.height() - 100.0).abs() < 8.0);
        let s = wobbly(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 95.0), Point::new(0.0, 95.0)], 20, 1.0);
        let Some(Recognized::Rectangle { rect: b, .. }) = recognize(&s) else { panic!() };
        assert!((b.width() - b.height()).abs() < 1e-9, "near-squares snap to squares");
    }

    #[test]
    fn rough_squares_are_rectangles_from_any_starting_point() {
        let cases = [
            [Point::new(0.0, 0.0), Point::new(100.0, 8.0), Point::new(94.0, 100.0), Point::new(10.0, 91.0)],
            [Point::new(0.0, 8.0), Point::new(94.0, 0.0), Point::new(100.0, 95.0), Point::new(7.0, 100.0)],
        ];
        for vertices in cases {
            for wobble in [0.5, 4.0] {
                let mut stroke = wobbly(&vertices, 24, wobble);
                stroke.pop();
                for start in [0, 5, 12, 20, 30, 47, 60, 79] {
                    stroke.rotate_left(start);
                    let mut closed = stroke.clone();
                    closed.push(stroke[0] + Vec2::new(2.0, -1.0));
                    assert!(
                        matches!(recognize(&closed), Some(Recognized::Rectangle { .. })),
                        "wobble {wobble}, start {start}: {:?}",
                        recognize(&closed)
                    );
                    stroke.rotate_right(start);
                }
            }
        }
    }

    #[test]
    fn ellipse_circle_triangle_hexagon_line() {
        assert!(
            matches!(recognize(&circle(Point::new(100.0, 100.0), 80.0, 40.0, 72)), Some(Recognized::Ellipse { rect: r, .. }) if r.width() > 150.0 && r.height() < 100.0)
        );
        assert!(
            matches!(recognize(&circle(Point::new(100.0, 100.0), 50.0, 48.0, 72)), Some(Recognized::Ellipse { rect: r, .. }) if (r.width() - r.height()).abs() < 1e-9)
        );
        let tri = wobbly(&[Point::new(100.0, 0.0), Point::new(200.0, 170.0), Point::new(0.0, 170.0)], 25, 1.5);
        assert!(matches!(recognize(&tri), Some(Recognized::Polygon { sides: 3, rotation, .. }) if rotation.abs() < 5.0), "{:?}", recognize(&tri));
        let hex: Vec<Point> = (0..6)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 6.0;
                Point::new(100.0 + 80.0 * a.cos(), 100.0 + 80.0 * a.sin())
            })
            .collect();
        assert!(matches!(recognize(&wobbly(&hex, 15, 0.5)), Some(Recognized::Polygon { sides: 6, .. })), "{:?}", recognize(&wobbly(&hex, 15, 0.5)));
        let line: Vec<Point> = (0..30).map(|i| Point::new(i as f64 * 10.0, i as f64 * 3.0 + (i as f64).sin())).collect();
        assert!(matches!(recognize(&line), Some(Recognized::Line { .. })));
    }

    #[test]
    fn circles_and_real_polygons_keep_their_identity_across_stroke_seams() {
        for (rx, ry) in [(50.0, 50.0), (80.0, 40.0), (50.0, 40.0)] {
            let mut stroke = circle(Point::new(100.0, 100.0), rx, ry, 192);
            stroke.pop();
            for start in [0, 13, 37, 62, 109] {
                stroke.rotate_left(start);
                let mut closed = stroke.clone();
                closed.push(stroke[0]);
                assert!(matches!(recognize(&closed), Some(Recognized::Ellipse { .. })), "ellipse {rx}/{ry}, seam {start}: {:?}", recognize(&closed));
                stroke.rotate_right(start);
            }
        }
        for sides in [3, 5, 6, 8] {
            let corners: Vec<Point> = (0..sides)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / sides as f64;
                    Point::new(100.0 + 80.0 * a.cos(), 100.0 + 80.0 * a.sin())
                })
                .collect();
            let mut stroke = wobbly(&corners, 24, 1.0);
            stroke.pop();
            for start in [0, 9, 17, 29, 45] {
                stroke.rotate_left(start);
                let mut closed = stroke.clone();
                closed.push(stroke[0]);
                assert!(
                    matches!(recognize(&closed), Some(Recognized::Polygon { sides: n, .. }) if n == sides),
                    "{sides} sides, seam {start}: {:?}",
                    recognize(&closed)
                );
                stroke.rotate_right(start);
            }
        }
    }

    #[test]
    fn rough_triangles_snap_upright_or_upside_down() {
        for direction in [0.0_f64, 180.0] {
            for tilt in [-12.0_f64, -3.0, 4.0, 11.0] {
                let corners: Vec<Point> = (0..3)
                    .map(|i| {
                        let a = (direction + tilt - 90.0).to_radians() + std::f64::consts::TAU * i as f64 / 3.0;
                        Point::new(200.0 + 80.0 * a.cos(), 300.0 + 80.0 * a.sin())
                    })
                    .collect();
                for reversed in [false, true] {
                    let mut stroke = wobbly(&corners, 24, 1.5);
                    stroke.pop();
                    if reversed {
                        stroke.reverse();
                    }
                    for seam in [0, 9, 28, 51] {
                        stroke.rotate_left(seam);
                        let mut closed = stroke.clone();
                        closed.push(stroke[0]);
                        let Some(Recognized::Polygon { sides: 3, rotation, .. }) = recognize(&closed) else {
                            panic!("direction {direction}, tilt {tilt}, seam {seam}: {:?}", recognize(&closed));
                        };
                        assert_eq!(rotation, direction, "tilt {tilt}, seam {seam}, reversed {reversed}");
                        stroke.rotate_right(seam);
                    }
                }
            }
        }
    }

    #[test]
    fn recognized_lines_keep_their_free_angle() {
        for (dx, dy) in [(120.0, 7.0), (-130.0, -9.0), (8.0, 90.0), (-6.0, -140.0), (60.0, 50.0), (-50.0, -60.0)] {
            let first = Point::new(200.0, 300.0);
            let last = first + Vec2::new(dx, dy);
            let stroke: Vec<Point> = (0..=30).map(|i| first + (last - first) * (i as f64 / 30.0)).collect();
            assert_eq!(recognize(&stroke), Some(Recognized::Line { a: first, b: last }));
        }
    }

    #[test]
    fn tilted_rectangles_ellipses_and_hexagons_snap_to_their_angle_steps() {
        for direction in [0.0_f64, 45.0, 90.0, 135.0] {
            for tilt in [-7.0_f64, 7.0] {
                let xf = Affine::translate((200.0, 300.0)) * Affine::rotate((direction + tilt).to_radians());
                let corners: Vec<Point> =
                    [(-90.0, -40.0), (90.0, -40.0), (90.0, 40.0), (-90.0, 40.0)].into_iter().map(|p| xf * Point::from(p)).collect();
                let stroke = wobbly(&corners, 24, 1.0);
                let Some(Recognized::Rectangle { rect, rotation }) = recognize(&stroke) else {
                    panic!("rectangle {direction}: {:?}", recognize(&stroke))
                };
                assert_eq!(rotation % 45.0, 0.0);
                assert!(rect.center().distance(Point::new(200.0, 300.0)) < 4.0);
                assert!((rect.width().max(rect.height()) - 180.0).abs() < 15.0);
                let ellipse: Vec<Point> = circle(Point::ZERO, 90.0, 40.0, 192).into_iter().map(|p| xf * p).collect();
                let Some(Recognized::Ellipse { rect, rotation }) = recognize(&ellipse) else {
                    panic!("ellipse {direction}: {:?}", recognize(&ellipse))
                };
                assert_eq!(rotation, direction);
                assert!((rect.width() - 180.0).abs() < 4.0 && (rect.height() - 80.0).abs() < 4.0);
            }
        }
        for direction in [0.0_f64, 90.0] {
            for tilt in [-7.0_f64, 7.0] {
                let corners: Vec<Point> = (0..6)
                    .map(|i| {
                        let a = (direction + tilt - 90.0).to_radians() + std::f64::consts::TAU * i as f64 / 6.0;
                        Point::new(200.0 + 70.0 * a.cos(), 300.0 + 70.0 * a.sin())
                    })
                    .collect();
                let stroke = wobbly(&corners, 24, 0.5);
                let Some(Recognized::Polygon { sides: 6, rotation, .. }) = recognize(&stroke) else { panic!("hexagon: {:?}", recognize(&stroke)) };
                assert_eq!(rotation, direction);
            }
        }
        // Even sideways input can only produce an upright or inverted triangle.
        for direction in [83.0_f64, 97.0, 263.0, 277.0] {
            let corners: Vec<Point> = (0..3)
                .map(|i| {
                    let a = (direction - 90.0).to_radians() + std::f64::consts::TAU * i as f64 / 3.0;
                    Point::new(200.0 + 70.0 * a.cos(), 300.0 + 70.0 * a.sin())
                })
                .collect();
            assert!(matches!(recognize(&wobbly(&corners, 24, 0.5)), Some(Recognized::Polygon { sides: 3, rotation: 0.0 | 180.0, .. })));
        }
    }

    #[test]
    fn hexagon_rotation_never_depends_on_rounding_between_equivalent_angles() {
        // 0° and 180° (and 90° and 270°) are the same hexagon, so only 0° and 90° may come back, whatever
        // rounding does to the tied scores: on FreeBSD the tilted test above got 270°.
        for direction in [0.0_f64, 90.0] {
            for tilt in [-9.0_f64, -7.0, -3.0, 3.0, 7.0, 9.0] {
                for offset in [0.0, 1e-7, 0.1, 1e3, 1e6] {
                    let corners: Vec<Point> = (0..6)
                        .map(|i| {
                            let a = (direction + tilt - 90.0).to_radians() + std::f64::consts::TAU * i as f64 / 6.0;
                            Point::new(offset + 70.0 * a.cos(), offset + 70.0 * a.sin())
                        })
                        .collect();
                    assert_eq!(polygon_rotation(&corners, 90.0), direction, "tilt {tilt}, offset {offset}");
                    let mut shifted = corners.clone();
                    shifted.rotate_left(3);
                    assert_eq!(polygon_rotation(&shifted, 90.0), direction, "tilt {tilt}, offset {offset}, from the opposite corner");
                }
            }
        }
        // Triangles have no such twin among the candidates: 0° and 180° stay distinct.
        let down: Vec<Point> = (0..3).map(|i| Point::new(0.0, 0.0) + Vec2::from_angle((90.0_f64 + 120.0 * i as f64).to_radians()) * 50.0).collect();
        assert_eq!(polygon_rotation(&down, 180.0), 180.0);
    }

    #[test]
    fn square_fit_ignores_sampling_density_and_document_offset() {
        let mut stroke = wobbly(&[Point::new(0.0, 0.0), Point::new(100.0, 8.0), Point::new(94.0, 100.0), Point::new(10.0, 91.0)], 24, 4.0);
        for scale in [0.25, 1.0, 20.0] {
            let shifted: Vec<Point> = stroke.iter().map(|p| Point::new(1e9 + p.x * scale, -1e9 + p.y * scale)).collect();
            assert!(matches!(recognize(&shifted), Some(Recognized::Rectangle { .. })), "scale {scale}: {:?}", recognize(&shifted));
        }
        let dense: Vec<Point> = stroke.iter().enumerate().flat_map(|(i, p)| std::iter::repeat_n(*p, if i < 24 { 100 } else { 1 })).collect();
        assert_eq!(recognize(&stroke), recognize(&dense));
        stroke.extend(std::iter::repeat_n(*stroke.last().unwrap(), 10_000));
        assert!(matches!(recognize(&stroke), Some(Recognized::Rectangle { .. })));
    }

    #[test]
    fn non_finite_and_overflowing_strokes_are_not_shapes() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(recognize(&[Point::new(0.0, 0.0), Point::new(bad, 100.0)]), None);
        }
        assert_eq!(recognize(&[Point::new(-f64::MAX, 0.0), Point::new(f64::MAX, 100.0)]), None);
    }

    #[test]
    fn scribble_and_noise() {
        let z: Vec<Point> = (0..40).map(|i| Point::new(100.0 + (i % 2) as f64 * 60.0, 100.0 + i as f64 * 2.0)).collect();
        assert!(matches!(recognize(&z), Some(Recognized::Scribble(_))));
        assert_eq!(recognize(&[Point::new(0.0, 0.0)]), None);
        // An open curl is not a shape.
        let curl: Vec<Point> = (0..60)
            .map(|i| {
                let a = i as f64 * 0.08;
                Point::new(100.0 + 50.0 * a.cos(), 100.0 + 50.0 * a.sin())
            })
            .collect();
        assert_eq!(recognize(&curl), None);
    }
}
