//! Gradients in Vector Drawable terms.
//!
//! A gradient is given on a frame `F`, an affine map of the unit box into the viewport: (0,0) is
//! where it starts, (1,0) where it ends and (0,1) its width handle. A linear gradient bands
//! along `F`'s second axis, so its Vector Drawable end point is the first axis with the part
//! along the second removed: exact for any frame, skewed too. Radial and sweep gradients live on
//! the unit circle: `F = T · R(φ) · S(sx, sy) · R(θ)` (a 2×2 singular value decomposition; `sy`
//! is negative for a mirrored frame), and a circle doesn't change under the last rotation, so a
//! `<group>` with translate `T`, rotation `φ` and scale `S` holds a unit gradient at the origin
//! and the path mapped back by the group's inverse. A sweep turns with `θ`: its stops are
//! shifted by `θ / 2π` instead.

use kurbo::{Affine, Vec2};

use crate::model::{Color, Gradient, GradientKind, GroupTransform};

/// The shape of a gradient on its frame ([`place`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// From (0,0) to (1,0), banded along the second axis.
    Linear,
    /// The unit circle around (0,0).
    Radial,
    /// Around (0,0), starting along the first axis.
    Sweep,
}

/// A gradient fill, ready to write.
#[derive(Clone, Debug, PartialEq)]
pub enum Placed {
    /// In viewport coordinates, on the path as it is.
    Plain(Gradient),
    /// In the coordinates of a `<group>` with this transform; `inverse` maps the path into them.
    Grouped { transform: GroupTransform, inverse: Affine, gradient: Gradient },
    /// One stop: a solid colour.
    Solid(Color),
    /// No stops, or a frame that collapses to a line or a point: nothing is painted.
    Nothing,
}

/// `m` (2×2, column vectors `(a, b)` and `(c, d)`) as `R(φ) · diag(sx, sy) · R(θ)`: `(φ, sx,
/// sy, θ)` in radians. `sx ≥ |sy|`; `sy < 0` when `m` mirrors.
pub fn svd(a: f64, b: f64, c: f64, d: f64) -> (f64, f64, f64, f64) {
    let (e, f, g, h) = ((a + d) / 2.0, (a - d) / 2.0, (b + c) / 2.0, (b - c) / 2.0);
    let (q, r) = (e.hypot(h), f.hypot(g));
    let (a1, a2) = (g.atan2(f), h.atan2(e));
    ((a2 + a1) / 2.0, q + r, q - r, (a2 - a1) / 2.0)
}

/// The colour of `stops` (sorted, at least one) at `t`, clamped at the ends.
pub fn color_at(stops: &[(f64, Color)], t: f64) -> Color {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else { return [0.0; 4] };
    if t <= first.0 {
        return first.1;
    }
    for w in stops.windows(2) {
        if let [(o0, c0), (o1, c1)] = w
            && t <= *o1
        {
            let span = o1 - o0;
            let k = if span > 0.0 { ((t - o0) / span) as f32 } else { 1.0 };
            return [0, 1, 2, 3].map(|i| c0[i] + (c1[i] - c0[i]) * k);
        }
    }
    last.1
}

/// Sweep stops turned by `shift` of a turn: what was at `t` is at `t + shift` (wrapping), with
/// the seam where the gradient starts over kept as a hard step.
pub fn turn_stops(stops: &[(f64, Color)], shift: f64) -> Vec<(f64, Color)> {
    let d = shift.rem_euclid(1.0);
    if !d.is_finite() || !(1e-9..=1.0 - 1e-9).contains(&d) || stops.is_empty() {
        return stops.to_vec();
    }
    let seam = 1.0 - d;
    let mut out = vec![(0.0, color_at(stops, seam))];
    out.extend(stops.iter().filter(|(o, _)| *o > seam).map(|(o, c)| (o - seam, *c)));
    out.push((d, color_at(stops, 1.0)));
    out.push((d, color_at(stops, 0.0)));
    out.extend(stops.iter().filter(|(o, _)| *o < seam).map(|(o, c)| (o + d, *c)));
    out.push((1.0, color_at(stops, seam)));
    // A stop on the seam itself would repeat at the same offset: keep one of each.
    out.dedup_by(|b, a| (a.0 - b.0).abs() < 1e-9 && a.1 == b.1);
    out
}

/// Whether `xf` keeps angles and scales evenly (a rotation, flip and uniform scale), so a stroke
/// width stays one width and Android can draw the stroke itself.
pub fn similarity(xf: Affine) -> bool {
    let [a, b, c, d, _, _] = xf.as_coeffs();
    let s2 = (a * a + b * b + c * c + d * d) / 2.0;
    s2.is_finite() && s2 > 0.0 && (a * c + b * d).abs() <= 1e-6 * s2 && (a * a + b * b - c * c - d * d).abs() <= 1e-6 * s2
}

/// A gradient of `shape` with `stops` (offsets ascending in 0..=1, opacity folded into the
/// colours) on `frame`, the unit box mapped into the viewport, as a drawable can say it.
pub fn place(shape: Shape, stops: Vec<(f64, Color)>, frame: Affine) -> Placed {
    match stops.as_slice() {
        [] => return Placed::Nothing,
        [(_, c)] => return Placed::Solid(*c),
        _ => {}
    }
    let [a, b, c, d, e, ff] = frame.as_coeffs();
    let det = a * d - b * c;
    let scale = (a * a + b * b + c * c + d * d).max(f64::MIN_POSITIVE);
    if !(det.is_finite() && e.is_finite() && ff.is_finite()) || det.abs() <= 1e-12 * scale {
        return Placed::Nothing;
    }
    if shape == Shape::Linear {
        let (a0, a1) = (Vec2::new(a, b), Vec2::new(c, d));
        let k = a0.dot(a1) / a1.hypot2();
        let n = a0 - a1 * k;
        return Placed::Plain(Gradient { kind: GradientKind::Linear { start: (e, ff), end: (e + n.x, ff + n.y) }, stops });
    }
    let (phi, sx, sy, theta) = svd(a, b, c, d);
    if !(phi.is_finite() && sx.is_finite() && sy.is_finite() && theta.is_finite()) || sy.abs() <= 1e-9 * sx.abs() {
        return Placed::Nothing;
    }
    let group = Affine::translate((e, ff)) * Affine::rotate(phi) * Affine::scale_non_uniform(sx, sy);
    let transform = GroupTransform { translate: (e, ff), rotation: phi.to_degrees(), scale: (sx, sy) };
    let gradient = if shape == Shape::Sweep {
        Gradient { kind: GradientKind::Sweep { center: (0.0, 0.0) }, stops: turn_stops(&stops, theta / std::f64::consts::TAU) }
    } else {
        Gradient { kind: GradientKind::Radial { center: (0.0, 0.0), radius: 1.0 }, stops }
    };
    Placed::Grouped { transform, inverse: group.inverse(), gradient }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use kurbo::Point;

    use super::*;

    const RED: Color = [1.0, 0.0, 0.0, 1.0];
    const GREEN: Color = [0.0, 1.0, 0.0, 1.0];
    const BLUE: Color = [0.0, 0.0, 1.0, 1.0];
    const YELLOW: Color = [1.0, 1.0, 0.0, 1.0];

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    fn stops() -> Vec<(f64, Color)> {
        vec![(0.0, RED), (0.25, GREEN), (0.7, BLUE), (1.0, YELLOW)]
    }

    /// Frames a layer's transform and a gradient's handles make: turned, skewed, mirrored.
    fn frames() -> Vec<Affine> {
        vec![
            Affine::scale(10.0),
            Affine::new([8.0, 0.0, 0.0, 3.0, 5.0, 5.0]),
            Affine::new([4.0, 3.0, -2.0, 5.0, 1.0, -2.0]),
            Affine::new([4.0, 3.0, 2.0, -5.0, 0.0, 0.0]),
            Affine::rotate(1.1) * Affine::new([6.0, 1.0, 2.5, 4.0, -3.0, 7.0]),
        ]
    }

    /// Where on the gradient the written drawable samples viewport point `p`.
    fn written_t(out: &Placed, p: Point) -> (f64, &Gradient) {
        let (g, p) = match out {
            Placed::Plain(g) => (g, p),
            Placed::Grouped { transform, inverse, gradient } => {
                // The group's transform as Android applies it puts the path back where it was.
                let (sx, sy) = transform.scale;
                let android =
                    Affine::translate(transform.translate) * Affine::rotate(transform.rotation.to_radians()) * Affine::scale_non_uniform(sx, sy);
                let back = android * *inverse * p;
                assert!(close(back.x, p.x) && close(back.y, p.y), "{back:?} vs {p:?}");
                (gradient, *inverse * p)
            }
            other => panic!("not a gradient: {other:?}"),
        };
        let t = match g.kind {
            GradientKind::Linear { start, end } => {
                let (s, e) = (Point::from(start), Point::from(end));
                (p - s).dot(e - s) / (e - s).hypot2()
            }
            GradientKind::Radial { center, radius } => (p - Point::from(center)).hypot() / radius,
            GradientKind::Sweep { center } => {
                let v = p - Point::from(center);
                v.y.atan2(v.x).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU
            }
        };
        (t, g)
    }

    /// Where on the gradient the frame itself puts viewport point `p`.
    fn frame_t(shape: Shape, frame: Affine, p: Point) -> f64 {
        let u = frame.inverse() * p;
        match shape {
            Shape::Linear => u.x,
            Shape::Radial => u.to_vec2().hypot(),
            Shape::Sweep => u.y.atan2(u.x).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU,
        }
    }

    #[test]
    fn every_gradient_samples_where_its_frame_samples_it() {
        for frame in frames() {
            for shape in [Shape::Linear, Shape::Radial, Shape::Sweep] {
                let out = place(shape, stops(), frame);
                assert_eq!(matches!(out, Placed::Plain(_)), shape == Shape::Linear, "{shape:?}");
                for (x, y) in [(0.3, 0.2), (-0.4, 0.6), (0.9, -0.1), (0.1, -0.8), (-0.5, -0.5)] {
                    let p = frame * Point::new(x, y);
                    let want = frame_t(shape, frame, p).clamp(0.0, 1.0);
                    let (got, grad) = written_t(&out, p);
                    let (a, b) = (color_at(&stops(), want), color_at(&grad.stops, got.clamp(0.0, 1.0)));
                    assert!((0..4).all(|i| (a[i] - b[i]).abs() < 1e-3), "{shape:?} {frame:?} at {x},{y}: {a:?} vs {b:?}");
                }
            }
        }
    }

    #[test]
    fn svd_puts_the_matrix_back_together() {
        let ms = [[1.0, 0.0, 0.0, 1.0], [2.0, 0.5, -0.3, 0.7], [0.0, 3.0, -2.0, 0.0], [-1.0, 0.2, 0.4, 1.5], [0.3, -0.9, -1.1, -0.2]];
        for [a, b, c, d] in ms {
            let (phi, sx, sy, theta) = svd(a, b, c, d);
            assert!(sx >= sy.abs() - 1e-12, "{sx} {sy}");
            assert_eq!(sy < 0.0, a * d - b * c < 0.0, "a mirror shows in sy");
            let m = Affine::rotate(phi) * Affine::scale_non_uniform(sx, sy) * Affine::rotate(theta);
            for (x, y) in m.as_coeffs().iter().zip([a, b, c, d]) {
                assert!(close(*x, y), "{:?} vs {:?}", m.as_coeffs(), [a, b, c, d]);
            }
        }
    }

    #[test]
    fn turned_stops_keep_every_colour_where_it_was() {
        let stops = stops();
        for shift in [0.1, 0.5, 0.95, -0.3, 1.25] {
            let turned = turn_stops(&stops, shift);
            assert!(turned.windows(2).all(|w| w[0].0 <= w[1].0), "sorted: {turned:?}");
            assert_eq!((turned[0].0, turned[turned.len() - 1].0), (0.0, 1.0));
            for i in 1..200 {
                let t = f64::from(i) / 200.0;
                let moved = (t + shift).rem_euclid(1.0);
                if !moved.is_nan() && !(1e-6..=1.0 - 1e-6).contains(&moved) {
                    continue;
                }
                let (a, b) = (color_at(&stops, t), color_at(&turned, moved));
                assert!((0..3).all(|i| (a[i] - b[i]).abs() < 1e-4), "{shift}: {t}");
            }
        }
        assert_eq!(turn_stops(&stops, 0.0), stops);
        assert_eq!(turn_stops(&stops, 2.0), stops);
        assert_eq!(turn_stops(&stops, f64::NAN), stops);
    }

    #[test]
    fn turned_stops_on_the_seam_are_not_repeated() {
        let out = turn_stops(&[(0.0, RED), (0.5, BLUE), (1.0, RED)], 0.5);
        assert!(out.windows(2).all(|w| w[0] != w[1]), "{out:?}");
    }

    #[test]
    fn a_collapsed_frame_paints_nothing_and_one_stop_is_solid() {
        let two = vec![(0.0, RED), (1.0, BLUE)];
        for shape in [Shape::Linear, Shape::Radial, Shape::Sweep] {
            // The width handle on the start point, both handles on one line, and numbers that aren't.
            assert_eq!(place(shape, two.clone(), Affine::new([5.0, 0.0, 0.0, 0.0, 1.0, 1.0])), Placed::Nothing);
            assert_eq!(place(shape, two.clone(), Affine::new([2.0, 4.0, 1.0, 2.0, 0.0, 0.0])), Placed::Nothing);
            assert_eq!(place(shape, two.clone(), Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0])), Placed::Nothing);
            assert_eq!(place(shape, two.clone(), Affine::new([1.0, 0.0, 0.0, 1.0, f64::INFINITY, 0.0])), Placed::Nothing);
            assert_eq!(place(shape, vec![(0.3, GREEN)], Affine::IDENTITY), Placed::Solid(GREEN));
            assert_eq!(place(shape, vec![], Affine::IDENTITY), Placed::Nothing);
        }
    }

    #[test]
    fn a_stroke_keeps_one_width_only_under_a_similarity() {
        assert!(similarity(Affine::IDENTITY) && similarity(Affine::rotate(0.7) * Affine::scale(3.0)));
        assert!(similarity(Affine::scale_non_uniform(2.0, -2.0)), "a flip");
        assert!(!similarity(Affine::scale_non_uniform(2.0, 1.0)) && !similarity(Affine::skew(0.3, 0.0)));
        assert!(!similarity(Affine::scale(0.0)) && !similarity(Affine::scale(f64::NAN)));
    }
}
