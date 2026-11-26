//! Whole-buffer sample-rate conversion with a Kaiser-windowed sinc filter, for files and
//! exports rather than live streams (see [`crate::resample`] for those).
//!
//! The filter has 32 zero crossings either side at the cutoff and a Kaiser window with
//! beta 9. Its cutoff sits at 0.97 of the input's Nyquist frequency when converting up and at
//! 0.94 of the output's when converting down, so content above the new Nyquist frequency is
//! removed. For rates with a small common ratio (44.1 and 48 kHz and their multiples) the
//! kernel is precomputed for every phase, which makes each output sample one dot product over
//! contiguous input; other pairs evaluate the kernel per tap. Both give the same samples.

use std::f64::consts::PI;

use crate::{MAX_CHANNELS, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE};

/// Most frames [`resample`] writes per channel (about 93 minutes at 48 kHz).
pub const MAX_OFFLINE_FRAMES: usize = 1 << 28;

/// Zero crossings of the kernel either side of its center, at the cutoff.
const SINC_ZC: f64 = 32.0;
/// Kernel table entries per zero crossing.
const SINC_RES: usize = 512;
/// Most kernel weights a [`Polyphase`] bank may hold (16 MiB of f32).
const MAX_POLYPHASE_WEIGHTS: usize = 1 << 22;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let y = x * x / 4.0;
    for k in 1..64 {
        term *= y / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-12 {
            break;
        }
    }
    sum
}

/// One side of the windowed-sinc kernel for a cutoff (a fraction of the input's Nyquist
/// frequency), tabulated at [`SINC_RES`] points per zero crossing, so the table has the same
/// size at every cutoff.
struct SincTable {
    table: Vec<f32>,
    cutoff: f64,
    half_width: f64,
}

impl SincTable {
    fn new(cutoff: f64) -> Self {
        // The lowest cutoff `resample` asks for is 1 kHz from 1.536 MHz: 0.94 / 1536.
        let cutoff = if cutoff.is_finite() { cutoff.clamp(1e-4, 1.0) } else { 1.0 };
        let half_width = SINC_ZC / cutoff;
        let n = SINC_ZC as usize * SINC_RES + 2;
        let beta = 9.0;
        let i0b = bessel_i0(beta);
        let table = (0..n)
            .map(|i| {
                // `u` is the distance from the center in zero crossings: input samples × cutoff.
                let u = i as f64 / SINC_RES as f64;
                let x = u / SINC_ZC;
                if x >= 1.0 {
                    return 0.0;
                }
                let arg = PI * u;
                let sinc = if arg.abs() < 1e-12 { 1.0 } else { arg.sin() / arg };
                let w = bessel_i0(beta * (1.0 - x * x).max(0.0).sqrt()) / i0b;
                (cutoff * sinc * w) as f32
            })
            .collect();
        SincTable { table, cutoff, half_width }
    }

    /// The kernel at `t` input samples from its center.
    #[inline]
    fn at(&self, t: f64) -> f32 {
        let pos = t.abs() * self.cutoff * SINC_RES as f64;
        let i = pos as usize;
        let f = (pos - i as f64) as f32;
        match (self.table.get(i), self.table.get(i + 1)) {
            (Some(a), Some(b)) => a + (b - a) * f,
            (Some(a), None) => *a,
            _ => 0.0,
        }
    }
}

/// Input sample `j`, with a non-finite sample read as 0 (one NaN would otherwise spread to
/// every output within the kernel's reach).
#[inline]
fn sample(x: &[f32], j: usize) -> f32 {
    x.get(j).copied().filter(|v| v.is_finite()).unwrap_or(0.0)
}

/// One channel resampled to exactly `out_len` samples; output sample `n` is read at input time
/// `n / ratio`.
fn resample_channel(x: &[f32], ratio: f64, out_len: usize, table: &SincTable) -> Vec<f32> {
    let len = x.len() as i64;
    let hw = table.half_width;
    (0..out_len)
        .map(|n| {
            let t = n as f64 / ratio;
            let lo = (t - hw).ceil().max(0.0) as i64;
            let hi = ((t + hw).floor() as i64).min(len - 1);
            let mut acc = 0.0f32;
            let mut j = lo;
            while j <= hi {
                acc += sample(x, j as usize) * table.at(t - j as f64);
                j += 1;
            }
            acc
        })
        .collect()
}

/// The kernel of [`resample_channel`] precomputed for every phase of a rational rate change
/// `from` to `to` (reduced to `step / phases`). Output sample `n` reads input time
/// `n * step / phases`: whole part `i`, phase `p`. Its taps are input samples `i + first ..`, one
/// weight each. The weights are the ones `resample_channel` computes, taps beyond the kernel's
/// half width weigh 0, and taps are summed in the same order.
struct Polyphase {
    phases: u64,
    step: u64,
    /// Offset of the first tap from `i`.
    first: i64,
    taps: usize,
    /// `phases * taps` weights, phase-major.
    weights: Vec<f32>,
}

impl Polyphase {
    /// `None` when the bank would be too large (rates with no small common ratio); the caller
    /// then uses [`resample_channel`].
    fn new(from: u32, to: u32, table: &SincTable) -> Option<Polyphase> {
        let g = gcd(from, to);
        let (step, phases) = (u64::from(from / g), u64::from(to / g));
        let hw = table.half_width;
        let first = (-hw).floor() as i64;
        let last = hw.ceil() as i64 + 1;
        let taps = usize::try_from(last - first + 1).ok()?;
        if usize::try_from(phases).ok()?.checked_mul(taps)? > MAX_POLYPHASE_WEIGHTS {
            return None;
        }
        let mut weights = Vec::with_capacity(taps * phases as usize);
        for p in 0..phases {
            let frac = p as f64 / phases as f64;
            for k in first..first + taps as i64 {
                let d = frac - k as f64;
                weights.push(if d.abs() <= hw { table.at(d) } else { 0.0 });
            }
        }
        Some(Polyphase { phases, step, first, taps, weights })
    }

    fn run(&self, x: &[f32], out_len: usize) -> Vec<f32> {
        let len = x.len() as i64;
        let clean = x.iter().all(|v| v.is_finite());
        let mut out = Vec::with_capacity(out_len);
        for n in 0..out_len as u64 {
            let Some(pos) = n.checked_mul(self.step) else { break };
            let (i, p) = ((pos / self.phases) as i64, (pos % self.phases) as usize);
            let w = self.weights.get(p * self.taps..(p + 1) * self.taps).unwrap_or_default();
            let start = i + self.first;
            let inside = usize::try_from(start).ok().and_then(|s| x.get(s..s + self.taps));
            let acc = match inside {
                // Inside a finite signal: a plain dot product.
                Some(xs) if clean => xs.iter().zip(w).fold(0.0f32, |a, (v, k)| a + v * k),
                // At the edges taps outside the signal are skipped, as in `resample_channel`.
                _ => (0..self.taps as i64)
                    .filter(|k| (0..len).contains(&(start + k)))
                    .fold(0.0f32, |a, k| a + sample(x, (start + k) as usize) * w.get(k as usize).copied().unwrap_or(0.0)),
            };
            out.push(acc);
        }
        out.resize(out_len, 0.0);
        out
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

fn usable_rate(rate: u32) -> bool {
    (MIN_SAMPLE_RATE as u32..=MAX_SAMPLE_RATE as u32).contains(&rate)
}

/// Converts planar audio from `from` Hz to `to` Hz. Each output channel has
/// `round(len * to / from)` samples, capped at [`MAX_OFFLINE_FRAMES`]; at most
/// [`crate::MAX_CHANNELS`] channels are converted. Equal rates return a copy. `None` when a rate
/// is outside [`crate::MIN_SAMPLE_RATE`] ..= [`crate::MAX_SAMPLE_RATE`]. Non-finite input samples
/// are read as 0.
pub fn resample(channels: &[&[f32]], from: u32, to: u32) -> Option<Vec<Vec<f32>>> {
    if !usable_rate(from) || !usable_rate(to) {
        return None;
    }
    let channels = channels.get(..channels.len().min(MAX_CHANNELS)).unwrap_or_default();
    if from == to {
        let copy = |c: &&[f32]| c.iter().take(MAX_OFFLINE_FRAMES).map(|&v| if v.is_finite() { v } else { 0.0 }).collect();
        return Some(channels.iter().map(copy).collect());
    }
    let ratio = f64::from(to) / f64::from(from);
    let cutoff = if ratio < 1.0 { ratio * 0.94 } else { 0.97 };
    let table = SincTable::new(cutoff);
    let bank = Polyphase::new(from, to, &table);
    Some(
        channels
            .iter()
            .map(|c| {
                let out_len = ((c.len() as f64 * ratio).round() as usize).min(MAX_OFFLINE_FRAMES);
                match &bank {
                    Some(b) => b.run(c, out_len),
                    None => resample_channel(c, ratio, out_len, &table),
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{db, sine, tone_amplitude};

    /// The polyphase bank gives what the per-tap kernel gives, at common rate pairs in both
    /// directions and at the signal's edges.
    #[test]
    fn polyphase_matches_the_per_tap_kernel() {
        let x: Vec<f32> = (0..3_000).map(|i| (i as f32 * 0.071).sin() * 0.6 + (i as f32 * 0.53).sin() * 0.3).collect();
        let pairs = [(44_100, 48_000), (48_000, 44_100), (48_000, 96_000), (96_000, 48_000), (22_050, 48_000), (8_000, 44_100), (48_000, 22_050)];
        for (from, to) in pairs {
            let ratio = f64::from(to) / f64::from(from);
            let out_len = (x.len() as f64 * ratio).round() as usize;
            let cutoff = if ratio < 1.0 { ratio * 0.94 } else { 0.97 };
            let table = SincTable::new(cutoff);
            let reference = resample_channel(&x, ratio, out_len, &table);
            let bank = Polyphase::new(from, to, &table).expect("small common ratio");
            let fast = bank.run(&x, out_len);
            assert_eq!(fast.len(), reference.len(), "{from}->{to}");
            let worst = fast.iter().zip(&reference).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(worst < 1e-5, "{from}->{to}: worst difference {worst}");
            assert_eq!(resample(&[&x], from, to).unwrap()[0], fast, "resample uses the bank");
        }
        // Signals shorter than the kernel are all edge.
        let short = [0.5f32, -0.25, 0.75];
        let table = SincTable::new(0.97);
        let reference = resample_channel(&short, 48_000.0 / 44_100.0, 3, &table);
        let fast = Polyphase::new(44_100, 48_000, &table).expect("bank").run(&short, 3);
        assert!(fast.iter().zip(&reference).all(|(a, b)| (a - b).abs() < 1e-6), "{fast:?} {reference:?}");
        // Rates with no small common ratio use the per-tap kernel.
        assert!(Polyphase::new(44_100, 96_001, &table).is_none());
        assert_eq!(resample(&[&short], 44_100, 96_001).unwrap()[0].len(), 7);
    }

    /// Checked against the signal itself: a tone keeps its frequency and level, and a tone above
    /// the new Nyquist frequency is removed.
    #[test]
    fn tones_keep_frequency_and_level_and_aliases_are_removed() {
        let x = sine(1_000.0, 0.5, 48_000.0, 48_000, 0.0);
        let out = resample(&[&x], 48_000, 44_100).unwrap();
        assert_eq!(out[0].len(), 44_100);
        let a = tone_amplitude(&out[0][2_000..42_000], 1_000.0, 44_100.0);
        assert!(db(a / 0.5).abs() < 0.05, "passband {} dB", db(a / 0.5));
        let up = resample(&[&out[0]], 44_100, 96_000).unwrap();
        assert_eq!(up[0].len(), 96_000);
        assert!(db(tone_amplitude(&up[0][4_000..90_000], 1_000.0, 96_000.0) / 0.5).abs() < 0.05);
        // 23 kHz is above 22.05 kHz's Nyquist frequency of 11.025 kHz.
        let hi = sine(23_000.0, 0.5, 48_000.0, 48_000, 0.0);
        let down = resample(&[&hi], 48_000, 22_050).unwrap();
        let peak = down[0][1_000..20_000].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.005, "alias {peak}");
    }

    /// The widest accepted ratio, 1.536 MHz to 1 kHz: a 100 Hz tone passes and a 5.3 kHz tone,
    /// above the new Nyquist frequency of 500 Hz (it would fold to 300 Hz), is removed.
    #[test]
    fn the_widest_downsampling_ratio_still_removes_aliases() {
        let (from, to) = (1_536_000u32, 1_000u32);
        let low = sine(100.0, 0.5, f64::from(from), 1_536_000, 0.0);
        let out = resample(&[&low], from, to).unwrap();
        assert_eq!(out[0].len(), 1_000);
        let a = tone_amplitude(&out[0][100..900], 100.0, f64::from(to));
        assert!(db(a / 0.5).abs() < 0.1, "passband {} dB", db(a / 0.5));
        let high = sine(5_300.0, 0.5, f64::from(from), 1_536_000, 0.0);
        let out = resample(&[&high], from, to).unwrap();
        let peak = out[0][100..900].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.005, "alias {peak}");
    }

    #[test]
    fn hostile_rates_and_samples() {
        assert!(resample(&[&[1.0; 10]], 0, 48_000).is_none());
        assert!(resample(&[&[1.0; 10]], 48_000, 1).is_none());
        assert!(resample(&[&[1.0; 10]], 48_000, u32::MAX).is_none());
        assert_eq!(resample(&[&[1.0; 10]], 48_000, 48_000).unwrap(), vec![vec![1.0; 10]]);
        assert!(resample(&[], 44_100, 48_000).unwrap().is_empty());
        assert!(resample(&[&[]], 44_100, 48_000).unwrap()[0].is_empty());
        // A NaN sample does not spread through the kernel, on either path.
        let mut x = sine(500.0, 0.5, 48_000.0, 4_800, 0.0);
        x[2_000] = f32::NAN;
        x[3_000] = f32::INFINITY;
        for to in [44_100, 44_101] {
            assert!(resample(&[&x], 48_000, to).unwrap()[0].iter().all(|v| v.is_finite()), "to {to}");
        }
        // Equal rates read non-finite samples as 0 too.
        assert_eq!(resample(&[&[1.0, f32::NAN, f32::NEG_INFINITY]], 48_000, 48_000).unwrap(), vec![vec![1.0, 0.0, 0.0]]);
        let many: Vec<&[f32]> = vec![&[0.0f32; 4]; 100];
        assert_eq!(resample(&many, 44_100, 48_000).unwrap().len(), MAX_CHANNELS);
    }
}
