//! What the reference app's pixel records share: a bare `QVariantMap` header followed by the pixel payload, which is
//! an LZ4 *block* (no frame) or raw bytes. Used by [`super::image`] and [`super::texture_cache`].

use super::DecodeError;
use super::image::Compression;
use crate::container::qds::{QDataStream, QVariant, QVariantMap};

/// Largest decompressed payload accepted: an 8K RGBA32F texture. Anything larger is treated as corrupt.
pub(super) const MAX_RAW_SIZE: usize = 8192 * 8192 * 16;
/// LZ4 cannot expand input by more than ~255x; a larger claim means the header is corrupt.
const LZ4_MAX_RATIO: usize = 255;

/// The header map of one record, with typed field access that names the record in errors.
pub(super) struct RecordHeader {
    record: &'static str,
    map: QVariantMap,
    len: usize,
}

impl RecordHeader {
    /// Reads the header map at the start of `data`; `data` only needs to hold the header itself.
    pub fn read(record: &'static str, data: &[u8]) -> Result<Self, DecodeError> {
        let mut stream = QDataStream::new(data);
        let map = stream.read_variant_map().map_err(DecodeError::qds(record))?;
        Ok(RecordHeader { record, map, len: stream.position() })
    }

    /// Byte length of the header, i.e. where the payload (or its length prefix) starts.
    pub fn len(&self) -> usize {
        self.len
    }

    fn field(&self, name: &'static str) -> Result<&QVariant, DecodeError> {
        self.map.get(name).ok_or(DecodeError::MissingField { record: self.record, field: name })
    }

    /// An integer field of any QVariant integer type (the reference app switched some from uint to int over time).
    pub fn integer(&self, name: &'static str) -> Result<i64, DecodeError> {
        let value = self.field(name)?;
        value.as_i64().ok_or_else(|| DecodeError::invalid(self.record, format!("field '{name}' is not an integer: {value:?}")))
    }

    pub fn u32(&self, name: &'static str) -> Result<u32, DecodeError> {
        let v = self.integer(name)?;
        u32::try_from(v).map_err(|_| DecodeError::invalid(self.record, format!("field '{name}' out of range: {v}")))
    }

    pub fn size(&self, name: &'static str) -> Result<usize, DecodeError> {
        let v = self.integer(name)?;
        usize::try_from(v).map_err(|_| DecodeError::invalid(self.record, format!("field '{name}' out of range: {v}")))
    }

    /// `Ok(None)` when the field is absent.
    pub fn optional_size(&self, name: &'static str) -> Result<Option<usize>, DecodeError> {
        match self.map.get(name) {
            Some(_) => self.size(name).map(Some),
            None => Ok(None),
        }
    }

    pub fn field_value(&self, name: &'static str) -> Result<&QVariant, DecodeError> {
        self.field(name)
    }

    /// The `compression` field: `lz4`, or `raw` for uncompressed pixels.
    pub fn compression(&self) -> Result<Compression, DecodeError> {
        match self.field("compression")?.as_str() {
            Some("lz4") => Ok(Compression::Lz4),
            Some("raw") | Some("none") => Ok(Compression::None),
            Some("jp2") => Ok(Compression::Jp2),
            other => Err(DecodeError::invalid(self.record, format!("unknown compression {other:?}"))),
        }
    }

    /// A decompressed size, refused above [`MAX_RAW_SIZE`].
    pub fn raw_size(&self, name: &'static str) -> Result<usize, DecodeError> {
        let size = self.size(name)?;
        if size > MAX_RAW_SIZE {
            return Err(DecodeError::invalid(self.record, format!("'{name}' of {size} bytes exceeds the {MAX_RAW_SIZE} byte limit")));
        }
        Ok(size)
    }
}

/// `size` bytes of `data` starting at `start`; an error if the data is shorter. Trailing bytes are ignored.
pub(super) fn take_payload<'a>(record: &'static str, data: &'a [u8], start: usize, size: usize) -> Result<&'a [u8], DecodeError> {
    let payload = start.checked_add(size).and_then(|end| data.get(start..end));
    match payload {
        Some(payload) => Ok(payload),
        None => Err(DecodeError::invalid(record, format!("payload of {size} bytes at offset {start} does not fit in {} bytes", data.len()))),
    }
}

/// Pixel bytes padded to the declared size, and how many bytes at the end the stored block did not cover.
/// The reference app occasionally writes an LZ4 block that is slightly short (seen once in the corpus: an 8K image
/// 22,725 bytes short); importers can accept that with a warning instead of failing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decompressed {
    pub bytes: Vec<u8>,
    pub missing: usize,
}

/// Decompresses `payload` into exactly `raw_size` bytes, validating sizes before allocating.
pub(super) fn decompress(record: &'static str, compression: Compression, raw_size: usize, payload: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let d = decompress_padded(record, compression, raw_size, payload)?;
    if d.missing > 0 {
        return Err(DecodeError::invalid(record, format!("LZ4 block expanded to {} bytes, header says {raw_size}", raw_size - d.missing)));
    }
    Ok(d.bytes)
}

/// Like [`decompress`], but a block that decodes cleanly to fewer bytes than declared is zero-padded and the
/// shortfall reported. Corrupt blocks and wrong uncompressed sizes are still errors.
pub(super) fn decompress_padded(record: &'static str, compression: Compression, raw_size: usize, payload: &[u8]) -> Result<Decompressed, DecodeError> {
    if raw_size > MAX_RAW_SIZE {
        return Err(DecodeError::invalid(record, format!("raw size {raw_size} exceeds the limit")));
    }
    match compression {
        Compression::None if payload.len() == raw_size => Ok(Decompressed { bytes: payload.to_vec(), missing: 0 }),
        Compression::None => Err(DecodeError::invalid(record, format!("uncompressed payload is {} bytes, header says {raw_size}", payload.len()))),
        Compression::Jp2 => Err(DecodeError::invalid(record, "JPEG 2000 where only LZ4 or raw pixels are possible")),
        Compression::Lz4 => decompress_lz4(record, raw_size, payload),
    }
}

fn decompress_lz4(record: &'static str, raw_size: usize, payload: &[u8]) -> Result<Decompressed, DecodeError> {
    if raw_size > payload.len().saturating_mul(LZ4_MAX_RATIO).saturating_add(16) {
        return Err(DecodeError::invalid(record, format!("{} LZ4 bytes cannot expand to {raw_size} bytes", payload.len())));
    }
    let mut bytes = vec![0u8; raw_size];
    let written = lz4_flex::block::decompress_into(payload, &mut bytes).map_err(|e| DecodeError::invalid(record, format!("LZ4 block: {e}")))?;
    Ok(Decompressed { bytes, missing: raw_size - written })
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: &str = "test record";

    #[test]
    fn lz4_round_trip_with_exact_size() {
        let raw: Vec<u8> = (0..4096u32).map(|i| (i % 7) as u8).collect();
        let packed = lz4_flex::block::compress(&raw);
        assert_eq!(decompress(R, Compression::Lz4, raw.len(), &packed), Ok(raw.clone()));
        assert!(decompress(R, Compression::Lz4, raw.len() + 1, &packed).is_err());
        assert!(decompress(R, Compression::Lz4, raw.len() - 1, &packed).is_err());
    }

    #[test]
    fn a_short_block_is_an_error_strictly_and_padded_leniently() {
        let raw: Vec<u8> = (0..4096u32).map(|i| (i % 7) as u8).collect();
        let packed = lz4_flex::block::compress(&raw);
        let d = decompress_padded(R, Compression::Lz4, raw.len() + 10, &packed).unwrap();
        assert_eq!((d.missing, &d.bytes[..raw.len()], &d.bytes[raw.len()..]), (10, &raw[..], &[0u8; 10][..]));
        assert!(decompress(R, Compression::Lz4, raw.len() + 10, &packed).is_err());
    }

    #[test]
    fn corrupt_or_oversized_claims_fail_without_allocating() {
        assert!(decompress(R, Compression::Lz4, MAX_RAW_SIZE + 1, &[0; 16]).is_err());
        assert!(decompress(R, Compression::Lz4, 1 << 20, &[0xF0; 8]).is_err());
        assert!(decompress(R, Compression::Lz4, 64, &[0xFF; 64]).is_err());
        assert!(decompress(R, Compression::None, 3, &[1, 2]).is_err());
        assert!(decompress_padded(R, Compression::None, 3, &[1, 2]).is_err());
        assert_eq!(decompress(R, Compression::None, 2, &[1, 2]), Ok(vec![1, 2]));
    }

    #[test]
    fn payload_bounds_are_checked() {
        assert_eq!(take_payload(R, &[1, 2, 3, 4], 1, 2), Ok(&[2, 3][..]));
        assert!(take_payload(R, &[1, 2, 3], 2, 2).is_err());
        assert!(take_payload(R, &[1, 2, 3], usize::MAX, 2).is_err());
    }
}
