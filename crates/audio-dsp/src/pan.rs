//! Panning: mono-to-stereo pan laws, a surround panner over any loudspeaker layout of up to
//! [`MAX_SPEAKERS`] speakers, and first-order ambisonics.
//!
//! Directions are in degrees. Azimuth 0 is straight ahead and positive azimuths are to the
//! right (clockwise seen from above); elevation is positive upward. Every function here is
//! allocation-free and returns finite gains for any input, NaN and infinities included.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

/// How much a centered signal is attenuated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PanLaw {
    /// Constant power: -3 dB at center.
    #[default]
    Minus3,
    /// Compromise: -4.5 dB at center.
    Minus4_5,
    /// Constant voltage: -6 dB at center.
    Minus6,
    /// Balance: 0 dB at center, the far side fades out linearly.
    Zero,
}

impl PanLaw {
    /// Center attenuation in dB (0 or negative).
    pub fn center_db(self) -> f32 {
        match self {
            PanLaw::Minus3 => -3.0,
            PanLaw::Minus4_5 => -4.5,
            PanLaw::Minus6 => -6.0,
            PanLaw::Zero => 0.0,
        }
    }
}

/// Left and right gains for `pan` in -1 (hard left) ..= 1 (hard right). NaN pans to center.
pub fn gains(pan: f32, law: PanLaw) -> (f32, f32) {
    let pan = if pan.is_nan() { 0.0 } else { pan.clamp(-1.0, 1.0) };
    if law == PanLaw::Zero {
        return ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0));
    }
    let theta = (pan + 1.0) * FRAC_PI_4;
    let (l, r) = (theta.cos().max(0.0), theta.sin().max(0.0));
    // cos(pi/4) is -3.0103 dB; a power of it reaches the requested center level.
    let exp = law.center_db() / -3.010_3;
    let l = if pan <= -1.0 { 1.0 } else { l.powf(exp) };
    let r = if pan >= 1.0 { 1.0 } else { r.powf(exp) };
    (l, r)
}

/// Most loudspeakers the surround panner handles (9.1.6 is 16).
pub const MAX_SPEAKERS: usize = 16;

/// One loudspeaker of a layout: its direction, and whether it is the LFE (never panned to).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpeakerPos {
    /// Azimuth in degrees, positive to the right.
    pub az: f32,
    /// Elevation in degrees; above 1 degree counts as the height layer.
    pub el: f32,
    pub lfe: bool,
}

/// Surround panner controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurroundParams {
    /// -1 (left) ..= 1 (right).
    pub x: f32,
    /// -1 (back) ..= 1 (front).
    pub y: f32,
    /// 0 (ear level) ..= 1 (overhead).
    pub z: f32,
    /// 0 is a point source ..= 1 is every speaker.
    pub divergence: f32,
    /// 0 ..= 1: the share of center-speaker energy kept on the center; the rest goes to a
    /// phantom center on left and right.
    pub center: f32,
}

fn fin(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

fn wrap180(a: f32) -> f32 {
    let mut a = fin(a, 0.0) % 360.0;
    if a > 180.0 {
        a -= 360.0;
    } else if a <= -180.0 {
        a += 360.0;
    }
    a
}

/// Azimuth of a position on the panner's square. The square's front corners land on the front
/// left and right speakers (30 degrees when the layout has them, else 45), its side midpoints at
/// 90 degrees and its rear corners on the layout's rearmost surround (110 for 5.x, 150 for 7.x,
/// 135 when there is none), so a puck in a corner hits exactly one speaker.
pub fn puck_azimuth(layout: &[SpeakerPos], x: f32, y: f32) -> f32 {
    let (x, y) = (fin(x, 0.0).clamp(-1.0, 1.0), fin(y, 1.0).clamp(-1.0, 1.0));
    if x.abs() < 1e-9 && y.abs() < 1e-9 {
        return 0.0;
    }
    let rear =
        layout.iter().filter(|s| !s.lfe && s.el.abs() < 1.0 && s.az.abs() > 90.5 && s.az.abs() < 179.5).map(|s| s.az.abs()).fold(0.0f32, f32::max);
    let rear = if rear > 0.0 { rear } else { 135.0 };
    let front = if layout.iter().any(|s| !s.lfe && s.el.abs() < 1.0 && (s.az.abs() - 30.0).abs() < 0.5) { 30.0 } else { 45.0 };
    let phi = x.atan2(y).to_degrees();
    let a = phi.abs();
    let warped = if a <= 45.0 {
        a / 45.0 * front
    } else if a <= 90.0 {
        front + (a - 45.0) / 45.0 * (90.0 - front)
    } else if a <= 135.0 {
        90.0 + (a - 90.0) / 45.0 * (rear - 90.0)
    } else {
        rear + (a - 135.0) / 45.0 * (180.0 - rear)
    };
    warped.copysign(phi)
}

/// The point on the square's edge whose [`puck_azimuth`] is `az`, for drawing speakers on a
/// square panner.
pub fn azimuth_to_puck(layout: &[SpeakerPos], az: f32) -> (f32, f32) {
    // Bisect the monotonic warp on the square's angle.
    let target = wrap180(az);
    let (mut lo, mut hi) = (0.0f32, 180.0f32);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        let r = mid.to_radians();
        let w = puck_azimuth(layout, r.sin(), r.cos());
        if w < target.abs() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let phi = (0.5 * (lo + hi)).to_radians().copysign(if target < 0.0 { -1.0 } else { 1.0 });
    let (sx, sy) = (phi.sin(), phi.cos());
    let m = sx.abs().max(sy.abs()).max(1e-6);
    (sx / m, sy / m)
}

/// Constant-power pairwise pan of a direction onto one layer (bed or height) of `layout`,
/// adding `weight * gain^2` into `pow`. Returns `false` when the layer has no speakers.
fn pair_power(layout: &[SpeakerPos], height: bool, az: f32, weight: f32, pow: &mut [f32; MAX_SPEAKERS]) -> bool {
    let mut idx = [0usize; MAX_SPEAKERS];
    let mut n = 0;
    for (i, s) in layout.iter().enumerate().take(MAX_SPEAKERS) {
        if !s.lfe
            && (s.el > 1.0) == height
            && let Some(slot) = idx.get_mut(n)
        {
            *slot = i;
            n += 1;
        }
    }
    let ids = idx.get(..n).unwrap_or(&[]);
    let az_of = |i: usize| layout.get(i).map_or(0.0, |s| wrap180(s.az));
    match ids {
        [] => false,
        [only] => {
            if let Some(p) = pow.get_mut(*only) {
                *p += weight;
            }
            true
        }
        [first, ..] => {
            let a = wrap180(az);
            // The nearest speaker clockwise (increasing azimuth) and the nearest counterclockwise.
            let (mut cw, mut cw_d) = (*first, f32::MAX);
            let (mut acw, mut acw_d) = (*first, f32::MAX);
            for &i in ids {
                let d = (az_of(i) - a).rem_euclid(360.0);
                if d < cw_d {
                    cw_d = d;
                    cw = i;
                }
                let d2 = (a - az_of(i)).rem_euclid(360.0);
                if d2 < acw_d {
                    acw_d = d2;
                    acw = i;
                }
            }
            if cw == acw || cw_d < 1e-4 {
                if let Some(p) = pow.get_mut(cw) {
                    *p += weight;
                }
                return true;
            }
            let span = (cw_d + acw_d).max(1e-6);
            let t = (acw_d / span).clamp(0.0, 1.0) * FRAC_PI_2;
            if let Some(p) = pow.get_mut(acw) {
                *p += weight * t.cos() * t.cos();
            }
            if let Some(p) = pow.get_mut(cw) {
                *p += weight * t.sin() * t.sin();
            }
            true
        }
    }
}

/// Point-source gains for a direction on `layout`: pairwise constant power on the layer
/// nearest in elevation. A height direction on a layout without heights folds to the bed at
/// -3 dB, and an ear-level direction on a layout with only heights goes to the heights at full
/// level, as in [`surround_gains`]. Writes `out[..layout.len()]`.
pub fn direction_gains(layout: &[SpeakerPos], az: f32, el: f32, out: &mut [f32]) {
    let mut pow = [0.0f32; MAX_SPEAKERS];
    let height = fin(el, 0.0) > 1.0;
    if height && !pair_power(layout, true, az, 1.0, &mut pow) {
        pair_power(layout, false, az, 0.5, &mut pow);
    } else if !height && !pair_power(layout, false, az, 1.0, &mut pow) {
        pair_power(layout, true, az, 1.0, &mut pow);
    }
    for (o, p) in out.iter_mut().zip(pow.iter()) {
        *o = p.max(0.0).sqrt();
    }
}

/// Surround panner: a gain for every speaker of `layout` (the LFE gets 0; the caller adds an LFE
/// send). The squared gains of the other speakers always sum to 1.
///
/// The puck's azimuth comes from [`puck_azimuth`]. Divergence, and moving the puck toward the
/// middle of the room, spread the image: the gains are the power average of point sources
/// sampled across `spread * 180` degrees either side. `center` below 1 moves that share of the
/// center speaker's power to a phantom center on left and right. Elevation crossfades the bed
/// and height layers at constant power.
pub fn surround_gains(layout: &[SpeakerPos], p: &SurroundParams, out: &mut [f32]) {
    out.iter_mut().for_each(|g| *g = 0.0);
    let layout = layout.get(..layout.len().min(MAX_SPEAKERS)).unwrap_or(&[]);
    if layout.is_empty() {
        return;
    }
    let az = puck_azimuth(layout, p.x, p.y);
    let dist = fin(p.x, 0.0).abs().max(fin(p.y, 1.0).abs()).clamp(0.0, 1.0);
    let div = fin(p.divergence, 0.0).clamp(0.0, 1.0);
    let spread = 1.0 - dist * (1.0 - div);
    let k = 1 + (spread * 24.0).round() as usize;
    let width = spread * 180.0;
    let has_height = layout.iter().any(|s| !s.lfe && s.el > 1.0);
    let z = if has_height { fin(p.z, 0.0).clamp(0.0, 1.0) } else { 0.0 };
    let (bed_w, top_w) = {
        let a = z * FRAC_PI_2;
        (a.cos() * a.cos(), a.sin() * a.sin())
    };
    let mut pow = [0.0f32; MAX_SPEAKERS];
    for i in 0..k {
        let a = if k == 1 { az } else { az - width + 2.0 * width * (i as f32 + 0.5) / k as f32 };
        let w = 1.0 / k as f32;
        if bed_w > 1e-9 && !pair_power(layout, false, a, w * bed_w, &mut pow) {
            pair_power(layout, true, a, w * bed_w, &mut pow);
        }
        if top_w > 1e-9 {
            pair_power(layout, true, a, w * top_w, &mut pow);
        }
    }
    // Center share: move energy from the center speaker to a phantom center on left and right.
    let find = |az: f32| layout.iter().position(|s| !s.lfe && s.el.abs() < 1.0 && (s.az - az).abs() < 0.5);
    let c_share = fin(p.center, 1.0).clamp(0.0, 1.0);
    if let (Some(c), Some(l), Some(r)) = (find(0.0), find(-30.0), find(30.0))
        && c_share < 1.0
    {
        let pc = pow.get(c).copied().unwrap_or(0.0);
        let moved = pc * (1.0 - c_share);
        if let Some(x) = pow.get_mut(c) {
            *x = pc - moved;
        }
        for i in [l, r] {
            if let Some(x) = pow.get_mut(i) {
                *x += 0.5 * moved;
            }
        }
    }
    for (o, p) in out.iter_mut().zip(pow.iter()) {
        *o = p.max(0.0).sqrt();
    }
}

/// First-order ambisonic encoding gains for a direction, in ACN channel order (W, Y, Z, X) with
/// SN3D normalization, with the directional components scaled by `1 - spread`. Channels past
/// the fourth get 0.
pub fn ambisonic_encode(az: f32, el: f32, spread: f32, out: &mut [f32]) {
    out.iter_mut().for_each(|g| *g = 0.0);
    let (a, e) = (fin(az, 0.0).to_radians(), fin(el, 0.0).to_radians());
    let d = 1.0 - fin(spread, 0.0).clamp(0.0, 1.0);
    // Ambisonic azimuth runs counterclockwise (left positive); this module's runs clockwise.
    let vals = [1.0, -a.sin() * e.cos() * d, e.sin() * d, a.cos() * e.cos() * d];
    for (o, v) in out.iter_mut().zip(vals) {
        *o = v;
    }
}

/// Basic (sampling) first-order decode gains (W, Y, Z, X) for one speaker of an `n`-speaker
/// layout spread evenly over the sphere. Each speaker's feed is the dot product of these gains
/// with the SN3D signals [`ambisonic_encode`] makes.
pub fn ambisonic_decode(az: f32, el: f32, n: usize) -> [f32; 4] {
    let (a, e) = (fin(az, 0.0).to_radians(), fin(el, 0.0).to_radians());
    let k = 1.0 / n.max(1) as f32;
    [k, 3.0 * k * -a.sin() * e.cos(), 3.0 * k * e.sin(), 3.0 * k * a.cos() * e.cos()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gain_to_db;

    #[test]
    fn center_levels() {
        for law in [PanLaw::Minus3, PanLaw::Minus4_5, PanLaw::Minus6, PanLaw::Zero] {
            let (l, r) = gains(0.0, law);
            assert!((gain_to_db(l) - law.center_db()).abs() < 0.02, "{law:?}");
            assert!((l - r).abs() < 1e-6);
            let (l, r) = gains(-1.0, law);
            assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
        }
        assert_eq!(gains(f32::NAN, PanLaw::Minus6), gains(0.0, PanLaw::Minus6));
    }

    fn layout_51() -> Vec<SpeakerPos> {
        [(-30.0, false), (30.0, false), (0.0, false), (0.0, true), (-110.0, false), (110.0, false)]
            .iter()
            .map(|&(az, lfe)| SpeakerPos { az, el: 0.0, lfe })
            .collect()
    }

    fn layout_714() -> Vec<SpeakerPos> {
        let mut v: Vec<SpeakerPos> =
            [-30.0, 30.0, 0.0, 0.0, -150.0, 150.0, -90.0, 90.0].iter().enumerate().map(|(i, &az)| SpeakerPos { az, el: 0.0, lfe: i == 3 }).collect();
        for az in [-45.0, 45.0, -135.0, 135.0] {
            v.push(SpeakerPos { az, el: 45.0, lfe: false });
        }
        v
    }

    fn params(x: f32, y: f32) -> SurroundParams {
        SurroundParams { x, y, z: 0.0, divergence: 0.0, center: 1.0 }
    }

    #[test]
    fn surround_energy_is_constant() {
        for layout in [layout_51(), layout_714()] {
            let mut g = [0.0f32; MAX_SPEAKERS];
            for xi in -4..=4 {
                for yi in -4..=4 {
                    for div in [0.0, 0.3, 1.0] {
                        for z in [0.0, 0.5, 1.0] {
                            for center in [0.0, 0.5, 1.0] {
                                let p = SurroundParams { x: xi as f32 / 4.0, y: yi as f32 / 4.0, z, divergence: div, center };
                                surround_gains(&layout, &p, &mut g);
                                let e: f32 = g.iter().zip(layout.iter()).filter(|(_, s)| !s.lfe).map(|(x, _)| x * x).sum();
                                assert!((e - 1.0).abs() < 1e-4, "energy {e} at {p:?}");
                                assert!(g.iter().zip(layout.iter()).all(|(x, s)| !s.lfe || *x == 0.0), "the LFE is never panned to");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn surround_point_positions() {
        let l = layout_51();
        let mut g = [0.0f32; MAX_SPEAKERS];
        // Front center with no divergence: the center speaker only.
        surround_gains(&l, &params(0.0, 1.0), &mut g);
        assert!((g[2] - 1.0).abs() < 1e-5 && g.iter().enumerate().all(|(i, x)| i == 2 || x.abs() < 1e-3), "{g:?}");
        // Divergence spreads the center image.
        surround_gains(&l, &SurroundParams { divergence: 0.5, ..params(0.0, 1.0) }, &mut g);
        assert!(g[2] < 0.99 && g[0] > 0.05 && g[1] > 0.05);
        // Front-left corner: the left speaker only.
        surround_gains(&l, &params(-1.0, 1.0), &mut g);
        assert!((g[0] - 1.0).abs() < 1e-5 && g.iter().enumerate().all(|(i, x)| i == 0 || x.abs() < 1e-3), "{g:?}");
        // Rear-right corner: the right surround only.
        surround_gains(&l, &params(1.0, -1.0), &mut g);
        assert!((g[5] - 1.0).abs() < 1e-5, "{g:?}");
        // Phantom center (center share 0): left and right at -3 dB, nothing on the center.
        surround_gains(&l, &SurroundParams { center: 0.0, ..params(0.0, 1.0) }, &mut g);
        assert!(g[2].abs() < 1e-6 && (g[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        // Overhead on 7.1.4: the heights only.
        let l = layout_714();
        surround_gains(&l, &SurroundParams { z: 1.0, ..params(-1.0, 1.0) }, &mut g);
        let top: f32 = g[8..12].iter().map(|x| x * x).sum();
        assert!((top - 1.0).abs() < 1e-4 && g[8] > 0.9, "{g:?}");
        // Corner positions map onto speaker angles and back.
        let (x, y) = azimuth_to_puck(&l, -150.0);
        assert!((x + 1.0).abs() < 1e-3 && (y + 1.0).abs() < 1e-3, "{x} {y}");
        assert!((puck_azimuth(&l, 0.0, -1.0).abs() - 180.0).abs() < 1e-3);
        // Hostile input never produces NaN, and an empty layout gives silence.
        surround_gains(&l, &SurroundParams { x: f32::NAN, y: f32::INFINITY, z: f32::NAN, divergence: f32::NAN, center: f32::NAN }, &mut g);
        assert!(g.iter().all(|x| x.is_finite()));
        surround_gains(&[], &params(0.0, 1.0), &mut g);
        assert!(g.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn direction_gains_fold_heights_to_the_bed() {
        let mut g = [0.0f32; MAX_SPEAKERS];
        direction_gains(&layout_51(), 30.0, 45.0, &mut g);
        assert!((g[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5, "{g:?}");
    }

    #[test]
    fn direction_gains_use_the_heights_when_there_is_no_bed() {
        let layout: Vec<SpeakerPos> = [-45.0, 45.0, -135.0, 135.0].iter().map(|&az| SpeakerPos { az, el: 45.0, lfe: false }).collect();
        let mut g = [0.0f32; MAX_SPEAKERS];
        direction_gains(&layout, 45.0, 0.0, &mut g);
        assert!((g[1] - 1.0).abs() < 1e-5, "{g:?}");
        let power: f32 = g.iter().map(|v| v * v).sum();
        assert!((power - 1.0).abs() < 1e-5, "{g:?}");
    }

    #[test]
    fn ambisonics_encode_and_decode_point_at_the_source() {
        // SN3D: an undiffused source has W = 1 and directional components of unit length.
        let mut b = [0.0f32; 6];
        ambisonic_encode(40.0, 20.0, 0.0, &mut b);
        assert_eq!(b[0], 1.0);
        assert!(((b[1] * b[1] + b[2] * b[2] + b[3] * b[3]) - 1.0).abs() < 1e-5);
        assert_eq!(&b[4..], &[0.0, 0.0]);
        // Hard left (-90 degrees here) is +Y in ambisonic coordinates.
        ambisonic_encode(-90.0, 0.0, 0.0, &mut b);
        assert!((b[1] - 1.0).abs() < 1e-6 && b[3].abs() < 1e-6);
        // Full spread leaves only W.
        ambisonic_encode(40.0, 20.0, 1.0, &mut b);
        assert_eq!(&b[..4], &[1.0, 0.0, 0.0, 0.0]);
        // On a cube of 8 speakers the speaker nearest the source gets the most.
        let cube: Vec<(f32, f32)> = [-45.0, 45.0, 135.0, -135.0].iter().flat_map(|&az| [(az, 35.26), (az, -35.26)]).collect();
        ambisonic_encode(50.0, 30.0, 0.0, &mut b);
        let feed = |&(az, el): &(f32, f32)| ambisonic_decode(az, el, cube.len()).iter().zip(&b).map(|(d, s)| d * s).sum::<f32>();
        let feeds: Vec<f32> = cube.iter().map(feed).collect();
        let best = feeds.iter().enumerate().fold((0, f32::MIN), |m, (i, &v)| if v > m.1 { (i, v) } else { m }).0;
        assert_eq!(cube[best], (45.0, 35.26), "{feeds:?}");
        assert!(ambisonic_decode(f32::NAN, f32::INFINITY, 0).iter().all(|v| v.is_finite()));
    }
}
