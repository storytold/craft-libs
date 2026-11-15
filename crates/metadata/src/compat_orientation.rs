//! Compatibility orientation reading for legacy clients that accepted loosely formed classic EXIF.
//! The ordinary `exif_orientation` API remains strict; use this only to retain older behavior.

/// Read the last IFD0 orientation entry as older lightweight readers did. Entry count is bounded;
/// a complete entry table is required, but an absent next-IFD pointer is accepted. BYTE, UNDEFINED,
/// SHORT, LONG and IFD values are read inline regardless of count. Invalid/missing values give 1.
pub fn orientation(bytes: &[u8]) -> u16 {
    let b = bytes.strip_prefix(b"Exif\0\0").unwrap_or(bytes);
    let little = match b.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return 1,
    };
    let u16_at =
        |at: usize| b.get(at..at.checked_add(2)?).map(|v| if little { u16::from_le_bytes([v[0], v[1]]) } else { u16::from_be_bytes([v[0], v[1]]) });
    let u32_at = |at: usize| {
        b.get(at..at.checked_add(4)?)
            .map(|v| if little { u32::from_le_bytes([v[0], v[1], v[2], v[3]]) } else { u32::from_be_bytes([v[0], v[1], v[2], v[3]]) })
    };
    if u16_at(2) != Some(42) {
        return 1;
    }
    let Some(ifd) = u32_at(4).and_then(|v| usize::try_from(v).ok()) else { return 1 };
    let Some(count) = u16_at(ifd).map(usize::from).filter(|n| *n <= 4096) else { return 1 };
    let Some(start) = ifd.checked_add(2) else { return 1 };
    let Some(end) = count.checked_mul(12).and_then(|n| start.checked_add(n)).filter(|end| *end <= b.len()) else { return 1 };
    let mut orientation = None;
    for at in (start..end).step_by(12) {
        if u16_at(at) == Some(0x0112) {
            let value = match u16_at(at + 2) {
                Some(3) => u16_at(at + 8).map(u32::from),
                Some(4 | 13) => u32_at(at + 8),
                Some(1 | 7) => b.get(at + 8).copied().map(u32::from),
                _ => None,
            };
            orientation = value.map(|v| v as u16).filter(|v| (1..=8).contains(v));
        }
    }
    orientation.unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_legacy_count_zero_but_bounds_the_table() {
        let mut b = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\0\0\0\0\x06\0\0\0".to_vec();
        assert_eq!(orientation(&b), 6);
        b.truncate(21);
        assert_eq!(orientation(&b), 1);
    }
}
