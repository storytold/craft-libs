//! The inverse MDCT of a block (Vorbis I §4.3.7) and the window slopes.
//!
//! `y[j] = sum_k X[k] cos(2*pi/n * (j + 1/2 + n/4) * (k + 1/2))` for `n/2` coefficients and `n`
//! samples, computed as a DCT-IV of the coefficients through an `n/8`-point complex FFT, then
//! unfolded.

use std::f64::consts::PI;

#[derive(Clone, Copy, Default)]
struct Cpx {
    re: f64,
    im: f64,
}

impl Cpx {
    fn mul(self, o: Cpx) -> Cpx {
        Cpx { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }
}

fn cis(t: f64) -> Cpx {
    Cpx { re: t.cos(), im: t.sin() }
}

pub(super) struct Imdct {
    n: usize,
    twiddle: Vec<Cpx>,
    post: Vec<Cpx>,
    fft_tw: Vec<Cpx>,
    rev: Vec<usize>,
}

impl Imdct {
    /// For blocks of `n` samples, a power of two of at least 16.
    pub(super) fn new(n: usize) -> Self {
        let m = n / 2;
        let h = m / 2;
        let twiddle = (0..h).map(|j| cis(-PI * (4.0 * j as f64 + 1.0) / (4.0 * m as f64))).collect();
        let post = (0..h).map(|k| cis(-PI * k as f64 / m as f64)).collect();
        let fft_tw = (0..h / 2).map(|j| cis(-2.0 * PI * j as f64 / h as f64)).collect();
        let bits = h.trailing_zeros();
        let rev = (0..h).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) }).collect();
        Self { n, twiddle, post, fft_tw, rev }
    }

    /// `x`: `n/2` coefficients; `out`: `n` samples.
    pub(super) fn inverse(&self, x: &[f32], out: &mut [f32]) {
        let m = self.n / 2;
        let q = self.n / 4;
        let h = m / 2;
        let mut z: Vec<Cpx> = (0..h).map(|j| Cpx { re: f64::from(x[2 * j]), im: f64::from(x[m - 1 - 2 * j]) }.mul(self.twiddle[j])).collect();
        self.fft(&mut z);
        let mut v = vec![0f64; m];
        for (k, c) in z.iter().enumerate() {
            let d = c.mul(self.post[k]);
            v[2 * k] = d.re;
            v[m - 1 - 2 * k] = -d.im;
        }
        // Unfold the DCT-IV output to the block.
        for i in 0..q {
            out[3 * q - 1 - i] = (-v[i]) as f32;
            out[3 * q + i] = (-v[i]) as f32;
        }
        for i in q..2 * q {
            out[i - q] = v[i] as f32;
            out[3 * q - 1 - i] = (-v[i]) as f32;
        }
    }

    fn fft(&self, a: &mut [Cpx]) {
        let h = a.len();
        for (i, &j) in self.rev.iter().enumerate() {
            if i < j {
                a.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= h {
            let step = h / len;
            for start in (0..h).step_by(len) {
                for k in 0..len / 2 {
                    let w = self.fft_tw[k * step];
                    let u = a[start + k];
                    let t = a[start + k + len / 2].mul(w);
                    a[start + k] = Cpx { re: u.re + t.re, im: u.im + t.im };
                    a[start + k + len / 2] = Cpx { re: u.re - t.re, im: u.im - t.im };
                }
            }
            len *= 2;
        }
    }
}

/// The rising half of the Vorbis window over `len` samples (§4.3.1):
/// `sin(pi/2 * sin^2((i + 0.5) / len * pi/2))`.
pub(super) fn slope(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let s = (PI / 2.0 * (i as f64 + 0.5) / len as f64).sin();
            (PI / 2.0 * s * s).sin() as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_direct_formula() {
        for n in [16usize, 64, 256, 2048] {
            let x: Vec<f32> = (0..n / 2).map(|i| ((i * 7919 % 211) as f32 - 105.0) / 50.0).collect();
            let mut fast = vec![0f32; n];
            Imdct::new(n).inverse(&x, &mut fast);
            for j in [0usize, 1, 2, n / 4, n / 2, n - 1] {
                let direct: f64 =
                    (0..n / 2).map(|k| f64::from(x[k]) * (2.0 * PI / n as f64 * (j as f64 + 0.5 + n as f64 / 4.0) * (k as f64 + 0.5)).cos()).sum();
                assert!((direct - f64::from(fast[j])).abs() < 1e-3 * (1.0 + direct.abs()), "n={n} j={j}: {direct} vs {}", fast[j]);
            }
        }
    }

    #[test]
    fn windowed_overlap_add_reconstructs_the_signal() {
        // Forward MDCT by the definition with the 2/M scale an encoder uses, then this inverse.
        let n = 64usize;
        let m = n / 2;
        let signal: Vec<f64> = (0..n * 6).map(|i| ((i * 37 % 101) as f64 - 50.0) / 50.0).collect();
        let w: Vec<f32> = slope(m).into_iter().chain(slope(m).into_iter().rev()).collect();
        let imdct = Imdct::new(n);
        let mut out = vec![0f64; signal.len()];
        let mut start = 0;
        while start + n <= signal.len() {
            let coeffs: Vec<f32> = (0..m)
                .map(|k| {
                    let s: f64 = (0..n)
                        .map(|j| {
                            signal[start + j] * f64::from(w[j]) * (2.0 * PI / n as f64 * (j as f64 + 0.5 + n as f64 / 4.0) * (k as f64 + 0.5)).cos()
                        })
                        .sum();
                    (s * 2.0 / m as f64) as f32
                })
                .collect();
            let mut y = vec![0f32; n];
            imdct.inverse(&coeffs, &mut y);
            for j in 0..n {
                out[start + j] += f64::from(y[j]) * f64::from(w[j]);
            }
            start += m;
        }
        for i in m..signal.len() - m {
            assert!((out[i] - signal[i]).abs() < 1e-4, "sample {i}: {} vs {}", out[i], signal[i]);
        }
    }

    #[test]
    fn the_slope_is_power_complementary() {
        let s = slope(1024);
        for i in 0..1024 {
            let sum = s[i] * s[i] + s[1023 - i] * s[1023 - i];
            assert!((sum - 1.0).abs() < 1e-5, "{i}: {sum}");
        }
    }
}
