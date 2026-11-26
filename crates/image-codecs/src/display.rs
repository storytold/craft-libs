//! Conversion of an image's colors into a display's RGB profile through `craft-cms` (feature
//! `cms`).
//!
//! [`Image::to_rgba8`] changes only the layout and sample type, so the RGBA it returns is in the
//! image's own color space. [`Image::to_display_rgba8`] converts the colors as well: from the
//! embedded ICC profile, or from the CICP description when there is no profile, into the display
//! profile the caller passes (`craft_cms::builtin::srgb()` for an sRGB display).

use craft_cms::{ColorSpace, Profile, TransformOptions, builtin, cached};

use crate::cicp::{self, Cicp};
use crate::error::CodecError;
use crate::image::{ChannelLayout, Image, SampleType, convert_samples, read_normalized};
use crate::options::Limits;

/// Pixels converted per step on the float path: the temporary buffers hold this many pixels.
const BAND_PIXELS: usize = 1 << 16;

/// H.273 "unspecified" code point.
const UNSPECIFIED: u16 = 2;

impl Image {
    /// The profile that describes this image's samples: the embedded ICC profile, or for an RGB
    /// image without one, the profile of its CICP description. `None` when the image has neither,
    /// which callers treat as sRGB.
    ///
    /// CICP primaries or transfer 2 (unspecified) are read as BT.709 primaries and the sRGB
    /// transfer; HLG's luma weights follow the same primaries. PQ and HLG (transfer 16 and 18) give the linear-light profile on the same
    /// primaries, which describes the samples once [`cicp::linearize`] has decoded them.
    ///
    /// Returns [`CodecError::InvalidImage`] when the ICC profile does not parse, describes a
    /// different color model from the image's layout, or the CICP code points have no profile.
    pub fn source_profile(&self) -> Result<Option<Profile>, CodecError> {
        if let Some(icc) = &self.icc {
            let profile = Profile::parse(icc).map_err(|e| CodecError::InvalidImage(format!("embedded ICC profile: {e}")))?;
            if !matches_layout(profile.color_space, self.layout()) {
                return Err(CodecError::InvalidImage(format!("embedded ICC profile is {:?}, the image is {:?}", profile.color_space, self.layout())));
            }
            return Ok(Some(profile));
        }
        match self.cicp {
            Some(c) if self.layout().is_rgb() => cicp_profile(c).map(Some),
            _ => Ok(None),
        }
    }

    /// Interleaved RGBA, 8 bits per sample, in the colors of the `display` profile, which must be
    /// an RGB profile. The colors are converted from [`Image::source_profile`] with `opts` (the
    /// rendering intent and black point compensation). An image with no source profile is taken
    /// as sRGB: its [`Image::to_rgba8`] values are returned unchanged for an sRGB display. For any
    /// other display, an untagged RGB image is converted from its own samples, the same as with an
    /// sRGB profile, and a gray, CMYK or Lab image from its [`Image::to_rgba8`] values. PQ and
    /// HLG samples are decoded to linear light first, and light above diffuse white is clipped.
    /// Alpha is copied, and an image without alpha gets opaque alpha.
    ///
    /// The output buffer is checked against `limits` and reserved before any pixel is converted;
    /// a size over the limits, or a reservation the allocator refuses, returns
    /// [`CodecError::LimitExceeded`]. An 8-bit image with a source profile, or an untagged
    /// 8-bit RGB image for a display other than sRGB, is converted straight into the output; any
    /// other is converted a band of pixels at a time, so the temporary buffers do not grow with
    /// the image. An image with no pixels gives an empty vector. The errors of [`Image::source_profile`] are returned as they are, and a
    /// display profile that is not RGB, or a transform `craft-cms` cannot build, returns
    /// [`CodecError::InvalidImage`].
    pub fn to_display_rgba8(&self, display: &Profile, opts: TransformOptions, limits: &Limits) -> Result<Vec<u8>, CodecError> {
        if display.color_space != ColorSpace::Rgb {
            return Err(CodecError::InvalidImage(format!("display profile is {:?}, not RGB", display.color_space)));
        }
        if self.width() == 0 || self.height() == 0 {
            return Ok(Vec::new());
        }
        limits.check(self.width(), self.height(), ChannelLayout::Rgba, SampleType::U8)?;
        let source = self.source_profile()?;
        let len = self.pixel_count().checked_mul(4).ok_or_else(|| CodecError::LimitExceeded("RGBA8 size overflows usize".into()))?;
        let mut out = Vec::new();
        out.try_reserve_exact(len).map_err(|e| CodecError::LimitExceeded(format!("cannot reserve {len} bytes for RGBA8: {e}")))?;
        let channels = self.layout().channels();
        let band = BAND_PIXELS.min(self.pixel_count());

        let source = match &source {
            Some(profile) => profile,
            None if self.layout().is_rgb() && !display.same_bytes(builtin::srgb()) => builtin::srgb(),
            None => return self.untagged_rgba8(out, band, display, opts),
        };

        let transform = cached(source, display, opts).map_err(|e| CodecError::InvalidImage(format!("color transform: {e}")))?;
        let hdr = self.cicp.filter(|c| self.icc.is_none() && c.is_hdr() && self.layout().is_rgb());
        out.resize(len, u8::MAX);
        if self.sample_type() == SampleType::U8 && hdr.is_none() {
            transform.convert_u8(self.data(), channels, &mut out, 4, true).map_err(|e| CodecError::InvalidImage(format!("color transform: {e}")))?;
            return Ok(out);
        }

        let luma = hdr.map(|c| cicp::luma_weights(cicp::primaries_or_bt709(c.primaries)));
        let mut src = Vec::new();
        let mut dst = Vec::new();
        src.try_reserve_exact(band * channels).map_err(|e| CodecError::LimitExceeded(format!("cannot reserve a conversion band: {e}")))?;
        dst.try_reserve_exact(band * 4).map_err(|e| CodecError::LimitExceeded(format!("cannot reserve a conversion band: {e}")))?;
        for (first, out_band) in (0..self.pixel_count()).step_by(band).zip(out.chunks_mut(band * 4)) {
            let pixels = out_band.len() / 4;
            src.clear();
            src.extend((first * channels..(first + pixels) * channels).map(|i| read_normalized(self.data(), self.sample_type(), i)));
            if let (Some(c), Some(luma)) = (hdr, luma) {
                for px in src.chunks_exact_mut(channels) {
                    let [r, g, b, ..] = *px else { continue };
                    let lin = if c.transfer == Cicp::TRANSFER_HLG { cicp::hlg_to_linear([r, g, b], luma) } else { [r, g, b].map(cicp::pq_to_linear) };
                    px[..3].copy_from_slice(&lin);
                }
            }
            dst.clear();
            dst.resize(pixels * 4, 1.0);
            transform.convert_f32(&src, channels, &mut dst, 4, true).map_err(|e| CodecError::InvalidImage(format!("color transform: {e}")))?;
            for (o, v) in out_band.iter_mut().zip(&dst) {
                *o = quantize8(*v);
            }
        }
        Ok(out)
    }

    /// An image with no source profile for an sRGB display, or a gray, CMYK or Lab one for any
    /// display: its [`Image::to_rgba8`] values, a band at a time, converted from sRGB into
    /// `display` when that is not sRGB.
    fn untagged_rgba8(&self, mut out: Vec<u8>, band: usize, display: &Profile, opts: TransformOptions) -> Result<Vec<u8>, CodecError> {
        let pixel_bytes = self.layout().channels() * self.sample_type().bytes();
        for chunk in self.data().chunks(band * pixel_bytes) {
            let pixels = u32::try_from(chunk.len() / pixel_bytes).map_err(|_| CodecError::LimitExceeded("conversion band is too large".into()))?;
            out.extend_from_slice(&convert_samples(chunk, pixels, 1, (self.layout(), self.sample_type()), (ChannelLayout::Rgba, SampleType::U8))?);
        }
        let srgb = builtin::srgb();
        if !display.same_bytes(srgb) {
            convert_rgba8_in_place(&mut out, srgb, display, opts)?;
        }
        Ok(out)
    }
}

/// `true` when an ICC profile of color space `cs` describes samples in `layout`.
fn matches_layout(cs: ColorSpace, layout: ChannelLayout) -> bool {
    match layout {
        ChannelLayout::Gray | ChannelLayout::GrayA => cs == ColorSpace::Gray,
        ChannelLayout::Rgb | ChannelLayout::Rgba => cs == ColorSpace::Rgb,
        ChannelLayout::Cmyk | ChannelLayout::CmykA => cs == ColorSpace::Cmyk,
        ChannelLayout::Lab | ChannelLayout::LabA => cs == ColorSpace::Lab,
    }
}

/// The RGB profile for `c`, with unspecified code points read as BT.709 and sRGB.
fn cicp_profile(c: Cicp) -> Result<Profile, CodecError> {
    let transfer = if c.transfer == UNSPECIFIED { 13 } else { c.transfer };
    builtin::cicp_rgb(cicp::primaries_or_bt709(c.primaries), transfer, c.is_hdr())
        .ok_or_else(|| CodecError::InvalidImage(format!("no RGB profile for CICP primaries {} and transfer {}", c.primaries, c.transfer)))
}

/// Converts interleaved RGBA8 from `src` to `dst` a band at a time, keeping alpha.
fn convert_rgba8_in_place(buf: &mut [u8], src: &Profile, dst: &Profile, opts: TransformOptions) -> Result<(), CodecError> {
    let transform = cached(src, dst, opts).map_err(|e| CodecError::InvalidImage(format!("color transform: {e}")))?;
    let mut band = Vec::new();
    band.try_reserve_exact(BAND_PIXELS * 4).map_err(|e| CodecError::LimitExceeded(format!("cannot reserve a conversion band: {e}")))?;
    for chunk in buf.chunks_mut(BAND_PIXELS * 4) {
        band.clear();
        band.extend_from_slice(chunk);
        transform.convert_u8(&band, 4, chunk, 4, true).map_err(|e| CodecError::InvalidImage(format!("color transform: {e}")))?;
    }
    Ok(())
}

fn quantize8(v: f32) -> u8 {
    if v.is_nan() { 0 } else { (v.clamp(0.0, 1.0) * 255.0).round() as u8 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_cms::Builtin;

    /// Display P3 coordinates of sRGB red (1, 0, 0), from the two spaces' published primaries
    /// (IEC 61966-2-1 and SMPTE EG 432-1) and their shared sRGB transfer.
    const P3_OF_SRGB_RED: [f32; 3] = [0.9175, 0.2003, 0.1386];

    fn opts() -> TransformOptions {
        TransformOptions::default()
    }

    fn close(a: &[u8], b: &[u8], tol: u8) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= tol)
    }

    fn p3_red(sample: SampleType, layout: ChannelLayout) -> Image {
        let mut v = P3_OF_SRGB_RED.to_vec();
        if layout.has_alpha() {
            v.push(0.5);
        }
        Image::from_normalized(1, 1, layout, sample, &v).unwrap_or_else(|e| panic!("{e}"))
    }

    fn display(img: &Image) -> Vec<u8> {
        img.to_display_rgba8(builtin::srgb(), opts(), &Limits::default()).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn display_p3_icc_converts_to_srgb() {
        let icc = Builtin::DisplayP3.profile().to_bytes().to_vec();
        for sample in SampleType::ALL {
            let img = p3_red(sample, ChannelLayout::Rgb).with_icc(Some(icc.clone()));
            let out = display(&img);
            assert!(close(&out, &[255, 0, 0, 255], 2), "{sample:?}: {out:?}");
            // Without the profile the same samples are shown as they are.
            let plain = display(&p3_red(sample, ChannelLayout::Rgb));
            assert_eq!(plain, img.to_rgba8(), "{sample:?}");
            assert!(plain[1] > 40, "{plain:?}");
        }
    }

    #[test]
    fn cicp_p3_matches_the_icc_profile_and_keeps_alpha() {
        let p3 = Cicp { primaries: 12, transfer: 13, matrix: 0, full_range: true };
        let icc = Builtin::DisplayP3.profile().to_bytes().to_vec();
        for sample in [SampleType::U8, SampleType::U16] {
            let mut by_cicp = p3_red(sample, ChannelLayout::Rgba);
            by_cicp.cicp = Some(p3);
            let by_icc = p3_red(sample, ChannelLayout::Rgba).with_icc(Some(icc.clone()));
            let (a, b) = (display(&by_cicp), display(&by_icc));
            assert!(close(&a, &b, 1) && close(&a[..3], &[255, 0, 0], 2), "{sample:?}: {a:?} {b:?}");
            assert_eq!(a[3], 128, "{sample:?}");
        }
    }

    #[test]
    fn srgb_converts_into_a_p3_display() {
        let red = Image::from_u8(1, 1, ChannelLayout::Rgb, vec![255, 0, 0]).unwrap_or_else(|e| panic!("{e}"));
        let want: Vec<u8> = P3_OF_SRGB_RED.iter().map(|v| (v * 255.0).round() as u8).chain([255]).collect();
        let untagged = red.to_display_rgba8(Builtin::DisplayP3.profile(), opts(), &Limits::default()).unwrap_or_else(|e| panic!("{e}"));
        assert!(close(&untagged, &want, 2), "{untagged:?} {want:?}");
        let tagged = red
            .clone()
            .with_icc(Some(builtin::srgb().to_bytes().to_vec()))
            .to_display_rgba8(Builtin::DisplayP3.profile(), opts(), &Limits::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(close(&tagged, &want, 2), "{tagged:?} {want:?}");
    }

    #[test]
    fn untagged_high_depth_converts_like_srgb_tagged() {
        // A 16-bit and a float ramp into a P3 display: the untagged image gives what the same
        // samples tagged with sRGB give, without first rounding them to 8 bits.
        let n = 4096;
        let vals: Vec<f32> = (0..n).flat_map(|i| [i as f32 / (n - 1) as f32, 0.3, 0.7]).collect();
        let srgb = builtin::srgb().to_bytes().to_vec();
        for sample in [SampleType::U16, SampleType::F32] {
            let img = Image::from_normalized(n as u32, 1, ChannelLayout::Rgb, sample, &vals).unwrap_or_else(|e| panic!("{e}"));
            let to_p3 =
                |img: &Image| img.to_display_rgba8(Builtin::DisplayP3.profile(), opts(), &Limits::default()).unwrap_or_else(|e| panic!("{e}"));
            let untagged = to_p3(&img);
            let tagged = to_p3(&img.clone().with_icc(Some(srgb.clone())));
            assert_eq!(untagged, tagged, "{sample:?}");
        }
    }

    #[test]
    fn gray_with_alpha_through_a_gray_profile() {
        let icc = Builtin::SGray.profile().to_bytes().to_vec();
        let img = Image::from_u8(2, 1, ChannelLayout::GrayA, vec![128, 200, 0, 10]).unwrap_or_else(|e| panic!("{e}")).with_icc(Some(icc));
        let out = display(&img);
        assert!(close(&out, &[128, 128, 128, 200, 0, 0, 0, 10], 1), "{out:?}");
    }

    #[test]
    fn hdr_pq_white_is_display_white_and_highlights_clip() {
        let pq = Cicp { primaries: 9, transfer: Cicp::TRANSFER_PQ, matrix: 0, full_range: true };
        let white = cicp::linear_to_pq(1.0);
        let mut img = Image::from_normalized(3, 1, ChannelLayout::Rgb, SampleType::U16, &[white, white, white, 1.0, 1.0, 1.0, 0.3, 0.3, 0.3])
            .unwrap_or_else(|e| panic!("{e}"));
        img.cicp = Some(pq);
        let out = display(&img);
        assert!(close(&out[..8], &[255, 255, 255, 255, 255, 255, 255, 255], 1), "{out:?}");
        assert!(out[8] < 200 && out[8] > 0, "{out:?}");
        // The PQ profile is the linear one on BT.2020 primaries.
        assert_eq!(img.source_profile().ok().flatten().map(|p| p.color_space), Some(ColorSpace::Rgb));
    }

    #[test]
    fn unspecified_cicp_is_srgb() {
        let mut img = p3_red(SampleType::U8, ChannelLayout::Rgb);
        img.cicp = Some(Cicp { primaries: UNSPECIFIED, transfer: UNSPECIFIED, matrix: 2, full_range: true });
        assert!(close(&display(&img), &img.to_rgba8(), 1));
    }

    #[test]
    fn unspecified_hlg_primaries_are_bt709() {
        let hlg = |primaries| {
            let mut img = Image::from_normalized(1, 1, ChannelLayout::Rgb, SampleType::U16, &[0.5, 0.25, 0.1]).unwrap_or_else(|e| panic!("{e}"));
            img.cicp = Some(Cicp { primaries, transfer: Cicp::TRANSFER_HLG, matrix: 0, full_range: true });
            display(&img)
        };
        let (unspecified, bt709, bt2020) = (hlg(UNSPECIFIED), hlg(1), hlg(9));
        assert_eq!(unspecified, bt709);
        assert_ne!(unspecified, bt2020);
        // Decoding to linear light first, as the HEIF and AVIF readers do, gives the same colors.
        let mut img = Image::from_normalized(1, 1, ChannelLayout::Rgb, SampleType::U16, &[0.5, 0.25, 0.1]).unwrap_or_else(|e| panic!("{e}"));
        let c = Cicp { primaries: UNSPECIFIED, transfer: Cicp::TRANSFER_HLG, matrix: 0, full_range: true };
        img.cicp = Some(c);
        let linear = cicp::linearize(img, c).unwrap_or_else(|e| panic!("{e}"));
        let decoded = display(&linear);
        assert_eq!(decoded, bt709);
    }

    #[test]
    fn bad_profiles_and_code_points_are_errors() {
        let base = p3_red(SampleType::U8, ChannelLayout::Rgb);
        let garbage = base.clone().with_icc(Some(vec![0; 200]));
        assert!(matches!(garbage.to_display_rgba8(builtin::srgb(), opts(), &Limits::default()), Err(CodecError::InvalidImage(_))));
        let gray_on_rgb = base.clone().with_icc(Some(Builtin::SGray.profile().to_bytes().to_vec()));
        assert!(matches!(gray_on_rgb.source_profile(), Err(CodecError::InvalidImage(_))));
        let mut unknown = base.clone();
        unknown.cicp = Some(Cicp { primaries: 200, transfer: 13, matrix: 0, full_range: true });
        assert!(matches!(display_err(&unknown, builtin::srgb()), CodecError::InvalidImage(_)));
        assert!(matches!(display_err(&base, Builtin::SGray.profile()), CodecError::InvalidImage(_)));
    }

    fn display_err(img: &Image, display: &Profile) -> CodecError {
        match img.to_display_rgba8(display, opts(), &Limits::default()) {
            Err(e) => e,
            Ok(v) => panic!("expected an error, got {v:?}"),
        }
    }

    #[test]
    fn limits_are_checked_and_empty_images_give_nothing() {
        let img = Image::new(64, 64, ChannelLayout::Rgb, SampleType::F32).unwrap_or_else(|e| panic!("{e}"));
        let tight = Limits { max_alloc: 64 * 64 * 4 - 1, ..Limits::default() };
        assert!(matches!(img.to_display_rgba8(builtin::srgb(), opts(), &tight), Err(CodecError::LimitExceeded(_))));
        let empty = Image::new(0, 5, ChannelLayout::Rgb, SampleType::U8).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(display(&empty), Vec::<u8>::new());
    }

    #[test]
    fn float_path_crosses_bands_with_the_same_result_as_one_pixel() {
        let icc = Builtin::DisplayP3.profile().to_bytes().to_vec();
        let n = BAND_PIXELS + 3;
        let vals: Vec<f32> = P3_OF_SRGB_RED.iter().copied().cycle().take(n * 3).collect();
        let img = Image::from_f32(n as u32, 1, ChannelLayout::Rgb, &vals).unwrap_or_else(|e| panic!("{e}")).with_icc(Some(icc.clone()));
        let out = display(&img);
        let one = display(&p3_red(SampleType::F32, ChannelLayout::Rgb).with_icc(Some(icc)));
        assert_eq!(out.len(), n * 4);
        assert!(out.as_chunks::<4>().0.iter().all(|px| px[..] == one[..]), "{:?}", &out[out.len() - 8..]);
        // Without a profile the bands give what one whole-image conversion gives.
        let untagged = img.with_icc(None);
        assert_eq!(display(&untagged), untagged.to_rgba8());
    }
}
