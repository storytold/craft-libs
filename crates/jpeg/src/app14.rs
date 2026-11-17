//! Adobe APP14 transform inspection for JPEG samples.

/// Read the Adobe APP14 transform byte needed for RGB/CMYK JPEG interpretation.
pub fn adobe_transform(jpeg: &[u8]) -> Option<u8> {
    let mut at = 2usize;
    if jpeg.get(..2) != Some(&[0xFF, 0xD8]) {
        return None;
    }
    while at + 1 < jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        if marker == 0xDA {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([*jpeg.get(at + 2)?, *jpeg.get(at + 3)?]));
        if len < 2 {
            return None;
        }
        let end = at.checked_add(2 + len)?;
        let segment = jpeg.get(at + 4..end)?;
        if marker == 0xEE && segment.get(..5) == Some(b"Adobe") {
            return segment.get(11).copied();
        }
        at = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::adobe_transform;
    #[test]
    fn reads_adobe_transform_and_rejects_truncated_segments() {
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xee, 0, 14];
        jpeg.extend_from_slice(b"Adobe\0\x64\0\0\0\0\x02");
        assert_eq!(adobe_transform(&jpeg), Some(2));
        assert_eq!(adobe_transform(&jpeg[..jpeg.len() - 1]), None);
        assert_eq!(adobe_transform(&[0xff, 0xd8, 0xff, 0xda]), None);
        assert_eq!(adobe_transform(&[0xff, 0xd8, 0xff, 0xee, 0, 1]), None);
    }
}
