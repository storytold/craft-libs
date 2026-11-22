//! The tiled JPEG 2000 payload of image resources saved by version 12.2 (`compression` = `jp2`, Verified on 7 re-saved
//! projects: all 130 images have the same pixels as in the older project, the coding being reversible).
//!
//! `tilesCount` records follow the header. Each is four `u32` LE giving x0, y0, x1, y1 in pixels, a `u64` LE byte length,
//! and that many bytes of a JP2 file with one component per channel; an opaque tile may leave out the alpha component.
//! Each record starts at a multiple of 8 from the first.

use super::DecodeError;
use super::image::PixelFormat;
use crate::container::bytes::ByteReader;
use hayro_jpeg2000::{DecodeSettings, DecoderContext, Image};

const RECORD: &str = "resource image";
const RECORD_ALIGN: usize = 8;

/// The channels of a pixel and the bytes of each sample, for formats stored as integers.
fn layout(format: &PixelFormat) -> Option<(usize, usize)> {
    match format {
        PixelFormat::Lum8 => Some((1, 1)),
        PixelFormat::Lum16 => Some((1, 2)),
        PixelFormat::Rgba8 | PixelFormat::Bgra8 => Some((4, 1)),
        PixelFormat::Rgba16 => Some((4, 2)),
        _ => None,
    }
}

fn invalid(message: impl Into<String>) -> DecodeError {
    DecodeError::invalid(RECORD, message.into())
}

fn read_u32(r: &mut ByteReader) -> Result<usize, DecodeError> {
    Ok(r.u32_le().map_err(DecodeError::truncated(RECORD))? as usize)
}

/// Checks the sizes a file claims before the decoder allocates by them: one image of `w` by `h` as one codestream tile.
fn check_sizes(jp2: &[u8], w: usize, h: usize) -> Result<(), DecodeError> {
    const SIZ: [u8; 4] = [0xff, 0x4f, 0xff, 0x51];
    const IHDR: &[u8] = b"ihdr";
    let be = |at: usize| -> Option<usize> {
        let b: [u8; 4] = jp2.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(u32::from_be_bytes(b) as usize)
    };
    let find = |what: &[u8]| jp2.windows(what.len()).position(|w| w == what);
    let (Some(ihdr), Some(siz)) = (find(IHDR), find(&SIZ)) else {
        return Err(invalid("tile without JP2 image header or codestream"));
    };
    // ihdr: height, width. SIZ, from its marker: length, capabilities, then x, y, x origin, y origin, tile x, tile y.
    let header = (be(ihdr + 4), be(ihdr + 8));
    let image = (be(siz + 8), be(siz + 12), be(siz + 16), be(siz + 20), be(siz + 24), be(siz + 28));
    if header != (Some(h), Some(w)) || image != (Some(w), Some(h), Some(0), Some(0), Some(w), Some(h)) {
        return Err(invalid("tile sizes disagree"));
    }
    Ok(())
}

/// One tile's samples, per component.
fn decode_tile(jp2: &[u8], w: usize, h: usize) -> Result<(u32, u32, Vec<Vec<f32>>), DecodeError> {
    check_sizes(jp2, w, h)?;
    // A third-party decoder on untrusted bytes: a panic in it must stay an error here.
    std::panic::catch_unwind(|| {
        let image = Image::new(jp2, &DecodeSettings::default()).map_err(|e| invalid(format!("JPEG 2000: {e:?}")))?;
        let (w, h) = (image.width(), image.height());
        let mut context = DecoderContext::default();
        let decoded = image.decode(&mut context).map_err(|e| invalid(format!("JPEG 2000: {e:?}")))?;
        let samples = decoded.components().iter().map(|c| c.samples().to_vec()).collect();
        Ok((w, h, samples))
    })
    .unwrap_or_else(|_| Err(invalid("JPEG 2000 decoder failed")))
}

/// Decodes all tiles into tightly packed rows of `raw_size` bytes, little endian.
pub(super) fn decode(
    payload: &[u8],
    tiles: usize,
    width: usize,
    height: usize,
    format: &PixelFormat,
    raw_size: usize,
) -> Result<Vec<u8>, DecodeError> {
    let Some((channels, sample)) = layout(format) else {
        return Err(invalid(format!("JPEG 2000 tiles in format {}", format.name())));
    };
    let pitch = width * channels * sample;
    if pitch.checked_mul(height) != Some(raw_size) {
        return Err(invalid("JPEG 2000 image size does not match its raw size"));
    }
    let max = ((1u32 << (sample * 8)) - 1) as f32;
    let mut out = vec![0u8; raw_size];
    let mut r = ByteReader::new(payload);
    for _ in 0..tiles {
        r.seek(r.pos().next_multiple_of(RECORD_ALIGN)).map_err(DecodeError::truncated(RECORD))?;
        let (x0, y0, x1, y1) = (read_u32(&mut r)?, read_u32(&mut r)?, read_u32(&mut r)?, read_u32(&mut r)?);
        let len = usize::try_from(r.u64_le().map_err(DecodeError::truncated(RECORD))?).map_err(|_| invalid("tile length"))?;
        let jp2 = r.bytes(len).map_err(DecodeError::truncated(RECORD))?;
        if x1 <= x0 || y1 <= y0 || x1 > width || y1 > height {
            return Err(invalid(format!("tile {x0},{y0}..{x1},{y1} outside the image")));
        }
        let (w, h, components) = decode_tile(jp2, x1 - x0, y1 - y0)?;
        if (w as usize, h as usize) != (x1 - x0, y1 - y0) {
            return Err(invalid(format!("tile holds {w}x{h} pixels")));
        }
        let opaque_only = channels == 4 && components.len() == 3;
        if components.len() != channels && !opaque_only {
            return Err(invalid(format!("tile holds {} components for {} channels", components.len(), channels)));
        }
        for row in 0..h as usize {
            let Some(line) = out.get_mut((y0 + row) * pitch..(y0 + row + 1) * pitch) else {
                return Err(invalid("tile row outside the image"));
            };
            let Some(line) = line.get_mut(x0 * channels * sample..x1 * channels * sample) else {
                return Err(invalid("tile column outside the image"));
            };
            for (col, pixel) in line.chunks_exact_mut(channels * sample).enumerate() {
                for (c, target) in pixel.chunks_exact_mut(sample).enumerate() {
                    let at = row * w as usize + col;
                    let value = match components.get(c) {
                        Some(samples) => samples.get(at).copied().ok_or_else(|| invalid("tile samples missing"))?,
                        None => max,
                    };
                    let value = (value.round().clamp(0.0, max) as u32).to_le_bytes();
                    for (t, v) in target.iter_mut().zip(value) {
                        *t = v;
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JP2 files made with the open source OpenJPEG encoder from the numbers in the tests.
    const GREY_16: &str = "0000000c6a5020200d0a870a00000014667479706a703220000000006a7032200000002d6a7032680000001669686472000000020000000200010f0700000000000f636f6c72010000000000110000008e6a703263ff4fff5100290000000000020000000200000000000000000000000200000002000000000000000000010f0101ff52000c00000001000004040001ff5c00044080ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e34ff90000a00000000001c0001ff93dff8905007a36e153f49dbf32d5fffd9";
    const RGB_8: &str = "0000000c6a5020200d0a870a00000014667479706a703220000000006a7032200000002d6a703268000000166968647200000002000000020003070700000000000f636f6c72010000000000100000009f6a703263ff4fff51002f000000000002000000020000000000000000000000020000000200000000000000000003070101070101070101ff52000c00000001000004040001ff5c00044040ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e34ff90000a0000000000270001ff93cfb41406487feb8bcfb4180659427eff7fcfb41407a4bcda2cffd9";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
    }

    fn tile(x0: u32, y0: u32, x1: u32, y1: u32, jp2: &[u8]) -> Vec<u8> {
        let mut t = Vec::new();
        for v in [x0, y0, x1, y1] {
            t.extend_from_slice(&v.to_le_bytes());
        }
        t.extend_from_slice(&(jp2.len() as u64).to_le_bytes());
        t.extend_from_slice(jp2);
        t.resize(t.len().next_multiple_of(RECORD_ALIGN), 0);
        t
    }

    fn u16s(bytes: &[u8]) -> Vec<u16> {
        bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect()
    }

    #[test]
    fn tiles_land_in_their_place_at_full_depth() {
        let grey = hex(GREY_16);
        let mut payload = tile(2, 0, 4, 2, &grey); // padded to 8 before the next record
        assert_eq!(payload.len() % 8, 0);
        payload.extend(tile(0, 0, 2, 2, &grey));
        let raw = decode(&payload, 2, 4, 2, &PixelFormat::Lum16, 16).unwrap();
        assert_eq!(u16s(&raw), [0, 1000, 0, 1000, 40000, 65535, 40000, 65535]);
    }

    #[test]
    fn opaque_tiles_get_alpha() {
        let raw = decode(&tile(0, 0, 2, 2, &hex(RGB_8)), 1, 2, 2, &PixelFormat::Rgba8, 16).unwrap();
        assert_eq!(raw, [10, 20, 30, 255, 50, 60, 70, 255, 90, 100, 110, 255, 130, 140, 150, 255]);
    }

    #[test]
    fn malformed_payloads_are_errors() {
        let grey = hex(GREY_16);
        let good = tile(0, 0, 2, 2, &grey);
        assert!(decode(&good, 2, 2, 2, &PixelFormat::Lum16, 8).is_err(), "second tile missing");
        assert!(decode(&tile(1, 1, 3, 3, &grey), 1, 2, 2, &PixelFormat::Lum16, 8).is_err(), "outside");
        assert!(decode(&tile(0, 0, 1, 1, &grey), 1, 2, 2, &PixelFormat::Lum16, 8).is_err(), "size");
        assert!(decode(&good, 1, 2, 2, &PixelFormat::Rgba16F, 32).is_err(), "float format");
        assert!(decode(&good, 1, 2, 2, &PixelFormat::Lum16, 9).is_err(), "raw size");
        let unpadded = 24 + grey.len();
        for len in 0..unpadded {
            assert!(decode(&good[..len], 1, 2, 2, &PixelFormat::Lum16, 8).is_err(), "a tile cut at {len} bytes");
        }
        for at in 24..good.len() {
            let mut bad = good.clone();
            bad[at] ^= 0xff;
            let _ = decode(&bad, 1, 2, 2, &PixelFormat::Lum16, 8);
        }
    }

    #[test]
    fn hostile_tile_payloads_never_panic() {
        let seeds = [tile(0, 0, 2, 2, &hex(GREY_16)), tile(0, 0, 2, 2, &hex(RGB_8)), hex(GREY_16)];
        crate::robustness_tests::never_panics("jp2 tiles", &seeds, |d| {
            let _ = decode(d, 1, 2, 2, &PixelFormat::Lum16, 8);
            let _ = decode(d, 1, 2, 2, &PixelFormat::Rgba8, 16);
            let _ = decode_tile(d, 2, 2);
        });
    }
}
