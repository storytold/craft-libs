//! Dither: requantization to 16, 20 or 24 bits with triangular (TPDF) dither and optional
//! first-order noise shaping, for the last step before a fixed-point file or encoder.

use crate::generator::Noise;
use crate::{AudioEffect, MAX_CHANNELS, ParamSpec, ParamValues, block_len, param_plumbing};

/// TPDF dither and requantization. Each output sample is a multiple of the target word length's
/// step (2^(1 - bits)) within -1 ..= 1 - step. With noise shaping, each channel's quantization
/// error is fed back into its next sample, which moves the noise toward high frequencies.
/// Digital silence stays silent, and a non-finite sample becomes 0.
pub struct Dither {
    pv: ParamValues,
    noise: Noise,
    lsb: f32,
    shape: bool,
    err: Vec<f32>,
}

impl Dither {
    pub const PARAMS: &'static [ParamSpec] =
        &[ParamSpec::choice("bits", "Bit Depth", &["16", "20", "24"], 0), ParamSpec::toggle("noise_shaping", "Noise Shaping", false)];

    pub fn new(_sample_rate: f32, channels: usize) -> Self {
        let mut s = Dither {
            pv: ParamValues::new(Self::PARAMS),
            noise: Noise::new(0xD17E),
            lsb: 0.0,
            shape: false,
            err: vec![0.0; channels.clamp(1, MAX_CHANNELS)],
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, _snap: bool) {
        let bits = [16.0f32, 20.0, 24.0].get(self.pv.idx("bits")).copied().unwrap_or(16.0);
        self.lsb = 2f32.powf(1.0 - bits);
        self.shape = self.pv.on("noise_shaping");
    }
}

impl AudioEffect for Dither {
    param_plumbing!("dither");
    fn reset(&mut self) {
        self.err.iter_mut().for_each(|e| *e = 0.0);
        self.noise = Noise::new(0xD17E);
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = block_len(channels);
        let lsb = self.lsb;
        // Frame by frame, so the noise sequence does not depend on how the stream is cut.
        for i in 0..n {
            for (ch, e) in channels.iter_mut().zip(self.err.iter_mut()) {
                let x = &mut ch[i];
                if *x == 0.0 || !x.is_finite() {
                    *x = 0.0;
                    *e = 0.0;
                    continue;
                }
                // Full scale bounds the input, so the error fed back stays within a few steps.
                let x0 = x.clamp(-1.0, 1.0);
                let v = if self.shape { x0 - *e } else { x0 };
                let d = (self.noise.bipolar() + self.noise.bipolar()) * 0.5 * lsb;
                let q = ((v + d) / lsb).round() * lsb;
                *e = q - v;
                *x = q.clamp(-1.0, 1.0 - lsb);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Rng, rms};

    #[test]
    fn output_sits_on_the_16_bit_grid_with_tpdf_noise() {
        let mut d = Dither::new(48_000.0, 1);
        let mut r = Rng::new(3);
        let src: Vec<f32> = (0..96_000).map(|_| r.uniform() * 0.5).collect();
        let mut x = src.clone();
        d.process(&mut [&mut x]);
        let lsb = 2f32.powi(-15);
        assert!(x.iter().all(|v| (v / lsb - (v / lsb).round()).abs() < 1e-3));
        // TPDF dither of 2 LSB peak to peak plus rounding: the error's RMS is about 0.5 LSB.
        let err: Vec<f32> = x.iter().zip(&src).map(|(a, b)| a - b).collect();
        let e = rms(&err) / f64::from(lsb);
        assert!((0.4..0.65).contains(&e), "error RMS {e} LSB");
        assert!(err.iter().all(|v| v.abs() <= 1.5 * lsb));
    }

    #[test]
    fn silence_stays_silent_and_nan_is_zeroed() {
        let mut d = Dither::new(48_000.0, 2);
        let (mut a, mut b) = (vec![0.0f32; 512], vec![f32::NAN; 512]);
        d.process(&mut [&mut a, &mut b]);
        assert!(a.iter().chain(&b).all(|v| *v == 0.0));
    }

    #[test]
    fn huge_samples_with_noise_shaping_stay_on_the_grid() {
        let mut d = Dither::new(48_000.0, 1);
        assert!(d.set_param("noise_shaping", 1.0));
        let mut x = vec![f32::MAX, -f32::MAX, 0.25, 0.25, 1e30, 0.1];
        d.process(&mut [&mut x]);
        let lsb = 2f32.powi(-15);
        assert!(x.iter().all(|v| v.is_finite() && (-1.0..=1.0 - lsb).contains(v)), "{x:?}");
        assert!((x[2] - 0.25).abs() < 4.0 * lsb && (x[5] - 0.1).abs() < 4.0 * lsb, "{x:?}");
    }

    #[test]
    fn noise_shaping_moves_the_error_up_in_frequency() {
        let run = |shape: bool| {
            let mut d = Dither::new(48_000.0, 1);
            assert!(d.set_param("noise_shaping", if shape { 1.0 } else { 0.0 }));
            let src: Vec<f32> = (0..48_000).map(|i| 0.25 * (i as f32 * 0.01).sin()).collect();
            let mut x = src.clone();
            d.process(&mut [&mut x]);
            let err: Vec<f32> = x.iter().zip(&src).map(|(a, b)| a - b).collect();
            // Low-frequency share of the error: a 32-sample moving average keeps below ~1.5 kHz.
            let low: Vec<f32> = err.windows(32).map(|w| w.iter().sum::<f32>() / 32.0).collect();
            rms(&low)
        };
        assert!(run(true) < run(false) * 0.5);
    }
}
