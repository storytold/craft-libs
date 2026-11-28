//! Parallel row/map helpers, sRGB transfer, bilinear sampling, hash, and box-blur radii
//! matching PhotoCraft's `photo_util` and `fxutil::gauss_box_radii` bit-for-bit.

pub(crate) use craft_color::{linear_to_srgb, srgb_to_linear};

/// Applies `f(y, row)` to each `w * ch`-wide row (in parallel on native targets).
pub(crate) fn par_rows<T: Send + Sync>(buf: &mut [T], w: usize, ch: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    let stride = (w * ch).max(1);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        buf.par_chunks_mut(stride).enumerate().for_each(|(y, row)| f(y, row));
    }
    #[cfg(target_arch = "wasm32")]
    buf.chunks_mut(stride).enumerate().for_each(|(y, row)| f(y, row));
}

/// Maps `f` over `0..n` (in parallel on native targets), keeping order.
pub(crate) fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        (0..n).into_par_iter().map(f).collect()
    }
    #[cfg(target_arch = "wasm32")]
    (0..n).map(f).collect()
}

pub(crate) fn hash01(x: i64, y: i64, seed: u64) -> f32 {
    let mut z =
        (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// Bilinear sample of channel `c` of a `w × h × ch` buffer, clamped to the edges.
pub(crate) fn bilinear(buf: &[f32], w: usize, h: usize, ch: usize, c: usize, x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    let p = |xx: usize, yy: usize| buf[(yy * w + xx) * ch + c];
    let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
    let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
    top + (bot - top) * ty
}

pub(crate) fn gauss_box_radii(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let w_ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0)).round().clamp(0.0, n) as usize;
    let mut radii = std::array::from_fn(|i| if i < m { (wl as usize - 1) / 2 } else { (wu as usize - 1) / 2 });
    if radii.iter().all(|&r| r == 0) {
        radii[0] = 1;
    }
    radii
}
