//! PSD Grid and Guides version 2 (resource 1092) record decoding.
//!
//! Extracted from PhotoCraft `guides_map.rs` at f49564d281bf8892db58ed154767717223aff4f6
//! by Bram Heerink. Document colour preference and resource-retention policy belong to callers.
//! Unknown direction values retain PhotoCraft's vertical interpretation. Grid cycles, reserved
//! bytes and trailing data are not interpreted. Resource 1097 remains opaque except its count.

/// Grid and Guides version 2 resource ID.
pub const GUIDES_V2: u16 = 1092;
/// Associated guide identifiers resource ID.
pub const GUIDE_IDS: u16 = 1097;

/// One version 2 guide record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GuideV2 {
    /// Signed position in units of 1/32 pixel.
    pub location: i32,
    /// True for vertical guides (direction byte other than 1).
    pub vertical: bool,
    /// Stored RGB colour; interpretation of preference colours is app policy.
    pub rgb: [u8; 3],
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    let b = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Validate the full declared record region before returning an allocation-free iterator.
/// Returns `None` for an unsupported version, short header or truncated records.
/// Record count cannot cause allocation or work beyond the supplied byte slice.
pub fn parse_v2(data: &[u8]) -> Option<impl ExactSizeIterator<Item = GuideV2> + '_> {
    if be_u32(data, 0)? != 2 {
        return None;
    }
    let count = usize::try_from(be_u32(data, 12)?).ok()?;
    let end = 16usize.checked_add(count.checked_mul(9)?)?;
    let records = data.get(16..end)?;
    Some(records.as_chunks::<9>().0.iter().map(|g| GuideV2 {
        location: i32::from_be_bytes([g[0], g[1], g[2], g[3]]),
        vertical: g[4] != 1,
        rgb: [g[6], g[7], g[8]],
    }))
}

/// Only the declared entry count in resource 1097; this does not validate its GUID payload.
pub fn ids_count(data: &[u8]) -> Option<u32> {
    be_u32(data, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut v = Vec::new();
        for n in [2u32, 576, 576, 2] {
            v.extend_from_slice(&n.to_be_bytes());
        }
        for (position, direction, rgb) in [(512i32, 0u8, [74, 255, 255]), (-32, 1, [1, 2, 3])] {
            v.extend_from_slice(&position.to_be_bytes());
            v.extend_from_slice(&[direction, 0]);
            v.extend_from_slice(&rgb);
        }
        v
    }

    #[test]
    fn decodes_signed_positions_directions_and_colours() {
        let data = fixture();
        let actual = parse_v2(&data).map(Iterator::collect::<Vec<_>>);
        assert_eq!(
            actual,
            Some(vec![GuideV2 { location: 512, vertical: true, rgb: [74, 255, 255] }, GuideV2 { location: -32, vertical: false, rgb: [1, 2, 3] },])
        );
    }

    #[test]
    fn validates_every_truncation_and_hostile_count_before_iteration() {
        let mut data = fixture();
        for n in 0..data.len() {
            assert!(parse_v2(&data[..n]).is_none());
        }
        data[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse_v2(&data).is_none());
        data[12..16].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(parse_v2(&data).map(|v| v.len()), Some(0));
        data[3] = 1;
        assert!(parse_v2(&data).is_none());
    }

    #[test]
    fn preserves_unknown_direction_reserved_bytes_and_trailing_data_policy() {
        let mut data = fixture();
        data[20] = 255;
        data[21] = 42;
        data.extend_from_slice(&[9; 12]);
        assert_eq!(parse_v2(&data).and_then(|mut v| v.next()).map(|g| g.vertical), Some(true));
        assert_eq!(parse_v2(&data).map(|v| v.len()), Some(2));
        assert_eq!(ids_count(&[0, 0, 0, 2, 255]), Some(2));
        assert_eq!(ids_count(&[0, 0, 0]), None);
    }
}
