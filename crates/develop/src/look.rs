//! Estimate the starting look of a raw without a camera colour matrix (Sony ARW, Nikon NEF, Panasonic
//! RW2, Fujifilm RAF, Canon CR2/CR3, Pentax PEF, Samsung SRW, Olympus ORF) from its own JPEG. Colour and luminance are fitted separately; the JPEG supplies correspondences only,
//! never output pixels or a replacement for RAW editing.
//! A global matrix can't follow the camera's hue-dependent rendering (the best matrix rendered a
//! lime shirt olive that the camera kept lime): a hue/saturation table fitted to the residuals
//! (applied like a DNG `ProfileHueSatMap`) corrects that when it also improves the held-out pixels.
use crate::color::{D50, D65, Mat3, PROPHOTO, REC2020, Xy, bradford, luminance_2020};
use crate::raster::{
    Rgb32f,
    resample::{Filter, fit, resize},
};
use crate::sensor::{
    ColorData, RawFormat, RawImage,
    color::CameraTransform,
    profile::{HsvTable, ProfileLook, ToneCurve},
};
use crate::tone::{CameraTone, ToneMap};

mod colour;
mod hue_sat;
mod look_fit;
mod mono;
mod pairs;
pub mod profiles;
mod proxy;
mod tone;

#[cfg(test)]
pub(crate) use colour::{PROFILE_PROXY, dng_colour, dng_tone_eligible};
#[cfg(test)]
use colour::{PROXY, starting_colour_with};
pub use colour::{file_local_look, starting_colour, starting_colour_tags, with_starting_colour};
pub use hue_sat::HueSat;
#[cfg(test)]
use hue_sat::{MIN_LUMINANCE_KEPT, NO_LUMINANCE_KEPT, fit_hue_sat, hue_saturation, restore_weight};
#[cfg(test)]
use look_fit::{at_most, clips_more, correct_vignetting, fit_look, fit_look_with, sony_dro_off_tone, with_dro_off_tone, without_clipped};
#[cfg(test)]
use mono::{fit_dark_tone, fit_tone_only, monochrome, monochrome_look, to_prophoto};
#[cfg(test)]
use pairs::{
    Attempt, Pairs, TONE_FITS, collect_pairs, colour_pair, displayed, fit_attempt, fit_pairs, fit_pairs_ordered, fit_pairs_with, fit_partial,
    fit_tone_with_shadows, luma, rank_correlation_ties, search_ordered,
};
#[cfg(test)]
pub use proxy::LensGeometry;
#[cfg(test)]
use proxy::{
    Cr3Framing, MIN_PREVIEW_FRACTION, alignment, estimate_cr3_framing, into_lens_geometry, proxies, reference_at, sensor_proxy, structure_agrees,
};
pub use proxy::{fit_profile, lens_geometry, profile_pairs};
#[cfg(test)]
use tone::{HighlightCheck, ToneFit, fit_tone, fit_tone_conditional, highlights};

/// The version of the camera-look fit: how a raw's colour matrix, hue/saturation table and tone
/// and chroma curves are chosen. Smart previews record it in their header (`look_version`), because
/// they keep the curve and the colour the fit produced; one written by an older version is rebuilt
/// from its original when that is online, and kept as it is while it is offline.
///
/// **Bump this whenever a change makes the fit give a different result for the same file** (the
/// fit, its gates, the fallbacks, the camera profiles' effect on it). Do not bump it for changes
/// that leave the fitted look alone. `RENDER_CACHE_VERSION` covers renders; this covers proxies.
///
/// 1: the fit as of storytold/lightcraft#499's follow-up; 2: Sony DRO (tone curve lowered to Sony's curve without DRO)
/// and the ILCE-7CR profile (storytold/lightcraft#528, storytold/lightcraft#583, storytold/lightcraft#568, storytold/lightcraft#616); 3: the camera JPEG read in the colour space the raw's EXIF
/// states, for every make (storytold/lightcraft#778).
/// 4: a tone curve from black-and-white camera JPEGs (storytold/lightcraft#577).
/// 5: dark JPEG luminance pairs for the camera toe (storytold/lightcraft#1307).
/// 6: the ILCA-99M2 profile.
/// 7: the sensor proxy follows the file's lens warp when the camera JPEG is lens-corrected.
/// 8: the bundled NIKON Z 6_2 profile.
/// 9: the bundled NIKON Z 8 and NIKON D5300 profiles.
/// 10: DNG tone fit for profiles with a look table and for 1024 px (Lightroom) previews.
/// 12: tone curves of low-contrast scenes (range limit 1.2, with the bright-area check for curves under 1.5, storytold/lightcraft#699);
/// a tone-only fallback for near-black scenes with too few colour pairs (storytold/lightcraft#1587); DNG tone fitting uses a
/// lens-corrected proxy when the file carries lens opcodes; validated A7CR distortion, including the lens data
/// stored with each smart preview.
/// 13: tone curves no steeper than a log-log slope of 4 between knots (storytold/lightcraft#1904).
/// 13: the Canon CR3/CR2 camera JPEG read in the colour space of the Canon maker note (storytold/lightcraft#2654).
/// 13: the sensor proxy of every per-file fit gets the embedded vignetting correction the render applies (Fujifilm
/// RAF; other makes with a lens database), so the render no longer applies it twice, at the position a lens warp samples;
/// the warped candidate's alignment leaves out the pixels outside the sensor instead of reading them as black.
pub const LOOK_VERSION: u32 = 13;

#[derive(Clone, Debug)]
pub struct CameraLook {
    pub matrix: Mat3,
    pub tone: CameraTone,
    /// Hue/saturation correction after `matrix`, in linear ProPhoto RGB (DNG `ProfileHueSatMap`).
    pub hue_sat: Option<HsvTable>,
}

#[cfg(test)]
mod tests;
