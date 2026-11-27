//! Windows metafiles as the BLIP store holds them: without the 22-byte placeable header a `.wmf`
//! file starts with, which says how large the picture is.

/// A WMF's records with a placeable header put back in front, made from the BLIP's 34-byte
/// `OfficeArtMetafileHeader` (uncompressed size, bounds, size in EMUs, saved size, compression,
/// filter): the bounds in the metafile's units, and its units per inch from the bounds' width over
/// the width in EMUs (914400 an inch; 1440 when that can't be worked out). `None` when the header
/// is too short.
pub fn with_placeable_header(header: &[u8], records: Vec<u8>) -> Option<Vec<u8>> {
    let i32_at = |at: usize| Some(i32::from_le_bytes(header.get(at..at + 4)?.try_into().ok()?));
    let (l, t, r, b) = (i32_at(4)?, i32_at(8)?, i32_at(12)?, i32_at(16)?);
    let width_emu = f64::from(i32_at(20)?);
    let units = f64::from(r.saturating_sub(l));
    let inch = if width_emu > 0.0 && units > 0.0 { (units * 914_400.0 / width_emu).round().clamp(1.0, 65_535.0) as u16 } else { 1440 };
    let clamp16 = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    let mut out = Vec::with_capacity(22 + records.len());
    out.extend_from_slice(&0x9AC6_CDD7u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    for v in [l, t, r, b] {
        out.extend_from_slice(&clamp16(v).to_le_bytes());
    }
    out.extend_from_slice(&inch.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    // Checksum: the XOR of the ten words before it.
    let sum = out.chunks(2).fold(0u16, |x, w| x ^ u16::from_le_bytes([w[0], *w.get(1).unwrap_or(&0)]));
    out.extend_from_slice(&sum.to_le_bytes());
    out.extend_from_slice(&records);
    Some(out)
}
