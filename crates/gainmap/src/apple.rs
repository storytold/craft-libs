//! Reconstruction of Apple's legacy, single-channel gain maps.
//!
//! This is deliberately separate from ISO 21496-1 / Ultra HDR's log2 interpolation.
//! Container readers must identify the Apple family before calling this module.

use crate::{Error, GainImage, Result};

/// Already-parsed metadata for Apple's legacy gain-map family (not ISO Adaptive HDR).
/// Container parsing and validation of the Apple gain-map version remain the caller's job.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metadata {
    /// `HDRGainMapHeadroom`, a linear ratio, used only without maker-note tags.
    /// This fallback has arithmetic coverage but no independent Core Image measurement.
    pub headroom: Option<f64>,
    /// Apple maker-note tag 33, paired with tag 48.
    pub maker33: Option<f64>,
    /// Apple maker-note tag 48, paired with tag 33.
    pub maker48: Option<f64>,
}

impl Metadata {
    /// Resolve linear headroom. A complete maker-note pair takes precedence over the tag.
    /// An incomplete maker-note pair or any supplied non-finite value is refused.
    /// No headroom is guessed for a file that states none.
    pub fn headroom(&self) -> Result<f32> {
        if [self.headroom, self.maker33, self.maker48].into_iter().flatten().any(|v| !v.is_finite()) {
            return Err(invalid("Apple: non-finite metadata"));
        }
        let ratio = match (self.maker33, self.maker48) {
            (Some(a), Some(b)) => {
                // Apple's published piecewise conversion from maker-note values to stops:
                // https://developer.apple.com/documentation/appkit/applying-apple-hdr-effect-to-your-photos
                let (slope, intercept) = match (a < 1.0, b <= 0.01) {
                    (true, true) => (-20.0, 1.8),
                    (true, false) => (-0.101, 1.601),
                    (false, true) => (-70.0, 3.0),
                    (false, false) => (-0.303, 2.303),
                };
                (slope * b + intercept).max(0.0).exp2()
            }
            (None, None) => self.headroom.ok_or_else(|| invalid("Apple: missing headroom"))?,
            _ => return Err(invalid("Apple: incomplete maker-note pair")),
        };
        // A conservative reconstruction budget; do not let hostile metadata overflow a render.
        if !(1.0..=65536.0).contains(&ratio) {
            return Err(invalid("Apple: headroom outside 1..=65536"));
        }
        Ok(ratio as f32)
    }
}

fn invalid(reason: &'static str) -> Error {
    Error(reason)
}

/// Apply the full Apple HDR rendition to linear RGB in its source primaries.
///
/// `gain` contains unlinearized, grayscale 8-bit codes. Base and map must cover the same
/// image rectangle in the same orientation. Resampling uses aligned pixel centres, bilinear
/// interpolation of codes, and edge clamping; gamma is applied *after* interpolation.
/// Each channel is multiplied by `1 + (headroom - 1) * (code / 255)^2.2`, with no offsets.
/// The 2.2 curve is an empirical Core Image compatibility choice; Apple's published description
/// instead calls for inverse Rec.709. This API does not model display adaptation or tone mapping.
///
/// Invalid dimensions, metadata, or non-finite/overflowing base samples return an error before
/// changing `base`. Finite negative RGB values are preserved (wide-gamut linear RGB can be negative).
pub fn apply(base: &mut [[f32; 3]], width: usize, height: usize, gain: &GainImage, metadata: &Metadata) -> Result<()> {
    let count = width.checked_mul(height).ok_or_else(|| invalid("Apple: base dimensions overflow"))?;
    let map_count = gain.width.checked_mul(gain.height).ok_or_else(|| invalid("Apple: map dimensions overflow"))?;
    if width == 0 || height == 0 || gain.width == 0 || gain.height == 0 || base.len() != count || gain.channels != 1 || gain.data.len() != map_count {
        return Err(invalid("Apple: invalid base or grayscale map dimensions"));
    }
    let headroom = metadata.headroom()?;
    // Multiplying two f32 values is exact in f64. Dividing MAX in f32 can round the
    // admissible boundary upward and admit a value whose later multiplication overflows.
    if base.iter().flatten().any(|v| !v.is_finite() || f64::from(v.abs()) * f64::from(headroom) > f64::from(f32::MAX)) {
        return Err(invalid("Apple: base sample is non-finite or would overflow"));
    }
    let scale_x = gain.width as f32 / width as f32;
    let scale_y = gain.height as f32 / height as f32;
    for (index, pixel) in base.iter_mut().enumerate() {
        let x = (index % width) as f32 + 0.5;
        let y = (index / width) as f32 + 0.5;
        let code = gain.sample(x * scale_x, y * scale_y, 0) / 255.0;
        let multiplier = 1.0 + (headroom - 1.0) * code.powf(2.2);
        for value in pixel {
            *value *= multiplier;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tagged(headroom: f64) -> Metadata {
        Metadata { headroom: Some(headroom), ..Metadata::default() }
    }

    fn map(width: usize, height: usize, data: &[u8]) -> GainImage {
        GainImage { width, height, channels: 1, data: data.to_vec() }
    }

    #[test]
    fn maker_note_segments_boundaries_and_precedence() {
        for (a, b, stops) in
            [(0.9, 0.0, 1.8), (0.9, 0.01, 1.6), (0.9, 1.0, 1.5), (1.0, 0.0, 3.0), (1.0, 0.01, 2.3), (1.0, 1.0, 2.0), (1.0, 10.0, 0.0)]
        {
            let metadata = Metadata { headroom: Some(4.5), maker33: Some(a), maker48: Some(b) };
            assert!((metadata.headroom().unwrap() as f64 - 2.0_f64.powf(stops)).abs() < 1e-6);
        }
        assert_eq!(tagged(4.5).headroom().unwrap(), 4.5);
    }

    #[test]
    fn malformed_metadata_is_not_silently_replaced() {
        for metadata in [
            Metadata::default(),
            Metadata { maker33: Some(1.0), ..tagged(4.0) },
            Metadata { maker48: Some(0.0), ..tagged(4.0) },
            tagged(0.0),
            tagged(-1.0),
            tagged(0.5),
            tagged(65537.0),
            Metadata { maker33: Some(1.0), maker48: Some(-100.0), ..Metadata::default() },
        ] {
            assert!(metadata.headroom().is_err(), "{metadata:?}");
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for metadata in [
                tagged(bad),
                Metadata { maker33: Some(bad), maker48: Some(0.0), ..tagged(4.0) },
                Metadata { maker33: Some(1.0), maker48: Some(bad), ..tagged(4.0) },
                Metadata { maker33: Some(1.0), maker48: Some(0.0), ..tagged(bad) },
            ] {
                assert!(metadata.headroom().is_err());
            }
        }
    }

    // These procedural vectors test the chosen arithmetic, not an independent platform oracle.
    #[test]
    fn endpoints_and_interior_codes_are_ratio_interpolated() {
        let mut base = [[0.25, 0.5, -0.125]; 3];
        apply(&mut base, 3, 1, &map(3, 1, &[0, 128, 255]), &tagged(8.0)).unwrap();
        assert_eq!(base[0], [0.25, 0.5, -0.125]);
        assert_eq!(base[2], [2.0, 4.0, -1.0]);
        // Independently tabulated double-precision value at code 128: gain = 2.536638026.
        assert!((base[1][1] - 1.268_319).abs() < 1e-6);
        assert!((base[1][1] - 0.5 * 8.0_f32.powf(128.0 / 255.0)).abs() > 0.1);
        let mut unity = [[0.5; 3]; 3];
        apply(&mut unity, 3, 1, &map(3, 1, &[0, 128, 255]), &tagged(1.0)).unwrap();
        assert_eq!(unity, [[0.5; 3]; 3]);
    }

    #[test]
    fn pixel_centres_edges_and_gamma_order() {
        let mut base = [[1.0; 3]; 16];
        apply(&mut base, 4, 4, &map(2, 2, &[0, 255, 255, 0]), &tagged(2.0)).unwrap();
        // Pixel-centre interpolation of the checkerboard, before the nonlinear curve.
        let codes = [0.0_f32, 0.25, 0.75, 1.0, 0.25, 0.375, 0.625, 0.75, 0.75, 0.625, 0.375, 0.25, 1.0, 0.75, 0.25, 0.0];
        for (pixel, code) in base.iter().zip(codes) {
            assert!((pixel[0] - (1.0 + code.powf(2.2))).abs() < 1e-6);
        }
        // A non-integral scale must use the same centre convention.
        let mut non_integral = [[1.0; 3]; 5];
        apply(&mut non_integral, 5, 1, &map(2, 1, &[0, 255]), &tagged(2.0)).unwrap();
        for (pixel, code) in non_integral.iter().zip([0.0_f32, 0.1, 0.5, 0.9, 1.0]) {
            assert!((pixel[0] - (1.0 + code.powf(2.2))).abs() < 1e-6);
        }
        let mut singleton = [[0.5; 3]; 6];
        apply(&mut singleton, 3, 2, &map(1, 1, &[255]), &tagged(2.0)).unwrap();
        assert_eq!(singleton, [[1.0; 3]; 6]);
    }

    #[test]
    fn rounded_overflow_boundary_is_rejected_before_any_pixel_changes() {
        let rounded_boundary = f32::MAX / 100.0;
        assert!(!(rounded_boundary * 100.0).is_finite());
        for sign in [1.0_f32, -1.0] {
            let original = [[0.25, -0.5, 1.0], [0.5, sign * rounded_boundary, 0.25]];
            let mut base = original;
            assert!(apply(&mut base, 2, 1, &map(1, 1, &[255]), &tagged(100.0)).is_err());
            assert_eq!(base, original);
        }
        // The adjacent smaller magnitude is safe for either sign and remains accepted.
        let safe = rounded_boundary.next_down();
        let mut base = [[safe, -safe, 0.5]];
        apply(&mut base, 1, 1, &map(1, 1, &[255]), &tagged(100.0)).unwrap();
        assert!(base.iter().flatten().all(|v| v.is_finite()));
        assert_eq!(base[0], [safe * 100.0, -safe * 100.0, 50.0]);
    }

    #[test]
    fn errors_leave_the_base_unchanged() {
        let bad_maps = [map(0, 1, &[]), map(2, 2, &[0]), map(usize::MAX, 2, &[]), GainImage { channels: 3, ..map(1, 1, &[0, 0, 0]) }];
        for gain in bad_maps {
            let mut base = [[0.5; 3]];
            assert!(apply(&mut base, 1, 1, &gain, &tagged(2.0)).is_err());
            assert_eq!(base, [[0.5; 3]]);
        }
        for (w, h) in [(0, 1), (2, 1), (usize::MAX, 2)] {
            assert!(apply(&mut [[0.5; 3]], w, h, &map(1, 1, &[0]), &tagged(2.0)).is_err());
        }
        for bad in [f32::NAN, f32::INFINITY, f32::MAX] {
            let mut base = [[0.5; 3], [bad; 3]];
            assert!(apply(&mut base, 2, 1, &map(1, 1, &[255]), &tagged(8.0)).is_err());
            assert_eq!(base[0], [0.5; 3]);
        }
    }
}
