//! YCbCr images (PhotometricInterpretation 6, TIFF 6.0 §20–21): the conversion to RGB, and the
//! decoding of uncompressed and PackBits chunky 8-bit data units of any subsampling.
//!
//! [`YCbCr`] maps stored Y, Cb, Cr codes to RGB: ReferenceBlackWhite scales the codes and
//! YCbCrCoefficients gives the luma weights; missing or unusable tags fall back to the TIFF
//! defaults (BT.601 weights, full-range luma, chroma centred on the middle code). JPEG-compressed
//! data (compression 7) uses the JPEG (JFIF) conversion and ignores both tags, as libtiff does
//! when it asks the JPEG library for RGB. Use it on samples another decoder produced.
//!
//! [`decode_rgb8`] decodes a whole image whose data this crate can read itself: one strip-based,
//! chunky, 8-bit, uncompressed or PackBits YCbCr image, the data units expanded (chroma repeated
//! across each unit) and converted.
//!
//! # Green: a compatibility choice
//!
//! Red and blue follow the specification (TIFF 6.0 p. 91: `R = Cr·(2 − 2·LumaRed) + Y`,
//! `B = Cb·(2 − 2·LumaBlue) + Y`). Green follows libtiff (`TIFFYCbCrToRGBInit`), which rewrites
//! the specification's `G = (Y − LumaBlue·B − LumaRed·R) / LumaGreen` as
//! `G = Y − (LumaRed·(2 − 2·LumaRed) / LumaGreen)·Cr − (LumaBlue·(2 − 2·LumaBlue) / LumaGreen)·Cb`,
//! adding Y unscaled. The two agree whenever the weights sum to 1, as every standard set does
//! (BT.601, BT.709). For other weights the specification's form tints neutral grays; libtiff's
//! keeps them neutral and matches libtiff-based tools.

use crate::{Ifd, TiffError, packbits, tags};

/// YCbCrCoefficients.
pub const YCBCR_COEFFICIENTS: u16 = 529;
/// ReferenceBlackWhite.
pub const REFERENCE_BLACK_WHITE: u16 = 532;

/// BT.601 luma weights: the TIFF default for YCbCrCoefficients.
pub const BT601: [f64; 3] = [0.299, 0.587, 0.114];

/// How one image's YCbCr codes map to RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YCbCr {
    /// Luma weights of red, green and blue.
    pub luma: [f64; 3],
    /// Black and white codes of Y, Cb and Cr.
    pub reference_black_white: [f64; 6],
}

impl YCbCr {
    /// TIFF's defaults for samples whose largest code is `max` (255 or 65535): BT.601 weights,
    /// full-range luma, chroma centred on the middle code. Also the JPEG (JFIF) conversion.
    pub fn defaults(max: u32) -> Self {
        let m = f64::from(max);
        let mid = (m + 1.0) / 2.0;
        YCbCr { luma: BT601, reference_black_white: [0.0, m, mid, m, mid, m] }
    }

    /// The conversion `ifd` asks for, for samples whose largest code is `max`. Weights must each
    /// lie strictly between 0 and 1 and every black/white pair must differ; otherwise that tag's
    /// default is used. JPEG-compressed images (compression 7) always use the defaults.
    pub fn from_ifd(ifd: &Ifd, max: u32) -> Self {
        let defaults = Self::defaults(max);
        if ifd.u16(tags::COMPRESSION) == Some(tags::compression::JPEG) {
            return defaults;
        }
        let luma = ifd
            .f64s(YCBCR_COEFFICIENTS)
            .and_then(|v| <[f64; 3]>::try_from(v.get(..3)?).ok())
            .filter(|l| l.iter().all(|c| c.is_finite() && *c > 0.0 && *c < 1.0))
            .unwrap_or(defaults.luma);
        let reference_black_white = ifd
            .f64s(REFERENCE_BLACK_WHITE)
            .and_then(|v| <[f64; 6]>::try_from(v.get(..6)?).ok())
            .filter(|r| r.iter().all(|v| v.is_finite()) && r.as_chunks::<2>().0.iter().all(|[black, white]| (white - black).abs() > f64::EPSILON))
            .unwrap_or(defaults.reference_black_white);
        YCbCr { luma, reference_black_white }
    }

    /// Lookup tables for samples whose largest code is `max`, so a pixel costs a few additions.
    pub fn tables(&self, max: u32) -> Tables {
        let m = f64::from(max);
        let [lr, lg, lb] = self.luma;
        let [y_black, y_white, cb_black, cb_white, cr_black, cr_white] = self.reference_black_white;
        // TIFF 6.0: chroma codes span ±(max + 1) / 2 − 1 around the black code; with the default
        // ReferenceBlackWhite that is the code minus the middle code.
        let half = (m + 1.0) / 2.0 - 1.0;
        let codes = || (0..=max).map(f64::from);
        let y = codes().map(|v| (v - y_black) * m / (y_white - y_black)).collect();
        let cb: Vec<f64> = codes().map(|v| (v - cb_black) * half / (cb_white - cb_black)).collect();
        let cr: Vec<f64> = codes().map(|v| (v - cr_black) * half / (cr_white - cr_black)).collect();
        let (kr, kb) = (2.0 - 2.0 * lr, 2.0 - 2.0 * lb);
        Tables {
            max: m,
            y,
            cr_r: cr.iter().map(|v| kr * v).collect(),
            cr_g: cr.iter().map(|v| -lr * kr * v / lg).collect(),
            cb_g: cb.iter().map(|v| -lb * kb * v / lg).collect(),
            cb_b: cb.iter().map(|v| kb * v).collect(),
        }
    }

    /// Converts interleaved 8-bit pixels of `channels` samples, the first three Y, Cb, Cr, to R,
    /// G, B in place; other samples (alpha) are kept. A trailing partial pixel is left as it is.
    /// The work is per pixel, so a caller may convert bands of whole pixels in parallel.
    pub fn convert_u8(&self, data: &mut [u8], channels: usize) {
        if channels < 3 {
            return;
        }
        let t = self.tables(255);
        for px in data.chunks_exact_mut(channels) {
            if let [y, cb, cr, ..] = px {
                let [r, g, b] = t.rgb(usize::from(*y), usize::from(*cb), usize::from(*cr));
                (*y, *cb, *cr) = (r as u8, g as u8, b as u8);
            }
        }
    }

    /// [`YCbCr::convert_u8`] for 16-bit samples.
    pub fn convert_u16(&self, data: &mut [u16], channels: usize) {
        if channels < 3 {
            return;
        }
        let t = self.tables(65535);
        for px in data.chunks_exact_mut(channels) {
            if let [y, cb, cr, ..] = px {
                let [r, g, b] = t.rgb(usize::from(*y), usize::from(*cb), usize::from(*cr));
                (*y, *cb, *cr) = (r as u16, g as u16, b as u16);
            }
        }
    }
}

/// Per-code contributions of one [`YCbCr`] conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct Tables {
    max: f64,
    /// Scaled luma of each Y code.
    y: Vec<f64>,
    /// Red and green contributions of each Cr code.
    cr_r: Vec<f64>,
    cr_g: Vec<f64>,
    /// Green and blue contributions of each Cb code.
    cb_g: Vec<f64>,
    cb_b: Vec<f64>,
}

impl Tables {
    /// R, G and B of one pixel, rounded and clamped to the code range. Codes past the range read
    /// as 0.
    pub fn rgb(&self, y: usize, cb: usize, cr: usize) -> [u32; 3] {
        let at = |t: &[f64], i: usize| t.get(i).copied().unwrap_or(0.0);
        let yv = at(&self.y, y);
        // Green adds Y unscaled, as libtiff does (see "Green: a compatibility choice").
        let r = yv + at(&self.cr_r, cr);
        let g = yv + at(&self.cr_g, cr) + at(&self.cb_g, cb);
        let b = yv + at(&self.cb_b, cb);
        [r, g, b].map(|v| v.round().clamp(0.0, self.max) as u32)
    }
}

/// The YCbCrSubSampling of `ifd` (default 2 × 2), when it is valid: each factor 1, 2 or 4.
pub fn subsampling(ifd: &Ifd) -> Option<(u32, u32)> {
    match ifd.u64s(tags::YCBCR_SUBSAMPLING).as_deref() {
        None => Some((2, 2)),
        Some([h, v, ..]) if [1, 2, 4].contains(h) && [1, 2, 4].contains(v) => Some((*h as u32, *v as u32)),
        Some(_) => None,
    }
}

/// Bytes of the data units of a `width` × `height` image at `(h, v)` subsampling: the image
/// rounded up to whole units, each `h × v` luma samples then Cb and Cr (TIFF 6.0 §21).
pub fn units_len(width: u32, height: u32, (h, v): (u32, u32)) -> Option<usize> {
    let units = u64::from(width.div_ceil(h.max(1))).checked_mul(u64::from(height.div_ceil(v.max(1))))?;
    usize::try_from(units.checked_mul(u64::from(h) * u64::from(v) + 2)?).ok()
}

/// Expands 8-bit data units to interleaved Y, Cb, Cr pixels, `width × height × 3` bytes, top row
/// first: each unit's chroma is repeated across its luma samples. `None` when `units` is shorter
/// than [`units_len`].
pub fn expand_units(units: &[u8], width: u32, height: u32, (h, v): (u32, u32)) -> Option<Vec<u8>> {
    let (w, ht, sh, sv) = (width as usize, height as usize, h.max(1) as usize, v.max(1) as usize);
    let need = units_len(width, height, (h, v))?;
    let units = units.get(..need)?;
    let units_x = w.div_ceil(sh);
    let mut out = vec![0u8; w.checked_mul(ht)?.checked_mul(3)?];
    for (i, unit) in units.chunks_exact(sh * sv + 2).enumerate() {
        let (ux, uy) = (i % units_x, i / units_x);
        let (luma, chroma) = unit.split_at(sh * sv);
        let [cb, cr] = <[u8; 2]>::try_from(chroma).ok()?;
        for (k, &y) in luma.iter().enumerate() {
            let (x, row) = (ux * sh + k % sh, uy * sv + k / sh);
            if x < w && row < ht {
                let at = (row * w + x) * 3;
                if let Some(px) = out.get_mut(at..at + 3) {
                    px.copy_from_slice(&[y, cb, cr]);
                }
            }
        }
    }
    Some(out)
}

/// Decodes the YCbCr image of `ifd` in `data` (the whole file) to RGB, `width × height × 3`
/// bytes, top row first. The image must be strip-based, chunky, three 8-bit samples a pixel and
/// uncompressed or PackBits; anything else is [`TiffError::Invalid`]. More than `max_pixels`
/// pixels is [`TiffError::Limit`], and a file too small to hold the image is refused before the data units are allocated.
pub fn decode_rgb8(data: &[u8], ifd: &Ifd, max_pixels: u64) -> crate::Result<Vec<u8>> {
    // TiffError has no "unsupported" variant; adding one would break exhaustive matches downstream.
    let unsupported = |what: &str| TiffError::Invalid(format!("unsupported YCbCr image with {what}"));
    let invalid = |what: &str| TiffError::Invalid(format!("YCbCr image: {what}"));
    if ifd.u16(tags::PHOTOMETRIC) != Some(tags::photometric::YCBCR) {
        return Err(invalid("PhotometricInterpretation is not YCbCr"));
    }
    let width = ifd.u32(tags::IMAGE_WIDTH).ok_or(TiffError::MissingTag(tags::IMAGE_WIDTH))?;
    let height = ifd.u32(tags::IMAGE_LENGTH).ok_or(TiffError::MissingTag(tags::IMAGE_LENGTH))?;
    if width == 0 || height == 0 {
        return Err(invalid("empty image"));
    }
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(TiffError::Limit("pixel count"));
    }
    if ifd.u64(tags::SAMPLES_PER_PIXEL).unwrap_or(1) != 3 {
        return Err(unsupported("other than three samples per pixel"));
    }
    if ifd.u64s(tags::BITS_PER_SAMPLE).unwrap_or_else(|| vec![1]).iter().any(|&b| b != 8) {
        return Err(unsupported("other than 8 bits per sample"));
    }
    if ifd.u64(tags::PLANAR_CONFIGURATION).unwrap_or(1) != 1 {
        return Err(unsupported("separate planes"));
    }
    if ifd.contains(tags::TILE_OFFSETS) {
        return Err(unsupported("tiles"));
    }
    let packed = match ifd.u16(tags::COMPRESSION).unwrap_or(tags::compression::NONE) {
        tags::compression::NONE => false,
        tags::compression::PACKBITS => true,
        c => return Err(unsupported(&format!("compression {c}"))),
    };
    let sub = subsampling(ifd).ok_or_else(|| invalid("invalid YCbCrSubSampling"))?;
    let need = units_len(width, height, sub).ok_or(TiffError::Limit("pixel count"))?;
    // Uncompressed units are the file's bytes; PackBits expands at most 64 to 1 (two bytes, 128 out).
    if need > data.len().saturating_mul(if packed { 64 } else { 1 }) {
        return Err(TiffError::Truncated(data.len() as u64));
    }
    let offsets = ifd.u64s(tags::STRIP_OFFSETS).ok_or(TiffError::MissingTag(tags::STRIP_OFFSETS))?;
    let counts = ifd.u64s(tags::STRIP_BYTE_COUNTS).ok_or(TiffError::MissingTag(tags::STRIP_BYTE_COUNTS))?;
    // Fallible: `need` can be hundreds of MiB for a small PackBits file.
    let mut units: Vec<u8> = Vec::new();
    units.try_reserve_exact(need).map_err(|_| TiffError::Limit("memory"))?;
    let mut strip_bytes = 0usize;
    for (&off, &len) in offsets.iter().zip(&counts) {
        if units.len() >= need {
            break;
        }
        let strip = usize::try_from(off)
            .ok()
            .zip(usize::try_from(len).ok())
            .and_then(|(o, l)| data.get(o..o.checked_add(l)?))
            .ok_or(TiffError::Truncated(off))?;
        // The strips of one image do not overlap, so together they cannot exceed the file.
        strip_bytes = strip_bytes.saturating_add(strip.len());
        if strip_bytes > data.len() {
            return Err(invalid("strips overlap"));
        }
        if packed {
            packbits::decode(strip, &mut units, need);
        } else {
            units.extend_from_slice(strip.get(..need - units.len()).unwrap_or(strip));
        }
    }
    if units.len() < need {
        return Err(TiffError::Truncated(data.len() as u64));
    }
    let mut rgb = expand_units(&units, width, height, sub).ok_or_else(|| invalid("data units"))?;
    YCbCr::from_ifd(ifd, 255).convert_u8(&mut rgb, 3);
    Ok(rgb)
}
