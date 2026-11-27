//! Inverse MDCT with the low-overlap window and in-place TDAC (RFC 6716 §4.3.7).
//!
//! The IMDCT is computed through an N/4-point complex FFT (pre-rotation, FFT, post-rotation),
//! producing the N/2 "folded" time samples; the window overlap region is then unfolded against the
//! folded tail of the previous block, which the caller keeps in place in the output buffer.

use crate::opus::fft::{Cpx, Fft};

pub struct Imdct {
    /// Full MDCT length (2x the number of coefficients).
    n: usize,
    trig: Vec<f32>,
    fft: Fft,
}

impl Imdct {
    pub fn new(n: usize) -> Imdct {
        let trig = (0..n / 2).map(|i| (2.0 * std::f64::consts::PI * (i as f64 + 0.125) / n as f64).cos() as f32).collect();
        Imdct { n, trig, fft: Fft::new(n / 4) }
    }

    /// Inverse transform of `n/2` coefficients read from `input` at `stride`.
    ///
    /// Writes the folded block to `out[overlap/2 .. overlap/2 + n/2]` and unfolds the first
    /// `overlap` samples against the previous block's folded tail already stored in
    /// `out[0 .. overlap/2]`.
    pub fn backward(&self, input: &[f32], stride: usize, out: &mut [f32], window: &[f32], overlap: usize, work: &mut ImdctScratch) {
        let n2 = self.n >> 1;
        let n4 = self.n >> 2;
        let t = &self.trig;
        let z = &mut work.z;
        z.clear();
        for i in 0..n4 {
            let x1 = input[2 * i * stride];
            let x2 = input[stride * (n2 - 1 - 2 * i)];
            let yr = x2 * t[i] + x1 * t[n4 + i];
            let yi = x1 * t[i] - x2 * t[n4 + i];
            z.push(Cpx { re: yi, im: yr });
        }
        self.fft.process(z, &mut work.scratch);
        let base = overlap >> 1;
        let y = &mut out[base..base + n2];
        for i in 0..n4.div_ceil(2) {
            let front = z[i];
            let back = z[n4 - 1 - i];
            let (re, im) = (front.im, front.re);
            let (t0, t1) = (t[i], t[n4 + i]);
            let yr0 = re * t0 + im * t1;
            let yi0 = re * t1 - im * t0;
            let (re, im) = (back.im, back.re);
            let (t0, t1) = (t[n4 - i - 1], t[n2 - i - 1]);
            let yr1 = re * t0 + im * t1;
            let yi1 = re * t1 - im * t0;
            y[2 * i] = yr0;
            y[n2 - 1 - 2 * i] = yi0;
            y[n2 - 2 - 2 * i] = yr1;
            y[2 * i + 1] = yi1;
        }
        // Mirror on both sides for TDAC.
        for i in 0..overlap / 2 {
            let x1 = out[overlap - 1 - i];
            let x2 = out[i];
            let w1 = window[i];
            let w2 = window[overlap - 1 - i];
            out[i] = x2 * w2 - x1 * w1;
            out[overlap - 1 - i] = x2 * w1 + x1 * w2;
        }
    }
}

#[derive(Default)]
pub struct ImdctScratch {
    z: Vec<Cpx>,
    scratch: Vec<Cpx>,
}
