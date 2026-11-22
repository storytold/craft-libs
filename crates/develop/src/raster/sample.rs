//! Point sampling at continuous coordinates for geometric warps (alignment, projections).
//!
//! [`Rgb32f::sample_lanczos3`] interpolates with a 6 × 6 Lanczos-3 kernel. Bilinear sampling (the
//! [`Rgb32f::sample_bilinear`] next to it) averages two pixels at a half-pixel offset: a [½ ½]
//! filter that passes nothing at the Nyquist frequency, so a frame warped by a fraction of a pixel
//! loses its finest detail. Lanczos-3 keeps most of it. Its negative lobes would overshoot next to
//! a hard edge (a clipped highlight, a star), so each sample is held to the range of the 4 × 4
//! pixels around it, channel by channel: in smooth areas and fine texture that range holds the
//! interpolated value anyway (a 2 × 2 range would cut the peaks of texture near Nyquist that fall
//! between pixels), and at an edge the overshoot goes.

use std::sync::OnceLock;

use crate::Rgb32f;
use crate::resample::Filter;

/// Phases of a pixel the kernel's taps are tabulated for.
const PHASES: usize = 1024;

/// The six Lanczos-3 weights (summing to 1) for a sample `t` (0..1) past the third of the six
/// pixels it reads: pixel `k` (0..6) is at distance `t + 2 − k`.
pub fn lanczos3_taps(t: f32) -> [f32; 6] {
    static TABLE: OnceLock<Vec<[f32; 6]>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..=PHASES)
            .map(|i| {
                let t = i as f32 / PHASES as f32;
                let w: [f32; 6] = std::array::from_fn(|k| Filter::Lanczos3.eval(t + 2.0 - k as f32));
                let s: f32 = w.iter().sum();
                w.map(|v| v / s)
            })
            .collect()
    });
    let i = if t.is_finite() { (t.clamp(0.0, 1.0) * PHASES as f32).round() as usize } else { 0 };
    table.get(i).copied().unwrap_or([0.0, 0.0, 1.0, 0.0, 0.0, 0.0])
}

impl Rgb32f {
    /// Lanczos-3 sample at continuous pixel coordinates (pixel centres at +0.5), held to the range
    /// of the 4 × 4 pixels around it (see the module docs). Past the edges the edge pixels repeat;
    /// a coordinate that isn't finite reads the nearest edge, as [`Rgb32f::sample_bilinear`] does.
    pub fn sample_lanczos3(&self, x: f32, y: f32) -> [f32; 3] {
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 {
            return [0.0; 3];
        }
        let fx = x.clamp(-1.0, w as f32 + 1.0) - 0.5;
        let fy = y.clamp(-1.0, h as f32 + 1.0) - 0.5;
        let (fx, fy) = (if fx.is_finite() { fx } else { 0.0 }, if fy.is_finite() { fy } else { 0.0 });
        let (x0, y0) = (fx.floor(), fy.floor());
        let (wx, wy) = (lanczos3_taps(fx - x0), lanczos3_taps(fy - y0));
        let (x0, y0) = (x0 as isize, y0 as isize);
        let col: [usize; 6] = std::array::from_fn(|k| (x0 + k as isize - 2).clamp(0, w as isize - 1) as usize);
        let row: [usize; 6] = std::array::from_fn(|k| (y0 + k as isize - 2).clamp(0, h as isize - 1) as usize * w);
        let px = |r: usize, c: usize| self.data.get(r + c).copied().unwrap_or_default();
        let mut acc = [0f32; 3];
        // and the range of the 4 × 4 pixels around the sample (taps 1 to 4 on each axis)
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for (j, (r, &wr)) in row.iter().zip(&wy).enumerate() {
            let mut line = [0f32; 3];
            for (i, (&c, &wc)) in col.iter().zip(&wx).enumerate() {
                let p = px(*r, c);
                for ch in 0..3 {
                    line[ch] += wc * p[ch];
                }
                if (1..5).contains(&i) && (1..5).contains(&j) {
                    for ch in 0..3 {
                        lo[ch] = lo[ch].min(p[ch]);
                        hi[ch] = hi[ch].max(p[ch]);
                    }
                }
            }
            for ch in 0..3 {
                acc[ch] += wr * line[ch];
            }
        }
        // no finite pixel in the 4 × 4 range (NaN all around): nothing to hold the sample to
        std::array::from_fn(|ch| if lo[ch] <= hi[ch] { acc[ch].clamp(lo[ch], hi[ch]) } else { acc[ch] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// High-pass energy of a row-wise pattern: the mean squared second difference.
    fn hp(img: &Rgb32f) -> f64 {
        let w = img.width;
        let mut s = 0.0;
        let mut n = 0;
        for y in 4..img.height - 4 {
            for x in 4..w - 4 {
                let v = |dx: isize| img.get((x as isize + dx) as usize, y)[1] as f64;
                s += (2.0 * v(0) - v(-1) - v(1)).powi(2);
                n += 1;
            }
        }
        s / n as f64
    }

    #[test]
    fn taps_sum_to_one_and_hit_pixels_exactly() {
        for t in [0.0, 0.1, 0.5, 0.77, 1.0] {
            let w = lanczos3_taps(t);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        let w = lanczos3_taps(0.0);
        assert!((w[2] - 1.0).abs() < 1e-5 && w.iter().enumerate().all(|(k, v)| k == 2 || v.abs() < 1e-5));
        assert_eq!(lanczos3_taps(f32::NAN), lanczos3_taps(0.0));
    }

    /// A fine pattern moved by half a pixel: bilinear keeps cos²(π f) of its high-pass energy (a
    /// third at 0.3 cycles per pixel), Lanczos-3 twice that, and nearly all of it at 0.2.
    #[test]
    fn a_half_pixel_shift_keeps_fine_detail() {
        for (f, lanczos_min, over_bilinear) in [(0.2f32, 0.9, 1.3), (0.3, 0.65, 1.9)] {
            let img = Rgb32f::from_fn(64, 16, |x, _| [0.5 + 0.4 * (x as f32 * f * std::f32::consts::TAU).sin(); 3]);
            let shifted = |s: &dyn Fn(f32, f32) -> [f32; 3]| Rgb32f::from_fn(64, 16, |x, y| s(x as f32 + 1.0, y as f32 + 0.5));
            let bil = shifted(&|x, y| img.sample_bilinear(x, y));
            let lan = shifted(&|x, y| img.sample_lanczos3(x, y));
            let (e0, eb, el) = (hp(&img), hp(&bil), hp(&lan));
            assert!(el > lanczos_min * e0, "{f} c/px: lanczos {el} vs {e0}");
            assert!(el > over_bilinear * eb, "{f} c/px: lanczos {el} vs bilinear {eb}");
        }
    }

    /// Next to a hard edge Lanczos-3 would overshoot; the sample stays within its neighbours.
    #[test]
    fn no_overshoot_at_a_hard_edge() {
        let img = Rgb32f::from_fn(16, 4, |x, _| if x < 8 { [0.0; 3] } else { [1.0; 3] });
        for i in 0..160 {
            let v = img.sample_lanczos3(i as f32 * 0.1, 2.0);
            assert!((0.0..=1.0).contains(&v[0]), "{v:?} at {}", i as f32 * 0.1);
        }
        // pixel centres come back exactly
        assert_eq!(img.sample_lanczos3(8.5, 1.5), [1.0; 3]);
        assert_eq!(img.sample_lanczos3(7.5, 1.5), [0.0; 3]);
    }

    #[test]
    fn edges_and_odd_coordinates_never_panic() {
        let img = Rgb32f::from_fn(3, 2, |x, y| [x as f32, y as f32, 1.0]);
        for (x, y) in [(f32::INFINITY, f32::INFINITY), (f32::NEG_INFINITY, 0.0), (f32::NAN, f32::NAN), (f32::MAX, -1e30), (-5.0, 9.0)] {
            let v = img.sample_lanczos3(x, y);
            assert!(v.iter().all(|c| c.is_finite()), "{v:?}");
        }
        assert_eq!(Rgb32f::new(0, 0).sample_lanczos3(1.0, 1.0), [0.0; 3]);
        // pixels that are all NaN (a damaged float file) give a NaN sample, never a panic
        let nan = Rgb32f::from_fn(8, 8, |_, _| [f32::NAN; 3]);
        assert!(nan.sample_lanczos3(4.3, 4.6).iter().all(|v| v.is_nan()));
        assert_eq!(img.sample_lanczos3(0.5, 0.5), [0.0, 0.0, 1.0]);
    }
}
