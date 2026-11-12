//! Small constant tables from the H.264 specification.

/// 4x4 frame zig-zag scan: scan position -> raster index (row * 4 + col).
pub const ZIGZAG4: [u8; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

/// 8x8 frame zig-zag scan: scan position -> raster index (row * 8 + col).
pub const ZIGZAG8: [u8; 64] = zigzag8();

const fn zigzag8() -> [u8; 64] {
    let mut out = [0u8; 64];
    let mut k = 0;
    let mut d: i32 = 0; // anti-diagonal index = row + col
    while d < 15 {
        // even diagonals go up-right (row decreasing), odd go down-left
        let mut i: i32 = 0;
        while i < 8 {
            let (row, col) = if d % 2 == 0 { (d - i, i) } else { (i, d - i) };
            if row >= 0 && row < 8 && col >= 0 && col < 8 {
                out[k] = (row * 8 + col) as u8;
                k += 1;
            }
            i += 1;
        }
        d += 1;
    }
    out
}

/// Table 7-3.
pub const DEFAULT_4X4_INTRA: [u8; 16] = [6, 13, 13, 20, 20, 20, 28, 28, 28, 28, 32, 32, 32, 37, 37, 42];
pub const DEFAULT_4X4_INTER: [u8; 16] = [10, 14, 14, 20, 20, 20, 24, 24, 24, 24, 27, 27, 27, 30, 30, 34];
/// Table 7-4.
#[rustfmt::skip]
pub const DEFAULT_8X8_INTRA: [u8; 64] = [
    6, 10, 10, 13, 11, 13, 16, 16, 16, 16, 18, 18, 18, 18, 18, 23,
    23, 23, 23, 23, 23, 25, 25, 25, 25, 25, 25, 25, 27, 27, 27, 27,
    27, 27, 27, 27, 29, 29, 29, 29, 29, 29, 29, 31, 31, 31, 31, 31,
    31, 33, 33, 33, 33, 33, 36, 36, 36, 36, 38, 38, 38, 40, 40, 42,
];
#[rustfmt::skip]
pub const DEFAULT_8X8_INTER: [u8; 64] = [
    9, 13, 13, 15, 13, 15, 17, 17, 17, 17, 19, 19, 19, 19, 19, 21,
    21, 21, 21, 21, 21, 22, 22, 22, 22, 22, 22, 22, 24, 24, 24, 24,
    24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 27, 27, 27, 27, 27,
    27, 28, 28, 28, 28, 28, 30, 30, 30, 30, 32, 32, 32, 33, 33, 35,
];

/// Table 8-15: QPc as a function of qPI (0..=51).
pub const QPC_TABLE: [u8; 52] = {
    let mut t = [0u8; 52];
    let tail = [29u8, 30, 31, 32, 32, 33, 34, 34, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38, 39, 39, 39, 39];
    let mut i = 0;
    while i < 52 {
        t[i] = if i < 30 { i as u8 } else { tail[i - 30] };
        i += 1;
    }
    t
};

/// (8-315) v matrix for normAdjust4x4.
pub const NORM_ADJUST4: [[u16; 3]; 6] = [[10, 16, 13], [11, 18, 14], [13, 20, 16], [14, 23, 18], [16, 25, 20], [18, 29, 23]];
/// (8-318) v matrix for normAdjust8x8.
pub const NORM_ADJUST8: [[u16; 6]; 6] = [
    [20, 18, 32, 19, 25, 24],
    [22, 19, 35, 21, 28, 26],
    [26, 23, 42, 24, 33, 31],
    [28, 25, 45, 26, 35, 33],
    [32, 28, 51, 30, 40, 38],
    [36, 32, 58, 34, 46, 43],
];

/// normAdjust4x4(m, i, j) for raster position `pos` (i = row, j = col).
pub const fn norm_adjust4(m: usize, pos: usize) -> u16 {
    let (i, j) = (pos / 4, pos % 4);
    if i % 2 == 0 && j % 2 == 0 {
        NORM_ADJUST4[m][0]
    } else if i % 2 == 1 && j % 2 == 1 {
        NORM_ADJUST4[m][1]
    } else {
        NORM_ADJUST4[m][2]
    }
}

/// normAdjust8x8(m, i, j) for raster position `pos` (i = row, j = col).
pub const fn norm_adjust8(m: usize, pos: usize) -> u16 {
    let (i, j) = (pos / 8, pos % 8);
    let k = if i % 4 == 0 && j % 4 == 0 {
        0
    } else if i % 2 == 1 && j % 2 == 1 {
        1
    } else if i % 4 == 2 && j % 4 == 2 {
        2
    } else if (i % 4 == 0 && j % 2 == 1) || (i % 2 == 1 && j % 4 == 0) {
        3
    } else if (i % 4 == 0 && j % 4 == 2) || (i % 4 == 2 && j % 4 == 0) {
        4
    } else {
        5
    };
    NORM_ADJUST8[m][k]
}

/// Table 8-16 alpha' (indexA 0..=51).
#[rustfmt::skip]
pub const ALPHA: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 5, 6, 7, 8, 9, 10, 12, 13,
    15, 17, 20, 22, 25, 28, 32, 36, 40, 45, 50, 56, 63, 71, 80, 90, 101, 113, 127, 144, 162, 182, 203, 226, 255, 255,
];
/// Table 8-16 beta' (indexB 0..=51).
#[rustfmt::skip]
pub const BETA: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4,
    6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18,
];
/// Table 8-17 tC0' for bS = 1, 2, 3 (indexA 0..=51).
#[rustfmt::skip]
pub const TC0: [[u8; 3]; 52] = {
    let b1: [u8; 52] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1,
        1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 13,
    ];
    let b2: [u8; 52] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 7, 8, 8, 10, 11, 12, 13, 15, 17,
    ];
    let b3: [u8; 52] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 23, 25,
    ];
    let mut t = [[0u8; 3]; 52];
    let mut i = 0;
    while i < 52 {
        t[i] = [b1[i], b2[i], b3[i]];
        i += 1;
    }
    t
};

/// Luma 4x4 block index (decoding order, 6.4.3) -> (x, y) in 4-sample units.
pub const BLK4_XY: [(u8, u8); 16] =
    [(0, 0), (1, 0), (0, 1), (1, 1), (2, 0), (3, 0), (2, 1), (3, 1), (0, 2), (1, 2), (0, 3), (1, 3), (2, 2), (3, 2), (2, 3), (3, 3)];

/// Raster 4x4 position (y * 4 + x) -> luma4x4BlkIdx.
pub const RASTER_TO_BLK4: [u8; 16] = [0, 1, 4, 5, 2, 3, 6, 7, 8, 9, 12, 13, 10, 11, 14, 15];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zigzag8_known_prefix() {
        assert_eq!(&ZIGZAG8[..16], &[0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5]);
        assert_eq!(&ZIGZAG8[48..], &[58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63]);
        let mut seen = [false; 64];
        for &z in &ZIGZAG8 {
            seen[z as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn blk4_maps_consistent() {
        for (b, &(x, y)) in BLK4_XY.iter().enumerate() {
            let r = (y * 4 + x) as usize;
            assert_eq!(RASTER_TO_BLK4[r] as usize, b);
        }
    }

    #[test]
    fn qpc() {
        assert_eq!(QPC_TABLE[29], 29);
        assert_eq!(QPC_TABLE[30], 29);
        assert_eq!(QPC_TABLE[51], 39);
    }
}
