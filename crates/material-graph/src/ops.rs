// SPDX-License-Identifier: MIT OR Apache-2.0
//! Image-processing operators used by graph nodes. All are wrap-around (tileable).
// per-channel and per-pixel kernels read best with their indices
#![allow(clippy::needless_range_loop)]
use crate::measured::{
    COMPILED_8_BIT, COMPILED_16_BIT, COMPILED_HALF, CUT_16_TO_12, FINE_8, FINE_8_TIE_DOWN, FINE_16, FINE_16_BITS, FINE_16_TIE_DOWN, FORMAT_8_BIT,
    FORMAT_16_BIT, FORMAT_HALF, STEPS_8, STEPS_16, WEIGHT_STEPS,
};
use crate::tex::{GreyIn, GreyOut, Steps, Tex, Texels, lum};
use std::f32::consts::{PI, TAU};

fn sm(a: f32, b: f32, x: f32) -> f32 {
    let d = b - a;
    let t = if d.abs() < 1e-9 { if x >= b { 1.0 } else { 0.0 } } else { ((x - a) / d).clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

/// Shortest signed distance on a wrapping unit axis: maps any offset into [-0.5, 0.5], so shapes centred
/// anywhere on a tile continue seamlessly across its borders.
pub fn wrapd(d: f32) -> f32 {
    d - d.round()
}

/// Evaluate a centred shape at the wrapped offset (x, y). When the shape is larger than half a tile (`big`), the
/// neighbouring copies overlap this tile, so the union of the 3x3 copies is taken instead of only the nearest one.
pub fn tiled_max<F: Fn(f32, f32) -> f32>(big: bool, x: f32, y: f32, f: F) -> f32 {
    if !big {
        return f(x, y);
    }
    let mut m = f32::MIN;
    for dy in -1..=1 {
        for dx in -1..=1 {
            m = m.max(f(x + dx as f32, y + dy as f32));
        }
    }
    m
}

/// Make any texture tile: cross-fades it with a copy shifted by half a tile, with weights that hand over completely
/// to the shifted copy at the borders. Used when an operation cannot be periodic by itself.
pub fn seamless_blend(t: &Tex) -> Tex {
    let (w, h) = (t.w, t.h);
    let wt = |i: usize, n: usize| {
        let x = (i as f32 + 0.5) / n as f32;
        let d = (2.0 * x - 1.0).abs(); // 0 centre .. 1 border
        let k = 1.0 - d;
        k * k * (3.0 - 2.0 * k)
    };
    let mut o = t.clone();
    for y in 0..h {
        let wy = wt(y, h);
        for x in 0..w {
            let wx = wt(x, w);
            let (x2, y2) = ((x + w / 2) % w, (y + h / 2) % h);
            let a = t.px_at(y * w + x);
            let b = t.px_at(y * w + x2);
            let c = t.px_at(y2 * w + x);
            let d = t.px_at(y2 * w + x2);
            let mix = |a: f32, b: f32, c: f32, d: f32| a * wx * wy + b * (1.0 - wx) * wy + c * wx * (1.0 - wy) + d * (1.0 - wx) * (1.0 - wy);
            let r = [mix(a[0], b[0], c[0], d[0]), mix(a[1], b[1], c[1], d[1]), mix(a[2], b[2], c[2], d[2]), mix(a[3], b[3], c[3], d[3])];
            o.set_px(y * w + x, r);
        }
    }
    o
}

// ---------- blur / sharpen ----------
/// Widths (odd) of three successive box blurs whose combination approximates a gaussian of the given sigma.
pub fn gauss_boxes(sigma: f32) -> [i64; 3] {
    let n = 3.0f32;
    let w_ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    // wrapping: an infinite sigma saturates the cast, and release builds always wrapped here
    let mut wl = w_ideal.floor() as i64;
    if wl % 2 == 0 {
        wl = wl.wrapping_sub(1);
    }
    let wu = wl.wrapping_add(2);
    let m_ideal = (12.0 * sigma * sigma - n * (wl.wrapping_mul(wl)) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0);
    let m = m_ideal.round() as i64;
    [if 0 < m { wl } else { wu }, if 1 < m { wl } else { wu }, if 2 < m { wl } else { wu }]
}

/// One wrapping box blur along the rows of `src` (width w, height h), radius r, in parallel over rows.
fn box_rows(src: &[[f32; 4]], w: usize, h: usize, r: i64) -> Vec<[f32; 4]> {
    let mut dst = vec![[0.0f32; 4]; w * h];
    let inv = 1.0 / (2 * r + 1) as f32;
    let cancel = &crate::context::current_cancel();
    crate::par::for_each_run_mut(&mut dst, w.max(1), |first, out| {
        if crate::context::is_set(cancel) {
            return;
        }
        let Some(row) = src.get(first..first + w) else { return };
        let wi = w as i64;
        let mut acc = [0.0f32; 4];
        for k in -r..=r {
            let p = row.get(k.rem_euclid(wi) as usize).copied().unwrap_or([0.0; 4]);
            for (a, v) in acc.iter_mut().zip(p) {
                *a += v;
            }
        }
        let (mut ia, mut is) = ((r + 1).rem_euclid(wi), (-r).rem_euclid(wi));
        for o in out.iter_mut().take(w) {
            *o = [acc[0] * inv, acc[1] * inv, acc[2] * inv, acc[3] * inv];
            let add = row.get(ia as usize).copied().unwrap_or([0.0; 4]);
            let sub = row.get(is as usize).copied().unwrap_or([0.0; 4]);
            for ((a, ad), sb) in acc.iter_mut().zip(add).zip(sub) {
                *a += ad - sb;
            }
            ia += 1;
            if ia == wi {
                ia = 0;
            }
            is += 1;
            if is == wi {
                is = 0;
            }
        }
    });
    dst
}

/// Single-channel version of `box_rows` (grayscale images need a quarter of the memory traffic).
fn box_rows1(src: &[f32], w: usize, h: usize, r: i64) -> Vec<f32> {
    let mut dst = vec![0.0f32; w * h];
    let inv = 1.0 / (2 * r + 1) as f32;
    let cancel = &crate::context::current_cancel();
    crate::par::for_each_run_mut(&mut dst, w.max(1), |first, out| {
        if crate::context::is_set(cancel) {
            return;
        }
        let Some(row) = src.get(first..first + w) else { return };
        let wi = w as i64;
        let mut acc = 0.0f32;
        for k in -r..=r {
            acc += row.get(k.rem_euclid(wi) as usize).copied().unwrap_or(0.0);
        }
        let (mut ia, mut is) = ((r + 1).rem_euclid(wi), (-r).rem_euclid(wi));
        for o in out.iter_mut().take(w) {
            *o = acc * inv;
            acc += row.get(ia as usize).copied().unwrap_or(0.0) - row.get(is as usize).copied().unwrap_or(0.0);
            ia += 1;
            if ia == wi {
                ia = 0;
            }
            is += 1;
            if is == wi {
                is = 0;
            }
        }
    });
    dst
}

/// Vertical wrapping box blur of a single-channel image without transposing it: every piece of work owns a band of
/// columns and slides a running sum down the rows, reading whole row segments (cache friendly).
fn box_cols1(src: &[f32], w: usize, h: usize, r: i64) -> Vec<f32> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let inv = 1.0 / (2 * r + 1) as f32;
    let cancel = &crate::context::current_cancel();
    // each band of columns is blurred into a buffer of its own; the bands are stitched together afterwards (a cancelled
    // evaluation leaves bands empty)
    let bands: Vec<Vec<f32>> = crate::tex::par_map(w.div_ceil(COLUMN_BAND), |b| {
        let (x0, x1) = (b * COLUMN_BAND, ((b + 1) * COLUMN_BAND).min(w));
        let bw = x1 - x0;
        let mut out = vec![0.0f32; bw * h];
        let hi = h as i64;
        let mut acc = vec![0.0f32; bw];
        for k in -r..=r {
            let row = src.get((k.rem_euclid(hi) as usize) * w + x0..(k.rem_euclid(hi) as usize) * w + x1).unwrap_or(&[]);
            for (a, v) in acc.iter_mut().zip(row) {
                *a += *v;
            }
        }
        for (y, o) in out.chunks_exact_mut(bw).enumerate() {
            if y % 32 == 0 && crate::context::is_set(cancel) {
                break;
            }
            let ia = ((y as i64 + r + 1).rem_euclid(hi)) as usize;
            let is = ((y as i64 - r).rem_euclid(hi)) as usize;
            let (add, sub) = (src.get(ia * w + x0..ia * w + x1).unwrap_or(&[]), src.get(is * w + x0..is * w + x1).unwrap_or(&[]));
            for (((oi, a), ad), sb) in o.iter_mut().zip(acc.iter_mut()).zip(add).zip(sub) {
                *oi = *a * inv;
                *a += ad - sb;
            }
        }
        out
    });
    // stitch in parallel too: each piece fills a whole row from every column band
    let mut dst = vec![0.0f32; w * h];
    let bands = &bands;
    crate::par::for_each_run_mut(&mut dst, w, |first, d| {
        let y = first / w;
        for (b, out) in bands.iter().enumerate() {
            let bw = out.len() / h;
            let x0 = b * COLUMN_BAND;
            if let (Some(dst), Some(src)) = (d.get_mut(x0..x0 + bw), out.get(y * bw..(y + 1) * bw)) {
                dst.copy_from_slice(src);
            }
        }
    });
    dst
}

/// Columns [`box_cols1`] blurs in one piece of work.
const COLUMN_BAND: usize = 64;

fn transpose(src: &[[f32; 4]], w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut dst = vec![[0.0f32; 4]; w * h];
    // a piece of work per row of the transposed image (= column of the source)
    crate::par::for_each_run_mut(&mut dst, h.max(1), |first, out| {
        let x = first / h.max(1);
        for (y, o) in out.iter_mut().enumerate() {
            *o = src.get(y * w + x).copied().unwrap_or([0.0; 4]);
        }
    });
    dst
}

/// Gaussian blur with wrap-around (tileable) edges. Large radii use three box passes (cost independent of the radius,
/// within a few percent of a true gaussian); small radii use the exact kernel.
pub fn blur(t: &Tex, sigma: f32) -> Tex {
    if sigma < 0.1 {
        return t.clone();
    }
    let (w, h) = (t.w, t.h);
    if sigma >= 2.5 && t.px.iter().all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 1.0) {
        // grayscale: blur one channel
        let mut cur: Vec<f32> = t.px.iter().map(|p| p[0]).collect();
        for bw in gauss_boxes(sigma) {
            let r = (bw / 2).max(0);
            cur = box_rows1(&cur, w, h, r.min((w as i64 - 1) / 2));
            cur = box_cols1(&cur, w, h, r.min((h as i64 - 1) / 2));
        }
        return Tex { w, h, px: cur.into_iter().map(|v| [v, v, v, 1.0]).collect() };
    }
    if sigma >= 2.5 {
        let mut cur = t.px.clone();
        for bw in gauss_boxes(sigma) {
            let r = (bw / 2).max(0);
            // a window wider than the image would wrap more than once; the result is then simply the mean
            cur = box_rows(&cur, w, h, r.min((w as i64 - 1) / 2));
            cur = transpose(&cur, w, h);
            cur = box_rows(&cur, h, w, r.min((h as i64 - 1) / 2));
            cur = transpose(&cur, h, w);
        }
        return Tex { w, h, px: cur };
    }
    let r = ((sigma * 3.0).ceil() as i64).clamp(1, 64);
    let mut k: Vec<f32> = (-r..=r).map(|i| math::exp(-((i * i) as f32) / (2.0 * sigma * sigma))).collect();
    let s: f32 = k.iter().sum();
    k.iter_mut().for_each(|x| *x /= s);
    let mut tmp = t.clone();
    for y in 0..h {
        for x in 0..w {
            let mut a = [0.0f32; 4];
            for (i, kv) in k.iter().enumerate() {
                let p = t.at(x as i64 + i as i64 - r, y as i64);
                for (ac, pc) in a.iter_mut().zip(p) {
                    *ac += pc * kv;
                }
            }
            tmp.set_px(y * w + x, a);
        }
    }
    let mut out = tmp.clone();
    for y in 0..h {
        for x in 0..w {
            let mut a = [0.0f32; 4];
            for (i, kv) in k.iter().enumerate() {
                let p = tmp.at(x as i64, y as i64 + i as i64 - r);
                for (ac, pc) in a.iter_mut().zip(p) {
                    *ac += pc * kv;
                }
            }
            out.set_px(y * w + x, a);
        }
    }
    out
}

pub fn directional_blur(t: &Tex, angle: f32, length: f32) -> Tex {
    let (s, c) = math::sin_cos(angle * TAU);
    let n = 24;
    Tex::from_fn(t.w, t.h, |u, v| {
        let mut a = [0.0f32; 4];
        for i in 0..n {
            let f = (i as f32 / (n - 1) as f32 - 0.5) * length;
            let p = t.sample(u + c * f, v + s * f);
            for (ak, pk) in a.iter_mut().zip(p) {
                *ak += pk / n as f32;
            }
        }
        a
    })
}

pub fn sharpen(t: &Tex, radius: f32, amount: f32) -> Tex {
    let b = blur(t, radius);
    let mut o = t.clone();
    for ((p, s), bp) in o.px.iter_mut().zip(&t.px).zip(&b.px) {
        for ((pc, sc), bc) in p.iter_mut().zip(s).zip(bp).take(3) {
            // no clamp: values above 1 (HDR) and the small undershoot next to edges are kept
            *pc = sc + (sc - bc) * amount;
        }
    }
    o
}

// ---------- tonal ----------
pub fn levels(t: &Tex, in_lo: f32, in_hi: f32, gamma: f32, out_lo: f32, out_hi: f32) -> Tex {
    let d = if in_hi >= in_lo { (in_hi - in_lo).max(1e-6) } else { (in_hi - in_lo).min(-1e-6) };
    let g = 1.0 / gamma.max(1e-3);
    let unit_gamma = (g - 1.0).abs() < 1e-6;
    let lv = |x: f32| {
        let v = ((x - in_lo) / d).clamp(0.0, 1.0);
        out_lo + (out_hi - out_lo) * if unit_gamma { v } else { math::powf(v, g) }
    };
    t.map(|p| {
        if p[0] == p[1] && p[1] == p[2] {
            let v = lv(p[0]);
            return [v, v, v, p[3]];
        }
        [lv(p[0]), lv(p[1]), lv(p[2]), p[3]]
    })
}

pub fn adjust(t: &Tex, brightness: f32, contrast: f32) -> Tex {
    t.map(|p| {
        let mut o = p;
        for (oc, pc) in o.iter_mut().zip(p).take(3) {
            *oc = (pc - 0.5) * contrast + 0.5 + brightness;
        }
        o
    })
}

/// Cyclic shift by a fraction of the tile (output pixel (x, y) reads input pixel (x + fx * w, y + fy * h)).
pub fn roll<P: crate::tex::Pixel>(t: &crate::tex::Image<P>, fx: f32, fy: f32) -> crate::tex::Image<P> {
    let (w, h) = (t.w, t.h);
    if w == 0 || h == 0 {
        // the remainders below would divide by zero
        return t.clone();
    }
    let sx = ((fx * w as f32).round() as i64).rem_euclid(w as i64) as usize;
    let sy = ((fy * h as f32).round() as i64).rem_euclid(h as i64) as usize;
    crate::tex::Image { w, h, px: (0..w * h).map(|i| t.px_at(((i / w + sy) % h) * w + (i % w + sx) % w)).collect() }
}

pub fn invert(t: &Tex) -> Tex {
    t.map(|p| [1.0 - p[0], 1.0 - p[1], 1.0 - p[2], p[3]])
}

pub fn threshold(t: &Tex, th: f32, soft: f32) -> Tex {
    t.map(|p| {
        let g = sm(th - soft * 0.5, th + soft * 0.5, lum(p));
        [g, g, g, 1.0]
    })
}

/// Threshold: mode 0 greater, 1 greater or equal, 2 lower, 3 lower or equal. With softness > 0 the
/// edge is smoothed and the comparison direction is all that remains of the mode.
pub fn threshold_mode(t: &Tex, th: f32, soft: f32, mode: u32) -> Tex {
    t.map(move |p| {
        let l = lum(p);
        let g = if soft > 1e-6 {
            let g = sm(th - soft * 0.5, th + soft * 0.5, l);
            if mode >= 2 { 1.0 - g } else { g }
        } else {
            let on = match mode {
                0 => l > th,
                1 => l >= th,
                2 => l < th,
                _ => l <= th,
            };
            if on { 1.0 } else { 0.0 }
        };
        [g, g, g, 1.0]
    })
}

pub fn histogram_scan(t: &Tex, pos: f32, width: f32) -> Tex {
    let lo = pos - width * 0.5;
    let w = width.max(1e-4);
    t.map(|p| {
        let g = ((lum(p) - lo) / w).clamp(0.0, 1.0);
        [g, g, g, 1.0]
    })
}

/// Fewest levels [`quantize`] keeps: black and white (a guard against a division by zero, not measured).
const QUANTIZE_MIN_LEVELS: f32 = 2.0;

pub fn quantize(t: &Tex, steps: f32) -> Tex {
    let s = steps.max(QUANTIZE_MIN_LEVELS) - 1.0;
    t.map(|p| {
        let q = |v: f32| (v * s).round() / s;
        [q(p[0]), q(p[1]), q(p[2]), p[3]]
    })
}

pub fn to_gray(t: &Tex) -> Tex {
    t.map(|p| {
        let g = lum(p);
        [g, g, g, 1.0]
    })
}

pub fn channel(t: &Tex, ch: usize) -> Tex {
    let ch = ch.min(3);
    t.map(|p| {
        let v = match ch {
            0 => p[0],
            1 => p[1],
            2 => p[2],
            _ => p[3],
        };
        [v, v, v, 1.0]
    })
}

pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    let h = if d < 1e-9 {
        0.0
    } else if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, if mx <= 0.0 { 0.0 } else { d / mx }, mx)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

pub fn hsl_adjust(t: &Tex, hue: f32, sat: f32, val: f32) -> Tex {
    t.map(|p| {
        let (h, s, v) = rgb_to_hsv(p[0], p[1], p[2]);
        let (r, g, b) = hsv_to_rgb(h + hue, (s * sat).clamp(0.0, 1.0), (v * val).max(0.0));
        [r, g, b, p[3]]
    })
}

/// Gradient map. stops: (position, rgb).
pub fn colorize(t: &Tex, stops: &[(f32, [f32; 3])], interp: u32) -> Tex {
    let mut st: Vec<(f32, [f32; 3])> = stops.to_vec();
    st.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    t.map(|p| {
        let g = lum(p).clamp(0.0, 1.0);
        let (Some(&first), Some(&last)) = (st.first(), st.last()) else {
            return [g, g, g, p[3]];
        };
        if g <= first.0 {
            let c = first.1;
            return [c[0], c[1], c[2], p[3]];
        }
        for (&a, &b) in st.iter().zip(st.iter().skip(1)) {
            if g <= b.0 {
                let d = (b.0 - a.0).max(1e-9);
                let mut f = (g - a.0) / d;
                if interp == 1 {
                    f = f * f * (3.0 - 2.0 * f);
                } else if interp == 2 {
                    f = 0.0;
                }
                return [a.1[0] + (b.1[0] - a.1[0]) * f, a.1[1] + (b.1[1] - a.1[1]) * f, a.1[2] + (b.1[2] - a.1[2]) * f, p[3]];
            }
        }
        let c = last.1;
        [c[0], c[1], c[2], p[3]]
    })
}

// ---------- blending ----------
fn bm(mode: u32, f: f32, b: f32) -> f32 {
    match mode {
        0 => f,
        1 => f + b,
        2 => b - f,
        3 => f * b,
        4 => 1.0 - (1.0 - f) * (1.0 - b),
        5 => {
            if b < 0.5 {
                2.0 * f * b
            } else {
                1.0 - 2.0 * (1.0 - f) * (1.0 - b)
            }
        }
        6 => f.max(b),
        7 => f.min(b),
        8 => (f - b).abs(),
        9 => (1.0 - 2.0 * f) * b * b + 2.0 * f * b,
        10 => {
            if f >= 0.999 {
                1.0
            } else {
                b / (1.0 - f)
            }
        }
        11 => {
            if f <= 0.001 {
                0.0
            } else {
                1.0 - (1.0 - b) / f
            }
        }
        12 => f + b - 1.0,
        13 => {
            if f < 0.5 {
                2.0 * f * b
            } else {
                1.0 - 2.0 * (1.0 - f) * (1.0 - b)
            }
        }
        14 => f + b - 2.0 * f * b,
        15 => {
            if f <= 0.001 {
                b
            } else {
                b / f
            }
        }
        // soft light of compiled graphs: square root above the middle over the whole range
        36 => {
            if f <= 0.5 {
                b - (1.0 - 2.0 * f) * b * (1.0 - b)
            } else {
                b + (2.0 * f - 1.0) * (b.max(0.0).sqrt() - b)
            }
        }
        16 => b + 2.0 * f - 1.0,
        20 => b + (f - 0.5) * 2.0,
        _ => f,
    }
}

/// Modes: 0 copy,1 add,2 sub,3 multiply,4 screen,5 overlay,6 max,7 min,8 difference,9 soft light,10 color dodge,11 color burn,12 linear burn,13 hard light,14 exclusion,15 divide,16 linear light,17 color,18 luminosity,19 dissolve,20 add sub,21 switch (copy).
pub fn blend(fg: &Tex, bg: &Tex, mask: Option<&Tex>, mode: u32, opacity: f32) -> Tex {
    blend_ex(fg, bg, mask, mode, opacity, 1, [0.0, 0.0, 1.0, 1.0])
}

/// Blend with alpha handling and cropping area. `alpha_mode`: 0 use source alpha, 1 ignore alpha, 2 straight alpha
/// blending, 3 premultiplied alpha blending (a copy lays the foreground's colour over what its alpha leaves; other modes as 2), 4 the foreground alpha scales the opacity and the background alpha stays. `crop` = [x0, y0, x1, y1] in 0..1; outside it only the background shows.
pub fn blend_ex(fg: &Tex, bg: &Tex, mask: Option<&Tex>, mode: u32, opacity: f32, alpha_mode: u32, crop: [f32; 4]) -> Tex {
    let mut out = bg.clone();
    let (ow, oh) = (out.w, out.h);
    let cropped = crop != [0.0, 0.0, 1.0, 1.0];
    crate::tex::par_rows(&mut out.px, ow, |start, chunk| {
        for (j, dst) in chunk.iter_mut().enumerate() {
            let i = start + j;
            let f = fg.px.get(i).copied().unwrap_or([0.0; 4]);
            let b = bg.px_at(i);
            let mut m = mask.and_then(|m| m.px.get(i)).map(|p| lum(*p)).unwrap_or(1.0) * opacity;
            if cropped {
                let (px, py) = (((i % ow) as f32 + 0.5) / ow as f32, ((i / ow) as f32 + 0.5) / oh as f32);
                if px < crop[0] || px > crop[2] || py < crop[1] || py > crop[3] {
                    m = 0.0;
                }
            }
            // alpha: 1 leaves the background alpha alone, the others let the foreground alpha act as extra opacity
            // (4 keeps the background alpha too)
            let m_plain = m;
            match alpha_mode {
                // a switch picks its input whole, alpha too, whatever the foreground's alpha (measured: ivy_branch #380,
                // alpha mode 4, whose foreground's alpha is mostly 0: the reference renderer shows the foreground, mean 9.3 to 0.03;
                // stylization #101, alpha mode 2, roughness 36.6 to 0.01)
                0 | 2 | 3 | 4 if mode == 21 => dst[3] = b[3] + (f[3] - b[3]) * m,
                4 => m *= f[3],
                0 => {
                    dst[3] = b[3] + (f[3] - b[3]) * m_plain;
                    m *= f[3];
                }
                2 | 3 => {
                    dst[3] = f[3] * m_plain + b[3] * (1.0 - f[3] * m_plain);
                    m *= f[3];
                }
                _ => {}
            }
            if alpha_mode == 3 && mode == 0 {
                // premultiplied copy: the foreground's colour comes in whole, the background shows through what its alpha
                // leaves (measured: ivy_branch #377, whose foreground holds colour where its alpha is 0: mean 9.4 to 0.09)
                for ((d, fc), bc) in dst.iter_mut().zip(f).zip(b).take(3) {
                    *d = (fc * m_plain + bc * (1.0 - f[3] * m_plain)).clamp(0.0, 1.0);
                }
                continue;
            }
            if (17..=19).contains(&mode) {
                // 17 color (hue+saturation of the foreground, brightness of the background), 18 luminosity, 19 dissolve
                let set_lum = |c: [f32; 4], l: f32| {
                    let d = l - lum(c);
                    [(c[0] + d).clamp(0.0, 1.0), (c[1] + d).clamp(0.0, 1.0), (c[2] + d).clamp(0.0, 1.0)]
                };
                let r = match mode {
                    17 => set_lum(f, lum(b)),
                    18 => set_lum(b, lum(f)),
                    _ => {
                        let keep = crate::noise::rnd((i % out.w) as i32, (i / out.w) as i32, 4242) < m;
                        if keep { [f[0], f[1], f[2]] } else { [b[0], b[1], b[2]] }
                    }
                };
                let m2 = if mode == 19 { 1.0 } else { m };
                for ((d, bc), rc) in dst.iter_mut().zip(b).zip(r) {
                    *d = (bc + (rc - bc) * m2).clamp(0.0, 1.0);
                }
                continue;
            }
            if (32..=35).contains(&mode) {
                for ((d, fc), bc) in dst.iter_mut().zip(f).zip(b).take(3) {
                    *d = compiled_blend(mode, fc, bc, m).clamp(0.0, 1.0);
                }
                continue;
            }
            if mode == 21 {
                // a compiled graph's switch is a mix that keeps values outside 0..1: quantize's prefix sums hold pixel counts
                // through a chain of them (measured: the reference renderer's palette needs counts up to 65536 there)
                for ((d, fc), bc) in dst.iter_mut().zip(f).zip(b).take(3) {
                    *d = bc + (fc - bc) * m;
                }
                continue;
            }
            for ((d, fc), bc) in dst.iter_mut().zip(f).zip(b).take(3) {
                *d = (bc + (bm(mode, fc, bc).clamp(0.0, 1.0) - bc) * m).clamp(0.0, 1.0);
            }
        }
    });
    out
}

/// A blend's settings: mode, opacity, alpha mode and crop rectangle (x0, y0, x1, y1).
#[derive(Clone, Copy, Debug)]
pub(crate) struct BlendSet {
    pub mode: u32,
    pub opacity: f32,
    pub alpha_mode: u32,
    pub crop: [f32; 4],
}

/// [`blend_ex`] on grey images: the same arithmetic on the grey value as on each colour channel there, and on the alphas
/// (1 or +0) as there. `None` where the result would not be grey with one alpha of 1 or +0 bit for bit: the modes that
/// mix the channels (colour, luminosity, dissolve), a premultiplied copy, images of different sizes, or alphas that come
/// out otherwise. With `steps`, the result in the steps of the node's format where every value is one.
pub(crate) fn blend_grey(fg: GreyIn, bg: GreyIn, mask: Option<GreyIn>, set: BlendSet, steps: Option<&Steps>) -> Option<GreyOut> {
    let BlendSet { mode, opacity, alpha_mode, crop } = set;
    if (17..=19).contains(&mode) || (alpha_mode == 3 && mode == 0) {
        return None;
    }
    if fg.len() != bg.len() || mask.is_some_and(|m| m.len() != bg.len()) {
        return None;
    }
    let (ow, oh) = (bg.width(), bg.height());
    let cropped = crop != [0.0, 0.0, 1.0, 1.0];
    let pixel = |i: usize| -> (f32, f32) {
        let f = fg.texel(i);
        let b = bg.texel(i);
        let mut m = mask.map(|m| m.texel(i)).map(|p| lum([p, p, p, 1.0])).unwrap_or(1.0) * opacity;
        if cropped {
            let (px, py) = (((i % ow) as f32 + 0.5) / ow as f32, ((i / ow) as f32 + 0.5) / oh as f32);
            if px < crop[0] || px > crop[2] || py < crop[1] || py > crop[3] {
                m = 0.0;
            }
        }
        // the alpha the RGBA form gives from the two alphas, which must come out the same everywhere: 1 or +0
        let (fa, ba) = (fg.alpha(), bg.alpha());
        let m_plain = m;
        let alpha = match alpha_mode {
            0 | 2 | 3 | 4 if mode == 21 => ba + (fa - ba) * m,
            4 => {
                m *= fa;
                ba
            }
            0 => {
                let a = ba + (fa - ba) * m_plain;
                m *= fa;
                a
            }
            2 | 3 => {
                let a = fa * m_plain + ba * (1.0 - fa * m_plain);
                m *= fa;
                a
            }
            _ => ba,
        };
        let v = if (32..=35).contains(&mode) {
            compiled_blend(mode, f, b, m).clamp(0.0, 1.0)
        } else if mode == 21 {
            b + (f - b) * m
        } else {
            (b + (bm(mode, f, b).clamp(0.0, 1.0) - b) * m).clamp(0.0, 1.0)
        };
        (v, alpha)
    };
    crate::tex::fill_grey(ow, oh, pixel, steps)
}

/// The blending modes of compiled graphs in which the opacity scales the foreground (or, for divide, acts before the
/// result is limited to 0..1), so the result is not a mix of the background and the limited blend.
fn compiled_blend(mode: u32, f: f32, b: f32, opacity: f32) -> f32 {
    match mode {
        32 => b + f * opacity,
        33 => b - f * opacity,
        34 => b + (2.0 * f - 1.0) * opacity,
        _ => {
            // by zero 1, also for 0 / 0 (measured)
            let d = if f > 0.0 { b / f } else { 1.0 };
            b + (d - b) * opacity
        }
    }
}

// ---------- geometry ----------
pub fn transform(t: &Tex, ox: f32, oy: f32, sx: f32, sy: f32, rot: f32, tile: bool) -> Tex {
    let (s, c) = math::sin_cos(rot * TAU);
    let sx = sx.abs().max(1e-4);
    let sy = sy.abs().max(1e-4);
    // the resampling is periodic only if the matrix maps the tile lattice onto itself (whole numbers)
    let whole = [c / sx, s / sx, s / sy, c / sy].iter().all(|m| (m - m.round()).abs() < 1e-3);
    let out = Tex::from_fn(t.w, t.h, |u, v| {
        let x = u - 0.5;
        let y = v - 0.5;
        let rx = c * x + s * y;
        let ry = -s * x + c * y;
        let (su, sv) = (rx / sx + 0.5 - ox, ry / sy + 0.5 - oy);
        if !tile && (!(0.0..1.0).contains(&su) || !(0.0..1.0).contains(&sv)) {
            return [0.0, 0.0, 0.0, 1.0];
        }
        t.sample_cubic(su, sv)
    });
    if tile && !whole { seamless_blend(&out) } else { out }
}

pub fn mirror(t: &Tex, axis: u32) -> Tex {
    let (w, h) = (t.w as i64, t.h as i64);
    let mut o = t.clone();
    for y in 0..h {
        for x in 0..w {
            let sx = if axis == 0 || axis == 2 { x.min(w - 1 - x) } else { x };
            let sy = if axis == 1 || axis == 2 { y.min(h - 1 - y) } else { y };
            o.set_px((y * w + x) as usize, t.at(sx, sy));
        }
    }
    o
}

/// Mirror: `axis` 0 mirrors across a vertical axis, 1 across a horizontal axis, 2 both, 3 corner.
/// `ox`/`oy` are the axis positions (0..1); `inv_*` copy the other side; `corner` 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
pub fn mirror_ex(t: &Tex, axis: u32, ox: f32, oy: f32, inv_x: bool, inv_y: bool, corner: u32) -> Tex {
    let (w, h) = (t.w as i64, t.h as i64);
    // pixel on the source side stays, the other side reads the reflected pixel
    let fold = |x: i64, n: i64, o: f32, inv: bool| -> i64 {
        let ax = o.clamp(0.0, 1.0) * n as f32;
        let xc = x as f32 + 0.5;
        let source = if inv { xc >= ax } else { xc < ax };
        if source { x } else { ((2.0 * ax - xc - 0.5).round() as i64).rem_euclid(n) }
    };
    let (do_x, do_y, ix, iy, ox, oy) = match axis {
        0 => (true, false, inv_x, false, ox, 0.5),
        1 => (false, true, false, inv_y, 0.5, oy),
        2 => (true, true, inv_x, inv_y, ox, oy),
        _ => (true, true, corner == 1 || corner == 3, corner == 2 || corner == 3, 0.5, 0.5),
    };
    let mut o = t.clone();
    for y in 0..h {
        let sy = if do_y { fold(y, h, oy, iy) } else { y };
        for x in 0..w {
            let sx = if do_x { fold(x, w, ox, ix) } else { x };
            o.set_px((y * w + x) as usize, t.at(sx, sy));
        }
    }
    o
}

/// Sampling used by the warps: 0 bilinear, 1 nearest, 2 bicubic.
fn samp(t: &Tex, mode: u32, u: f32, v: f32) -> [f32; 4] {
    match mode {
        0 => t.sample(u, v),
        1 => t.at((u.rem_euclid(1.0) * t.w as f32) as i64, (v.rem_euclid(1.0) * t.h as f32) as i64),
        _ => t.sample_cubic(u, v),
    }
}

pub fn warp_ex(t: &Tex, w: &Tex, intensity: f32, filtering: u32) -> Tex {
    Tex::from_fn(t.w, t.h, |u, v| {
        let x = (u * w.w as f32) as i64;
        let y = (v * w.h as f32) as i64;
        let dx = w.gray_at(x + 1, y) - w.gray_at(x - 1, y);
        let dy = w.gray_at(x, y + 1) - w.gray_at(x, y - 1);
        samp(t, filtering, u + dx * intensity, v + dy * intensity)
    })
}

/// `offset` is subtracted from the intensity map (0.5 = mid grey does not move the pixel).
pub fn directional_warp_ex(t: &Tex, w: &Tex, angle: f32, intensity: f32, offset: f32, filtering: u32) -> Tex {
    let (s, c) = math::sin_cos(angle * TAU);
    Tex::from_fn(t.w, t.h, |u, v| {
        let d = (lum(w.sample(u, v)) - offset) * intensity;
        samp(t, filtering, u + c * d, v + s * d)
    })
}

/// Displace by the gradient of `w` (slope warp).
pub fn warp(t: &Tex, w: &Tex, intensity: f32) -> Tex {
    Tex::from_fn(t.w, t.h, |u, v| {
        let x = (u * w.w as f32) as i64;
        let y = (v * w.h as f32) as i64;
        let dx = w.gray_at(x + 1, y) - w.gray_at(x - 1, y);
        let dy = w.gray_at(x, y + 1) - w.gray_at(x, y - 1);
        t.sample_cubic(u + dx * intensity, v + dy * intensity)
    })
}

pub fn directional_warp(t: &Tex, w: &Tex, angle: f32, intensity: f32) -> Tex {
    let (s, c) = math::sin_cos(angle * TAU);
    Tex::from_fn(t.w, t.h, |u, v| {
        let d = (lum(w.sample(u, v)) - 0.5) * intensity;
        t.sample_cubic(u + c * d, v + s * d)
    })
}

// ---------- surface analysis ----------
pub fn normal_from_height(h: &Tex, strength: f32, directx: bool) -> Tex {
    let k = strength * h.w as f32 * 0.05;
    let (w, hh) = (h.w, h.h);
    // luminance once per pixel instead of nine times
    let gray: Vec<f32> = h.px.iter().map(|p| lum(*p)).collect();
    let mut out = Tex { w, h: hh, px: vec![[0.0, 0.0, 1.0, 1.0]; w * hh] };
    crate::tex::par_rows(&mut out.px, w, |start, chunk| {
        for (j, dst) in chunk.iter_mut().enumerate() {
            let i = start + j;
            let (x, y) = (i % w, i / w);
            let (xm, xp) = (if x == 0 { w - 1 } else { x - 1 }, if x + 1 == w { 0 } else { x + 1 });
            let (ym, yp) = (if y == 0 { hh - 1 } else { y - 1 }, if y + 1 == hh { 0 } else { y + 1 });
            // 3x3 Scharr kernel: smoother and more rotation-invariant than a plain central difference
            let g = |xx: usize, yy: usize| gray.get(yy * w + xx).copied().unwrap_or(0.0);
            let dx = (3.0 * (g(xp, ym) - g(xm, ym)) + 10.0 * (g(xp, y) - g(xm, y)) + 3.0 * (g(xp, yp) - g(xm, yp))) / 32.0 * k;
            let mut dy = (3.0 * (g(xm, yp) - g(xm, ym)) + 10.0 * (g(x, yp) - g(x, ym)) + 3.0 * (g(xp, yp) - g(xp, ym))) / 32.0 * k;
            if directx {
                dy = -dy;
            }
            let (nx, ny, nz) = (-dx, dy, 1.0f32);
            let l = (nx * nx + ny * ny + nz * nz).sqrt();
            *dst = [nx / l * 0.5 + 0.5, ny / l * 0.5 + 0.5, nz / l * 0.5 + 0.5, 1.0];
        }
    });
    out
}

pub fn normal_blend(a: &Tex, b: &Tex) -> Tex {
    let mut o = a.clone();
    for (i, (op, &pa)) in o.px.iter_mut().zip(&a.px).enumerate() {
        let pb = b.px.get(i).copied().unwrap_or([0.5, 0.5, 1.0, 1.0]);
        let x = (pa[0] - 0.5) * 2.0 + (pb[0] - 0.5) * 2.0;
        let y = (pa[1] - 0.5) * 2.0 + (pb[1] - 0.5) * 2.0;
        let z = (pa[2] - 0.5) * 2.0 * (pb[2] - 0.5) * 2.0;
        let l = (x * x + y * y + z * z).sqrt().max(1e-6);
        *op = [x / l * 0.5 + 0.5, y / l * 0.5 + 0.5, z / l * 0.5 + 0.5, 1.0];
    }
    o
}

/// Normal Blend with Opacity and optional Mask: `a` is the background, `b` the foreground.
pub fn normal_blend_ex(a: &Tex, b: &Tex, opacity: f32, mask: Option<&Tex>) -> Tex {
    let full = normal_blend(a, b);
    let mut o = full.clone();
    for (i, ((op, &pa), &pf)) in o.px.iter_mut().zip(&a.px).zip(&full.px).enumerate() {
        let k = (opacity * mask.and_then(|m| m.px.get(i)).map(|p| lum(*p)).unwrap_or(1.0)).clamp(0.0, 1.0);
        if k >= 1.0 {
            continue;
        }
        let mix = |pa: f32, pf: f32| (pa - 0.5) * 2.0 * (1.0 - k) + (pf - 0.5) * 2.0 * k;
        let v = [mix(pa[0], pf[0]), mix(pa[1], pf[1]), mix(pa[2], pf[2])];
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
        *op = [v[0] / l * 0.5 + 0.5, v[1] / l * 0.5 + 0.5, v[2] / l * 0.5 + 0.5, 1.0];
    }
    o
}

pub fn ambient_occlusion(h: &Tex, radius: f32, intensity: f32) -> Tex {
    let b = blur(&to_gray(h), radius);
    let g = to_gray(h);
    let mut o = g.clone();
    for ((op, bp), gp) in o.px.iter_mut().zip(&b.px).zip(&g.px) {
        let v = (1.0 - (bp[0] - gp[0]) * intensity).clamp(0.0, 1.0);
        *op = [v, v, v, 1.0];
    }
    o
}

pub fn curvature(h: &Tex, radius: f32, intensity: f32) -> Tex {
    let g = to_gray(h);
    let b = blur(&g, radius);
    let mut o = g.clone();
    for ((op, gp), bp) in o.px.iter_mut().zip(&g.px).zip(&b.px) {
        let v = (0.5 + (gp[0] - bp[0]) * intensity).clamp(0.0, 1.0);
        *op = [v, v, v, 1.0];
    }
    o
}

pub fn edge_detect(t: &Tex, intensity: f32) -> Tex {
    Tex::from_fn(t.w, t.h, |u, v| {
        let x = (u * t.w as f32) as i64;
        let y = (v * t.h as f32) as i64;
        let gx = -t.gray_at(x - 1, y - 1) - 2.0 * t.gray_at(x - 1, y) - t.gray_at(x - 1, y + 1)
            + t.gray_at(x + 1, y - 1)
            + 2.0 * t.gray_at(x + 1, y)
            + t.gray_at(x + 1, y + 1);
        let gy = -t.gray_at(x - 1, y - 1) - 2.0 * t.gray_at(x, y - 1) - t.gray_at(x + 1, y - 1)
            + t.gray_at(x - 1, y + 1)
            + 2.0 * t.gray_at(x, y + 1)
            + t.gray_at(x + 1, y + 1);
        let m = ((gx * gx + gy * gy).sqrt() * intensity).clamp(0.0, 1.0);
        [m, m, m, 1.0]
    })
}

/// Separable grey-scale dilation (maximum filter) of radius `r` pixels, wrapping at the borders.
pub fn dilate(t: &Tex, r: i64) -> Tex {
    if r <= 0 {
        return t.clone();
    }
    let h = Tex::from_fn(t.w, t.h, |u, v| {
        let (x, y) = ((u * t.w as f32) as i64, (v * t.h as f32) as i64);
        let m = (-r..=r).map(|d| t.gray_at(x + d, y)).fold(0.0f32, f32::max);
        [m, m, m, 1.0]
    });
    Tex::from_fn(t.w, t.h, |u, v| {
        let (x, y) = ((u * t.w as f32) as i64, (v * t.h as f32) as i64);
        let m = (-r..=r).map(|d| h.gray_at(x, y + d)).fold(0.0f32, f32::max);
        [m, m, m, 1.0]
    })
}

/// Edge Detect: white lines of `width` pixels where the image changes by more than `tolerance`;
/// `roundness` (pixels) rounds the corners of the lines; `invert` flips the result.
pub fn edge_detect_ex(t: &Tex, width: f32, roundness: f32, invert: bool, tolerance: f32) -> Tex {
    let g = to_gray(t);
    // Sobel magnitude, 1 for a full black / white step
    let e = Tex::from_fn(t.w, t.h, |u, v| {
        let (x, y) = ((u * t.w as f32) as i64, (v * t.h as f32) as i64);
        let gx = -g.gray_at(x - 1, y - 1) - 2.0 * g.gray_at(x - 1, y) - g.gray_at(x - 1, y + 1)
            + g.gray_at(x + 1, y - 1)
            + 2.0 * g.gray_at(x + 1, y)
            + g.gray_at(x + 1, y + 1);
        let gy = -g.gray_at(x - 1, y - 1) - 2.0 * g.gray_at(x, y - 1) - g.gray_at(x + 1, y - 1)
            + g.gray_at(x - 1, y + 1)
            + 2.0 * g.gray_at(x, y + 1)
            + g.gray_at(x + 1, y + 1);
        let m = (gx * gx + gy * gy).sqrt() * 0.25;
        let k = tolerance.clamp(0.0, 0.999);
        // everything below the tolerance is no edge; a step of the tolerance size already counts as a full edge
        let o = if m <= k * 0.5 { 0.0 } else { ((m - k * 0.5) / (0.5 * (1.0 - k)).max(1e-3)).clamp(0.0, 1.0) };
        [o, o, o, 1.0]
    });
    let mut d = dilate(&e, ((width.max(1.0) - 1.0) * 0.5).round() as i64);
    if roundness > 0.5 {
        let b = blur(&d, roundness * 0.5);
        d = Tex::from_fn(t.w, t.h, |u, v| {
            let x = sm(0.25, 0.75, lum(b.sample(u, v)));
            [x, x, x, 1.0]
        });
    }
    if invert {
        for p in d.px.iter_mut() {
            let v = 1.0 - p[0];
            *p = [v, v, v, 1.0];
        }
    }
    d
}

/// Curvature of a normal map: the divergence of its XY components (convex bright, concave dark).
pub fn curvature_normal(n: &Tex, intensity: f32, directx: bool) -> Tex {
    let s = if directx { -1.0 } else { 1.0 };
    Tex::from_fn(n.w, n.h, |u, v| {
        let (x, y) = ((u * n.w as f32) as i64, (v * n.h as f32) as i64);
        let div = (n.at(x + 1, y)[0] - n.at(x - 1, y)[0]) - s * (n.at(x, y + 1)[1] - n.at(x, y - 1)[1]);
        let g = (0.5 + div * intensity).clamp(0.0, 1.0);
        [g, g, g, 1.0]
    })
}

/// Mosaic driven by a map: every region of equal map value takes the mean colour of the input inside it.
/// `intensity` blends between the input (0) and the mosaic (1).
pub fn mosaic_map(t: &Tex, map: &Tex, intensity: f32) -> Tex {
    use std::collections::HashMap;
    let key = |u: f32, v: f32| (lum(map.sample(u, v)) * 4095.0).round() as i32;
    let mut acc: HashMap<i32, ([f64; 4], f64)> = HashMap::new();
    for y in 0..t.h {
        for x in 0..t.w {
            let (u, v) = ((x as f32 + 0.5) / t.w as f32, (y as f32 + 0.5) / t.h as f32);
            let e = acc.entry(key(u, v)).or_insert(([0.0; 4], 0.0));
            let p = t.px_at(y * t.w + x);
            for (ec, pc) in e.0.iter_mut().zip(p) {
                *ec += pc as f64;
            }
            e.1 += 1.0;
        }
    }
    let k = intensity.clamp(0.0, 1.0);
    Tex::from_fn(t.w, t.h, |u, v| {
        let src = t.sample(u, v);
        match acc.get(&key(u, v)) {
            Some((s, n)) if *n > 0.0 => {
                let mix = |sc: f64, src: f32| src + ((sc / n) as f32 - src) * k;
                [mix(s[0], src[0]), mix(s[1], src[1]), mix(s[2], src[2]), mix(s[3], src[3])]
            }
            _ => src,
        }
    })
}

pub fn emboss(t: &Tex, angle: f32, intensity: f32) -> Tex {
    let (s, c) = math::sin_cos(angle * TAU);
    Tex::from_fn(t.w, t.h, |u, v| {
        let x = (u * t.w as f32) as i64;
        let y = (v * t.h as f32) as i64;
        let dx = t.gray_at(x + 1, y) - t.gray_at(x - 1, y);
        let dy = t.gray_at(x, y + 1) - t.gray_at(x, y - 1);
        let g = (0.5 + (dx * c + dy * s) * intensity).clamp(0.0, 1.0);
        [g, g, g, 1.0]
    })
}

// ---------- shapes ----------
pub fn polygon_sdf(x: f32, y: f32, sides: f32, radius: f32, rot: f32) -> f32 {
    let n = sides.round().max(3.0);
    let a = math::atan2(y, x) - rot * TAU;
    let sector = TAU / n;
    let r = (x * x + y * y).sqrt();
    let local = a.rem_euclid(sector) - sector * 0.5;
    r * math::cos(local) - radius * math::cos(PI / n)
}

#[allow(clippy::too_many_arguments)]
pub fn bricks(u: f32, v: f32, rows: f32, cols: f32, mortar: f32, bevel: f32, offset: f32, var: f32, seed: u32) -> f32 {
    let rows = rows.round().max(1.0);
    let cols = cols.round().max(1.0);
    let row = (v * rows).floor();
    let shift = if (row as i64) % 2 != 0 { offset } else { 0.0 };
    let cx = u * cols + shift;
    let col = cx.floor();
    let fu = cx - col;
    let fv = v * rows - row;
    let e = fu.min(1.0 - fu).min(fv.min(1.0 - fv));
    let hgt = ((e - mortar) / bevel.max(1e-4)).clamp(0.0, 1.0);
    let id = crate::noise::rnd(col.rem_euclid(cols) as i32, row as i32, seed);
    hgt * (1.0 - var + var * id)
}

// ---------- scattering ----------
#[allow(clippy::too_many_arguments)]
pub fn splatter(w: usize, h: usize, pattern: Option<&Tex>, count: u32, scale: f32, scale_var: f32, rot_var: f32, lum_var: f32, seed: u32) -> Tex {
    use crate::noise::rnd;
    let inst: Vec<(f32, f32, f32, f32, f32)> = (0..count.min(2000))
        .map(|i| {
            let i = i as i32;
            (
                rnd(i, 1, seed),
                rnd(i, 2, seed),
                (scale * (1.0 + (rnd(i, 3, seed) - 0.5) * 2.0 * scale_var)).max(1e-3),
                (rnd(i, 4, seed) - 0.5) * rot_var * TAU,
                1.0 - lum_var * rnd(i, 5, seed),
            )
        })
        .collect();
    Tex::from_fn(w, h, |u, v| {
        let mut best = 0.0f32;
        for &(cx, cy, s, r, l) in &inst {
            let dx = (u - cx + 0.5).rem_euclid(1.0) - 0.5;
            let dy = (v - cy + 0.5).rem_euclid(1.0) - 0.5;
            let (sn, cs) = math::sin_cos(r);
            let lx = (cs * dx + sn * dy) / s + 0.5;
            let ly = (-sn * dx + cs * dy) / s + 0.5;
            if !(0.0..=1.0).contains(&lx) || !(0.0..=1.0).contains(&ly) {
                continue;
            }
            let val = match pattern {
                Some(p) => lum(p.sample(lx, ly)),
                None => {
                    let d = (math::powi(lx - 0.5, 2) + math::powi(ly - 0.5, 2)).sqrt() * 2.0;
                    1.0 - sm(0.8, 1.0, d)
                }
            };
            best = best.max(val * l);
        }
        [best, best, best, 1.0]
    })
}

// ---------- 3D-ish preview ----------
pub fn lit_preview(base: &Tex, normal: &Tex, rough: &Tex, metal: &Tex, ao: &Tex, light: [f32; 3]) -> Tex {
    let l = {
        let n = (light[0] * light[0] + light[1] * light[1] + light[2] * light[2]).sqrt().max(1e-6);
        [light[0] / n, light[1] / n, light[2] / n]
    };
    let hv = {
        let x = l[0];
        let y = l[1];
        let z = l[2] + 1.0;
        let n = (x * x + y * y + z * z).sqrt().max(1e-6);
        [x / n, y / n, z / n]
    };
    Tex::from_fn(base.w, base.h, |u, v| {
        let b = base.sample(u, v);
        let n = normal.sample(u, v);
        let (nx, ny, nz) = (n[0] * 2.0 - 1.0, n[1] * 2.0 - 1.0, n[2] * 2.0 - 1.0);
        let r = lum(rough.sample(u, v)).clamp(0.05, 1.0);
        let m = lum(metal.sample(u, v)).clamp(0.0, 1.0);
        let o = lum(ao.sample(u, v));
        let ndl = (nx * l[0] + ny * l[1] + nz * l[2]).max(0.0);
        let ndh = (nx * hv[0] + ny * hv[1] + nz * hv[2]).max(0.0);
        let shin = 2.0 / (r * r * r * r) - 2.0;
        let spec = math::powf(ndh, shin.clamp(1.0, 2000.0)) * (1.0 - r * 0.5);
        let mut out = [0.0; 4];
        for (oc, bc) in out.iter_mut().zip(b).take(3) {
            let diff = bc * (1.0 - m) * ndl;
            let sp = spec * (0.04 * (1.0 - m) + bc * m);
            *oc = ((0.1 * bc + diff + sp) * o).clamp(0.0, 1.0);
        }
        out[3] = 1.0;
        out
    })
}

// ---------- SDF (2D and procedural 3D) ----------
fn sd_box(x: f32, y: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = x.abs() - hx + r;
    let qy = y.abs() - hy + r;
    (math::powi(qx.max(0.0), 2) + math::powi(qy.max(0.0), 2)).sqrt() + qx.max(qy).min(0.0) - r
}

/// 2D signed distance (negative inside). shape: 0 circle, 1 box, 2 polygon, 3 ring.
#[allow(clippy::too_many_arguments)]
pub fn sdf2d(shape: u32, x: f32, y: f32, size: f32, aspect: f32, sides: f32, rot: f32, round: f32) -> f32 {
    let (s, c) = math::sin_cos(rot * TAU);
    let (rx, ry) = (c * x + s * y, -s * x + c * y);
    match shape {
        0 => (x * x + y * y).sqrt() - size,
        1 => sd_box(rx, ry, size, size * aspect.max(0.01), round.min(size * 0.99)),
        2 => polygon_sdf(x, y, sides, size, rot),
        _ => ((x * x + y * y).sqrt() - size).abs() - (size * 0.2).max(0.005),
    }
}

pub fn sdf_to_value(d: f32, range: f32) -> f32 {
    (0.5 - d / (2.0 * range.max(1e-4))).clamp(0.0, 1.0)
}

pub fn value_to_sdf(v: f32, range: f32) -> f32 {
    (0.5 - v) * 2.0 * range.max(1e-4)
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k < 1e-5 {
        return a.min(b);
    }
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b * (1.0 - h) + a * h - k * h * (1.0 - h)
}

/// op: 0 union, 1 subtract (a - b), 2 intersect. k = smoothing radius.
pub fn sdf_combine(op: u32, da: f32, db: f32, k: f32) -> f32 {
    match op {
        0 => smin(da, db, k),
        1 => -smin(-da, db, k),
        _ => -smin(-da, -db, k),
    }
}

fn map3(shape: u32, x: f32, y: f32, z: f32, s: f32, r: f32) -> f32 {
    match shape {
        0 => (x * x + y * y + z * z).sqrt() - s,
        1 => {
            let q = [x.abs() - s + r, y.abs() - s + r, z.abs() - s * 0.8 + r];
            let o = (math::powi(q[0].max(0.0), 2) + math::powi(q[1].max(0.0), 2) + math::powi(q[2].max(0.0), 2)).sqrt();
            o + q[0].max(q[1]).max(q[2]).min(0.0) - r
        }
        2 => {
            let a = ((x * x + y * y).sqrt() - s * 0.7, z);
            (a.0 * a.0 + a.1 * a.1).sqrt() - s * 0.3
        }
        _ => {
            let d = (((x * x + y * y).sqrt() - s), z.abs() - s);
            d.0.max(d.1).min(0.0) + (math::powi(d.0.max(0.0), 2) + math::powi(d.1.max(0.0), 2)).sqrt()
        }
    }
}

/// Height of a procedural 3D SDF form, ray-marched from the top. Domain-repeated and tileable.
#[allow(clippy::too_many_arguments)]
pub fn sdf_form(u: f32, v: f32, shape: u32, size: f32, repeat: f32, round: f32, scatter: f32, seed: u32, hscale: f32) -> f32 {
    let rep = repeat.round().clamp(1.0, 32.0);
    let (cu, cv) = (u * rep, v * rep);
    let (ci, cj) = (cu.floor() as i32, cv.floor() as i32);
    let (x, y) = (cu - ci as f32 - 0.5, cv - cj as f32 - 0.5);
    let (wi, wj) = (ci.rem_euclid(rep as i32), cj.rem_euclid(rep as i32));
    let r1 = crate::noise::rnd(wi, wj, seed);
    let s = (size * (1.0 - scatter * 0.6 * r1)).clamp(0.01, 0.5);
    let zc = match shape {
        0 | 3 => s,
        1 => s * 0.8,
        _ => s * 0.3,
    };
    let top = 2.0f32;
    let mut t = 0.0f32;
    for _ in 0..72 {
        let z = top - t;
        let d = map3(shape, x, y, z - zc, s, round.min(s * 0.9));
        if d < 0.0007 {
            return (z * hscale).clamp(0.0, 1.0);
        }
        t += d.max(0.003);
        if t > top {
            break;
        }
    }
    0.0
}

// =====================================================================================
// Additional nodes
// =====================================================================================
use crate::math;
use crate::noise::{rnd, voronoi_ext};

pub fn contrast_invert(t: &Tex, contrast: f32, invert: bool) -> Tex {
    t.map(|p| {
        let mut o = p;
        for (oc, pc) in o.iter_mut().zip(p).take(3) {
            let v = ((pc - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
            *oc = if invert { 1.0 - v } else { v };
        }
        o
    })
}

/// Procedural shape. kind: 0 square, 1 disc, 2 paraboloid, 3 gaussian, 4 diamond, 5 pyramid, 6 half sphere, 7 cone.
#[allow(clippy::too_many_arguments)]
pub fn shape_value(kind: u32, u: f32, v: f32, px: f32, py: f32, sx: f32, sy: f32, rot: f32, tiling: f32, soft: f32) -> f32 {
    let t = tiling.round().clamp(1.0, 32.0);
    let x = ((u - 0.5 - px) * t + 0.5).rem_euclid(1.0) - 0.5;
    let y = ((v - 0.5 - py) * t + 0.5).rem_euclid(1.0) - 0.5;
    let (s, c) = math::sin_cos(rot * TAU);
    let (rx, ry) = (c * x + s * y, -s * x + c * y);
    let nx = rx / (sx.max(1e-3) * 0.5);
    let ny = ry / (sy.max(1e-3) * 0.5);
    let r = (nx * nx + ny * ny).sqrt();
    let so = soft.clamp(0.0, 1.0).max(1e-3);
    match kind {
        0 => 1.0 - sm(1.0 - so, 1.0, nx.abs().max(ny.abs())),
        1 => 1.0 - sm(1.0 - so, 1.0, r),
        2 => (1.0 - r * r).clamp(0.0, 1.0),
        3 => math::exp(-math::powi(r * 1.8, 2)),
        4 => 1.0 - sm(1.0 - so, 1.0, nx.abs() + ny.abs()),
        5 => (1.0 - nx.abs().max(ny.abs())).clamp(0.0, 1.0),
        6 => (1.0 - r * r).max(0.0).sqrt(),
        _ => (1.0 - r).clamp(0.0, 1.0),
    }
}

pub struct TileP {
    pub nx: f32,
    pub ny: f32,
    pub size: f32,
    pub size_rand: f32,
    pub rot: f32,
    pub rot_rand: f32,
    pub pos_rand: f32,
    pub color: f32,
    pub color_rand: f32,
    pub offset: f32,
    pub seed: u32,
    pub thresh: f32,
    pub default_disc: bool,
    pub brick: bool,
}

/// Grid of pattern instances (Tile Sampler / Tile Generator).
pub fn tile_grid(w: usize, h: usize, pattern: Option<&Tex>, mask: Option<&Tex>, p: &TileP) -> Tex {
    let nx = p.nx.round().clamp(1.0, 64.0) as i32;
    let ny = p.ny.round().clamp(1.0, 64.0) as i32;
    let (fnx, fny) = (nx as f32, ny as f32);
    Tex::from_fn(w, h, |u, v| {
        let (ci, cj) = ((u * fnx).floor() as i32, (v * fny).floor() as i32);
        let mut best = 0.0f32;
        for dj in -1..=1 {
            for di in -1..=1 {
                let (i, j) = (ci + di, cj + dj);
                let (wi, wj) = (i.rem_euclid(nx), j.rem_euclid(ny));
                let shift = if p.brick && (wj & 1) == 1 { p.offset } else { 0.0 };
                let (r1, r2, r3, r4) = (
                    rnd(wi, wj, p.seed),
                    rnd(wi, wj, p.seed.wrapping_add(1)),
                    rnd(wi, wj, p.seed.wrapping_add(2)),
                    rnd(wi, wj, p.seed.wrapping_add(3)),
                );
                let cx = (i as f32 + 0.5 + shift) / fnx + (r1 - 0.5) * p.pos_rand / fnx;
                let cy = (j as f32 + 0.5) / fny + (r2 - 0.5) * p.pos_rand / fny;
                if let Some(m) = mask
                    && lum(m.sample(cx, cy)) < p.thresh
                {
                    continue;
                }
                let s = (p.size * (1.0 + (r3 - 0.5) * 2.0 * p.size_rand)).max(0.01);
                let ang = (p.rot + (r4 - 0.5) * 2.0 * p.rot_rand) * TAU;
                let col = p.color * (1.0 - p.color_rand * rnd(wi, wj, p.seed.wrapping_add(9)));
                let dx = (u - cx + 0.5).rem_euclid(1.0) - 0.5;
                let dy = (v - cy + 0.5).rem_euclid(1.0) - 0.5;
                let (sn, cs) = math::sin_cos(ang);
                let lx = (cs * dx + sn * dy) * fnx / s + 0.5;
                let ly = (-sn * dx + cs * dy) * fny / s + 0.5;
                if !(0.0..=1.0).contains(&lx) || !(0.0..=1.0).contains(&ly) {
                    continue;
                }
                let val = match pattern {
                    Some(pt) => lum(pt.sample(lx, ly)),
                    None if p.default_disc => 1.0 - sm(0.85, 1.0, (math::powi((lx - 0.5) * 2.0, 2) + math::powi((ly - 0.5) * 2.0, 2)).sqrt()),
                    None => 1.0,
                };
                best = best.max(val * col);
            }
        }
        [best, best, best, 1.0]
    })
}

/// Instances placed on a circle.
#[allow(clippy::too_many_arguments)]
pub fn splatter_circular(
    w: usize,
    h: usize,
    pattern: Option<&Tex>,
    n: u32,
    radius: f32,
    size: f32,
    size_rand: f32,
    rot: f32,
    rot_rand: f32,
    pos_rand: f32,
    seed: u32,
) -> Tex {
    let n = n.clamp(1, 256);
    let inst: Vec<(f32, f32, f32, f32)> = (0..n)
        .map(|k| {
            let i = k as i32;
            let a = k as f32 / n as f32 * TAU;
            let cx = 0.5 + radius * math::cos(a) + (rnd(i, 1, seed) - 0.5) * pos_rand * 0.2;
            let cy = 0.5 + radius * math::sin(a) + (rnd(i, 2, seed) - 0.5) * pos_rand * 0.2;
            let s = (size * (1.0 + (rnd(i, 3, seed) - 0.5) * 2.0 * size_rand)).max(0.005);
            let r = (rot + (rnd(i, 4, seed) - 0.5) * 2.0 * rot_rand) * TAU;
            (cx, cy, s, r)
        })
        .collect();
    Tex::from_fn(w, h, |u, v| {
        let mut best = 0.0f32;
        for &(cx, cy, s, r) in &inst {
            let (dx, dy) = (u - cx, v - cy);
            let (sn, cs) = math::sin_cos(r);
            let (lx, ly) = ((cs * dx + sn * dy) / s + 0.5, (-sn * dx + cs * dy) / s + 0.5);
            if !(0.0..=1.0).contains(&lx) || !(0.0..=1.0).contains(&ly) {
                continue;
            }
            let val = match pattern {
                Some(p) => lum(p.sample(lx, ly)),
                None => 1.0 - sm(0.85, 1.0, (math::powi((lx - 0.5) * 2.0, 2) + math::powi((ly - 0.5) * 2.0, 2)).sqrt()),
            };
            best = best.max(val);
        }
        [best, best, best, 1.0]
    })
}

// ---------------- flood fill family ----------------
pub fn flood_fill(t: &Tex, thresh: f32) -> Tex {
    let (w, h) = (t.w, t.h);
    let n = w * h;
    let mask: Vec<bool> = t.px.iter().map(|p| lum(*p) > thresh).collect();
    let mut label = vec![0u32; n];
    // position of every pixel relative to the region's first pixel, measured WITHOUT wrapping, so a region that
    // crosses a tile border still gets one continuous gradient
    let mut rel = vec![(0i32, 0i32); n];
    let mut bbox: Vec<[i32; 4]> = vec![[0; 4]];
    let mut stack: Vec<usize> = Vec::new();
    let mut count = 0u32;
    for s in 0..n {
        if !mask.get(s).copied().unwrap_or(false) || label.get(s) != Some(&0) {
            continue;
        }
        count += 1;
        if let Some(l) = label.get_mut(s) {
            *l = count;
        }
        stack.push(s);
        let mut bb = [0i32, 0, 0, 0];
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let (rx, ry) = rel.get(i).copied().unwrap_or((0, 0));
            bb = [bb[0].min(rx), bb[1].min(ry), bb[2].max(rx), bb[3].max(ry)];
            for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                let nx = (x as i64 + dx).rem_euclid(w as i64) as usize;
                let ny = (y as i64 + dy).rem_euclid(h as i64) as usize;
                let j = ny * w + nx;
                if mask.get(j).copied().unwrap_or(false)
                    && let (Some(l), Some(r)) = (label.get_mut(j), rel.get_mut(j))
                    && *l == 0
                {
                    *l = count;
                    *r = (rx + dx as i32, ry + dy as i32);
                    stack.push(j);
                }
            }
        }
        bbox.push(bb);
    }
    let px = label
        .iter()
        .zip(&rel)
        .map(|(&li, &(rx, ry))| {
            let Some(&b) = bbox.get(li as usize).filter(|_| li != 0) else {
                return [0.0, 0.0, 0.0, 1.0];
            };
            let gx = ((rx - b[0]) as f32 + 0.5) / ((b[2] - b[0] + 1) as f32);
            let gy = ((ry - b[1]) as f32 + 0.5) / ((b[3] - b[1] + 1) as f32);
            [0.1 + 0.9 * rnd(li as i32, 3, 17), gx, gy, 1.0]
        })
        .collect();
    Tex { w, h, px }
}

fn region_id(p: [f32; 4]) -> Option<i32> {
    if p[0] == 0.0 && p[1] == 0.0 && p[2] == 0.0 { None } else { Some((p[0] * 100_000.0) as i32) }
}

/// A flood-fill region's centre in uv, its bounding-box size in uv and its pixel count.
pub type RegionGeometry = ([f32; 2], [f32; 2], u32);

/// Per-region geometry of a flood-fill result (regions are 4-connected pixels with the same region id; they may
/// wrap around the tile border). Returns the region index of every pixel (0 = none) and, per region, the centre in
/// uv (wrapped into 0..1), the bounding-box size in uv, and the pixel count. Index 0 of the vector is the background.
pub fn flood_regions(t: &Tex) -> (Vec<u32>, Vec<RegionGeometry>) {
    let (w, h) = (t.w, t.h);
    let n = w * h;
    let ids: Vec<Option<i32>> = t.px.iter().map(|p| region_id(*p)).collect();
    let mut label = vec![0u32; n];
    let mut stats = vec![([0.0f32; 2], [0.0f32; 2], 0u32)];
    let mut stack = Vec::new();
    for s in 0..n {
        let id_s = ids.get(s).copied().flatten();
        if id_s.is_none() || label.get(s) != Some(&0) {
            continue;
        }
        let k = stats.len() as u32;
        if let Some(l) = label.get_mut(s) {
            *l = k;
        }
        stack.push((s, 0i32, 0i32));
        let (mut x0, mut x1, mut y0, mut y1, mut count) = (0i32, 0i32, 0i32, 0i32, 0u32);
        let (mut sx, mut sy) = (0i64, 0i64);
        while let Some((i, rx, ry)) = stack.pop() {
            count += 1;
            x0 = x0.min(rx);
            x1 = x1.max(rx);
            y0 = y0.min(ry);
            y1 = y1.max(ry);
            sx += rx as i64;
            sy += ry as i64;
            let (x, y) = (i % w, i / w);
            for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                let j = (y as i64 + dy).rem_euclid(h as i64) as usize * w + (x as i64 + dx).rem_euclid(w as i64) as usize;
                if let Some(l) = label.get_mut(j)
                    && *l == 0
                    && ids.get(j).copied().flatten() == id_s
                {
                    *l = k;
                    stack.push((j, rx + dx as i32, ry + dy as i32));
                }
            }
        }
        // centre of the bounding box relative to the first pixel, moved back into the tile
        let _ = (sx, sy);
        let cx = ((s % w) as f32 + 0.5 + (x0 + x1) as f32 * 0.5).rem_euclid(w as f32) / w as f32;
        let cy = ((s / w) as f32 + 0.5 + (y0 + y1) as f32 * 0.5).rem_euclid(h as f32) / h as f32;
        stats.push(([cx, cy], [(x1 - x0 + 1) as f32 / w as f32, (y1 - y0 + 1) as f32 / h as f32], count));
    }
    (label, stats)
}

/// Flood Fill to Index: every region gets its running number as a gray level (index / region count).
pub fn flood_fill_to_index(t: &Tex) -> Tex {
    let (label, stats) = flood_regions(t);
    let div = (stats.len().max(2) - 1) as f32;
    Tex {
        w: t.w,
        h: t.h,
        px: label
            .iter()
            .map(|&l| {
                if l == 0 {
                    [0.0, 0.0, 0.0, 1.0]
                } else {
                    let v = l as f32 / div;
                    [v, v, v, 1.0]
                }
            })
            .collect(),
    }
}

/// Flood Fill to Position (`size == false`) or to Bounding Box Size (`size == true`): the region's centre or extent
/// in uv as red/green.
pub fn flood_fill_to_geometry(t: &Tex, size: bool) -> Tex {
    let (label, stats) = flood_regions(t);
    Tex {
        w: t.w,
        h: t.h,
        px: label
            .iter()
            .map(|&l| match stats.get(l as usize).filter(|_| l != 0) {
                None => [0.0, 0.0, 0.0, 1.0],
                Some(s) => {
                    let v = if size { s.1 } else { s.0 };
                    [v[0], v[1], 0.0, 1.0]
                }
            })
            .collect(),
    }
}

pub fn flood_fill_random(t: &Tex, seed: u32, lo: f32, hi: f32) -> Tex {
    t.map(|p| match region_id(p) {
        None => [0.0, 0.0, 0.0, 1.0],
        Some(id) => {
            let g = lo + (hi - lo) * rnd(id, 5, seed);
            [g, g, g, 1.0]
        }
    })
}

pub fn flood_fill_color(t: &Tex, seed: u32) -> Tex {
    t.map(|p| match region_id(p) {
        None => [0.0, 0.0, 0.0, 1.0],
        Some(id) => [0.2 + 0.8 * rnd(id, 6, seed), 0.2 + 0.8 * rnd(id, 7, seed), 0.2 + 0.8 * rnd(id, 8, seed), 1.0],
    })
}

pub fn flood_fill_gradient(t: &Tex, angle: f32, variation: f32, seed: u32) -> Tex {
    t.map(|p| match region_id(p) {
        None => [0.0, 0.0, 0.0, 1.0],
        Some(id) => {
            let a = angle * TAU + (rnd(id, 9, seed) - 0.5) * variation * TAU;
            let (s, c) = math::sin_cos(a);
            let g = (0.5 + ((p[1] - 0.5) * c + (p[2] - 0.5) * s) / (c.abs() + s.abs()).max(1e-3)).clamp(0.0, 1.0);
            [g, g, g, 1.0]
        }
    })
}

// ---------------- distance / bevel ----------------
/// Approximate Euclidean distance (pixels) from each `true` pixel to the nearest `false` pixel. Wraps (tileable).
pub fn distance_field(mask: &[bool], w: usize, h: usize) -> Vec<f32> {
    let mut d: Vec<f32> = mask.iter().map(|&m| if m { 1e9 } else { 0.0 }).collect();
    let idx = |x: i64, y: i64| (y.rem_euclid(h as i64) as usize) * w + x.rem_euclid(w as i64) as usize;
    for _ in 0..2 {
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let i = idx(x, y);
                for (dx, dy, c) in [(-1i64, 0i64, 1.0f32), (0, -1, 1.0), (-1, -1, 1.414), (1, -1, 1.414)] {
                    let j = idx(x + dx, y + dy);
                    let dj = d.get(j).copied().unwrap_or(f32::INFINITY);
                    if let Some(di) = d.get_mut(i) {
                        *di = (*di).min(dj + c);
                    }
                }
            }
        }
        for y in (0..h as i64).rev() {
            for x in (0..w as i64).rev() {
                let i = idx(x, y);
                for (dx, dy, c) in [(1i64, 0i64, 1.0f32), (0, 1, 1.0), (1, 1, 1.414), (-1, 1, 1.414)] {
                    let j = idx(x + dx, y + dy);
                    let dj = d.get(j).copied().unwrap_or(f32::INFINITY);
                    if let Some(di) = d.get_mut(i) {
                        *di = (*di).min(dj + c);
                    }
                }
            }
        }
    }
    d
}

pub fn bevel(t: &Tex, dist_px: f32, smoothing: f32, thr: f32) -> Tex {
    bevel_ex(t, dist_px, smoothing, thr, false)
}

/// `round` = circular (quarter-circle) profile, otherwise the linear ramp of an angular bevel.
pub fn bevel_ex(t: &Tex, dist_px: f32, smoothing: f32, thr: f32, round: bool) -> Tex {
    let mask: Vec<bool> = t.px.iter().map(|p| lum(*p) > thr).collect();
    let d = distance_field(&mask, t.w, t.h);
    let px = d
        .iter()
        .zip(&mask)
        .map(|(d, m)| {
            if !*m {
                return [0.0, 0.0, 0.0, 1.0];
            }
            let tt = (d / dist_px.max(1.0)).clamp(0.0, 1.0);
            let tt = if round { (1.0 - (1.0 - tt) * (1.0 - tt)).sqrt() } else { tt };
            let g = tt + (tt * tt * (3.0 - 2.0 * tt) - tt) * smoothing.clamp(0.0, 1.0);
            [g, g, g, 1.0]
        })
        .collect();
    Tex { w: t.w, h: t.h, px }
}

/// mode: 0 inside, 1 outside, 2 both (0.5 = edge).
pub fn distance(t: &Tex, range_px: f32, thr: f32, mode: u32) -> Tex {
    let mask: Vec<bool> = t.px.iter().map(|p| lum(*p) > thr).collect();
    let inside = distance_field(&mask, t.w, t.h);
    let inv: Vec<bool> = mask.iter().map(|m| !m).collect();
    let outside = distance_field(&inv, t.w, t.h);
    let r = range_px.max(1.0);
    let px = inside
        .iter()
        .zip(&outside)
        .map(|(ins, out)| {
            let (a, b) = ((ins / r).clamp(0.0, 1.0), (out / r).clamp(0.0, 1.0));
            let g = match mode {
                0 => a,
                1 => b,
                _ => 0.5 + (a - b) * 0.5,
            };
            [g, g, g, 1.0]
        })
        .collect();
    Tex { w: t.w, h: t.h, px }
}

// ---------------- warps ----------------
pub fn vector_warp(t: &Tex, vecmap: &Tex, intensity: f32) -> Tex {
    Tex::from_fn(t.w, t.h, |u, v| {
        let m = vecmap.sample(u, v);
        t.sample_cubic(u + (m[0] - 0.5) * 2.0 * intensity, v + (m[1] - 0.5) * 2.0 * intensity)
    })
}

/// Vector Warp with a Vector Format option: `directx` reads the green channel as pointing down.
pub fn vector_warp_ex(t: &Tex, vecmap: &Tex, intensity: f32, directx: bool) -> Tex {
    let gs = if directx { -1.0 } else { 1.0 };
    Tex::from_fn(t.w, t.h, |u, v| {
        let m = vecmap.sample(u, v);
        t.sample_cubic(u + (m[0] - 0.5) * 2.0 * intensity, v + gs * (m[1] - 0.5) * 2.0 * intensity)
    })
}

/// mode: 0 blur (average), 1 min, 2 max.
pub fn slope_blur(t: &Tex, slope: &Tex, samples: u32, intensity: f32, mode: u32) -> Tex {
    let (sw, sh) = (slope.w, slope.h);
    let gray: Vec<f32> = slope.px.iter().map(|p| lum(*p)).collect();
    // slope direction (central differences) as a two-channel plane
    let mut gv = vec![[0.0f32; 2]; sw * sh];
    {
        let gray = &gray;
        let cancel = &crate::context::current_cancel();
        // a piece of work per 16 rows
        crate::par::for_each_run_mut(&mut gv, sw.max(1) * 16, |first, chunk| {
            for (j, dst) in chunk.iter_mut().enumerate() {
                if j % sw == 0 && crate::context::is_set(cancel) {
                    return;
                }
                let idx = first + j;
                let (x, y) = (idx % sw, idx / sw);
                let (xm, xp) = (if x == 0 { sw - 1 } else { x - 1 }, if x + 1 == sw { 0 } else { x + 1 });
                let (ym, yp) = (if y == 0 { sh - 1 } else { y - 1 }, if y + 1 == sh { 0 } else { y + 1 });
                let g = |i: usize| gray.get(i).copied().unwrap_or(0.0);
                *dst = [g(y * sw + xp) - g(y * sw + xm), g(yp * sw + x) - g(ym * sw + x)];
            }
        });
    }
    let n = samples.clamp(1, 64);
    let step = intensity / n as f32;
    let gain = slope.w as f32 * 0.5;
    let nf = n as f32;
    // bilinear lookups on plain planes (wrapped)
    #[inline]
    fn bil<const C: usize>(d: &[[f32; C]], w: usize, h: usize, u: f32, v: f32) -> [f32; C] {
        if w == 0 || h == 0 {
            // an empty slope or input image: the wrap below would divide by zero
            return [0.0; C];
        }
        let u = if u.is_finite() { u } else { 0.0 };
        let v = if v.is_finite() { v } else { 0.0 };
        let (x, y) = (u * w as f32 - 0.5, v * h as f32 - 0.5);
        let (x0, y0) = (crate::tex::ffloor(x), crate::tex::ffloor(y));
        let (fx, fy) = (x - x0, y - y0);
        let (wi, hi) = (w as i64, h as i64);
        let wrap = |i: i64, n: i64| if i >= 0 && i < n { i } else { i.rem_euclid(n) };
        let (xa, ya) = (wrap(x0 as i64, wi), wrap(y0 as i64, hi));
        let xb = if xa + 1 == wi { 0 } else { xa + 1 };
        let yb = if ya + 1 == hi { 0 } else { ya + 1 };
        let (r0, r1) = ((ya * wi) as usize, (yb * wi) as usize);
        let at = |i: usize| d.get(i).copied().unwrap_or([0.0; C]);
        let (a, b, c, e) = (at(r0 + xa as usize), at(r0 + xb as usize), at(r1 + xa as usize), at(r1 + xb as usize));
        let mut o = [0.0; C];
        for ((((oi, ai), bi), ci), ei) in o.iter_mut().zip(a).zip(b).zip(c).zip(e) {
            let top = ai + (bi - ai) * fx;
            let bot = ci + (ei - ci) * fx;
            *oi = top + (bot - top) * fy;
        }
        o
    }
    let walk = |u: f32, v: f32, mut visit: &mut dyn FnMut(f32, f32)| {
        let (mut pu, mut pv) = (u, v);
        for _ in 0..n {
            visit(pu, pv);
            let gvv = bil::<2>(&gv, sw, sh, pu, pv);
            let l = (gvv[0] * gvv[0] + gvv[1] * gvv[1]).sqrt().max(1e-6);
            let k = (l * gain).min(1.0) * step / l;
            pu += gvv[0] * k;
            pv += gvv[1] * k;
        }
        let _ = &mut visit;
    };
    let is_gray = t.px.iter().all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 1.0);
    if is_gray {
        let plane: Vec<[f32; 1]> = t.px.iter().map(|p| [p[0]]).collect();
        return Tex::from_fn(t.w, t.h, |u, v| {
            let mut acc = if mode == 1 { 1.0f32 } else { 0.0 };
            walk(u, v, &mut |pu, pv| {
                let p = bil::<1>(&plane, t.w, t.h, pu, pv)[0];
                acc = match mode {
                    1 => acc.min(p),
                    2 => acc.max(p),
                    _ => acc + p / nf,
                };
            });
            [acc, acc, acc, 1.0]
        });
    }
    Tex::from_fn(t.w, t.h, |u, v| {
        let mut acc = if mode == 1 { [1.0f32; 4] } else { [0.0f32; 4] };
        walk(u, v, &mut |pu, pv| {
            let p = bil::<4>(&t.px, t.w, t.h, pu, pv);
            for (ac, pc) in acc.iter_mut().zip(p) {
                *ac = match mode {
                    1 => (*ac).min(pc),
                    2 => (*ac).max(pc),
                    _ => *ac + pc / nf,
                };
            }
        });
        acc
    })
}

pub fn swirl(t: &Tex, amount: f32, radius: f32) -> Tex {
    let rad = radius.max(1e-3);
    Tex::from_fn(t.w, t.h, |u, v| {
        let (x, y) = (wrapd(u - 0.5), wrapd(v - 0.5));
        let r = (x * x + y * y).sqrt();
        let k = (1.0 - r / rad).clamp(0.0, 1.0);
        let a = amount * k * k * TAU;
        let (s, c) = math::sin_cos(a);
        t.sample(0.5 + c * x - s * y, 0.5 + s * x + c * y)
    })
}

pub fn highpass(t: &Tex, radius: f32) -> Tex {
    let b = blur(t, radius);
    let mut o = t.clone();
    for ((p, s), bp) in o.px.iter_mut().zip(&t.px).zip(&b.px) {
        for ((pc, sc), bc) in p.iter_mut().zip(s).zip(bp).take(3) {
            *pc = (sc - bc + 0.5).clamp(0.0, 1.0);
        }
    }
    o
}

// ---------------- colour / mask helpers ----------------
pub fn color_match(t: &Tex, reference: &Tex, strength: f32) -> Tex {
    let stats = |x: &Tex| -> ([f32; 3], [f32; 3]) {
        let n = x.px.len().max(1) as f32;
        let mut m = [0.0f32; 3];
        for p in &x.px {
            for (mc, pc) in m.iter_mut().zip(p) {
                *mc += pc / n;
            }
        }
        let mut sd = [0.0f32; 3];
        for p in &x.px {
            for ((sc, pc), mc) in sd.iter_mut().zip(p).zip(m) {
                *sc += math::powi(pc - mc, 2) / n;
            }
        }
        (m, [sd[0].sqrt(), sd[1].sqrt(), sd[2].sqrt()])
    };
    let ((m1, s1), (m2, s2)) = (stats(t), stats(reference));
    t.map(|p| {
        let mut o = p;
        for (((((oc, pc), m1c), s1c), s2c), m2c) in o.iter_mut().zip(p).zip(m1).zip(s1).zip(s2).zip(m2) {
            let matched = (pc - m1c) * (s2c / s1c.max(1e-4)) + m2c;
            *oc = (pc + (matched - pc) * strength.clamp(0.0, 1.0)).clamp(0.0, 1.0);
        }
        o
    })
}

/// Curvature from a tangent-space normal map (divergence of the normal), smoothed by `smooth_px`.
pub fn curvature_from_normal(n: &Tex, smooth_px: f32, intensity: f32) -> Tex {
    let c = Tex::from_fn(n.w, n.h, |u, v| {
        let (x, y) = ((u * n.w as f32) as i64, (v * n.h as f32) as i64);
        let div = (n.at(x + 1, y)[0] - n.at(x - 1, y)[0]) - (n.at(x, y + 1)[1] - n.at(x, y - 1)[1]);
        let g = (0.5 + div * intensity).clamp(0.0, 1.0);
        [g, g, g, 1.0]
    });
    blur(&c, smooth_px)
}

/// How much of the foreground height `fg` shows over the background height `bg` (0..1): the foreground wins
/// where it is higher, `offset` (0..1, 0.5 neutral) raises it, `contrast` sharpens the transition.
pub fn height_blend_factor(bg: f32, fg: f32, offset: f32, contrast: f32) -> f32 {
    let score = (fg + (offset - 0.5) * 2.0) - bg;
    (0.5 + score * contrast.max(0.05) * 2.0).clamp(0.0, 1.0) * sm(0.0, 0.08, offset)
}

/// Returns (height, blend factor).
pub fn height_blend(bg: &Tex, fg: &Tex, mask: Option<&Tex>, opacity: f32, contrast: f32) -> (Tex, Tex) {
    let mut h = bg.clone();
    let mut f = bg.clone();
    for (i, (hp, fp)) in h.px.iter_mut().zip(f.px.iter_mut()).enumerate() {
        let m = mask.and_then(|m| m.px.get(i)).map(|p| lum(*p)).unwrap_or(1.0) * opacity;
        let (b, g) = (lum(bg.px_at(i)), lum(fg.px.get(i).copied().unwrap_or([0.0; 4])));
        let k = height_blend_factor(b, g, m, contrast);
        let out = b + (g - b) * k;
        *hp = [out, out, out, 1.0];
        *fp = [k, k, k, 1.0];
    }
    (h, f)
}

pub fn make_tile_photo(t: &Tex, width: f32) -> Tex {
    make_tile_photo_ex(t, width, width)
}

/// Separate horizontal and vertical transition reach (Mask Size H / V).
pub fn make_tile_photo_ex(t: &Tex, width_h: f32, width_v: f32) -> Tex {
    let (wh, wv) = (width_h.clamp(0.1, 1.0), width_v.clamp(0.1, 1.0));
    let a = |x: f32, w: f32| sm(0.0, w, 1.0 - (2.0 * x - 1.0).abs());
    Tex::from_fn(t.w, t.h, |u, v| {
        let (au, av) = (a(u, wh), a(v, wv));
        let s = |du: f32, dv: f32| t.sample((u + du).rem_euclid(1.0), (v + dv).rem_euclid(1.0));
        let (c00, c10, c01, c11) = (s(0.0, 0.0), s(0.5, 0.0), s(0.0, 0.5), s(0.5, 0.5));
        let (w00, w10, w01, w11) = (au * av, (1.0 - au) * av, au * (1.0 - av), (1.0 - au) * (1.0 - av));
        let mix = |a: f32, b: f32, c: f32, d: f32| a * w00 + b * w10 + c * w01 + d * w11;
        [
            mix(c00[0], c10[0], c01[0], c11[0]),
            mix(c00[1], c10[1], c01[1], c11[1]),
            mix(c00[2], c10[2], c01[2], c11[2]),
            mix(c00[3], c10[3], c01[3], c11[3]),
        ]
    })
}

pub fn make_tile_patch(t: &Tex, patches: f32, softness: f32, seed: u32) -> Tex {
    let p = patches.round().clamp(2.0, 32.0) as i32;
    let soft = softness.clamp(0.01, 1.0) * 0.3;
    Tex::from_fn(t.w, t.h, |u, v| {
        let vr = voronoi_ext(u, v, p, p, seed, 1.0);
        let off = |id: f32| ((id * 10_000.0) as i32, rnd((id * 10_000.0) as i32, 1, seed), rnd((id * 10_000.0) as i32, 2, seed));
        let (a, b) = (off(vr.id1), off(vr.id2));
        let (s1, s2) = (t.sample(u + a.1, v + a.2), t.sample(u + b.1, v + b.2));
        let k = 0.5 + 0.5 * sm(0.0, soft, vr.f2 - vr.f1);
        crate::tex::zip4(s1, s2, |a, b| b + (a - b) * k)
    })
}

/// Seamless stripe direction. A stripe pattern `frac(a*u + b*v)` repeats exactly across the unit tile
/// only when (a, b) are integers, so an arbitrary angle cannot be exact. This returns the integer wave
/// vector whose direction is closest to `angle_turns` while keeping the stripe count within about
/// -40%/+60% of `count` (angle error is weighted most; count error is penalised softly).
pub fn tileable_wave(count: f32, angle_turns: f32) -> (i32, i32) {
    let c = (count as f64).clamp(1.0, 8192.0);
    let th = (angle_turns as f64) * std::f64::consts::TAU;
    let (st, ct) = math::sin_cos(th);
    let (lo, hi) = ((c * 0.6).max(1.0), (c * 1.6).max(1.5));
    let horizontal = ct.abs() >= st.abs();
    let prim = if horizontal { ct.abs() } else { st.abs() };
    let m = (hi * prim).ceil() as i64 + 2;
    let mut best = (f64::MAX, (c.round() as i32).max(1), 0i32);
    for p in -m..=m {
        let sec = if horizontal { p as f64 * st / ct } else { p as f64 * ct / st };
        let base = sec.floor() as i64;
        for s in (base - 1)..=(base + 2) {
            let (a, b) = if horizontal { (p, s) } else { (s, p) };
            if a == 0 && b == 0 {
                continue;
            }
            let len = ((a * a + b * b) as f64).sqrt();
            if len < lo || len > hi {
                continue;
            }
            let d = math::atan2(b as f64, a as f64) - th;
            let dth = math::atan2(math::sin(d), math::cos(d)).abs();
            let score = dth + 0.5 * math::ln(len / c).abs();
            if score < best.0 {
                best = (score, a as i32, b as i32);
            }
        }
    }
    (best.1, best.2)
}

/// Resample a texture to `size` x `size` (wrapped). Shrinking by a whole factor averages the covered texels exactly
/// (no aliasing); anything else is bilinear.
pub fn resize(t: &Tex, size: usize) -> Tex {
    resize_to(t, size, size)
}

/// Resample to `w` x `h`: exact box averaging for whole-number reductions, bilinear (wrapped) otherwise.
/// For compiled graphs: one bilinear read at each result pixel, keeping the image's edge rather than wrapping around. Also when
/// shrinking: measured with impulses, a node reading an input 8 times larger sees only the 2 x 2 pixels around its centre.
pub fn resize_keep_edge(t: &Tex, w: usize, h: usize) -> Tex {
    let (w, h) = (w.max(1), h.max(1));
    let (hx, hy) = (0.5 / t.w as f32, 0.5 / t.h as f32);
    Tex::from_fn(w, h, |u, v| t.sample(u.clamp(hx, 1.0 - hx), v.clamp(hy, 1.0 - hy)))
}

pub fn resize_to<P: crate::tex::Pixel>(t: &crate::tex::Image<P>, w: usize, h: usize) -> crate::tex::Image<P> {
    let (w, h) = (w.max(1), h.max(1));
    if t.w == w && t.h == h {
        return t.clone();
    }
    if t.w >= w && t.h >= h && t.w.is_multiple_of(w) && t.h.is_multiple_of(h) {
        let (fx, fy) = (t.w / w, t.h / h);
        let inv = 1.0 / (fx * fy) as f32;
        return crate::tex::Image::from_fn(w, h, |u, v| {
            let (x0, y0) = (((u * w as f32) as usize).min(w - 1) * fx, ((v * h as f32) as usize).min(h - 1) * fy);
            let mut a = [0.0f32; 4];
            for y in y0..y0 + fy {
                for x in x0..x0 + fx {
                    let p = t.px_at(y * t.w + x);
                    for (c, ac) in a.iter_mut().enumerate().take(P::CH) {
                        *ac += p.ch(c);
                    }
                }
            }
            P::from_ch(|c| crate::tex::get(&a, c) * inv)
        });
    }
    crate::tex::Image::from_fn(w, h, |u, v| t.sample(u, v))
}

// ---------- gradient map with alpha, addressing and smoothness ----------
/// Gradient map. `stops` are (position, rgb, alpha). `interp`: 0 linear, 1 smooth (`smoothness` blends linear and
/// smoothstep), 2 step, 3 smoother. `repeat` wraps the input instead of clamping it; `use_alpha` writes the
/// gradient's alpha into the result (otherwise the input's alpha is kept).
/// A gradient read through its table, as a texture: entries in 16-bit steps, weight in steps of 1/256, the read rounded half up to a 16-bit
/// step (measured on a fine ramp over a steep part: 75% of pixels exact, where straight lines between exact entries get 42%). measurement
/// "What nodes hold and how they read" ("Gradient map through its 256 table").
pub fn colorize_table(t: &Tex, table: &Tex, use_alpha: bool) -> Tex {
    let entries: Vec<[f64; 4]> = table.px.iter().map(|p| p.map(|c| (f64::from(c) * STEPS_16).round())).collect();
    let last = entries.len().saturating_sub(1);
    t.map(|p| {
        let pos = f64::from(lum(p).clamp(0.0, 1.0)) * last as f64;
        let k = (pos.floor() as usize).min(last.saturating_sub(1));
        let w = ((pos - k as f64) * WEIGHT_STEPS).round() / WEIGHT_STEPS;
        // an empty table reads as black (it used to panic)
        let (lo, hi) = (entries.get(k).copied().unwrap_or([0.0; 4]), entries.get((k + 1).min(last)).copied().unwrap_or([0.0; 4]));
        let read = |a: f64, b: f64| (((a * (1.0 - w) + b * w) + 0.5).floor() / STEPS_16) as f32;
        [read(lo[0], hi[0]), read(lo[1], hi[1]), read(lo[2], hi[2]), if use_alpha { read(lo[3], hi[3]) } else { p[3] }]
    })
}

pub fn colorize_ex(t: &Tex, stops: &[(f32, [f32; 3], f32)], interp: u32, smoothness: f32, repeat: bool, use_alpha: bool) -> Tex {
    let mut st: Vec<(f32, [f32; 3], f32)> = stops.to_vec();
    st.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let sm = smoothness.clamp(0.0, 1.0);
    t.map(|p| {
        let mut g = lum(p);
        g = if repeat { g.rem_euclid(1.0) } else { g.clamp(0.0, 1.0) };
        let out = |c: [f32; 3], a: f32| [c[0], c[1], c[2], if use_alpha { a } else { p[3] }];
        let (Some(&first), Some(&e)) = (st.first(), st.last()) else {
            return [g, g, g, p[3]];
        };
        if g <= first.0 {
            return out(first.1, first.2);
        }
        for (&a, &b) in st.iter().zip(st.iter().skip(1)) {
            if g <= b.0 {
                let mut f = (g - a.0) / (b.0 - a.0).max(1e-9);
                f = match interp {
                    1 => f + (f * f * (3.0 - 2.0 * f) - f) * sm,
                    2 => 0.0,
                    3 => f * f * f * (f * (f * 6.0 - 15.0) + 10.0),
                    _ => f,
                };
                let l = |x: f32, y: f32| x + (y - x) * f;
                return out([l(a.1[0], b.1[0]), l(a.1[1], b.1[1]), l(a.1[2], b.1[2])], l(a.2, b.2));
            }
        }
        out(e.1, e.2)
    })
}

// ---------- colour match (colour swap in OKLCh) ----------
fn to_lin(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { math::powf((c + 0.055) / 1.055, 2.4) }
}
fn from_lin(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * math::powf(c, 1.0 / 2.4) - 0.055 }
}
/// sRGB -> (L, C, h in turns)
pub fn rgb_to_lch(c: [f32; 3]) -> [f32; 3] {
    let (r, g, b) = (to_lin(c[0]), to_lin(c[1]), to_lin(c[2]));
    let l = math::cbrt(0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b);
    let m = math::cbrt(0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b);
    let s = math::cbrt(0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b);
    let ll = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let aa = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    [ll, (aa * aa + bb * bb).sqrt(), (math::atan2(bb, aa) / std::f32::consts::TAU).rem_euclid(1.0)]
}
pub fn lch_to_rgb(c: [f32; 3]) -> [f32; 3] {
    let (aa, bb) = (math::cos(c[2] * std::f32::consts::TAU) * c[1], math::sin(c[2] * std::f32::consts::TAU) * c[1]);
    let l = math::powi(c[0] + 0.396_337_78 * aa + 0.215_803_76 * bb, 3);
    let m = math::powi(c[0] - 0.105_561_346 * aa - 0.063_854_17 * bb, 3);
    let s = math::powi(c[0] - 0.089_484_18 * aa - 1.291_485_5 * bb, 3);
    [
        from_lin(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        from_lin(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s),
        from_lin(-0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s),
    ]
}
pub fn mean_rgb(t: &Tex) -> [f32; 3] {
    let n = t.px.len().max(1) as f32;
    let mut m = [0.0f32; 3];
    for p in &t.px {
        for (mc, pc) in m.iter_mut().zip(p) {
            *mc += pc / n;
        }
    }
    m
}

/// Parameters of the colour swap.
pub struct ColorSwap {
    pub source: [f32; 3],
    pub target: [f32; 3],
    pub hue_shift: f32, // turns
    pub chroma_mul: f32,
    pub luma_mul: f32,
    pub use_mask: bool,
    pub hue_range: f32, // turns
    pub chroma_range: f32,
    pub luma_range: f32,
    pub smooth: f32,
    pub strength: f32,
}

/// Moves every colour near `source` towards `target` (hue rotated, chroma and luma scaled by the ratio between the two,
/// each times its variation) and returns (result, mask). The mask is how much each pixel was selected.
pub fn color_swap(t: &Tex, c: &ColorSwap) -> (Tex, Tex) {
    let (s, g) = (rgb_to_lch(c.source), rgb_to_lch(c.target));
    let dh = g[2] - s[2] + c.hue_shift;
    let cr = (g[1] / s[1].max(1e-3)).clamp(0.0, 8.0) * c.chroma_mul;
    let lr = math::powf((g[0] / s[0].max(1e-3)).clamp(0.0, 8.0), 1.0) * c.luma_mul;
    // the chroma ratio is meaningless for a grey source: fall back to adding the target's chroma
    let grey = s[1] < 0.01;
    let wrap = |d: f32| {
        let d = d.rem_euclid(1.0);
        d.min(1.0 - d)
    };
    let calc = |p: [f32; 4]| -> ([f32; 4], f32) {
        let q = rgb_to_lch([p[0], p[1], p[2]]);
        let w = if c.use_mask {
            let d = (wrap(q[2] - s[2]) / c.hue_range.max(1e-4))
                .max((q[1] - s[1]).abs() / c.chroma_range.max(1e-4))
                .max((q[0] - s[0]).abs() / c.luma_range.max(1e-4));
            1.0 - ((d - 1.0) / c.smooth.max(0.02)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let w = (w * c.strength).clamp(0.0, 1.0);
        let nc = if grey { q[1] + g[1] * c.chroma_mul } else { q[1] * cr };
        let nl = (q[0] * lr).clamp(0.0, 1.2);
        let res = lch_to_rgb([nl, nc.max(0.0), (q[2] + dh).rem_euclid(1.0)]);
        ([p[0] + (res[0] - p[0]) * w, p[1] + (res[1] - p[1]) * w, p[2] + (res[2] - p[2]) * w, p[3]], w)
    };
    (
        t.map(|p| calc(p).0),
        t.map(|p| {
            let w = calc(p).1;
            [w, w, w, 1.0]
        }),
    )
}

// ---------- horizon based ambient occlusion ----------
/// Horizon-based AO of a height map. `radius_uv` is the search distance in image units (0..1), `height_scale` how tall
/// a height of 1.0 is in the same units (screen-relative: the "height depth"; world units: height cm / surface cm),
/// `dirs` the number of directions (4 / 8 / 16). Wraps at the borders, so tileable inputs give tileable output.
pub fn hbao(h: &Tex, radius_uv: f32, height_scale: f32, dirs: u32, intensity: f32) -> Tex {
    let g = to_gray(h);
    let (w, hh) = (g.w, g.h);
    let dirs = dirs.clamp(2, 64) as usize;
    let steps = 14usize;
    let tab: Vec<(f32, f32)> = (0..dirs)
        .map(|k| (math::cos(k as f32 / dirs as f32 * std::f32::consts::TAU), math::sin(k as f32 / dirs as f32 * std::f32::consts::TAU)))
        .collect();
    let rows = crate::tex::par_map(hh, |y| {
        let mut row = vec![[0.0f32; 4]; w];
        for (x, px) in row.iter_mut().enumerate() {
            let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / hh as f32);
            let h0 = g.px_at(y * w + x)[0];
            let mut occ = 0.0f32;
            for &(dx, dy) in &tab {
                let mut best = 0.0f32;
                for j in 1..=steps {
                    let t = j as f32 / steps as f32;
                    let d = radius_uv * t;
                    let hs = g.sample(u + dx * d, v + dy * d)[0];
                    let dz = (hs - h0) * height_scale;
                    let s = dz / (dz * dz + d * d).sqrt().max(1e-9);
                    let falloff = 1.0 - t * t;
                    best = best.max(s * falloff);
                }
                occ += best;
            }
            let a = (1.0 - occ / dirs as f32 * intensity).clamp(0.0, 1.0);
            *px = [a, a, a, 1.0];
        }
        row
    });
    let mut o = g.clone();
    for (y, r) in rows.into_iter().enumerate() {
        if let Some(dst) = o.px.get_mut(y * w..(y + 1) * w) {
            dst.copy_from_slice(&r);
        }
    }
    o
}

// ---------- colour quantisation (k-means) ----------
fn oklab(c: [f32; 3]) -> [f32; 3] {
    let l = rgb_to_lch(c);
    let a = math::cos(l[2] * std::f32::consts::TAU) * l[1];
    let b = math::sin(l[2] * std::f32::consts::TAU) * l[1];
    [l[0], a, b]
}

// Colour quantisation is this engine's own node, not a measured one: these are choices.
/// Most colours [`quantize_color`] keeps (a guard).
const MAX_PALETTE: usize = 128;
/// Rounds of k-means over the histogram.
const KMEANS_PASSES: usize = 10;
/// Dither amplitude, over the cube root of the palette size: about the spacing of the palette's colours.
const DITHER_SPREAD: f32 = 0.6;
/// Seed of the hash noise dither.
const NOISE_DITHER_SEED: u32 = 4242;

/// Reduces `t` to at most `k` colours chosen by k-means over the image's colour histogram. `smoothing` blurs the input
/// first (pixels) so contours between colour regions are smoother; `dither` (0..1) adds an ordered pattern
/// (`bayer`) or a hash noise before choosing the nearest colour; `lab` measures distance in OKLab instead of RGB.
pub fn quantize_color(t: &Tex, k: usize, smoothing: f32, dither: f32, bayer: bool, lab: bool, ignore_alpha: bool) -> Tex {
    quantize_color_full(t, k, smoothing, dither, bayer, lab, ignore_alpha).0
}

/// Like `quantize_color`, also returning the palette index of every pixel and the palette (RGB).
pub fn quantize_color_full(
    t: &Tex,
    k: usize,
    smoothing: f32,
    dither: f32,
    bayer: bool,
    lab: bool,
    ignore_alpha: bool,
) -> (Tex, Vec<u32>, Vec<[f32; 3]>) {
    let src = if smoothing > 0.0 { blur(t, smoothing) } else { t.clone() };
    let k = k.clamp(1, MAX_PALETTE);
    let space = |p: [f32; 4]| if lab { oklab([p[0], p[1], p[2]]) } else { [p[0], p[1], p[2]] };
    // histogram with 24 bins per channel (a choice): [count, sum of the three channels in the working space]
    const B: usize = 24;
    let mut hist: std::collections::HashMap<usize, (f64, [f64; 3], [f64; 3])> = std::collections::HashMap::new();
    for p in &src.px {
        let key = ((p[0].clamp(0.0, 1.0) * (B as f32 - 1e-3)) as usize * B + (p[1].clamp(0.0, 1.0) * (B as f32 - 1e-3)) as usize) * B
            + (p[2].clamp(0.0, 1.0) * (B as f32 - 1e-3)) as usize;
        let e = hist.entry(key).or_insert((0.0, [0.0; 3], [0.0; 3]));
        let s = space(*p);
        e.0 += 1.0;
        for (ec, sc) in e.1.iter_mut().zip(s) {
            *ec += sc as f64;
        }
        for (ec, pc) in e.2.iter_mut().zip(p) {
            *ec += *pc as f64;
        }
    }
    let mut bins: Vec<(usize, f64, [f32; 3], [f32; 3])> = hist
        .into_iter()
        .map(|(key, (n, s, r))| {
            (key, n, [(s[0] / n) as f32, (s[1] / n) as f32, (s[2] / n) as f32], [(r[0] / n) as f32, (r[1] / n) as f32, (r[2] / n) as f32])
        })
        .collect();
    bins.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0))); // deterministic
    let Some(first) = bins.first().map(|b| b.2) else {
        return (t.clone(), vec![0; t.px.len()], vec![[0.0; 3]]);
    };
    let d2 = |a: [f32; 3], b: [f32; 3]| math::powi(a[0] - b[0], 2) + math::powi(a[1] - b[1], 2) + math::powi(a[2] - b[2], 2);
    // index of the centre nearest to `q` (the first of equals)
    let nearest = |cent: &[[f32; 3]], q: [f32; 3]| {
        cent.iter()
            .enumerate()
            .min_by(|(_, x), (_, y)| d2(**x, q).partial_cmp(&d2(**y, q)).unwrap_or(std::cmp::Ordering::Equal))
            .map_or(0, |(j, _)| j)
    };
    // farthest-point initialisation starting from the most common colour
    let mut cent: Vec<[f32; 3]> = vec![first];
    while cent.len() < k.min(bins.len()) {
        let mut best = (0.0f32, 0usize);
        for (i, b) in bins.iter().enumerate() {
            let d = cent.iter().map(|c| d2(*c, b.2)).fold(f32::MAX, f32::min) * (b.1 as f32).sqrt();
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 <= 0.0 {
            break;
        }
        match bins.get(best.1) {
            Some(b) => cent.push(b.2),
            None => break,
        }
    }
    for _ in 0..KMEANS_PASSES {
        let mut acc = vec![([0.0f64; 3], 0.0f64); cent.len()];
        for b in &bins {
            if let Some(a) = acc.get_mut(nearest(&cent, b.2)) {
                for (ac, bc) in a.0.iter_mut().zip(b.2) {
                    *ac += bc as f64 * b.1;
                }
                a.1 += b.1;
            }
        }
        for (c, a) in cent.iter_mut().zip(&acc) {
            if a.1 > 0.0 {
                *c = [(a.0[0] / a.1) as f32, (a.0[1] / a.1) as f32, (a.0[2] / a.1) as f32];
            }
        }
    }
    // palette in RGB: mean RGB of the pixels (bins) that fall to each centre
    let mut pal = vec![([0.0f64; 3], 0.0f64); cent.len()];
    for b in &bins {
        if let Some(p) = pal.get_mut(nearest(&cent, b.2)) {
            for (pc, bc) in p.0.iter_mut().zip(b.3) {
                *pc += bc as f64 * b.1;
            }
            p.1 += b.1;
        }
    }
    let pal: Vec<[f32; 3]> = pal
        .iter()
        .zip(&cent)
        .map(|(p, c)| if p.1 > 0.0 { [(p.0[0] / p.1) as f32, (p.0[1] / p.1) as f32, (p.0[2] / p.1) as f32] } else { *c })
        .collect();
    const BAYER: [[f32; 8]; 8] = [
        [0., 32., 8., 40., 2., 34., 10., 42.],
        [48., 16., 56., 24., 50., 18., 58., 26.],
        [12., 44., 4., 36., 14., 46., 6., 38.],
        [60., 28., 52., 20., 62., 30., 54., 22.],
        [3., 35., 11., 43., 1., 33., 9., 41.],
        [51., 19., 59., 27., 49., 17., 57., 25.],
        [15., 47., 7., 39., 13., 45., 5., 37.],
        [63., 31., 55., 23., 61., 29., 53., 21.],
    ];
    let spread = dither * DITHER_SPREAD / math::cbrt(cent.len() as f32);
    let w = src.w;
    let mut out = src.clone();
    let mut ids = vec![0u32; src.px.len()];
    for (i, ((p, o), id)) in src.px.iter().zip(out.px.iter_mut()).zip(ids.iter_mut()).enumerate() {
        let (x, y) = (i % w, i / w);
        let off = if dither > 0.0 {
            let n = if bayer {
                BAYER.get(y % 8).and_then(|r| r.get(x % 8)).copied().unwrap_or(0.0) / 64.0
            } else {
                rnd(x as i32, y as i32, NOISE_DITHER_SEED)
            };
            (n - 0.5) * spread
        } else {
            0.0
        };
        let q = space([p[0] + off, p[1] + off, p[2] + off, p[3]]);
        let j = nearest(&cent, q);
        let pj = pal.get(j).copied().unwrap_or([0.0; 3]);
        *o = [pj[0], pj[1], pj[2], if ignore_alpha { 1.0 } else { p[3] }];
        *id = j as u32;
    }
    (out, ids, pal)
}

/// Per-node output precision (the node Output Format): 1 = 8 bit integer, 2 = 16 bit integer (both clamp to
/// 0..1 and quantise), 3 = 16 bit float (rounds to half precision, keeps values above 1), 5 to 7 as the compiled format's
/// nodes hold them ([`compiled_8bit`], [`compiled_16bit`], [`compiled_half`]), anything else = 32 bit float (untouched).
/// Alpha is treated like the colour channels. The codes are in [`crate::measured`].
pub fn apply_format<P: crate::tex::Pixel>(t: &mut crate::tex::Image<P>, fmt: u32) {
    if matches!(fmt, FORMAT_8_BIT | FORMAT_16_BIT | FORMAT_HALF | COMPILED_8_BIT | COMPILED_16_BIT | COMPILED_HALF) {
        map_channels(t, |c| format_value(c, fmt));
    }
}

/// One channel value through format `fmt` ([`apply_format`]).
#[inline]
pub fn format_value(c: f32, fmt: u32) -> f32 {
    match fmt {
        FORMAT_8_BIT | FORMAT_16_BIT => {
            let k = if fmt == FORMAT_8_BIT { STEPS_8 as f32 } else { STEPS_16 as f32 };
            (c.clamp(0.0, 1.0) * k).round() / k
        }
        FORMAT_HALF => crate::exr::half_to_f32(crate::exr::f32_to_half(c)),
        COMPILED_8_BIT => compiled_8bit(c),
        COMPILED_16_BIT => compiled_16bit(c),
        COMPILED_HALF => compiled_half(c),
        _ => c,
    }
}

/// Every channel value of `t` through `f`, spread over the cores in runs of pixels. Never stops early (unlike
/// `tex::par_rows`, which gives up on a stale editor job): what comes out is always whole, and each value depends on
/// itself alone, so the result is the same as one pass in order.
pub fn map_channels<P: crate::tex::Pixel>(t: &mut crate::tex::Image<P>, f: impl Fn(f32) -> f32 + Sync) {
    crate::par::for_each_run_mut(&mut t.px, 1 << 15, |_, run| {
        for p in run {
            *p = p.each(&f);
        }
    });
}

/// A read of an integer image: each value rounded half up to a 16-bit step (warps, pyramid levels, doublings). Measurement "What
/// nodes hold and how they read" ("rounds half up to a 16-bit step").
pub fn round_to_16bit_read<P: crate::tex::Pixel>(t: &mut crate::tex::Image<P>) {
    map_channels(t, read_rounded);
}

/// One value of [`round_to_16bit_read`].
#[inline]
pub fn read_rounded(c: f32) -> f32 {
    (crate::tex::floor64(f64::from(c) * STEPS_16 + 0.5) / STEPS_16) as f32
}

/// A value through an 8-bit node of a compiled graph, as measured over all 256 levels: to 16 bits to the nearest, to 12 bits
/// by cutting off, then to 8 bits to the nearest, a tie going down. So 0.5 is 127 / 255, and the switch to the next level
/// falls 0.5 to 0.5625 of a level up, a saw over 16 levels. Halfway between two levels (the mean of four pixels) goes down
/// except at levels 15, 31, .. 111, which the 16 bits lift over a 12-bit step. Measurement "Node formats" and "Fx-map frames, draw
/// order and small rules" › "Images".
pub fn compiled_8bit(v: f32) -> f32 {
    let fine = ((f64::from(v.clamp(0.0, 1.0)) * STEPS_16).round() as u32) >> CUT_16_TO_12;
    ((STEPS_8 * fine + FINE_8_TIE_DOWN) / FINE_8) as f32 / STEPS_8 as f32
}

/// A value through a 16-bit node of a compiled graph, measured with a pixel processor's ramp read back in single precision: the same form as
/// [`compiled_8bit`], cut to 20 bits, then to 16 to the nearest, a tie going down. The error runs from -0.5625 to 0.5 of a step.
/// Measurement "What nodes hold and how they read".
pub fn compiled_16bit(v: f32) -> f32 {
    // not negative: the cast cuts off as floor() does (NaN gives 0 either way)
    let fine = (f64::from(v.clamp(0.0, 1.0)) * FINE_16 as f64) as u64;
    ((STEPS_16 as u64 * fine + FINE_16_TIE_DOWN) >> FINE_16_BITS) as f32 / STEPS_16 as f32
}

/// A value through a half-float node of a compiled graph: cut to half precision, not rounded (measured on 0.125..1: the error runs over a
/// whole step, all of it downwards). Negative values are taken to go the same way towards zero: not measured. Measurement "What nodes
/// hold and how they read" ("Half float cuts towards zero").
pub fn compiled_half(v: f32) -> f32 {
    let a = v.abs();
    if !a.is_finite() || a >= HALF_TOO_LARGE {
        return v;
    }
    if a < HALF_SMALLEST_NORMAL {
        // below the normal range the step is fixed
        let step = HALF_SUBNORMAL_STEP;
        return (v / step).trunc() * step;
    }
    f32::from_bits(v.to_bits() & !HALF_DROPPED_BITS)
}

// Format data of IEEE half floats (binary16), which `compiled_half` cuts to: what it leaves alone, where its normal range starts,
// the fixed step below it (2^-24), and the low mantissa bits a single has beyond a half's 10.
const HALF_TOO_LARGE: f32 = 65536.0;
const HALF_SMALLEST_NORMAL: f32 = 1.0 / 16384.0;
const HALF_SUBNORMAL_STEP: f32 = 1.0 / 16_777_216.0f32;
const HALF_DROPPED_BITS: u32 = 0x1fff;

#[cfg(test)]
mod compiled_format_tests {
    #[test]
    fn sixteen_bit_nodes_cut_as_floor_would() {
        // the cast in `compiled_16bit` against the floor() it replaces, over every f32 from 0 to 1 a step apart and the edges
        let old = |v: f32| {
            let fine = (f64::from(v.clamp(0.0, 1.0)) * 1_048_576.0).floor() as u64;
            ((65535 * fine + 524_287) >> 20) as f32 / 65535.0
        };
        let mut v = vec![-1.0f32, -0.0, 0.0, 1.0, 2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MIN_POSITIVE, 1e-40];
        v.extend((0..=1u32 << 20).flat_map(|k| {
            let x = k as f32 / 1_048_576.0;
            [x, x.next_up(), x.next_down()]
        }));
        for x in v {
            assert_eq!(super::compiled_16bit(x).to_bits(), old(x).to_bits(), "{x:e}");
        }
    }

    #[test]
    fn read_rounding_is_the_same_spread_over_threads() {
        let mut t = super::Tex::from_fn(300, 257, |u, v| [u * 3.0 - 1.0, v, crate::math::sin(u * 997.0), f32::from_bits(((u * 1e6) as u32) << 9)]);
        t.px[5] = [f32::NAN, f32::INFINITY, -0.0, 1e30];
        let mut want = t.clone();
        for p in want.px.iter_mut() {
            for c in p.iter_mut() {
                *c = ((f64::from(*c) * 65535.0 + 0.5).floor() / 65535.0) as f32;
            }
        }
        super::round_to_16bit_read(&mut t);
        assert!(t.px.iter().flatten().zip(want.px.iter().flatten()).all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    #[test]
    fn sixteen_bit_nodes_cut_to_twenty_bits_and_stay_whole_steps() {
        use super::compiled_16bit;
        assert_eq!(compiled_16bit(0.0), 0.0);
        assert_eq!(compiled_16bit(1.0), 1.0);
        assert_eq!(compiled_16bit(2.0), 1.0);
        for k in 0..2000 {
            let at = k as f32 * 0.000_321;
            let n = f64::from(compiled_16bit(at)) * 65535.0;
            assert!((n - n.round()).abs() < 2e-3, "{n}");
            let error = n - f64::from(at) * 65535.0;
            assert!((-0.5635..=0.501).contains(&error), "{at}: {error}");
        }
    }

    #[test]
    fn half_float_nodes_cut_towards_zero() {
        use super::compiled_half;
        let step = crate::math::powi(2f32, -11); // half precision's step on 0.5..1
        assert_eq!(compiled_half(0.5 + 1.9 * step), 0.5 + step);
        assert_eq!(compiled_half(0.5 + step), 0.5 + step);
        assert_eq!(compiled_half(-0.5 - 1.9 * step), -0.5 - step);
        assert_eq!(compiled_half(1.5), 1.5);
        let tiny = crate::math::powi(2f32, -24); // the fixed step below the normal range
        assert_eq!(compiled_half(tiny * 2.5), tiny * 2.0);
    }

    use super::compiled_8bit;

    #[test]
    fn eight_bit_nodes_cut_to_twelve_bits_and_a_tie_goes_down() {
        assert_eq!(compiled_8bit(0.5), 127.0 / 255.0);
        assert_eq!(compiled_8bit(128.0 / 255.0), 128.0 / 255.0);
        // level 0 ends at 9 / 4096, not at half of 1 / 255
        assert_eq!(compiled_8bit(8.96 / 4096.0), 0.0);
        // halfway between two levels: down, but up from 15, 31, .. 111
        assert_eq!(compiled_8bit(14.5 / 255.0), 14.0 / 255.0);
        assert_eq!(compiled_8bit(15.5 / 255.0), 16.0 / 255.0);
        assert_eq!(compiled_8bit(111.5 / 255.0), 112.0 / 255.0);
        assert_eq!(compiled_8bit(127.5 / 255.0), 127.0 / 255.0);
        assert_eq!(compiled_8bit(9.0 / 4096.0), 1.0 / 255.0);
        assert_eq!(compiled_8bit(1.0), 1.0);
        // levels that are already 8 bits stay as they are
        for k in 0..=255u32 {
            assert_eq!(compiled_8bit(k as f32 / 255.0), k as f32 / 255.0);
        }
    }
}

#[cfg(test)]
mod blend_alpha_tests {
    use super::{Tex, blend_ex};

    fn one(p: [f32; 4]) -> Tex {
        Tex::from_fn(1, 1, |_, _| p)
    }

    #[test]
    fn a_premultiplied_copy_adds_the_foreground_colour_over_what_its_alpha_leaves() {
        // colour where the foreground is transparent still comes in; the background shows through all of it
        let (fg, bg) = (one([0.2, 0.1, 0.0, 0.0]), one([0.5, 0.5, 0.5, 1.0]));
        let o = blend_ex(&fg, &bg, None, 0, 1.0, 3, [0.0, 0.0, 1.0, 1.0]).px[0];
        assert!((o[0] - 0.7).abs() < 1e-6 && (o[1] - 0.6).abs() < 1e-6 && (o[2] - 0.5).abs() < 1e-6, "{o:?}");
        // straight alpha leaves that background alone
        let s = blend_ex(&fg, &bg, None, 0, 1.0, 2, [0.0, 0.0, 1.0, 1.0]).px[0];
        assert!((s[0] - 0.5).abs() < 1e-6, "{s:?}");
    }

    #[test]
    fn a_switch_takes_its_input_whole_whatever_the_foreground_alpha() {
        let (fg, bg) = (one([0.2, 0.3, 0.4, 0.05]), one([0.9, 0.9, 0.9, 1.0]));
        for am in [0, 2, 3] {
            let o = blend_ex(&fg, &bg, None, 21, 1.0, am, [0.0, 0.0, 1.0, 1.0]).px[0];
            assert!(o.iter().zip([0.2, 0.3, 0.4, 0.05]).all(|(x, y)| (x - y).abs() < 1e-6), "alpha mode {am}: {o:?}");
        }
        let o = blend_ex(&fg, &bg, None, 21, 1.0, 4, [0.0, 0.0, 1.0, 1.0]).px[0];
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(near(o, [0.2, 0.3, 0.4, 0.05]), "{o:?}");
        let b = blend_ex(&fg, &bg, None, 21, 0.0, 4, [0.0, 0.0, 1.0, 1.0]).px[0];
        assert!(near(b, [0.9, 0.9, 0.9, 1.0]), "{b:?}");
    }
}
