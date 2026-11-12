//! RFC 9924 tables: zig-zag scan, 8x8 transform matrix, quantization level scales, and level limits.

/// 8x8 zig-zag scan order (`ScanOrder[sPos] = yC * 8 + xC`), derived from RFC 9924 §4.4.1 Figure 5.
pub const ZIGZAG_8X8: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43,
    36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// 8x8 transform matrix (`transMatrix` in RFC 9924 §6.3.2.3 Figure 25).
pub const TRANS_MATRIX: [[i32; 8]; 8] = [
    [64, 64, 64, 64, 64, 64, 64, 64],
    [89, 75, 50, 18, -18, -50, -75, -89],
    [84, 35, -35, -84, -84, -35, 35, 84],
    [75, -18, -89, -50, 50, 89, 18, -75],
    [64, -64, -64, 64, 64, -64, -64, 64],
    [50, -89, 18, 75, -75, -18, 89, -50],
    [35, -84, 84, -35, -35, 84, -84, 35],
    [18, -50, 75, -89, 89, -75, 50, -18],
];

/// Quantization scaling list `levelScale[k]` for `k = qP % 6` (RFC 9924 §6.3.1).
pub const LEVEL_SCALE: [i32; 6] = [40, 45, 51, 57, 64, 71];

/// Default flat quantization matrix value when `use_q_matrix == 0` (RFC 9924 §5.3.7).
pub const DEFAULT_Q_MATRIX_VAL: u8 = 16;

/// General level limits (`level_idc`, max luma sample rate, and max coded data rate in Mbit/s per band)
/// from RFC 9924 §9.4.2 Table 4.
pub const LEVEL_LIMITS: [(u8, u64, [u32; 4]); 14] = [
    (30, 3_041_280, [8, 11, 15, 23]),
    (33, 6_082_560, [16, 21, 30, 45]),
    (60, 15_667_200, [39, 54, 76, 114]),
    (63, 31_334_400, [78, 108, 152, 227]),
    (90, 66_846_720, [114, 159, 222, 333]),
    (93, 133_693_440, [227, 317, 444, 666]),
    (120, 265_420_800, [455, 637, 892, 1_338]),
    (123, 530_841_600, [910, 1_274, 1_784, 2_675]),
    (150, 1_061_683_200, [1_820, 2_548, 3_567, 5_350]),
    (153, 2_123_366_400, [3_639, 5_095, 7_133, 10_699]),
    (180, 4_777_574_400, [7_278, 10_189, 14_265, 21_397]),
    (183, 8_493_465_600, [14_556, 20_378, 28_529, 42_793]),
    (210, 16_986_931_200, [29_111, 40_756, 57_058, 85_586]),
    (213, 33_973_862_400, [58_222, 81_511, 114_115, 171_172]),
];

/// Choose the lowest conforming `(level_idc, band_idc)` for a luma sample rate and bitrate (in kbps).
pub fn select_level_and_band(luma_samples_per_sec: u64, kbps: u32) -> (u8, u8) {
    let mbps = kbps.div_ceil(1000);
    for &(level_idc, max_samples, bands) in &LEVEL_LIMITS {
        if luma_samples_per_sec <= max_samples {
            for (band_idc, &max_mbps) in bands.iter().enumerate() {
                if mbps <= max_mbps {
                    return (level_idc, band_idc as u8);
                }
            }
        }
    }
    (213, 3)
}
