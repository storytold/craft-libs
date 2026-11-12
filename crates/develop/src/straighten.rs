//! Auto straighten: the crop angle that levels a photo, from three cues on the analysis image
//! ([`crate::upright::analysis_plane`]), tried in this order:
//!
//! 1. **Plumb lines**: the roll from the vertical vanishing point. Verticals that converge in
//!    perspective (a building shot upward) still name the camera's roll, where their mean tilt
//!    doesn't. Only a vanishing point backed by many lines across the frame counts.
//! 2. **The horizon**: the straight line across the whole width along which it is brighter on one
//!    side than on the other (sky and sea, sky and land). It is scored by its weaker columns, so
//!    the edges of waves, clouds or a ridge, which hold only in places, don't make one.
//! 3. **Long lines**: the consensus of near-horizontal or near-vertical segments (the rule Upright
//!    Level uses), with at least [`MIN_LINES`] agreeing. It also vetoes a horizon it disagrees with.
//!
//! Nothing past [`MAX_DEGREES`] is levelled.
//!
//! Measured on 113 photos straightened by hand in Lightroom Classic, against the long lines alone:
//! 50 instead of 29 come out straighter, 10 (as before) more tilted, and the mean error is 0.82°
//! instead of 1.01°. Of 300 photos left as shot, 11 instead of 8 are turned by more than 3°.

use crate::raster::Plane;

use crate::upright::{Analysis, Segment, detect_segments, level_angle_within, vanishing_point, vp_error};

/// The most Auto straighten turns a photo (degrees). None of the 113 photos straightened by hand
/// needed more than 6.1°, and larger "levels" came from texture (planks, a branch, a bird's wing).
pub const MAX_DEGREES: f64 = 5.0;

/// The fewest agreeing long lines that level a photo on their own: one line (the frame of a
/// window, a branch) said nothing about the horizon.
pub const MIN_LINES: usize = 3;

/// Long lines read up to this angle, so that a steeper consensus can veto a horizon.
const LINES_MAX_DEGREES: f64 = 10.0;
/// A horizon the long lines disagree with by more than this (degrees) is not used.
const VETO_DEGREES: f64 = 1.0;

/// Plumb lines: fewest inliers of the vertical vanishing point, their least total length and the
/// least spread across the frame (both in half long edges), and the most a vertical may lean.
const PLUMB_MIN_LINES: usize = 8;
const PLUMB_MIN_LENGTH: f64 = 1.5;
const PLUMB_MIN_SPREAD: f64 = 0.6;
const PLUMB_MAX_LEAN: f64 = 20.0;

/// Horizon: blur (px), rows averaged on each side of the line, the share of columns that may be
/// weaker than the score, the least score (relative to the image's strongest edges), and the
/// share of the height at the top and bottom a horizon doesn't lie in.
const HORIZON_SIGMA: f32 = 1.5;
const HORIZON_ROWS: usize = 4;
const HORIZON_QUANTILE: f64 = 0.3;
const HORIZON_MIN_SCORE: f64 = 0.3;
const HORIZON_MARGIN: f64 = 0.05;
/// Horizon search: the angles tried (degrees) before the limit applies, and their steps.
const HORIZON_SEARCH: f64 = 10.0;
const HORIZON_STEP: f64 = 0.25;
const HORIZON_FINE_STEP: f64 = 0.02;

/// The crop angle (degrees, clockwise +) that levels the photo whose analysis image is `lum` and
/// whose segments are analysed in `a`, or `None` when no cue can be trusted.
pub(crate) fn decide(lum: &Plane, a: &Analysis) -> Option<f64> {
    if let Some(roll) = plumb_roll(&a.vertical) {
        return Some(roll);
    }
    let lines = level_angle_within(&a.vertical, &a.horizontal, LINES_MAX_DEGREES, MIN_LINES);
    match (horizon(lum), lines) {
        (Some(h), Some(l)) => ((h - l).abs() <= VETO_DEGREES).then_some(h),
        (h, l) => h.or(l.filter(|l| l.abs() <= MAX_DEGREES)),
    }
}

/// [`decide`] for an analysis image alone (its segments detected here).
pub fn level_plane(lum: &Plane) -> Option<f64> {
    decide(lum, &crate::upright::analyze(&detect_segments(lum)))
}

fn fold(a: f64) -> f64 {
    if a > 90.0 {
        a - 180.0
    } else if a < -90.0 {
        a + 180.0
    } else {
        a
    }
}

/// The roll named by the vanishing point of the near-vertical segments.
fn plumb_roll(vertical: &[Segment]) -> Option<f64> {
    let lines: Vec<Segment> = vertical.iter().copied().filter(|s| s.tilt_from_vertical() < PLUMB_MAX_LEAN).collect();
    let (v, _) = vanishing_point(&lines)?;
    let thr = 2f64.to_radians();
    let inliers: Vec<&Segment> = lines.iter().filter(|s| vp_error(s, v) < thr).collect();
    let length: f64 = inliers.iter().map(|s| s.len()).sum();
    let (lo, hi) = inliers.iter().map(|s| s.mid().x).fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
    if inliers.len() < PLUMB_MIN_LINES || length < PLUMB_MIN_LENGTH || hi - lo < PLUMB_MIN_SPREAD {
        return None;
    }
    // a vanishing point inside or near the frame is a scene feature, not the camera's roll
    let finite = v[2].abs() > 1e-9;
    if finite && (v[0] / v[2]).hypot(v[1] / v[2]) < 2.0 {
        return None;
    }
    let (vx, vy) = if finite { (v[0] / v[2], v[1] / v[2]) } else { (v[0], v[1]) };
    let roll = fold(vx.atan2(vy).to_degrees());
    (roll.is_finite() && roll.abs() <= MAX_DEGREES).then_some(roll)
}

/// The crop angle that levels the horizon of `lum`, if it has one: the straight line across the
/// width along which the rows above are brighter (or darker) than those below in most columns.
fn horizon(lum: &Plane) -> Option<f64> {
    let img = crate::raster::blur::gaussian(lum, HORIZON_SIGMA);
    let (w, h) = (img.width, img.height);
    let k = HORIZON_ROWS;
    if w < 32 || h < 4 * k + 8 || img.data.len() < w * h {
        return None;
    }
    // per sampled column: the mean of the k rows above each row minus that of the k rows below
    let cols: Vec<usize> = (0..w).step_by(2).collect();
    let mut diff = vec![0f32; cols.len() * h];
    let mut prefix = vec![0f64; h + 1];
    for (ci, &x) in cols.iter().enumerate() {
        for y in 0..h {
            prefix[y + 1] = prefix[y] + f64::from(img.data[y * w + x]);
        }
        for y in k..h - k {
            let above = (prefix[y] - prefix[y - k]) / k as f64;
            let below = (prefix[y + k] - prefix[y]) / k as f64;
            diff[ci * h + y] = (above - below) as f32;
        }
    }
    let mut strengths: Vec<f32> = diff.iter().map(|d| d.abs()).filter(|d| *d > 0.0).collect();
    if strengths.len() < 100 {
        return None;
    }
    let p99 = strengths.len() * 99 / 100;
    strengths.select_nth_unstable_by(p99, f32::total_cmp);
    let norm = f64::from(strengths[p99]).max(1e-4);
    let cx = w as f64 / 2.0;
    let rows = ((h as f64 * HORIZON_MARGIN) as isize).max(k as isize)..((h as f64 * (1.0 - HORIZON_MARGIN)) as isize).min((h - k) as isize);
    let mut column = Vec::with_capacity(cols.len());
    // the score of the line through (cx, b) with slope t: the contrast most of its columns reach
    let mut score = |t: f64, b: f64, sign: f32| -> f64 {
        column.clear();
        for (ci, &x) in cols.iter().enumerate() {
            let y = (b + t * (x as f64 - cx)).round() as isize;
            if rows.contains(&y) {
                column.push(sign * diff[ci * h + y as usize]);
            }
        }
        // a line that leaves the frame for more than a fifth of the width is not a horizon
        if column.len() * 5 < cols.len() * 4 {
            return f64::MIN;
        }
        let i = ((column.len() - 1) as f64 * HORIZON_QUANTILE) as usize;
        column.select_nth_unstable_by(i, f32::total_cmp);
        f64::from(column[i])
    };
    let mut best = (f64::MIN, 0.0f64, 0.0f64, 1.0f32);
    for sign in [1.0f32, -1.0] {
        let mut deg = -HORIZON_SEARCH;
        while deg <= HORIZON_SEARCH {
            let t = deg.to_radians().tan();
            for b in rows.clone().step_by(2) {
                let s = score(t, b as f64, sign);
                if s > best.0 {
                    best = (s, deg, b as f64, sign);
                }
            }
            deg += HORIZON_STEP;
        }
    }
    let (coarse_deg, coarse_b, sign) = (best.1, best.2, best.3);
    let mut deg = coarse_deg - 0.3;
    while deg <= coarse_deg + 0.3 {
        let t = deg.to_radians().tan();
        let mut b = coarse_b - 3.0;
        while b <= coarse_b + 3.0 {
            let s = score(t, b, sign);
            if s > best.0 {
                best = (s, deg, b, sign);
            }
            b += 0.5;
        }
        deg += HORIZON_FINE_STEP;
    }
    let (s, deg) = (best.0 / norm, best.1);
    // a line descending `deg` to the right is levelled by the crop angle −deg
    (s >= HORIZON_MIN_SCORE && deg.abs() <= MAX_DEGREES).then_some(-deg)
}
