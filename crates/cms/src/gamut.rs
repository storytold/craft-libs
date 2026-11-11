//! Gamut checks against a proof (output) profile: Photoshop's View › Gamut Warning.

use crate::clut::Clut;
use crate::pipeline::{Pipeline, Stage};
use crate::profile::Profile;
use crate::transform::link_stages;
use crate::{CmsError, Intent, check_len};

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
    /// against across the whole space. About half the ΔE76 number for the same visual difference,
    /// so the usual warning threshold is 2.0 rather than 4.0.
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

/// Precomputed out-of-gamut test for colours of `src` against `proof`.
///
/// A colour is out of gamut when sending it to the proof device (relative colorimetric,
/// clipping) and back to Lab moves it by more than the threshold. The difference is tabulated
/// over the source device space (33³ grid for 3 channels, 17⁴ for 4, exact otherwise), in the
/// [`Metric`] the check was built with.
#[derive(Debug)]
pub struct GamutCheck {
    inputs: usize,
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

impl GamutCheck {
    /// A check measuring in ΔE76, as this has always done.
    pub fn new(src: &Profile, proof: &Profile, threshold: f32) -> Result<GamutCheck, CmsError> {
        GamutCheck::with_metric(src, proof, Metric::De76, threshold)
    }

    /// A check measuring in `metric`. [`Metric::default_threshold`] gives the threshold that
    /// metric is usually read against.
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
        Ok(GamutCheck { inputs: n, grid, exact: (direct, via), metric, threshold })
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
                o[0] = difference(self.metric, &a, &b);
            }
        }
        Ok(o[0])
    }

    pub fn out_of_gamut(&self, px: &[f32]) -> Result<bool, CmsError> {
        Ok(self.delta_e(px)? > self.threshold)
    }
}
