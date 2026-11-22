//! Image resources under `/resources` (`*.image` and unnamed hashes): a QVariantMap header followed by pixels.
//!
//! Header fields (Verified, version 6.2–9.0): `width`/`height` (uint), `pitch` (qlonglong, bytes per row),
//! `format` (string; an int in rare `v1` headers), `colorSpace`, `alphaFormat` (int), `flags` (uint),
//! `compression` (`lz4` or `raw`), `dataSizeRaw` (qulonglong), `dataSizeLZ4` (qulonglong, absent when raw),
//! `ICCdataSize` (uint, only in newer files, always 0 so far), `version` (`v1`/`v2`). Version 12.2 also stores `jp2`,
//! with `tilesCount` (uint) instead of `dataSizeLZ4`, and the payload runs to the end of the record.
//! The payload follows the header directly.

use super::DecodeError;
use super::Decompressed;
use super::lz4_payload::{self, RecordHeader};
use crate::container::qds::QVariant;

const RECORD: &str = "resource image";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
    Rgba16,
    Rgba16F,
    Rgba32F,
    /// One 8-bit channel.
    Lum8,
    /// One 16-bit channel (baked mesh maps such as AO and curvature).
    Lum16,
    /// A format name we have not mapped yet, kept verbatim (an unmapped `v1` format number is kept as text).
    Other(String),
}

impl PixelFormat {
    pub fn from_name(name: &str) -> Self {
        match name {
            "RGBA8" => PixelFormat::Rgba8,
            "BGRA8" => PixelFormat::Bgra8,
            "RGBA16" => PixelFormat::Rgba16,
            "RGBA16F" => PixelFormat::Rgba16F,
            "RGBA32F" => PixelFormat::Rgba32F,
            "LUM8" => PixelFormat::Lum8,
            "LUM16" => PixelFormat::Lum16,
            other => PixelFormat::Other(other.to_string()),
        }
    }

    /// The name the reference app uses for this format.
    pub fn name(&self) -> &str {
        match self {
            PixelFormat::Rgba8 => "RGBA8",
            PixelFormat::Bgra8 => "BGRA8",
            PixelFormat::Rgba16 => "RGBA16",
            PixelFormat::Rgba16F => "RGBA16F",
            PixelFormat::Rgba32F => "RGBA32F",
            PixelFormat::Lum8 => "LUM8",
            PixelFormat::Lum16 => "LUM16",
            PixelFormat::Other(name) => name,
        }
    }

    /// Bytes per pixel, if known.
    pub fn bytes_per_pixel(&self) -> Option<usize> {
        match self {
            PixelFormat::Lum8 => Some(1),
            PixelFormat::Lum16 => Some(2),
            PixelFormat::Rgba8 | PixelFormat::Bgra8 => Some(4),
            PixelFormat::Rgba16 | PixelFormat::Rgba16F => Some(8),
            PixelFormat::Rgba32F => Some(16),
            PixelFormat::Other(_) => None,
        }
    }

    fn from_variant(value: &QVariant) -> Option<Self> {
        match (value.as_str(), value.as_i64()) {
            (Some(name), _) => Some(Self::from_name(name)),
            // `v1` headers number the formats; 4 holds 2-byte little-endian grey values (Observed).
            (None, Some(4)) => Some(PixelFormat::Lum16),
            (None, Some(number)) => Some(PixelFormat::Other(number.to_string())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Lz4,
    /// Tiles of JPEG 2000 (`tilesCount` of them; see [`super::jp2_tiles`]).
    Jp2,
}

/// What an image's header says, readable from a prefix of the record without the payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageHeader {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub compression: Compression,
    /// Size of the pixel data after decompression (rows are tightly packed).
    pub raw_size: usize,
    /// Size of the stored payload that follows the header.
    pub payload_size: usize,
}

impl ImageHeader {
    /// Parses the header from the start of a record; the payload need not be present.
    pub fn parse(prefix: &[u8]) -> Result<ImageHeader, DecodeError> {
        Self::parse_with_len(prefix).map(|(header, _)| header)
    }

    /// The header and its byte length (where the payload starts).
    fn parse_with_len(data: &[u8]) -> Result<(ImageHeader, usize), DecodeError> {
        let h = RecordHeader::read(RECORD, data)?;
        let width = h.u32("width")?;
        let height = h.u32("height")?;
        let format_value = h.field_value("format")?;
        let format =
            PixelFormat::from_variant(format_value).ok_or_else(|| DecodeError::invalid(RECORD, format!("unreadable format {format_value:?}")))?;
        let compression = h.compression()?;
        let raw_size = h.raw_size("dataSizeRaw")?;
        let payload_size = match compression {
            Compression::Lz4 => h.size("dataSizeLZ4")?,
            Compression::None => raw_size,
            Compression::Jp2 => data.len().saturating_sub(h.len()),
        };
        if h.optional_size("ICCdataSize")?.unwrap_or(0) != 0 {
            // Never seen in the corpus, so where the profile sits relative to the pixels is unknown.
            return Err(DecodeError::invalid(RECORD, "embedded ICC profiles are not supported"));
        }
        let header = ImageHeader { width, height, format, compression, raw_size, payload_size };
        header.check_layout(h.size("pitch")?)?;
        Ok((header, h.len()))
    }

    /// Rows must be tightly packed and `dataSizeRaw` must cover exactly `height` rows.
    fn check_layout(&self, pitch: usize) -> Result<(), DecodeError> {
        let expected_raw = pitch.checked_mul(self.height as usize);
        if expected_raw != Some(self.raw_size) {
            return Err(DecodeError::invalid(RECORD, format!("pitch {pitch} x height {} does not match raw size {}", self.height, self.raw_size)));
        }
        let row = self.format.bytes_per_pixel().map(|bpp| bpp * self.width as usize);
        if row.is_some_and(|row| row != pitch) {
            return Err(DecodeError::invalid(RECORD, format!("pitch {pitch} is not width {} x {} bytes", self.width, self.format.name())));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageResource<'a> {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub compression: Compression,
    raw_size: usize,
    payload: &'a [u8],
    /// `tilesCount` of a JPEG 2000 record.
    tiles: usize,
}

impl<'a> ImageResource<'a> {
    /// Parses a complete record; fails if the payload is shorter than the header declares.
    pub fn parse(data: &'a [u8]) -> Result<Self, DecodeError> {
        let (header, header_len) = ImageHeader::parse_with_len(data)?;
        let payload = lz4_payload::take_payload(RECORD, data, header_len, header.payload_size)?;
        let tiles = match header.compression {
            Compression::Jp2 => RecordHeader::read(RECORD, data)?.size("tilesCount")?,
            _ => 0,
        };
        Ok(ImageResource {
            width: header.width,
            height: header.height,
            format: header.format,
            compression: header.compression,
            raw_size: header.raw_size,
            payload,
            tiles,
        })
    }

    pub fn header(&self) -> ImageHeader {
        ImageHeader {
            width: self.width,
            height: self.height,
            format: self.format.clone(),
            compression: self.compression,
            raw_size: self.raw_size,
            payload_size: self.payload.len(),
        }
    }

    fn decode_tiles(&self) -> Result<Vec<u8>, DecodeError> {
        let (w, h) = (self.width as usize, self.height as usize);
        super::jp2_tiles::decode(self.payload, self.tiles, w, h, &self.format, self.raw_size)
    }

    /// Size of the pixel data after decompression.
    pub fn raw_size(&self) -> usize {
        self.raw_size
    }

    /// The uncompressed pixel bytes, checked against the size the header declares.
    pub fn decompress(&self) -> Result<Vec<u8>, DecodeError> {
        if self.compression == Compression::Jp2 {
            return self.decode_tiles();
        }
        lz4_payload::decompress(RECORD, self.compression, self.raw_size, self.payload)
    }

    /// The pixel bytes, zero-padded if the stored block is short (see [`Decompressed`]). For importing.
    pub fn decompress_padded(&self) -> Result<Decompressed, DecodeError> {
        if self.compression == Compression::Jp2 {
            return Ok(Decompressed { bytes: self.decode_tiles()?, missing: 0 });
        }
        lz4_payload::decompress_padded(RECORD, self.compression, self.raw_size, self.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::qds::test_encoder::QdsWriter as W;

    /// A synthetic 2x2 LUM16 record (LZ4 unless `raw`) and its pixels.
    fn record(raw: bool, pitch: i64, extra: &[(&str, Vec<u8>)]) -> (Vec<u8>, Vec<u8>) {
        let pixels: Vec<u8> = (0..8).collect();
        let payload = if raw { pixels.clone() } else { lz4_flex::block::compress(&pixels) };
        let mut fields = vec![
            ("width", W::encode(|w| w.uint(2))),
            ("version", W::encode(|w| w.string("v2"))),
            ("pitch", W::encode(|w| w.long_long(pitch))),
            ("height", W::encode(|w| w.uint(2))),
            ("format", W::encode(|w| w.string("LUM16"))),
            ("dataSizeRaw", W::encode(|w| w.ulong_long(8))),
            ("compression", W::encode(|w| w.string(if raw { "raw" } else { "lz4" }))),
        ];
        if !raw {
            fields.push(("dataSizeLZ4", W::encode(|w| w.ulong_long(payload.len() as u64))));
        }
        fields.extend(extra.iter().cloned());
        let mut w = W::new();
        w.raw_map(&fields).raw_bytes(&payload);
        (w.finish(), pixels)
    }

    #[test]
    fn decodes_lz4_and_raw_records() {
        for raw in [false, true] {
            let (data, pixels) = record(raw, 4, &[]);
            let image = ImageResource::parse(&data).unwrap();
            assert_eq!((image.width, image.height, &image.format), (2, 2, &PixelFormat::Lum16));
            assert_eq!(image.raw_size(), 8);
            assert_eq!(image.decompress().unwrap(), pixels);
            assert_eq!(image.header(), ImageHeader::parse(&data).unwrap());
        }
    }

    #[test]
    fn header_parses_from_a_prefix_but_full_parse_needs_the_payload() {
        let (data, _) = record(false, 4, &[]);
        let header = ImageHeader::parse(&data[..data.len() - 3]).unwrap();
        assert_eq!(header.compression, Compression::Lz4);
        assert!(ImageResource::parse(&data[..data.len() - 1]).is_err());
    }

    #[test]
    fn format_names_and_numbers() {
        let number = |n| PixelFormat::from_variant(&QVariant::Int(n));
        assert_eq!(number(4), Some(PixelFormat::Lum16));
        assert_eq!(number(7), Some(PixelFormat::Other("7".into())));
        assert_eq!(PixelFormat::from_variant(&QVariant::Bool(true)), None);
        assert_eq!(PixelFormat::from_name("RGBA16F").bytes_per_pixel(), Some(8));
        assert_eq!(PixelFormat::from_name("XYZ").bytes_per_pixel(), None);
        assert_eq!(PixelFormat::from_name("BGRA8").name(), "BGRA8");
    }

    #[test]
    fn inconsistent_headers_are_rejected() {
        let no_icc = [("ICCdataSize", W::encode(|w| w.uint(0)))];
        assert!(ImageHeader::parse(&record(false, 4, &no_icc).0).is_ok());
        let icc = [("ICCdataSize", W::encode(|w| w.uint(10)))];
        assert!(ImageHeader::parse(&record(false, 4, &icc).0).is_err());
        assert!(ImageHeader::parse(&record(false, 6, &[]).0).is_err());
        assert!(ImageHeader::parse(&record(false, -1, &[]).0).is_err());
        assert!(ImageHeader::parse(&[0, 0, 0, 1]).is_err());
    }
}
