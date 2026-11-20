//! Header-only JPEG embedding probes without decoding or re-encoding pixels.
//!
//! Provides bounded marker walking, JFIF density parsing, APP2 ICC reassembly, and SOF/Adobe
//! CMYK detection for passthrough `/DCTDecode` embedding.

use std::fmt;
use std::ops::Range;

/// Control flow returned by a [`walk_jpeg_segments`] visitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegVisit {
    Continue,
    Stop,
}

/// Failure while probing a JPEG header for passthrough embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegProbeError {
    NotJpeg,
    TruncatedMarker,
    TruncatedSegmentLength,
    InvalidSegmentLength,
    SegmentLengthOverflow,
    TruncatedSegment,
    BadFrameHeader,
    NoImageSize,
    UnsupportedComponents(u8),
}

impl JpegProbeError {
    /// Human-readable diagnostic message for this probe error.
    pub fn message(self) -> String {
        match self {
            Self::NotJpeg => "not a JPEG file".into(),
            Self::TruncatedMarker => "truncated JPEG marker".into(),
            Self::TruncatedSegmentLength => "truncated JPEG segment length".into(),
            Self::InvalidSegmentLength => "invalid JPEG segment length".into(),
            Self::SegmentLengthOverflow => "JPEG segment length overflow".into(),
            Self::TruncatedSegment => "truncated JPEG segment".into(),
            Self::BadFrameHeader => "bad frame header".into(),
            Self::NoImageSize => "no image size".into(),
            Self::UnsupportedComponents(n) => format!("{n} colour components are not supported"),
        }
    }
}

impl fmt::Display for JpegProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for JpegProbeError {}

/// Header metadata needed to embed a JPEG stream without re-encoding.
#[derive(Debug, Clone, PartialEq)]
pub struct JpegProbe {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub color_space: &'static str,
    pub invert_cmyk: bool,
    pub dpi: (f64, f64),
    pub icc: Option<Vec<u8>>,
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|x| u16::from_be_bytes([x[0], x[1]]))
}

/// Walk JPEG marker segments before SOS (`0xDA`) or EOI (`0xD9`).
///
/// Calls `visit(marker, segment_span, payload)` for each marker segment with a length field, where
/// `segment_span` covers the `0xFF` marker byte through the end of the segment payload.
pub fn walk_jpeg_segments<'a>(
    bytes: &'a [u8],
    mut visit: impl FnMut(u8, Range<usize>, &'a [u8]) -> Result<JpegVisit, JpegProbeError>,
) -> Result<(), JpegProbeError> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return Err(JpegProbeError::NotJpeg);
    }
    let mut i = 2usize;
    while let Some(&byte) = bytes.get(i) {
        if byte != 0xFF {
            i += 1;
            continue;
        }
        i += 1;
        // Markers may be preceded by any number of 0xFF fill bytes.
        while bytes.get(i) == Some(&0xFF) {
            i += 1;
        }
        let marker_pos = i;
        let marker = *bytes.get(i).ok_or(JpegProbeError::TruncatedMarker)?;
        i += 1;
        // Metadata ends at the first scan; entropy-coded bytes are not segments.
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        let len = usize::from(be16(bytes, i).ok_or(JpegProbeError::TruncatedSegmentLength)?);
        if len < 2 {
            return Err(JpegProbeError::InvalidSegmentLength);
        }
        let end = i.checked_add(len).ok_or(JpegProbeError::SegmentLengthOverflow)?;
        let seg = bytes.get(i + 2..end).ok_or(JpegProbeError::TruncatedSegment)?;
        let span = marker_pos.saturating_sub(1)..end;
        if matches!(visit(marker, span, seg)?, JpegVisit::Stop) {
            break;
        }
        i = end;
    }
    Ok(())
}

/// Parse JFIF APP0 density `(x_dpi, y_dpi)` when unit is inches (1) or centimetres (2) and both
/// axes are positive.
pub fn jfif_dpi(seg: &[u8]) -> Option<(f64, f64)> {
    if !seg.starts_with(b"JFIF\0") || seg.len() < 12 {
        return None;
    }
    let (unit, x, y) = (seg[7], u16::from_be_bytes([seg[8], seg[9]]) as f64, u16::from_be_bytes([seg[10], seg[11]]) as f64);
    if x > 0.0 && y > 0.0 {
        match unit {
            1 => Some((x, y)),
            2 => Some((x * 2.54, y * 2.54)),
            _ => None,
        }
    } else {
        None
    }
}

/// A JPEG's ICC profile from its APP2 `ICC_PROFILE` chunks: (sequence number, chunk count,
/// data). `None` unless every chunk from 1 to the count is there exactly once.
fn jpeg_icc(mut chunks: Vec<(u8, u8, &[u8])>) -> Option<Vec<u8>> {
    let count = chunks.first()?.1;
    chunks.sort_by_key(|c| c.0);
    let complete = usize::from(count) == chunks.len() && chunks.iter().zip(1..=count).all(|(c, seq)| c.0 == seq && c.1 == count);
    complete.then(|| chunks.iter().flat_map(|c| c.2.iter().copied()).collect())
}

/// Whether `marker` is a JPEG SOFn frame marker (excluding DHT `0xC4`, JPG `0xC8`, DAC `0xCC`).
pub fn is_sof_marker(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xCF) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC
}

/// Probe a JPEG for passthrough `/DCTDecode` embedding without decoding or re-encoding pixels.
pub fn probe_jpeg(bytes: &[u8]) -> Result<JpegProbe, JpegProbeError> {
    let (mut size, mut comps, mut dpi, mut adobe) = (None, 0u8, (72.0, 72.0), false);
    let mut icc_chunks = Vec::new();
    walk_jpeg_segments(bytes, |marker, _span, seg| {
        match marker {
            // APP0 JFIF density.
            0xE0 => {
                if let Some(d) = jfif_dpi(seg) {
                    dpi = d;
                }
            }
            0xEE if seg.starts_with(b"Adobe") => adobe = true,
            // APP2 ICC profile, possibly split over several chunks.
            0xE2 => {
                if let Some(([seq, count], data)) = seg.strip_prefix(b"ICC_PROFILE\0").and_then(<[u8]>::split_first_chunk::<2>) {
                    icc_chunks.push((*seq, *count, data));
                }
            }
            _ if is_sof_marker(marker) && size.is_none() => {
                if seg.len() < 6 {
                    return Err(JpegProbeError::BadFrameHeader);
                }
                size = Some((u16::from_be_bytes([seg[3], seg[4]]) as u32, u16::from_be_bytes([seg[1], seg[2]]) as u32));
                comps = seg[5];
            }
            _ => {}
        }
        Ok(JpegVisit::Continue)
    })?;
    let (width, height) = size.filter(|(w, h)| *w > 0 && *h > 0).ok_or(JpegProbeError::NoImageSize)?;
    let (color_space, invert_cmyk) = match comps {
        1 => ("DeviceGray", false),
        3 => ("DeviceRGB", false),
        4 => ("DeviceCMYK", adobe),
        n => return Err(JpegProbeError::UnsupportedComponents(n)),
    };
    let icc = jpeg_icc(icc_chunks);
    Ok(JpegProbe { width, height, components: comps, color_space, invert_cmyk, dpi, icc })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_probe_preserves_error_diagnostics() {
        assert_eq!(probe_jpeg(b"\x89PNG").unwrap_err(), JpegProbeError::NotJpeg);
        assert_eq!(probe_jpeg(&[0xFF, 0xD8, 0xFF, 0xFF]).unwrap_err(), JpegProbeError::TruncatedMarker);
        assert_eq!(probe_jpeg(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap_err(), JpegProbeError::TruncatedSegmentLength);
        assert_eq!(probe_jpeg(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 1]).unwrap_err(), JpegProbeError::InvalidSegmentLength);
        assert_eq!(probe_jpeg(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 5, 1]).unwrap_err(), JpegProbeError::TruncatedSegment);
        assert_eq!(probe_jpeg(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 4, 8, 0]).unwrap_err(), JpegProbeError::BadFrameHeader);
        assert_eq!(probe_jpeg(&[0xFF, 0xD8, 0xFF, 0xD9]).unwrap_err(), JpegProbeError::NoImageSize);
        let two_comp = [0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8, 0, 2, 0, 3, 2, 1, 0x11, 0, 2, 0x11, 0];
        assert_eq!(probe_jpeg(&two_comp).unwrap_err(), JpegProbeError::UnsupportedComponents(2));
    }
}
