//! JPEG XL tiles and strips (DNG 1.7 compression 52546), decoded with `jxl-oxide` (pure Rust).
//!
//! Per the DNG 1.7 specification a JPEG XL chunk holds N-bit unsigned integers (8 ≤ N ≤ 16) or 16-bit
//! floats, in 1 or 3 planes, as either a bare codestream or an ISO-BMFF container; jxl-oxide reads both.
//!
//! jxl-oxide renders every channel as `f32`: integer samples come out as `value / (2^bits − 1)` with
//! `bits` the codestream's own declared bit depth, so scaling back by that maximum and rounding gives the
//! stored integers exactly (an `f32` holds `v / 65535` to far better than half a code value) — lossless
//! tiles decode bit-exactly, whatever bit depth the encoder declared relative to the IFD's
//! `BitsPerSample`. Float samples are returned as decoded.
//!
//! Lossless (non-XYB) tiles are rendered in the codestream's own colour encoding, which is the identity,
//! keeping the values in the raw IFD's sample space. Lossy (XYB) tiles are decoded by jxl-oxide to linear
//! sRGB primaries; the codestream's declared encoding is then restored by hand: its primaries with a
//! plain 3×3 matrix, then its transfer curve. A grey (single-plane) lossy tile is still coded as three
//! XYB channels; its declared grey encoding is restored as the luminance (CIE Y, D65 white) of the
//! linear sRGB output, then its transfer curve. The samples of a lossy tile mean what they would in a
//! lossless one with the same header, so a declared transfer curve is part of the stored values, not a
//! display hint. Lightroom Classic's lossy LinearRaw DNGs (Convert to DNG with lossy compression) store
//! perceptually shaped samples, label them with the sRGB curve and linearise them with a cubic
//! `MapPolynomial` per plane in `OpcodeList2`. Measured against Lightroom's own preview of two such
//! files, the polynomial applied to the samples in the declared encoding tracks Lightroom's rendering
//! (midtone log-log slope 0.98 and 1.38, about 1 EV apart); applied to the decoder's linear output it is
//! 3.6 EV too dark. We never let jxl-oxide convert to the declared encoding itself:
//! that step gamut-maps and clips to [0, 1], which destroys raw values above 1.0 (a `WhiteLevel` of
//! 32768 puts them far above) and desaturates. The result is the sample space the DNG's `WhiteLevel`
//! refers to, up to the codec's loss, and can exceed 1.0 or dip below 0.
//!
//! Every tile is decoded with wide (32-bit) modular buffers. A lossy tile whose header sets
//! `modular_16bit_buffers` can still carry values that need more than 16 bits; the narrow buffers then
//! fail with `InvalidAnsStream` or `UnexpectedEof`.

use crate::tiffraw::ChunkPx;
use crate::{RawError, Result};
use jxl_oxide::color::{ColourEncoding, ColourSpace, Primaries, TransferFunction, WhitePoint};
use jxl_oxide::frame::Encoding;
use jxl_oxide::image::BitDepth;
use jxl_oxide::{AllocTracker, EnumColourEncoding, JxlImage, JxlThreadPool, RenderingIntent};

/// Decoder memory (bytes) allowed per sample of a tile. jxl-oxide 0.12 peaks at about three 32-bit
/// buffers per sample: measured ≤ 10.3 bytes for lossless (modular) tiles and ≤ 13.5 bytes per
/// colour sample for lossy (VarDCT) ones, photon noise included.
const BYTES_PER_SAMPLE: usize = 16;
/// Fixed decoder overhead on top (headers, entropy-coding tables, group metadata): under 10 KiB
/// on a 16 × 16 tile.
const MARGIN_BYTES: usize = 64 << 10;

/// Linear sRGB primaries with a D65 white point (the space jxl-oxide decodes XYB to) and the matrix that
/// takes a linear colour in it to the codestream's declared primaries (row-major, D65 white point).
type Matrix = [[f64; 3]; 3];

const D65: [f64; 2] = [0.3127, 0.3290];
const SRGB: [[f64; 2]; 3] = [[0.640, 0.330], [0.300, 0.600], [0.150, 0.060]];
const BT2100: [[f64; 2]; 3] = [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]];
const P3: [[f64; 2]; 3] = [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]];

fn inverse(m: Matrix) -> Matrix {
    let c =
        |i: usize, j: usize| m[(i + 1) % 3][(j + 1) % 3] * m[(i + 2) % 3][(j + 2) % 3] - m[(i + 1) % 3][(j + 2) % 3] * m[(i + 2) % 3][(j + 1) % 3];
    let det = m[0][0] * c(0, 0) + m[0][1] * c(0, 1) + m[0][2] * c(0, 2);
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = c(j, i) / det;
        }
    }
    out
}

fn mul(a: Matrix, b: Matrix) -> Matrix {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// Linear RGB → XYZ for primaries `p` (xy) and the D65 white point.
fn rgb_to_xyz(p: [[f64; 2]; 3]) -> Matrix {
    let xyz = |[x, y]: [f64; 2]| [x / y, 1.0, (1.0 - x - y) / y];
    let cols = [xyz(p[0]), xyz(p[1]), xyz(p[2])];
    let prim = [[cols[0][0], cols[1][0], cols[2][0]], [1.0, 1.0, 1.0], [cols[0][2], cols[1][2], cols[2][2]]];
    let w = xyz(D65);
    let inv = inverse(prim);
    let s: [f64; 3] = std::array::from_fn(|i| (0..3).map(|k| inv[i][k] * w[k]).sum());
    std::array::from_fn(|i| std::array::from_fn(|j| prim[i][j] * s[j]))
}

/// A declared transfer curve, applied to linear values (linear → coded). Mirrored for negative values
/// and not clipped above 1.0, so lossy undershoot and raw values above white survive.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Curve {
    Linear,
    /// IEC 61966-2-1 (sRGB).
    Srgb,
    /// ITU-R BT.709.
    Bt709,
    /// A pure power: coded = linear^exponent.
    Power(f32),
}

impl Curve {
    fn encode(self, v: f32) -> f32 {
        let a = v.abs();
        let e = match self {
            Curve::Linear => return v,
            Curve::Srgb if a <= 0.003_130_8 => 12.92 * a,
            Curve::Srgb => 1.055 * a.powf(1.0 / 2.4) - 0.055,
            Curve::Bt709 if a < 0.018 => 4.5 * a,
            Curve::Bt709 => 1.099 * a.powf(0.45) - 0.099,
            Curve::Power(p) => a.powf(p),
        };
        e.copysign(v)
    }
}

/// How a chunk is rendered by jxl-oxide.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Output {
    /// In the codestream's own colour encoding (non-XYB: the identity).
    AsCoded,
    /// XYB (lossy) data: linear sRGB primaries from the decoder, then `matrix` to the declared
    /// primaries (`None` when those are sRGB's), then the declared transfer `curve`.
    FromXyb { matrix: Option<[[f32; 3]; 3]>, curve: Curve },
}

/// Decide how to render a chunk from its header: whether it is XYB-coded, the declared colour
/// encoding and the intensity target. Lossy data is only accepted when the declared encoding is an RGB
/// one (three planes) or a grey one (one plane) with a D65 white point and an SDR transfer curve (linear, sRGB, BT.709 or a power: what raw DNG
/// writers signal) and no HDR tone mapping would apply, otherwise the samples cannot be restored
/// faithfully.
fn output_for(xyb: bool, encoding: &ColourEncoding, intensity_target: f32, planes: usize) -> Result<Output> {
    if !xyb {
        return Ok(Output::AsCoded);
    }
    let unsupported = |what: &str| RawError::Unsupported(format!("lossy JPEG XL tile with {what}"));
    let ColourEncoding::Enum(e) = encoding else {
        return Err(unsupported("an ICC profile instead of a colour encoding"));
    };
    // a single plane is declared grey, three are declared RGB (the header's grey flag is the colour space)
    let grey = planes == 1;
    if grey && e.colour_space != ColourSpace::Grey {
        return Err(unsupported("a single colour plane that is not declared grey"));
    }
    if !grey && e.colour_space != ColourSpace::Rgb {
        return Err(unsupported("a declared encoding that is not RGB"));
    }
    let curve = match e.tf {
        TransferFunction::Linear => Curve::Linear,
        TransferFunction::Srgb => Curve::Srgb,
        TransferFunction::Bt709 => Curve::Bt709,
        TransferFunction::Dci => Curve::Power(1.0 / 2.6),
        // as jxl-oxide reads it: a codestream's gamma is the encoding exponent (×1e7) with `inverted`
        TransferFunction::Gamma { g, inverted } => {
            let g = g as f32 / 1e7;
            Curve::Power(if inverted { g } else { 1.0 / g })
        }
        TransferFunction::Pq | TransferFunction::Hlg => return Err(unsupported("an HDR transfer curve (PQ or HLG)")),
        TransferFunction::Unknown => return Err(unsupported("an unknown transfer curve")),
    };
    if let Curve::Power(p) = curve
        && !(p.is_finite() && p > 0.0)
    {
        return Err(unsupported("an invalid gamma"));
    }
    if e.white_point != WhitePoint::D65 {
        return Err(unsupported("a white point other than D65"));
    }
    if intensity_target > 255.0 {
        return Err(unsupported("an intensity target above 255 nits"));
    }
    if grey {
        // a grey encoding has no primaries, only the white point: its single plane is the luminance (CIE Y)
        // of the decoder's linear sRGB, which is the row of sRGB → XYZ that gives Y. The rows of the
        // matrix are all that row; only the first is used for one plane.
        let y = rgb_to_xyz(SRGB)[1].map(|v| v as f32);
        return Ok(Output::FromXyb { matrix: Some([y; 3]), curve });
    }
    let primaries = match e.primaries {
        Primaries::Srgb => return Ok(Output::FromXyb { matrix: None, curve }),
        Primaries::Bt2100 => BT2100,
        Primaries::P3 => P3,
        Primaries::Custom { red, green, blue } => [red, green, blue].map(|c| c.as_float().map(f64::from)),
    };
    if primaries.iter().any(|p| p[1] <= 0.0) {
        return Err(unsupported("invalid primaries"));
    }
    let m = mul(inverse(rgb_to_xyz(primaries)), rgb_to_xyz(SRGB));
    if m.iter().flatten().any(|v| !v.is_finite()) {
        return Err(unsupported("degenerate primaries"));
    }
    Ok(Output::FromXyb { matrix: Some(m.map(|r| r.map(|v| v as f32))), curve })
}

fn corrupt(e: impl std::fmt::Display) -> RawError {
    RawError::Corrupt(format!("JPEG XL tile: {e}"))
}

/// Parse a chunk's headers (no pixel decoding) and check them against the chunk it must fill:
/// nominal `cw × ch`, or the part of it inside the image `vw × vh` (edge tiles), `cpp` colour planes,
/// a sample type matching the IFD. Returns the decoder and the codestream's integer maximum (0 for float).
#[allow(clippy::too_many_arguments)]
fn open(src: &[u8], cw: usize, ch: usize, vw: usize, vh: usize, cpp: usize, bits: u32, float: bool) -> Result<(JxlImage, f32, Output)> {
    if !matches!(cpp, 1 | 3) {
        return Err(RawError::Unsupported(format!("JPEG XL with {cpp} samples per pixel")));
    }
    if float && bits != 16 || !float && !(8..=16).contains(&bits) {
        return Err(RawError::Unsupported(format!("JPEG XL with {bits}-bit {} samples", if float { "float" } else { "integer" })));
    }
    // The decoder's memory is bounded by the tile itself: a codestream that needs more (extra
    // channels, layers, …) fails to decode instead of allocating what its header asks for.
    let pixels = cw.saturating_mul(ch);
    let budget = |planes: usize| pixels.saturating_mul(planes).saturating_mul(BYTES_PER_SAMPLE);
    let tracker = AllocTracker::with_limit(budget(cpp).saturating_add(MARGIN_BYTES));
    let mut img = JxlImage::builder()
        .pool(JxlThreadPool::none()) // chunks are already decoded in parallel
        .force_wide_buffers(true) // see the module documentation
        .alloc_tracker(tracker.clone())
        .read(std::io::Cursor::new(src))
        .map_err(corrupt)?;
    // lossy (VarDCT) decoding always works on three colour channels, even for a grey tile
    if cpp == 1 && img.frame_header(0).is_some_and(|f| f.encoding == Encoding::VarDct) {
        tracker.expand_limit(budget(2));
    }
    let header = img.image_header();
    let (jw, jh) = (img.width() as usize, img.height() as usize);
    if (jw, jh) != (cw, ch) && (jw, jh) != (vw, vh) {
        return Err(RawError::Corrupt(format!("JPEG XL tile is {jw}×{jh}, expected {cw}×{ch} (or {vw}×{vh} at the image edge)")));
    }
    if header.metadata.orientation != 1 {
        return Err(RawError::Corrupt("JPEG XL tile with a non-identity orientation".into()));
    }
    let planes = if header.metadata.grayscale() { 1 } else { 3 };
    if planes != cpp {
        return Err(RawError::Corrupt(format!("JPEG XL tile has {planes} colour planes, the IFD {cpp} samples per pixel")));
    }
    let max = match header.metadata.bit_depth {
        BitDepth::IntegerSample { bits_per_sample } if !float && (1..=16).contains(&bits_per_sample) => ((1u32 << bits_per_sample) - 1) as f32,
        BitDepth::FloatSample { .. } if float => 0.0,
        d => return Err(RawError::Corrupt(format!("JPEG XL tile sample type {d:?} does not match the IFD"))),
    };
    let output = output_for(header.metadata.xyb_encoded, &header.metadata.colour_encoding, header.metadata.tone_mapping.intensity_target, planes)?;
    if matches!(output, Output::FromXyb { .. }) {
        img.request_color_encoding(EnumColourEncoding::srgb_linear(RenderingIntent::Relative));
    }
    Ok((img, max, output))
}

/// Check the first chunk's headers ([`open`]) without decoding pixels (header-only probing).
#[allow(clippy::too_many_arguments)]
pub(crate) fn check(src: &[u8], cw: usize, ch: usize, vw: usize, vh: usize, cpp: usize, bits: u32, float: bool) -> Result<()> {
    open(src, cw, ch, vw, vh, cpp, bits, float).map(|_| ())
}

/// Decode one chunk into `cw × ch × cpp` samples (row-major, interleaved); a chunk coded at its
/// in-image size `vw × vh` fills the top-left of that, the rest stays 0.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode(src: &[u8], cw: usize, ch: usize, vw: usize, vh: usize, cpp: usize, bits: u32, float: bool) -> Result<ChunkPx> {
    let (img, max, output) = open(src, cw, ch, vw, vh, cpp, bits, float)?;
    if img.num_loaded_keyframes() == 0 {
        return Err(corrupt("no complete frame"));
    }
    let render = img.render_frame(0).map_err(corrupt)?;
    let fb = render.image_all_channels();
    let (jw, jh, fc) = (fb.width(), fb.height(), fb.channels());
    if jw == 0 || jh == 0 || jw > cw || jh > ch || fc < cpp {
        return Err(corrupt(format!("rendered {jw}×{jh}×{fc}, expected {cw}×{ch}×{cpp}")));
    }
    let n = cw.saturating_mul(ch).saturating_mul(cpp);
    let rows = fb.buf().chunks_exact(jw * fc);
    // lossy data: restore the declared primaries and transfer curve (see the module documentation)
    let (matrix, curve) = match output {
        Output::FromXyb { matrix, curve } => (matrix, curve),
        Output::AsCoded => (None, Curve::Linear),
    };
    let restore = |s: &[f32], d: &mut [f32]| {
        match matrix {
            Some(m) => {
                for (d, row) in d.iter_mut().zip(&m) {
                    *d = row[0] * s[0] + row[1] * s[1] + row[2] * s[2];
                }
            }
            None => d.copy_from_slice(&s[..cpp]),
        }
        if curve != Curve::Linear {
            d.iter_mut().for_each(|v| *v = curve.encode(*v));
        }
    };
    Ok(if float {
        let mut out = vec![0f32; n];
        for (dst, src) in out.chunks_exact_mut(cw * cpp).zip(rows) {
            for (d, s) in dst.chunks_exact_mut(cpp).zip(src.chunks_exact(fc)) {
                restore(s, d);
            }
        }
        ChunkPx::F32(out)
    } else {
        let mut out = vec![0u16; n];
        let mut px = [0f32; 3];
        for (dst, src) in out.chunks_exact_mut(cw * cpp).zip(rows) {
            for (d, s) in dst.chunks_exact_mut(cpp).zip(src.chunks_exact(fc)) {
                restore(s, &mut px[..cpp]);
                // `as` saturates (NaN → 0): lossy overshoot clips to the integer range
                d.iter_mut().zip(&px).for_each(|(d, s)| *d = (s * max).round() as u16);
            }
        }
        ChunkPx::U16(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jxl_oxide::color::Customxy;

    fn linear(primaries: Primaries) -> ColourEncoding {
        ColourEncoding::Enum(EnumColourEncoding {
            colour_space: ColourSpace::Rgb,
            white_point: WhitePoint::D65,
            primaries,
            tf: TransferFunction::Linear,
            rendering_intent: RenderingIntent::Perceptual,
        })
    }

    fn matrix(o: Output) -> [[f32; 3]; 3] {
        match o {
            Output::FromXyb { matrix: Some(m), .. } => m,
            other => panic!("expected a matrix, got {other:?}"),
        }
    }

    #[test]
    fn data_that_is_not_xyb_is_rendered_as_coded() {
        // lossless tiles: whatever the header declares, the decoder must not convert anything
        for e in
            [linear(Primaries::Srgb), linear(Primaries::Bt2100), ColourEncoding::Enum(EnumColourEncoding::srgb_gamma22(RenderingIntent::Relative))]
        {
            assert_eq!(output_for(false, &e, 255.0, 3).unwrap(), Output::AsCoded);
            assert_eq!(output_for(false, &e, 10_000.0, 1).unwrap(), Output::AsCoded);
        }
    }

    #[test]
    fn lossy_data_with_srgb_primaries_needs_no_matrix() {
        assert_eq!(output_for(true, &linear(Primaries::Srgb), 255.0, 3).unwrap(), Output::FromXyb { matrix: None, curve: Curve::Linear });
    }

    #[test]
    fn lossy_grey_data_is_the_luminance_in_the_declared_curve() {
        // Lightroom Classic's merged monochrome LinearRaw DNGs: one plane, declared grey with a linear
        // (16-bit float) or an sRGB (16-bit integer) curve
        let grey = |tf| {
            let mut e = EnumColourEncoding::srgb_linear(RenderingIntent::Perceptual);
            e.colour_space = ColourSpace::Grey;
            e.tf = tf;
            output_for(true, &ColourEncoding::Enum(e), 255.0, 1).unwrap()
        };
        let Output::FromXyb { matrix: Some(m), curve: Curve::Linear } = grey(TransferFunction::Linear) else { panic!("grey, linear") };
        // the luminance of linear sRGB (IEC 61966-2-1: 0.2126, 0.7152, 0.0722); white stays white
        for (v, e) in m[0].iter().zip([0.2126, 0.7152, 0.0722]) {
            assert!((v - e).abs() < 1e-4, "{m:?}");
        }
        assert!((m[0].iter().sum::<f32>() - 1.0).abs() < 1e-5, "{m:?}");
        assert!(matches!(grey(TransferFunction::Srgb), Output::FromXyb { curve: Curve::Srgb, .. }));
    }

    #[test]
    fn lossy_data_keeps_the_declared_transfer_curve() {
        // Lightroom Classic's lossy LinearRaw DNGs: Rec.2100 primaries, sRGB transfer curve
        let mut e = EnumColourEncoding::srgb_linear(RenderingIntent::Perceptual);
        e.primaries = Primaries::Bt2100;
        e.tf = TransferFunction::Srgb;
        let curve = |tf| {
            let mut e = e.clone();
            e.tf = tf;
            match output_for(true, &ColourEncoding::Enum(e), 255.0, 3).unwrap() {
                Output::FromXyb { matrix: Some(_), curve } => curve,
                other => panic!("expected a matrix and a curve, got {other:?}"),
            }
        };
        assert_eq!(curve(TransferFunction::Srgb), Curve::Srgb);
        assert_eq!(curve(TransferFunction::Bt709), Curve::Bt709);
        assert_eq!(curve(TransferFunction::Dci), Curve::Power(1.0 / 2.6));
        // a codestream's gamma (as read: inverted) is the encoding exponent; jxl-oxide's own γ 2.2 is not
        assert_eq!(curve(TransferFunction::Gamma { g: 4_545_455, inverted: true }), Curve::Power(0.454_545_5));
        assert_eq!(curve(TransferFunction::Gamma { g: 22_000_000, inverted: false }), Curve::Power(1.0 / 2.2));
    }

    #[test]
    fn transfer_curves_encode_unclipped_and_mirrored() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        // sRGB: linear toe, 18 % grey, white, and the toe/power junction is continuous
        assert!(close(Curve::Srgb.encode(0.002), 0.002 * 12.92));
        assert!(close(Curve::Srgb.encode(0.18), 0.461_356));
        assert!(close(Curve::Srgb.encode(1.0), 1.0));
        assert!(close(Curve::Srgb.encode(0.003_130_8), Curve::Srgb.encode(0.003_130_9)));
        assert!(close(Curve::Bt709.encode(1.0), 1.0));
        assert!((Curve::Bt709.encode(0.017_999) - Curve::Bt709.encode(0.018)).abs() < 1e-3);
        assert!(close(Curve::Power(0.5).encode(0.25), 0.5));
        // raw values above white keep growing, lossy undershoot keeps its sign, linear is the identity
        assert!(Curve::Srgb.encode(2.0) > 1.3);
        assert!(close(Curve::Srgb.encode(-0.18), -0.461_356));
        assert_eq!(Curve::Linear.encode(-3.5), -3.5);
        assert_eq!(Curve::Power(0.5).encode(0.0), 0.0);
    }

    #[test]
    fn lossy_data_is_restored_to_the_declared_primaries() {
        // linear sRGB to linear BT.2020/BT.2100 primaries (D65), as published in ITU-R BT.2087
        let expect = [[0.6274, 0.3293, 0.0433], [0.0691, 0.9195, 0.0114], [0.0164, 0.0880, 0.8956]];
        let m = matrix(output_for(true, &linear(Primaries::Bt2100), 255.0, 3).unwrap());
        for (r, e) in m.iter().zip(&expect) {
            for (v, e) in r.iter().zip(e) {
                assert!((v - e).abs() < 1e-3, "{m:?}");
            }
            // the white point maps to itself: rows sum to one
            assert!((r.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{m:?}");
        }
        // custom primaries equal to the named ones give the same matrix
        let xy = |[x, y]: [f64; 2]| Customxy { x: (x * 1e6) as i32, y: (y * 1e6) as i32 };
        let custom = Primaries::Custom { red: xy(BT2100[0]), green: xy(BT2100[1]), blue: xy(BT2100[2]) };
        let c = matrix(output_for(true, &linear(custom), 255.0, 3).unwrap());
        assert!(c.iter().flatten().zip(m.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-5), "{c:?} {m:?}");
        // the sRGB primaries lie inside the wider P3 gamut: the full-saturation sRGB green is a valid P3 colour
        let inv = matrix(output_for(true, &linear(Primaries::P3), 255.0, 3).unwrap());
        let srgb_green_in_p3 = [inv[0][1], inv[1][1], inv[2][1]];
        assert!(srgb_green_in_p3.iter().all(|v| (0.0..=1.0).contains(v)), "sRGB green lies inside P3");
    }

    // 16 × 16 RGB tiles of 16-bit samples coded lossily (XYB) by libjxl at distance 0.5, each declaring BT.2020
    // primaries, a D65 white point and the named transfer curve. The samples are [`gradient`]; generated for these
    // tests (original work).
    const SRGB_TILE: &[u8] = &[
        0xff, 0x0a, 0x43, 0xf0, 0x93, 0x90, 0xc7, 0x8d, 0x00, 0x13, 0x08, 0x00, 0xf0, 0x01, 0x77, 0x3f, 0xc8, 0x01, 0x00, 0x50, 0xa1, 0x32, 0xca,
        0xb8, 0xc1, 0xcb, 0xb9, 0x9e, 0x2f, 0x3f, 0x74, 0x58, 0x4c, 0xdb, 0xb8, 0xce, 0xd0, 0x56, 0xdb, 0x16, 0x96, 0x30, 0x36, 0xdb, 0x0b, 0x83,
        0x84, 0x84, 0x33, 0x70, 0xde, 0x90, 0x97, 0xbd, 0x79, 0x64, 0xe4, 0x3b, 0x62, 0xaf, 0x1b, 0xb2, 0x74, 0xf6, 0x36, 0x43, 0x17, 0x72, 0xe7,
        0x31, 0x86, 0xb6, 0x41, 0x1c, 0x00, 0x15, 0x52, 0x20, 0x11, 0x70, 0xcb, 0x46, 0x32, 0x92, 0x65, 0xcd, 0xc8, 0x18, 0xc9, 0x48, 0xe6, 0x49,
        0xe6, 0x4f, 0x94, 0x35, 0xc7, 0x5c, 0x91, 0xb0, 0x5d, 0xdf, 0x2b, 0x9b, 0x93, 0x3b, 0xb6, 0x70, 0x7d, 0x24, 0xf0, 0xdd, 0x26, 0xb2, 0x26,
        0xdf, 0x49, 0x5f, 0xd4, 0xaa, 0x1c, 0x8f, 0x7d, 0xc4, 0xf2, 0x6a, 0xd2, 0x27, 0x86, 0x93, 0xe7, 0x15, 0x9a, 0x01, 0x60, 0x84, 0xc0, 0x09,
    ];
    const BT709_TILE: &[u8] = &[
        0xff, 0x0a, 0x43, 0xf0, 0x93, 0x90, 0xa7, 0x08, 0x00, 0x13, 0x08, 0x00, 0xe8, 0x01, 0x77, 0x3f, 0xc8, 0x01, 0x00, 0x50, 0xa1, 0x32, 0xca,
        0xb8, 0xc1, 0xcb, 0xb9, 0x9e, 0x2f, 0x3f, 0x74, 0x58, 0x4c, 0xdb, 0xb8, 0xce, 0xd0, 0x56, 0xdb, 0x16, 0x96, 0x30, 0x36, 0xdb, 0x0b, 0x83,
        0x84, 0x84, 0x33, 0xd8, 0x9c, 0x32, 0xe2, 0x85, 0x6f, 0x3e, 0x6c, 0xa2, 0x1c, 0xaa, 0xdf, 0x1b, 0x32, 0xf4, 0xef, 0x36, 0xe3, 0x96, 0xf2,
        0xe0, 0x65, 0xe8, 0x15, 0xc4, 0x01, 0x50, 0x21, 0x05, 0xd2, 0x00, 0xd8, 0x8c, 0x8c, 0x14, 0x59, 0xc9, 0x31, 0x32, 0x46, 0x52, 0x60, 0x82,
        0x71, 0x03, 0xd9, 0xff, 0x8a, 0x3e, 0x7c, 0xb4, 0xc1, 0xa8, 0x49, 0xfa, 0x83, 0xa8, 0x74, 0xd7, 0x1d, 0xa8, 0xb4, 0x25, 0x67, 0xc1, 0x83,
        0x93, 0x3a, 0xf3, 0x81, 0xb5, 0x2f, 0x44, 0x50, 0x35, 0x36, 0x68, 0x1c, 0xf9, 0x20, 0x17, 0x98, 0x87, 0x1d, 0x83, 0xb0, 0x02,
    ];
    const GAMMA22_TILE: &[u8] = &[
        0xff, 0x0a, 0x43, 0xf0, 0x93, 0x90, 0xf7, 0x75, 0xab, 0x28, 0x02, 0x00, 0x13, 0x08, 0x00, 0xf8, 0x01, 0x77, 0x3f, 0xc8, 0x01, 0x00, 0x50,
        0xa1, 0x32, 0xca, 0xb8, 0xc1, 0xcb, 0xb9, 0x9e, 0x2f, 0x3f, 0x74, 0x58, 0x4c, 0xdb, 0xb8, 0xce, 0xd0, 0x56, 0xdb, 0x16, 0x96, 0x30, 0x36,
        0xdb, 0x0b, 0x83, 0x84, 0x84, 0x33, 0xd8, 0x9c, 0x02, 0xed, 0x85, 0x6f, 0x3e, 0x2c, 0x72, 0x1e, 0xb8, 0xe3, 0x3b, 0x12, 0x75, 0xf8, 0x4e,
        0xa3, 0x97, 0x71, 0xea, 0x31, 0x43, 0xa7, 0x20, 0x0e, 0x80, 0x0a, 0x29, 0x90, 0x08, 0xc0, 0x8a, 0x8c, 0x8a, 0x64, 0x74, 0x59, 0x45, 0xc6,
        0x48, 0x46, 0x32, 0x4c, 0x32, 0x4c, 0xa0, 0xa5, 0x39, 0x23, 0xff, 0xdf, 0xcd, 0xf9, 0x33, 0x93, 0x5c, 0xb4, 0xec, 0xc1, 0x06, 0xf5, 0xcc,
        0x92, 0x12, 0xf4, 0xbb, 0xef, 0x15, 0x84, 0xc7, 0xf2, 0xf1, 0x48, 0xfc, 0x80, 0xb9, 0x7a, 0xab, 0x58, 0x3e, 0x79, 0x3a, 0xcf, 0x67, 0xc7,
        0xd3, 0xfc, 0x80, 0x60, 0x00,
    ];
    const LINEAR_TILE: &[u8] = &[
        0xff, 0x0a, 0x43, 0xf0, 0x93, 0x90, 0x47, 0x8b, 0x00, 0x13, 0x08, 0x00, 0x14, 0x02, 0x77, 0x3f, 0xc8, 0x01, 0x00, 0x50, 0xa1, 0x32, 0xca,
        0xb8, 0xc1, 0xcb, 0xb9, 0x9e, 0x2f, 0x3f, 0x74, 0x58, 0x4c, 0xdb, 0xb8, 0xce, 0xd0, 0x56, 0xdb, 0x16, 0x96, 0x30, 0x36, 0xdb, 0x0b, 0x83,
        0x84, 0x84, 0x43, 0xd8, 0x36, 0xe1, 0xbe, 0x88, 0x9b, 0x27, 0x48, 0x71, 0x04, 0xe2, 0xeb, 0x4e, 0xf9, 0x74, 0x9d, 0xe7, 0x00, 0x8d, 0x74,
        0xcb, 0xc0, 0xe0, 0x5d, 0x58, 0x10, 0x80, 0x0a, 0x29, 0x90, 0x06, 0xd8, 0xac, 0x18, 0x19, 0x45, 0xc5, 0x28, 0x89, 0x8a, 0x24, 0xd3, 0xff,
        0xff, 0x1c, 0xc4, 0x2f, 0xd8, 0x26, 0x80, 0x6c, 0xe1, 0xc3, 0xbe, 0xef, 0xab, 0xe1, 0x79, 0x1e, 0xff, 0x1b, 0x2e, 0x57, 0xf7, 0x2c, 0x1f,
        0x1f, 0xf8, 0xec, 0xd3, 0x33, 0x21, 0x9e, 0x12, 0x22, 0x43, 0xc2, 0xfd, 0xb3, 0xf2, 0xe0, 0x40, 0xdf, 0x25, 0x0f, 0x67, 0xe7, 0xca, 0x7e,
        0xa5, 0x7f, 0x1f, 0x21, 0xa4, 0x6a, 0x1e, 0xa0, 0x0e,
    ];

    /// The samples the fixture tiles were coded from: `(x, y)` → RGB.
    fn gradient(x: usize, y: usize) -> [f32; 3] {
        let (x, y) = (x as f32, y as f32);
        [4000.0 + x * 3500.0, 6000.0 + y * 3200.0, 30000.0 + (x - y) * 1500.0]
    }

    #[test]
    fn lossy_tiles_decode_to_their_samples_whatever_sdr_curve_they_declare() {
        for (name, tile) in [("sRGB", SRGB_TILE), ("BT.709", BT709_TILE), ("gamma 2.2", GAMMA22_TILE), ("linear", LINEAR_TILE)] {
            let ChunkPx::U16(px) = decode(tile, 16, 16, 16, 16, 3, 16, false).unwrap() else { panic!("{name}: integer samples") };
            let (mut sum, mut worst) = (0.0f32, 0.0f32);
            for (i, p) in px.as_chunks::<3>().0.iter().enumerate() {
                for (got, want) in p.iter().zip(gradient(i % 16, i / 16)) {
                    let err = (f32::from(*got) - want).abs();
                    sum += err;
                    worst = worst.max(err);
                }
            }
            // the reference decoder (libjxl) lands within 0.5 % on average and 6 % at worst on these tiles
            let mean = sum / px.len() as f32;
            assert!(mean < 0.01 * 65535.0, "{name}: mean error {mean}");
            assert!(worst < 0.1 * 65535.0, "{name}: worst error {worst}");
        }
    }

    /// A fixture tile re-declared grey: its colour encoding's colour space goes from RGB to Grey and,
    /// as a grey encoding has no primaries, the 6-bit BT.2100 primaries field is dropped (ISO/IEC 18181-1
    /// ColourEncoding; the bits after it move up and the header's zero padding to the frame grows by 6).
    /// The XYB frame data is untouched, so the codestream is one grey plane coded as three XYB channels
    /// whose colour is the gradient's, like Lightroom Classic's merged monochrome DNGs.
    fn as_grey(tile: &[u8]) -> Vec<u8> {
        // bit offset of the colour space, read LSB first, and the frame's first byte in these fixture tiles
        const AT: usize = 42;
        const FRAME: usize = 8;
        let mut bits: Vec<u8> = tile.iter().flat_map(|b| (0..8).map(move |i| (b >> i) & 1)).collect();
        // colour space Rgb (selector 0), white point D65 (selector 1), primaries Bt2100 (selector 2 + 4 bits of 7)
        assert_eq!(bits[AT..AT + 10], [0, 0, 1, 0, 0, 1, 1, 1, 1, 0], "the fixture's colour encoding");
        bits[AT] = 1; // colour space Grey (selector 1)
        bits.drain(AT + 4..AT + 10);
        bits.splice(FRAME * 8 - 6..FRAME * 8 - 6, [0; 6]);
        bits.as_chunks::<8>().0.iter().map(|b| b.iter().enumerate().fold(0, |a, (i, v)| a | v << i)).collect()
    }

    #[test]
    fn lossy_grey_tiles_decode_to_the_luminance_of_their_colour() {
        let srgb_eotf = |v: f32| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let y = rgb_to_xyz(BT2100)[1].map(|v| v as f32);
        for (name, tile, curve) in [("linear", LINEAR_TILE, Curve::Linear), ("sRGB", SRGB_TILE, Curve::Srgb)] {
            let grey = as_grey(tile);
            // the decoder still renders three unequal channels: the restore must reduce them, not pick one
            let mut img = JxlImage::builder().read(std::io::Cursor::new(&grey)).unwrap();
            assert!(img.image_header().metadata.grayscale(), "{name}: declared grey");
            img.request_color_encoding(EnumColourEncoding::srgb_linear(RenderingIntent::Relative));
            let render = img.render_frame(0).unwrap();
            let fb = render.image_all_channels();
            assert_eq!(fb.channels(), 3, "{name}");
            assert!((fb.buf()[2] - fb.buf()[0]).abs() > 0.1, "{name}: the rendered channels differ");

            let ChunkPx::U16(px) = decode(&grey, 16, 16, 16, 16, 1, 16, false).unwrap() else { panic!("{name}: integer samples") };
            assert_eq!(px.len(), 256, "{name}: one plane");
            let (mut sum, mut worst) = (0.0f32, 0.0f32);
            for (i, got) in px.iter().enumerate() {
                // the gradient's samples are coded in the tile's curve: back to linear for the luminance
                let rgb = gradient(i % 16, i / 16).map(|v| if curve == Curve::Srgb { srgb_eotf(v / 65535.0) } else { v / 65535.0 });
                let want = curve.encode(y[0] * rgb[0] + y[1] * rgb[1] + y[2] * rgb[2]) * 65535.0;
                let err = (f32::from(*got) - want).abs();
                sum += err;
                worst = worst.max(err);
            }
            let mean = sum / px.len() as f32;
            assert!(mean < 0.02 * 65535.0, "{name}: mean error {mean}");
            assert!(worst < 0.1 * 65535.0, "{name}: worst error {worst}");
        }
    }

    #[test]
    fn lossy_data_that_cannot_be_restored_is_unsupported() {
        let bad = |xyb_planes: usize, e: ColourEncoding, it: f32| matches!(output_for(true, &e, it, xyb_planes), Err(RawError::Unsupported(_)));
        assert!(bad(1, linear(Primaries::Srgb), 255.0), "one plane declared RGB");
        let mut g = EnumColourEncoding::srgb_linear(RenderingIntent::Relative);
        g.colour_space = ColourSpace::Grey;
        assert!(bad(1, ColourEncoding::Enum(g.clone()), 10_000.0), "grey with an HDR intensity target");
        g.tf = TransferFunction::Pq;
        assert!(bad(1, ColourEncoding::Enum(g.clone()), 255.0), "grey with PQ");
        g.tf = TransferFunction::Linear;
        g.white_point = WhitePoint::Dci;
        assert!(bad(1, ColourEncoding::Enum(g), 255.0), "grey with a white point other than D65");
        assert!(bad(3, linear(Primaries::Srgb), 10_000.0), "HDR intensity target would tone-map");
        let mut e = EnumColourEncoding::srgb_linear(RenderingIntent::Relative);
        for tf in [TransferFunction::Pq, TransferFunction::Hlg, TransferFunction::Unknown, TransferFunction::Gamma { g: 0, inverted: true }] {
            e.tf = tf;
            assert!(bad(3, ColourEncoding::Enum(e.clone()), 255.0), "{tf:?}");
        }
        e.tf = TransferFunction::Linear;
        e.white_point = WhitePoint::Dci;
        assert!(bad(3, ColourEncoding::Enum(e.clone()), 255.0), "white point");
        e.white_point = WhitePoint::D65;
        e.primaries =
            Primaries::Custom { red: Customxy { x: 0, y: 0 }, green: Customxy { x: 300_000, y: 600_000 }, blue: Customxy { x: 150_000, y: 60_000 } };
        assert!(bad(3, ColourEncoding::Enum(e.clone()), 255.0), "degenerate primaries");
        e.colour_space = ColourSpace::Grey;
        assert!(bad(3, ColourEncoding::Enum(e.clone()), 255.0), "grey declared");
    }
}
