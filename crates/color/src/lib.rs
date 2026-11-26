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
    /// This is not a pure-gamma BT.1886 decode.
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
