//! Header-only JPEG 2000 (JP2 box container and raw J2K codestream) embedding probes without
//! decoding or re-encoding pixels.

use std::fmt;

/// Signature of a JP2 file box stream.
pub const JP2_SIGNATURE: &[u8] = &[0, 0, 0, 0x0C, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

/// Signature of a raw JPEG 2000 codestream (SOC + SIZ).
pub const J2K_CODESTREAM_SIGNATURE: &[u8] = &[0xFF, 0x4F, 0xFF, 0x51];

/// Whether `bytes` begins with a JP2 file signature or a raw JPEG 2000 codestream signature.
pub fn is_jpx(bytes: &[u8]) -> bool {
    bytes.starts_with(JP2_SIGNATURE) || bytes.starts_with(J2K_CODESTREAM_SIGNATURE)
}

/// Header metadata needed to embed a JPEG 2000 stream without re-encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JpxProbe {
    pub width: u32,
    pub height: u32,
    pub dpi: (f64, f64),
}

/// Failure while probing a JPEG 2000 header for passthrough embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpxProbeError {
    MissingHeader,
    MissingImageSize,
    BadImageHeader,
    DamagedCodestream,
    UnusualImageSize,
}

impl JpxProbeError {
    /// Human-readable diagnostic message for this probe error.
    pub fn message(self) -> &'static str {
        match self {
            Self::MissingHeader => "a JPEG 2000 file without a header",
            Self::MissingImageSize => "a JPEG 2000 file without image size",
            Self::BadImageHeader => "bad image header",
            Self::DamagedCodestream => "a damaged JPEG 2000 codestream",
            Self::UnusualImageSize => "unusual JPEG 2000 image size",
        }
    }
}

impl fmt::Display for JpxProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for JpxProbeError {}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|x| u16::from_be_bytes([x[0], x[1]]))
}

/// The boxes of a JP2 box sequence: (type, payload).
fn jp2_boxes(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 8 <= b.len() {
        let len = be32(b, i).unwrap_or(0) as u64;
        let ty = [b[i + 4], b[i + 5], b[i + 6], b[i + 7]];
        let (head, len) = match len {
            0 => (8, (b.len() - i) as u64),
            1 => match b.get(i + 8..i + 16) {
                Some(x) => (16, u64::from_be_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]])),
                None => break,
            },
            n => (8, n),
        };
        // `len as usize` would truncate on 32-bit targets and put the end before the payload.
        let end = i.saturating_add(usize::try_from(len).unwrap_or(usize::MAX)).min(b.len());
        if len < head as u64 || end <= i {
            break;
        }
        out.push((ty, &b[i + head..end]));
        i = end;
    }
    out
}

/// Probe a JPEG 2000 image (JP2 file or raw codestream) for passthrough `/JPXDecode` embedding
/// without decoding or re-encoding pixels.
pub fn probe_jpx(bytes: &[u8]) -> Result<JpxProbe, JpxProbeError> {
    let mut dpi = (72.0, 72.0);
    let (width, height) = if bytes.starts_with(JP2_SIGNATURE) {
        let header = jp2_boxes(bytes).into_iter().find(|(t, _)| t == b"jp2h").map(|(_, p)| p).ok_or(JpxProbeError::MissingHeader)?;
        let inner = jp2_boxes(header);
        let ihdr = inner.iter().find(|(t, _)| t == b"ihdr").map(|(_, p)| *p).ok_or(JpxProbeError::MissingImageSize)?;
        // Capture resolution (pixels per metre), when given.
        if let Some((_, res)) = inner.iter().find(|(t, _)| t == b"res ")
            && let Some((_, r)) = jp2_boxes(res).into_iter().find(|(t, _)| t == b"resc" || t == b"resd")
            && r.len() >= 10
        {
            let part = |n: u16, d: u16, e: i8| if d == 0 { 0.0 } else { n as f64 / d as f64 * 10f64.powi(e as i32) * 0.0254 };
            let (v, hz) = (
                part(be16(r, 0).unwrap_or(0), be16(r, 2).unwrap_or(0), r[8] as i8),
                part(be16(r, 4).unwrap_or(0), be16(r, 6).unwrap_or(0), r[9] as i8),
            );
            if v > 1.0 && hz > 1.0 {
                dpi = (hz, v);
            }
        }
        (be32(ihdr, 4).ok_or(JpxProbeError::BadImageHeader)?, be32(ihdr, 0).ok_or(JpxProbeError::BadImageHeader)?)
    } else {
        // SIZ: Lsiz Rsiz Xsiz Ysiz XOsiz YOsiz …
        let (x, y, xo, yo) = (be32(bytes, 8), be32(bytes, 12), be32(bytes, 16), be32(bytes, 20));
        match (x, y, xo, yo) {
            (Some(x), Some(y), Some(xo), Some(yo)) if x > xo && y > yo => (x - xo, y - yo),
            _ => return Err(JpxProbeError::DamagedCodestream),
        }
    };
    if width == 0 || height == 0 || width > 100_000 || height > 100_000 {
        return Err(JpxProbeError::UnusualImageSize);
    }
    Ok(JpxProbe { width, height, dpi })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn jpx_probe_preserves_bounds_and_diagnostics() {
        let bx = |ty: &[u8], payload: &[u8]| {
            let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(ty);
            v.extend_from_slice(payload);
            v
        };
        assert_eq!(probe_jpx(JP2_SIGNATURE).unwrap_err(), JpxProbeError::MissingHeader);
        let no_ihdr = [JP2_SIGNATURE.to_vec(), bx(b"jp2h", &[])].concat();
        assert_eq!(probe_jpx(&no_ihdr).unwrap_err(), JpxProbeError::MissingImageSize);
        let bad_ihdr = [JP2_SIGNATURE.to_vec(), bx(b"jp2h", &bx(b"ihdr", &[0, 0, 0, 1]))].concat();
        assert_eq!(probe_jpx(&bad_ihdr).unwrap_err(), JpxProbeError::BadImageHeader);
        assert_eq!(probe_jpx(&[0xFF, 0x4F, 0xFF, 0x51, 0, 41]).unwrap_err(), JpxProbeError::DamagedCodestream);

        let mut huge = vec![0xFF, 0x4F, 0xFF, 0x51, 0, 41, 0, 0];
        for v in [100_001u32, 10, 0, 0] {
            huge.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(probe_jpx(&huge).unwrap_err(), JpxProbeError::UnusualImageSize);

        // Codestream with non-zero XOsiz/YOsiz offsets.
        let mut offset_siz = vec![0xFF, 0x4F, 0xFF, 0x51, 0, 41, 0, 0];
        for v in [100u32, 80, 20, 10] {
            offset_siz.extend_from_slice(&v.to_be_bytes());
        }
        let p = probe_jpx(&offset_siz).unwrap();
        assert_eq!((p.width, p.height, p.dpi), (80, 70, (72.0, 72.0)));
    }
}
