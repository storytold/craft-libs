//! Windows metafiles as the BLIP store holds them: without the 22-byte placeable header a `.wmf`
//! file starts with, which says how large the picture is.

/// Bytes of the placeable header [`with_placeable_header`] adds.
pub const PLACEABLE_HEADER: usize = 22;

/// The bounds of a BLIP's `OfficeArtMetafileHeader` as 16-bit coordinates, when they are.
fn bounds16(header: &[u8]) -> Option<[i16; 4]> {
    let v = |at: usize| i16::try_from(i32::from_le_bytes(header.get(at..at + 4)?.try_into().ok()?)).ok();
    Some([v(4)?, v(8)?, v(12)?, v(16)?])
}

/// The key a placeable header starts with.
const PLACEABLE_KEY: u32 = 0x9AC6_CDD7;

/// Whether a WMF starts with a placeable header already.
pub fn is_placeable(wmf: &[u8]) -> bool {
    wmf.get(..4).is_some_and(|k| k == PLACEABLE_KEY.to_le_bytes())
}

/// Whether [`with_placeable_header`] would add a header: the BLIP header is whole and its bounds
/// fit 16 bits.
pub fn gets_header(header: &[u8]) -> bool {
    header.len() >= 24 && bounds16(header).is_some()
}

/// A WMF's records with a placeable header put back in front, made from the BLIP's 34-byte
/// `OfficeArtMetafileHeader` (uncompressed size, bounds, size in EMUs, saved size, compression,
/// filter): the bounds in the metafile's units, and its units per inch from the bounds' width over
/// the width in EMUs (914400 an inch; 1440 when that can't be worked out). `None` when the header
/// is too short. Bounds outside the 16-bit coordinates WMF records use describe no placeable
/// header: the records are kept as they are, without one.
pub fn with_placeable_header(header: &[u8], records: Vec<u8>) -> Option<Vec<u8>> {
    let i32_at = |at: usize| Some(i32::from_le_bytes(header.get(at..at + 4)?.try_into().ok()?));
    let (l, r) = (i32_at(4)?, i32_at(12)?);
    let width_emu = f64::from(i32_at(20)?);
    let Some(bounds) = bounds16(header) else {
        log::warn!("officeart: a WMF's bounds don't fit its 16-bit coordinates; it is kept without a placeable header");
        return Some(records);
    };
    let units = f64::from(r.saturating_sub(l));
    let inch = if width_emu > 0.0 && units > 0.0 { (units * 914_400.0 / width_emu).round().clamp(1.0, 65_535.0) as u16 } else { 1440 };
    let mut out = Vec::with_capacity(PLACEABLE_HEADER + records.len());
    out.extend_from_slice(&PLACEABLE_KEY.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    for v in bounds {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&inch.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    // Checksum: the XOR of the ten words before it.
    let sum = out.chunks(2).fold(0u16, |x, w| x ^ u16::from_le_bytes([w[0], *w.get(1).unwrap_or(&0)]));
    out.extend_from_slice(&sum.to_le_bytes());
    out.extend_from_slice(&records);
    Some(out)
}
