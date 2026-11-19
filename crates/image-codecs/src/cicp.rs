//! ITU-T H.273 colour descriptions (CICP: coding-independent code points), as HEIF `colr`
//! `nclx` boxes and video streams declare them, and the HDR transfer functions of ITU-R BT.2100.
//!
//! PQ and HLG can't be written as an ICC tone curve, so an image using them is decoded here to
//! linear light (32-bit float) instead: diffuse white (BT.2408's 203 cd/m²) becomes 1.0 and the
//! highlights above it are kept as values over 1.0, as Photoshop holds HDR in a 32-bit document.
//! The caller tags the result with a linear profile on the declared primaries.

use crate::error::CodecError;
use crate::image::{Image, SampleType};

/// A colour description: the four H.273 code points as written in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cicp {
    /// Colour primaries: 1 BT.709, 9 BT.2020, 12 P3-D65…
    pub primaries: u16,
    /// Transfer characteristics: 13 sRGB, 1/6/14/15 BT.709, 8 linear, 16 PQ, 18 HLG…
    pub transfer: u16,
    /// Matrix coefficients.
    pub matrix: u16,
    /// Full-range samples rather than studio swing.
    pub full_range: bool,
}

impl Cicp {
    /// SMPTE ST 2084 / BT.2100 perceptual quantizer.
    pub const TRANSFER_PQ: u16 = 16;
    /// ARIB STD-B67 / BT.2100 hybrid log-gamma.
    pub const TRANSFER_HLG: u16 = 18;

    /// PQ or HLG: samples that [`linearize`] decodes to linear light.
    pub fn is_hdr(self) -> bool {
        matches!(self.transfer, Cicp::TRANSFER_PQ | Cicp::TRANSFER_HLG)
    }
}

/// The luminance that becomes 1.0: BT.2408's HDR reference (diffuse) white.
pub const REFERENCE_WHITE_NITS: f32 = 203.0;
/// The nominal display peak HLG is rendered for (BT.2100's reference display).
pub const HLG_PEAK_NITS: f32 = 1000.0;
/// PQ's absolute peak.
pub const PQ_PEAK_NITS: f32 = 10_000.0;

/// BT.2100 HLG inverse OETF: non-linear signal `E'` (0..1) to normalised scene light `E` (0..1).
pub fn hlg_inverse_oetf(e: f32) -> f32 {
    const A: f32 = 0.178_832_77;
    const B: f32 = 1.0 - 4.0 * A;
    const C: f32 = 0.559_910_7;
    let e = if e.is_finite() { e.clamp(0.0, 1.0) } else { 0.0 };
    if e <= 0.5 { e * e / 3.0 } else { (((e - C) / A).exp() + B) / 12.0 }
}

/// BT.2100 HLG signal to display light relative to [`REFERENCE_WHITE_NITS`]: the inverse OETF,
/// then the OOTF for a [`HLG_PEAK_NITS`] display (system gamma 1.2, zero black level), whose
/// luminance uses the weights of the `luma` primaries.
pub fn hlg_to_linear(rgb: [f32; 3], luma: [f32; 3]) -> [f32; 3] {
    let s = rgb.map(hlg_inverse_oetf);
    let ys = luma[0] * s[0] + luma[1] * s[1] + luma[2] * s[2];
    // γ = 1.2 + 0.42·log10(Lw / 1000) is exactly 1.2 at 1000 cd/m².
    let gain = if ys > 0.0 { HLG_PEAK_NITS * ys.powf(0.2) / REFERENCE_WHITE_NITS } else { 0.0 };
    s.map(|v| v * gain)
}

/// SMPTE ST 2084 (PQ) signal `E'` (0..1) to display light relative to [`REFERENCE_WHITE_NITS`].
pub fn pq_to_linear(e: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let e = if e.is_finite() { e.clamp(0.0, 1.0) } else { 0.0 };
    let p = e.powf(1.0 / M2);
    let y = ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1);
    y * PQ_PEAK_NITS / REFERENCE_WHITE_NITS
}

/// Relative luminance weights of the RGB primaries `code` (BT.2020's for anything else, the
/// space BT.2100 defines HLG in).
fn luma_weights(code: u16) -> [f32; 3] {
    match code {
        1 => [0.2126, 0.7152, 0.0722],
        12 => [0.2290, 0.6917, 0.0793],
        _ => [0.2627, 0.6780, 0.0593],
    }
}

/// `img` (RGB or RGBA, decoded with `cicp`'s PQ or HLG transfer) as linear light in 32-bit float,
/// alpha unchanged. Its ICC profile is dropped: whatever it said described the encoded samples,
/// not linear light. Anything else comes back unchanged.
pub fn linearize(img: Image, cicp: Cicp) -> Result<Image, CodecError> {
    let layout = img.layout();
    if !cicp.is_hdr() || !layout.is_rgb() {
        return Ok(img);
    }
    let (w, h) = img.dimensions();
    let ch = layout.channels();
    let mut v = img.to_normalized();
    let luma = luma_weights(cicp.primaries);
    for px in v.chunks_exact_mut(ch) {
        let [r, g, b, ..] = *px else { continue };
        let out = if cicp.transfer == Cicp::TRANSFER_HLG { hlg_to_linear([r, g, b], luma) } else { [r, g, b].map(pq_to_linear) };
        px[..3].copy_from_slice(&out);
    }
    let mut out = Image::from_raw(w, h, layout, SampleType::F32, v.iter().flat_map(|x| x.to_ne_bytes()).collect())?;
    out.meta = img.meta;
    out.warnings = img.warnings;
    out.cicp = Some(cicp);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::ChannelLayout;

    const BT2020: [f32; 3] = [0.2627, 0.6780, 0.0593];

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn hlg_inverse_oetf_matches_bt2100() {
        assert_eq!(hlg_inverse_oetf(0.0), 0.0);
        // The two segments meet at E' = 0.5, E = 1/12.
        assert!(close(hlg_inverse_oetf(0.5), 1.0 / 12.0, 1e-6));
        assert!(close(hlg_inverse_oetf(0.500_001), 1.0 / 12.0, 1e-5));
        assert!(close(hlg_inverse_oetf(1.0), 1.0, 1e-4));
        // BT.2408's HLG reference white, 75 % signal, is E ≈ 0.265.
        assert!(close(hlg_inverse_oetf(0.75), 0.2650, 1e-3));
        for bad in [f32::NAN, f32::INFINITY, -1.0] {
            assert!(hlg_inverse_oetf(bad).is_finite());
        }
    }

    #[test]
    fn hlg_reference_white_is_one_and_highlights_are_kept() {
        // BT.2408: a 75 % HLG grey on a 1000 cd/m² display is 203 cd/m², our 1.0.
        let white = hlg_to_linear([0.75; 3], BT2020);
        assert!(white.iter().all(|&v| close(v, 1.0, 0.005)), "{white:?}");
        // Peak signal is the display peak: 1000 / 203.
        let peak = hlg_to_linear([1.0; 3], BT2020);
        assert!(peak.iter().all(|&v| close(v, 1000.0 / 203.0, 0.01)), "{peak:?}");
        // 50 % grey: 1000 · (1/12)^1.2 ≈ 50.7 cd/m².
        let mid = hlg_to_linear([0.5; 3], BT2020);
        assert!(mid.iter().all(|&v| close(v * 203.0, 50.71, 0.1)), "{mid:?}");
        assert_eq!(hlg_to_linear([0.0; 3], BT2020), [0.0; 3]);
        // A saturated colour keeps its hue: only red lit stays only red.
        let red = hlg_to_linear([0.75, 0.0, 0.0], BT2020);
        assert!(red[0] > 0.0 && red[1] == 0.0 && red[2] == 0.0, "{red:?}");
    }

    #[test]
    fn pq_matches_st2084() {
        assert_eq!(pq_to_linear(0.0), 0.0);
        assert!(close(pq_to_linear(1.0), 10_000.0 / 203.0, 0.05));
        // 203 cd/m² is PQ signal ≈ 0.5807 (BT.2408).
        assert!(close(pq_to_linear(0.5807), 1.0, 0.005), "{}", pq_to_linear(0.5807));
        assert!(pq_to_linear(f32::NAN).is_finite());
    }

    #[test]
    fn linearize_decodes_hlg_to_float_and_keeps_alpha() {
        let hlg = Cicp { primaries: 9, transfer: Cicp::TRANSFER_HLG, matrix: 9, full_range: true };
        let three_quarters = (0.75f32 * 65535.0).round() as u16;
        let img = Image::from_u16(2, 1, ChannelLayout::Rgba, &[three_quarters, three_quarters, three_quarters, 65535, 0, 0, 0, 32768])
            .unwrap()
            .with_icc(Some(b"encoded-profile".to_vec()));
        let out = linearize(img, hlg).unwrap();
        assert_eq!((out.sample_type(), out.layout(), out.icc.as_deref(), out.cicp), (SampleType::F32, ChannelLayout::Rgba, None, Some(hlg)));
        let v = out.to_normalized();
        assert!(v[..3].iter().all(|&x| close(x, 1.0, 0.005)), "{v:?}");
        assert!(close(v[3], 1.0, 1e-6) && v[4..7] == [0.0; 3] && close(v[7], 0.5, 1e-3), "{v:?}");
    }

    #[test]
    fn linearize_leaves_sdr_untouched() {
        let srgb = Cicp { primaries: 1, transfer: 13, matrix: 6, full_range: false };
        let img = Image::from_u16(1, 1, ChannelLayout::Rgb, &[1, 2, 3]).unwrap();
        assert_eq!(linearize(img.clone(), srgb).unwrap(), img);
    }
}
