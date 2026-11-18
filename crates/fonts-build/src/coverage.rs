//! The characters a face maps, as [`ServedFace::coverage`](crate::ServedFace::coverage) stores
//! them: its sorted, disjoint code point ranges, each as two unsigned LEB128 numbers, the gap from
//! the end of the previous range (from 0 for the first) and the range's length minus one. A
//! Chinese interface face cut to an app's catalogs takes about 3 KB.

use crate::cmap::Cmap;

/// The coverage of face `index` of `font` (a `.ttf`/`.otf`, or a `.ttc` collection); `None` when
/// the file isn't a font or has no Unicode `cmap` subtable in format 4 or 12.
pub fn of_face(font: &[u8], index: u32) -> Option<Vec<u8>> {
    Some(encode(&Cmap::read(font, index)?.ranges()))
}

/// The coverage of inclusive code point ranges, sorted by start. Overlaps are dropped.
pub fn encode(ranges: &[(u32, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut next = 0;
    for &(start, end) in ranges {
        let start = start.max(next);
        if end < start {
            continue;
        }
        leb128(&mut out, start - next);
        leb128(&mut out, end - start);
        let Some(after) = end.checked_add(1) else { break };
        next = after;
    }
    out
}

/// Whether `coverage` holds `c`.
pub fn contains(coverage: &[u8], c: char) -> bool {
    let c = u32::from(c);
    let mut bytes = coverage.iter().copied();
    let mut next = 0u32;
    while let (Some(gap), Some(len)) = (read(&mut bytes), read(&mut bytes)) {
        let Some(start) = next.checked_add(gap) else { return false };
        let Some(end) = start.checked_add(len) else { return false };
        if c < start {
            return false;
        }
        if c <= end {
            return true;
        }
        let Some(after) = end.checked_add(1) else { return false };
        next = after;
    }
    false
}

fn leb128(out: &mut Vec<u8>, mut n: u32) {
    while n >= 0x80 {
        out.push((n & 0x7F) as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn read(bytes: &mut impl Iterator<Item = u8>) -> Option<u32> {
    let mut n = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = bytes.next()?;
        n |= u32::from(byte & 0x7F).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}
