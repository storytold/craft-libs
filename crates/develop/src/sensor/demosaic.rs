//! Demosaicing: CFA mosaic → camera RGB ([`Rgb32f`]), values as normalised (white = 1.0).
//!
//! Methods (all from published descriptions, see the README):
//! - [`Method::Bilinear`] — average of same-colour neighbours (drafts / thumbnails); works for any pattern.
//! - [`Method::Ppg`] — Patterned Pixel Grouping (Chuan-kai Lin): gradient-selected green, colour-difference R/B.
//! - [`Method::Ahd`] — Adaptive Homogeneity-Directed (Hirakawa & Parks, IEEE TIP 2005): horizontal/vertical
//!   candidates chosen per pixel by CIELab homogeneity. The quality default for Bayer sensors.
//! - Non-Bayer patterns (X-Trans, …) always use `xtrans::directional`, our own edge-weighted colour-difference
//!   interpolation.
//!
//! Every method reproduces a constant-colour mosaic exactly.

mod ahd;
pub(super) mod bilinear;
mod ppg;
mod xtrans;

use super::{Cfa, Normalized, Rect, Rgb32f};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Method {
    Bilinear,
    Ppg,
    #[default]
    Ahd,
}

pub(super) fn mosaic_method(cfa: &Cfa, w: usize, h: usize, method: Method) -> Method {
    if (w < 4 || h < 4) && cfa.is_bayer() { Method::Bilinear } else { method }
}

/// Mirror-reflected access to a single-plane mosaic (reflection keeps Bayer parity: −k → k, w−1+k → w−1−k).
#[derive(Clone, Copy)]
pub(crate) struct Mosaic<'a> {
    pub w: usize,
    pub h: usize,
    pub data: &'a [f32],
    pub cfa: &'a Cfa,
}

#[inline]
pub(crate) fn reflect(i: isize, n: usize) -> usize {
    if i >= 0 && (i as usize) < n {
        return i as usize;
    }
    let n = n as isize;
    if n == 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let mut i = i.rem_euclid(period);
    if i >= n {
        i = period - i;
    }
    i as usize
}

/// `v` clamped between `a` and `b`: `v.clamp(a.min(b), a.max(b))`, except that it returns `v` as is when both
/// bounds are NaN (NaN samples in a float raw), where `f32::clamp` panics.
#[inline]
pub(crate) fn clamp_between(v: f32, a: f32, b: f32) -> f32 {
    let (lo, hi) = (a.min(b), a.max(b));
    if lo <= hi { v.clamp(lo, hi) } else { v }
}

impl Mosaic<'_> {
    #[inline]
    pub fn get(&self, x: isize, y: isize) -> f32 {
        self.data[reflect(y, self.h) * self.w + reflect(x, self.w)]
    }
    #[inline]
    pub fn color(&self, x: isize, y: isize) -> u8 {
        self.cfa.color_at(reflect(x, self.w), reflect(y, self.h))
    }
}

/// Demosaic normalised raw data. `cpp == 3` data is returned as is; monochrome data is replicated to grey.
pub fn demosaic(n: &Normalized, method: Method) -> Rgb32f {
    let (w, h) = (n.width, n.height);
    if w == 0 || h == 0 {
        return Rgb32f::new(w, h);
    }
    match (&n.cfa, n.cpp) {
        (_, 3) => Rgb32f { width: w, height: h, data: n.data.as_chunks::<3>().0.iter().map(|c| [c[0], c[1], c[2]]).collect() },
        // four colours develop from their first three; narrower data (refused by `RawImage::validate`, issue #1511)
        // repeats its last sample instead of reading past the pixel
        (_, cpp) if cpp != 1 => Rgb32f {
            width: w,
            height: h,
            data: n
                .data
                .chunks_exact(cpp.max(1))
                .map(|c| {
                    let s = |i: usize| c.get(i).or(c.last()).copied().unwrap_or(0.0);
                    [s(0), s(1), s(2)]
                })
                .collect(),
        },
        (None, _) => Rgb32f { width: w, height: h, data: n.data.iter().map(|&v| [v; 3]).collect() },
        (Some(cfa), _) => {
            let m = Mosaic { w, h, data: &n.data, cfa };
            match mosaic_method(cfa, w, h, method) {
                Method::Bilinear => bilinear::bilinear(&m),
                _ if !cfa.is_bayer() => xtrans::directional(&m),
                Method::Ppg => ppg::ppg(&m),
                Method::Ahd => ahd::ahd(&m),
            }
        }
    }
}

/// [`demosaic`] with [`Method::Ahd`] of the window `r` alone, for Bayer mosaics (`None` for any other data, or a
/// window not inside the mosaic): the same pixels as demosaicing it all and cropping that to `r`, without
/// demosaicing the rest or copying the window out of it.
pub(crate) fn ahd_window(n: &Normalized, r: Rect) -> Option<Rgb32f> {
    let (w, h) = (n.width, n.height);
    let cfa = n.cfa.as_ref()?;
    let inside = r.x.checked_add(r.width).is_some_and(|e| e <= w) && r.y.checked_add(r.height).is_some_and(|e| e <= h);
    if n.cpp != 1 || !cfa.is_bayer() || w < 4 || h < 4 || n.data.len() != w * h || r.width == 0 || r.height == 0 || !inside {
        return None;
    }
    let m = Mosaic { w, h, data: &n.data, cfa };
    Some(ahd::ahd_window(&m, r.x, r.y, r.width, r.height))
}

/// Build a mosaic from an RGB image by sampling each pixel's CFA colour (for tests and synthetic data).
pub fn mosaic_from_rgb(img: &Rgb32f, cfa: &Cfa) -> Normalized {
    let data = (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).map(|(x, y)| img.get(x, y)[cfa.color_at(x, y) as usize]).collect();
    Normalized { width: img.width, height: img.height, cpp: 1, data, cfa: Some(cfa.clone()) }
}

/// Peak signal-to-noise ratio (dB, peak 1.0) between two images over an interior region (`border` pixels excluded).
pub fn psnr(a: &Rgb32f, b: &Rgb32f, border: usize) -> f64 {
    let mut se = 0.0f64;
    let mut n = 0usize;
    for y in border..a.height.saturating_sub(border) {
        for x in border..a.width.saturating_sub(border) {
            let (p, q) = (a.get(x, y), b.get(x, y));
            for c in 0..3 {
                se += ((p[c] - q[c]) as f64).powi(2);
                n += 1;
            }
        }
    }
    if n == 0 || se == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (1.0 / (se / n as f64)).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smooth_scene(w: usize, h: usize) -> Rgb32f {
        Rgb32f::from_fn(w, h, |x, y| {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            [0.2 + 0.6 * fx * fy, 0.3 + 0.4 * (fx * 3.0).sin().abs() * 0.5 + 0.2 * fy, 0.1 + 0.5 * (1.0 - fx) * (0.5 + 0.5 * (fy * 5.0).cos())]
        })
    }

    /// A photo-like scene: sharp luminance structure (disc, stripes, diagonal edge, fine texture) modulating a
    /// smoothly varying colour — edges are mostly luminance edges, as in natural images.
    fn edgy_scene(w: usize, h: usize) -> Rgb32f {
        Rgb32f::from_fn(w, h, |x, y| {
            let (fx, fy) = (x as f32, y as f32);
            let d = ((fx - w as f32 * 0.35).powi(2) + (fy - h as f32 * 0.4).powi(2)).sqrt();
            let lum = if d < w as f32 * 0.18 {
                0.85
            } else if x > w * 2 / 3 && (y / 3) % 2 == 0 {
                0.15
            } else if fx + fy * 0.5 > w as f32 * 1.1 {
                0.7
            } else {
                0.35 + 0.1 * ((fx * 0.7).sin() * (fy * 0.9).cos())
            };
            let tint = [0.9 + 0.1 * (fx / w as f32), 0.8, 0.55 + 0.3 * (fy / h as f32)];
            [lum * tint[0], lum * tint[1], lum * tint[2]]
        })
    }

    fn run(img: &Rgb32f, cfa: &Cfa, m: Method) -> Rgb32f {
        demosaic(&mosaic_from_rgb(img, cfa), m)
    }

    #[test]
    fn constant_colour_is_exact_for_all_methods_and_phases() {
        let img = Rgb32f::filled(17, 13, [0.3, 0.55, 0.8]);
        for pat in ["RGGB", "BGGR", "GRBG", "GBRG"] {
            let cfa = Cfa::bayer(pat).unwrap();
            for m in [Method::Bilinear, Method::Ppg, Method::Ahd] {
                let out = run(&img, &cfa, m);
                for p in &out.data {
                    for c in 0..3 {
                        assert!((p[c] - img.data[0][c]).abs() < 1e-5, "{pat} {m:?} {p:?}");
                    }
                }
            }
        }
        let out = run(&Rgb32f::filled(19, 14, [0.3, 0.55, 0.8]), &Cfa::xtrans(), Method::Ahd);
        assert!(out.data.iter().all(|p| (p[0] - 0.3).abs() < 1e-5 && (p[1] - 0.55).abs() < 1e-5 && (p[2] - 0.8).abs() < 1e-5));
    }

    /// A window demosaiced alone is the same, bit for bit, as that part of the whole picture: at the edges and
    /// corners (where the mosaic is reflected), in the middle, across tile seams, at every CFA phase.
    #[test]
    fn ahd_window_is_the_crop_of_the_whole() {
        let img = edgy_scene(300, 270);
        for pat in ["RGGB", "GBRG"] {
            let n = mosaic_from_rgb(&img, &Cfa::bayer(pat).unwrap());
            let whole = demosaic(&n, Method::Ahd);
            for r in
                [Rect::new(0, 0, 300, 270), Rect::new(0, 0, 7, 5), Rect::new(1, 3, 298, 266), Rect::new(121, 130, 140, 9), Rect::new(293, 263, 7, 7)]
            {
                let part = ahd_window(&n, r).unwrap();
                let want = whole.clone().into_crop(r.x, r.y, r.width, r.height);
                assert_eq!((part.width, part.height), (r.width, r.height));
                assert!(part.data.iter().zip(&want.data).all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits)), "{pat} {r:?}");
            }
            assert!(ahd_window(&n, Rect::new(1, 0, 300, 270)).is_none(), "a window past the mosaic is refused");
        }
    }

    #[test]
    fn known_samples_are_preserved() {
        let img = edgy_scene(40, 32);
        let cfa = Cfa::bayer("GRBG").unwrap();
        for m in [Method::Bilinear, Method::Ppg, Method::Ahd] {
            let out = run(&img, &cfa, m);
            for y in 0..32 {
                for x in 0..40 {
                    let c = cfa.color_at(x, y) as usize;
                    assert!((out.get(x, y)[c] - img.get(x, y)[c]).abs() < 1e-6, "{m:?}");
                }
            }
        }
    }

    #[test]
    fn psnr_thresholds_and_ranking() {
        let smooth = smooth_scene(96, 80);
        let edgy = edgy_scene(96, 80);
        let cfa = Cfa::bayer("RGGB").unwrap();
        let p = |img: &Rgb32f, m| psnr(img, &run(img, &cfa, m), 4);
        let (sb, sp, sa) = (p(&smooth, Method::Bilinear), p(&smooth, Method::Ppg), p(&smooth, Method::Ahd));
        assert!(sb > 38.0 && sp > 38.0 && sa > 38.0, "smooth: bilinear {sb:.1} ppg {sp:.1} ahd {sa:.1}");
        let (eb, ep, ea) = (p(&edgy, Method::Bilinear), p(&edgy, Method::Ppg), p(&edgy, Method::Ahd));
        eprintln!("smooth: bilinear {sb:.1} ppg {sp:.1} ahd {sa:.1}; edgy: bilinear {eb:.1} ppg {ep:.1} ahd {ea:.1}");
        assert!(eb > 20.0, "edgy bilinear {eb:.1}");
        assert!(ep > eb + 6.0, "ppg {ep:.1} vs bilinear {eb:.1}");
        assert!(ea > eb + 6.0, "ahd {ea:.1} vs bilinear {eb:.1}");
        let xt = Cfa::xtrans();
        let xp = psnr(&edgy, &run(&edgy, &xt, Method::Ahd), 6);
        let xb = psnr(&edgy, &run(&edgy, &xt, Method::Bilinear), 6);
        assert!(xp > 20.0 && xp >= xb - 0.5, "xtrans directional {xp:.1} bilinear {xb:.1}");
    }

    /// Mean anisotropy of a demosaiced picture's fine luminance detail: per pixel, how much more its 3 × 3 high-pass
    /// correlates with its horizontal neighbours than with its vertical ones (or the reverse). Noise has none; the
    /// short horizontal and vertical strokes of a maze pattern have a lot.
    fn anisotropy(img: &Rgb32f) -> f64 {
        let (w, h) = (img.width, img.height);
        let lum: Vec<f32> = img.data.iter().map(|p| 0.25 * p[0] + 0.5 * p[1] + 0.25 * p[2]).collect();
        let mean3 = |x: usize, y: usize| (0..9).map(|k| lum[(y + k / 3 - 1) * w + x + k % 3 - 1]).sum::<f32>() / 9.0;
        let hp: Vec<f32> =
            (0..w * h).map(|i| if i % w == 0 || i % w == w - 1 || i < w || i >= w * (h - 1) { 0.0 } else { lum[i] - mean3(i % w, i / w) }).collect();
        let (mut sum, mut n) = (0f64, 0f64);
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                let i = y * w + x;
                sum += f64::from((hp[i] * (hp[i - 1] + hp[i + 1] - hp[i - w] - hp[i + w])).abs());
                n += 1.0;
            }
        }
        sum / n
    }

    #[test]
    fn flat_noise_does_not_turn_into_a_maze() {
        // a flat grey patch with sensor-like noise: AHD picked horizontal or vertical candidates per pixel by a
        // homogeneity count that noise alone decides, so the picture became short horizontal and vertical strokes
        let (w, h) = (192, 160);
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut noise = || {
            // sum of four uniforms ≈ Gaussian, σ ≈ 0.02
            (0..4)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
                })
                .sum::<f32>()
                * 0.035
        };
        let cfa = Cfa::bayer("RGGB").unwrap();
        let data = (0..w * h).map(|_| 0.2 + noise()).collect();
        let n = Normalized { width: w, height: h, cpp: 1, data, cfa: Some(cfa) };
        let (ahd, bilinear) = (anisotropy(&demosaic(&n, Method::Ahd)), anisotropy(&demosaic(&n, Method::Bilinear)));
        eprintln!("anisotropy: ahd {ahd:.3e}, bilinear {bilinear:.3e}");
        // the hard choice gave about 11 times bilinear's, the blend about 7.5 times
        assert!(ahd < 9.0 * bilinear, "ahd {ahd:.3e} bilinear {bilinear:.3e}");
    }

    #[test]
    fn tiny_and_degenerate_inputs() {
        let cfa = Cfa::bayer("RGGB").unwrap();
        for (w, h) in [(1, 1), (2, 2), (3, 5), (5, 3), (1, 7)] {
            let img = Rgb32f::filled(w, h, [0.5; 3]);
            for m in [Method::Bilinear, Method::Ppg, Method::Ahd] {
                let out = run(&img, &cfa, m);
                assert_eq!((out.width, out.height), (w, h));
                assert!(out.data.iter().all(|p| p.iter().all(|v| v.is_finite())));
            }
        }
        let n = Normalized { width: 2, height: 1, cpp: 1, data: vec![0.2, 0.4], cfa: None };
        assert_eq!(demosaic(&n, Method::Ahd).data, vec![[0.2; 3], [0.4; 3]]);
        let n = Normalized { width: 1, height: 1, cpp: 3, data: vec![0.1, 0.2, 0.3], cfa: None };
        assert_eq!(demosaic(&n, Method::Ahd).data, vec![[0.1, 0.2, 0.3]]);
    }

    #[test]
    fn a_nan_sample_spreads_no_further_than_the_chosen_candidate_takes_it() {
        // vertical stripes: the vertical candidate wins outright beside them, and the horizontal one, which reads two
        // samples to each side, is the only one that sees a NaN two columns away
        let img = Rgb32f::from_fn(32, 24, |x, _| if (x / 2) % 2 == 0 { [0.2, 0.3, 0.25] } else { [0.7, 0.8, 0.6] });
        let cfa = Cfa::bayer("RGGB").unwrap();
        let mut n = mosaic_from_rgb(&img, &cfa);
        n.data[12 * 32 + 16] = f32::NAN;
        let out = demosaic(&n, Method::Ahd);
        let bad = out.data.iter().filter(|p| p.iter().any(|v| !v.is_finite())).count();
        // the pixels that read the NaN sample themselves, as many as the outright pick gave; the blend made it 33
        assert!(bad <= 9, "{bad} pixels not finite");
    }

    #[test]
    fn non_finite_samples_do_not_panic() {
        // regression: NaN samples (a float DNG) gave AHD and PPG NaN bounds for `f32::clamp`, which panics
        let cfa = Cfa::bayer("RGGB").unwrap();
        let mut patchy = mosaic_from_rgb(&edgy_scene(24, 20), &cfa);
        for (i, v) in patchy.data.iter_mut().enumerate() {
            let (x, y) = (i % 24, i / 24);
            if (8..12).contains(&x) && (6..10).contains(&y) {
                *v = f32::NAN;
            } else if i % 7 == 0 {
                *v = if i % 2 == 0 { f32::INFINITY } else { f32::NEG_INFINITY };
            }
        }
        let all_nan = Normalized { width: 9, height: 7, cpp: 1, data: vec![f32::NAN; 63], cfa: Some(cfa) };
        for n in [&patchy, &all_nan] {
            for m in [Method::Bilinear, Method::Ppg, Method::Ahd] {
                let out = demosaic(n, m);
                assert_eq!((out.width, out.height), (n.width, n.height), "{m:?}");
            }
        }
        // without NaN bounds it is the `f32::clamp` it replaced, bit for bit (finite results stay identical)
        let vals = [f32::NEG_INFINITY, -3e38, -2.5, -0.0, 0.0, 1e-40, 0.5, 1.0, 3e38, f32::INFINITY];
        for v in vals {
            for a in vals {
                for b in vals {
                    assert_eq!(clamp_between(v, a, b).to_bits(), v.clamp(a.min(b), a.max(b)).to_bits(), "{v} {a} {b}");
                }
            }
        }
    }

    #[test]
    fn reflect_indices() {
        assert_eq!(reflect(-1, 5), 1);
        assert_eq!(reflect(-2, 5), 2);
        assert_eq!(reflect(5, 5), 3);
        assert_eq!(reflect(6, 5), 2);
        assert_eq!(reflect(3, 1), 0);
        assert_eq!(reflect(-7, 2), 1);
    }

    #[test]
    #[ignore]
    fn throughput() {
        let (w, h) = (6000, 4000);
        let img = smooth_scene(w, h);
        let cfa = Cfa::bayer("RGGB").unwrap();
        let n = mosaic_from_rgb(&img, &cfa);
        for m in [Method::Bilinear, Method::Ppg, Method::Ahd] {
            let t = std::time::Instant::now();
            let _ = demosaic(&n, m);
            let s = t.elapsed().as_secs_f64();
            println!("{m:?}: {:.1} MP/s ({:.0} ms for 24 MP)", (w * h) as f64 / 1e6 / s, s * 1e3);
        }
        let n = mosaic_from_rgb(&img, &Cfa::xtrans());
        let t = std::time::Instant::now();
        let _ = demosaic(&n, Method::Ahd);
        println!("X-Trans directional: {:.1} MP/s", (w * h) as f64 / 1e6 / t.elapsed().as_secs_f64());
    }
}
