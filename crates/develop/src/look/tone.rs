use super::look_fit::dro_off;
use super::pairs::{Pairs, displayed, luma};
use super::{CameraTone, Rgb32f, ToneMap, luminance_2020};
#[allow(unused_imports)]
use crate::sensor::prelude::*;

/// How the per-photo tone curve is fitted ([`fit_pairs_ordered`] tries them in order).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ToneFit {
    /// [`fit_tone`]: no flat steps, no pixel-exact registration needed.
    Quantile,
    /// [`fit_tone_conditional`]: the earlier fit, kept as the fallback.
    ConditionalMedian,
}

impl ToneFit {
    pub(super) fn fit(self, pairs: Vec<(f64, f64)>) -> Option<CameraTone> {
        match self {
            ToneFit::Quantile => fit_tone(pairs),
            ToneFit::ConditionalMedian => fit_tone_conditional(pairs),
        }
    }
}

/// The bright areas a narrow tone curve is checked against ([`highlights_hold`]): every proxy pixel
/// whose camera JPEG luminance is 0.85 or more, with its sensor value as developed (for a clipped
/// pixel a lower bound of the scene). Taken from the proxies before any training filter (clipped
/// pixels, edges, the sensor value range of [`collect_pairs`]), so no retry of the search can
/// leave them out.
pub(super) fn highlights(sensor: &Rgb32f, reference: &Rgb32f) -> Pairs {
    if sensor.data.len() != reference.data.len() {
        return Vec::new();
    }
    sensor
        .data
        .iter()
        .zip(&reference.data)
        .filter(|(x, y)| x.iter().chain(y.iter()).all(|v| v.is_finite() && *v >= 0.0) && luminance_2020(**y) >= 0.85)
        .map(|(x, y)| (x.map(f64::from), y.map(f64::from)))
        .collect()
}

/// Smallest ratio between the scene luminance of the brightest and of the darkest 1/32 of the
/// pairs a tone curve is fitted to (its last and first knot). Below it the training pairs are
/// nearly uniform (under 0.26 EV from their darker to their brighter parts): the curve would be
/// fitted to a sliver of the tone range and the rest of it extrapolated. Above it the acceptance
/// gates judge the curve. This was 1.5, which rejected low-contrast scenes the curve fits very
/// well: in a hazy ILCE-7RM4 burst the frames at 1.46–1.49 opened with the neutral fallback (no full or partial
/// look, 18 L* darker than the camera JPEG) while the frame at 1.51 fitted at a held-out RMS of
/// 0.013, as the others do once allowed (issue storytold/lightcraft#631). Across 135 public Sony files the lowest is 4.0.
/// A curve under [`NARROW_TONE_RANGE`] must also pass [`highlights_hold`].
const MIN_TONE_RANGE: f32 = 1.2;

/// [`MIN_TONE_RANGE`] before issue storytold/lightcraft#631. A curve fitted to a narrower range than this must also
/// render the photo's bright areas no further from the camera JPEG than the neutral fallback does
/// ([`highlights_hold`]). Neither the fit nor the held-out gates see those areas (camera JPEG
/// luminance 0.85 and up, left out of the training pairs by [`collect_pairs`]), and a curve fitted
/// to a sliver of the tone range extrapolates them: in review, a synthetic frame whose curve passed
/// the gates at a held-out RMS of 0.0004 rendered its bright sky at 0.29 where the camera JPEG has
/// 0.93.
const NARROW_TONE_RANGE: f32 = 1.5;

/// What a narrow tone curve is checked against ([`highlights_hold`]).
pub(super) struct HighlightCheck {
    /// The frame's [`highlights`].
    pub(super) pairs: Pairs,
    /// The photo's curve will be lowered to Sony's without DRO ([`with_dro_off_tone`]) after the
    /// fit, so the check looks at it lowered.
    pub(super) dro: bool,
    /// The camera JPEG is black and white ([`fit_tone_only_with`]): the check compares luminance, as that
    /// fit's gates do, since the JPEG has no colour to match.
    pub(super) grey: bool,
}

/// Whether a look with this `tone` and `colour` (sensor value to the finish stage's input) keeps
/// the frame's [`highlights`] (camera JPEG luminance 0.85 and up, left out of the fit) no further
/// from the camera JPEG than the neutral fallback does, with the tone curve as the photo will get
/// it (lowered for DRO), in luminance alone for a black-and-white JPEG. Only asked of curves narrower than
/// [`NARROW_TONE_RANGE`]: true for wider ones, and for frames without such pixels.
pub(super) fn highlights_hold(tone: &CameraTone, check: &HighlightCheck, colour: impl Fn([f64; 3]) -> [f64; 3]) -> bool {
    let knots = tone.knots();
    if knots[31][0] >= knots[0][0] * NARROW_TONE_RANGE {
        return true;
    }
    // the curve as the photo will get it
    let used = if check.dro { dro_off(tone) } else { *tone };
    let (map, neutral) = (ToneMap::camera(&used, 0.0, 0.0, 0.0), ToneMap::new(0.0, 0.0, 0.0));
    let (mut look, mut fallback) = (0.0, 0.0);
    for (x, target) in &check.pairs {
        let (rendered, base) = (displayed(colour(*x), &map), displayed(*x, &neutral));
        if check.grey {
            look += (luma(rendered) - luma(*target)).powi(2);
            fallback += (luma(base) - luma(*target)).powi(2);
            continue;
        }
        for c in 0..3 {
            look += (rendered[c] - target[c]).powi(2);
            fallback += (base[c] - target[c]).powi(2);
        }
    }
    let hold = look.is_finite() && look <= fallback;
    if !hold && crate::profiling() {
        eprintln!("[profile] camera look: narrow tone curve renders the bright areas worse than the fallback ({look:.3} > {fallback:.3})");
    }
    hold
}

/// The camera's tone curve from scene/JPEG luminance pairs, by quantile matching: knot `i` pairs
/// the median of the `i`-th 1/32 of the sorted scene luminances with the median of the `i`-th
/// 1/32 of the sorted JPEG luminances. A tone curve is monotone, so it maps each quantile of the
/// scene to the same quantile of the JPEG; matching the two distributions needs no pixel-exact
/// registration and is increasing by construction.
///
/// Binning the pairs by scene luminance and taking the JPEG's median per bin instead (followed by
/// isotonic pooling) fails on real files: camera JPEGs are lens-corrected and sharpened, so on a
/// 96 px proxy of a busy scene neighbouring pairs disagree by more than the gap between bins.
/// The conditional medians then zig-zag, pooling turns them into flat steps (13 of 31 segments
/// on a Z 6II forest, issue storytold/lightcraft#475) that render as posterised patches, and regression toward the
/// mean flattens both ends of the curve (lifted blacks, dull highlights).
pub(super) fn fit_tone(pairs: Vec<(f64, f64)>) -> Option<CameraTone> {
    if pairs.len() < 128 || !pairs.iter().all(|(x, y)| x.is_finite() && *x > 0.0 && y.is_finite()) {
        return None;
    }
    let mut xs: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let mut ys: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    let n = xs.len();
    let mut knots = [[0.0; 2]; 32];
    for (i, knot) in knots.iter_mut().enumerate() {
        let (a, b) = (i * n / 32, (i + 1) * n / 32);
        let mid = a + (b - a) / 2;
        *knot = [*xs.get(mid)? as f32, *ys.get(mid)? as f32];
    }
    if knots[31][0] < knots[0][0] * MIN_TONE_RANGE {
        return None;
    }
    CameraTone::new(limit_elasticity(knots, &pairs)?)
}

/// Largest log-log slope (elasticity, d ln y / d ln x with [`ELASTICITY_BLACK`] added to y) a fitted tone curve may
/// have between two knots. A much steeper segment is not a camera curve but two pixel populations the camera renders at
/// different brightness for the same scene luminance (by hue): quantile matching puts all of one population's scene
/// range below the other's JPEG range and joins them with a near-vertical step, which amplifies the luminance noise of
/// every pixel on it and renders as contours and blotches. On a dark ISO 1100 D3400 frame of saturated blue and red
/// (issue storytold/lightcraft#1904; reds 1.55x brighter in the JPEG than blues of the same scene luminance) the step had elasticity 12-28,
/// and the render's fine luminance noise in the blue area was 1.46x that of Adobe's DNG of the same frame; with the
/// bound it is 0.99x. Of 514 fitted looks on the user samples the median curve's steepest segment is 2.1 and the 95th
/// percentile 3.6. The smoothed window ends where its end-to-end slope is first under the bound, so a looser bound
/// (5 or 6) only spreads the D3400 step over a few knots and leaves the noise as it was.
pub(super) const MAX_TONE_ELASTICITY: f64 = 4.0;
/// Log-log slope of a curve end left free by [`limit_elasticity`]: about the median steepest segment of fitted looks
/// (2.1), so the knots past the step follow an ordinary camera curve. At the bound itself (4) the D3400 frame's fine
/// luminance noise stays 1.2x Adobe's; at 2 it is 0.99x.
pub(super) const TONE_REPAIR_ELASTICITY: f64 = 2.0;
/// Display level added to both ends of a segment's elasticity, so a toe rising from black is not counted as infinitely
/// steep.
pub(super) const ELASTICITY_BLACK: f64 = 0.002;

/// `knots` with every segment steeper than [`MAX_TONE_ELASTICITY`] smoothed. The knots inside the smallest window
/// around the segment (widened a knot on each side at a time) whose end-to-end elasticity is under the bound move onto
/// the straight log-log line between the window's ends, so the step is spread over the scene range of the populations it
/// joined. A window that reaches an end of the curve is pinned there, which can drag most of the curve onto one line
/// (a step at the top of a Coolpix P340 curve pulled knots 3-29 up, held-out error +4 dE00), so it is weighed against
/// leaving that end free: the knots past the segment on that side move onto [`TONE_REPAIR_ELASTICITY`] from the
/// segment's other knot instead. Of the two the one closer to `pairs` (absolute error) is kept. The curve outside the
/// window and its monotonicity are kept. `None` when nothing brings the curve under the bound. Knots within the bound are returned as
/// they are.
#[allow(clippy::needless_range_loop)] // the index addresses several arrays at once
pub(super) fn limit_elasticity(mut knots: [[f32; 2]; 32], pairs: &[(f64, f64)]) -> Option<[[f32; 2]; 32]> {
    let (bound, repair) = (MAX_TONE_ELASTICITY, TONE_REPAIR_ELASTICITY);
    let at = |k: &[[f32; 2]; 32], i: usize| (f64::from(k[i][0]).ln(), (f64::from(k[i][1]) + ELASTICITY_BLACK).ln());
    let slope = |k: &[[f32; 2]; 32], a: usize, b: usize| {
        let ((ua, va), (ub, vb)) = (at(k, a), at(k, b));
        (vb - va) / (ub - ua)
    };
    let level = |v: f64| (v.exp() - ELASTICITY_BLACK).max(0.0) as f32;
    let error = |k: &[[f32; 2]; 32]| {
        let tone = CameraTone::new(*k)?;
        Some(pairs.iter().map(|&(x, y)| (f64::from(tone.apply(x as f32)) - y).abs()).sum::<f64>())
    };
    for _ in 0..64 {
        let Some(i) = (0..31).find(|&i| slope(&knots, i, i + 1) > bound) else { return Some(knots) };
        let (mut lo, mut hi) = (i, i + 1);
        while slope(&knots, lo, hi) > bound && !(lo == 0 && hi == 31) {
            lo = lo.saturating_sub(1);
            hi = (hi + 1).min(31);
        }
        let mut pinned = knots;
        let pinned_ok = slope(&knots, lo, hi) <= bound;
        if pinned_ok {
            let ((ua, va), (ub, vb)) = (at(&knots, lo), at(&knots, hi));
            for k in lo + 1..hi {
                pinned[k][1] = level(va + (vb - va) * (f64::from(knots[k][0]).ln() - ua) / (ub - ua));
            }
        }
        // A window that reaches an end of the curve can leave that end free instead: the knots below the segment rise
        // onto the repair slope down from knot i + 1, or the knots above it drop onto it up from knot i.
        let mut free = None;
        if lo == 0 || hi == 31 {
            let mut k2 = knots;
            if lo == 0 {
                let (ub, vb) = at(&knots, i + 1);
                for k in 0..=i {
                    let (u, v) = at(&knots, k);
                    k2[k][1] = level(v.max(vb - repair * (ub - u)));
                }
            } else {
                let (ua, va) = at(&knots, i);
                for k in i + 1..32 {
                    let (u, v) = at(&knots, k);
                    k2[k][1] = level(v.min(va + repair * (u - ua)));
                }
            }
            free = Some(k2);
        }
        let next = match (pinned_ok, free) {
            (true, Some(free)) => {
                if error(&free).unwrap_or(f64::INFINITY) < error(&pinned).unwrap_or(f64::INFINITY) {
                    free
                } else {
                    pinned
                }
            }
            (true, None) => pinned,
            (false, Some(free)) => free,
            (false, None) => return None,
        };
        if crate::profiling() {
            let moved: Vec<usize> = (0..32).filter(|&k| next[k][1] != knots[k][1]).collect();
            eprintln!("[profile] camera tone: elasticity {:.1} at scene {:.4}, knots {moved:?} moved", slope(&knots, i, i + 1), knots[i][0]);
        }
        if next == knots {
            return None;
        }
        knots = next;
    }
    None
}

fn median(values: &mut [f64]) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

/// The earlier tone fit, kept as the fallback of [`ToneFit`]: the JPEG's median per equal-population
/// bin of scene luminance, then pooled-adjacent-violators isotonic regression. It follows the
/// per-pixel pairs closely when the two images are registered, which is why it can clear the
/// acceptance gates where [`fit_tone`] just misses them, but on lens-corrected JPEGs of busy scenes
/// the pooling leaves flat steps (issue storytold/lightcraft#475).
pub(super) fn fit_tone_conditional(mut pairs: Vec<(f64, f64)>) -> Option<CameraTone> {
    if pairs.len() < 128 || !pairs.iter().all(|(x, y)| x.is_finite() && *x > 0.0 && y.is_finite()) {
        return None;
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut knots = [[0.0; 2]; 32];
    for (i, knot) in knots.iter_mut().enumerate() {
        let bin = pairs.get(i * pairs.len() / 32..(i + 1) * pairs.len() / 32)?;
        let mut xs: Vec<_> = bin.iter().map(|p| p.0).collect();
        let mut ys: Vec<_> = bin.iter().map(|p| p.1).collect();
        *knot = [median(&mut xs)? as f32, median(&mut ys)? as f32];
    }
    if knots[31][0] < knots[0][0] * MIN_TONE_RANGE {
        return None;
    }
    // Pool adjacent violating bins (isotonic regression): no reversals or arbitrary polynomial.
    let mut blocks: Vec<(f32, usize)> = Vec::new();
    for knot in knots {
        blocks.push((knot[1], 1));
        while blocks.len() >= 2 {
            let (a, an) = *blocks.get(blocks.len() - 2)?;
            let (b, bn) = *blocks.last()?;
            if a <= b {
                break;
            }
            blocks.truncate(blocks.len() - 2);
            blocks.push(((a * an as f32 + b * bn as f32) / (an + bn) as f32, an + bn));
        }
    }
    let mut i = 0;
    for (y, n) in blocks {
        for knot in knots.get_mut(i..i + n)? {
            knot[1] = y;
        }
        i += n;
    }
    CameraTone::new(limit_elasticity(knots, &pairs)?)
}
