//! Sun Raster files (`.ras`, `.sun`): 1-bit, 8-bit (gray or colour-mapped), 24-bit RGB and 32-bit
//! RGBA, uncompressed or byte-encoded (RLE) on read; uncompressed on write.
//!
//! Follows the published rasterfile format: a 32-byte big-endian header, an optional colour map,
//! then rows padded to an even number of bytes. In the "standard" type (1) 24- and 32-bit pixels
//! are stored blue-first; in the "RGB" type (3) red-first. Other types (TIFF, IFF, experimental)
//! are reported as unsupported.

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::Limits;

const F: Format = Format::SunRaster;
const HEADER: usize = 32;
const MAGIC: u32 = 0x59a6_6a95;

fn bad(reason: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, reason)
}

fn be32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Whether `b` starts with a Sun rasterfile header.
pub(crate) fn looks_like_sunraster(b: &[u8]) -> bool {
    b.len() >= HEADER && be32(b, 0) == MAGIC && matches!(be32(b, 12), 1 | 8 | 24 | 32)
}

/// Expand byte-encoded data (`0x80` escapes a run) until `want` bytes are produced.
fn unrle(src: &[u8], want: usize) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    out.try_reserve_exact(want).map_err(|_| bad("pixel buffer is too large"))?;
    let mut i = 0;
    while out.len() < want {
        let b = *src.get(i).ok_or_else(|| bad("truncated RLE data"))?;
        i += 1;
        if b != 0x80 {
            out.push(b);
            continue;
        }
        let n = *src.get(i).ok_or_else(|| bad("truncated RLE data"))?;
        i += 1;
        if n == 0 {
            out.push(0x80);
            continue;
        }
        let v = *src.get(i).ok_or_else(|| bad("truncated RLE data"))?;
        i += 1;
        let count = usize::from(n) + 1;
        if count > want - out.len() {
            return Err(bad("RLE run passes the end of the image"));
        }
        out.resize(out.len() + count, v);
    }
    Ok(out)
}

pub(crate) fn decode(b: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    if !looks_like_sunraster(b) {
        return Err(bad("invalid Sun raster header"));
    }
    let (w, h, depth, kind, map_type, map_len) = (be32(b, 4), be32(b, 8), be32(b, 12) as usize, be32(b, 20), be32(b, 24), be32(b, 28) as usize);
    if !matches!(kind, 0..=3) {
        return Err(CodecError::unsupported(F, format!("Sun raster type {kind} is not supported")));
    }
    if map_type > 2 {
        return Err(CodecError::unsupported(F, format!("Sun raster map type {map_type} is not supported")));
    }
    let layout = match depth {
        1 | 8 if map_type != 1 => ChannelLayout::Gray,
        1 | 8 | 24 => ChannelLayout::Rgb,
        _ => ChannelLayout::Rgba,
    };
    limits.check(w, h, layout, SampleType::U8)?;
    let (wu, hu) = (w as usize, h as usize);
    let stride = wu.checked_mul(depth).and_then(|n| n.checked_add(15)).ok_or_else(|| bad("image size overflow"))? / 16 * 2;
    let want = stride.checked_mul(hu).ok_or_else(|| bad("image size overflow"))?;
    let data_at = HEADER.checked_add(map_len).ok_or_else(|| bad("colour map size overflow"))?;
    let map = b.get(HEADER..data_at).ok_or_else(|| bad("truncated colour map"))?;
    let rest = &b[data_at..];
    let raw;
    let rows: &[u8] = if kind == 2 {
        raw = unrle(rest, want)?;
        &raw
    } else {
        rest.get(..want).ok_or_else(|| bad("the file is too short for the declared image"))?
    };
    let channels = layout.channels();
    let mut out = Vec::new();
    out.try_reserve_exact(wu * hu * channels).map_err(|_| bad("pixel buffer is too large"))?;
    let entries = map_len / 3;
    for y in 0..hu {
        let row = &rows[y * stride..(y + 1) * stride];
        for x in 0..wu {
            match depth {
                1 => {
                    let bit = (row[x / 8] >> (7 - x % 8)) & 1;
                    if map_type == 1 {
                        // The bit is an index into the (two-entry or longer) colour map.
                        let i = usize::from(bit);
                        if i >= entries {
                            return Err(bad("pixel index is outside the colour map"));
                        }
                        out.extend_from_slice(&[map[i], map[entries + i], map[2 * entries + i]]);
                    } else {
                        // A set bit is black.
                        out.push(if bit == 1 { 0 } else { 255 });
                    }
                }
                8 if map_type == 1 => {
                    let i = usize::from(row[x]);
                    if i >= entries {
                        return Err(bad("pixel index is outside the colour map"));
                    }
                    out.extend_from_slice(&[map[i], map[entries + i], map[2 * entries + i]]);
                }
                8 => out.push(row[x]),
                24 => {
                    let p = &row[3 * x..3 * x + 3];
                    out.extend_from_slice(&if kind == 3 { [p[0], p[1], p[2]] } else { [p[2], p[1], p[0]] });
                }
                _ => {
                    let p = &row[4 * x..4 * x + 4];
                    // In the standard types the first byte (X in XBGR) is unused, not alpha.
                    out.extend_from_slice(&if kind == 3 { [p[1], p[2], p[3], p[0]] } else { [p[3], p[2], p[1], 255] });
                }
            }
        }
    }
    Image::from_u8(w, h, layout, out)
}

/// A buffer of `cap` bytes, or an error instead of an abort when the allocation cannot be made.
fn reserve(cap: usize) -> std::result::Result<Vec<u8>, std::collections::TryReserveError> {
    let mut v = Vec::new();
    v.try_reserve_exact(cap)?;
    Ok(v)
}

/// Writes an uncompressed rasterfile: 8-bit gray, 24-bit RGB or 32-bit RGBA, in the RGB-ordered type.
pub(crate) fn encode(src: &Image, plan: Plan) -> Result<Vec<u8>, CodecError> {
    let enc = |msg: String| CodecError::encode(F, msg);
    let img = src.converted(plan.layout, plan.sample);
    if img.sample_type() != SampleType::U8 {
        return Err(enc(format!("Sun raster stores 8 bits per channel, not {:?}", img.sample_type())));
    }
    let depth: usize = match img.layout() {
        ChannelLayout::Gray => 8,
        ChannelLayout::Rgb => 24,
        ChannelLayout::Rgba => 32,
        l => return Err(enc(format!("no Sun raster layout for {l:?}"))),
    };
    let (w, h) = img.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let bytes = depth / 8;
    let stride = (wu * bytes).next_multiple_of(2);
    let len = stride.checked_mul(hu).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| enc("image too large".into()))?;
    let data = img.data();
    if data.len() != wu * hu * bytes {
        return Err(enc("the image has the wrong number of samples".into()));
    }
    let mut out = reserve(HEADER.checked_add(len as usize).ok_or_else(|| enc("image too large".into()))?)
        .map_err(|_| enc("the image is too large to encode".into()))?;
    for v in [MAGIC, w, h, depth as u32, len, 3, 0, 0] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for y in 0..hu {
        let row = &data[y * wu * bytes..(y + 1) * wu * bytes];
        if depth == 32 {
            // Stored alpha first.
            for p in row.as_chunks::<4>().0 {
                out.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
            }
        } else {
            out.extend_from_slice(row);
        }
        out.resize(HEADER + (y + 1) * stride, 0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(w: u32, h: u32, layout: ChannelLayout) -> Image {
        let n = w as usize * h as usize * layout.channels();
        Image::from_u8(w, h, layout, (0..n).map(|i| ((i * 13 + i / 3) % 253) as u8).collect()).unwrap()
    }

    #[test]
    fn feature_config_round_trips_gray_rgb_and_rgba_with_odd_widths() {
        for layout in [ChannelLayout::Gray, ChannelLayout::Rgb, ChannelLayout::Rgba] {
            let img = pattern(13, 7, layout);
            let bytes = encode(&img, Plan { layout, sample: SampleType::U8 }).unwrap();
            assert!(looks_like_sunraster(&bytes));
            let back = decode(&bytes, &Limits::default()).unwrap();
            assert_eq!((back.layout(), back.data()), (layout, img.data()));
        }
    }

    fn file(depth: u32, kind: u32, map: &[u8], map_type: u32, w: u32, h: u32, data: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        for v in [MAGIC, w, h, depth, data.len() as u32, kind, map_type, map.len() as u32] {
            f.extend_from_slice(&v.to_be_bytes());
        }
        f.extend_from_slice(map);
        f.extend_from_slice(data);
        f
    }

    #[test]
    fn feature_config_standard_type_is_blue_first_and_rle_expands() {
        let f = file(24, 1, &[], 0, 1, 1, &[1, 2, 3, 0]);
        assert_eq!(decode(&f, &Limits::default()).unwrap().data(), &[3, 2, 1]);
        // 0x80 0x00 is a literal 0x80; 0x80 n v is n + 1 copies of v. The 2x2 image has stride 2.
        let f = file(8, 2, &[], 0, 2, 2, &[0x80, 0x00, 0x80, 0x00, 0x80, 0x01, 7]);
        assert_eq!(decode(&f, &Limits::default()).unwrap().data(), &[0x80, 0x80, 7, 7]);
        // A run that passes the end of the image is an error.
        assert!(decode(&file(8, 2, &[], 0, 2, 2, &[0x80, 0x09, 7]), &Limits::default()).is_err());
    }

    #[test]
    fn feature_config_colour_mapped_and_one_bit_files() {
        // Two entries: red plane, green plane, blue plane.
        let map = [10, 20, 30, 40, 50, 60];
        let f = file(8, 1, &map, 1, 2, 1, &[0, 1]);
        let img = decode(&f, &Limits::default()).unwrap();
        assert_eq!((img.layout(), img.data()), (ChannelLayout::Rgb, &[10, 30, 50, 20, 40, 60][..]));
        let f = file(8, 1, &map, 1, 1, 1, &[5, 0]);
        assert!(decode(&f, &Limits::default()).is_err());
        let f = file(1, 1, &[], 0, 3, 1, &[0b1010_0000, 0]);
        assert_eq!(decode(&f, &Limits::default()).unwrap().data(), &[0, 255, 0]);
    }

    #[test]
    fn feature_config_one_bit_files_with_a_colour_map_read_as_colour() {
        // Two entries: red plane, green plane, blue plane; bit 0 is entry 0.
        let map = [10, 20, 30, 40, 50, 60];
        let f = file(1, 1, &map, 1, 3, 1, &[0b0100_0000, 0]);
        let img = decode(&f, &Limits::default()).unwrap();
        assert_eq!((img.layout(), img.data()), (ChannelLayout::Rgb, &[10, 30, 50, 20, 40, 60, 10, 30, 50][..]));
        // A one-entry map has no entry for a set bit.
        assert!(decode(&file(1, 1, &[1, 2, 3], 1, 1, 1, &[0b1000_0000, 0]), &Limits::default()).is_err());
    }

    #[test]
    fn feature_config_the_unused_first_byte_of_standard_32_bit_pixels_is_not_alpha() {
        let img = decode(&file(32, 1, &[], 0, 1, 1, &[0, 30, 20, 10]), &Limits::default()).unwrap();
        assert_eq!((img.layout(), img.data()), (ChannelLayout::Rgba, &[10, 20, 30, 255][..]));
        // In the RGB-ordered type the first byte is the alpha the writer stores there.
        let img = decode(&file(32, 3, &[], 0, 1, 1, &[7, 10, 20, 30]), &Limits::default()).unwrap();
        assert_eq!(img.data(), &[10, 20, 30, 7]);
    }

    #[test]
    fn feature_config_an_output_buffer_too_large_to_allocate_is_an_error() {
        assert!(reserve(usize::MAX).is_err());
        assert!(reserve(16).is_ok());
    }

    #[test]
    fn feature_config_hostile_input_is_an_error_not_a_panic() {
        let img = pattern(9, 5, ChannelLayout::Rgb);
        let good = encode(&img, Plan { layout: ChannelLayout::Rgb, sample: SampleType::U8 }).unwrap();
        for len in 0..good.len() {
            let _ = decode(&good[..len], &Limits::default());
        }
        let mut huge = good.clone();
        huge[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        huge[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&huge, &Limits::default()).is_err());
        let mut bad_map = good;
        bad_map[28..32].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&bad_map, &Limits::default()).is_err());
        assert!(matches!(decode(&file(24, 4, &[], 0, 1, 1, &[0; 4]), &Limits::default()), Err(CodecError::Unsupported { .. })));
    }
}
