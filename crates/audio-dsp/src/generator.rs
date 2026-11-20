//! Test signals: band-limited sine, square and saw (PolyBLEP) and triangle (PolyBLAMP)
//! oscillators, and white and pink noise.
//!
//! [`Oscillator`] produces one sample per call and never allocates, for real-time use (the
//! `signal_generator` effect). [`signal`] renders a whole buffer offline.

use std::f32::consts::TAU;

use crate::{MAX_CHANNELS, db_to_gain, sane_sample_rate};

/// Most samples [`signal`] renders over all channels (256 MiB of `f32`; about 11 minutes of
/// stereo at 48 kHz).
pub const MAX_SIGNAL_SAMPLES: usize = 1 << 26;

/// Test-signal waveforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Waveform {
    Sine,
    Square,
    Saw,
    Triangle,
    WhiteNoise,
    PinkNoise,
}

impl Waveform {
    pub const ALL: [Waveform; 6] = [Waveform::Sine, Waveform::Square, Waveform::Saw, Waveform::Triangle, Waveform::WhiteNoise, Waveform::PinkNoise];

    /// The waveform for a choice index; an index out of range gives [`Waveform::Sine`].
    pub fn from_index(i: usize) -> Waveform {
        Self::ALL.get(i).copied().unwrap_or(Waveform::Sine)
    }
}

#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        0.0
    } else if t < dt {
        let t = t / dt;
        2.0 * t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// PolyBLAMP residual of a unit change of slope per sample at phase 0: `t` is the phase since the
/// corner (0..1), `dt` the phase step per sample.
#[inline]
fn poly_blamp(t: f32, dt: f32) -> f32 {
    let cube = |x: f32| x * x * x / 6.0;
    if dt <= 0.0 {
        0.0
    } else if t < dt {
        cube(1.0 - t / dt)
    } else if t > 1.0 - dt {
        cube(1.0 - (1.0 - t) / dt)
    } else {
        0.0
    }
}

/// Xorshift32 noise source: deterministic for a seed, and it does not allocate.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Noise(u32);

impl Noise {
    pub(crate) fn new(seed: u32) -> Self {
        Noise(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    #[inline]
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform white noise in -1..1.
    #[inline]
    pub(crate) fn bipolar(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0
    }
}

/// Pink noise: white noise through a sum of first-order low-passes (the "economy" pinking
/// filter), within about 0.5 dB of a -3 dB per octave slope from 10 Hz to 20 kHz at 48 kHz.
#[derive(Clone, Copy, Debug, Default)]
struct Pinking {
    b: [f32; 7],
}

impl Pinking {
    #[inline]
    fn next(&mut self, white: f32) -> f32 {
        let b = &mut self.b;
        b[0] = 0.99886 * b[0] + white * 0.055_517_9;
        b[1] = 0.99332 * b[1] + white * 0.075_075_9;
        b[2] = 0.96900 * b[2] + white * 0.153_852;
        b[3] = 0.86650 * b[3] + white * 0.310_485_6;
        b[4] = 0.55000 * b[4] + white * 0.532_952_2;
        b[5] = -0.7616 * b[5] - white * 0.016_898;
        let out = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + white * 0.5362;
        b[6] = white * 0.115_926;
        out * 0.11
    }
}

/// Phase-accumulator oscillator. Periodic waveforms peak at 1; white noise is uniform in -1..1
/// and pink noise stays within -1..1.
#[derive(Clone, Copy, Debug)]
pub struct Oscillator {
    phase: f32,
    seed: u32,
    noise: Noise,
    pink: Pinking,
}

impl Oscillator {
    /// An oscillator at phase 0. `seed` picks the noise sequence.
    pub fn new(seed: u32) -> Self {
        Oscillator { phase: 0.0, seed, noise: Noise::new(seed), pink: Pinking::default() }
    }

    /// Back to phase 0 and the start of the noise sequence.
    pub fn reset(&mut self) {
        *self = Oscillator::new(self.seed);
    }

    /// One sample of `wave`; `step` is frequency divided by sample rate, clamped to 0..0.49
    /// (a non-finite step holds the phase).
    #[inline]
    pub fn next(&mut self, wave: Waveform, step: f32) -> f32 {
        let dt = if step.is_finite() { step.clamp(0.0, 0.49) } else { 0.0 };
        let t = self.phase;
        let out = match wave {
            Waveform::Sine => (t * TAU).sin(),
            Waveform::Saw => 2.0 * t - 1.0 - poly_blep(t, dt),
            Waveform::Square => {
                let naive = if t < 0.5 { 1.0 } else { -1.0 };
                let t2 = if t + 0.5 >= 1.0 { t - 0.5 } else { t + 0.5 };
                naive + poly_blep(t, dt) - poly_blep(t2, dt)
            }
            Waveform::Triangle => {
                // Starts at 0 and rises, like the sine.
                let u = t + 0.25;
                let u = if u >= 1.0 { u - 1.0 } else { u };
                // The slope changes by -8·dt per sample at the peak (u = 0.5) and by +8·dt at
                // the trough (u = 0); PolyBLAMP rounds both corners.
                let since_peak = if u >= 0.5 { u - 0.5 } else { u + 0.5 };
                1.0 - 4.0 * (u - 0.5).abs() + 8.0 * dt * (poly_blamp(u, dt) - poly_blamp(since_peak, dt))
            }
            Waveform::WhiteNoise => self.noise.bipolar(),
            Waveform::PinkNoise => {
                let w = self.noise.bipolar();
                self.pink.next(w).clamp(-1.0, 1.0)
            }
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        out
    }
}

/// Renders `frames` frames of a test signal on `channels` identical channels. Periodic
/// waveforms peak at `level_db` dBFS (capped at 0); noise stays at or below it. `channels` is
/// capped at [`crate::MAX_CHANNELS`] and `frames` so that all channels together hold at most
/// [`MAX_SIGNAL_SAMPLES`]; an unusable rate is
/// replaced as [`crate::sane_sample_rate`] describes, a non-finite frequency gives 1 kHz and a
/// NaN level gives -20 dBFS.
pub fn signal(wave: Waveform, freq: f32, level_db: f32, sample_rate: f32, frames: usize, channels: usize) -> Vec<Vec<f32>> {
    let sr = sane_sample_rate(sample_rate);
    let channels = channels.min(MAX_CHANNELS);
    if channels == 0 {
        return Vec::new();
    }
    let frames = frames.min(MAX_SIGNAL_SAMPLES / channels);
    let f = if freq.is_finite() { freq.clamp(0.0, sr * 0.49) } else { 1000.0 };
    let g = db_to_gain(if level_db.is_nan() { -20.0 } else { level_db.min(0.0) });
    let mut osc = Oscillator::new(0x1234_5678);
    let mono: Vec<f32> = (0..frames).map(|_| osc.next(wave, f / sr) * g).collect();
    let mut out: Vec<Vec<f32>> = (1..channels).map(|_| mono.clone()).collect();
    out.push(mono);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{db, rms, tone_amplitude};

    #[test]
    fn periodic_waves_peak_at_the_level_and_sit_on_the_frequency() {
        for wave in [Waveform::Sine, Waveform::Square, Waveform::Saw, Waveform::Triangle] {
            let out = signal(wave, 1000.0, -6.0, 48_000.0, 48_000, 2);
            assert_eq!(out.len(), 2);
            assert_eq!(out[0], out[1]);
            let peak = out[0].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            // PolyBLEP corrections overshoot the edges of the square and saw slightly.
            assert!((db(f64::from(peak)) + 6.0).abs() < 0.6, "{wave:?}: peak {peak}");
            let fundamental = tone_amplitude(&out[0], 1000.0, 48_000.0);
            assert!(fundamental > 0.3, "{wave:?}: fundamental {fundamental}");
        }
    }

    #[test]
    fn the_square_is_band_limited() {
        // A 5 kHz square's 9th harmonic (45 kHz) folds to 3 kHz. PolyBLEP keeps that alias well
        // below the one a naive square has.
        let blep = signal(Waveform::Square, 5_000.0, 0.0, 48_000.0, 48_000, 1).remove(0);
        let mut phase = 0.0f32;
        let naive: Vec<f32> = (0..48_000)
            .map(|_| {
                let v = if phase < 0.5 { 1.0 } else { -1.0 };
                phase += 5_000.0 / 48_000.0;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                v
            })
            .collect();
        let alias = |x: &[f32]| tone_amplitude(x, 3_000.0, 48_000.0) / tone_amplitude(x, 5_000.0, 48_000.0);
        assert!(alias(&blep) < alias(&naive) / 4.0, "alias {} dB, naive {} dB", db(alias(&blep)), db(alias(&naive)));
    }

    #[test]
    fn the_triangle_is_band_limited() {
        // A 10 kHz triangle's 3rd harmonic (30 kHz) folds to 18 kHz. PolyBLAMP keeps that alias
        // well below the one a naive triangle has.
        let blamp = signal(Waveform::Triangle, 10_000.0, 0.0, 48_000.0, 48_000, 1).remove(0);
        let mut phase = 0.25f32;
        let naive: Vec<f32> = (0..48_000)
            .map(|_| {
                let v = 1.0 - 4.0 * (phase - 0.5).abs();
                phase += 10_000.0 / 48_000.0;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                v
            })
            .collect();
        let alias = |x: &[f32]| tone_amplitude(x, 18_000.0, 48_000.0) / tone_amplitude(x, 10_000.0, 48_000.0);
        assert!(alias(&blamp) < alias(&naive) / 4.0, "alias {} dB, naive {} dB", db(alias(&blamp)), db(alias(&naive)));
    }

    #[test]
    fn pink_noise_falls_3_db_per_octave() {
        let x = signal(Waveform::PinkNoise, 0.0, 0.0, 48_000.0, 1 << 17, 1).remove(0);
        // Power in two bands an octave apart, each the sum of 128 equally spaced single-bin probes.
        let band = |lo: f64| (0..128).map(|k| tone_amplitude(&x, lo * (1.0 + k as f64 / 128.0), 48_000.0).powi(2)).sum::<f64>();
        let low = band(500.0);
        let high = band(1000.0);
        // Equal-width probes: a -3 dB/octave density gives 3 dB less per probe an octave up.
        let slope = 10.0 * (high / low).log10();
        assert!((slope + 3.0).abs() < 1.5, "slope {slope} dB per octave");
        assert!(x.iter().all(|v| v.abs() <= 1.0));
        assert!(rms(&x) > 0.01);
    }

    #[test]
    fn hostile_arguments_give_bounded_output() {
        assert!(signal(Waveform::Sine, f32::NAN, f32::NAN, f32::NAN, 100, 1)[0].iter().all(|v| v.is_finite() && v.abs() <= 0.11));
        assert_eq!(signal(Waveform::Sine, 1000.0, 0.0, 48_000.0, 10, 1000).len(), MAX_CHANNELS);
        assert!(signal(Waveform::Saw, 1e9, 40.0, 0.0, 64, 1)[0].iter().all(|v| v.abs() <= 1.5));
        assert!(signal(Waveform::Sine, 1000.0, 0.0, 48_000.0, 0, 2).iter().all(Vec::is_empty));
        assert!(signal(Waveform::Sine, 1000.0, 0.0, 48_000.0, usize::MAX, 0).is_empty());
    }

    #[test]
    fn reset_restarts_the_sequence() {
        let mut o = Oscillator::new(7);
        let a: Vec<f32> = (0..64).map(|_| o.next(Waveform::WhiteNoise, 0.0)).collect();
        o.reset();
        let b: Vec<f32> = (0..64).map(|_| o.next(Waveform::WhiteNoise, 0.0)).collect();
        assert_eq!(a, b);
    }
}
