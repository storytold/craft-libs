//! Colour tags and the conversions compositor and display frames need.
//!
//! This is the slim colour-metadata layer used by craft-frame and by media
//! `MediaInfo`: [`ColorInfo`] (matrix, transfer, primaries, range), [`HdrMetadata`], Y'CbCr
//! matrices, transfer functions, and a few helpers (sRGB tables, HSL, hex). It is not a
//! colour-managed pipeline: LUTs, camera log, grade and tone mapping stay out.
//!
//! Came from FilmCraft's `filmcraft-color` (the metadata and transfer part of that crate).
//! Distinct from `craft-video`'s decoder colour tags (`ColorTags` in #1231 / #1768).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// HDR reference white in cd/m² (ITU-R BT.2408).
pub const REFERENCE_WHITE_NITS: f64 = 203.0;

/// YUV ↔ RGB matrix coefficients.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Matrix {
    Bt601,
    #[default]
    Bt709,
    Bt2020Ncl,
}

impl Matrix {
    /// (Kr, Kb) luma coefficients.
    pub fn kr_kb(self) -> (f32, f32) {
        match self {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
            Matrix::Bt2020Ncl => (0.2627, 0.0593),
        }
    }
    /// From an ISO/IEC 23091-2 `matrix_coefficients` code.
    pub fn from_code(c: u8) -> Option<Matrix> {
        match c {
            1 => Some(Matrix::Bt709),
            5 | 6 => Some(Matrix::Bt601),
            // H.273 table 4: code 10 is constant luminance, not this matrix.
            9 => Some(Matrix::Bt2020Ncl),
            _ => None,
        }
    }
    /// The ISO/IEC 23091-2 `matrix_coefficients` code to write for this matrix.
    /// [`Matrix::Bt601`] writes 6 (SMPTE 170M); code 5 has the same coefficients.
    pub fn to_code(self) -> u8 {
        match self {
            Matrix::Bt709 => 1,
            Matrix::Bt601 => 6,
            Matrix::Bt2020Ncl => 9,
        }
    }
}

/// Transfer characteristics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Transfer {
    /// BT.709 display, approximated with the sRGB curve by this slim conversion layer.
    /// This is not a pure-gamma BT.1886 decode; [`bt1886_eotf`] is.
    #[default]
    Bt709,
    Srgb,
    Linear,
    /// SMPTE ST 2084.
    Pq,
    /// ARIB STD-B67.
    Hlg,
}

impl Transfer {
    /// From an ISO/IEC 23091-2 `transfer_characteristics` code.
    pub fn from_code(c: u8) -> Option<Transfer> {
        match c {
            1 | 6 | 14 | 15 => Some(Transfer::Bt709),
            13 => Some(Transfer::Srgb),
            8 => Some(Transfer::Linear),
            16 => Some(Transfer::Pq),
            18 => Some(Transfer::Hlg),
            _ => None,
        }
    }
    /// The ISO/IEC 23091-2 `transfer_characteristics` code to write for this transfer.
    /// [`Transfer::Bt709`] writes 1; codes 6, 14 and 15 also read as it.
    pub fn to_code(self) -> u8 {
        match self {
            Transfer::Bt709 => 1,
            Transfer::Linear => 8,
            Transfer::Srgb => 13,
            Transfer::Pq => 16,
            Transfer::Hlg => 18,
        }
    }
}

/// Chromaticity primaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Primaries {
    #[default]
    Bt709,
    Bt601_625,
    Bt601_525,
    Bt2020,
    P3D65,
}

impl Primaries {
    /// From an ISO/IEC 23091-2 `colour_primaries` code.
    pub fn from_code(c: u8) -> Option<Primaries> {
        match c {
            1 => Some(Primaries::Bt709),
            5 => Some(Primaries::Bt601_625),
            // SMPTE 240M (7) has the same primaries as SMPTE 170M (6).
            6 | 7 => Some(Primaries::Bt601_525),
            9 => Some(Primaries::Bt2020),
            // H.273 table 2: code 11 is DCI-P3 with its own white point, not D65.
            12 => Some(Primaries::P3D65),
            _ => None,
        }
    }
    /// The ISO/IEC 23091-2 `colour_primaries` code to write for these primaries.
    pub fn to_code(self) -> u8 {
        match self {
            Primaries::Bt709 => 1,
            Primaries::Bt601_625 => 5,
            Primaries::Bt601_525 => 6,
            Primaries::Bt2020 => 9,
            Primaries::P3D65 => 12,
        }
    }
}

/// Quantisation range of integer Y'CbCr samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Range {
    /// 16–235 (8-bit) "video" range.
    #[default]
    Limited,
    Full,
}

/// Colour metadata attached to a compositor or display frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColorInfo {
    pub matrix: Matrix,
    pub transfer: Transfer,
    pub primaries: Primaries,
    pub range: Range,
}

impl ColorInfo {
    pub const SRGB_FULL: ColorInfo = ColorInfo { matrix: Matrix::Bt709, transfer: Transfer::Srgb, primaries: Primaries::Bt709, range: Range::Full };
    pub const REC709: ColorInfo = ColorInfo { matrix: Matrix::Bt709, transfer: Transfer::Bt709, primaries: Primaries::Bt709, range: Range::Limited };
}

/// HDR static metadata of a stream: SMPTE ST 2086 mastering display luminance and the CTA-861.3
/// content light levels (MaxCLL / MaxFALL), in cd/m². Read from `mdcv` / `clli` (MP4 / MOV) and
/// Matroska `MasteringMetadata` / `MaxCLL` / `MaxFALL`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HdrMetadata {
    pub mastering_max_nits: Option<f32>,
    pub mastering_min_nits: Option<f32>,
    pub max_cll: Option<f32>,
    pub max_fall: Option<f32>,
}

impl HdrMetadata {
    /// The content's peak for tone mapping: MaxCLL when signalled (the brightest pixel actually
    /// in the content), else the mastering display's peak; `None` when neither is (0 means
    /// unknown in both). Never below HDR reference white.
    pub fn peak_nits(&self) -> Option<f32> {
        let known = |v: Option<f32>| v.filter(|n| *n > 0.0);
        let p = match (known(self.max_cll), known(self.mastering_max_nits)) {
            (Some(c), _) => c,
            (None, m) => m?,
        };
        Some(p.clamp(REFERENCE_WHITE_NITS as f32, 10_000.0))
    }
}

/// Convert normalised Y'CbCr (Y in 0..1, Cb/Cr in -0.5..0.5) to non-linear R'G'B'.
#[inline]
pub fn ycbcr_to_rgb(y: f32, cb: f32, cr: f32, m: Matrix) -> [f32; 3] {
    let (kr, kb) = m.kr_kb();
    let kg = 1.0 - kr - kb;
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    let g = (y - kr * r - kb * b) / kg;
    [r, g, b]
}

/// Convert non-linear R'G'B' to normalised Y'CbCr.
#[inline]
pub fn rgb_to_ycbcr(r: f32, g: f32, b: f32, m: Matrix) -> [f32; 3] {
    let (kr, kb) = m.kr_kb();
    let kg = 1.0 - kr - kb;
    let y = kr * r + kg * g + kb * b;
    [y, (b - y) / (2.0 * (1.0 - kb)), (r - y) / (2.0 * (1.0 - kr))]
}

/// Unsupported integer sample depth. Normalization supports 8 through 16 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidBitDepth(pub u32);

impl std::fmt::Display for InvalidBitDepth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported sample bit depth {}; expected 8..=16", self.0)
    }
}
impl std::error::Error for InvalidBitDepth {}

/// Normalise an integer luma code value to 0..1 given bit depth and range.
#[inline]
pub fn normalize_y(v: u32, bits: u32, range: Range) -> Result<f32, InvalidBitDepth> {
    if !(8..=16).contains(&bits) {
        return Err(InvalidBitDepth(bits));
    }
    let scale = (1u32 << (bits - 8)) as f32;
    Ok(match range {
        Range::Limited => (v as f32 - 16.0 * scale) / (219.0 * scale),
        Range::Full => v as f32 / ((1u32 << bits) - 1) as f32,
    })
}

/// Normalise an integer chroma code value to −0.5..0.5 given bit depth and range.
#[inline]
pub fn normalize_c(v: u32, bits: u32, range: Range) -> Result<f32, InvalidBitDepth> {
    if !(8..=16).contains(&bits) {
        return Err(InvalidBitDepth(bits));
    }
    let scale = (1u32 << (bits - 8)) as f32;
    Ok(match range {
        Range::Limited => (v as f32 - 128.0 * scale) / (224.0 * scale),
        Range::Full => (v as f32 - (1u32 << (bits - 1)) as f32) / ((1u32 << bits) - 1) as f32,
    })
}

/// Quantize a luma signal in 0..1 to an integer code value: the inverse of [`normalize_y`].
///
/// Limited range follows ITU-R BT.709 and BT.2100 (`round((219 * E + 16) * 2^(bits - 8))`), full
/// range follows BT.2100 (`round((2^bits - 1) * E)`). Values outside the signal range are clamped
/// to the codes `0..=2^bits - 1`, and NaN is treated as 0.
pub fn quantize_y(e: f32, bits: u32, range: Range) -> Result<u16, InvalidBitDepth> {
    if !(8..=16).contains(&bits) {
        return Err(InvalidBitDepth(bits));
    }
    let e = if e.is_nan() { 0.0 } else { f64::from(e) };
    let scale = f64::from(1u32 << (bits - 8));
    let code = match range {
        Range::Limited => (219.0 * e + 16.0) * scale,
        Range::Full => f64::from((1u32 << bits) - 1) * e,
    };
    Ok(clamp_code(code, bits))
}

/// Quantize a chroma signal in −0.5..0.5 to an integer code value: the inverse of
/// [`normalize_c`].
///
/// Limited range is `round((224 * E + 128) * 2^(bits - 8))`, full range is
/// `round((2^bits - 1) * E + 2^(bits - 1))` (BT.2100). Values outside the signal range are
/// clamped to the codes `0..=2^bits - 1`, and NaN is treated as 0 (no chroma).
pub fn quantize_c(e: f32, bits: u32, range: Range) -> Result<u16, InvalidBitDepth> {
    if !(8..=16).contains(&bits) {
        return Err(InvalidBitDepth(bits));
    }
    let e = if e.is_nan() { 0.0 } else { f64::from(e) };
    let scale = f64::from(1u32 << (bits - 8));
    let code = match range {
        Range::Limited => (224.0 * e + 128.0) * scale,
        Range::Full => f64::from((1u32 << bits) - 1) * e + f64::from(1u32 << (bits - 1)),
    };
    Ok(clamp_code(code, bits))
}

#[inline]
fn clamp_code(code: f64, bits: u32) -> u16 {
    // `bits` is at most 16, so the maximum fits in u16.
    code.round().clamp(0.0, f64::from((1u32 << bits) - 1)) as u16
}

/// Convert non-linear R'G'B' (0..1) to integer Y'CbCr code values at a bit depth and range.
pub fn rgb_to_ycbcr_codes(r: f32, g: f32, b: f32, m: Matrix, bits: u32, range: Range) -> Result<[u16; 3], InvalidBitDepth> {
    let [y, cb, cr] = rgb_to_ycbcr(r, g, b, m);
    Ok([quantize_y(y, bits, range)?, quantize_c(cb, bits, range)?, quantize_c(cr, bits, range)?])
}

/// Convert integer Y'CbCr code values at a bit depth and range to non-linear R'G'B'.
///
/// The result is not clamped: codes outside the nominal range (limited-range footroom and
/// headroom) give values below 0 or above 1.
pub fn ycbcr_codes_to_rgb(y: u32, cb: u32, cr: u32, m: Matrix, bits: u32, range: Range) -> Result<[f32; 3], InvalidBitDepth> {
    Ok(ycbcr_to_rgb(normalize_y(y, bits, range)?, normalize_c(cb, bits, range)?, normalize_c(cr, bits, range)?, m))
}

/// Fixed-point conversion between 8-bit R'G'B' and 8-bit Y'CbCr for one matrix and range.
///
/// The coefficients are computed once by [`Ycbcr8::new`]; each conversion is integer
/// multiply-adds with 16 fractional bits, for per-pixel loops over camera or encoder frames.
/// Results stay within one code of [`rgb_to_ycbcr_codes`] and [`ycbcr_codes_to_rgb`], and
/// neutral grays keep chroma at exactly 128.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ycbcr8 {
    y_offset: i32,
    // R'G'B' to Y'CbCr.
    y_rgb: [i32; 3],
    cb_rgb: [i32; 3],
    cr_rgb: [i32; 3],
    // Y'CbCr to R'G'B'.
    rgb_y: i32,
    r_cr: i32,
    g_cb: i32,
    g_cr: i32,
    b_cb: i32,
}

const FIXED_ONE: f64 = 65536.0;
const FIXED_HALF: i32 = 1 << 15;

impl Ycbcr8 {
    /// Coefficients for matrix `m` and range `range`.
    pub fn new(m: Matrix, range: Range) -> Ycbcr8 {
        let (kr, kb) = m.kr_kb();
        let (kr, kb) = (f64::from(kr), f64::from(kb));
        let kg = 1.0 - kr - kb;
        let (y_offset, y_scale, c_scale) = match range {
            Range::Limited => (16, 219.0, 224.0),
            Range::Full => (0, 255.0, 255.0),
        };
        let fixed = |v: f64| (v * FIXED_ONE).round() as i32;
        // Luma: the green weight takes the rounding remainder so 255 gray reaches the top code.
        let y_total = fixed(y_scale / 255.0);
        let (y_r, y_b) = (fixed(kr * y_scale / 255.0), fixed(kb * y_scale / 255.0));
        // Chroma: the green weight cancels the other two so every gray gives exactly 128.
        let cb_scale = c_scale / 255.0 / (2.0 * (1.0 - kb));
        let cr_scale = c_scale / 255.0 / (2.0 * (1.0 - kr));
        let (cb_r, cb_b) = (fixed(-kr * cb_scale), fixed((1.0 - kb) * cb_scale));
        let (cr_r, cr_b) = (fixed((1.0 - kr) * cr_scale), fixed(-kb * cr_scale));
        let r_cr = 2.0 * (1.0 - kr) * 255.0 / c_scale;
        let b_cb = 2.0 * (1.0 - kb) * 255.0 / c_scale;
        Ycbcr8 {
            y_offset,
            y_rgb: [y_r, y_total - y_r - y_b, y_b],
            cb_rgb: [cb_r, -cb_r - cb_b, cb_b],
            cr_rgb: [cr_r, -cr_r - cr_b, cr_b],
            rgb_y: fixed(255.0 / y_scale),
            r_cr: fixed(r_cr),
            g_cb: fixed(-kb * b_cb / kg),
            g_cr: fixed(-kr * r_cr / kg),
            b_cb: fixed(b_cb),
        }
    }

    /// 8-bit R'G'B' to 8-bit Y'CbCr code values.
    #[inline]
    pub fn rgb_to_ycbcr(&self, r: u8, g: u8, b: u8) -> [u8; 3] {
        let rgb = [i32::from(r), i32::from(g), i32::from(b)];
        let dot = |k: &[i32; 3]| k[0] * rgb[0] + k[1] * rgb[1] + k[2] * rgb[2];
        [to_u8(dot(&self.y_rgb) + (self.y_offset << 16)), to_u8(dot(&self.cb_rgb) + (128 << 16)), to_u8(dot(&self.cr_rgb) + (128 << 16))]
    }

    /// 8-bit Y'CbCr code values to 8-bit R'G'B', clamped to 0..=255.
    #[inline]
    pub fn ycbcr_to_rgb(&self, y: u8, cb: u8, cr: u8) -> [u8; 3] {
        let y = self.rgb_y * (i32::from(y) - self.y_offset);
        let (cb, cr) = (i32::from(cb) - 128, i32::from(cr) - 128);
        [to_u8(y + self.r_cr * cr), to_u8(y + self.g_cb * cb + self.g_cr * cr), to_u8(y + self.b_cb * cb)]
    }
}

/// Round a 16.16 fixed-point value to the nearest integer and clamp it to a u8.
#[inline]
fn to_u8(v: i32) -> u8 {
    ((v + FIXED_HALF) >> 16).clamp(0, 255) as u8
}

/// sRGB electro-optical transfer (encoded → linear).
#[inline]
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// sRGB inverse EOTF (linear → encoded).
#[inline]
pub fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Encoded → linear for a transfer function (display-referred; PQ normalised to 100 nits = 1.0).
pub fn to_linear(v: f32, t: Transfer) -> f32 {
    match t {
        Transfer::Srgb => srgb_to_linear(v),
        // Rec.709 is treated as display gamma 2.4; the sRGB curve is visually identical for
        // previews and keeps round-trips exact.
        Transfer::Bt709 => srgb_to_linear(v),
        Transfer::Linear => v,
        Transfer::Pq => pq_eotf(v) * 100.0,
        Transfer::Hlg => hlg_inverse_oetf(v),
    }
}

/// Linear → encoded for a transfer function (the inverse of [`to_linear`]).
pub fn from_linear(v: f32, t: Transfer) -> f32 {
    match t {
        Transfer::Srgb | Transfer::Bt709 => linear_to_srgb(v),
        Transfer::Linear => v,
        Transfer::Pq => pq_inverse_eotf(v / 100.0),
        Transfer::Hlg => hlg_oetf(v),
    }
}

/// SMPTE ST 2084 EOTF, output normalised to 10 000 nits = 1.0.
pub fn pq_eotf(e: f32) -> f32 {
    let (m1, m2) = (0.159_301_76_f32, 78.84375_f32);
    let (c1, c2, c3) = (0.835_937_5_f32, 18.851_563_f32, 18.6875_f32);
    let p = e.max(0.0).powf(1.0 / m2);
    ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1)
}

/// Inverse of [`pq_eotf`].
pub fn pq_inverse_eotf(y: f32) -> f32 {
    let (m1, m2) = (0.159_301_76_f32, 78.84375_f32);
    let (c1, c2, c3) = (0.835_937_5_f32, 18.851_563_f32, 18.6875_f32);
    let p = y.max(0.0).powf(m1);
    ((c1 + c2 * p) / (1.0 + c3 * p)).powf(m2)
}

/// ARIB STD-B67 HLG OETF.
pub fn hlg_oetf(l: f32) -> f32 {
    let (a, b, c) = (0.178_832_77_f32, 0.284_668_92_f32, 0.559_910_7_f32);
    if l <= 1.0 / 12.0 { (3.0 * l.max(0.0)).sqrt() } else { a * (12.0 * l - b).ln() + c }
}

/// Inverse of [`hlg_oetf`].
pub fn hlg_inverse_oetf(e: f32) -> f32 {
    let (a, b, c) = (0.178_832_77_f32, 0.284_668_92_f32, 0.559_910_7_f32);
    if e <= 0.5 { e * e / 3.0 } else { (((e - c) / a).exp() + b) / 12.0 }
}

/// The BT.1886 exponent.
const BT1886_GAMMA: f64 = 2.4;

/// BT.1886 constants `a` and `b` for a display's white and black luminance, or `None` when white
/// is not measurably above black, or is NaN or infinite. A black below 0, or NaN, is taken as 0.
/// The constants are in `f64`, so a black close to white keeps the curve's endpoints.
fn bt1886_ab(white: f32, black: f32) -> Option<(f64, f64)> {
    let (white, black) = (f64::from(white), f64::from(black.max(0.0)));
    let (w, k) = (white.powf(1.0 / BT1886_GAMMA), black.powf(1.0 / BT1886_GAMMA));
    let d = w - k;
    if d.is_finite() && d > 0.0 { Some((d.powf(BT1886_GAMMA), k / d)) } else { None }
}

/// ITU-R BT.1886 reference display EOTF: a BT.709 signal (0..1) to screen luminance, for a
/// display with the given white and black luminance (any unit, e.g. cd/m²). With `white` 1 and
/// `black` 0 it is a pure 2.4 gamma. [`to_linear`] with [`Transfer::Bt709`] uses the sRGB curve
/// instead. A `black` below 0, or NaN, is taken as 0. When `white` is not measurably above
/// `black`, or is NaN or infinite, the result is `black`.
pub fn bt1886_eotf(v: f32, white: f32, black: f32) -> f32 {
    match bt1886_ab(white, black) {
        Some((a, b)) => (a * (f64::from(v) + b).max(0.0).powf(BT1886_GAMMA)) as f32,
        None => black.max(0.0),
    }
}

/// Inverse of [`bt1886_eotf`]: screen luminance to a BT.709 signal. Luminance at or below
/// `black` gives 0. When `white` is not measurably above `black`, or is NaN or infinite, the
/// result is 0.
pub fn bt1886_inverse_eotf(l: f32, white: f32, black: f32) -> f32 {
    match bt1886_ab(white, black) {
        Some((a, b)) if l > black.max(0.0) => ((f64::from(l) / a).powf(1.0 / BT1886_GAMMA) - b).max(0.0) as f32,
        _ => 0.0,
    }
}

/// 256-entry table: sRGB-encoded u8 → linear f32.
pub fn srgb_u8_to_linear_table() -> &'static [f32; 256] {
    static T: OnceLock<[f32; 256]> = OnceLock::new();
    T.get_or_init(|| std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0)))
}

/// 4096-entry table: linear (0..1, quantised to 12 bits) → sRGB-encoded u8.
pub fn linear_to_srgb_u8_table() -> &'static [u8; 4096] {
    static T: OnceLock<[u8; 4096]> = OnceLock::new();
    T.get_or_init(|| std::array::from_fn(|i| (linear_to_srgb(i as f32 / 4095.0) * 255.0 + 0.5).clamp(0.0, 255.0) as u8))
}

/// Linear 0..1 to an sRGB-encoded u8, via [`linear_to_srgb_u8_table`].
#[inline]
pub fn linear_to_srgb_u8(v: f32) -> u8 {
    let i = (v.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize;
    linear_to_srgb_u8_table()[i]
}

/// Rec.709 luma of a linear or encoded RGB triple.
#[inline]
pub fn luma709(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// RGB (0..1) → HSL (h in 0..1).
pub fn rgb_to_hsl(r: f32, g: f32, b: f32) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-7 {
        return [0.0, 0.0, l];
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h / 6.0, s, l]
}

/// Inverse of [`rgb_to_hsl`].
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s <= 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

/// Parse `#rrggbb` / `#rrggbbaa` into 0..1 floats.
pub fn parse_hex(s: &str) -> Option<[f32; 4]> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|v| v as f32 / 255.0);
    match s.len() {
        6 => Some([b(0)?, b(2)?, b(4)?, 1.0]),
        8 => Some([b(0)?, b(2)?, b(4)?, b(6)?]),
        _ => None,
    }
}

/// The inverse of [`parse_hex`], lowercase, omitting alpha when it is 1.
pub fn to_hex(c: [f32; 4]) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if c[3] >= 0.999 {
        format!("#{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]))
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]), q(c[3]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_rejects_signs() {
        assert_eq!(parse_hex("#+f+f+f"), None);
        assert_eq!(parse_hex("#-f-f-f"), None);
        assert_eq!(parse_hex("##0f0f0f"), None);
        assert_eq!(parse_hex("#0f0f0"), None);
        assert!(parse_hex("#0f0f0f").is_some());
        assert!(parse_hex("0f0f0f80").is_some());
    }

    #[test]
    fn ycbcr_roundtrip() {
        for m in [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020Ncl] {
            let rgb = [0.8, 0.3, 0.1];
            let y = rgb_to_ycbcr(rgb[0], rgb[1], rgb[2], m);
            let back = ycbcr_to_rgb(y[0], y[1], y[2], m);
            for k in 0..3 {
                assert!((rgb[k] - back[k]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn invalid_bit_depths_are_errors_not_shifts() {
        for bits in [0, 1, 7, 17, 31, 32, u32::MAX] {
            assert_eq!(normalize_y(0, bits, Range::Full), Err(InvalidBitDepth(bits)));
            assert_eq!(normalize_c(0, bits, Range::Limited), Err(InvalidBitDepth(bits)));
        }
    }

    #[test]
    fn limited_range_levels() {
        assert_eq!(normalize_y(16, 8, Range::Limited).unwrap(), 0.0);
        assert_eq!(normalize_y(235, 8, Range::Limited).unwrap(), 1.0);
        assert_eq!(normalize_y(940, 10, Range::Limited).unwrap(), 1.0);
        assert_eq!(normalize_c(128, 8, Range::Limited).unwrap(), 0.0);
    }

    #[test]
    fn transfer_roundtrips() {
        for t in [Transfer::Srgb, Transfer::Pq, Transfer::Hlg, Transfer::Linear] {
            for v in [0.0, 0.1, 0.5, 0.9] {
                let l = to_linear(v, t);
                assert!((from_linear(l, t) - v).abs() < 1e-3, "{t:?} {v}");
            }
        }
        assert_eq!(linear_to_srgb_u8(1.0), 255);
        assert_eq!(linear_to_srgb_u8(0.0), 0);
        assert_eq!(linear_to_srgb_u8(srgb_u8_to_linear_table()[128]), 128);
    }

    /// SMPTE ST 2084 inverse EOTF in `f64`, with the constants as exact fractions from
    /// Recommendation ITU-R BT.2100 Table 4.
    fn st2084_inverse_eotf(y: f64) -> f64 {
        let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
        let (c1, c2, c3) = (3424.0 / 4096.0, 2413.0 / 4096.0 * 32.0, 2392.0 / 4096.0 * 32.0);
        let p = y.powf(m1);
        ((c1 + c2 * p) / (1.0 + c3 * p)).powf(m2)
    }

    #[test]
    fn pq_matches_the_st_2084_equations() {
        for nits in [0.01_f64, 0.1, 1.0, 10.0, 100.0, 203.0, 1000.0, 4000.0, 10_000.0] {
            let want = st2084_inverse_eotf(nits / 10_000.0);
            let got = pq_inverse_eotf((nits / 10_000.0) as f32) as f64;
            assert!((got - want).abs() < 1e-5, "{nits} nits: {got} vs {want}");
            let back = pq_eotf(want as f32) as f64 * 10_000.0;
            assert!((back - nits).abs() <= nits * 1e-4, "{want}: {back} vs {nits} nits");
        }
        assert!((pq_inverse_eotf(1.0) - 1.0).abs() < 1e-6);
        assert!((pq_eotf(1.0) - 1.0).abs() < 1e-6);
    }

    /// Report ITU-R BT.2408-7 Table 1: display luminance in cd/m², then the nominal signal level in
    /// whole percent for a PQ reference display and for a 1 000 cd/m² HLG display.
    const BT2408_TABLE_1: [(f32, f32, f32); 4] = [(26.0, 38.0, 38.0), (162.0, 56.0, 71.0), (179.0, 57.0, 73.0), (203.0, 58.0, 75.0)];

    #[test]
    fn signal_levels_match_bt2408_table_1() {
        for (nits, pq_percent, hlg_percent) in BT2408_TABLE_1 {
            let pq = pq_inverse_eotf(nits / 10_000.0) * 100.0;
            assert!((pq - pq_percent).abs() <= 0.5, "{nits} nits: PQ {pq}% vs {pq_percent}%");
            // The BT.2100 HLG OOTF has a system gamma of 1.2 on a 1 000 cd/m² display, so a
            // neutral gray shown at `nits` comes from scene light (nits / 1000)^(1 / 1.2).
            let hlg = hlg_oetf((nits / 1000.0).powf(1.0 / 1.2)) * 100.0;
            assert!((hlg - hlg_percent).abs() <= 0.5, "{nits} nits: HLG {hlg}% vs {hlg_percent}%");
        }
    }

    #[test]
    fn hlg_matches_the_bt2100_equations() {
        // Recommendation ITU-R BT.2100 Table 5: a = 0.17883277, b = 1 - 4a, c = 0.5 - a ln(4a).
        let a = 0.178_832_77_f64;
        let b = 1.0 - 4.0 * a;
        let c = 0.5 - a * (4.0 * a).ln();
        let oetf = |e: f64| if e <= 1.0 / 12.0 { (3.0 * e).sqrt() } else { a * (12.0 * e - b).ln() + c };
        for e in [0.0_f64, 0.001, 0.01, 1.0 / 12.0, 0.1, 0.25, 0.5, 0.75, 1.0] {
            let want = oetf(e);
            let got = hlg_oetf(e as f32) as f64;
            assert!((got - want).abs() < 1e-6, "E = {e}: {got} vs {want}");
            let back = hlg_inverse_oetf(want as f32) as f64;
            assert!((back - e).abs() <= 1e-6 + e * 1e-5, "E' = {want}: {back} vs {e}");
        }
        assert!((hlg_oetf(1.0 / 12.0) - 0.5).abs() < 1e-6);
        assert!((hlg_oetf(1.0) - 1.0).abs() < 1e-6);
    }

    /// The sRGB transfer of IEC 61966-2-1 (Amendment 1), as the ICC sRGB registry entry gives it.
    #[test]
    fn srgb_tables_match_the_iec_61966_2_1_equations() {
        let decode = |v: f64| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let encode = |l: f64| if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        for (i, got) in srgb_u8_to_linear_table().iter().enumerate() {
            let want = decode(i as f64 / 255.0);
            assert!((*got as f64 - want).abs() <= 1e-6, "code {i}: {got} vs {want}");
        }
        for (i, got) in linear_to_srgb_u8_table().iter().enumerate() {
            let want = (encode(i as f64 / 4095.0) * 255.0).round() as u8;
            assert_eq!(*got, want, "linear {i}/4095");
        }
    }

    #[test]
    fn bt1886_follows_the_recommendation() {
        // BT.1886 with Lw = 100 cd/m² and Lb = 0: L = 100 · V^2.4.
        assert!((bt1886_eotf(0.5, 100.0, 0.0) - 100.0 * 0.5f32.powf(2.4)).abs() < 1e-3);
        // A pure gamma has no linear toe, so near black it is darker than the sRGB curve.
        assert!(bt1886_eotf(0.02, 1.0, 0.0) < to_linear(0.02, Transfer::Bt709) / 10.0);
        // A black level lifts code 0 to the display's black and keeps code 1 at its white.
        for (white, black) in [(100.0, 0.1), (300.0, 0.5), (1.0, 0.0)] {
            assert!((bt1886_eotf(0.0, white, black) - black).abs() < 1e-4 * white);
            assert!((bt1886_eotf(1.0, white, black) - white).abs() < 1e-4 * white);
            for v in [0.0, 0.1, 0.5, 0.9, 1.0] {
                let l = bt1886_eotf(v, white, black);
                assert!((bt1886_inverse_eotf(l, white, black) - v).abs() < 1e-4, "{white} {black} {v}");
            }
            assert_eq!(bt1886_inverse_eotf(black / 2.0, white, black), 0.0);
        }
        assert!(bt1886_eotf(0.5, 100.0, 0.1) > bt1886_eotf(0.5, 100.0, 0.0));
        // A black close to white keeps both endpoints of the inverse.
        assert_eq!(bt1886_inverse_eotf(99.99, 100.0, 99.99), 0.0);
        assert!((bt1886_inverse_eotf(100.0, 100.0, 99.99) - 1.0).abs() < 1e-3);
        // A display whose white is not above its black.
        assert_eq!(bt1886_eotf(0.5, 1.0, 1.0), 1.0);
        assert_eq!(bt1886_inverse_eotf(0.5, 0.0, 0.0), 0.0);
        assert_eq!(bt1886_inverse_eotf(0.5, f32::NAN, 0.0), 0.0);
        assert_eq!(bt1886_eotf(0.5, 100.0, f32::NAN), bt1886_eotf(0.5, 100.0, 0.0));
        for (white, black) in [(1.0, 0.999_999_94), (f32::INFINITY, 0.0)] {
            assert!(bt1886_eotf(0.5, white, black).is_finite());
            assert!(bt1886_inverse_eotf(0.5, white, black).is_finite());
        }
    }

    #[test]
    fn hsl_roundtrip() {
        let c = [0.2, 0.6, 0.9];
        let h = rgb_to_hsl(c[0], c[1], c[2]);
        let b = hsl_to_rgb(h[0], h[1], h[2]);
        for k in 0..3 {
            assert!((c[k] - b[k]).abs() < 1e-5);
        }
    }

    #[test]
    fn hex() {
        assert_eq!(parse_hex("#ff8000"), Some([1.0, 128.0 / 255.0, 0.0, 1.0]));
        assert_eq!(to_hex([1.0, 0.0, 0.0, 1.0]), "#ff0000");
    }

    #[test]
    fn hdr_peak_nits() {
        assert_eq!(HdrMetadata::default().peak_nits(), None);
        assert_eq!(HdrMetadata { max_cll: Some(4000.0), mastering_max_nits: Some(1000.0), ..HdrMetadata::default() }.peak_nits(), Some(4000.0));
        assert_eq!(HdrMetadata { max_cll: Some(1000.0), mastering_max_nits: Some(4000.0), ..HdrMetadata::default() }.peak_nits(), Some(1000.0));
        assert_eq!(HdrMetadata { max_cll: Some(0.0), mastering_max_nits: Some(400.0), ..HdrMetadata::default() }.peak_nits(), Some(400.0));
        // never below reference white
        assert_eq!(HdrMetadata { max_cll: Some(100.0), ..HdrMetadata::default() }.peak_nits(), Some(REFERENCE_WHITE_NITS as f32));
    }

    #[test]
    fn code_values_match_published_levels() {
        // The BT.709 and BT.2100 quantization levels: black, nominal peak and achromatic chroma.
        assert_eq!(rgb_to_ycbcr_codes(0.0, 0.0, 0.0, Matrix::Bt709, 8, Range::Limited), Ok([16, 128, 128]));
        assert_eq!(rgb_to_ycbcr_codes(1.0, 1.0, 1.0, Matrix::Bt709, 8, Range::Limited), Ok([235, 128, 128]));
        assert_eq!(rgb_to_ycbcr_codes(1.0, 1.0, 1.0, Matrix::Bt2020Ncl, 10, Range::Limited), Ok([940, 512, 512]));
        assert_eq!(rgb_to_ycbcr_codes(0.0, 0.0, 0.0, Matrix::Bt2020Ncl, 10, Range::Limited), Ok([64, 512, 512]));
        assert_eq!(rgb_to_ycbcr_codes(1.0, 1.0, 1.0, Matrix::Bt2020Ncl, 12, Range::Full), Ok([4095, 2048, 2048]));
        // 100% red: Y'CbCr of the BT.709 and BT.601 color bars.
        assert_eq!(rgb_to_ycbcr_codes(1.0, 0.0, 0.0, Matrix::Bt709, 8, Range::Limited), Ok([63, 102, 240]));
        assert_eq!(rgb_to_ycbcr_codes(1.0, 0.0, 0.0, Matrix::Bt601, 8, Range::Limited), Ok([81, 90, 240]));
        // JFIF (ITU-T T.871) full-range BT.601: red is Y 76, Cb 85, Cr 255.5 clamped to 255.
        assert_eq!(rgb_to_ycbcr_codes(1.0, 0.0, 0.0, Matrix::Bt601, 8, Range::Full), Ok([76, 85, 255]));
    }

    #[test]
    fn quantize_inverts_normalize() {
        for range in [Range::Limited, Range::Full] {
            for bits in [8, 10, 12, 16] {
                for v in [0, 1, 16, 64, 128, 235, 240, 255] {
                    let v = v << (bits - 8);
                    assert_eq!(u32::from(quantize_y(normalize_y(v, bits, range).unwrap(), bits, range).unwrap()), v, "{range:?} {bits} y {v}");
                    assert_eq!(u32::from(quantize_c(normalize_c(v, bits, range).unwrap(), bits, range).unwrap()), v, "{range:?} {bits} c {v}");
                }
                let top = (1u32 << bits) - 1;
                assert_eq!(u32::from(quantize_y(f32::INFINITY, bits, range).unwrap()), top);
                assert_eq!(quantize_c(f32::NEG_INFINITY, bits, range), Ok(0));
                assert_eq!(quantize_c(f32::NAN, bits, range), quantize_c(0.0, bits, range));
                assert_eq!(quantize_y(f32::NAN, bits, range), quantize_y(0.0, bits, range));
            }
        }
        for bits in [0, 7, 17, u32::MAX] {
            assert_eq!(quantize_y(0.5, bits, Range::Full), Err(InvalidBitDepth(bits)));
            assert_eq!(quantize_c(0.0, bits, Range::Limited), Err(InvalidBitDepth(bits)));
            assert_eq!(rgb_to_ycbcr_codes(0.5, 0.5, 0.5, Matrix::Bt709, bits, Range::Full), Err(InvalidBitDepth(bits)));
            assert_eq!(ycbcr_codes_to_rgb(0, 0, 0, Matrix::Bt709, bits, Range::Full), Err(InvalidBitDepth(bits)));
        }
    }

    #[test]
    fn ycbcr_codes_roundtrip() {
        for m in [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020Ncl] {
            for range in [Range::Limited, Range::Full] {
                let [y, cb, cr] = rgb_to_ycbcr_codes(0.8, 0.3, 0.1, m, 10, range).unwrap();
                let back = ycbcr_codes_to_rgb(y.into(), cb.into(), cr.into(), m, 10, range).unwrap();
                for (k, v) in [0.8, 0.3, 0.1].into_iter().enumerate() {
                    assert!((back[k] - v).abs() < 4e-3, "{m:?} {range:?} {back:?}");
                }
            }
        }
    }

    #[test]
    fn fixed_point_8_bit_matches_float() {
        let close = |a: [u8; 3], b: [u16; 3]| a.iter().zip(b).all(|(&a, b)| (i32::from(a) - i32::from(b)).abs() <= 1);
        for m in [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020Ncl] {
            for range in [Range::Limited, Range::Full] {
                let conv = Ycbcr8::new(m, range);
                for r in (0..=255u8).step_by(5) {
                    for g in (0..=255u8).step_by(5) {
                        for b in (0..=255u8).step_by(5) {
                            let float = rgb_to_ycbcr_codes(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, m, 8, range).unwrap();
                            let fixed = conv.rgb_to_ycbcr(r, g, b);
                            assert!(close(fixed, float), "{m:?} {range:?} {r} {g} {b}: {fixed:?} vs {float:?}");
                            let [y, cb, cr] = fixed;
                            let rgb = ycbcr_codes_to_rgb(y.into(), cb.into(), cr.into(), m, 8, range).unwrap();
                            let want = rgb.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u16);
                            assert!(close(conv.ycbcr_to_rgb(y, cb, cr), want), "{m:?} {range:?} {fixed:?}");
                        }
                    }
                }
                for v in 0..=255u8 {
                    let [_, cb, cr] = conv.rgb_to_ycbcr(v, v, v);
                    assert_eq!((cb, cr), (128, 128), "{m:?} {range:?} gray {v}");
                }
                // Out-of-range codes clamp instead of wrapping.
                assert_eq!(conv.ycbcr_to_rgb(255, 255, 255)[0], 255);
                assert_eq!(conv.ycbcr_to_rgb(0, 0, 0)[0], 0);
            }
            assert_eq!(Ycbcr8::new(m, Range::Limited).rgb_to_ycbcr(255, 255, 255), [235, 128, 128]);
            assert_eq!(Ycbcr8::new(m, Range::Full).rgb_to_ycbcr(255, 255, 255), [255, 128, 128]);
            assert_eq!(Ycbcr8::new(m, Range::Limited).ycbcr_to_rgb(235, 128, 128), [255, 255, 255]);
            assert_eq!(Ycbcr8::new(m, Range::Limited).ycbcr_to_rgb(16, 128, 128), [0, 0, 0]);
        }
    }

    #[test]
    fn codes() {
        assert_eq!(Matrix::from_code(9), Some(Matrix::Bt2020Ncl));
        assert_eq!(Matrix::from_code(10), None);
        assert_eq!(Matrix::from_code(1), Some(Matrix::Bt709));
        assert_eq!(Matrix::from_code(6), Some(Matrix::Bt601));
        assert_eq!(Transfer::from_code(16), Some(Transfer::Pq));
        assert_eq!(Transfer::from_code(99), None);
    }

    #[test]
    fn codes_written_are_the_h273_values() {
        assert_eq!([Matrix::Bt709, Matrix::Bt601, Matrix::Bt2020Ncl].map(Matrix::to_code), [1, 6, 9]);
        assert_eq!([Transfer::Bt709, Transfer::Linear, Transfer::Srgb, Transfer::Pq, Transfer::Hlg].map(Transfer::to_code), [1, 8, 13, 16, 18]);
        assert_eq!(
            [Primaries::Bt709, Primaries::Bt601_625, Primaries::Bt601_525, Primaries::Bt2020, Primaries::P3D65].map(Primaries::to_code),
            [1, 5, 6, 9, 12]
        );
    }

    #[test]
    fn primaries_codes() {
        assert_eq!(Primaries::from_code(1), Some(Primaries::Bt709));
        assert_eq!(Primaries::from_code(5), Some(Primaries::Bt601_625));
        assert_eq!(Primaries::from_code(6), Some(Primaries::Bt601_525));
        assert_eq!(Primaries::from_code(7), Some(Primaries::Bt601_525));
        assert_eq!(Primaries::from_code(9), Some(Primaries::Bt2020));
        assert_eq!(Primaries::from_code(12), Some(Primaries::P3D65));
        for c in [0, 2, 3, 4, 8, 10, 11, 13, 22, 255] {
            assert_eq!(Primaries::from_code(c), None, "{c}");
        }
    }

    #[test]
    fn written_codes_read_back() {
        for m in [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020Ncl] {
            assert_eq!(Matrix::from_code(m.to_code()), Some(m));
        }
        for t in [Transfer::Bt709, Transfer::Srgb, Transfer::Linear, Transfer::Pq, Transfer::Hlg] {
            assert_eq!(Transfer::from_code(t.to_code()), Some(t));
        }
        for p in [Primaries::Bt709, Primaries::Bt601_625, Primaries::Bt601_525, Primaries::Bt2020, Primaries::P3D65] {
            assert_eq!(Primaries::from_code(p.to_code()), Some(p));
        }
    }
}
