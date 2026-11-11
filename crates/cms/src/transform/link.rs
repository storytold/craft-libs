//! Linking two profiles through the PCS: stage chains, PCS adjustment and black point.

use super::*;

// ------------------------------------------------------------ linking

/// Stages from `src` device values to `dst` device values.
pub(crate) fn link_stages(src: &Profile, dst: &Profile, intent: Intent, bpc: bool) -> Result<Vec<Stage>, CmsError> {
    for p in [src, dst] {
        // A Lab DeviceLink stands in for an abstract profile only when it also ends in Lab.
        let lab_to_lab =
            p.color_space == ColorSpace::Lab && p.a2b.iter().flatten().all(|l| l.outputs == 3) && p.b2a.iter().flatten().all(|l| l.inputs == 3);
        if matches!(p.class, ProfileClass::DeviceLink | ProfileClass::NamedColor | ProfileClass::Abstract) && !lab_to_lab {
            return Err(CmsError::Unsupported(format!("{:?} profiles cannot be used as a source or destination", p.class)));
        }
    }
    let (mut stages, s_pcs) = src.device_to_pcs(intent)?;
    let (d_stages, d_pcs) = dst.pcs_to_device(intent)?;
    let adjust = connection(src, dst, intent, bpc)?;
    match adjust {
        Some((scale, offset)) => {
            if s_pcs == Pcs::Lab {
                stages.push(Stage::LabToXyz);
            }
            stages.push(Stage::Matrix {
                rows: 3,
                cols: 3,
                m: vec![scale[0], 0.0, 0.0, 0.0, scale[1], 0.0, 0.0, 0.0, scale[2]],
                offset: offset.to_vec(),
            });
            if d_pcs == Pcs::Lab {
                stages.push(Stage::XyzToLab);
            }
        }
        None => match (s_pcs, d_pcs) {
            (Pcs::Lab, Pcs::Xyz) => stages.push(Stage::LabToXyz),
            (Pcs::Xyz, Pcs::Lab) => stages.push(Stage::XyzToLab),
            _ => {}
        },
    }
    stages.extend(d_stages);
    Ok(stages)
}

/// Per-component XYZ scale and offset applied in the PCS: absolute colorimetric media-white
/// scaling or black point compensation. `None` = identity.
/// Per-component XYZ `(scale, offset)`.
type Affine3 = ([f64; 3], [f64; 3]);

fn connection(src: &Profile, dst: &Profile, intent: Intent, bpc: bool) -> Result<Option<Affine3>, CmsError> {
    if intent == Intent::AbsoluteColorimetric {
        let (ws, wd) = (src.media_white(), dst.media_white());
        // `wtpt` comes straight from the file. A zero, negative or non-finite component would
        // make the scale NaN or infinite and poison every colour the transform produces, so a
        // profile with such a media white is refused for this intent instead.
        let usable = |w: &[f64; 3]| w.iter().all(|v| v.is_finite() && *v > 0.0);
        if !usable(&ws) || !usable(&wd) {
            return Err(CmsError::Invalid(format!("absolute colorimetric needs a positive media white point (source {ws:?}, destination {wd:?})")));
        }
        let s = [ws[0] / wd[0], ws[1] / wd[1], ws[2] / wd[2]];
        if s.iter().all(|v| (v - 1.0).abs() < 1e-6) {
            return Ok(None);
        }
        return Ok(Some((s, [0.0; 3])));
    }
    if !bpc {
        return Ok(None);
    }
    let bs = black_point(src, intent, true)?;
    let bd = black_point(dst, intent, false)?;
    if (bs - bd).abs() < 1e-6 {
        return Ok(None);
    }
    // Neutral black points (luminance only, as in Adobe's BPC), mapped linearly in XYZ so that
    // the source black lands on the destination black and the white stays fixed.
    let w = math::D50;
    let mut scale = [1.0; 3];
    let mut offset = [0.0; 3];
    for i in 0..3 {
        let (s, d) = (bs * w[i], bd * w[i]);
        let a = (w[i] - d) / (w[i] - s);
        scale[i] = a;
        offset[i] = w[i] * (1.0 - a);
    }
    Ok(Some((scale, offset)))
}

/// Luminance (Y, D50-relative) of the profile's black point for `intent`.
///
/// Follows the structure of Adobe's published BPC algorithm: matrix/TRC profiles use device
/// zero; v4 LUT profiles in perceptual/saturation use the perceptual reference black; other LUT
/// profiles use the darkest colour the device reaches (Lab 0 round-tripped through BToA/AToB,
/// or device black for additive spaces). The destination-side curve fitting of the paper is
/// not implemented; the round-trip black is used directly.
pub fn black_point(p: &Profile, intent: Intent, _input: bool) -> Result<f64, CmsError> {
    if matches!(p.color_space, ColorSpace::Lab | ColorSpace::Xyz) {
        return Ok(0.0);
    }
    let lut_based = p.a2b.iter().any(Option::is_some);
    if lut_based && p.version.0 >= 4 && matches!(intent, Intent::Perceptual | Intent::Saturation) {
        return Ok(PERCEPTUAL_BLACK[1]);
    }
    let pcs_y = |st: Vec<Stage>, pcs: Pcs, dev: &[f32]| -> Result<f64, CmsError> {
        let pipe = Pipeline::new(p.channels(), st)?;
        let mut o = [0.0f32; 16];
        pipe.eval(dev, &mut o)?;
        let y = match pcs {
            Pcs::Xyz => o[1] as f64,
            Pcs::Lab => math::lab_to_xyz([o[0] as f64, 0.0, 0.0], math::D50)[1],
        };
        Ok(y.clamp(0.0, 0.5))
    };
    let (to_pcs, pcs) = p.device_to_pcs(intent)?;
    let additive = matches!(p.color_space, ColorSpace::Rgb | ColorSpace::Gray);
    if additive || !lut_based {
        let zero = vec![0.0f32; p.channels()];
        return pcs_y(to_pcs, pcs, &zero);
    }
    // Subtractive LUT profile: darkest reachable colour.
    let colorimetric = if intent == Intent::Saturation { Intent::Saturation } else { Intent::RelativeColorimetric };
    match p.pcs_to_device(colorimetric) {
        Ok((from_pcs, fpcs)) => {
            let mut st = Vec::new();
            if fpcs == Pcs::Xyz {
                st.push(Stage::LabToXyz);
            }
            st.extend(from_pcs);
            let pipe = Pipeline::new(3, st)?;
            let mut dev = [0.0f32; 16];
            pipe.eval(&[0.0, 0.0, 0.0], &mut dev)?;
            pcs_y(to_pcs, pcs, dev.get(..p.channels()).unwrap_or(&dev))
        }
        Err(_) => {
            let ones = vec![1.0f32; p.channels()];
            pcs_y(to_pcs, pcs, &ones)
        }
    }
}
