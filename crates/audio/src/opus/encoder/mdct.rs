//! Forward MDCT with the CELT low-overlap window (RFC 6716 §4.3.7, run in the analysis
//! direction).
//!
//! A 20 ms frame is a 1920-point MDCT (960 coefficients) whose window is zero for 420 samples,
//! rises over the 120-sample overlap, is flat for 840 samples, falls over the next 120 samples
//! and is zero again for the last 420. The input block is therefore the 120 overlap samples kept
//! from the previous frame followed by the 960 new samples. The MDCT is folded into a DCT-IV,
//! which is computed with an N/2-point complex FFT.

use crate::opus::encoder::tables::OVERLAP;

#[derive(Clone, Copy, Default, Debug)]
pub struct Cpx {
    pub re: f32,
    pub im: f32,
}

impl Cpx {
    #[inline]
    fn mul(self, o: Cpx) -> Cpx {
        Cpx { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }
    #[inline]
    fn add(self, o: Cpx) -> Cpx {
        Cpx { re: self.re + o.re, im: self.im + o.im }
    }
}

/// Mixed-radix (2, 3, 4, 5) forward complex FFT, unscaled (decimation in time, recursive).
pub struct Fft {
    n: usize,
    factors: Vec<usize>,
    twiddle: Vec<Cpx>,
}

impl Fft {
    pub fn new(n: usize) -> Fft {
        let mut factors = Vec::new();
        let mut m = n;
        for p in [4usize, 2, 3, 5] {
            while m.is_multiple_of(p) {
                factors.push(p);
                m /= p;
            }
        }
        assert_eq!(m, 1, "unsupported FFT size {n}");
        let twiddle = (0..n)
            .map(|k| {
                let ph = -2.0 * std::f64::consts::PI * k as f64 / n as f64;
                Cpx { re: ph.cos() as f32, im: ph.sin() as f32 }
            })
            .collect();
        Fft { n, factors, twiddle }
    }

    pub fn process(&self, data: &mut [Cpx], scratch: &mut Vec<Cpx>) {
        scratch.clear();
        scratch.extend_from_slice(&data[..self.n]);
        self.rec(&mut data[..self.n], scratch, 1, self.n, 0, 1);
    }

    fn rec(&self, out: &mut [Cpx], inp: &[Cpx], in_stride: usize, n: usize, fi: usize, tw_stride: usize) {
        let p = self.factors[fi];
        let m = n / p;
        if m == 1 {
            for (i, o) in out.iter_mut().enumerate().take(p) {
                *o = inp[i * in_stride];
            }
        } else {
            for q in 0..p {
                self.rec(&mut out[q * m..(q + 1) * m], &inp[q * in_stride..], in_stride * p, m, fi + 1, tw_stride * p);
            }
        }
        let big = self.n;
        let mut t = [Cpx::default(); 5];
        for k in 0..m {
            for (q, tq) in t.iter_mut().enumerate().take(p) {
                *tq = out[q * m + k].mul(self.twiddle[(q * k * tw_stride) % big]);
            }
            for q2 in 0..p {
                let mut acc = Cpx::default();
                for (q, tq) in t.iter().enumerate().take(p) {
                    acc = acc.add(tq.mul(self.twiddle[((q * q2) % p) * (big / p)]));
                }
                out[q2 * m + k] = acc;
            }
        }
    }
}

/// Forward MDCT producing `n` coefficients from a block of `n + OVERLAP` samples.
pub struct Mdct {
    /// Number of coefficients (half the MDCT length).
    n: usize,
    fft: Fft,
    pre: Vec<Cpx>,
    post: Vec<Cpx>,
    /// Rising half of the low-overlap window (RFC 6716 §4.3.7).
    pub window: [f32; OVERLAP],
    scale: f32,
    z: Vec<Cpx>,
    scratch: Vec<Cpx>,
    v: Vec<f32>,
}

impl Mdct {
    pub fn new(n: usize) -> Mdct {
        let n2 = n / 2;
        let pi = std::f64::consts::PI;
        let pre = (0..n2)
            .map(|m| {
                let ph = -pi * m as f64 / n as f64;
                Cpx { re: ph.cos() as f32, im: ph.sin() as f32 }
            })
            .collect();
        let post = (0..n2)
            .map(|p| {
                let ph = -pi * (p as f64 + 0.25) / n as f64;
                Cpx { re: ph.cos() as f32, im: ph.sin() as f32 }
            })
            .collect();
        let mut window = [0f32; OVERLAP];
        for (i, w) in window.iter_mut().enumerate() {
            let x = std::f64::consts::FRAC_PI_2 * (i as f64 + 0.5) / OVERLAP as f64;
            let s = x.sin();
            *w = (std::f64::consts::FRAC_PI_2 * s * s).sin() as f32;
        }
        Mdct { n, fft: Fft::new(n2), pre, post, window, scale: 2.0 / n as f32, z: Vec::new(), scratch: Vec::new(), v: vec![0.0; n] }
    }

    /// Transforms `block` (`n + OVERLAP` samples: the previous frame's last `OVERLAP` samples, then
    /// `n` new ones) into `out[..n]`, scaled so that the RFC 6716 decoder's inverse MDCT and
    /// overlap-add reproduce the input.
    pub fn forward(&mut self, block: &[f32], out: &mut [f32]) {
        let n = self.n;
        let n2 = n / 2;
        debug_assert_eq!(block.len(), n + OVERLAP);
        // Place the windowed block into the (virtual) 2n-sample MDCT input u at offset `pad`.
        let pad = (n - OVERLAP) / 2;
        let w = &self.window;
        let u = |t: usize| -> f32 {
            if t < pad || t >= pad + n + OVERLAP {
                return 0.0;
            }
            let j = t - pad;
            let g = if j < OVERLAP {
                w[j]
            } else if j >= n {
                w[OVERLAP - 1 - (j - n)]
            } else {
                1.0
            };
            block[j] * g
        };
        // Fold to a DCT-IV input: v = (-c_r - d, a - b_r) over the four quarters of u.
        for m in 0..n2 {
            self.v[m] = -u(n + n2 - 1 - m) - u(n + n2 + m);
            self.v[n2 + m] = u(m) - u(n2 + n2 - 1 - m);
        }
        // DCT-IV via an n/2-point complex FFT.
        self.z.clear();
        for m in 0..n2 {
            let wv = Cpx { re: self.v[2 * m], im: self.v[n - 1 - 2 * m] };
            self.z.push(wv.mul(self.pre[m]));
        }
        self.fft.process(&mut self.z, &mut self.scratch);
        for p in 0..n2 {
            let y = self.z[p].mul(self.post[p]);
            out[2 * p] = y.re * self.scale;
            out[n - 1 - 2 * p] = -y.im * self.scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Direct MDCT definition X[k] = sum u[t] cos(pi/N (t + 1/2 + N/2)(k + 1/2)).
    fn direct(block: &[f32], n: usize, window: &[f32; OVERLAP]) -> Vec<f64> {
        let pad = (n - OVERLAP) / 2;
        let mut u = vec![0f64; 2 * n];
        for (j, &b) in block.iter().enumerate() {
            let g = if j < OVERLAP {
                window[j]
            } else if j >= n {
                window[OVERLAP - 1 - (j - n)]
            } else {
                1.0
            };
            u[pad + j] = (b * g) as f64;
        }
        (0..n)
            .map(|k| {
                (0..2 * n).map(|t| u[t] * (std::f64::consts::PI / n as f64 * (t as f64 + 0.5 + n as f64 / 2.0) * (k as f64 + 0.5)).cos()).sum::<f64>()
                    * 2.0
                    / n as f64
            })
            .collect()
    }

    #[test]
    fn matches_direct_formula() {
        for &n in &[240usize, 960] {
            let block: Vec<f32> = (0..n + OVERLAP).map(|i| ((i * 37 % 101) as f32 - 50.0) / 7.0).collect();
            let mut m = Mdct::new(n);
            let mut out = vec![0f32; n];
            m.forward(&block, &mut out);
            let d = direct(&block, n, &m.window);
            for k in 0..n {
                assert!((out[k] as f64 - d[k]).abs() < 1e-2, "n={n} k={k}: {} vs {}", out[k], d[k]);
            }
        }
    }

    // ---- Replica of the RFC 6716 decoder inverse MDCT (as in FilmCraft's decoder) ----

    fn imdct_backward(n_full: usize, input: &[f32], out: &mut [f32], window: &[f32]) {
        let n2 = n_full >> 1;
        let n4 = n_full >> 2;
        let t: Vec<f32> = (0..n2).map(|i| (2.0 * std::f64::consts::PI * (i as f64 + 0.125) / n_full as f64).cos() as f32).collect();
        let fft = Fft::new(n4);
        let mut z = Vec::new();
        for i in 0..n4 {
            let x1 = input[2 * i];
            let x2 = input[n2 - 1 - 2 * i];
            let yr = x2 * t[i] + x1 * t[n4 + i];
            let yi = x1 * t[i] - x2 * t[n4 + i];
            z.push(Cpx { re: yi, im: yr });
        }
        fft.process(&mut z, &mut Vec::new());
        let overlap = OVERLAP;
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
        for i in 0..overlap / 2 {
            let x1 = out[overlap - 1 - i];
            let x2 = out[i];
            let w1 = window[i];
            let w2 = window[overlap - 1 - i];
            out[i] = x2 * w2 - x1 * w1;
            out[overlap - 1 - i] = x2 * w1 + x1 * w2;
        }
    }

    /// MDCT analysis followed by the decoder's IMDCT + TDAC overlap-add reproduces the input
    /// delayed by exactly `OVERLAP` samples.
    #[test]
    fn perfect_reconstruction_through_decoder_imdct() {
        let n = 960;
        let frames = 5;
        let sig: Vec<f32> = (0..n * frames).map(|i| (i as f32 * 0.013).sin() * 1000.0 + ((i * 7919) % 211) as f32 - 105.0).collect();
        let mut m = Mdct::new(n);
        let mut hist = vec![0f32; OVERLAP];
        // Decoder memory: [frame output n][folded tail OVERLAP/2] laid out like decode_mem.
        let mut mem = vec![0f32; n + OVERLAP];
        let mut decoded = Vec::new();
        for f in 0..frames {
            let mut block = hist.clone();
            block.extend_from_slice(&sig[f * n..(f + 1) * n]);
            hist.copy_from_slice(&block[n..]);
            let mut coefs = vec![0f32; n];
            m.forward(&block, &mut coefs);
            // Shift: the previous folded tail (at n..n+OVERLAP/2) moves to 0..OVERLAP/2.
            mem.copy_within(n..n + OVERLAP / 2, 0);
            imdct_backward(2 * n, &coefs, &mut mem, &m.window);
            decoded.extend_from_slice(&mem[..n]);
        }
        let mut err = 0f64;
        let mut sum = 0f64;
        for t in OVERLAP..n * frames {
            let d = decoded[t] as f64 - sig[t - OVERLAP] as f64;
            err += d * d;
            sum += (sig[t - OVERLAP] as f64).powi(2);
        }
        let snr = 10.0 * (sum / err).log10();
        assert!(snr > 80.0, "reconstruction SNR {snr:.1} dB");
    }
}
