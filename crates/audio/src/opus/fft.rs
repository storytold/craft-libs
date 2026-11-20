//! Shared CELT forward complex FFT.

#[derive(Clone, Copy, Default, Debug)]
pub struct Cpx {
    pub re: f32,
    pub im: f32,
}

impl Cpx {
    #[inline]
    pub(crate) fn mul(self, o: Cpx) -> Cpx {
        Cpx { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }
    #[inline]
    pub(crate) fn add(self, o: Cpx) -> Cpx {
        Cpx { re: self.re + o.re, im: self.im + o.im }
    }
    #[inline]
    pub(crate) fn sub(self, o: Cpx) -> Cpx {
        Cpx { re: self.re - o.re, im: self.im - o.im }
    }
}

/// Mixed-radix (2, 3, 4, 5) forward complex FFT, unscaled (decimation in time, recursive).
pub struct Fft {
    pub(crate) n: usize,
    pub(crate) factors: Vec<usize>,
    pub(crate) twiddle: Vec<Cpx>,
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
        // The twiddle index q * k * tw_stride is always below n, so no modulo is needed.
        let tw = &self.twiddle;
        match p {
            2 => {
                for k in 0..m {
                    let a0 = out[k];
                    let a1 = out[m + k].mul(tw[k * tw_stride]);
                    out[k] = a0.add(a1);
                    out[m + k] = a0.sub(a1);
                }
            }
            4 => {
                for k in 0..m {
                    let t0 = out[k];
                    let t1 = out[m + k].mul(tw[k * tw_stride]);
                    let t2 = out[2 * m + k].mul(tw[2 * k * tw_stride]);
                    let t3 = out[3 * m + k].mul(tw[3 * k * tw_stride]);
                    let (s0, d0) = (t0.add(t2), t0.sub(t2));
                    let (s1, d1) = (t1.add(t3), t1.sub(t3));
                    out[k] = s0.add(s1);
                    out[2 * m + k] = s0.sub(s1);
                    // The odd outputs combine d0 with -j * d1 = (d1.im, -d1.re) and its negative.
                    out[m + k] = Cpx { re: d0.re + d1.im, im: d0.im - d1.re };
                    out[3 * m + k] = Cpx { re: d0.re - d1.im, im: d0.im + d1.re };
                }
            }
            3 => {
                // Imaginary part of exp(-2 pi i / 3) = -sqrt(3) / 2.
                let h = tw[self.n / 3].im;
                for k in 0..m {
                    let t0 = out[k];
                    let t1 = out[m + k].mul(tw[k * tw_stride]);
                    let t2 = out[2 * m + k].mul(tw[2 * k * tw_stride]);
                    let (s, d) = (t1.add(t2), t1.sub(t2));
                    let a = Cpx { re: t0.re - 0.5 * s.re, im: t0.im - 0.5 * s.im };
                    out[k] = t0.add(s);
                    // j * h * d = (-h * d.im, h * d.re)
                    out[m + k] = Cpx { re: a.re - h * d.im, im: a.im + h * d.re };
                    out[2 * m + k] = Cpx { re: a.re + h * d.im, im: a.im - h * d.re };
                }
            }
            _ => {
                // Generic DFT butterfly (radix 5, once per transform).
                let roots: [Cpx; 5] = std::array::from_fn(|q| if q < p { tw[q * (self.n / p)] } else { Cpx::default() });
                let mut t = [Cpx::default(); 5];
                for k in 0..m {
                    t[0] = out[k];
                    for q in 1..p {
                        t[q] = out[q * m + k].mul(tw[q * k * tw_stride]);
                    }
                    for q2 in 0..p {
                        let mut acc = t[0];
                        for q in 1..p {
                            acc = acc.add(t[q].mul(roots[(q * q2) % p]));
                        }
                        out[q2 * m + k] = acc;
                    }
                }
            }
        }
    }
}
