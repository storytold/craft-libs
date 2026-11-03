//! Process color conversion for plate mode through craft-cms.

use std::sync::{Arc, OnceLock};

use craft_cms::{Builtin, ColorSpace, Intent, Profile, Transform, TransformOptions, builtin, cached, math};
use craft_pdf::interpret::color::{CmykConversion, CmykSimulation};

/// Converts process color into a CMYK printing condition with craft-cms: relative
/// colorimetric, without black point compensation, as the plate renderer expects (see
/// [`CmykConversion`]). CMYK shows as sRGB relative colorimetric with black point
/// compensation. Transforms are built on first use and shared through craft-cms's cache.
pub struct CmsConversion {
    profile: Profile,
    perceptual: bool,
    from_srgb: OnceLock<Option<Arc<Transform>>>,
    from_lab: OnceLock<Option<Arc<Transform>>>,
    to_srgb: OnceLock<Option<Arc<Transform>>>,
}

/// Why a profile can't be a printing condition.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProfileError {
    #[error("the profile could not be read: {0}")]
    Unreadable(String),
    #[error("the profile is not CMYK")]
    NotCmyk,
    #[error("the profile has no PCS-to-device (BToA) tables")]
    NoPcsToDevice,
}

impl CmsConversion {
    /// The printing condition of ICC profile bytes `icc`.
    pub fn new(icc: &[u8]) -> Result<Self, ProfileError> {
        Self::from_profile(Profile::parse(icc).map_err(|e| ProfileError::Unreadable(format!("{e:?}")))?)
    }

    /// The printing condition of a parsed CMYK profile with PCS-to-device tables.
    pub fn from_profile(profile: Profile) -> Result<Self, ProfileError> {
        if profile.color_space != ColorSpace::Cmyk {
            return Err(ProfileError::NotCmyk);
        }
        if profile.pcs_to_device(Intent::RelativeColorimetric).is_err() {
            return Err(ProfileError::NoPcsToDevice);
        }
        // Without a colorimetric BToA1 table craft-cms uses BToA0, the perceptual one.
        let perceptual = profile.b2a[1].is_none() && profile.b2a[0].is_some();
        Ok(Self { profile, perceptual, from_srgb: OnceLock::new(), from_lab: OnceLock::new(), to_srgb: OnceLock::new() })
    }

    /// craft-cms's Coated FOGRA39 profile.
    pub fn fogra39() -> Self {
        let profile = Builtin::CoatedFogra39.profile().clone();
        let perceptual = profile.b2a[1].is_none() && profile.b2a[0].is_some();
        Self { profile, perceptual, from_srgb: OnceLock::new(), from_lab: OnceLock::new(), to_srgb: OnceLock::new() }
    }

    /// A plate-mode printing condition converting through this, for the profile bytes `icc`
    /// this was made from. ICCBased CMYK colors embedding those same bytes separate as they
    /// are.
    pub fn simulation(self, icc: &[u8]) -> CmykSimulation {
        CmykSimulation::with_conversion(icc, self)
    }

    fn transform(cell: &OnceLock<Option<Arc<Transform>>>, src: &Profile, dst: &Profile, bpc: bool) -> Option<Arc<Transform>> {
        cell.get_or_init(|| {
            // The fast path interpolates the same tables that 8-bit buffers use: plates convert
            // images sample by sample through here.
            let opts = TransformOptions { intent: Intent::RelativeColorimetric, bpc, precise_float: false };
            cached(src, dst, opts).ok()
        })
        .clone()
    }
}

fn whole(input: &[f32], channels: usize, cmyk: &[f32]) -> bool {
    input.len().is_multiple_of(channels) && cmyk.len().is_multiple_of(4) && input.len() / channels == cmyk.len() / 4
}

fn finite01(cmyk: &mut [f32]) {
    for v in cmyk {
        *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    }
}

impl CmykConversion for CmsConversion {
    fn from_srgb(&self, rgb: &[f32], cmyk: &mut [f32]) -> Option<()> {
        if !whole(rgb, 3, cmyk) {
            return None;
        }
        let t = Self::transform(&self.from_srgb, builtin::srgb(), &self.profile, false)?;
        t.convert_f32(rgb, 3, cmyk, 4, false).ok()?;
        finite01(cmyk);
        Some(())
    }

    fn from_pcs_xyz(&self, xyz: &[f32], cmyk: &mut [f32]) -> Option<()> {
        if !whole(xyz, 3, cmyk) {
            return None;
        }
        // Into the Lab D50 identity profile's device values (ICC v4 Lab encoding).
        let lab: Vec<f32> = xyz
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| {
                let [l, a, b] = math::xyz_to_lab_f32(*c);
                [l / 100.0, (a + 128.0) / 255.0, (b + 128.0) / 255.0]
            })
            .collect();
        let t = Self::transform(&self.from_lab, Builtin::LabD50.profile(), &self.profile, false)?;
        t.convert_f32(&lab, 3, cmyk, 4, false).ok()?;
        finite01(cmyk);
        Some(())
    }

    fn to_srgb8(&self, cmyk: &[u8], rgb: &mut [u8]) -> Option<()> {
        if !cmyk.len().is_multiple_of(4) || cmyk.len() / 4 * 3 != rgb.len() {
            return None;
        }
        let t = Self::transform(&self.to_srgb, &self.profile, builtin::srgb(), true)?;
        t.convert_u8(cmyk, 4, rgb, 3, false).ok()
    }

    fn description(&self) -> Option<&str> {
        let d = self.profile.description.trim();
        (!d.is_empty()).then_some(d)
    }

    fn perceptual(&self) -> bool {
        self.perceptual
    }
}

/// CIE L\*a\*b\* (D50) of an sRGB color, relative colorimetric; `None` if craft-cms can't
/// convert.
pub(crate) fn srgb8_to_lab(rgb: [u8; 3]) -> Option<[f32; 3]> {
    static T: OnceLock<Option<Arc<Transform>>> = OnceLock::new();
    let t = CmsConversion::transform(&T, builtin::srgb(), Builtin::LabD50.profile(), false)?;
    let mut lab = [0.0_f32; 3];
    let rgb = rgb.map(|c| f32::from(c) / 255.0);
    t.convert_f32(&rgb, 3, &mut lab, 3, false).ok()?;
    Some([lab[0] * 100.0, lab[1] * 255.0 - 128.0, lab[2] * 255.0 - 128.0])
}

/// sRGB of CIE L\*a\*b\* (D50) colors, relative colorimetric, three values per color in
/// both; `None` if craft-cms can't convert or the lengths disagree.
pub(crate) fn lab_to_srgb8(lab: &[f32], rgb: &mut [u8]) -> Option<()> {
    static T: OnceLock<Option<Arc<Transform>>> = OnceLock::new();
    if !lab.len().is_multiple_of(3) || lab.len() != rgb.len() {
        return None;
    }
    let t = CmsConversion::transform(&T, Builtin::LabD50.profile(), builtin::srgb(), false)?;
    // The Lab D50 identity profile's device values (ICC v4 Lab encoding).
    let encoded: Vec<f32> = lab.as_chunks::<3>().0.iter().flat_map(|[l, a, b]| [l / 100.0, (a + 128.0) / 255.0, (b + 128.0) / 255.0]).collect();
    let mut out = vec![0.0_f32; encoded.len()];
    t.convert_f32(&encoded, 3, &mut out, 3, false).ok()?;
    for (byte, v) in rgb.iter_mut().zip(out) {
        *byte = if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
    }
    Some(())
}
