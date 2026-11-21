//! Gamut checks against a proof (output) profile: Photoshop's View › Gamut Warning.

use crate::clut::Clut;
use crate::pipeline::{Pipeline, Stage};
use crate::profile::{ColorSpace, Profile};
use crate::transform::link_stages;
use crate::{CmsError, Intent, check_len, invalid_arg};

/// Default ΔE76 threshold above which a colour counts as out of gamut.
pub const DEFAULT_THRESHOLD: f32 = 4.0;

/// Which colour-difference formula a gamut check measures the round trip with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Metric {
    /// ΔE*ab (CIE 1976): the Euclidean distance in Lab. It overstates differences in saturated
    /// colours, blues most, and understates them near neutrals, so one threshold flags too much
    /// in one part of the space and too little in another.
    #[default]
    De76,
    /// ΔE00 (CIEDE2000, CIE 142-2001): the weighted difference, which one threshold can be set
    /// against across the whole space. Its usual warning threshold is 2.0 (see
    /// [`Metric::default_threshold`]). That is a default of its own, not a conversion: a ΔE76
    /// threshold can't be halved into a ΔE00 one, since the two disagree by different amounts in
    /// different parts of the space, and near neutrals ΔE00 can be the larger.
    De2000,
}

impl Metric {
    /// The threshold a gamut warning is usually drawn at in this metric.
    pub const fn default_threshold(self) -> f32 {
        match self {
            Metric::De76 => DEFAULT_THRESHOLD,
            Metric::De2000 => 2.0,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
const PAR_MIN_PIXELS: usize = 16 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const PAR_CHUNK_PIXELS: usize = 16 * 1024;

/// Precomputed out-of-gamut test for colours of `src` against `proof`.
///
/// A colour is out of gamut when sending it to the proof device (relative colorimetric,
/// clipping) and back to Lab moves it by more than the threshold. The difference is tabulated
/// over the source device space (256 points for 1 channel, a 33³ grid for 3 channels, 17⁴ for
/// 4), and [`GamutCheck::delta_e`] interpolates between the grid's nodes; for other channel
/// counts it is computed exactly. Either way it is in the [`Metric`] the check was built with.
#[derive(Debug)]
pub struct GamutCheck {
    inputs: usize,
    source_space: ColorSpace,
    grid: Option<Clut>,
    exact: (Pipeline, Pipeline),
    metric: Metric,
    pub threshold: f32,
}

/// The difference between two Lab colours in the ICC v4 encoding the pipelines here emit
/// (L/100, (a+128)/255), in `metric`.
fn difference(metric: Metric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        // The scaled differences, as before: the offsets cancel, and the result is the same
        // number this reported before there was a choice of metric, bit for bit.
        Metric::De76 => {
            let d = |i: usize, s: f32| (a[i] - b[i]) * s;
            (d(0, 100.0).powi(2) + d(1, 255.0).powi(2) + d(2, 255.0).powi(2)).sqrt()
        }
        // ΔE00 weights lightness, chroma and hue, so it needs the colours themselves.
        Metric::De2000 => {
            let lab = |v: &[f32]| [f64::from(v[0]) * 100.0, f64::from(v[1]) * 255.0 - 128.0, f64::from(v[2]) * 255.0 - 128.0];
            crate::math::delta_e2000(lab(a), lab(b)) as f32
        }
    }
}

/// [`difference`] for the exact path: ΔE76 as chained `hypot`s, as this computed it before there
/// was a choice of metric, so the number is the same bit for bit (review). The grid has always
/// sampled the square-root sum, which `difference` keeps.
fn exact_difference(metric: Metric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        Metric::De76 => ((a[0] - b[0]) * 100.0).hypot((a[1] - b[1]) * 255.0).hypot((a[2] - b[2]) * 255.0),
        Metric::De2000 => difference(metric, a, b),
    }
}

impl GamutCheck {
    /// A check measuring in ΔE76, as this has always done.
    pub fn new(src: &Profile, proof: &Profile, threshold: f32) -> Result<GamutCheck, CmsError> {
        GamutCheck::with_metric(src, proof, Metric::De76, threshold)
    }

    /// A check measuring in `metric`. [`Metric::default_threshold`] gives the threshold that
    /// metric is usually read against; the caller passes the threshold, none is chosen here.
    ///
    /// ```
    /// use craft_cms::{Builtin, GamutCheck, Metric};
    /// let check = GamutCheck::with_metric(Builtin::Srgb.profile(), Builtin::CoatedCmyk.profile(), Metric::De2000, Metric::De2000.default_threshold()).unwrap();
    /// assert!(check.out_of_gamut(&[0.0, 0.0, 1.0]).unwrap(), "sRGB blue is outside coated CMYK");
    /// ```
    ///
    /// # Errors
    /// [`CmsError`] when a conversion the check needs can't be built: `src` to Lab, `src` to
    /// `proof`, or `proof` back to Lab. A proof profile with a usable device-to-PCS transform but
    /// no usable reverse one fails at the second.
    pub fn with_metric(src: &Profile, proof: &Profile, metric: Metric, threshold: f32) -> Result<GamutCheck, CmsError> {
        let lab = crate::Builtin::LabD50.profile();
        let direct = Pipeline::new(src.channels(), link_stages(src, lab, Intent::RelativeColorimetric, false)?)?;
        let mut via = link_stages(src, proof, Intent::RelativeColorimetric, false)?;
        via.push(Stage::Clamp01);
        via.extend(link_stages(proof, lab, Intent::RelativeColorimetric, false)?);
        let via = Pipeline::new(src.channels(), via)?;
        let de = |p: &[f32], o: &mut [f32]| {
            let (mut a, mut b) = ([0.0f32; 16], [0.0f32; 16]);
            direct.run(p, &mut a);
            via.run(p, &mut b);
            o[0] = difference(metric, &a, &b);
        };
        let n = src.channels();
        let grid = match n {
            1 => Some(Clut::sample(vec![256], 1, de)?),
            3 => Some(Clut::sample(vec![33; 3], 1, de)?),
            4 => Some(Clut::sample(vec![17; 4], 1, de)?),
            _ => None,
        };
        Ok(GamutCheck { inputs: n, source_space: src.color_space, grid, exact: (direct, via), metric, threshold })
    }

    pub fn inputs(&self) -> usize {
        self.inputs
    }

    /// The metric this check measures in.
    pub fn metric(&self) -> Metric {
        self.metric
    }

    /// The difference between the colour (at least `inputs` normalised values) and its proof
    /// round trip, in [`GamutCheck::metric`].
    pub fn delta_e(&self, px: &[f32]) -> Result<f32, CmsError> {
        check_len("gamut check input", px.len(), self.inputs)?;
        let mut o = [0.0f32; 1];
        match &self.grid {
            Some(g) => {
                let mut c = [0.0f32; 16];
                for (d, s) in c.iter_mut().zip(&px[..self.inputs]) {
                    *d = s.clamp(0.0, 1.0);
                }
                g.interp(&c[..self.inputs], &mut o);
            }
            None => {
                let (mut a, mut b) = ([0.0f32; 16], [0.0f32; 16]);
                self.exact.0.run(px, &mut a);
                self.exact.1.run(px, &mut b);
                o[0] = exact_difference(self.metric, &a, &b);
            }
        }
        Ok(o[0])
    }

    pub fn out_of_gamut(&self, px: &[f32]) -> Result<bool, CmsError> {
        Ok(self.delta_e(px)? > self.threshold)
    }

    /// Out-of-gamut mask for RGBA pixels: `255` where the pixel has alpha > 0 and its RGB
    /// channels are out of gamut, `0` elsewhere. Returns the number of flagged pixels. Only
    /// `min(px.len(), mask.len())` pixels are processed. Runs on the rayon pool (not on wasm);
    /// the result is identical to calling [`Self::out_of_gamut`] per pixel.
    ///
    /// # Errors
    /// [`CmsError::InvalidArgument`] when the check was not built from an RGB source profile: an
    /// RGBA pixel carries only RGB colour channels, so it cannot be tested against a CMYK, gray
    /// or Lab source. Nothing is written to `mask` in that case.
    pub fn mask_rgba(&self, px: &[[f32; 4]], mask: &mut [u8]) -> Result<usize, CmsError> {
        if self.source_space != ColorSpace::Rgb || self.inputs != 3 {
            return Err(invalid_arg(format!("gamut mask needs an RGB source profile, got {:?} with {} channels", self.source_space, self.inputs)));
        }
        let n = px.len().min(mask.len());
        let (Some(px), Some(mask)) = (px.get(..n), mask.get_mut(..n)) else { return Ok(0) };
        let run = |p: &[[f32; 4]], m: &mut [u8]| {
            let mut count = 0;
            for (p, m) in p.iter().zip(m.iter_mut()) {
                // The source has three channels (checked above), so the check cannot fail.
                let hit = p[3] > 0.0 && self.out_of_gamut(&[p[0], p[1], p[2]]).unwrap_or(false);
                *m = if hit { 255 } else { 0 };
                count += usize::from(hit);
            }
            count
        };
        #[cfg(not(target_arch = "wasm32"))]
        if n >= PAR_MIN_PIXELS {
            use rayon::prelude::*;
            return Ok(px.par_chunks(PAR_CHUNK_PIXELS).zip(mask.par_chunks_mut(PAR_CHUNK_PIXELS)).map(|(p, m)| run(p, m)).sum());
        }
        Ok(run(px, mask))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Builtin;

    fn image(n: usize) -> Vec<[f32; 4]> {
        let mut s = 0x9E37_79B9u32;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 8) as f32 / (1u32 << 24) as f32
        };
        // Includes out-of-range values and zero alpha.
        (0..n).map(|i| [rnd() * 1.2 - 0.1, rnd(), rnd(), if i % 7 == 0 { 0.0 } else { rnd() }]).collect()
    }

    #[test]
    fn mask_rgba_matches_per_pixel_check() {
        let src = Builtin::Srgb.profile();
        // Sizes straddle the parallel threshold and the chunk size, odd ones included.
        for (proof, threshold) in [(Builtin::CoatedCmyk, 4.0), (Builtin::CoatedCmyk, 1.5), (Builtin::GrayGamma22, 4.0), (Builtin::DisplayP3, 0.5)] {
            let check = GamutCheck::new(src, proof.profile(), threshold).unwrap();
            for n in [0, 1, 1023, 16 * 1024 - 1, 16 * 1024, 16 * 1024 + 1, 50_001] {
                let px = image(n);
                let want: Vec<u8> = px.iter().map(|p| if p[3] > 0.0 && check.out_of_gamut(&p[..3]).unwrap() { 255 } else { 0 }).collect();
                let mut got = vec![7u8; n];
                let count = check.mask_rgba(&px, &mut got).unwrap();
                assert_eq!(got, want, "{proof:?} threshold {threshold} n {n}");
                assert_eq!(count, want.iter().filter(|&&m| m == 255).count());
            }
        }
    }

    /// The exact path's ΔE76 is the chained `hypot` it always was, bit for bit, and its ΔE00 is
    /// the same as the grid's formula.
    #[test]
    fn the_exact_de76_is_the_chained_hypot() {
        let mut s = 0x2545_F491u32;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 8) as f32 / (1u32 << 24) as f32
        };
        for _ in 0..10_000 {
            let (a, b) = ([rnd(), rnd(), rnd()], [rnd(), rnd(), rnd()]);
            let hypot = ((a[0] - b[0]) * 100.0).hypot((a[1] - b[1]) * 255.0).hypot((a[2] - b[2]) * 255.0);
            assert_eq!(exact_difference(Metric::De76, &a, &b).to_bits(), hypot.to_bits(), "{a:?} {b:?}");
            assert_eq!(exact_difference(Metric::De2000, &a, &b).to_bits(), difference(Metric::De2000, &a, &b).to_bits());
        }
    }

    #[test]
    fn mask_rgba_rejects_non_rgb_source() {
        // A CMYK (4-channel), gray (1-channel) and Lab (3-channel, not RGB) source cannot be tested from RGBA pixels.
        for src in [Builtin::CoatedCmyk, Builtin::GrayGamma22, Builtin::LabD50] {
            let check = GamutCheck::new(src.profile(), Builtin::Srgb.profile(), 4.0).unwrap();
            let px = image(64);
            let mut mask = vec![9u8; 64];
            assert!(matches!(check.mask_rgba(&px, &mut mask), Err(CmsError::InvalidArgument(_))), "{src:?}");
            assert!(mask.iter().all(|&m| m == 9), "mask must stay untouched");
        }
    }
}
