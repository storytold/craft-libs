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
