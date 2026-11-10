//! Rendered textures under `/texture-cache/<a>/<b>/<n>`: a QVariantMap header followed by pixels.
//!
//! Layout (Verified, version 2019–9.0): header map with `width`, `height` (uint), `internalFormat`
//! (OpenGL enum; uint before 2023, int since), `compression` (`lz4`), `bufferSize` (uint, decompressed
//! size), `version` (`v1`); then a big-endian u32 with the payload length, then the LZ4 block.

use super::DecodeError;
use super::Decompressed;
use super::image::Compression;
use super::lz4_payload::{self, RecordHeader};
use crate::container::bytes::ByteReader;

const RECORD: &str = "texture cache entry";

/// OpenGL internal formats: (enum, name, bytes per pixel). The first four occur in the original's cache.
const GL_FORMATS: &[(u32, &str, usize)] = &[
    (0x8C43, "SRGB8_ALPHA8", 4),
    (0x822B, "RG8", 2),
    (0x822F, "RG16F", 4),
    (0x881A, "RGBA16F", 8),
    (0x8058, "RGBA8", 4),
    (0x805B, "RGBA16", 8),
    (0x8814, "RGBA32F", 16),
    (0x8229, "R8", 1),
    (0x822A, "R16", 2),
    (0x822D, "R16F", 2),
    (0x822E, "R32F", 4),
    (0x822C, "RG16", 4),
    (0x8230, "RG32F", 8),
];

fn gl_format(value: u32) -> Option<&'static (u32, &'static str, usize)> {
    GL_FORMATS.iter().find(|(v, _, _)| *v == value)
}

/// What a cache entry's header says, readable from a prefix of the record without the payload.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheHeader {
    pub width: u32,
    pub height: u32,
    /// OpenGL internal format enum, e.g. 35907 = GL_SRGB8_ALPHA8.
    pub gl_internal_format: u32,
    pub compression: Compression,
    /// Size of the pixel data after decompression (`bufferSize`).
    pub raw_size: usize,
    /// Size of the stored payload after its length prefix.
    pub payload_size: usize,
}

impl CacheHeader {
    /// Parses the header and the payload length prefix; the payload need not be present.
    pub fn parse(prefix: &[u8]) -> Result<CacheHeader, DecodeError> {
        Self::parse_with_len(prefix).map(|(header, _)| header)
    }

    /// Readable name of the GL format, e.g. "SRGB8_ALPHA8", or "unknown".
    pub fn gl_format_name(&self) -> &'static str {
        gl_format(self.gl_internal_format).map_or("unknown", |(_, name, _)| name)
    }

    /// Bytes per pixel of the GL format, if known.
    pub fn bytes_per_pixel(&self) -> Option<usize> {
        gl_format(self.gl_internal_format).map(|(_, _, bpp)| *bpp)
    }

    /// The header and the offset where the payload starts (after the length prefix).
    fn parse_with_len(data: &[u8]) -> Result<(CacheHeader, usize), DecodeError> {
        let h = RecordHeader::read(RECORD, data)?;
        let mut prefix = ByteReader::new(data);
        prefix.seek(h.len()).map_err(DecodeError::truncated(RECORD))?;
        let payload_size = prefix.u32_be().map_err(DecodeError::truncated(RECORD))? as usize;
        let header = CacheHeader {
            width: h.u32("width")?,
            height: h.u32("height")?,
            gl_internal_format: h.u32("internalFormat")?,
            compression: h.compression()?,
            raw_size: h.raw_size("bufferSize")?,
            payload_size,
        };
        header.check_size()?;
        Ok((header, prefix.pos()))
    }

    /// For known formats, `bufferSize` must be exactly width x height x bytes per pixel.
    fn check_size(&self) -> Result<(), DecodeError> {
        let expected = self.bytes_per_pixel().and_then(|bpp| (self.width as usize).checked_mul(self.height as usize)?.checked_mul(bpp));
        match expected {
            Some(expected) if expected != self.raw_size => {
                Err(DecodeError::invalid(RECORD, format!("bufferSize {} is not {}x{} {}", self.raw_size, self.width, self.height, self.gl_format_name())))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CacheTexture<'a> {
    pub width: u32,
    pub height: u32,
    /// OpenGL internal format enum, e.g. 35907 = GL_SRGB8_ALPHA8.
    pub gl_internal_format: u32,
    pub compression: Compression,
    raw_size: usize,
    payload: &'a [u8],
}

impl<'a> CacheTexture<'a> {
    /// Parses a complete record; fails if the payload is shorter than its length prefix declares.
    pub fn parse(data: &'a [u8]) -> Result<Self, DecodeError> {
        let (header, start) = CacheHeader::parse_with_len(data)?;
        let payload = lz4_payload::take_payload(RECORD, data, start, header.payload_size)?;
        Ok(CacheTexture {
            width: header.width,
            height: header.height,
            gl_internal_format: header.gl_internal_format,
            compression: header.compression,
            raw_size: header.raw_size,
            payload,
        })
    }

    pub fn header(&self) -> CacheHeader {
        CacheHeader {
            width: self.width,
            height: self.height,
            gl_internal_format: self.gl_internal_format,
            compression: self.compression,
            raw_size: self.raw_size,
            payload_size: self.payload.len(),
        }
    }

    /// Readable name of the GL format, e.g. "SRGB8_ALPHA8", or "unknown".
    pub fn gl_format_name(&self) -> &'static str {
        self.header().gl_format_name()
    }

    pub fn raw_size(&self) -> usize {
        self.raw_size
    }

    pub fn decompress(&self) -> Result<Vec<u8>, DecodeError> {
        lz4_payload::decompress(RECORD, self.compression, self.raw_size, self.payload)
    }

    /// The pixel bytes, zero-padded if the stored block is short (see [`Decompressed`]). For importing.
    pub fn decompress_padded(&self) -> Result<Decompressed, DecodeError> {
        lz4_payload::decompress_padded(RECORD, self.compression, self.raw_size, self.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::qds::test_encoder::QdsWriter as W;

    /// A synthetic 2x1 RG8 entry; `int_format` picks the post-2023 int encoding of `internalFormat`.
    fn entry(int_format: bool, buffer_size: u32) -> (Vec<u8>, Vec<u8>) {
        let pixels = vec![10, 20, 30, 40];
        let payload = lz4_flex::block::compress(&pixels);
        let format = if int_format { W::encode(|w| w.int(0x822B)) } else { W::encode(|w| w.uint(0x822B)) };
        let mut w = W::new();
        w.raw_map(&[
            ("width", W::encode(|w| w.uint(2))),
            ("version", W::encode(|w| w.string("v1"))),
            ("internalFormat", format),
            ("height", W::encode(|w| w.uint(1))),
            ("compression", W::encode(|w| w.string("lz4"))),
            ("bufferSize", W::encode(|w| w.uint(buffer_size))),
        ])
        .raw_u32(payload.len() as u32)
        .raw_bytes(&payload);
        (w.finish(), pixels)
    }

    #[test]
    fn decodes_entries_with_int_or_uint_format() {
        for int_format in [false, true] {
            let (data, pixels) = entry(int_format, 4);
            let tex = CacheTexture::parse(&data).unwrap();
            assert_eq!((tex.width, tex.height, tex.gl_internal_format), (2, 1, 33323));
            assert_eq!(tex.gl_format_name(), "RG8");
            assert_eq!(tex.decompress().unwrap(), pixels);
            assert_eq!(tex.header(), CacheHeader::parse(&data).unwrap());
        }
    }

    #[test]
    fn header_needs_only_the_prefix() {
        let (data, _) = entry(false, 4);
        let header = CacheHeader::parse(&data[..data.len() - 2]).unwrap();
        assert_eq!(header.bytes_per_pixel(), Some(2));
        assert!(CacheTexture::parse(&data[..data.len() - 1]).is_err());
    }

    #[test]
    fn size_mismatch_and_truncation_are_errors() {
        assert!(CacheHeader::parse(&entry(false, 6).0).is_err());
        let (data, _) = entry(false, 4);
        let header_only = CacheHeader::parse(&data).unwrap();
        let cut = data.len() - header_only.payload_size - 2;
        assert!(CacheHeader::parse(&data[..cut]).is_err());
    }

    #[test]
    fn unknown_formats_have_no_name() {
        let header = CacheHeader { width: 1, height: 1, gl_internal_format: 1, compression: Compression::Lz4, raw_size: 1, payload_size: 1 };
        assert_eq!(header.gl_format_name(), "unknown");
        assert_eq!(header.bytes_per_pixel(), None);
    }
}
