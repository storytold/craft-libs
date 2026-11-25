//! Sliding an anchor along its path: the anchor moves to a point on one of its two segments and
//! the segments beside it are refitted, so the outline stays where it was. It is a new anchor at
//! that point (an exact split) with the old one removed by the shared keep-shape refit.

use craft_geom::SubPath;
use kurbo::{CubicBez, ParamCurve, ParamCurveDeriv, ParamCurveNearest, Point};

use super::{ACCURACY, TINY, insert_at, neighbours};
use crate::addons::keep_shape::remove_anchor_keep_shape;
use crate::fit::is_straight;

/// How far along a neighbouring segment an anchor can slide: stopping short of the next anchor
/// keeps a segment between them.
pub const MAX_SLIDE: f64 = 0.98;

/// Steps of the search for the places on a curve where the tangent is at a snapping angle.
const STOP_SAMPLES: usize = 64;
/// Bisection steps that pin one of those places down.
const STOP_STEPS: usize = 40;
/// A stop this close to the anchor is the anchor itself.
const MIN_STOP: f64 = 1e-6;

/// The indices of the anchors beside anchor `i`; `None` for an end of an open subpath.
fn both_neighbours(sp: &SubPath, i: usize) -> Option<(usize, usize)> {
    if i >= sp.anchors.len() {
        return None;
    }
    match neighbours(sp, i) {
        (Some(prev), Some(next)) => Some((prev, next)),
        _ => None,
    }
}

/// Anchor `index` of `sp` moved along the path by `t` → (the new subpath, how far its outline
/// strays from the old one, in points). `t` says where: a positive one is the curve parameter on
/// the segment after the anchor, a negative one the parameter back from the end of the segment
/// before it (`-t` from 1). The subpath keeps its anchor count and the anchor its index. `None` when the anchor can't slide: an end of
/// an open subpath, out of range, `t` not a number or past [`MAX_SLIDE`], non-finite coordinates,
/// or a closed subpath of two anchors.
pub fn slide_anchor(sp: &SubPath, index: usize, t: f64) -> Option<(SubPath, f64)> {
    if t.is_nan() || t.abs() > MAX_SLIDE {
        return None;
    }
    let (prev, _) = both_neighbours(sp, index)?;
    let forward = t > 0.0;
    let (split, _) = if forward { insert_at(sp, &[(index, t)]) } else { insert_at(sp, &[(prev, 1.0 + t)]) };
    // Too small a move to split a segment: the anchor stays.
    if split.anchors.len() == sp.anchors.len() {
        return Some((sp.clone(), 0.0));
    }
    // Backward the new anchor comes just before the old one, or last on a closed subpath's first.
    let old = if !forward && index > 0 { index + 1 } else { index };
    let (mut out, strayed) = remove_anchor_keep_shape(&split, old)?;
    if !forward && index == 0 {
        out.anchors.rotate_right(1);
    }
    Some((out, strayed))
}

/// The slide (see [`slide_anchor`]) that brings anchor `index` nearest to `p`, held to
/// [`MAX_SLIDE`]; `None` when the anchor can't slide.
pub fn slide_t_at(sp: &SubPath, index: usize, p: Point) -> Option<f64> {
    let (prev, _) = both_neighbours(sp, index)?;
    let (before, after) = (sp.segment(prev).nearest(p, ACCURACY), sp.segment(index).nearest(p, ACCURACY));
    let t = if before.distance_sq < after.distance_sq { before.t - 1.0 } else { after.t };
    Some(t.clamp(-MAX_SLIDE, MAX_SLIDE))
}

/// The slides of anchor `index` (see [`slide_anchor`]) that land where the path's tangent is at a
/// multiple of 45° from `base` degrees, in order.
pub fn slide_stops(sp: &SubPath, index: usize, base: f64) -> Vec<f64> {
    let Some((prev, _)) = both_neighbours(sp, index) else { return vec![] };
    let before = tangent_stops(&sp.segment(prev), base).into_iter().map(|t| t - 1.0);
    before.chain(tangent_stops(&sp.segment(index), base)).filter(|t| (MIN_STOP..=MAX_SLIDE).contains(&t.abs())).collect()
}

/// The parameters of `c` inside 0..1 where its tangent is at a multiple of 45° from `base`
/// degrees, in order. A straight segment has none: its tangent never reaches one or never leaves.
fn tangent_stops(c: &CubicBez, base: f64) -> Vec<f64> {
    if is_straight(c, TINY) {
        return vec![];
    }
    let d = c.deriv();
    let base = base.to_radians();
    // Zero at each multiple, and it doesn't jump where the angle wraps round or the curve reverses.
    let f = |t: f64| {
        let v = d.eval(t).to_vec2();
        (v.hypot() > TINY).then(|| ((v.y.atan2(v.x) - base) * 4.0).sin())
    };
    let mut out = vec![];
    for k in 0..STOP_SAMPLES {
        let (mut a, mut b) = (k as f64 / STOP_SAMPLES as f64, (k + 1) as f64 / STOP_SAMPLES as f64);
        let (Some(fa), Some(fb)) = (f(a), f(b)) else { continue };
        if (fa < 0.0) == (fb < 0.0) {
            continue;
        }
        for _ in 0..STOP_STEPS {
            let m = (a + b) / 2.0;
            match f(m) {
                Some(fm) if (fm < 0.0) == (fa < 0.0) => a = m,
                _ => b = m,
            }
        }
        out.push(b);
    }
    out
}
