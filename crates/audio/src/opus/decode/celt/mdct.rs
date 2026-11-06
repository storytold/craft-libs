//! Inverse MDCT with the low-overlap window and in-place TDAC (RFC 6716 §4.3.7).
//!
//! The IMDCT is computed through an N/4-point complex FFT (pre-rotation, FFT, post-rotation),
//! producing the N/2 "folded" time samples; the window overlap region is then unfolded against the
//! folded tail of the previous block, which the caller keeps in place in the output buffer.

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

/// Mixed-radix (2, 3, 4, 5) forward complex FFT, unscaled.
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
            for q in 0..p {
                let w = self.twiddle[(q * k * tw_stride) % big];
                t[q] = out[q * m + k].mul(w);
            }
            for q2 in 0..p {
                let mut acc = Cpx::default();
                for (q, tq) in t.iter().enumerate().take(p) {
                    let w = self.twiddle[((q * q2) % p) * (big / p)];
                    acc = acc.add(tq.mul(w));
                }
                out[q2 * m + k] = acc;
            }
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_matches_dft() {
        for &n in &[60usize, 120, 240, 480] {
            let fft = Fft::new(n);
            let x: Vec<Cpx> = (0..n).map(|i| Cpx { re: ((i * 7) % 13) as f32 - 6.0, im: ((i * 3) % 5) as f32 - 2.0 }).collect();
            let mut y = x.clone();
            fft.process(&mut y, &mut Vec::new());
            for k in [0, 1, 7, n / 2, n - 1] {
                let mut acc = (0f64, 0f64);
                for (i, v) in x.iter().enumerate() {
                    let ph = -2.0 * std::f64::consts::PI * (i * k) as f64 / n as f64;
                    acc.0 += v.re as f64 * ph.cos() - v.im as f64 * ph.sin();
                    acc.1 += v.re as f64 * ph.sin() + v.im as f64 * ph.cos();
                }
                assert!((acc.0 - y[k].re as f64).abs() < 1e-2 && (acc.1 - y[k].im as f64).abs() < 1e-2, "n={n} k={k}");
            }
        }
    }

    /// The folded output must equal the direct IMDCT formula folded by TDAC symmetry.
    #[test]
    fn imdct_matches_direct_formula() {
        let n = 240;
        let n2 = n / 2;
        let im = Imdct::new(n);
        let x: Vec<f32> = (0..n2).map(|k| ((k * 37 % 11) as f32 - 5.0) / 5.0).collect();
        let overlap = 0;
        let mut out = vec![0f32; n2];
        im.backward(&x, 1, &mut out, &[], overlap, &mut ImdctScratch::default());
        // Direct IMDCT y[t] = sum X[k] cos(2pi/N (t + 1/2 + N/4)(k + 1/2)), t in 0..N.
        let y: Vec<f64> = (0..n)
            .map(|t| {
                (0..n2)
                    .map(|k| x[k] as f64 * (2.0 * std::f64::consts::PI / n as f64 * (t as f64 + 0.5 + n as f64 / 4.0) * (k as f64 + 0.5)).cos())
                    .sum()
            })
            .collect();
        // The folded half: out[i] relates to y in the middle half; check proportionality.
        let mid: Vec<f64> = (0..n2).map(|i| y[n / 4 + i]).collect();
        let dot: f64 = mid.iter().zip(&out).map(|(a, b)| a * *b as f64).sum();
        let ea: f64 = mid.iter().map(|a| a * a).sum();
        let eb: f64 = out.iter().map(|b| (*b as f64).powi(2)).sum();
        let corr = dot.abs() / (ea.sqrt() * eb.sqrt());
        assert!(corr > 0.999, "corr {corr}");
    }
}
