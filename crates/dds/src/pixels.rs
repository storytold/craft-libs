//! Turning GPU textures' stored pixels into RGBA: uncompressed layouts, packed 32-bit layouts,
//! and 4×4 block formats (BC1–BC7, ETC1, and EAC). Rows come out in the order they're stored.

use crate::{DdsError, alloc_zeroed, bc, bptc, eac, packed, with_alpha};

/// Default RGBA output budget: 256 MiB, matching an 8192×8192 RGBA8 texture.
pub const DEFAULT_MAX_RGBA_BYTES: u64 = 256 << 20;

/// How a texture's pixels are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelLayout {
    /// Alpha only (shown as white with that alpha).
    Alpha8,
    /// One grey channel.
    R8,
    /// One unsigned 16-bit channel, shown as grey.
    R16,
    /// Two unsigned 8-bit channels; blue is zero and alpha is opaque.
    Rg8,
    /// Two unsigned 16-bit channels; blue is zero and alpha is opaque.
    Rg16,
    /// Three unsigned 16-bit channels; alpha is opaque.
    Rgb16,
    /// Four unsigned 16-bit channels, including alpha.
    Rgba16,
    RHalf,
    RgHalf,
    RgbaHalf,
    RFloat,
    RgFloat,
    RgbaFloat,
    /// Nine-bit RGB mantissas with a shared five-bit exponent, displayed as opaque sRGB.
    Rgb9e5,
    /// Packed unsigned 10-bit RGB with two-bit alpha, low bits first.
    Rgba1010102,
    /// Packed unsigned 11/11/10-bit floating-point RGB, displayed as opaque sRGB.
    Rgb111110Float,
    Rgb24,
    Rgba32,
    Argb32,
    Bgra32,
    Rgb565,
    Rgba4444,
    Argb4444,
    /// BC1 / DXT1.
    Dxt1,
    /// BC2 / DXT3: four-colour blocks with one explicit four-bit alpha value per texel.
    Dxt3,
    /// BC3 / DXT5.
    Dxt5,
    /// One channel in blocks (shown as grey).
    Bc4,
    /// Two channels in blocks, usually a normal map (blue is rebuilt from red and green).
    Bc5,
    /// High dynamic range RGB in blocks (unsigned half floats), tone-mapped to 8-bit sRGB.
    Bc6h,
    /// BC6H with signed half floats.
    Bc6hSigned,
    Bc7,
    /// ETC1 RGB, with individual or differential base colours.
    Etc1,
    /// One EAC channel, shown as grey.
    EacR,
    /// One signed EAC channel; -1..1 maps to 0..255.
    EacRSigned,
    /// Two EAC channels; blue is zero and alpha is opaque.
    EacRg,
    /// Two signed EAC channels; -1..1 maps to 0..255.
    EacRgSigned,
}

/// Bytes of a `width`×`height` picture in `layout`.
pub fn size(layout: PixelLayout, width: u32, height: u32) -> Option<usize> {
    let (w, h) = (width as usize, height as usize);
    let blocks = w.div_ceil(4).checked_mul(h.div_ceil(4))?;
    match layout {
        PixelLayout::Alpha8 | PixelLayout::R8 => w.checked_mul(h),
        PixelLayout::Rgb565 | PixelLayout::Rgba4444 | PixelLayout::Argb4444 | PixelLayout::RHalf | PixelLayout::R16 | PixelLayout::Rg8 => {
            w.checked_mul(h)?.checked_mul(2)
        }
        PixelLayout::Rgb24 => w.checked_mul(h)?.checked_mul(3),
        PixelLayout::Rg16 => w.checked_mul(h)?.checked_mul(4),
        PixelLayout::Rgb16 => w.checked_mul(h)?.checked_mul(6),
        PixelLayout::Rgba16 => w.checked_mul(h)?.checked_mul(8),
        PixelLayout::Rgba32
        | PixelLayout::Argb32
        | PixelLayout::Bgra32
        | PixelLayout::RgHalf
        | PixelLayout::RFloat
        | PixelLayout::Rgb9e5
        | PixelLayout::Rgba1010102
        | PixelLayout::Rgb111110Float => w.checked_mul(h)?.checked_mul(4),
        PixelLayout::RgbaHalf | PixelLayout::RgFloat => w.checked_mul(h)?.checked_mul(8),
        PixelLayout::RgbaFloat => w.checked_mul(h)?.checked_mul(16),
        PixelLayout::Dxt1 | PixelLayout::Bc4 | PixelLayout::Etc1 | PixelLayout::EacR | PixelLayout::EacRSigned => blocks.checked_mul(8),
        PixelLayout::Dxt3
        | PixelLayout::Dxt5
        | PixelLayout::Bc5
        | PixelLayout::Bc6h
        | PixelLayout::Bc6hSigned
        | PixelLayout::Bc7
        | PixelLayout::EacRg
        | PixelLayout::EacRgSigned => blocks.checked_mul(16),
    }
}

/// Convert an IEEE 754 binary16 half-float's bits to `f32`.
pub fn half_to_f32(h: u16) -> f32 {
    bptc::half(h)
}

/// Convert a linear-light value to 8-bit sRGB; 0..1 is kept, brighter and negative values clip.
pub fn linear_to_srgb8(v: f32) -> u8 {
    bptc::srgb8(v)
}

/// Decode one BC1 (DXT1) 8-byte block into 16 RGBA8 texels (row by row).
pub fn decode_bc1_block(block: &[u8; 8]) -> [[u8; 4]; 16] {
    bc::decode_colour(block, true)
}

/// Decode one BC2 (DXT3) 16-byte block into 16 RGBA8 texels (row by row).
pub fn decode_bc2_block(block: &[u8; 16]) -> [[u8; 4]; 16] {
    with_alpha(bc::decode_colour(block.get(8..).unwrap_or_default(), false), bc::decode_bc2_alpha(block.get(..8).unwrap_or_default()))
}

/// Decode one BC3 (DXT5) 16-byte block into 16 RGBA8 texels (row by row).
pub fn decode_bc3_block(block: &[u8; 16]) -> [[u8; 4]; 16] {
    with_alpha(bc::decode_colour(block.get(8..).unwrap_or_default(), false), bc::decode_values(block.get(..8).unwrap_or_default()))
}

/// Decode one BC4 8-byte block into 16 greyscale RGBA8 texels (`[r, r, r, 255]`, row by row).
pub fn decode_bc4_block(block: &[u8; 8]) -> [[u8; 4]; 16] {
    bc::decode_values(block).map(|r| [r, r, r, 255])
}

/// Decode one BC5 16-byte normal-map block into 16 RGBA8 texels with reconstructed Z (row by row).
pub fn decode_bc5_block(block: &[u8; 16]) -> [[u8; 4]; 16] {
    let (r, g) = (bc::decode_values(block.get(..8).unwrap_or_default()), bc::decode_values(block.get(8..).unwrap_or_default()));
    std::array::from_fn(|k| {
        let (x, y) = (f32::from(r[k]) / 127.5 - 1.0, f32::from(g[k]) / 127.5 - 1.0);
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        [r[k], g[k], ((z + 1.0) * 127.5) as u8, 255]
    })
}

/// Decode one BC6H 16-byte block into 16 sRGB8 RGBA texels (opaque).
pub fn decode_bc6h_block(block: &[u8; 16], signed: bool) -> [[u8; 4]; 16] {
    bptc::bc6h(block, signed)
}

/// Decode one BC7 16-byte block into 16 RGBA8 texels (row by row).
pub fn decode_bc7_block(block: &[u8; 16]) -> [[u8; 4]; 16] {
    bptc::bc7(block)
}

/// Decode one ETC1 8-byte block into 16 RGBA8 texels; returns `None` if differential colours overflow 5 bits.
pub fn decode_etc1_block(block: &[u8; 8]) -> Option<[[u8; 4]; 16]> {
    eac::etc1(block)
}

/// Decode one single-channel EAC 8-byte block into 16 greyscale RGBA8 texels.
pub fn decode_eac_r_block(block: &[u8; 8], signed: bool) -> [[u8; 4]; 16] {
    eac::scalar(block, signed)
}

/// Decode one dual-channel EAC 16-byte block into 16 RGBA8 texels (blue 0, alpha 255).
pub fn decode_eac_rg_block(block: &[u8; 16], signed: bool) -> [[u8; 4]; 16] {
    eac::dual(block, signed)
}

/// Decode one little-endian `RGB9E5` 4-byte pixel into opaque sRGB8 RGBA.
pub fn decode_rgb9e5(bytes: &[u8; 4]) -> [u8; 4] {
    packed::rgb9e5(bytes)
}

/// Decode one little-endian `RGBA1010102` 4-byte pixel into RGBA8.
pub fn decode_rgba1010102(bytes: &[u8; 4]) -> [u8; 4] {
    packed::rgba1010102(bytes)
}

/// Decode one little-endian `R11G11B10_FLOAT` 4-byte pixel into opaque sRGB8 RGBA.
pub fn decode_rgb111110_float(bytes: &[u8; 4]) -> [u8; 4] {
    packed::rgb111110(bytes)
}

fn unorm8(value: u16) -> u8 {
    ((u32::from(value) * 255 + 32767) / 65535) as u8
}

fn unorm16_pixels(samples: impl Iterator<Item = u16>, channels: usize) -> impl Iterator<Item = [u8; 4]> {
    let mut samples = samples.map(unorm8);
    std::iter::from_fn(move || {
        let (r, g) = (samples.next()?, samples.next()?);
        let b = if channels > 2 { samples.next()? } else { 0 };
        let a = if channels > 3 { samples.next()? } else { 255 };
        Some([r, g, b, a])
    })
}

fn float_pixels(mut samples: impl Iterator<Item = f32>, channels: usize) -> impl Iterator<Item = [u8; 4]> {
    std::iter::from_fn(move || {
        let r = bptc::srgb8(samples.next()?);
        if channels == 1 {
            return Some([r, r, r, 255]);
        }
        let g = bptc::srgb8(samples.next()?);
        if channels == 2 {
            return Some([r, g, 0, 255]);
        }
        let b = bptc::srgb8(samples.next()?);
        let a = (samples.next()?.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some([r, g, b, a])
    })
}

fn expand5(v: u16) -> u8 {
    let v = (v & 31) as u8;
    (v << 3) | (v >> 2)
}

fn expand6(v: u16) -> u8 {
    let v = (v & 63) as u8;
    (v << 2) | (v >> 4)
}

fn rgb565(v: u16) -> [u8; 4] {
    [expand5(v >> 11), expand6(v >> 5), expand5(v), 255]
}

fn nibbles(v: u16, order: [u32; 4]) -> [u8; 4] {
    order.map(|shift| ((v >> shift) & 15) as u8 * 17)
}

fn place_blocks(out: &mut [u8], w: usize, h: usize, texels: impl Iterator<Item = [[u8; 4]; 16]>) {
    let across = w.div_ceil(4);
    for (i, block) in texels.enumerate() {
        let (bx, by) = (i % across, i / across);
        for (k, p) in block.iter().enumerate() {
            let (x, y) = (bx * 4 + k % 4, by * 4 + k / 4);
            if x < w && y < h {
                let at = (y * w + x) * 4;
                if let Some(dst) = out.get_mut(at..at + 4) {
                    dst.copy_from_slice(p);
                }
            }
        }
    }
}

/// Decode a `width`×`height` picture (`data` starts with it) into RGBA8, rows in stored order.
///
/// Caps the output at [`DEFAULT_MAX_RGBA_BYTES`]; use [`decode_with_limit`] for a different budget.
pub fn decode(layout: PixelLayout, width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>, DdsError> {
    decode_with_limit(layout, width, height, data, DEFAULT_MAX_RGBA_BYTES)
}

/// Decode into RGBA8, allowing at most `max_bytes` of output.
///
/// The budget applies to RGBA bytes, not the encoded input, and is checked before reading pixels
/// or reserving the output.
pub fn decode_with_limit(layout: PixelLayout, width: u32, height: u32, data: &[u8], max_bytes: u64) -> Result<Vec<u8>, DdsError> {
    let need = size(layout, width, height).ok_or(DdsError::Alloc(usize::MAX))?;
    let (w, h) = (width as usize, height as usize);
    let out_len =
        w.checked_mul(h).and_then(|pixels| pixels.checked_mul(4)).filter(|bytes| *bytes <= isize::MAX as usize).ok_or(DdsError::Alloc(usize::MAX))?;
    if out_len as u64 > max_bytes {
        return Err(DdsError::LimitExceeded { required: out_len, limit: max_bytes });
    }
    let data = data.get(..need).ok_or_else(|| DdsError::Truncated(format!("{} bytes of pixels, {need} needed", data.len())))?;
    if w == 0 || h == 0 {
        return Err(DdsError::Malformed("an empty picture".into()));
    }
    let mut out: Vec<u8> = alloc_zeroed(out_len)?;
    let mut put = |px: &mut dyn Iterator<Item = [u8; 4]>| {
        for (dst, p) in out.as_chunks_mut::<4>().0.iter_mut().zip(px) {
            *dst = p;
        }
    };
    let u16s = data.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b));
    let floats = data.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b));
    match layout {
        PixelLayout::RHalf => put(&mut float_pixels(u16s.map(bptc::half), 1)),
        PixelLayout::RgHalf => put(&mut float_pixels(u16s.map(bptc::half), 2)),
        PixelLayout::RgbaHalf => put(&mut float_pixels(u16s.map(bptc::half), 4)),
        PixelLayout::RFloat => put(&mut float_pixels(floats, 1)),
        PixelLayout::RgFloat => put(&mut float_pixels(floats, 2)),
        PixelLayout::RgbaFloat => put(&mut float_pixels(floats, 4)),
        PixelLayout::Rgba1010102 => put(&mut data.as_chunks::<4>().0.iter().map(packed::rgba1010102)),
        PixelLayout::Rgb111110Float => put(&mut data.as_chunks::<4>().0.iter().map(packed::rgb111110)),
        PixelLayout::Rgb9e5 => put(&mut data.as_chunks::<4>().0.iter().map(packed::rgb9e5)),
        PixelLayout::Alpha8 => put(&mut data.iter().map(|a| [255, 255, 255, *a])),
        PixelLayout::R8 => put(&mut data.iter().map(|r| [*r, *r, *r, 255])),
        PixelLayout::R16 => put(&mut u16s.map(|value| {
            let grey = unorm8(value);
            [grey, grey, grey, 255]
        })),
        PixelLayout::Rg8 => put(&mut data.as_chunks::<2>().0.iter().map(|[r, g]| [*r, *g, 0, 255])),
        PixelLayout::Rg16 => put(&mut unorm16_pixels(u16s, 2)),
        PixelLayout::Rgb16 => put(&mut unorm16_pixels(u16s, 3)),
        PixelLayout::Rgba16 => put(&mut unorm16_pixels(u16s, 4)),
        PixelLayout::Rgb24 => put(&mut data.as_chunks::<3>().0.iter().map(|[r, g, b]| [*r, *g, *b, 255])),
        PixelLayout::Rgba32 => put(&mut data.as_chunks::<4>().0.iter().copied()),
        PixelLayout::Argb32 => put(&mut data.as_chunks::<4>().0.iter().map(|[a, r, g, b]| [*r, *g, *b, *a])),
        PixelLayout::Bgra32 => put(&mut data.as_chunks::<4>().0.iter().map(|[b, g, r, a]| [*r, *g, *b, *a])),
        PixelLayout::Rgb565 => put(&mut u16s.map(rgb565)),
        PixelLayout::Rgba4444 => put(&mut u16s.map(|v| nibbles(v, [12, 8, 4, 0]))),
        PixelLayout::Argb4444 => put(&mut u16s.map(|v| nibbles(v, [8, 4, 0, 12]))),
        PixelLayout::Dxt1 => place_blocks(&mut out, w, h, data.as_chunks::<8>().0.iter().map(decode_bc1_block)),
        PixelLayout::Dxt3 => place_blocks(&mut out, w, h, data.as_chunks::<16>().0.iter().map(decode_bc2_block)),
        PixelLayout::Dxt5 => place_blocks(&mut out, w, h, data.as_chunks::<16>().0.iter().map(decode_bc3_block)),
        PixelLayout::Bc4 => place_blocks(&mut out, w, h, data.as_chunks::<8>().0.iter().map(decode_bc4_block)),
        PixelLayout::Bc5 => place_blocks(&mut out, w, h, data.as_chunks::<16>().0.iter().map(decode_bc5_block)),
        PixelLayout::Bc6h => place_blocks(&mut out, w, h, data.as_chunks::<16>().0.iter().map(|b| bptc::bc6h(b, false))),
        PixelLayout::Bc6hSigned => place_blocks(&mut out, w, h, data.as_chunks::<16>().0.iter().map(|b| bptc::bc6h(b, true))),
        PixelLayout::Bc7 => place_blocks(&mut out, w, h, data.as_chunks::<16>().0.iter().map(bptc::bc7)),
        PixelLayout::EacR | PixelLayout::EacRSigned => {
            place_blocks(&mut out, w, h, data.as_chunks::<8>().0.iter().map(|block| eac::scalar(block, layout == PixelLayout::EacRSigned)))
        }
        PixelLayout::EacRg | PixelLayout::EacRgSigned => {
            place_blocks(&mut out, w, h, data.as_chunks::<16>().0.iter().map(|block| eac::dual(block, layout == PixelLayout::EacRgSigned)))
        }
        PixelLayout::Etc1 => {
            let mut valid = true;
            let texels = data.as_chunks::<8>().0.iter().map_while(|block| {
                let pixels = eac::etc1(block);
                valid &= pixels.is_some();
                pixels
            });
            place_blocks(&mut out, w, h, texels);
            if !valid {
                return Err(DdsError::Malformed("ETC1 differential colour is outside its five-bit range".into()));
            }
        }
    }
    Ok(out)
}

/// Flip a `width`-wide RGBA8 picture upside down.
pub fn flip_rows(rgba: &[u8], width: u32) -> Vec<u8> {
    let row = (width as usize).saturating_mul(4).max(1);
    rgba.chunks(row).rev().flatten().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::{PixelLayout, decode, decode_with_limit, flip_rows, size};
    use crate::DdsError;

    #[test]
    fn uncompressed_and_swizzled_layouts() {
        assert_eq!(decode(PixelLayout::Rgba32, 1, 2, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap(), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(decode(PixelLayout::Argb32, 1, 1, &[9, 1, 2, 3]).unwrap(), [1, 2, 3, 9]);
        assert_eq!(decode(PixelLayout::Bgra32, 1, 1, &[3, 2, 1, 9]).unwrap(), [1, 2, 3, 9]);
        assert_eq!(decode(PixelLayout::Rgb24, 1, 1, &[1, 2, 3]).unwrap(), [1, 2, 3, 255]);
        assert_eq!(decode(PixelLayout::Alpha8, 1, 1, &[7]).unwrap(), [255, 255, 255, 7]);
        assert_eq!(decode(PixelLayout::R8, 1, 1, &[7]).unwrap(), [7, 7, 7, 255]);
        assert_eq!(decode(PixelLayout::Rgb565, 1, 1, &0xf800u16.to_le_bytes()).unwrap(), [255, 0, 0, 255]);
        assert_eq!(decode(PixelLayout::Rgba4444, 1, 1, &0xf00fu16.to_le_bytes()).unwrap(), [255, 0, 0, 255]);
        assert_eq!(decode(PixelLayout::Argb4444, 1, 1, &0xff00u16.to_le_bytes()).unwrap(), [255, 0, 0, 255]);
        assert_eq!(flip_rows(&[1, 1, 1, 1, 2, 2, 2, 2], 1), [2, 2, 2, 2, 1, 1, 1, 1]);
        assert!(matches!(decode(PixelLayout::Rgba32, 2, 2, &[0; 15]), Err(DdsError::Truncated(_))));
        assert!(matches!(decode(PixelLayout::Rgba32, 0, 1, &[]), Err(DdsError::Malformed(_))));
        assert!(matches!(decode_with_limit(PixelLayout::R8, 2, 2, &[0; 4], 15), Err(DdsError::LimitExceeded { .. })));
    }

    #[test]
    fn block_and_packed_layouts_crop_partial_blocks() {
        assert_eq!(size(PixelLayout::Dxt1, 5, 5), Some(32));
        assert_eq!(decode(PixelLayout::Dxt1, 5, 5, &[0; 32]).unwrap().len(), 100);
        // DXT1: colour 0 red, colour 1 blue; the first texel uses index 1, the rest index 0.
        let px = decode(PixelLayout::Dxt1, 4, 4, &[0x00, 0xf8, 0x1f, 0x00, 0b01, 0, 0, 0]).unwrap();
        assert_eq!((&px[..4], &px[4..8]), (&[0, 0, 255, 255][..], &[255, 0, 0, 255][..]));
        // DXT5: alpha 255 -> 0; first texel index 1 (0), rest index 0 (255).
        let mut b = vec![255, 0, 1, 0, 0, 0, 0, 0];
        b.extend_from_slice(&[0x00, 0xf8, 0x1f, 0x00, 0, 0, 0, 0]);
        let px = decode(PixelLayout::Dxt5, 4, 4, &b).unwrap();
        assert_eq!((px[3], px[7]), (0, 255));
        // BC4 & BC5.
        assert_eq!(&decode(PixelLayout::Bc4, 4, 4, &[200, 10, 0, 0, 0, 0, 0, 0]).unwrap()[..4], &[200, 200, 200, 255]);
        let mut n = vec![128, 128, 0, 0, 0, 0, 0, 0];
        n.extend_from_slice(&[128, 128, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&decode(PixelLayout::Bc5, 4, 4, &n).unwrap()[..4], &[128, 128, 254, 255]);
        // Packed RGB9E5 & RGBA1010102.
        let rgb9e5 = (256u32 | (128 << 9) | (64 << 18) | (15 << 27)).to_le_bytes();
        assert_eq!(decode(PixelLayout::Rgb9e5, 1, 1, &rgb9e5).unwrap(), [188, 137, 99, 255]);
        let rgba10 = (1023u32 | (512 << 10) | (2 << 30)).to_le_bytes();
        assert_eq!(decode(PixelLayout::Rgba1010102, 1, 1, &rgba10).unwrap(), [255, 128, 0, 170]);
    }
}
