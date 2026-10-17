//! Page length: how a scanned page got its length, and the opt-in inference of a sheet's end.
//!
//! # The policy
//!
//! A flatbed has a fixed bed. A document feeder does not know how long the sheet it just pulled is,
//! unless the device says so; many do not (a WIA driver can list one `A4` page size and an adjustable
//! vertical extent, and nothing that reports the sheet's length). [`PageLength`] is the request, in
//! priority order:
//!
//! 1. **Device detection**, when the device publishes it (WIA `WIA_IPS_AUTO_CROP`, the page-size value
//!    `WIA_PAGE_AUTO`, `WIA_IPS_LONG_DOCUMENT`; WS-Scan `DocumentSizeAutoDetect`; TWAIN
//!    `ICAP_AUTOMATICLENGTHDETECTION`; eSCL has none). Support is three-state ([`Presence`]): it counts
//!    as supported only when the property or value is *published*. Recorded as [`LengthSource::Device`].
//! 2. **An explicit page size**: [`ScanSettings::page_size`] becomes extents. This is deterministic and
//!    works on any scanner that takes an extent. A size the item does not list is written as extents
//!    inside the item's published range, never as an unlisted page-size value; outside the range it is
//!    clamped or refused. Recorded as [`LengthSource::Size`].
//! 3. **Inference from the image** ([`PageLength::AutoInfer`], opt in): request the item's maximum
//!    extent and crop the received page at the end of the sheet, found by [`infer_sheet_end`]. This is a
//!    heuristic measured on one scanner and is labelled approximate ([`LengthRecord::approximate`]). It
//!    is never the default.
//!
//! [`PageLength::Driver`] (the default) does none of this: the driver keeps whatever extent it has.
//! [`PageLength::Size`] is step 2 alone, for a caller that wants the chosen size's length even when the device
//! could detect one.
//!
//! # The inference rule
//!
//! Some scanners fill everything beyond the sheet end (and a thin frame outside the default region) with
//! pure white `255`, while the paper itself carries scanner noise (values scattered in the high 200s).
//! The last row that holds a pixel that is not `255` is therefore the sheet end. Two tests keep the rule
//! from cutting at the end of printed content instead:
//!
//! * a **fill** pixel is exactly `255` (`tolerance` 0 for lossless formats; a small tolerance for JPEG);
//! * the rows just above the candidate end (the *band*) must show **paper noise**: at least
//!   [`InferOptions::min_noise_permille`] of their pixels are near-white but not fill (`noise_floor..=254`).
//!   A scan whose white margin is exactly `255` everywhere (a noiseless scanner, a synthetic page) has no
//!   such noise, so the end of the printing cannot be told from the end of the sheet and inference
//!   **refuses** ([`Refusal::NoPaperNoise`]).
//!
//! Only the bottom is ever cropped; the width is never touched. Nothing is cropped when content or noise
//! runs to the last row (no padding to remove).
//!
//! Honest limits: the rule was measured on one scanner; a scanner that pads with something other than
//! pure white is not recognised; a photograph with near-white noisy content reaching the bottom edge of
//! a noiseless scan cannot be told from paper; JPEG smooths small noise away, so JPEG pages are refused
//! more often.

#[cfg(doc)]
use crate::ScanSettings;
use crate::{Page, PageSize, Presence};

/// What the caller asks for as the page length. See the [module documentation](self).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum PageLength {
    /// Leave the extent to the driver, exactly as before this setting existed. The default.
    #[default]
    Driver,
    /// The length of [`ScanSettings::page_size`] as extents, whatever the device could detect. Deterministic.
    /// Without a page size that names a length (`Auto` and `Custom` do not) it is the driver default.
    Size,
    /// Device detection when the device publishes it; otherwise the extents of
    /// [`ScanSettings::page_size`] when one is set; otherwise the driver default. Deterministic: never
    /// inspects the image.
    Auto,
    /// [`Self::Auto`], and when none of those applies, request the item's maximum extent and infer the
    /// sheet's end from the image ([`infer_sheet_end`]). Approximate.
    AutoInfer,
}

impl PageLength {
    /// Every value, in the order a UI lists them.
    pub const ALL: &'static [PageLength] = &[PageLength::Driver, PageLength::Size, PageLength::Auto, PageLength::AutoInfer];

    /// Wire string: `driver`, `size`, `auto`, `infer`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Driver => "driver",
            Self::Size => "size",
            Self::Auto => "auto",
            Self::AutoInfer => "infer",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "driver" | "default" => Some(Self::Driver),
            "size" | "match" => Some(Self::Size),
            "auto" => Some(Self::Auto),
            "infer" | "auto-infer" | "auto_infer" => Some(Self::AutoInfer),
            _ => None,
        }
    }

    /// The request may run [`infer_sheet_end`].
    pub fn may_infer(self) -> bool {
        self == Self::AutoInfer
    }

    /// The request asks for more than the driver default.
    pub fn is_requested(self) -> bool {
        self != Self::Driver
    }
}

/// How the length of a delivered page was obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum LengthSource {
    /// Whatever the driver delivered; nothing was asked or decided.
    #[default]
    Driver,
    /// The device detected the sheet length itself.
    Device,
    /// Extents written from a page size the caller chose.
    Size,
    /// Inferred from the image: approximate.
    Inferred,
}

impl LengthSource {
    pub const ALL: &'static [LengthSource] = &[LengthSource::Driver, LengthSource::Device, LengthSource::Size, LengthSource::Inferred];

    /// Wire string: `driver`, `device`, `size`, `inferred`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Driver => "driver",
            Self::Device => "device",
            Self::Size => "size",
            Self::Inferred => "inferred",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|value| value.as_str() == text)
    }
}

/// The length of one delivered page and where it came from ([`Page::length`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct LengthRecord {
    pub source: LengthSource,
    /// The delivered length in thousandths of an inch (height over vertical resolution). `None` when it
    /// cannot be computed (no height or no resolution).
    pub length_mils: Option<u32>,
    /// The length is a guess ([`LengthSource::Inferred`]); every other source states a length the device
    /// or the caller fixed.
    pub approximate: bool,
}

impl LengthRecord {
    /// The record of a page nobody sized: [`LengthSource::Driver`], with the length it has.
    pub fn from_delivered(height: u32, dpi_y: u32) -> Self {
        Self { source: LengthSource::Driver, length_mils: mils_of(height, dpi_y), approximate: false }
    }

    /// The same page, delivered at `height` rows, with `source` as the reason.
    pub fn with_source(height: u32, dpi_y: u32, source: LengthSource) -> Self {
        Self { source, length_mils: mils_of(height, dpi_y), approximate: source == LengthSource::Inferred }
    }

    pub fn length_inches(&self) -> Option<f64> {
        self.length_mils.map(|mils| f64::from(mils) / 1000.0)
    }

    pub fn length_mm(&self) -> Option<f64> {
        self.length_mils.map(|mils| f64::from(mils) * 0.0254)
    }
}

/// Rows at a vertical resolution, in thousandths of an inch (rounded).
pub fn mils_of(rows: u32, dpi: u32) -> Option<u32> {
    if rows == 0 || dpi == 0 {
        return None;
    }
    u32::try_from((u64::from(rows) * 1000 + u64::from(dpi) / 2) / u64::from(dpi)).ok()
}

/// Rows covering `mils` at `dpi` (rounded to the nearest row, at least 1).
pub fn rows_for_mils(mils: u32, dpi: u32) -> u32 {
    u32::try_from((u64::from(mils) * u64::from(dpi) + 500) / 1000).unwrap_or(u32::MAX).max(1)
}

/// What a device publishes about the length of a page, per item (glass or feeder). Every default means
/// "unknown".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PageLengthSupport {
    /// The device can detect the sheet length itself. [`Presence::Present`] only when the property or
    /// value is published; [`Presence::Absent`] when the item's properties were read without one.
    pub device_detect: Presence,
    /// The longest page the item can scan, in thousandths of an inch, when its vertical extent is
    /// published as a range.
    pub max_length_mils: Option<u32>,
}

impl PageSize {
    /// Width and height of a named size in thousandths of an inch. `None` for [`PageSize::Auto`] and
    /// [`PageSize::Custom`], which name no dimensions.
    pub fn extent_mils(self) -> Option<(u32, u32)> {
        match self {
            Self::A4 => Some((8267, 11692)),
            Self::Letter => Some((8500, 11000)),
            Self::Legal => Some((8500, 14000)),
            Self::BusinessCard => Some((3543, 2165)),
            Self::Auto | Self::Custom => None,
        }
    }
}

// ---- inference -----------------------------------------------------------------------------------

/// Tunables of [`infer_sheet_end`]. The defaults suit lossless pages from a scanner that fills beyond
/// the sheet with pure white.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InferOptions {
    /// A pixel is *fill* when its luminance is at least `255 - tolerance`. `0` (exact `255`) for BMP, PNG
    /// and TIFF; [`Self::JPEG_TOLERANCE`] for JPEG, whose flat white can decode to 253 to 255.
    pub tolerance: u8,
    /// Added below the last non-fill row, in thousandths of an inch, because the last row of the sheet is
    /// partly fill (the edge is not row-aligned). Default 10 (0.01 inch).
    pub margin_mils: u32,
    /// Height of the band above the candidate end that is checked for paper noise, in thousandths of an
    /// inch. Default 40.
    pub band_mils: u32,
    /// Paper noise is a luminance in `noise_floor..=254 - tolerance`. Default 200: darker pixels are ink.
    pub noise_floor: u8,
    /// Share of the band's pixels (in 1/1000) that must be paper noise. Default 50 (5 %); the sheet the
    /// rule was measured on shows about 400.
    pub min_noise_permille: u16,
    /// A sheet shorter than this (thousandths of an inch) is not inferred. Default 1000.
    pub min_length_mils: u32,
}

impl InferOptions {
    /// The tolerance used for JPEG.
    pub const JPEG_TOLERANCE: u8 = 2;

    /// Options for the format of `bytes` (JPEG gets a tolerance, everything else is exact).
    pub fn for_bytes(bytes: &[u8]) -> Self {
        let tolerance = if crate::image_format(bytes) == "jpeg" { Self::JPEG_TOLERANCE } else { 0 };
        Self { tolerance, ..Self::default() }
    }
}

impl Default for InferOptions {
    fn default() -> Self {
        Self { tolerance: 0, margin_mils: 10, band_mils: 40, noise_floor: 200, min_noise_permille: 50, min_length_mils: 1000 }
    }
}

/// A sheet end found by [`infer_sheet_end`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inferred {
    /// Rows to keep (the sheet end plus the margin, never more than the image has).
    pub rows: u32,
    /// The first row that is entirely fill (the sheet end before the margin).
    pub sheet_end_row: u32,
    /// The kept length in thousandths of an inch.
    pub length_mils: u32,
    /// Share of the band that was paper noise, in 1/1000 (the evidence).
    pub noise_permille: u16,
}

/// Why no sheet end was inferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The bytes are not an image this crate decodes, or are 1-bit.
    Unreadable,
    /// Every row is fill: nothing to measure.
    AllFill,
    /// Content or noise runs to the last row: there is no padding to remove.
    NoPadding,
    /// The rows above the candidate end show no paper noise, so the end of printing cannot be told from
    /// the end of the sheet.
    NoPaperNoise,
    /// The candidate sheet is shorter than [`InferOptions::min_length_mils`].
    TooShort,
}

impl Refusal {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Unreadable => "the page is not an image that can be measured",
            Self::AllFill => "the page is entirely white fill",
            Self::NoPadding => "the image has content to its last row, so there is nothing to crop",
            Self::NoPaperNoise => "no paper noise above the candidate end, so the end of the sheet cannot be told from white margin",
            Self::TooShort => "the candidate sheet is shorter than the minimum length",
        }
    }
}

/// Find where the sheet ends in a scan whose bottom may be padded with fill. `bytes` is a delivered image
/// (BMP, PNG, TIFF or JPEG), `dpi_y` its vertical resolution. See the [module documentation](self) for the
/// rule and its limits. Pure: reads `bytes` only.
pub fn assess_sheet_end(bytes: &[u8], dpi_y: u32, options: &InferOptions) -> Result<Inferred, Refusal> {
    let Some((width, height, pixels)) = crate::bw::decode_luma(bytes) else { return Err(Refusal::Unreadable) };
    assess_luma(width, height, &pixels, dpi_y, options)
}

/// [`assess_sheet_end`] as an `Option`.
pub fn infer_sheet_end(bytes: &[u8], dpi_y: u32, options: &InferOptions) -> Option<Inferred> {
    assess_sheet_end(bytes, dpi_y, options).ok()
}

/// The inference over decoded top-down luminance.
pub fn assess_luma(width: usize, height: usize, pixels: &[u8], dpi_y: u32, options: &InferOptions) -> Result<Inferred, Refusal> {
    if width == 0 || height == 0 || pixels.len() != width.saturating_mul(height) || dpi_y == 0 {
        return Err(Refusal::Unreadable);
    }
    let fill_from = 255u8.saturating_sub(options.tolerance);
    let is_fill_row = |row: usize| pixels[row * width..(row + 1) * width].iter().all(|v| *v >= fill_from);
    let Some(last) = (0..height).rev().find(|row| !is_fill_row(*row)) else { return Err(Refusal::AllFill) };
    if last + 1 == height {
        return Err(Refusal::NoPadding);
    }
    let sheet_end_row = last + 1;
    let band_rows = (rows_for_mils(options.band_mils, dpi_y) as usize).min(sheet_end_row);
    let band = &pixels[(sheet_end_row - band_rows) * width..sheet_end_row * width];
    let ceiling = fill_from.saturating_sub(1);
    let noise = band.iter().filter(|v| **v >= options.noise_floor && **v <= ceiling).count();
    let permille = u16::try_from(noise * 1000 / band.len().max(1)).unwrap_or(1000);
    if permille < options.min_noise_permille {
        return Err(Refusal::NoPaperNoise);
    }
    let margin = rows_for_mils(options.margin_mils, dpi_y) as usize;
    let rows = (sheet_end_row + margin).min(height);
    let rows = u32::try_from(rows).map_err(|_| Refusal::Unreadable)?;
    let length_mils = mils_of(rows, dpi_y).unwrap_or(0);
    if length_mils < options.min_length_mils {
        return Err(Refusal::TooShort);
    }
    Ok(Inferred { rows, sheet_end_row: u32::try_from(sheet_end_row).unwrap_or(rows), length_mils, noise_permille: permille })
}

// ---- cropping ------------------------------------------------------------------------------------

/// The bytes of an image cut to its top `rows` rows (width untouched). A BMP is cut exactly, whatever
/// its depth and row order. PNG, TIFF and JPEG are decoded and re-encoded in the same format (JPEG at
/// quality 92 keeps its density; PNG and TIFF lose any density tag, so the page's `dpi` fields stay the
/// record). `None` when the image cannot be cut (compressed BMP, unreadable) or `rows` is not shorter
/// than the image.
pub fn crop_bottom_bytes(bytes: &[u8], rows: u32) -> Option<Vec<u8>> {
    if let Some(info) = crate::bmp::parse(bytes) {
        return crop_bmp(bytes, &info, rows);
    }
    crop_encoded(bytes, rows)
}

fn crop_bmp(bytes: &[u8], info: &crate::bmp::BmpInfo, rows: u32) -> Option<Vec<u8>> {
    if info.compression != crate::bmp::Compression::Rgb || rows == 0 || rows >= info.height {
        return None;
    }
    let (stride, data_at) = (info.stride()?, info.data_at()?);
    let pixels = info.pixels(bytes)?;
    let (keep, total) = (usize::try_from(rows).ok()?, usize::try_from(info.height).ok()?);
    // Bottom-up files store the bottom row first, so the rows to keep are the last ones.
    let first = if info.top_down { 0 } else { (total - keep) * stride };
    let mut out = Vec::new();
    out.try_reserve_exact(data_at.checked_add(keep.checked_mul(stride)?)?).ok()?;
    out.extend_from_slice(bytes.get(..data_at)?);
    out.extend_from_slice(pixels.get(first..first + keep * stride)?);
    let size = u32::try_from(out.len()).ok()?;
    out.get_mut(2..6)?.copy_from_slice(&size.to_le_bytes());
    if info.header_size == 12 {
        out.get_mut(20..22)?.copy_from_slice(&u16::try_from(rows).ok()?.to_le_bytes());
    } else {
        let signed = i32::try_from(rows).ok()?;
        let signed = if info.top_down { -signed } else { signed };
        out.get_mut(22..26)?.copy_from_slice(&signed.to_le_bytes());
        let image_size = u32::from_le_bytes(out.get(34..38)?.try_into().ok()?);
        if image_size != 0 {
            out.get_mut(34..38)?.copy_from_slice(&u32::try_from(keep.checked_mul(stride)?).ok()?.to_le_bytes());
        }
    }
    Some(out)
}

fn crop_encoded(bytes: &[u8], rows: u32) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let format = crate::image_format(bytes);
    let decoded = image::load_from_memory(bytes).ok()?;
    if rows == 0 || rows >= decoded.height() {
        return None;
    }
    let cropped = decoded.crop_imm(0, 0, decoded.width(), rows);
    let mut out = std::io::Cursor::new(Vec::new());
    match format {
        "png" => cropped.write_to(&mut out, image::ImageFormat::Png).ok()?,
        "tiff" => cropped.write_to(&mut out, image::ImageFormat::Tiff).ok()?,
        "jpeg" => {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92);
            if let Some((dx, dy)) = crate::density::image_density(bytes) {
                let (dx, dy) = (dx.round().clamp(1.0, 65535.0) as u16, dy.round().clamp(1.0, 65535.0) as u16);
                encoder
                    .set_pixel_density(image::codecs::jpeg::PixelDensity { density: (dx, dy), unit: image::codecs::jpeg::PixelDensityUnit::Inches });
            }
            let rgb = cropped.to_rgb8();
            encoder.write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8).ok()?;
        }
        _ => return None,
    }
    Some(out.into_inner())
}

/// What [`infer_and_crop`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inference {
    /// The page was cut; the page carries [`LengthSource::Inferred`].
    Cropped(Inferred),
    /// Left alone: why.
    Kept(Refusal),
    /// A sheet end was found but the image could not be cut (compressed BMP, encoder failure).
    CannotCrop(Inferred),
}

/// Run [`assess_sheet_end`] on `page` and cut it at the sheet end. On success the page's bytes, height and
/// [`Page::length`] are updated (`source` [`LengthSource::Inferred`], `approximate`). On any refusal the page
/// is untouched. The width, resolution and format never change.
pub fn infer_and_crop(page: &mut Page, options: &InferOptions) -> Inference {
    let found = match assess_sheet_end(&page.bytes, page.dpi_y, options) {
        Ok(found) => found,
        Err(refusal) => return Inference::Kept(refusal),
    };
    let Some(bytes) = crop_bottom_bytes(&page.bytes, found.rows) else { return Inference::CannotCrop(found) };
    page.bytes = bytes;
    page.height = found.rows;
    page.length = LengthRecord::with_source(found.rows, page.dpi_y, LengthSource::Inferred);
    Inference::Cropped(found)
}

#[cfg(test)]
mod tests;
