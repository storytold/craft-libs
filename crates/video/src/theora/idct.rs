//! 8x8 Inverse and Forward Discrete Cosine Transforms for Theora (Section 7.9.3).
//!
//! A compliant decoder MUST use the exact 16-bit integerized approximation of the 8-point
//! 1D inverse DCT based on the Chen factorization specified in Section 7.9.3.1.

use super::tables::{C1, C2, C3, C4, C5, C6, C7};

// Sine aliases from Table 7.65
#[allow(dead_code)]
const S1: i32 = C7;
#[allow(dead_code)]
const S2: i32 = C6;
const S3: i32 = C5;
const S6: i32 = C2;
const S7: i32 = C1;

/// The 1D 8-point inverse DCT (Section 7.9.3.1).
#[inline]
pub fn idct_1d(y: &[i16; 8]) -> [i16; 8] {
    let mut t = [0i32; 8];

    // 1-3. T[0] = C4 * (Y[0] + Y[4]) >> 16
    let t0_in = (y[0] as i32 + y[4] as i32) as i16 as i32;
    t[0] = (C4 * t0_in) >> 16;

    // 4-6. T[1] = C4 * (Y[0] - Y[4]) >> 16
    let t1_in = (y[0] as i32 - y[4] as i32) as i16 as i32;
    t[1] = (C4 * t1_in) >> 16;

    // 7-12
    t[2] = ((C6 * y[2] as i32) >> 16) - ((S6 * y[6] as i32) >> 16);
    t[3] = ((S6 * y[2] as i32) >> 16) + ((C6 * y[6] as i32) >> 16);
    t[4] = ((C7 * y[1] as i32) >> 16) - ((S7 * y[7] as i32) >> 16);
    t[5] = ((C3 * y[5] as i32) >> 16) - ((S3 * y[3] as i32) >> 16);
    t[6] = ((S3 * y[5] as i32) >> 16) + ((C3 * y[3] as i32) >> 16);
    t[7] = ((S7 * y[1] as i32) >> 16) + ((C7 * y[7] as i32) >> 16);

    // 13-17
    let r1 = t[4] + t[5];
    let t5_diff = (t[4] - t[5]) as i16 as i32;
    t[5] = (C4 * t5_diff) >> 16;
    t[4] = r1;

    // 18-22
    let r2 = t[7] + t[6];
    let t6_diff = (t[7] - t[6]) as i16 as i32;
    t[6] = (C4 * t6_diff) >> 16;
    t[7] = r2;

    // 23-25
    let r3 = t[0] + t[3];
    t[3] = t[0] - t[3];
    t[0] = r3;

    // 26-28
    let r4 = t[1] + t[2];
    t[2] = t[1] - t[2];
    t[1] = r4;

    // 29-31
    let r5 = t[6] + t[5];
    t[5] = t[6] - t[5];
    t[6] = r5;

    // 32-55 Output truncation to 16-bit signed
    [
        (t[0] + t[7]) as i16,
        (t[1] + t[6]) as i16,
        (t[2] + t[5]) as i16,
        (t[3] + t[4]) as i16,
        (t[3] - t[4]) as i16,
        (t[2] - t[5]) as i16,
        (t[1] - t[6]) as i16,
        (t[0] - t[7]) as i16,
    ]
}

/// The 2D 8x8 inverse DCT (Section 7.9.3.2).
///
/// `dqc` is the 64 dequantized DCT coefficients in natural raster order (row-major).
/// Returns an 8x8 residual array with ties rounded towards +infinity.
pub fn idct_2d(dqc: &[i16; 64]) -> [[i16; 8]; 8] {
    let mut intermediate = [[0i16; 8]; 8];

    // Pass 1: 1D IDCT on each row
    for ri in 0..8 {
        let mut row_in = [0i16; 8];
        for ci in 0..8 {
            row_in[ci] = dqc[ri * 8 + ci];
        }
        intermediate[ri] = idct_1d(&row_in);
    }

    // Pass 2: 1D IDCT on each column
    let mut res = [[0i16; 8]; 8];
    for ci in 0..8 {
        let mut col_in = [0i16; 8];
        for ri in 0..8 {
            col_in[ri] = intermediate[ri][ci];
        }
        let col_out = idct_1d(&col_in);
        for ri in 0..8 {
            // (x + 8) >> 4 with ties towards positive infinity
            res[ri][ci] = ((col_out[ri] as i32 + 8) >> 4) as i16;
        }
    }

    res
}

/// Fast DC-only residual calculation (Section 7.9.4 step 2(d)vii).
///
/// Used when a block has at most one non-zero coefficient (the DC coefficient).
#[inline]
pub fn idct_dc_only(coeff0: i16, qmat0: u16) -> [[i16; 8]; 8] {
    let dc = ((coeff0 as i32 * qmat0 as i32 + 15) >> 5) as i16;
    [[dc; 8]; 8]
}

/// Proper division by 2^16 (rounding towards zero) for forward DCT.
#[inline]
fn div2_16(v: i32) -> i32 {
    let offset = if v < 0 { 0xFFFF } else { 0 };
    (v + offset) >> 16
}

/// The 1D 8-point forward DCT (Section 7.9.3.3).
#[inline]
pub fn fdct_1d(x: &[i16; 8]) -> [i16; 8] {
    let mut t = [0i32; 8];

    t[0] = x[0] as i32 + x[7] as i32;
    t[1] = x[1] as i32 + x[6] as i32;
    t[2] = x[2] as i32 + x[5] as i32;
    t[3] = x[3] as i32 + x[4] as i32;
    t[4] = x[3] as i32 - x[4] as i32;
    t[5] = x[2] as i32 - x[5] as i32;
    t[6] = x[1] as i32 - x[6] as i32;
    t[7] = x[0] as i32 - x[7] as i32;

    let r1 = t[0] + t[3];
    t[3] = t[0] - t[3];
    t[0] = r1;

    let r2 = t[1] + t[2];
    t[2] = t[1] - t[2];
    t[1] = r2;

    let r3 = t[6] - t[5];
    t[6] = div2_16(C4 * (t[6] + t[5]));
    t[5] = div2_16(C4 * r3);

    let r4 = t[4] + t[5];
    t[5] = t[4] - t[5];
    t[4] = r4;

    let r5 = t[7] + t[6];
    t[6] = t[7] - t[6];
    t[7] = r5;

    [
        div2_16(C4 * (t[0] + t[1])) as i16,
        (div2_16(S7 * t[7]) + div2_16(C7 * t[4])) as i16,
        (div2_16(S6 * t[3]) + div2_16(C6 * t[2])) as i16,
        (div2_16(C3 * t[6]) - div2_16(S3 * t[5])) as i16,
        div2_16(C4 * (t[0] - t[1])) as i16,
        (div2_16(S3 * t[6]) + div2_16(C3 * t[5])) as i16,
        (div2_16(C6 * t[3]) - div2_16(S6 * t[2])) as i16,
        (div2_16(C7 * t[7]) - div2_16(S7 * t[4])) as i16,
    ]
}

/// The 2D 8x8 forward DCT (Section 7.9.3.3).
pub fn fdct_2d(pixel_block: &[[i16; 8]; 8]) -> [i16; 64] {
    let mut intermediate = [[0i16; 8]; 8];

    // Pass 1: 1D FDCT on each row
    for ri in 0..8 {
        intermediate[ri] = fdct_1d(&pixel_block[ri]);
    }

    // Pass 2: 1D FDCT on each column
    let mut coeffs = [0i16; 64];
    for ci in 0..8 {
        let mut col_in = [0i16; 8];
        for ri in 0..8 {
            col_in[ri] = intermediate[ri][ci];
        }
        let col_out = fdct_1d(&col_in);
        for ri in 0..8 {
            coeffs[ri * 8 + ci] = col_out[ri];
        }
    }

    coeffs
}
