//! Adaptive Homogeneity-Directed demosaicing (K. Hirakawa, T. W. Parks, "Adaptive homogeneity-directed
//! demosaicing algorithm", IEEE Trans. Image Processing 14(3), 2005), implemented from the paper:
//!
//! 1. Interpolate green horizontally and vertically (5-tap filter [−¼ ½ ½ ½ −¼] across the missing site,
//!    clamped to the two adjacent greens), giving two candidate green planes.
//! 2. Complete each candidate with red/blue by colour-difference (C − G) interpolation.
//! 3. Convert both candidates to CIELab and measure, per pixel, how many of its 4 neighbours lie within the
//!    adaptive luminance / chrominance tolerances (ε_L, ε_C from the paper) — the homogeneity.
//! 4. Sum homogeneity over a 3×3 window and pick the more homogeneous candidate. The paper picks one outright;
//!    we blend the two by how far apart their sums are (see [`BLEND`]), so a clear edge still takes one candidate.
//!
//! Work is done in overlapping tiles (bounded memory, rayon-parallel), one band of tiles at a time written
//! straight into the output. Each step runs only where the tile's output depends on it: the choice reads the
//! homogeneity 1 pixel around the tile, which reads Lab 2 pixels around, the candidates read the greens 3
//! pixels around, and those read the mosaic 5 pixels around, inside the `MARGIN`.

use super::super::Rgb32f;
use super::Mosaic;

const TILE: usize = 128;
const MARGIN: usize = 6;
/// Difference of the two candidates' 3×3 homogeneity sums (each 0–36) from which the more homogeneous one is taken
/// alone; closer sums blend the two linearly, equal sums half and half. In a flat, noisy area noise alone decides
/// which sum is larger, and picking outright turned the noise into short horizontal and vertical strokes (a maze,
/// coloured where white balance is strong). 12 keeps the PSNR of the outright pick on the edge tests.
const BLEND: f32 = 12.0;

pub(crate) fn ahd(m: &Mosaic) -> Rgb32f {
    ahd_window(m, 0, 0, m.w, m.h)
}

/// The `w × h` pixels at (`x`, `y`) of [`ahd`]'s picture (inside the mosaic), demosaiced alone: every pixel comes
/// from the mosaic around it, so they are the same as when the whole picture is demosaiced and then cropped.
pub(crate) fn ahd_window(m: &Mosaic, x: usize, y: usize, w: usize, h: usize) -> Rgb32f {
    let mut out = Rgb32f::new(w, h);
    if w == 0 || h == 0 {
        return out;
    }
    // one row of tiles per task, its tiles in parallel too, written straight into the picture
    let process_band = |ty: usize, band: &mut [[f32; 3]]| {
        let (y0, th) = (y + ty * TILE, band.len() / w.max(1));
        #[cfg(not(target_arch = "wasm32"))]
        let tiles: Vec<(usize, usize, Vec<[f32; 3]>)> = {
            use rayon::prelude::*;
            (0..w.div_ceil(TILE))
                .into_par_iter()
                .map(|tx| {
                    let (x0, tw) = (tx * TILE, TILE.min(w - tx * TILE));
                    (x0, tw, tile(m, x + x0, y0, tw, th))
                })
                .collect()
        };
        #[cfg(target_arch = "wasm32")]
        let tiles: Vec<(usize, usize, Vec<[f32; 3]>)> = (0..w.div_ceil(TILE))
            .map(|tx| {
                let (x0, tw) = (tx * TILE, TILE.min(w - tx * TILE));
                (x0, tw, tile(m, x + x0, y0, tw, th))
            })
            .collect();
        for (x0, tw, px) in tiles {
            for (row, src) in band.chunks_exact_mut(w).zip(px.chunks_exact(tw)) {
                row[x0..x0 + tw].copy_from_slice(src);
            }
        }
    };
    crate::raster::par_rows(&mut out.data, (w * TILE).max(1), process_band);
    out
}

/// Linear camera RGB (treated as sRGB/D65 primaries for the homogeneity metric only) → CIELab.
#[inline]
fn lab(p: [f32; 3]) -> [f32; 3] {
    let x = (0.4124 * p[0] + 0.3576 * p[1] + 0.1805 * p[2]) / 0.95047;
    let y = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let z = (0.0193 * p[0] + 0.1192 * p[1] + 0.9505 * p[2]) / 1.08883;
    #[inline]
    fn f(t: f32) -> f32 {
        if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 }
    }
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn tile(m: &Mosaic, x0: usize, y0: usize, tw: usize, th: usize) -> Vec<[f32; 3]> {
    let (bw, bh) = (tw + 2 * MARGIN, th + 2 * MARGIN);
    let n = bw * bh;
    let (ox, oy) = (x0 as isize - MARGIN as isize, y0 as isize - MARGIN as isize);
    let mut mos = vec![0f32; n];
    let mut col = vec![0u8; n];
    for (y, (mrow, crow)) in mos.chunks_exact_mut(bw).zip(col.chunks_exact_mut(bw)).enumerate() {
        let ry = super::reflect(oy + y as isize, m.h);
        if ox >= 0 && ox as usize + bw <= m.w {
            // inside the image: the row as it is, and its colours repeating with the pattern
            let x0 = ry * m.w + ox as usize;
            mrow.copy_from_slice(&m.data[x0..x0 + bw]);
            let period: Vec<u8> = (0..m.cfa.width).map(|k| m.cfa.color_at(ox as usize + k, ry)).collect();
            for (c, &p) in crow.iter_mut().zip(period.iter().cycle()) {
                *c = p;
            }
        } else {
            for x in 0..bw {
                mrow[x] = m.get(ox + x as isize, oy + y as isize);
                crow[x] = m.color(ox + x as isize, oy + y as isize);
            }
        }
    }
    // flat indices of the pixels `d` or more from the buffer's edge, row by row
    let inner = |d: usize| (d..bh - d).flat_map(move |y| y * bw + d..y * bw + bw - d);
    // 1. directional greens
    let mut gh = vec![0f32; n];
    let mut gv = vec![0f32; n];
    for i in inner(MARGIN - 3) {
        let c = mos[i];
        if col[i] == 1 {
            gh[i] = c;
            gv[i] = c;
            continue;
        }
        let (l, r) = (mos[i - 1], mos[i + 1]);
        let est = (l + r) * 0.5 + (2.0 * c - mos[i - 2] - mos[i + 2]) * 0.25;
        gh[i] = super::clamp_between(est, l, r);
        let (u, d) = (mos[i - bw], mos[i + bw]);
        let est = (u + d) * 0.5 + (2.0 * c - mos[i - 2 * bw] - mos[i + 2 * bw]) * 0.25;
        gv[i] = super::clamp_between(est, u, d);
    }
    // 2. full candidates, and 3. their Lab
    // (`p` with channel `c` set to `v`, in registers: writing channels through an index and reading the pixel
    // back for its Lab stalled on store forwarding)
    let set = |p: [f32; 3], c: u8, v: f32| [if c == 0 { v } else { p[0] }, if c == 1 { v } else { p[1] }, if c >= 2 { v } else { p[2] }];
    let complete = |g: &[f32]| -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let mut rgb = vec![[0f32; 3]; n];
        let mut lab_of = vec![[0f32; 3]; n];
        for i in inner(MARGIN - 2) {
            let own = col[i];
            let gvv = g[i];
            let d = |j: usize| mos[j] - g[j];
            let p = if own == 1 {
                let p = set([0.0, gvv, 0.0], col[i + 1], gvv + (d(i - 1) + d(i + 1)) * 0.5);
                set(p, col[i + bw], gvv + (d(i - bw) + d(i + bw)) * 0.5)
            } else {
                let p = set([0.0, gvv, 0.0], own, mos[i]);
                set(p, 2 - own, gvv + (d(i - bw - 1) + d(i - bw + 1) + d(i + bw - 1) + d(i + bw + 1)) * 0.25)
            };
            rgb[i] = p;
            lab_of[i] = lab(p);
        }
        (rgb, lab_of)
    };
    let (rh, lh) = complete(&gh);
    let (rv, lv) = complete(&gv);
    // homogeneity
    let dl = |a: &[f32; 3], b: &[f32; 3]| (a[0] - b[0]).abs();
    let dc = |a: &[f32; 3], b: &[f32; 3]| (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
    let mut hh = vec![0u8; n];
    let mut hv = vec![0u8; n];
    for i in inner(MARGIN - 1) {
        // left, right, up, down
        let (l, r, u, d) = (i - 1, i + 1, i - bw, i + bw);
        let (a, b) = (lh[i], lv[i]);
        let dlh = [dl(&a, &lh[l]), dl(&a, &lh[r]), dl(&a, &lh[u]), dl(&a, &lh[d])];
        let dch = [dc(&a, &lh[l]), dc(&a, &lh[r]), dc(&a, &lh[u]), dc(&a, &lh[d])];
        let dlv = [dl(&b, &lv[l]), dl(&b, &lv[r]), dl(&b, &lv[u]), dl(&b, &lv[d])];
        let dcv = [dc(&b, &lv[l]), dc(&b, &lv[r]), dc(&b, &lv[u]), dc(&b, &lv[d])];
        let eps_l = dlh[0].max(dlh[1]).min(dlv[2].max(dlv[3]));
        let eps_c = dch[0].max(dch[1]).min(dcv[2].max(dcv[3]));
        hh[i] = (0..4).map(|k| u8::from(dlh[k] <= eps_l && dch[k] <= eps_c)).sum();
        hv[i] = (0..4).map(|k| u8::from(dlv[k] <= eps_l && dcv[k] <= eps_c)).sum();
    }
    // 4. choose
    let mut out = Vec::with_capacity(tw * th);
    for i in inner(MARGIN) {
        let (mut sh, mut sv) = (0u32, 0u32);
        for j in [i - bw - 1, i - bw, i - bw + 1, i - 1, i, i + 1, i + bw - 1, i + bw, i + bw + 1] {
            sh += hh[j] as u32;
            sv += hv[j] as u32;
        }
        let wh = (0.5 + (sh as f32 - sv as f32) / (2.0 * BLEND)).clamp(0.0, 1.0);
        // a candidate taken alone is copied, and one that isn't finite (beside a float raw's NaN sample) is left out,
        // so the other's value doesn't become 0 · NaN
        let finite = |p: &[f32; 3]| p.iter().all(|v| v.is_finite());
        out.push(if wh >= 1.0 || !finite(&rv[i]) {
            rh[i]
        } else if wh <= 0.0 || !finite(&rh[i]) {
            rv[i]
        } else {
            [0, 1, 2].map(|c| rh[i][c] * wh + rv[i][c] * (1.0 - wh))
        });
    }
    out
}
