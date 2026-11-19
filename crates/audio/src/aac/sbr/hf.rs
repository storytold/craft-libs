//! HF generation (ISO/IEC 14496-3 §4.6.18.6) and HF adjustment (§4.6.18.7) of one channel.

use super::bitstream::Grid;
use super::freq::FreqTables;
use super::tables::NOISE;
use super::{Cx, Header, MAX_ENV, T_HF_ADJ, X_SLOTS};

/// Low band subband samples of a frame: slots -8 .. 32 relative to the frame (`X_Low`).
pub(crate) type LowBand = [[Cx; 32]; X_SLOTS];
/// High band subband samples, at slot `l + T_HF_ADJ` for QMF slot `l` of the frame's envelopes.
pub(crate) type HighBand = [[Cx; 64]; X_SLOTS];

/// The per-channel state the HF tools carry from frame to frame.
#[derive(Clone, Debug)]
pub(crate) struct History {
    /// `bs_invf_mode` and chirp factor per noise band.
    invf: [u8; 5],
    bw: [f32; 5],
    /// The last frame's transient envelope was its last (`l_A = L_E`): no noise in this frame's
    /// first.
    la_at_end: bool,
    /// Sinusoids present in the last envelope, by channel above `kx`.
    sine: [bool; 64],
    /// Gains and noise levels of the last four slots, newest first (smoothing).
    gains: [[f32; 64]; 4],
    noise_levels: [[f32; 64]; 4],
    /// The smoothing history holds data of the current tables.
    primed: bool,
    noise_index: usize,
    sine_index: usize,
}

impl History {
    pub(crate) fn new() -> History {
        History {
            invf: [0; 5],
            bw: [0.0; 5],
            la_at_end: false,
            sine: [false; 64],
            gains: [[0.0; 64]; 4],
            noise_levels: [[0.0; 64]; 4],
            primed: false,
            noise_index: 0,
            sine_index: 0,
        }
    }

    /// Forget what depends on the frequency tables (after a header that changes them).
    pub(crate) fn reset_tables(&mut self) {
        self.sine = [false; 64];
        self.primed = false;
        self.noise_index = 0;
    }
}

/// The chirp factor of a noise band from its inverse filtering mode and the last frame's.
fn chirp(mode: u8, prev_mode: u8, prev_bw: f32) -> f32 {
    let new_bw = match (mode, prev_mode) {
        (0, 1) => 0.6,
        (0, _) => 0.0,
        (1, 0) => 0.6,
        (1, _) => 0.75,
        (2, _) => 0.9,
        _ => 0.98,
    };
    let bw = if new_bw < prev_bw { 0.75 * new_bw + 0.25 * prev_bw } else { 0.90625 * new_bw + 0.09375 * prev_bw };
    if bw < 0.015625 { 0.0 } else { bw }
}

/// Second order linear prediction of one low band channel over the frame and the 6 slots
/// before it (the covariance method, §4.6.18.6.2): `(α0, α1)`.
fn predictor(x: &LowBand, k: usize) -> (Cx, Cx) {
    // φ(i, j) = Σ x(n − i + 2) · conj(x(n − j + 2)), n = 0 .. 37
    let phi = |i: usize, j: usize| {
        let (mut re, mut im) = (0f64, 0f64);
        for n in 0..SLOTS_PLUS_6 {
            let (a, b) = (x[n + 2 - i][k], x[n + 2 - j][k]);
            re += f64::from(a.re) * f64::from(b.re) + f64::from(a.im) * f64::from(b.im);
            im += f64::from(a.im) * f64::from(b.re) - f64::from(a.re) * f64::from(b.im);
        }
        (re, im)
    };
    let (p01, p02, p11, p12, p22) = (phi(0, 1), phi(0, 2), phi(1, 1).0, phi(1, 2), phi(2, 2).0);
    let d = p22 * p11 - (p12.0 * p12.0 + p12.1 * p12.1) / (1.0 + 1e-6);
    let a1 = if d != 0.0 {
        // (φ01 · φ12 − φ02 · φ11) / d
        ((p01.0 * p12.0 - p01.1 * p12.1 - p02.0 * p11) / d, (p01.0 * p12.1 + p01.1 * p12.0 - p02.1 * p11) / d)
    } else {
        (0.0, 0.0)
    };
    let a0 = if p11 != 0.0 {
        // −(φ01 + α1 · conj(φ12)) / φ11
        (-(p01.0 + a1.0 * p12.0 + a1.1 * p12.1) / p11, -(p01.1 + a1.1 * p12.0 - a1.0 * p12.1) / p11)
    } else {
        (0.0, 0.0)
    };
    let mag = |a: (f64, f64)| (a.0 * a.0 + a.1 * a.1).sqrt();
    if mag(a0) >= 4.0 || mag(a1) >= 4.0 {
        return (Cx::default(), Cx::default());
    }
    (Cx { re: a0.0 as f32, im: a0.1 as f32 }, Cx { re: a1.0 as f32, im: a1.1 as f32 })
}

/// Slots of the covariance sums: the frame's 32 and 6 before it.
const SLOTS_PLUS_6: usize = super::SLOTS + 6;

/// The index of the band of `bands` holding channel `k` (bands are increasing borders).
fn band_of(bands: &[usize], k: usize) -> usize {
    bands.partition_point(|&b| b <= k).saturating_sub(1).min(bands.len().saturating_sub(2))
}

/// Patch the low band into the SBR range, inverse filtered (§4.6.18.6.3), for QMF slots
/// `from .. to` of the frame.
pub(crate) fn generate(x: &LowBand, t: &FreqTables, invf: &[u8; 5], h: &mut History, (from, to): (usize, usize), out: &mut HighBand) {
    let mut bw = [0f32; 5];
    for i in 0..t.n_q() {
        bw[i] = chirp(invf[i], h.invf[i], h.bw[i]);
    }
    (h.invf, h.bw) = (*invf, bw);
    let mut pred: [Option<(Cx, Cx)>; 32] = [None; 32];
    let mut k = t.kx;
    for &(start, count) in &t.patches {
        for p in start..start + count {
            let (a0, a1) = *pred[p].get_or_insert_with(|| predictor(x, p));
            let g = bw[band_of(&t.noise, k)];
            let (c0, c1) = (a0.scale(g), a1.scale(g * g));
            for l in from..to {
                let i = l + T_HF_ADJ;
                out[i][k] = x[i][p] + c0 * x[i - 1][p] + c1 * x[i - 2][p];
            }
            k += 1;
        }
    }
}

/// Dequantised envelope and noise floor values of one channel in one frame.
#[derive(Clone, Debug)]
pub(crate) struct Levels {
    /// Envelope energy per envelope and band (of that envelope's resolution).
    pub(crate) energy: [[f32; 64]; MAX_ENV],
    /// Noise to signal ratio per noise floor and noise band.
    pub(crate) noise: [[f32; 5]; 2],
}

/// Gains for one envelope, per channel of the SBR range.
struct EnvelopeGains {
    gain: [f32; 64],
    noise: [f32; 64],
    sine: [f32; 64],
}

const LIMITER_GAINS: [f32; 4] = [0.70795, 1.0, 1.41254, 1e10];
/// `h_smooth`, as the specification gives it.
#[allow(clippy::excessive_precision)]
const SMOOTHING: [f32; 5] = [0.33333333333333, 0.30150283239582, 0.21816949906249, 0.11516383427084, 0.03183050093751];
const EPS: f32 = 1.0;
const EPS0: f32 = 1e-12;

/// What the adjustment of one channel's frame works from.
pub(crate) struct Frame<'a> {
    /// The generated high band.
    pub(crate) x: &'a HighBand,
    pub(crate) t: &'a FreqTables,
    pub(crate) hdr: &'a Header,
    pub(crate) g: &'a Grid,
    pub(crate) lv: &'a Levels,
    /// `bs_add_harmonic`, when the frame codes sinusoids.
    pub(crate) harmonic: Option<&'a [bool; 64]>,
}

/// The gains of envelope `l` (§4.6.18.7.3–4): `delta` is 0 for a transient envelope, which gets
/// no noise.
fn envelope_gains(f: &Frame, l: usize, sine_index: &[bool; 64], delta: bool) -> EnvelopeGains {
    let Frame { x, t, hdr, g, lv, .. } = *f;
    let (kx, m) = (t.kx, t.m);
    let bands = t.bands(g.freq_res[l]);
    let (from, to) = (2 * g.t_env[l] + T_HF_ADJ, 2 * g.t_env[l + 1] + T_HF_ADJ);
    let slots = (to - from) as f32;
    let floor = usize::from(g.num_noise == 2 && g.t_env[l] >= g.t_noise[1]);
    // per channel: original energy, current energy, noise ratio, sinusoid in its band
    let mut e_orig = [0f32; 64];
    let mut e_curr = [0f32; 64];
    let mut q = [0f32; 64];
    let mut s_band = [false; 64];
    for j in 0..bands.len() - 1 {
        let (lo, hi) = (bands[j] - kx, bands[j + 1] - kx);
        let any_sine = sine_index[lo..hi].iter().any(|&s| s);
        let mut sum = 0f32;
        for c in lo..hi {
            let ch = c + kx;
            let e: f32 = (from..to).map(|i| x[i][ch].norm_sqr()).sum::<f32>() / slots;
            e_curr[c] = e;
            sum += e;
            e_orig[c] = lv.energy[l][j];
            q[c] = lv.noise[floor][band_of(&t.noise, ch)];
            s_band[c] = any_sine;
        }
        if !hdr.interpol_freq {
            e_curr[lo..hi].fill(sum / (hi - lo) as f32);
        }
    }
    let mut out = EnvelopeGains { gain: [0.0; 64], noise: [0.0; 64], sine: [0.0; 64] };
    for c in 0..m {
        // `q / (1 + q)` first: energy and noise ratio are each clamped, their product is not
        let (eo, ec, q) = (e_orig[c], e_curr[c], q[c]);
        let share = q / (1.0 + q);
        out.noise[c] = (eo * share).sqrt();
        if sine_index[c] {
            out.sine[c] = (eo / (1.0 + q)).sqrt();
        }
        out.gain[c] = if s_band[c] { (eo / (EPS + ec) * share).sqrt() } else { (eo / ((EPS + ec) * (1.0 + if delta { q } else { 0.0 }))).sqrt() };
    }
    // limit per limiter band, then boost back the energy the limiter took
    let lim_gain = LIMITER_GAINS[usize::from(hdr.limiter_gains & 3)];
    for w in t.lim.windows(2) {
        let (lo, hi) = (w[0] - kx, w[1] - kx);
        let (so, sc) = (e_orig[lo..hi].iter().sum::<f32>(), e_curr[lo..hi].iter().sum::<f32>());
        let max = (lim_gain * ((EPS0 + so) / (EPS0 + sc)).sqrt()).min(1e5);
        let mut den = 0f32;
        for c in lo..hi {
            if out.gain[c] > max {
                out.noise[c] *= max / out.gain[c];
                out.gain[c] = max;
            }
            den += e_curr[c] * out.gain[c] * out.gain[c];
            if out.sine[c] != 0.0 {
                den += out.sine[c] * out.sine[c];
            } else if delta {
                den += out.noise[c] * out.noise[c];
            }
        }
        let boost = ((EPS0 + so) / (EPS0 + den)).sqrt().min(1.584_893_2);
        for c in lo..hi {
            out.gain[c] *= boost;
            out.noise[c] *= boost;
            out.sine[c] *= boost;
        }
    }
    out
}

/// Adjust the generated high band to the transmitted envelopes and add noise and sinusoids
/// (§4.6.18.7), for the frame's envelopes, into `y`.
pub(crate) fn adjust(f: &Frame, h: &mut History, y: &mut HighBand) {
    let Frame { x, t, hdr, g, harmonic, .. } = *f;
    let (kx, m) = (t.kx, t.m);
    let smooth = !hdr.smoothing_mode;
    let prev_la_at_end = h.la_at_end;
    for l in 0..g.num_env {
        // sinusoids start at the transient envelope, unless one was already playing in the band
        let mut sine_index = [false; 64];
        if let Some(harm) = harmonic {
            for i in 0..t.n_high() {
                let c = (t.high[i] + t.high[i + 1]) / 2 - kx;
                sine_index[c] = harm[i] && (g.la.is_none_or(|la| l >= la) || h.sine[c]);
            }
        }
        let transient = g.la == Some(l) || (l == 0 && prev_la_at_end);
        let e = envelope_gains(f, l, &sine_index, !transient);
        if !h.primed {
            h.gains = [e.gain; 4];
            h.noise_levels = [e.noise; 4];
            h.primed = true;
        }
        for i in 2 * g.t_env[l]..2 * g.t_env[l + 1] {
            let (mut gain, mut noise) = (e.gain, e.noise);
            if smooth && !transient {
                for c in 0..m {
                    gain[c] = SMOOTHING[0] * e.gain[c] + (0..4).map(|j| SMOOTHING[j + 1] * h.gains[j][c]).sum::<f32>();
                    noise[c] = SMOOTHING[0] * e.noise[c] + (0..4).map(|j| SMOOTHING[j + 1] * h.noise_levels[j][c]).sum::<f32>();
                }
            }
            h.gains.rotate_right(1);
            h.noise_levels.rotate_right(1);
            (h.gains[0], h.noise_levels[0]) = (e.gain, e.noise);
            let (sin_re, sin_im) = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)][h.sine_index];
            let row = &mut y[i + T_HF_ADJ];
            for c in 0..m {
                let ch = c + kx;
                let mut v = x[i + T_HF_ADJ][ch].scale(gain[c]);
                h.noise_index = (h.noise_index + 1) & 511;
                if e.sine[c] != 0.0 {
                    let sign = if ch % 2 == 0 { 1.0 } else { -1.0 };
                    v.re += e.sine[c] * sin_re;
                    v.im += e.sine[c] * sin_im * sign;
                } else if !transient {
                    let [nr, ni] = NOISE[h.noise_index];
                    v.re += noise[c] * nr;
                    v.im += noise[c] * ni;
                }
                row[ch] = v;
            }
            h.sine_index = (h.sine_index + 1) & 3;
        }
        if l + 1 == g.num_env {
            h.sine = sine_index;
        }
    }
    h.la_at_end = g.la == Some(g.num_env);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chirp factor follows §4.6.18.6.2: rising quickly, falling slowly, and zero below 1/64.
    #[test]
    fn chirp_factors() {
        assert!((chirp(3, 3, 0.0) - 0.90625 * 0.98).abs() < 1e-6);
        assert!((chirp(0, 3, 0.98) - 0.25 * 0.98).abs() < 1e-6);
        assert_eq!(chirp(0, 0, 0.05), 0.0);
        assert!((chirp(1, 0, 0.0) - 0.90625 * 0.6).abs() < 1e-6);
    }

    /// A channel that is a decaying complex exponential is predicted exactly by its first order
    /// term.
    #[test]
    fn prediction_of_a_resonance() {
        let mut x: LowBand = [[Cx::default(); 32]; X_SLOTS];
        let r = Cx { re: 0.9 * 0.3f32.cos(), im: 0.9 * 0.3f32.sin() };
        let mut v = Cx { re: 1.0, im: 0.0 };
        for slot in x.iter_mut() {
            slot[3] = v;
            v = v * r;
        }
        let (a0, a1) = predictor(&x, 3);
        // x(n) + α0·x(n−1) + α1·x(n−2) = 0 with α0 = −r, α1 = 0
        assert!((a0.re + r.re).abs() < 1e-3 && (a0.im + r.im).abs() < 1e-3, "{a0:?}");
        assert!(a1.re.abs() < 1e-3 && a1.im.abs() < 1e-3, "{a1:?}");
        // a silent channel has no predictor
        assert_eq!(predictor(&x, 4), (Cx::default(), Cx::default()));
    }
}
