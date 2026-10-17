//! Software black-and-white for scans.
//!
//! A scanner with no native 1-bit mode (or a driver that answers a black-and-white request with
//! 8- or 24-bit data) hands back deeper pages although the user chose black and white. When the
//! request is [`Color::BlackAndWhite`], [`page_for`] turns any page deeper than 1 bit into a real 1-bit
//! page. The conversion is [`threshold_to_one_bit`], a pure function from the delivered bytes.
//!
//! Rules:
//! - Luminance is BT.601 (`0.299 R + 0.587 G + 0.114 B`, integer, rounded). A pixel whose
//!   luminance is below the threshold is ink (black); at or above it is paper (white).
//! - The output is a bottom-up 1-bpp BMP with the palette `[black, white]` (bit 0 = black), the
//!   same polarity the drivers deliver and the PDF import expects. Padding bits are white.
//! - A page that is already 1-bit is left alone: it is never re-thresholded.
//! - BMP (4, 8, 24 and 32 bits, uncompressed, either row order, OS/2 core or Windows headers; header
//!   rules in [`crate::bmp`]) is read here. JPEG, PNG, TIFF and other BMP forms (RLE, bit fields,
//!   16 bits) go through the `image` decoders, and their 1-bit PNG and TIFF files are recognised from
//!   the header so they stay as delivered. A page that cannot be read is returned unchanged.

use crate::{Color, InferOptions, LengthRecord, LengthSource, Page, ScanSettings, assess_sheet_end, bmp, density, image_format, image_size, length};

/// Used when the request carries no threshold.
pub const DEFAULT_THRESHOLD: u8 = 128;

/// What the page builder needs to know about a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRule {
    pub dpi: u32,
    /// `Some` for a black-and-white request: pages deeper than 1 bit are thresholded at this value.
    pub bw_threshold: Option<u8>,
    /// How the page's length was decided and whether to infer it from the pixels ([`LengthPlan`]).
    pub length: LengthPlan,
}

/// What the backend decided about page length before the transfer, for [`page_for`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LengthPlan {
    /// The source recorded on the page when nothing is inferred: [`LengthSource::Driver`] unless the backend
    /// wrote extents ([`LengthSource::Size`]) or the device detected the length ([`LengthSource::Device`]).
    pub source: LengthSource,
    /// `Some` when the backend asked for the item's maximum extent so the sheet end can be inferred. The
    /// inference runs on the delivered pixels **before** any 1-bit thresholding (which would remove the paper
    /// noise the rule needs); a refusal leaves the page as delivered with `source`.
    pub infer: Option<InferOptions>,
}

impl PageRule {
    pub fn for_settings(settings: &ScanSettings) -> Self {
        let bw_threshold = (settings.color == Color::BlackAndWhite).then(|| threshold_byte(settings.threshold));
        Self { dpi: settings.dpi, bw_threshold, length: LengthPlan::default() }
    }

    /// The same rule with the backend's length decision.
    pub fn with_length(self, length: LengthPlan) -> Self {
        Self { length, ..self }
    }
}

/// The request's threshold as a byte, or [`DEFAULT_THRESHOLD`] when it carries none.
pub fn threshold_byte(threshold: Option<i32>) -> u8 {
    threshold.map_or(DEFAULT_THRESHOLD, |value| u8::try_from(value.clamp(0, 255)).unwrap_or(DEFAULT_THRESHOLD))
}

/// A 1-bit page made from a deeper one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneBit {
    /// A 1-bpp BMP file.
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Bits per pixel of the page as delivered.
    pub from_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Thresholded {
    /// The page is 1-bit already and stays as it is.
    AlreadyOneBit,
    Converted(OneBit),
    /// Not an image this crate can read; the page stays as it is.
    Unreadable,
}

/// Luminance of one pixel, BT.601, rounded.
pub fn luminance(red: u8, green: u8, blue: u8) -> u8 {
    let sum = 299 * u32::from(red) + 587 * u32::from(green) + 114 * u32::from(blue);
    u8::try_from((sum + 500) / 1000).unwrap_or(255)
}

/// Top-down gray rows.
struct Luma {
    width: usize,
    height: usize,
    from_bits: u32,
    pixels: Vec<u8>,
}

enum Read {
    OneBit,
    Luma(Luma),
    Unreadable,
}

/// Turn the delivered image into a real 1-bit page when it is deeper than 1 bit.
///
/// `dpi` only fills the BMP's resolution fields.
pub fn threshold_to_one_bit(bytes: &[u8], threshold: u8, dpi: u32) -> Thresholded {
    let per_metre = u32::try_from(u64::from(dpi) * 10_000 / 254).unwrap_or(0);
    threshold_with_pels(bytes, threshold, (per_metre, per_metre))
}

/// [`threshold_to_one_bit`] with the two axes' resolution in dots per inch (rounded to whole pels per metre).
pub fn threshold_to_one_bit_at(bytes: &[u8], threshold: u8, (x, y): density::Density) -> Thresholded {
    let pels = |dpi: f64| (dpi / 0.0254).round().clamp(0.0, f64::from(u32::MAX)) as u32;
    threshold_with_pels(bytes, threshold, (pels(x), pels(y)))
}

fn threshold_with_pels(bytes: &[u8], threshold: u8, pels: (u32, u32)) -> Thresholded {
    let read = match read_bmp(bytes) {
        Some(read) => read,
        None => read_other(bytes),
    };
    let luma = match read {
        Read::OneBit => return Thresholded::AlreadyOneBit,
        Read::Unreadable => return Thresholded::Unreadable,
        Read::Luma(luma) => luma,
    };
    let (Ok(width), Ok(height)) = (u32::try_from(luma.width), u32::try_from(luma.height)) else { return Thresholded::Unreadable };
    match pack_bmp(&luma, threshold, pels) {
        Some(bytes) => Thresholded::Converted(OneBit { bytes, width, height, from_bits: luma.from_bits }),
        None => Thresholded::Unreadable,
    }
}

/// Build a delivered page. For a black-and-white rule, a page deeper than 1 bit becomes a 1-bit BMP
/// (the second value is the depth it had). Everything else keeps the bytes the driver sent.
///
/// The page's resolution is the one the delivered file carries (see [`crate::density`]); the request's dpi is only
/// the fallback for a file that carries nothing usable, and is recorded as [`Page::requested_dpi`].
pub fn page_for(delivered: &[u8], rule: PageRule) -> (Page, Option<u32>) {
    let found = density::image_density(delivered);
    let (dx, dy) = found.unwrap_or((f64::from(rule.dpi), f64::from(rule.dpi)));
    let (dpi, dpi_y) = (round_dpi(dx), round_dpi(dy));
    let mut source = rule.length.source;
    let mut cut = None;
    // A JPEG's flat white does not decode to exactly 255 everywhere, so it gets the format's tolerance.
    let infer = rule.length.infer.map(|mut options| {
        if image_format(delivered) == "jpeg" {
            options.tolerance = options.tolerance.max(InferOptions::JPEG_TOLERANCE);
        }
        options
    });
    if let Some(options) = infer
        && let Ok(sheet) = assess_sheet_end(delivered, dpi_y, &options)
        && let Some(bytes) = length::crop_bottom_bytes(delivered, sheet.rows)
    {
        cut = Some(bytes);
        source = LengthSource::Inferred;
    }
    let bytes = cut.as_deref().unwrap_or(delivered);
    if let Some(threshold) = rule.bw_threshold
        && let Thresholded::Converted(one) = threshold_to_one_bit_at(bytes, threshold, (dx, dy))
    {
        let length = LengthRecord::with_source(one.height, dpi_y, source);
        let page = Page {
            bytes: one.bytes,
            format: "bmp",
            width: one.width,
            height: one.height,
            dpi,
            dpi_y,
            requested_dpi: rule.dpi,
            thresholded: true,
            length,
        };
        return (page, Some(one.from_bits));
    }
    let (width, height) = image_size(bytes).unwrap_or((0, 0));
    let length = LengthRecord::with_source(height, dpi_y, source);
    (
        Page { format: image_format(bytes), width, height, dpi, dpi_y, requested_dpi: rule.dpi, bytes: bytes.to_vec(), thresholded: false, length },
        None,
    )
}

fn round_dpi(dpi: f64) -> u32 {
    dpi.round().clamp(1.0, f64::from(u32::MAX)) as u32
}

/// Top-down luminance of a BMP this crate reads itself: `(width, height, bits per pixel, pixels)`.
/// `None` for a 1-bit file, for forms the `image` decoders take over, and for anything unreadable.
/// Test and tooling hook; [`threshold_to_one_bit`] is the production entry.
pub fn bmp_luma(bytes: &[u8]) -> Option<(u32, u32, u32, Vec<u8>)> {
    match read_bmp(bytes)? {
        Read::Luma(luma) => Some((u32::try_from(luma.width).ok()?, u32::try_from(luma.height).ok()?, luma.from_bits, luma.pixels)),
        Read::OneBit | Read::Unreadable => None,
    }
}

/// Top-down luminance of any page this crate can decode (BMP read here, JPEG/PNG/TIFF/other BMP through the
/// `image` decoders): `(width, height, pixels)`. `None` for a 1-bit page and for anything unreadable.
pub(crate) fn decode_luma(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let read = match read_bmp(bytes) {
        Some(read) => read,
        None => read_other(bytes),
    };
    match read {
        Read::Luma(luma) => Some((luma.width, luma.height, luma.pixels)),
        Read::OneBit | Read::Unreadable => None,
    }
}

/// `None` when the file is not a BMP this reader handles (the `image` decoders then try it).
fn read_bmp(bytes: &[u8]) -> Option<Read> {
    let info = bmp::parse(bytes)?;
    if info.bits == 1 {
        return Some(Read::OneBit);
    }
    if info.compression != bmp::Compression::Rgb || !matches!(info.bits, 4 | 8 | 24 | 32) {
        return None;
    }
    let (width, height) = (usize::try_from(info.width).ok()?, usize::try_from(info.height).ok()?);
    let (stride, data) = (info.stride()?, info.pixels(bytes)?);
    let mut table = [0u8; 256];
    if info.bits <= 8 {
        for (index, slot) in table.iter_mut().enumerate().take(1 << info.bits) {
            let [red, green, blue] = info.colour(bytes, index);
            *slot = luminance(red, green, blue);
        }
    }
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(width.checked_mul(height)?).ok()?;
    for row in 0..height {
        let line = data.get(info.file_row(row)?.checked_mul(stride)?..)?.get(..stride)?;
        match info.bits {
            4 => pixels.extend((0..width).map(|x| {
                let pair = line.get(x / 2).copied().unwrap_or(0);
                table[usize::from((pair >> if x % 2 == 0 { 4 } else { 0 }) & 0x0f)]
            })),
            8 => pixels.extend(line.iter().take(width).map(|index| table[usize::from(*index)])),
            24 => pixels.extend(line.as_chunks::<3>().0.iter().take(width).map(|p| luminance(p[2], p[1], p[0]))),
            _ => pixels.extend(line.as_chunks::<4>().0.iter().take(width).map(|p| luminance(p[2], p[1], p[0]))),
        }
    }
    Some(Read::Luma(Luma { width, height, from_bits: u32::from(info.bits), pixels }))
}

fn read_other(bytes: &[u8]) -> Read {
    let one_bit = match image_format(bytes) {
        "png" => png_is_one_bit(bytes),
        "tiff" => tiff_is_one_bit(bytes),
        _ => false,
    };
    if one_bit {
        return Read::OneBit;
    }
    let Ok(image) = image::load_from_memory(bytes) else { return Read::Unreadable };
    let (Ok(width), Ok(height)) = (usize::try_from(image.width()), usize::try_from(image.height())) else { return Read::Unreadable };
    // A BMP reports the depth it was delivered at, not the decoder's output depth.
    let from_bits = bmp::parse(bytes).map_or_else(|| u32::from(image.color().bits_per_pixel()), |info| u32::from(info.bits));
    let pixels = match image {
        image::DynamicImage::ImageLuma8(gray) => gray.into_raw(),
        other => {
            let rgb = other.into_rgb8();
            let mut out = Vec::new();
            if out.try_reserve_exact(width.saturating_mul(height)).is_err() {
                return Read::Unreadable;
            }
            out.extend(rgb.as_raw().as_chunks::<3>().0.iter().map(|p| luminance(p[0], p[1], p[2])));
            out
        }
    };
    if width == 0 || height == 0 || pixels.len() != width.saturating_mul(height) {
        return Read::Unreadable;
    }
    Read::Luma(Luma { width, height, from_bits, pixels })
}

/// IHDR bit depth 1 with a gray or palette colour type.
fn png_is_one_bit(bytes: &[u8]) -> bool {
    bytes.get(..8) == Some(b"\x89PNG\r\n\x1a\n".as_slice())
        && bytes.get(12..16) == Some(b"IHDR".as_slice())
        && bytes.get(24).copied() == Some(1)
        && matches!(bytes.get(25).copied(), Some(0 | 3))
}

/// First IFD says one bit per sample and one sample per pixel (BitsPerSample defaults to 1).
fn tiff_is_one_bit(bytes: &[u8]) -> bool {
    let little = match bytes.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return false,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let raw: [u8; 2] = bytes.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if little { u16::from_le_bytes(raw) } else { u16::from_be_bytes(raw) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let raw: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(raw) } else { u32::from_be_bytes(raw) })
    };
    let one_bit = || -> Option<bool> {
        if u16_at(2)? != 42 {
            return None;
        }
        let ifd = usize::try_from(u32_at(4)?).ok()?;
        let entries = usize::from(u16_at(ifd)?);
        let (mut bits, mut samples) = (1u16, 1u16);
        for index in 0..entries {
            let entry = ifd.checked_add(2)?.checked_add(index.checked_mul(12)?)?;
            let (tag, count) = (u16_at(entry)?, u32_at(entry.checked_add(4)?)?);
            let value_at = entry.checked_add(8)?;
            match tag {
                258 => {
                    // One value sits in the entry; several are stored elsewhere (all alike here).
                    bits = if count <= 1 { u16_at(value_at)? } else { u16_at(usize::try_from(u32_at(value_at)?).ok()?)? };
                }
                277 => samples = u16_at(value_at)?,
                _ => {}
            }
        }
        Some(bits == 1 && samples == 1)
    };
    one_bit().unwrap_or(false)
}

/// A bottom-up 1-bpp BMP, palette `[black, white]`: luminance below `threshold` is ink.
fn pack_bmp(luma: &Luma, threshold: u8, (pels_x, pels_y): (u32, u32)) -> Option<Vec<u8>> {
    const PIXELS_AT: usize = 14 + 40 + 8;
    let stride = luma.width.div_ceil(32).checked_mul(4)?;
    let size = stride.checked_mul(luma.height)?;
    let total = size.checked_add(PIXELS_AT)?;
    let mut out = Vec::new();
    out.try_reserve_exact(total).ok()?;
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&u32::try_from(total).ok()?.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&u32::try_from(PIXELS_AT).ok()?.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&i32::try_from(luma.width).ok()?.to_le_bytes());
    out.extend_from_slice(&i32::try_from(luma.height).ok()?.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // BI_RGB
    out.extend_from_slice(&u32::try_from(size).ok()?.to_le_bytes());
    out.extend_from_slice(&pels_x.to_le_bytes());
    out.extend_from_slice(&pels_y.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]);
    for row in (0..luma.height).rev() {
        let line = luma.pixels.get(row * luma.width..)?.get(..luma.width)?;
        let start = out.len();
        out.resize(start + stride, 0xff);
        for (x, value) in line.iter().enumerate() {
            if *value < threshold
                && let Some(byte) = out.get_mut(start + x / 8)
            {
                *byte &= !(0x80 >> (x % 8));
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uncompressed BMP. `rows` lists the pixel rows top to bottom, each already packed (no padding).
    fn bmp(width: u32, bits: u16, palette: &[[u8; 3]], top_down: bool, rows: &[Vec<u8>]) -> Vec<u8> {
        let stride = (width as usize * bits as usize).div_ceil(32) * 4;
        let mut data = Vec::new();
        let ordered: Vec<&Vec<u8>> = if top_down { rows.iter().collect() } else { rows.iter().rev().collect() };
        for row in ordered {
            let mut padded = row.clone();
            padded.resize(stride, 0);
            data.extend(padded);
        }
        let pixel_at = 54 + palette.len() * 4;
        let height = rows.len() as i32;
        let mut out = b"BM".to_vec();
        out.extend(((pixel_at + data.len()) as u32).to_le_bytes());
        out.extend([0; 4]);
        out.extend((pixel_at as u32).to_le_bytes());
        out.extend(40u32.to_le_bytes());
        out.extend((width as i32).to_le_bytes());
        out.extend((if top_down { -height } else { height }).to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(bits.to_le_bytes());
        out.extend([0; 4]);
        out.extend((data.len() as u32).to_le_bytes());
        out.extend([0; 8]);
        out.extend((palette.len() as u32).to_le_bytes());
        out.extend([0; 4]);
        for [r, g, b] in palette {
            out.extend([*b, *g, *r, 0]);
        }
        out.extend(data);
        out
    }

    /// Black pixels (`true`) of a converted page, top to bottom.
    fn ink(one: &OneBit) -> Vec<Vec<bool>> {
        let b = &one.bytes;
        assert_eq!(&b[..2], b"BM");
        assert_eq!(u16::from_le_bytes([b[28], b[29]]), 1, "1 bpp");
        assert_eq!(&b[54..62], &[0, 0, 0, 0, 255, 255, 255, 0], "palette [black, white]");
        let (w, h) = (one.width as usize, one.height as usize);
        let stride = w.div_ceil(32) * 4;
        let at = u32::from_le_bytes([b[10], b[11], b[12], b[13]]) as usize;
        assert_eq!(b.len(), at + stride * h);
        let signed = i32::from_le_bytes([b[22], b[23], b[24], b[25]]);
        assert_eq!(signed, h as i32, "bottom-up");
        (0..h)
            .map(|top| {
                let row = h - 1 - top;
                (0..w).map(|x| b[at + row * stride + x / 8] & (0x80 >> (x % 8)) == 0).collect()
            })
            .collect()
    }

    fn converted(bytes: &[u8], threshold: u8) -> OneBit {
        match threshold_to_one_bit(bytes, threshold, 300) {
            Thresholded::Converted(one) => one,
            other => panic!("expected a conversion, got {other:?}"),
        }
    }

    fn rgb_rows(values: &[&[u8]]) -> Vec<Vec<u8>> {
        values.iter().map(|row| row.iter().flat_map(|v| [*v, *v, *v]).collect()).collect()
    }

    #[test]
    fn a_24_bit_bmp_becomes_one_bit_in_both_row_orders() {
        for top_down in [false, true] {
            // 13 wide: neither a multiple of 8 nor of 32, so rows have padding bits and bytes.
            let dark: Vec<u8> = (0..13).map(|x| if (3..=6).contains(&x) { 10 } else { 240 }).collect();
            let light = vec![240u8; 13];
            let rows = rgb_rows(&[&dark, &dark, &light, &light, &light]);
            let one = converted(&bmp(13, 24, &[], top_down, &rows), 128);
            assert_eq!((one.width, one.height, one.from_bits), (13, 5, 24));
            let grid = ink(&one);
            for (y, row) in grid.iter().enumerate() {
                for (x, black) in row.iter().enumerate() {
                    assert_eq!(*black, y < 2 && (3..=6).contains(&x), "top_down={top_down} x={x} y={y}");
                }
            }
            // Bits past the width in the last byte are paper.
            let stride = 4;
            let at = 62;
            assert_eq!(one.bytes[at + 1] & 0b0000_0111, 0b0000_0111);
            assert_eq!(one.bytes.len(), at + stride * 5);
        }
    }

    #[test]
    fn luminance_is_bt601() {
        assert_eq!(luminance(0, 0, 0), 0);
        assert_eq!(luminance(255, 255, 255), 255);
        assert_eq!(luminance(255, 0, 0), 76);
        assert_eq!(luminance(0, 255, 0), 150);
        assert_eq!(luminance(0, 0, 255), 29);
        assert_eq!(luminance(77, 77, 77), 77);
    }

    #[test]
    fn colour_pixels_use_luminance_not_a_single_channel() {
        // Pure red is luminance 76: dark at 128. Pure green is 150: light at 128.
        let row = vec![0, 0, 255, 0, 255, 0];
        let one = converted(&bmp(2, 24, &[], true, &[row]), 128);
        assert_eq!(ink(&one), vec![vec![true, false]]);
    }

    #[test]
    fn the_threshold_edge_is_paper() {
        let rows = vec![vec![99u8, 100, 101, 127, 128, 129]];
        let palette: Vec<[u8; 3]> = (0..=255u8).map(|v| [v, v, v]).collect();
        let bytes = bmp(6, 8, &palette, true, &rows);
        assert_eq!(ink(&converted(&bytes, 100))[0], vec![true, false, false, false, false, false]);
        assert_eq!(ink(&converted(&bytes, 128))[0], vec![true, true, true, true, false, false]);
        assert_eq!(ink(&converted(&bytes, 0))[0], vec![false; 6]);
        assert_eq!(ink(&converted(&bytes, 255))[0], vec![true; 6]);
    }

    #[test]
    fn an_8_bit_gray_bmp_in_both_row_orders() {
        let palette: Vec<[u8; 3]> = (0..=255u8).map(|v| [v, v, v]).collect();
        let rows = vec![vec![0u8, 255, 0, 255, 0], vec![255u8, 0, 255, 0, 255], vec![0u8, 0, 255, 255, 0]];
        for top_down in [false, true] {
            let one = converted(&bmp(5, 8, &palette, top_down, &rows), 128);
            assert_eq!(one.from_bits, 8);
            let expected: Vec<Vec<bool>> = rows.iter().map(|r| r.iter().map(|v| *v == 0).collect()).collect();
            assert_eq!(ink(&one), expected, "top_down={top_down}");
        }
    }

    #[test]
    fn an_8_bit_palette_bmp_follows_the_palette_not_the_index() {
        // Index 0 is white and index 1 is black: the opposite of the gray ramp.
        let palette = [[255, 255, 255], [0, 0, 0], [200, 30, 30]];
        let rows = vec![vec![0u8, 1, 2, 1, 0, 1, 0, 0, 1]];
        let one = converted(&bmp(9, 8, &palette, false, &rows), 128);
        // Index 2 is (200,30,30): luminance 85, dark.
        assert_eq!(ink(&one), vec![vec![false, true, true, true, false, true, false, false, true]]);
    }

    #[test]
    fn a_4_bit_bmp_reads_the_high_nibble_first() {
        let mut palette = vec![[255, 255, 255]; 16];
        palette[1] = [0, 0, 0];
        palette[9] = [90, 90, 90];
        // Pixels 0,1,9,1,0 packed in nibbles: 0x01 0x91 0x00.
        let one = converted(&bmp(5, 4, &palette, true, &[vec![0x01, 0x91, 0x00]]), 128);
        assert_eq!(one.from_bits, 4);
        assert_eq!(ink(&one), vec![vec![false, true, true, true, false]]);
    }

    #[test]
    fn a_32_bit_bmp_ignores_the_fourth_byte() {
        let row = vec![0, 0, 0, 255, 255, 255, 255, 0, 200, 200, 200, 7];
        let one = converted(&bmp(3, 32, &[], true, &[row]), 128);
        assert_eq!(one.from_bits, 32);
        assert_eq!(ink(&one), vec![vec![true, false, false]]);
    }

    #[test]
    fn widths_around_the_row_boundaries_keep_every_pixel() {
        for width in [1usize, 7, 8, 9, 31, 32, 33, 63, 64, 65] {
            let row: Vec<u8> = (0..width).map(|x| if x % 3 == 0 { 0 } else { 255 }).collect();
            let rows = rgb_rows(&[&row, &row]);
            let one = converted(&bmp(width as u32, 24, &[], false, &rows), 128);
            let grid = ink(&one);
            for line in &grid {
                for (x, black) in line.iter().enumerate() {
                    assert_eq!(*black, x % 3 == 0, "width {width} x {x}");
                }
            }
        }
    }

    #[test]
    fn an_already_one_bit_bmp_passes_through_in_either_palette_order() {
        for palette in [[[0, 0, 0], [255, 255, 255]], [[255, 255, 255], [0, 0, 0]]] {
            let bytes = bmp(13, 1, &palette, false, &[vec![0b1010_1010, 0b1010_0000]]);
            assert_eq!(threshold_to_one_bit(&bytes, 128, 300), Thresholded::AlreadyOneBit);
            let (page, from) = page_for(&bytes, PageRule { dpi: 300, bw_threshold: Some(128), length: LengthPlan::default() });
            assert_eq!(page.bytes, bytes);
            assert!(!page.thresholded);
            assert_eq!((page.format, page.width, page.height, from), ("bmp", 13, 1, None));
        }
    }

    #[test]
    fn one_bit_png_and_tiff_headers_pass_through() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend([0, 0, 0, 8, 0, 0, 0, 8, 1, 0, 0, 0, 0]);
        assert_eq!(threshold_to_one_bit(&png, 128, 300), Thresholded::AlreadyOneBit);
        png[24] = 8;
        assert_eq!(threshold_to_one_bit(&png, 128, 300), Thresholded::Unreadable);

        for little in [true, false] {
            let put16 = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
            let put32 = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };
            let tiff = |bits: u16| {
                let mut t = if little { b"II".to_vec() } else { b"MM".to_vec() };
                t.extend(put16(42));
                t.extend(put32(8));
                t.extend(put16(2));
                for (tag, value) in [(258u16, bits), (277, 1)] {
                    t.extend(put16(tag));
                    t.extend(put16(3));
                    t.extend(put32(1));
                    t.extend(put16(value));
                    t.extend([0, 0]);
                }
                t.extend(put32(0));
                t
            };
            assert_eq!(threshold_to_one_bit(&tiff(1), 128, 300), Thresholded::AlreadyOneBit);
            assert!(!tiff_is_one_bit(&tiff(8)));
        }
    }

    #[test]
    fn jpeg_png_and_tiff_deliveries_are_decoded_and_thresholded() {
        use image::{ImageEncoder, codecs::png::PngEncoder};
        // 8-bit gray PNG: left half black, right half white.
        let gray: Vec<u8> = (0..4).flat_map(|_| [0u8, 0, 255, 255]).collect();
        let mut png = Vec::new();
        PngEncoder::new(&mut png).write_image(&gray, 4, 4, image::ExtendedColorType::L8).expect("encode png");
        let one = converted(&png, 128);
        assert_eq!((one.width, one.height, one.from_bits), (4, 4, 8));
        assert_eq!(ink(&one)[0], vec![true, true, false, false]);

        // 24-bit RGB TIFF.
        let rgb: Vec<u8> = (0..4).flat_map(|_| [0u8, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255, 255]).collect();
        let mut tiff = std::io::Cursor::new(Vec::new());
        image::codecs::tiff::TiffEncoder::new(&mut tiff).write_image(&rgb, 4, 4, image::ExtendedColorType::Rgb8).expect("encode tiff");
        let one = converted(tiff.get_ref(), 128);
        assert_eq!(one.from_bits, 24);
        assert_eq!(ink(&one)[3], vec![true, true, false, false]);

        // A JPEG is never 1-bit; flat areas survive the codec.
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
            .write_image(&rgb, 4, 4, image::ExtendedColorType::Rgb8)
            .expect("encode jpeg");
        let one = converted(&jpeg, 128);
        assert_eq!(one.from_bits, 24);
        assert_eq!(ink(&one)[0].len(), 4);
    }

    #[test]
    fn truncated_or_unknown_input_is_left_alone() {
        let palette: Vec<[u8; 3]> = (0..=255u8).map(|v| [v, v, v]).collect();
        let mut bytes = bmp(5, 8, &palette, true, &[vec![0; 5], vec![0; 5]]);
        bytes.truncate(bytes.len() - 6);
        assert_eq!(threshold_to_one_bit(&bytes, 128, 300), Thresholded::Unreadable);
        assert_eq!(threshold_to_one_bit(b"not an image", 128, 300), Thresholded::Unreadable);
        assert_eq!(threshold_to_one_bit(b"", 128, 300), Thresholded::Unreadable);
        let (page, from) = page_for(&bytes, PageRule { dpi: 100, bw_threshold: Some(128), length: LengthPlan::default() });
        assert!(!page.thresholded && from.is_none());
        assert_eq!(page.bytes, bytes);
    }

    #[test]
    fn page_for_converts_only_a_black_and_white_request() {
        let rows = rgb_rows(&[&[0, 255, 0]]);
        let bytes = bmp(3, 24, &[], true, &rows);
        let (gray, from) = page_for(&bytes, PageRule { dpi: 150, bw_threshold: None, length: LengthPlan::default() });
        assert!(!gray.thresholded && from.is_none());
        assert_eq!((gray.bytes.as_slice(), gray.width, gray.height), (bytes.as_slice(), 3, 1));

        let (bw, from) = page_for(&bytes, PageRule { dpi: 150, bw_threshold: Some(128), length: LengthPlan::default() });
        assert!(bw.thresholded);
        assert_eq!(from, Some(24));
        assert_eq!((bw.format, bw.width, bw.height, bw.dpi), ("bmp", 3, 1, 150));
        assert_eq!(crate::bmp_size(&bw.bytes), Some((3, 1)));
        // The file carried no density, so the request's 150 dpi is used: 5906 pixels per metre (rounded).
        assert_eq!(u32::from_le_bytes([bw.bytes[38], bw.bytes[39], bw.bytes[40], bw.bytes[41]]), 5906);
        assert_eq!((bw.dpi, bw.dpi_y, bw.requested_dpi), (150, 150, 150));
    }

    #[test]
    fn the_request_threshold_maps_to_a_byte() {
        assert_eq!(threshold_byte(None), 128);
        assert_eq!(threshold_byte(Some(0)), 0);
        assert_eq!(threshold_byte(Some(200)), 200);
        assert_eq!(threshold_byte(Some(-5)), 0);
        assert_eq!(threshold_byte(Some(9999)), 255);
        let colour = ScanSettings { color: Color::Rgb, threshold: Some(10), ..ScanSettings::default() };
        assert_eq!(PageRule::for_settings(&colour).bw_threshold, None);
        let gray = ScanSettings { color: Color::Gray, ..ScanSettings::default() };
        assert_eq!(PageRule::for_settings(&gray).bw_threshold, None);
        let bw = ScanSettings { color: Color::BlackAndWhite, threshold: Some(77), dpi: 300, ..ScanSettings::default() };
        assert_eq!(PageRule::for_settings(&bw), PageRule { dpi: 300, bw_threshold: Some(77), length: LengthPlan::default() });
    }

    #[test]
    fn the_source_depth_of_a_bmp_the_image_decoders_take_is_the_files_own() {
        // RLE8 and 16-bit files are decoded by the `image` crate; `from_bits` is still what the file said.
        let mut rle = bmp(4, 8, &[[0, 0, 0], [255, 255, 255]], false, &[vec![0; 4]]);
        rle[30..34].copy_from_slice(&1u32.to_le_bytes());
        rle.truncate(62);
        rle.extend_from_slice(&[4, 1, 0, 1]);
        assert_eq!(converted(&rle, 128).from_bits, 8);
        let sixteen = bmp(2, 16, &[], false, &[vec![0xff; 4]]);
        assert_eq!(converted(&sixteen, 128).from_bits, 16);
    }

    #[test]
    fn a_one_bit_os2_bmp_is_left_alone_and_an_os2_8_bit_one_is_read() {
        let mut core = b"BM".to_vec();
        core.extend_from_slice(&[0; 12]);
        core.extend_from_slice(&12u32.to_le_bytes());
        core.extend_from_slice(&[3, 0, 1, 0, 1, 0, 8, 0]); // 3 x 1, one plane, 8 bits
        for i in 0..=255u8 {
            core.extend_from_slice(&[i, i, i]);
        }
        let at = u32::try_from(core.len()).unwrap_or(0);
        core[10..14].copy_from_slice(&at.to_le_bytes());
        core.extend_from_slice(&[10, 200, 130, 0]);
        let one = converted(&core, 128);
        assert_eq!((one.width, one.height, one.from_bits), (3, 1, 8));
        assert_eq!(one.bytes[62], 0x7f, "ink, paper, paper, then white padding");
        core[24..26].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(threshold_to_one_bit(&core, 128, 200), Thresholded::AlreadyOneBit);
    }

    #[test]
    fn bf_off_bits_zero_follows_the_palette_and_odd_header_sizes_are_not_bmp_pixels() {
        let palette = [[0, 0, 0], [255, 255, 255]];
        let mut zero = bmp(4, 8, &palette, false, &[vec![0, 1, 1, 0]]);
        let want = converted(&zero, 128).bytes;
        zero[10..14].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(converted(&zero, 128).bytes, want);
        let mut huge = bmp(4, 24, &[], false, &[vec![0; 12]]);
        huge[14..18].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert_eq!(threshold_to_one_bit(&huge, 128, 200), Thresholded::Unreadable);
    }
}
