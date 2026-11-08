//! 8×8 orthonormal DCT-II / DCT-III (IDCT) in single precision.
//!
//! The reconstruction IDCT is defined as the exact real-valued 2-D inverse DCT
//! `f(x,y) = Σ_u Σ_v C(u)C(v)/4 · F(v,u) · cos((2x+1)uπ/16) · cos((2y+1)vπ/16)`
//! evaluated separably (rows then columns) in `f32`. For ProRes coefficient ranges the `f32`
//! evaluation error is below 2⁻⁸ in the 12-bit sample domain, so the rounded output equals the
//! rounded exact IDCT except when the exact value lies within that distance of a rounding tie.

use crate::tables::BASIS;

/// In-place inverse DCT. `block` holds dequantised coefficients in raster order
/// (`block[v*8 + u]`, `v` = vertical frequency) and receives samples (`block[y*8 + x]`).
///
/// Each 1-D pass uses the even/odd symmetry of the basis (`B[u][7-x] = ±B[u][x]` for even/odd
/// `u`) and skips all-zero rows; DC-only blocks are filled directly.
#[inline]
pub(crate) fn idct8x8(block: &mut [f32; 64]) {
    let mut t = [[0f32; 8]; 8];
    let mut nz = [false; 8];
    let mut dc_only = true;
    // Horizontal pass: t[r][x] = Σ_u F[r][u] · B[u][x]
    for (r, c) in block.as_chunks::<8>().0.iter().enumerate() {
        if c[1..].iter().all(|&v| v == 0.0) {
            if c[0] != 0.0 {
                t[r] = [c[0] * BASIS[0][0]; 8];
                nz[r] = true;
                dc_only &= r == 0;
            }
            continue;
        }
        dc_only = false;
        nz[r] = true;
        let mut out = [0f32; 8];
        for x in 0..4 {
            let e = c[0] * BASIS[0][x] + c[2] * BASIS[2][x] + c[4] * BASIS[4][x] + c[6] * BASIS[6][x];
            let o = c[1] * BASIS[1][x] + c[3] * BASIS[3][x] + c[5] * BASIS[5][x] + c[7] * BASIS[7][x];
            out[x] = e + o;
            out[7 - x] = e - o;
        }
        t[r] = out;
    }
    if dc_only {
        *block = [t[0][0] * BASIS[0][0]; 64];
        return;
    }
    // Vertical pass: f[y][x] = Σ_v t[v][x] · B[v][y]
    for y in 0..4 {
        let mut e = [0f32; 8];
        let mut o = [0f32; 8];
        for v in [0, 2, 4, 6] {
            if nz[v] {
                let b = BASIS[v][y];
                for x in 0..8 {
                    e[x] += t[v][x] * b;
                }
            }
        }
        for v in [1, 3, 5, 7] {
            if nz[v] {
                let b = BASIS[v][y];
                for x in 0..8 {
                    o[x] += t[v][x] * b;
                }
            }
        }
        for x in 0..8 {
            block[y * 8 + x] = e[x] + o[x];
            block[(7 - y) * 8 + x] = e[x] - o[x];
        }
    }
}

/// In-place forward DCT (samples in, coefficients out; same layout as [`idct8x8`]).
pub(crate) fn fdct8x8(block: &mut [f32; 64]) {
    let mut t = [[0f32; 8]; 8];
    // Horizontal: t[y][u] = Σ_x f[y][x] · B[u][x]
    for y in 0..8 {
        for u in 0..8 {
            let mut s = 0f32;
            for x in 0..8 {
                s += block[y * 8 + x] * BASIS[u][x];
            }
            t[y][u] = s;
        }
    }
    // Vertical: F[v][u] = Σ_y t[y][u] · B[v][y]
    for v in 0..8 {
        let mut acc = [0f32; 8];
        for y in 0..8 {
            let b = BASIS[v][y];
            for u in 0..8 {
                acc[u] += t[y][u] * b;
            }
        }
        block[v * 8..v * 8 + 8].copy_from_slice(&acc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_idct(c: &[f32; 64]) -> [f64; 64] {
        let mut out = [0f64; 64];
        for y in 0..8 {
            for x in 0..8 {
                let mut s = 0f64;
                for v in 0..8 {
                    for u in 0..8 {
                        let cu = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                        let cv = if v == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                        s += cu * cv / 4.0
                            * c[v * 8 + u] as f64
                            * (((2 * x + 1) * u) as f64 * std::f64::consts::PI / 16.0).cos()
                            * (((2 * y + 1) * v) as f64 * std::f64::consts::PI / 16.0).cos();
                    }
                }
                out[y * 8 + x] = s;
            }
        }
        out
    }

    #[test]
    fn idct_matches_double_reference() {
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed >> 8
        };
        for _ in 0..200 {
            let mut c = [0f32; 64];
            for v in c.iter_mut() {
                if rnd() % 3 == 0 {
                    *v = ((rnd() % 8192) as i32 - 4096) as f32;
                }
            }
            let r = reference_idct(&c);
            let mut b = c;
            idct8x8(&mut b);
            for i in 0..64 {
                assert!((b[i] as f64 - r[i]).abs() < 4e-3, "{} vs {}", b[i], r[i]);
            }
        }
    }

    #[test]
    fn fdct_inverts_idct() {
        let mut b = [0f32; 64];
        for (i, v) in b.iter_mut().enumerate() {
            *v = ((i * 37) % 200) as f32 - 100.0;
        }
        let orig = b;
        fdct8x8(&mut b);
        idct8x8(&mut b);
        for i in 0..64 {
            assert!((b[i] - orig[i]).abs() < 1e-3);
        }
    }
}
