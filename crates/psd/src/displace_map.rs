//! Bounded two-plane PSD Displace (`DspD`) payloads.
//! App filter settings, image conversion and external-path policy belong to the caller.
//! Ported from PhotoCraft `crates/io/src/displace_map.rs` (MIT/Apache-2.0).

const HEADER: usize = 10;
const MODE_TWO_PLANES: u16 = 3;
/// Maximum supported decoded pixel count (8K by 8K).
pub const MAX_MAP_PIXELS: usize = 1 << 26;

/// A parsed embedded map: size and the two 8-bit planes (borrowed from the blob).
#[derive(Debug, PartialEq)]
pub struct EmbeddedMap<'a> {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Horizontal displacement samples.
    pub horizontal: &'a [u8],
    /// Vertical displacement samples.
    pub vertical: &'a [u8],
}

/// Parses an embedded displacement map; `None` for any other mode, a zero or oversized map, or a
/// blob shorter than the header plus both planes.
pub fn parse_map(data: &[u8]) -> Option<EmbeddedMap<'_>> {
    let u32_at = |i: usize| Some(u32::from_le_bytes(data.get(i..i + 4)?.try_into().ok()?));
    let height = usize::try_from(u32_at(0)?).ok()?;
    let width = usize::try_from(u32_at(4)?).ok()?;
    let mode = u16::from_le_bytes(data.get(8..HEADER)?.try_into().ok()?);
    let n = width.checked_mul(height)?;
    if mode != MODE_TWO_PLANES || n == 0 || n > MAX_MAP_PIXELS {
        return None;
    }
    let horizontal = data.get(HEADER..HEADER.checked_add(n)?)?;
    let vertical = data.get(HEADER + n..HEADER.checked_add(n.checked_mul(2)?)?)?;
    Some(EmbeddedMap { width, height, horizontal, vertical })
}

/// Writes a mode-3 payload from exact-sized 8-bit planes, without padding.
/// Returns `None` for empty/oversized dimensions, mismatched planes or allocation failure.
/// Callers retain their own (possibly lower) embedding and image-conversion budgets.
pub fn write_map(width: usize, height: usize, horizontal: &[u8], vertical: &[u8]) -> Option<Vec<u8>> {
    write_map_iter(width, height, horizontal.iter().copied(), vertical.iter().copied())
}

/// Writes exact-sized sample iterators without allocating intermediate planes.
/// Iterators must terminate after exactly width times height samples; extra or missing
/// samples are rejected. At most that many samples plus one are consumed per plane.
pub fn write_map_iter(
    width: usize,
    height: usize,
    mut horizontal: impl Iterator<Item = u8>,
    mut vertical: impl Iterator<Item = u8>,
) -> Option<Vec<u8>> {
    let n = width.checked_mul(height)?;
    if n == 0 || n > MAX_MAP_PIXELS {
        return None;
    }
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    let len = HEADER.checked_add(n.checked_mul(2)?)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len).ok()?;
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&MODE_TWO_PLANES.to_le_bytes());
    for samples in [&mut horizontal as &mut dyn Iterator<Item = u8>, &mut vertical] {
        for _ in 0..n {
            out.push(samples.next()?);
        }
        if samples.next().is_some() {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_planes_roundtrip_and_padding_is_ignored() {
        let mut bytes = write_map(2, 1, &[0, 255], &[128, 64]).unwrap();
        assert_eq!(bytes, [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 255, 128, 64]);
        bytes.extend_from_slice(&[99, 88]);
        let m = parse_map(&bytes).unwrap();
        assert_eq!((m.width, m.height), (2, 1));
        assert_eq!(m.horizontal, &[0, 255]);
        assert_eq!(m.vertical, &[128, 64]);
    }

    #[test]
    fn truncated_unsupported_and_overflow_are_rejected() {
        let bytes = write_map(2, 1, &[0, 255], &[128, 64]).unwrap();
        for end in 0..bytes.len() {
            assert!(parse_map(&bytes[..end]).is_none());
        }
        let mut invalid = bytes.clone();
        invalid[8] = 4;
        assert!(parse_map(&invalid).is_none());
        invalid[8] = 3;
        invalid[..8].fill(255);
        assert!(parse_map(&invalid).is_none());
        invalid[..8].fill(0);
        assert!(parse_map(&invalid).is_none());
        invalid[..4].copy_from_slice(&1u32.to_le_bytes());
        invalid[4..8].copy_from_slice(&((MAX_MAP_PIXELS + 1) as u32).to_le_bytes());
        assert!(parse_map(&invalid).is_none());
        assert!(write_map_iter(2, 1, std::iter::repeat(0), std::iter::repeat(0)).is_none());
        assert!(write_map(usize::MAX, 2, &[], &[]).is_none());
        assert!(write_map(MAX_MAP_PIXELS + 1, 1, &[], &[]).is_none());
        assert!(write_map(2, 1, &[0], &[0, 0]).is_none());
        assert!(write_map(2, 1, &[0, 0], &[0, 0, 0]).is_none());
    }
}
