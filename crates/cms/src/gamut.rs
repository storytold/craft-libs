//! Gamut checks against a proof (output) profile: Photoshop's View › Gamut Warning.

use crate::clut::Clut;
use crate::pipeline::{Pipeline, Stage};
use crate::profile::Profile;
use crate::transform::link_stages;
use crate::{CmsError, Intent, check_len, math};

/// Default ΔE76 threshold above which a colour counts as out of gamut.
pub const DEFAULT_THRESHOLD: f32 = 4.0;

/// Colour-difference metric used for a proof round trip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// CIE 1976 ΔE*ab (the default for [`GamutCheck::new`]).
    De76,
    /// CIEDE2000 with kL = kC = kH = 1.
    De2000,
}

/// Precomputed out-of-gamut test for colours of `src` against `proof`.
///
/// A colour is out of gamut when sending it to the proof device (relative colorimetric,
/// clipping) and back to Lab moves it by more than the threshold in the selected metric.
/// The ΔE is tabulated over the source device space (256 points for 1 channel, 33³ for 3,
/// 17⁴ for 4, exact otherwise). Between grid nodes the result is interpolated.
#[derive(Debug)]
pub struct GamutCheck {
    inputs: usize,
    grid: Option<Clut>,
    exact: (Pipeline, Pipeline),
    metric: Metric,
    /// Warning threshold in the selected metric's units.
    pub threshold: f32,
}

impl GamutCheck {
    /// Builds a gamut check using ΔE76 and the supplied threshold.
    pub fn new(src: &Profile, proof: &Profile, threshold: f32) -> Result<GamutCheck, CmsError> {
        Self::with_metric(src, proof, Metric::De76, threshold)
    }

    /// Builds a gamut check with `threshold` in the chosen metric's units.
    ///
    /// The metric is fixed when the lookup table is built. Thresholds are not interchangeable
    /// between metrics; [`DEFAULT_THRESHOLD`] remains the ΔE76 default.
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
            o[0] = match metric {
                Metric::De76 => {
                    let d = |i: usize, s: f32| (a[i] - b[i]) * s;
                    (d(0, 100.0).powi(2) + d(1, 255.0).powi(2) + d(2, 255.0).powi(2)).sqrt()
                }
                Metric::De2000 => delta_e2000(&a, &b),
            };
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

    /// ΔE in the selected metric between the colour (at least `inputs` normalised values)
    /// and its proof round trip.
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
                o[0] = match self.metric {
                    Metric::De76 => ((a[0] - b[0]) * 100.0).hypot((a[1] - b[1]) * 255.0).hypot((a[2] - b[2]) * 255.0),
                    Metric::De2000 => delta_e2000(&a, &b),
                };
            }
        }
        Ok(o[0])
    }

    pub fn out_of_gamut(&self, px: &[f32]) -> Result<bool, CmsError> {
        Ok(self.delta_e(px)? > self.threshold)
    }
}

fn delta_e2000(a: &[f32], b: &[f32]) -> f32 {
    let lab = |v: &[f32]| [v[0] as f64 * 100.0, v[1] as f64 * 255.0 - 128.0, v[2] as f64 * 255.0 - 128.0];
    math::delta_e2000(lab(a), lab(b)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{ColorSpace, Lut, LutKind, ProfileClass};
    use crate::{Builtin, math};

    fn constant_lab_profile(inputs: usize, lab: [f64; 3]) -> Profile {
        let mut p = Builtin::LabD50.profile().clone();
        p.class = ProfileClass::Input;
        p.color_space = if inputs == 1 { ColorSpace::Gray } else { ColorSpace::Color(inputs as u8) };
        p.a2b[1] = Some(Lut {
            kind: LutKind::Ab,
            inputs,
            outputs: 3,
            stages: vec![Stage::Matrix {
                rows: 3,
                cols: inputs,
                m: vec![0.0; 3 * inputs],
                offset: vec![lab[0] / 100.0, (lab[1] + 128.0) / 255.0, (lab[2] + 128.0) / 255.0],
            }],
        });
        p
    }

    #[test]
    fn selected_metric_reaches_sampled_and_exact_paths() -> Result<(), CmsError> {
        // Sharma reference pair 1: the same known difference through each channel-count route.
        let a = [50.0, 2.6772, -79.7751];
        let b = [50.0, 0.0, -82.7485];
        let mut proof = constant_lab_profile(3, b);
        proof.color_space = ColorSpace::Lab;
        for inputs in [1, 2, 3, 4] {
            let src = constant_lab_profile(inputs, a);
            let legacy = GamutCheck::new(&src, &proof, DEFAULT_THRESHOLD)?;
            let de76 = GamutCheck::with_metric(&src, &proof, Metric::De76, DEFAULT_THRESHOLD)?;
            let mut de2000 = GamutCheck::with_metric(&src, &proof, Metric::De2000, DEFAULT_THRESHOLD)?;
            let grid = match inputs {
                1 => Some(vec![256]),
                3 => Some(vec![33; 3]),
                4 => Some(vec![17; 4]),
                _ => None,
            };
            assert_eq!(de2000.grid.as_ref().map(|g| g.grid().to_vec()), grid);
            assert_eq!(de2000.inputs(), inputs);
            for value in [0.0, 0.37, 1.0] {
                let px = vec![value; inputs];
                assert_eq!(legacy.delta_e(&px)?, de76.delta_e(&px)?);
                assert!((de76.delta_e(&px)? as f64 - math::delta_e76(a, b)).abs() < 1e-4);
                assert!((de2000.delta_e(&px)? - 2.0425).abs() < 1e-4);
                assert!(legacy.out_of_gamut(&px)?);
                assert!(!de2000.out_of_gamut(&px)?);
            }
            let px = vec![0.5; inputs];
            de2000.threshold = de2000.delta_e(&px)?;
            assert!(!de2000.out_of_gamut(&px)?);
            de2000.threshold -= 0.01;
            assert!(de2000.out_of_gamut(&px)?);
            assert!(matches!(de2000.delta_e(&px[..inputs - 1]), Err(CmsError::InvalidArgument(_))));
        }
        Ok(())
    }

    #[test]
    fn de2000_grid_tracks_exact_display_p3_to_srgb_roundtrips() -> Result<(), CmsError> {
        let g = GamutCheck::with_metric(Builtin::DisplayP3.profile(), Builtin::Srgb.profile(), Metric::De2000, 2.0)?;
        let exact = |px: &[f32]| {
            let (mut a, mut b) = ([0.0; 3], [0.0; 3]);
            g.exact.0.run(px, &mut a);
            g.exact.1.run(px, &mut b);
            let decode = |l: [f32; 3]| [l[0] as f64 * 100.0, l[1] as f64 * 255.0 - 128.0, l[2] as f64 * 255.0 - 128.0];
            math::delta_e2000(decode(a), decode(b)) as f32
        };
        for r in [0.0, 0.5, 1.0] {
            for g0 in [0.0, 0.5, 1.0] {
                for b in [0.0, 0.5, 1.0] {
                    let px = [r, g0, b];
                    assert!((g.delta_e(&px)? - exact(&px)).abs() < 1e-4, "node {px:?}");
                }
            }
        }
        for r in [0.03, 0.17, 0.37, 0.61, 0.79, 0.97] {
            for g0 in [0.03, 0.17, 0.37, 0.61, 0.79, 0.97] {
                for b in [0.03, 0.17, 0.37, 0.61, 0.79, 0.97] {
                    let px = [r, g0, b];
                    let error = (g.delta_e(&px)? - exact(&px)).abs();
                    assert!(error < 0.5, "interpolated {px:?}: error {error}");
                }
            }
        }
        Ok(())
    }
}
