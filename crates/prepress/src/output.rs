//! The plates as they print, in a chosen output profile: process inks convert from the press's
//! CMYK straight to the output (CMYK → PCS → output, not through sRGB), and spot inks combine
//! in the output space.

use std::sync::Arc;

use craft_cms::{Builtin, ColorSpace, Intent, Profile, Transform, TransformOptions, cached};

use crate::cms::encode_lab;
use crate::composite::{cmyk_ramp, lab_ramp, on};
use crate::coverage::filled;
use crate::{Alternate, InkId, Plates, Process};

/// Pixels converted per call, to bound the scratch buffer.
const CHUNK: usize = 16 * 1024;

/// Why an [`Output`] can't be built.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OutputError {
    #[error("the press profile is not CMYK")]
    PressNotCmyk,
    #[error("the output profile is not gray, RGB or CMYK")]
    UnsupportedOutput,
    #[error("craft-cms can't convert into the output profile: {0}")]
    Transform(String),
}

/// A conversion from a press (the CMYK printing condition the plates were made for) to an
/// output profile, such as a printer's. Built once, used for every composite.
pub struct Output {
    process: Arc<Transform>,
    from_lab: Arc<Transform>,
    channels: usize,
    subtractive: bool,
}

impl Output {
    /// Press CMYK into `output` (gray, RGB or CMYK) at `intent`, with black point compensation
    /// when `bpc` (craft-cms ignores it for absolute colorimetric). Relative colorimetric with
    /// `bpc` maps the press's paper and black onto the output's; absolute colorimetric keeps
    /// the press paper's tint.
    pub fn new(press: &Profile, output: &Profile, intent: Intent, bpc: bool) -> Result<Self, OutputError> {
        if press.color_space != ColorSpace::Cmyk {
            return Err(OutputError::PressNotCmyk);
        }
        let subtractive = match output.color_space {
            ColorSpace::Gray | ColorSpace::Rgb => false,
            ColorSpace::Cmyk => true,
            _ => return Err(OutputError::UnsupportedOutput),
        };
        // The fast path's tables: a composite converts every pixel of a page.
        let opts = TransformOptions { intent, bpc, precise_float: false };
        let failed = |e: craft_cms::CmsError| OutputError::Transform(format!("{e:?}"));
        let process = cached(press, output, opts).map_err(failed)?;
        let from_lab = cached(Builtin::LabD50.profile(), output, opts).map_err(failed)?;
        Ok(Self { process, from_lab, channels: output.channels(), subtractive })
    }

    /// Bytes per pixel of the output: 1 gray, 3 RGB, 4 CMYK.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The spot's appearance alone at each tint byte, `channels` bytes per tint (see
    /// [`appearance`](crate::composite::appearance)); `None` when it can't convert.
    fn ramp(&self, alternate: Alternate) -> Option<Vec<u8>> {
        let mut out = filled(256 * self.channels, 0_u8)?;
        match alternate {
            Alternate::Lab(lab) => {
                let encoded: Vec<f32> = lab_ramp(lab).as_chunks::<3>().0.iter().flat_map(|c| encode_lab(*c)).collect();
                let mut values = filled(out.len(), 0.0_f32)?;
                self.from_lab.convert_f32(&encoded, 3, &mut values, self.channels, false).ok()?;
                for (byte, v) in out.iter_mut().zip(values) {
                    *byte = if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
                }
            }
            Alternate::Cmyk(solid) => self.process.convert_u8(&cmyk_ramp(solid), 4, &mut out, self.channels, false).ok()?,
        }
        Some(out)
    }

    /// `ink` printed over `px`: a multiply of an additive output's values, and of a
    /// subtractive one's complements (ISO 32000-2 §11.3.4), so inks add.
    fn combine(&self, px: &mut [u8], ink: &[u8]) {
        for (c, s) in px.iter_mut().zip(ink) {
            let (c16, s16) = (u16::from(*c), u16::from(*s));
            *c = if self.subtractive { (255 - ((255 - c16) * (255 - s16) + 127) / 255) as u8 } else { ((c16 * s16 + 127) / 255) as u8 };
        }
    }
}

impl Plates {
    /// The enabled inks as they print, in `output`'s space ([`Output::channels`] bytes per
    /// pixel, row-major, `width × height` pixels). `enabled` is as for
    /// [`Plates::composite`].
    ///
    /// The process plates convert from the press profile `output` was built with, directly.
    /// Each spot plate then combines over them with the spot's appearance at its tint, from its
    /// alternate, converted into the output; spots whose alternate can't convert are left out.
    /// The overprint decisions are the plates'. Empty when the buffer overflows, can't be
    /// allocated, or craft-cms can't convert the process inks.
    pub fn composite_to(&self, enabled: &[bool], output: &Output) -> Vec<u8> {
        let n = output.channels;
        let Some(mut out) = self.pixels().and_then(|p| p.checked_mul(n)).and_then(|len| filled(len, 0_u8)) else {
            return Vec::new();
        };
        let process: Vec<Option<&[u8]>> = Process::ALL
            .iter()
            .map(|p| {
                let id = InkId::process(*p);
                self.plane(id).filter(|_| on(enabled, id.index()))
            })
            .collect();
        let mut cmyk = Vec::with_capacity(CHUNK * 4);
        for (c, chunk) in out.chunks_mut(CHUNK * n).enumerate() {
            let start = c * CHUNK;
            cmyk.clear();
            for i in start..start + chunk.len() / n {
                cmyk.extend(process.iter().map(|plane| plane.and_then(|p| p.get(i)).copied().unwrap_or(0)));
            }
            if output.process.convert_u8(&cmyk, 4, chunk, n, false).is_err() {
                return Vec::new();
            }
        }

        for (id, spot) in self.inks.spots().filter(|(id, _)| on(enabled, id.index())) {
            let Some(plane) = self.plane(id).filter(|p| p.iter().any(|t| *t > 0)) else { continue };
            let Some(ramp) = output.ramp(spot.alternate) else { continue };
            for (px, tint) in out.chunks_exact_mut(n).zip(plane) {
                let at = usize::from(*tint) * n;
                if let Some(ink) = ramp.get(at..at + n).filter(|_| *tint > 0) {
                    output.combine(px, ink);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use craft_cms::{Builtin, Intent, builtin};

    use super::{Output, OutputError};
    use crate::cms::srgb8_to_lab;
    use crate::coverage::tests::synthetic;
    use crate::{Alternate, CmsConversion, Conversion, Ink, InkSet, Plates, Spot};

    fn fogra39() -> &'static craft_cms::Profile {
        Builtin::CoatedFogra39.profile()
    }

    fn near(got: &[u8], expected: &[u8], tolerance: u8) -> bool {
        got.len() == expected.len() && got.iter().zip(expected).all(|(g, e)| g.abs_diff(*e) <= tolerance)
    }

    #[test]
    fn a_cmyk_output_of_the_press_profile_keeps_the_press_inks() {
        // Cyan, red and a light blue come back within the profile's own round trip. Through
        // sRGB, cyan is out of gamut and comes back as [201, 59, 0, 0]. (Black is left out: the
        // output's BToA tables generate their own black, as they would for a printer.)
        let p = synthetic(3, 1, [70.0, 5.0, 60.0], [vec![255, 0, 64], vec![0, 255, 32], vec![0, 255, 0], vec![0; 3], vec![0; 3]]);
        let output = Output::new(fogra39(), fogra39(), Intent::RelativeColorimetric, false).unwrap();
        assert_eq!(output.channels(), 4);
        let cmyk = p.composite_to(&[], &output);
        assert!(near(&cmyk[0..4], &[255, 0, 0, 0], 12), "cyan: {:?}", &cmyk[0..4]);
        assert!(near(&cmyk[4..8], &[0, 255, 255, 0], 12), "red: {:?}", &cmyk[4..8]);
        assert!(near(&cmyk[8..12], &[64, 32, 0, 0], 12), "light blue: {:?}", &cmyk[8..12]);
    }

    #[test]
    fn an_srgb_output_matches_the_preview_composite() {
        const GOLD: [u8; 3] = [212, 175, 55];
        let lab = srgb8_to_lab(GOLD).unwrap();
        // Black alone, Gold alone, Gold over black, Gold at 50%.
        let p = synthetic(4, 1, lab, [vec![0; 4], vec![0; 4], vec![0; 4], vec![255, 0, 255, 0], vec![0, 255, 255, 128]]);
        let output = Output::new(fogra39(), builtin::srgb(), Intent::RelativeColorimetric, true).unwrap();
        let rgb = p.composite_to(&[], &output);
        let preview = p.composite(&[], &CmsConversion::fogra39());
        assert!(near(&rgb, &preview, 2), "{rgb:?} {preview:?}");
        assert!(near(&p.composite_to(&[true, true, true, false], &output)[0..3], &[255, 255, 255], 1), "black hidden");
    }

    #[test]
    fn a_spot_adds_its_ink_in_a_cmyk_output() {
        let mut inks = InkSet::new();
        inks.add(Ink::Spot(Spot { name: "Rubine".into(), alternate: Alternate::Cmyk([0.0, 1.0, 0.0, 0.0]), solidity: None }));
        // Cyan with the spot over it, and the spot alone.
        let planes = vec![vec![255, 0], vec![0; 2], vec![0; 2], vec![0; 2], vec![255, 255]];
        let p = Plates { inks, unseparated: Vec::new(), width: 2, height: 1, dpi: 72.0, conversion: Conversion::DeviceFormulas, planes };
        let output = Output::new(fogra39(), fogra39(), Intent::RelativeColorimetric, false).unwrap();
        let cmyk = p.composite_to(&[], &output);
        // Magenta's round trip through the profile adds a little yellow.
        assert!(near(&cmyk[0..4], &[255, 255, 8, 0], 8), "spot over cyan: {:?}", &cmyk[0..4]);
        assert!(near(&cmyk[4..8], &[0, 255, 8, 0], 8), "spot alone: {:?}", &cmyk[4..8]);
        assert!(near(&p.composite_to(&[true, true, true, true, false], &output)[0..4], &[255, 0, 0, 0], 6), "spot hidden");
    }

    #[test]
    fn outputs_need_a_cmyk_press_and_a_gray_rgb_or_cmyk_output() {
        let lab = Builtin::LabD50.profile();
        let srgb = builtin::srgb();
        assert_eq!(Output::new(srgb, srgb, Intent::RelativeColorimetric, true).err(), Some(OutputError::PressNotCmyk));
        assert_eq!(Output::new(fogra39(), lab, Intent::RelativeColorimetric, true).err(), Some(OutputError::UnsupportedOutput));
        let gray = Output::new(fogra39(), builtin::linear_gray(), Intent::RelativeColorimetric, true).unwrap();
        let p = synthetic(2, 1, [70.0, 5.0, 60.0], [vec![0; 2], vec![0; 2], vec![0; 2], vec![0, 255], vec![0; 2]]);
        let g = p.composite_to(&[], &gray);
        assert!(g.len() == 2 && g[0] > 250 && g[1] < 10, "paper and black in gray: {g:?}");
    }
}
