//! Bounded decoders for the bzip2, ZIP LZMA, XZ and Zstandard entry methods, from Craft
//! Anything's `decompress.rs`. Each caps its output at `max_out` bytes (`TooLarge` rather than
//! allocating without limit) and never panics on a corrupt or truncated stream.

use std::io;
#[cfg(any(feature = "lenient-bzip2", feature = "lenient-lzma"))]
use std::io::Read;
#[cfg(any(feature = "lenient-lzma", feature = "lenient-zstd"))]
use std::io::Write;

/// Why a decode stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DecompressError {
    #[error("the compressed stream is damaged")]
    Corrupt,
    #[error("the compressed stream ends early")]
    #[cfg(any(feature = "lenient-lzma", feature = "lenient-zstd"))]
    Truncated,
    #[error("the output would exceed the size limit")]
    TooLarge,
}

/// A `Write` sink that collects at most `cap` bytes, then refuses the rest. The `exceeded` flag lets
/// the caller tell a size-limit stop from a genuine decode error.
#[cfg(any(feature = "lenient-lzma", feature = "lenient-zstd"))]
struct CapWrite {
    buf: Vec<u8>,
    cap: usize,
    exceeded: bool,
}

#[cfg(any(feature = "lenient-lzma", feature = "lenient-zstd"))]
impl CapWrite {
    fn new(cap: usize) -> Self {
        CapWrite { buf: Vec::new(), cap, exceeded: false }
    }
}

#[cfg(any(feature = "lenient-lzma", feature = "lenient-zstd"))]
impl Write for CapWrite {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.buf.len().saturating_add(data.len()) > self.cap {
            self.exceeded = true;
            return Err(io::Error::other("output size limit reached"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn cap_of(max_out: u64) -> usize {
    usize::try_from(max_out).unwrap_or(usize::MAX)
}

/// Read a `Read` source into a `Vec`, capped at `cap` bytes.
#[cfg(feature = "lenient-bzip2")]
fn read_capped(mut src: impl Read, cap: usize) -> Result<Vec<u8>, DecompressError> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match src.read(&mut chunk) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                let slice = chunk.get(..n).unwrap_or(&[]);
                if out.len().saturating_add(slice.len()) > cap {
                    return Err(DecompressError::TooLarge);
                }
                out.extend_from_slice(slice);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(DecompressError::Corrupt),
        }
    }
}

#[cfg(feature = "lenient-lzma")]
/// Decompress every xz stream, including standard stream padding, within one output budget.
pub(crate) fn xz(input: &[u8], max_out: u64) -> Result<Vec<u8>, DecompressError> {
    let ranges = xz_stream_ranges(input, max_out)?;
    let mut out = CapWrite::new(cap_of(max_out));
    for range in ranges.into_iter().rev() {
        let stream = input.get(range).ok_or(DecompressError::Truncated)?;
        let mut cursor = io::Cursor::new(stream);
        match lzma_rs::xz_decompress(&mut cursor, &mut out) {
            Ok(()) => {}
            Err(_) if out.exceeded => return Err(DecompressError::TooLarge),
            Err(lzma_rs::error::Error::HeaderTooShort(_)) => return Err(DecompressError::Truncated),
            Err(_) => return Err(DecompressError::Corrupt),
        }
    }
    Ok(out.buf)
}

#[cfg(feature = "lenient-lzma")]
/// Find stream boundaries backwards through their checked footer/index sizes, without
/// repeatedly trying the decoder at candidate magic bytes inside compressed data.
fn xz_stream_ranges(input: &[u8], max_out: u64) -> Result<Vec<std::ops::Range<usize>>, DecompressError> {
    let mut ranges = Vec::new();
    let mut end = input.len();
    let mut declared = 0u64;
    while end > 0 {
        let padded_end = end;
        while end > 0 && input.get(end - 1) == Some(&0) {
            end -= 1;
        }
        if end == 0 || !(padded_end - end).is_multiple_of(4) {
            return Err(DecompressError::Corrupt);
        }
        let footer_at = end.checked_sub(12).ok_or(DecompressError::Truncated)?;
        let footer = input.get(footer_at..end).ok_or(DecompressError::Truncated)?;
        let read = crate::lenient::bytes::Reader::new(footer);
        let footer_fields = footer.get(4..10).ok_or(DecompressError::Truncated)?;
        if footer.get(10..) != Some(b"YZ") || read.u32_le(0) != Some(crate::lenient::crc32(footer_fields)) {
            return Err(DecompressError::Corrupt);
        }
        let backward = u64::from(read.u32_le(4).ok_or(DecompressError::Truncated)?);
        let index_len = usize::try_from((backward + 1) * 4).map_err(|_| DecompressError::Corrupt)?;
        let index_at = footer_at.checked_sub(index_len).ok_or(DecompressError::Truncated)?;
        let index = input.get(index_at..footer_at).ok_or(DecompressError::Truncated)?;
        let crc_at = index.len().checked_sub(4).ok_or(DecompressError::Truncated)?;
        let index_read = crate::lenient::bytes::Reader::new(index);
        let index_fields = index.get(..crc_at).ok_or(DecompressError::Truncated)?;
        if index.first() != Some(&0) || index_read.u32_le(crc_at) != Some(crate::lenient::crc32(index_fields)) {
            return Err(DecompressError::Corrupt);
        }
        let mut records = index.get(1..crc_at).ok_or(DecompressError::Corrupt)?;
        let count = xz_vli(&mut records)?;
        if count > (records.len() / 2) as u64 {
            return Err(DecompressError::Corrupt);
        }
        let mut block_bytes = 0usize;
        for _ in 0..count {
            let unpadded = xz_vli(&mut records)?;
            if unpadded == 0 {
                return Err(DecompressError::Corrupt);
            }
            let padded = usize::try_from(unpadded.checked_add(3).ok_or(DecompressError::Corrupt)? & !3).map_err(|_| DecompressError::Corrupt)?;
            block_bytes = block_bytes.checked_add(padded).filter(|bytes| *bytes <= index_at).ok_or(DecompressError::Corrupt)?;
            declared = declared.checked_add(xz_vli(&mut records)?).filter(|size| *size <= max_out).ok_or(DecompressError::TooLarge)?;
        }
        if records.len() > 3 || records.iter().any(|&byte| byte != 0) {
            return Err(DecompressError::Corrupt);
        }
        let start = index_at.checked_sub(block_bytes).and_then(|at| at.checked_sub(12)).ok_or(DecompressError::Corrupt)?;
        let magic_end = start.checked_add(6).ok_or(DecompressError::Corrupt)?;
        if input.get(start..magic_end) != Some(b"\xfd7zXZ\0") {
            return Err(DecompressError::Corrupt);
        }
        ranges.push(start..end);
        end = start;
    }
    if ranges.is_empty() {
        return Err(DecompressError::Corrupt);
    }
    Ok(ranges)
}

#[cfg(feature = "lenient-lzma")]
/// XZ variable-length integers have at most nine bytes and no redundant final zero.
fn xz_vli(input: &mut &[u8]) -> Result<u64, DecompressError> {
    let mut value = 0u64;
    for shift in (0..63).step_by(7) {
        let (&byte, rest) = input.split_first().ok_or(DecompressError::Truncated)?;
        *input = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return if shift != 0 && byte == 0 { Err(DecompressError::Corrupt) } else { Ok(value) };
        }
    }
    Err(DecompressError::Corrupt)
}

#[cfg(feature = "lenient-lzma")]
/// Decode the property bytes and raw stream of a ZIP method-14 member.
pub(crate) fn lzma_zip(input: &[u8], expected_size: u64, has_end_marker: bool, max_out: u64) -> Result<Vec<u8>, DecompressError> {
    let cap = cap_of(max_out).min(isize::MAX as usize);
    if expected_size > cap as u64 {
        return Err(DecompressError::TooLarge);
    }
    let (properties, stream) = input.split_at_checked(5).ok_or(DecompressError::Truncated)?;
    let [property, d0, d1, d2, d3] = <[u8; 5]>::try_from(properties).map_err(|_| DecompressError::Truncated)?;
    let window_limit = u32::try_from(cap).unwrap_or(u32::MAX).max(4096);
    let dictionary = u32::from_le_bytes([d0, d1, d2, d3]).max(4096).min(window_limit);
    let [d0, d1, d2, d3] = dictionary.to_le_bytes();
    let properties = [property, d0, d1, d2, d3];
    let size = if has_end_marker { None } else { Some(expected_size) };
    let options = lzma_rs::decompress::Options {
        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(size),
        memlimit: Some(dictionary as usize),
        ..Default::default()
    };
    let mut out = CapWrite::new(cap);
    let mut cursor = io::Cursor::new(properties).chain(io::Cursor::new(stream));
    match lzma_rs::lzma_decompress_with_options(&mut cursor, &mut out, &options) {
        Ok(()) if cursor.get_ref().1.position() == stream.len() as u64 => Ok(out.buf),
        Ok(()) => Err(DecompressError::Corrupt),
        Err(_) if out.exceeded => Err(DecompressError::TooLarge),
        Err(lzma_rs::error::Error::HeaderTooShort(_)) => Err(DecompressError::Truncated),
        Err(_) => Err(DecompressError::Corrupt),
    }
}

#[cfg(feature = "lenient-bzip2")]
/// Decompress a bzip2 stream, producing at most `max_out` bytes.
pub(crate) fn bzip2(input: &[u8], max_out: u64) -> Result<Vec<u8>, DecompressError> {
    read_capped(bzip2_rs::DecoderReader::new(input), cap_of(max_out))
}

#[cfg(feature = "lenient-zstd")]
/// Decompress a Zstandard stream, producing at most `max_out` bytes.
pub(crate) fn zstd(mut input: &[u8], max_out: u64) -> Result<Vec<u8>, DecompressError> {
    if input.is_empty() {
        return Err(DecompressError::Corrupt);
    }
    let mut out = CapWrite::new(cap_of(max_out));
    while !input.is_empty() {
        if matches!(input.get(..4), Some([0x50..=0x5f, 0x2a, 0x4d, 0x18])) {
            let header = input.get(4..8).and_then(|s| s.try_into().ok()).ok_or(DecompressError::Truncated)?;
            let len = u32::from_le_bytes(header) as usize;
            input = input.get(8..).and_then(|body| body.get(len..)).ok_or(DecompressError::Truncated)?;
            continue;
        }
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(&mut input).map_err(|_| DecompressError::Corrupt)?;
        if io::copy(&mut decoder, &mut out).is_err() {
            return Err(if out.exceeded { DecompressError::TooLarge } else { DecompressError::Corrupt });
        }
    }
    Ok(out.buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lenient::testkit as fx;

    #[cfg(feature = "lenient-lzma")]
    #[test]
    fn xz_round_trip() {
        let c = fx::XZ_FIXTURE;
        assert_eq!(xz(c, 1 << 20).unwrap(), fx::XZ_PLAIN);
        // A tiny cap reports TooLarge, never a partial lie.
        assert_eq!(xz(c, 4), Err(DecompressError::TooLarge));
        // Truncation never panics.
        for len in 0..c.len() {
            let _ = xz(&c[..len], 1 << 20);
        }
    }

    #[cfg(feature = "lenient-lzma")]
    #[test]
    fn xz_concatenated_streams_and_padding_share_one_budget() {
        let input = [fx::XZ_FIXTURE, &[0; 4], fx::XZ_FIXTURE, &[0; 8]].concat();
        let expected = [fx::XZ_PLAIN, fx::XZ_PLAIN].concat();
        assert_eq!(xz(&input, expected.len() as u64).unwrap(), expected);
        assert_eq!(xz(&input, expected.len() as u64 - 1), Err(DecompressError::TooLarge));
        let mut empty = Vec::new();
        lzma_rs::xz_compress(&mut io::Cursor::new(b""), &mut empty).unwrap();
        let with_empty = [&empty[..], fx::XZ_FIXTURE, &empty[..]].concat();
        assert_eq!(xz(&with_empty, fx::XZ_PLAIN.len() as u64).unwrap(), fx::XZ_PLAIN);
        assert!(xz(&empty, 0).unwrap().is_empty());
    }

    #[cfg(feature = "lenient-lzma")]
    #[test]
    fn xz_rejects_incomplete_members_and_nonstandard_padding() {
        for padding in [1, 2, 3, 5] {
            let input = [fx::XZ_FIXTURE, &vec![0; padding]].concat();
            assert!(xz(&input, 1000).is_err());
        }
        assert!(xz(&[&[0; 4][..], fx::XZ_FIXTURE].concat(), 1000).is_err());
        assert!(xz(&[fx::XZ_FIXTURE, b"junk"].concat(), 1000).is_err());
        for missing in 1..fx::XZ_FIXTURE.len() {
            let input = [fx::XZ_FIXTURE, &fx::XZ_FIXTURE[..fx::XZ_FIXTURE.len() - missing]].concat();
            assert!(xz(&input, 1000).is_err(), "missing {missing} bytes");
        }
        for at in [fx::XZ_FIXTURE.len() - 13, fx::XZ_FIXTURE.len() - 12] {
            let mut damaged = fx::XZ_FIXTURE.to_vec();
            damaged[at] ^= 1;
            assert!(xz(&damaged, 1000).is_err());
        }
    }

    #[cfg(feature = "lenient-lzma")]
    #[test]
    fn xz_index_sizes_and_integer_encodings_are_checked() {
        assert_eq!(xz_vli(&mut &[0x80, 0][..]), Err(DecompressError::Corrupt));
        assert_eq!(xz_vli(&mut &[0xff; 10][..]), Err(DecompressError::Corrupt));
        assert_eq!(xz_vli(&mut &[0x81, 1][..]), Ok(129));
        let mut damaged = fx::XZ_FIXTURE.to_vec();
        let footer = damaged.len() - 12;
        let backward = u32::from_le_bytes(damaged[footer + 4..footer + 8].try_into().unwrap());
        let index = footer - (backward as usize + 1) * 4;
        damaged[index + 2] = 0;
        let checksum = crate::lenient::crc32(&damaged[index..footer - 4]);
        damaged[footer - 4..footer].copy_from_slice(&checksum.to_le_bytes());
        assert!(xz(&damaged, 1000).is_err());
    }

    #[cfg(feature = "lenient-bzip2")]
    #[test]
    fn bzip2_round_trip() {
        let c = fx::BZIP2_FIXTURE;
        assert_eq!(bzip2(c, 1 << 20).unwrap(), fx::BZIP2_PLAIN);
        assert_eq!(bzip2(c, 2), Err(DecompressError::TooLarge));
        for len in 0..c.len() {
            let _ = bzip2(&c[..len], 1 << 20);
        }
    }

    #[cfg(feature = "lenient-zstd")]
    #[test]
    fn zstd_round_trip() {
        let c = fx::ZSTD_FIXTURE;
        assert_eq!(zstd(c, 1 << 20).unwrap(), fx::ZSTD_PLAIN);
        assert_eq!(zstd(c, 2), Err(DecompressError::TooLarge));
        for len in 0..c.len() {
            let _ = zstd(&c[..len], 1 << 20);
        }
    }

    #[cfg(feature = "lenient-zstd")]
    #[test]
    fn zstandard_members_can_contain_multiple_frames() {
        let frame = fx::ZSTD_FIXTURE;
        let plain = fx::ZSTD_PLAIN;
        let skip = [0x50, 0x2a, 0x4d, 0x18, 3, 0, 0, 0, 1, 2, 3];
        let input = [frame, &skip, frame].concat();
        let expected = [plain, plain].concat();
        assert_eq!(zstd(&input, expected.len() as u64).unwrap(), expected);
        assert_eq!(zstd(&input, plain.len() as u64), Err(DecompressError::TooLarge));
        assert!(zstd(&[frame, &skip[..9]].concat(), u64::MAX).is_err());
        assert!(zstd(&[frame, &frame[..5]].concat(), u64::MAX).is_err());
    }

    #[test]
    fn garbage_never_panics() {
        for seed in 0..64u64 {
            let n = fx::noise(seed, 400);
            #[cfg(feature = "lenient-lzma")]
            let _ = xz(&n, 1 << 20);
            #[cfg(feature = "lenient-bzip2")]
            let _ = bzip2(&n, 1 << 20);
            #[cfg(feature = "lenient-zstd")]
            let _ = zstd(&n, 1 << 20);
            let _ = &n;
        }
    }
}
