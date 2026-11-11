//! Reconstruction: inverse DCT, upsampling and colour conversion to pixels.

use super::parse::*;
use super::*;

/// One dimension of the inverse DCT (A.3.3): `t[x][u] = C(u)/2 · cos((2x+1)uπ/16)`, and its
/// transpose.
pub(super) struct Idct {
    t: [[f32; 8]; 8],
    tt: [[f32; 8]; 8],
}

pub(super) fn idct_basis() -> Idct {
    let mut t = [[0f32; 8]; 8];
    for (x, row) in t.iter_mut().enumerate() {
        for (u, v) in row.iter_mut().enumerate() {
            let c = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
            *v = c / 2.0 * (((2 * x + 1) * u) as f32 * std::f32::consts::PI / 16.0).cos();
        }
    }
    let mut tt = [[0f32; 8]; 8];
    for (x, row) in t.iter().enumerate() {
        for (u, &v) in row.iter().enumerate() {
            tt[u][x] = v;
        }
    }
    Idct { t, tt }
}

/// One block's samples (A.3.3): `blk` (zigzag order) dequantized with `q`, inverse transformed,
/// level-shifted and clamped to `0..=max`, written to the 8 rows of 8 at the start of `out`
/// that are `stride` apart. Rows that `out` doesn't reach are left out.
pub(super) fn idct_block(blk: &[i32; 64], q: &[u16; 64], idct: &Idct, max: f32, out: &mut [f32], stride: usize) {
    let (basis, tt) = (&idct.t, &idct.tt);
    let shift = (max + 1.0) / 2.0;
    if blk[1..].iter().all(|&c| c == 0) {
        // Only DC: every sample is T[y][0] · T[x][0] · F[0][0], the same products the full
        // transform below sums (its other terms are zero), so the result is identical.
        let t = basis[0][0];
        let v = (t * (t * (blk[0] as f32 * f32::from(q[0]))) + shift).clamp(0.0, max);
        for y in 0..8 {
            if let Some(row) = out.get_mut(y * stride..y * stride + 8) {
                row.fill(v);
            }
        }
        return;
    }
    // F[v][u], dequantized, and which rows v hold a coefficient.
    let mut f = [[0f32; 8]; 8];
    let mut rows = 0u8;
    for (k, (&c, &z)) in blk.iter().zip(&ZIGZAG).enumerate() {
        if c != 0 {
            f[z / 8][z % 8] = c as f32 * f32::from(q[k]);
            rows |= 1 << (z / 8);
        }
    }
    // Rows: r[v][x] = Σu T[x][u] F[v][u]; then columns: s[y][x] = Σv T[y][v] r[v][x]. Each sum is
    // taken in order of u (of v), eight values of x at a time; a zero term changes no sum but the
    // sign of a zero, which the level shift then removes.
    let mut r = [[0f32; 8]; 8];
    for (v, (fv, rv)) in f.iter().zip(r.iter_mut()).enumerate() {
        if rows & (1 << v) == 0 {
            continue;
        }
        let mut acc = [-0.0f32; 8];
        for (&fu, tu) in fv.iter().zip(tt) {
            if fu != 0.0 {
                for (a, &t) in acc.iter_mut().zip(tu) {
                    *a += t * fu;
                }
            }
        }
        *rv = acc;
    }
    for (y, ty) in basis.iter().enumerate() {
        let mut acc = [-0.0f32; 8];
        for (v, (&t, rv)) in ty.iter().zip(&r).enumerate() {
            if rows & (1 << v) != 0 {
                for (a, &rx) in acc.iter_mut().zip(rv) {
                    *a += t * rx;
                }
            }
        }
        if let Some(row) = out.get_mut(y * stride..y * stride + 8) {
            for (o, a) in row.iter_mut().zip(acc) {
                *o = (a + shift).clamp(0.0, max);
            }
        }
    }
}

/// The component's samples (padded to whole blocks, row stride `bw * 8`), level-shifted and
/// clamped to `0..=max`.
pub(super) fn reconstruct(comp: &Component, q: &[u16; 64], idct: &Idct, max: f32) -> Vec<f32> {
    let stride = comp.bw * 8;
    let mut plane = vec![0f32; stride * comp.bh * 8];
    for (n, blk) in comp.coefs.iter().enumerate() {
        let (bx, by) = (n % comp.bw, n / comp.bw);
        if let Some(out) = plane.get_mut(by * 8 * stride + bx * 8..) {
            idct_block(blk, q, idct, max, out, stride);
        }
    }
    plane
}

/// An output coordinate's two neighboring component samples and the weight of the second.
pub(super) type UpsampleStep = (usize, usize, f32);

/// For each output coordinate: the two neighboring component samples and the weight of the
/// second (centered linear interpolation of a subsampled component).
pub(super) fn upsample_map(out: usize, samples: usize, factor: usize, fmax: usize) -> Vec<UpsampleStep> {
    let last = samples.saturating_sub(1);
    (0..out)
        .map(|x| {
            let s = ((x as f32 + 0.5) * factor as f32 / fmax as f32 - 0.5).max(0.0);
            let x0 = (s.floor() as usize).min(last);
            ((x0), (x0 + 1).min(last), (s - x0 as f32).clamp(0.0, 1.0))
        })
        .collect()
}

/// Whether a frame's components, with identifiers `ids`, hold YCbCr (or YCCK) to convert, as
/// [`Decoded::components`] describes: three components unless the Adobe transform is 0 or the
/// identifiers are `R`, `G`, `B`; four only with Adobe transform 2.
pub(super) fn is_ycc(ids: impl Iterator<Item = u8>, adobe_transform: Option<u8>) -> bool {
    let ids: Vec<u8> = ids.collect();
    match ids.len() {
        3 => adobe_transform != Some(0) && ids != *b"RGB",
        4 => adobe_transform == Some(2),
        _ => false,
    }
}

/// YCbCr → RGB (JFIF / T.871) of samples centred on `center`; for YCCK the result is the
/// inverted CMY of Adobe's convention, i.e. the ink amounts.
#[inline]
pub(super) fn ycc_to_rgb(yy: f32, cb: f32, cr: f32, center: f32) -> [f32; 3] {
    let (cb, cr) = (cb - center, cr - center);
    [yy + 1.402 * cr, yy - 0.344_136 * cb - 0.714_136 * cr, yy + 1.772 * cb]
}

/// The output colour of one pixel's `nc` reconstructed samples (`0..=max`), in place.
pub(super) fn to_color(px: &mut [f32; 4], nc: usize, ycc: bool, max: f32) {
    if ycc {
        let [r, g, b] = ycc_to_rgb(px[0], px[1], px[2], (max + 1.0) / 2.0);
        (px[0], px[1], px[2]) = (r, g, b);
        if nc == 4 {
            px[3] = max - px[3];
        }
    } else if nc == 4 {
        // Adobe-style CMYK is stored inverted (max = no ink).
        for p in px {
            *p = max - *p;
        }
    }
}

/// An 8-bit output sample: `p` (finite) clamped to `0..=255` and rounded half away from zero.
#[inline]
pub(super) fn to_u8(p: f32) -> u8 {
    let p = p.clamp(0.0, 255.0);
    // On `0..=255` the truncation and the fraction it leaves are exact, so this is `p.round()`
    // without the library call `round` is on targets without SSE4.1.
    let t = p as u8;
    t.saturating_add(u8::from(p - f32::from(t) >= 0.5))
}

/// Turn the coefficients into interleaved output samples.
pub(super) fn render(frame: Frame, tables: &Tables, adobe_transform: Option<u8>) -> Result<(u8, Samples), Error> {
    let max = if frame.precision > 8 { 4095.0 } else { 255.0 };
    let idct = idct_basis();
    let (w, h) = (frame.width, frame.height);
    let mut planes = Vec::with_capacity(frame.comps.len());
    for comp in &frame.comps {
        // A component no scan reached has only zero coefficients; any defined table will do.
        let q = comp.quant.or_else(|| tables.quant.get(comp.tq).copied().flatten());
        let q = q.ok_or_else(|| err("component uses an undefined quantization table"))?;
        let plane = reconstruct(comp, &q, &idct, max);
        let xs = upsample_map(w, comp.cw, comp.h, frame.hmax);
        let ys = upsample_map(h, comp.ch, comp.v, frame.vmax);
        planes.push((plane, comp.bw * 8, xs, ys));
    }
    let nc = planes.len();
    let sixteen = frame.precision > 8;
    let ycc = is_ycc(frame.comps.iter().map(|c| c.id), adobe_transform);
    let (mut wide, mut narrow) = (Vec::new(), Vec::new());
    if sixteen {
        wide.reserve_exact(w * h * nc);
    } else {
        narrow.reserve_exact(w * h * nc);
    }
    let mut px = [0f32; 4];
    for y in 0..h {
        for x in 0..w {
            for (c, (plane, stride, xs, ys)) in planes.iter().enumerate() {
                let (Some(&(x0, x1, fx)), Some(&(y0, y1, fy))) = (xs.get(x), ys.get(y)) else {
                    return Err(err("sample map out of range"));
                };
                let at = |xx: usize, yy: usize| plane.get(yy * stride + xx).copied().unwrap_or(0.0);
                let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
                let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
                if let Some(p) = px.get_mut(c) {
                    *p = top + (bottom - top) * fy;
                }
            }
            to_color(&mut px, nc, ycc, max);
            for &p in px.iter().take(nc) {
                let p = p.clamp(0.0, max);
                if sixteen {
                    wide.push((p * 65535.0 / max).round() as u16);
                } else {
                    narrow.push(to_u8(p));
                }
            }
        }
    }
    let samples = if sixteen { Samples::U16(wide) } else { Samples::U8(narrow) };
    Ok((nc as u8, samples))
}
