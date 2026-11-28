//! Fixed tables of RFC 1951: length/distance bases and extra bits, and the symbol lookups built from them.
//! Shared by the compressor and by the decoder in `png::inflate`.

pub(crate) const LENGTH_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
pub(crate) const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
pub(crate) const DIST_BASE: [u16; 30] =
    [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
pub(crate) const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// Order in which code-length code lengths are transmitted in a dynamic block header.
pub(crate) const CODE_LENGTH_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

pub(super) const END_OF_BLOCK: usize = 256;
pub(super) const MIN_MATCH: usize = 3;
pub(super) const MAX_MATCH: usize = 258;
pub(super) const WINDOW: usize = 32768;

/// Length code index (0..29) for each match length 0..=258 (entries below 3 are unused).
// Evaluated at compile time only: an out-of-range index is a build error, not a runtime panic (and `get_mut` is not
// usable in const context).
#[allow(clippy::indexing_slicing)]
const LENGTH_CODE: [u8; 259] = {
    let mut t = [0u8; 259];
    let mut code = 0;
    while code < 29 {
        let mut l = LENGTH_BASE[code] as usize;
        let end = l + (1 << LENGTH_EXTRA[code]);
        while l < end && l <= MAX_MATCH {
            t[l] = code as u8;
            l += 1;
        }
        code += 1;
    }
    t
};

/// Distance code index: `[d - 1]` for d <= 256, then `[256 + ((d - 1) >> 7)]`.
// Evaluated at compile time only: an out-of-range index is a build error, not a runtime panic (and `get_mut` is not
// usable in const context).
#[allow(clippy::indexing_slicing)]
const DIST_CODE: [u8; 512] = {
    let mut t = [0u8; 512];
    let mut code = 0;
    while code < 30 {
        let mut d = DIST_BASE[code] as usize;
        let end = d + (1 << DIST_EXTRA[code]);
        while d < end {
            let x = d - 1;
            if x < 256 {
                t[x] = code as u8;
            } else {
                t[256 + (x >> 7)] = code as u8;
            }
            d += 1;
        }
        code += 1;
    }
    t
};

/// Index (0..29) of the length code for a match length in 3..=258.
pub(super) fn length_code(len: usize) -> usize {
    LENGTH_CODE.get(len).copied().unwrap_or(0) as usize
}

/// Index (0..30) of the distance code for a distance in 1..=32768.
pub(super) fn dist_code(dist: usize) -> usize {
    let x = dist - 1;
    let i = if x < 256 { x } else { 256 + (x >> 7) };
    DIST_CODE.get(i).copied().unwrap_or(0) as usize
}

/// Code lengths of the fixed literal/length code (block type 1).
pub(super) fn fixed_literal_lengths() -> [u8; 288] {
    let mut l = [0u8; 288];
    for (i, v) in l.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups_agree_with_bases() {
        for len in MIN_MATCH..=MAX_MATCH {
            let c = length_code(len);
            let base = LENGTH_BASE[c] as usize;
            assert!(len >= base && len - base < (1 << LENGTH_EXTRA[c]) || len == 258 && c == 28, "len {len}");
        }
        assert_eq!(length_code(258), 28);
        assert_eq!(length_code(257), 27);
        for dist in 1..=WINDOW {
            let c = dist_code(dist);
            let base = DIST_BASE[c] as usize;
            assert!(dist >= base && dist - base < (1 << DIST_EXTRA[c]), "dist {dist}");
        }
    }
}
