//! The 8×8 forward and inverse DCT of ISO/IEC 14496-2 Annex A, evaluated separably in `f64`.
//! The inverse rounds to the nearest integer and saturates to [-256, 255] (§7.4.4), well inside
//! the IEEE Std 1180-1990 accuracy the standard asks of a decoder's IDCT, so a conforming
//! decoder's reconstruction stays within a sample value of the encoder's.

use std::sync::OnceLock;

/// `M[x][u] = C(u)/2 · cos((2x+1)uπ/16)`, `C(0) = 1/√2`, `C(u>0) = 1`.
fn basis() -> &'static [[f64; 8]; 8] {
    static B: OnceLock<[[f64; 8]; 8]> = OnceLock::new();
    B.get_or_init(|| {
        let mut m = [[0f64; 8]; 8];
        for (x, row) in m.iter_mut().enumerate() {
            for (u, v) in row.iter_mut().enumerate() {
                let c = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                *v = c / 2.0 * (((2 * x + 1) * u) as f64 * std::f64::consts::PI / 16.0).cos();
            }
        }
        m
    })
}

/// Forward DCT of 64 samples (raster order) into rounded coefficients.
#[cfg(any(feature = "mpeg4-encode", test))]
pub(super) fn fdct(px: &[i32; 64], out: &mut [i32; 64]) {
    let m = basis();
    // rows: tmp[y][u] = Σ_x M[x][u] f[y][x]
    let mut tmp = [[0f64; 8]; 8];
    for y in 0..8 {
        for u in 0..8 {
            tmp[y][u] = (0..8).map(|x| m[x][u] * f64::from(px[y * 8 + x])).sum();
        }
    }
    for v in 0..8 {
        for u in 0..8 {
            let s: f64 = (0..8).map(|y| m[y][v] * tmp[y][u]).sum();
            out[v * 8 + u] = s.round() as i32;
        }
    }
}

/// In-place inverse DCT of dequantised coefficients (raster order), saturated to [-256, 255].
pub(super) fn idct(blk: &mut [i32; 64]) {
    if blk[1..].iter().all(|&c| c == 0) {
        // F[0][0]/8 everywhere, rounded half away from zero.
        let d = blk[0];
        let v = if d >= 0 { (d + 4) >> 3 } else { -((-d + 4) >> 3) };
        blk.fill(v.clamp(-256, 255));
        return;
    }
    let m = basis();
    // Each sum adds its terms in the order of the plain `Σ`, skipping zero coefficients and rows:
    // a skipped term is ±0, so the result is the same, and most blocks are sparse.
    // rows: tmp[v][x] = Σ_u M[x][u] F[v][u]
    let mut tmp = [[0f64; 8]; 8];
    let mut rows = [0usize; 8];
    let mut nrows = 0;
    for (v, t) in tmp.iter_mut().enumerate() {
        let r = &blk[v * 8..v * 8 + 8];
        if r.iter().all(|&c| c == 0) {
            continue;
        }
        rows[nrows] = v;
        nrows += 1;
        for (u, &c) in r.iter().enumerate() {
            if c != 0 {
                let c = f64::from(c);
                for (x, t) in t.iter_mut().enumerate() {
                    *t += m[x][u] * c;
                }
            }
        }
    }
    for y in 0..8 {
        let mut acc = [0f64; 8];
        for &v in &rows[..nrows] {
            let k = m[y][v];
            for (a, &t) in acc.iter_mut().zip(&tmp[v]) {
                *a += k * t;
            }
        }
        for (o, a) in blk[y * 8..y * 8 + 8].iter_mut().zip(acc) {
            *o = (a.round() as i32).clamp(-256, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_undoes_forward() {
        let mut x = 0x1234_5678u32;
        for _ in 0..200 {
            let px: [i32; 64] = std::array::from_fn(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x % 256) as i32
            });
            let mut c = [0; 64];
            fdct(&px, &mut c);
            idct(&mut c);
            for (a, b) in px.iter().zip(&c) {
                assert!((a - b).abs() <= 1, "{a} vs {b}");
            }
        }
    }

    #[test]
    fn flat_block_has_only_dc() {
        let mut c = [0; 64];
        fdct(&[100; 64], &mut c);
        assert_eq!(c[0], 800);
        assert!(c[1..].iter().all(|&v| v == 0));
    }
}
