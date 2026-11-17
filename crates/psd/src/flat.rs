//! Flat PSD export from borrowed interleaved samples, without an image-sized staging buffer.
use crate::{PsdError, Result};

/// Display-encoded samples, interleaved by pixel.
#[derive(Clone, Copy)]
pub enum Samples<'a> {
    /// Eight-bit samples.
    U8(&'a [u8]),
    /// Sixteen-bit samples.
    U16(&'a [u16]),
}

/// Optional resources embedded in a flat PSD.
#[derive(Clone, Copy, Default)]
pub struct Metadata<'a> {
    /// ICC profile bytes.
    pub icc: Option<&'a [u8]>,
    /// TIFF-structured EXIF bytes.
    pub exif: Option<&'a [u8]>,
    /// XMP document text.
    pub xmp: Option<&'a str>,
    /// Resolution in pixels per inch.
    pub ppi: Option<u16>,
}

/// Serialize a flat PSD. Input channels are gray, gray-alpha, RGB or RGBA; alpha is discarded.
/// Eight-bit rows use PackBits; sixteen-bit samples are stored raw and big-endian. The output
/// buffer and one eight-bit row are held at once; there is no image-sized pixel copy.
pub fn write(width: u32, height: u32, channels: u8, samples: Samples<'_>, meta: Metadata<'_>) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || width > 30_000 || height > 30_000 || !(1..=4).contains(&channels) {
        return Err(PsdError::invalid("invalid flat PSD dimensions or channels"));
    }
    let n = (width as usize).checked_mul(height as usize).ok_or(PsdError::LimitExceeded("image dimensions"))?;
    let ch = usize::from(channels);
    let needed = n.checked_mul(ch).ok_or(PsdError::LimitExceeded("sample count"))?;
    let (depth, have) = match samples {
        Samples::U8(s) => (8u16, s.len()),
        Samples::U16(s) => (16u16, s.len()),
    };
    if have < needed {
        return Err(PsdError::invalid("sample buffer too short"));
    }
    let color = if ch <= 2 { 1usize } else { 3 };
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&(color as u16).to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&depth.to_be_bytes());
    out.extend_from_slice(&(if color == 1 { 1u16 } else { 3 }).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    let mut resources = Vec::new();
    if let Some(ppi) = meta.ppi.filter(|p| *p > 0) {
        let fixed = (u32::from(ppi) << 16).to_be_bytes();
        let one = 1u16.to_be_bytes();
        resource(&mut resources, 1005, &[&fixed[..], &one, &one, &fixed, &one, &one].concat())?;
    }
    if let Some(data) = meta.icc {
        resource(&mut resources, 1039, data)?;
    }
    if let Some(data) = meta.exif {
        resource(&mut resources, 1058, data)?;
    }
    if let Some(data) = meta.xmp {
        resource(&mut resources, 1060, data.as_bytes())?;
    }
    out.extend_from_slice(&u32::try_from(resources.len()).map_err(|_| PsdError::LimitExceeded("resource section"))?.to_be_bytes());
    out.extend_from_slice(&resources);
    out.extend_from_slice(&0u32.to_be_bytes());
    match samples {
        Samples::U8(src) => {
            out.extend_from_slice(&1u16.to_be_bytes());
            let rows = color.checked_mul(height as usize).ok_or(PsdError::LimitExceeded("row count"))?;
            let table = out.len();
            let table_end =
                table.checked_add(rows.checked_mul(2).ok_or(PsdError::LimitExceeded("row table"))?).ok_or(PsdError::LimitExceeded("row table"))?;
            out.try_reserve(rows * 2).map_err(|_| PsdError::OutOfMemory("row table"))?;
            out.resize(table_end, 0);
            let mut row = Vec::new();
            row.try_reserve_exact(width as usize).map_err(|_| PsdError::OutOfMemory("row"))?;
            for c in 0..color {
                for y in 0..height as usize {
                    row.clear();
                    let start = y * width as usize * ch;
                    row.extend(src[start + c..start + width as usize * ch].iter().step_by(ch).copied());
                    let before = out.len();
                    crate::compression::packbits::encode(&row, &mut out);
                    let count = u16::try_from(out.len() - before).map_err(|_| PsdError::LimitExceeded("PSD row length"))?;
                    let at = table + (c * height as usize + y) * 2;
                    out[at..at + 2].copy_from_slice(&count.to_be_bytes());
                }
            }
        }
        Samples::U16(src) => {
            out.extend_from_slice(&0u16.to_be_bytes());
            for c in 0..color {
                for sample in src.iter().take(needed).skip(c).step_by(ch) {
                    out.extend_from_slice(&sample.to_be_bytes());
                }
            }
        }
    }
    Ok(out)
}

fn resource(out: &mut Vec<u8>, id: u16, data: &[u8]) -> Result<()> {
    let len = u32::try_from(data.len()).map_err(|_| PsdError::LimitExceeded("resource length"))?;
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(data);
    if !data.len().is_multiple_of(2) {
        out.push(0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_rgb_eight_bit_rows_and_preserves_resources() {
        let bytes = write(
            2,
            2,
            4,
            Samples::U8(&[10, 20, 30, 0, 40, 50, 60, 1, 70, 80, 90, 2, 100, 110, 120, 3]),
            Metadata { icc: Some(&[1, 2, 3]), xmp: Some("<x/>"), ppi: Some(300), ..Metadata::default() },
        )
        .unwrap();
        let psd = crate::PsdFile::from_bytes(&bytes).unwrap();
        assert_eq!(psd.header.channels, 3);
        assert_eq!(psd.decode_merged().unwrap(), [10, 40, 70, 100, 20, 50, 80, 110, 30, 60, 90, 120]);
        assert!(psd.resources.iter().any(|r| r.id == 1039 && r.data == [1, 2, 3]));
        assert!(psd.resources.iter().any(|r| r.id == 1060 && r.data == b"<x/>"));
    }

    #[test]
    fn writes_raw_sixteen_bit_gray_and_rejects_short_input() {
        let bytes = write(2, 1, 2, Samples::U16(&[1, 999, 0xabcd, 0]), Metadata::default()).unwrap();
        let psd = crate::PsdFile::from_bytes(&bytes).unwrap();
        assert_eq!(psd.header.channels, 1);
        assert_eq!(psd.decode_merged().unwrap(), [0, 1, 0xab, 0xcd]);
        assert!(write(2, 2, 3, Samples::U8(&[0; 11]), Metadata::default()).is_err());
        assert!(write(30_001, 1, 1, Samples::U8(&[]), Metadata::default()).is_err());
    }
}
