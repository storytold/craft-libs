//! APNG container assembly for already-compressed RGBA8 frame streams.
//!
//! Extracted from AniCraft's animated export (Tanish Manku, commit a20a488f).
//! Frame selection, timing quantization and pixel compression remain caller policy.
//! Payloads must be complete PNG IDAT zlib streams for non-interlaced RGBA8 images
//! of the declared frame size. They are not decompressed or validated here.

use crate::CodecError;

/// One compressed frame. The first must cover the entire canvas.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    /// Exact delay numerator; zero is retained without rounding.
    pub delay_num: u16,
    /// Exact denominator; zero has the APNG-defined meaning of 100.
    pub delay_den: u16,
    /// Blend over the previous canvas instead of replacing the rectangle.
    pub over: bool,
    /// Concatenated IDAT payload, not complete PNG chunks.
    pub data: &'a [u8],
}

fn invalid(message: &str) -> CodecError {
    CodecError::InvalidImage(message.into())
}

// PNG 3 §7.1 restricts unsigned integers to 31 bits. The last fdAT sequence
// is 2 * count - 2; validate arithmetically without allocating frame headers.
fn animation_count(len: usize, plays: u32) -> Result<u32, CodecError> {
    let count = u32::try_from(len).map_err(|_| invalid("too many APNG frames"))?;
    if plays > 0x7fff_ffff || count == 0 || count > (0x7fff_ffff / 2) + 1 {
        return Err(invalid("APNG play count or sequence exceeds PNG integer range"));
    }
    Ok(count)
}

/// Assemble an RGBA8 APNG with dispose-none frames and the first frame as default image.
/// `plays == 0` means infinite looping. `max_bytes` bounds the returned encoded file,
/// not caller-owned inputs or decoded pixels. Checked sizing precedes allocation;
/// malformed geometry, chunk lengths and sequence exhaustion fail before output.
pub fn encode(width: u32, height: u32, plays: u32, frames: &[Frame<'_>], max_bytes: usize) -> Result<Vec<u8>, CodecError> {
    if width == 0 || height == 0 || width > 0x7fff_ffff || height > 0x7fff_ffff || frames.is_empty() {
        return Err(invalid("APNG needs a nonempty canvas and frames"));
    }
    let count = animation_count(frames.len(), plays)?;
    let mut size = 8usize + 25 + 20 + 12; // signature, IHDR, acTL, IEND
    for (index, f) in frames.iter().enumerate() {
        if f.width == 0
            || f.height == 0
            || f.x.checked_add(f.width).is_none_or(|end| end > width)
            || f.y.checked_add(f.height).is_none_or(|end| end > height)
            || (index == 0 && (f.x != 0 || f.y != 0 || f.width != width || f.height != height))
        {
            return Err(invalid("APNG frame is outside canvas or first frame is not full size"));
        }
        let prefix = if index == 0 { 0 } else { 4 };
        let data_len = f.data.len().checked_add(prefix).ok_or_else(|| invalid("APNG chunk overflow"))?;
        if f.data.is_empty() || data_len > 0x7fff_ffff {
            return Err(invalid("APNG frame payload is empty or too large"));
        }
        size = size
            .checked_add(38 + 12)
            .and_then(|n| n.checked_add(data_len))
            .filter(|n| *n <= max_bytes)
            .ok_or_else(|| CodecError::LimitExceeded("APNG output byte budget".into()))?;
    }
    let mut out = Vec::new();
    out.try_reserve_exact(size).map_err(|_| CodecError::LimitExceeded("APNG output allocation".into()))?;
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8..].copy_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &[], &ihdr);
    let mut actl = [0u8; 8];
    actl[..4].copy_from_slice(&count.to_be_bytes());
    actl[4..].copy_from_slice(&plays.to_be_bytes());
    chunk(&mut out, b"acTL", &[], &actl);
    let mut sequence = 0u32;
    for (index, f) in frames.iter().enumerate() {
        let mut control = [0u8; 26];
        for (target, value) in control[..20].as_chunks_mut::<4>().0.iter_mut().zip([sequence, f.width, f.height, f.x, f.y]) {
            target.copy_from_slice(&value.to_be_bytes());
        }
        control[20..22].copy_from_slice(&f.delay_num.to_be_bytes());
        control[22..24].copy_from_slice(&f.delay_den.to_be_bytes());
        control[25] = u8::from(f.over);
        chunk(&mut out, b"fcTL", &[], &control);
        if index == 0 {
            chunk(&mut out, b"IDAT", &[], f.data);
        } else {
            chunk(&mut out, b"fdAT", &(sequence + 1).to_be_bytes(), f.data);
        }
        // The final frame does not need a next sequence number.
        if index + 1 < frames.len() {
            sequence += if index == 0 { 1 } else { 2 };
        }
    }
    chunk(&mut out, b"IEND", &[], &[]);
    Ok(out)
}

// Lengths and total capacity were validated before writing. Prefix avoids a frame-sized
// intermediate fdAT allocation. CRC covers the type, prefix and payload, as PNG requires.
#[allow(clippy::cast_possible_truncation)]
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], prefix: &[u8], data: &[u8]) {
    out.extend_from_slice(&((prefix.len() + data.len()) as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(prefix);
    out.extend_from_slice(data);
    let crc = crc32fast::hash(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn frame(data: &[u8]) -> Frame<'_> {
        Frame { width: 1, height: 1, x: 0, y: 0, delay_num: 200, delay_den: 2997, over: false, data }
    }

    fn chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        let mut pos = 8;
        let mut result = Vec::new();
        while pos < bytes.len() {
            let n = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
            let kind = bytes[pos + 4..pos + 8].try_into().unwrap();
            let end = pos + 8 + n;
            assert_eq!(crc32fast::hash(&bytes[pos + 4..end]), u32::from_be_bytes(bytes[end..end + 4].try_into().unwrap()));
            result.push((kind, bytes[pos + 8..end].to_vec()));
            pos = end + 4;
        }
        result
    }

    #[test]
    fn png_integer_boundaries_without_giant_allocations() {
        assert_eq!(animation_count(1, 0).unwrap(), 1);
        assert_eq!(animation_count(0x4000_0000, 0x7fff_ffff).unwrap(), 0x4000_0000);
        assert!(animation_count(0x4000_0001, 0).is_err());
        assert!(animation_count(0, 0).is_err());
        assert!(animation_count(1, 0x8000_0000).is_err());
        assert!(animation_count(1, u32::MAX).is_err());
        assert!(encode(1, 1, 0x8000_0000, &[frame(&[1])], 1000).is_err());
    }

    #[test]
    fn preserves_rgba_delays_loop_blend_and_sequences() {
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[12, 34, 56, 128]).unwrap();
            writer.finish().unwrap();
        }
        let data: Vec<u8> = chunks(&png_bytes).into_iter().filter(|(kind, _)| kind == b"IDAT").flat_map(|(_, data)| data).collect();
        let first = frame(&data);
        let second = Frame { over: true, delay_num: 0, delay_den: 0, ..first };
        let encoded = encode(1, 1, 7, &[first, second], 4096).unwrap();
        let parts = chunks(&encoded);
        assert_eq!(
            parts.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            vec![*b"IHDR", *b"acTL", *b"fcTL", *b"IDAT", *b"fcTL", *b"fdAT", *b"IEND"]
        );
        assert_eq!(parts[1].1, [0, 0, 0, 2, 0, 0, 0, 7]);
        assert_eq!(&parts[2].1[20..], &[0, 200, 11, 181, 0, 0]);
        assert_eq!(&parts[4].1[..4], &1u32.to_be_bytes());
        assert_eq!(&parts[4].1[20..], &[0, 0, 0, 0, 0, 1]);
        assert_eq!(&parts[5].1[..4], &2u32.to_be_bytes());
        assert_eq!(&parts[5].1[4..], data.as_slice());
        let mut reader = png::Decoder::new(std::io::Cursor::new(&encoded)).read_info().unwrap();
        let mut pixels = [0u8; 4];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(pixels, [12, 34, 56, 128]);
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(pixels, [12, 34, 56, 128]);
        assert!(encode(1, 1, 7, &[first, second], encoded.len() - 1).is_err());
        assert_eq!(encode(1, 1, 7, &[first, second], encoded.len()).unwrap(), encoded);
    }

    #[test]
    fn rejects_invalid_geometry_empty_payload_and_budget() {
        let f = frame(&[1]);
        assert!(encode(0, 1, 0, &[f], 1000).is_err());
        assert!(encode(1, 1, 0, &[], 1000).is_err());
        assert!(encode(2, 1, 0, &[f], 1000).is_err());
        assert!(encode(1, 1, 0, &[Frame { data: &[], ..f }], 1000).is_err());
        assert!(encode(1, 1, 0, &[f, Frame { x: u32::MAX, ..f }], 1000).is_err());
        assert!(encode(1, 1, 0, &[f, Frame { height: 0, ..f }], 1000).is_err());
        assert!(encode(1, 1, 0, &[f], 0).is_err());
    }
}
