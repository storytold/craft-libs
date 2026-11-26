// SPDX-License-Identifier: MIT OR Apache-2.0
//! Second set of nodes (batch 1). `compute` returns Ok(None) for kinds it does not know, so the
//! main evaluator can fall back to it. Every node here is tileable and independent of the output resolution:
//! pixel-sized parameters are given for a 512 px texture and scaled by `size / 512`.
use crate::graph::{Error, Node};
use crate::math;
use crate::noise;
use crate::ops;
use crate::tex::{Tex, lum};
use std::collections::BTreeMap;
use std::f32::consts::TAU;
use std::sync::Arc as Rc;

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let d = b - a;
    let t = if d.abs() < 1e-9 { if x >= b { 1.0 } else { 0.0 } } else { ((x - a) / d).clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ---------------------------------------------------------------- FFT (used by Normal To Height)
fn fft(a: &mut [[f32; 2]], inverse: bool) {
    let n = a.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            a.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = if inverse { TAU } else { -TAU } / len as f32;
        let (ws, wc) = math::sin_cos(ang);
        // `n` is a power of two, so the blocks are whole
        for block in a.chunks_exact_mut(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            let (lo, hi) = block.split_at_mut(len / 2);
            for (x, y) in lo.iter_mut().zip(hi.iter_mut()) {
                let (u, v) = (*x, *y);
                let t = [v[0] * cr - v[1] * ci, v[0] * ci + v[1] * cr];
                *x = [u[0] + t[0], u[1] + t[1]];
                *y = [u[0] - t[0], u[1] - t[1]];
                let nr = cr * wc - ci * ws;
                ci = cr * ws + ci * wc;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

fn transpose_c(a: &[[f32; 2]], n: usize) -> Vec<[f32; 2]> {
    let mut o = vec![[0.0; 2]; n * n];
    for (y, row) in a.chunks_exact(n.max(1)).take(n).enumerate() {
        for (x, &c) in row.iter().enumerate() {
            if let Some(e) = o.get_mut(x * n + y) {
                *e = c;
            }
        }
    }
    o
}

fn fft2(a: &mut Vec<[f32; 2]>, n: usize, inverse: bool) {
    for r in a.chunks_mut(n) {
        fft(r, inverse);
    }
    let mut t = transpose_c(a, n);
    for r in t.chunks_mut(n) {
        fft(r, inverse);
    }
    *a = transpose_c(&t, n);
}

/// Integrate a tangent-space normal map back into a height map (periodic Poisson solve in the Fourier domain).
fn normal_to_height(nm: &Tex, directx: bool, relief: f32, opacity: f32) -> Tex {
    let n = nm.w.next_power_of_two().clamp(2, 4096);
    let src = if nm.w == n && nm.h == n { nm.clone() } else { ops::resize(nm, n) };
    let mut gx = vec![[0.0f32; 2]; n * n];
    let mut gy = vec![[0.0f32; 2]; n * n];
    for ((p, gxi), gyi) in src.px.iter().zip(gx.iter_mut()).zip(gy.iter_mut()) {
        let (x, y, z) = ((p[0] - 0.5) * 2.0, (p[1] - 0.5) * 2.0, ((p[2] - 0.5) * 2.0).max(0.05));
        gxi[0] = -x / z;
        gyi[0] = (if directx { -y } else { y }) / z;
    }
    fft2(&mut gx, n, false);
    fft2(&mut gy, n, false);
    let mut h = vec![[0.0f32; 2]; n * n];
    for ky in 0..n {
        let sy = math::sin(TAU * ky as f32 / n as f32);
        for kx in 0..n {
            let sx = math::sin(TAU * kx as f32 / n as f32);
            let d = sx * sx + sy * sy;
            if d < 1e-9 {
                continue;
            }
            let i = ky * n + kx;
            let (Some(gxi), Some(gyi), Some(hi)) = (gx.get(i), gy.get(i), h.get_mut(i)) else { continue };
            // h = -i (sx gx + sy gy) / (sx^2 + sy^2)
            let (re, im) = (sx * gxi[0] + sy * gyi[0], sx * gxi[1] + sy * gyi[1]);
            // relief balance: 0.5 = plain integration, above favours large shapes, below favours fine detail
            let wgt = if (relief - 0.5).abs() > 1e-4 { math::powf(d, 0.5 - relief) } else { 1.0 };
            *hi = [im / d * wgt, -re / d * wgt];
        }
    }
    fft2(&mut h, n, true);
    let vals: Vec<f32> = h.iter().map(|c| c[0] / (n * n) as f32).collect();
    let (mn, mx) = vals.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
    let k = if mx - mn > 1e-9 { 1.0 / (mx - mn) } else { 0.0 };
    let out = Tex {
        w: n,
        h: n,
        px: vals
            .iter()
            .map(|v| {
                let g = if k > 0.0 { (v - mn) * k } else { 0.5 };
                let g = 0.5 + (g - 0.5) * opacity;
                [g, g, g, 1.0]
            })
            .collect(),
    };
    if nm.w == n && nm.h == n { out } else { ops::resize_to(&out, nm.w, nm.h) }
}

/// Run a node of this module. Ok(None) = not one of ours.
pub fn compute(kind: &str, id: u32, n: &Node, ins: &BTreeMap<String, Rc<Tex>>, s: usize, hs: usize, seed: u32) -> Result<Option<Tex>, Error> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let need = |slot: &str| -> Result<&Tex, Error> { ins.get(slot).map(|r| r.as_ref()).ok_or_else(|| Error::MissingInput(id, slot.to_string())) };
    let opt = |slot: &str| -> Option<&Tex> { ins.get(slot).map(|r| r.as_ref()) };
    let rs = s as f32 / 512.0;
    let per = |k: &str, d: f32| f(k, d).round().clamp(1.0, 256.0) as i32;
    let (con, inv) = (f("contrast", 1.0), f("invert", 0.0) > 0.5);
    let dz = noise::Disorder::new(seed, f("disorder", 0.0), f("disorder_speed", 1.0), f("anisotropy", 0.0), f("anisotropy_angle", 0.0));
    let fin = |t: Tex| -> Tex { if (con - 1.0).abs() > 1e-4 || inv { ops::contrast_invert(&t, con, inv) } else { t } };
    let out = match kind {
        // ------------------------------------------------ blurs and warps
        "non_uniform_blur" => {
            let (t, m) = (need("in")?, need("mask")?);
            let max = f("intensity", 8.0).clamp(0.0, 64.0) * rs;
            let lv: Vec<Tex> = (0..=4).map(|i| if i == 0 { t.clone() } else { ops::blur(t, max * i as f32 / 4.0) }).collect();
            Tex::from_fn(t.w, t.h, |u, v| {
                let k = lum(m.sample(u, v)).clamp(0.0, 1.0) * 4.0;
                let i = (k.floor() as usize).min(3);
                let fr = k - i as f32;
                let (Some(la), Some(lb)) = (lv.get(i), lv.get(i + 1)) else { return [0.0; 4] };
                let (a, b) = (la.sample(u, v), lb.sample(u, v));
                [lerp(a[0], b[0], fr), lerp(a[1], b[1], fr), lerp(a[2], b[2], fr), lerp(a[3], b[3], fr)]
            })
        }
        "radial_blur" => {
            let t = need("in")?;
            let (cx, cy) = (0.5 + f("x", 0.0), 0.5 + f("y", 0.0));
            if f("mode", 0.0) > 0.5 {
                // legacy zoom blur
                let (amount, rad) = (f("amount", 0.1).clamp(0.0, 1.0), f("radius", 0.5).clamp(0.01, 0.5));
                let taps = 24;
                Tex::from_fn(t.w, t.h, |u, v| {
                    let (dx, dy) = (ops::wrapd(u - cx), ops::wrapd(v - cy));
                    let r = (dx * dx + dy * dy).sqrt();
                    let amt = amount * smooth(0.0, 1.0, 1.0 - r / rad);
                    let mut a = [0.0f32; 4];
                    for k in 0..taps {
                        let sc = 1.0 - amt * (k as f32 / (taps - 1) as f32);
                        let p = t.sample(cx + dx * sc, cy + dy * sc);
                        for (ac, pc) in a.iter_mut().zip(p) {
                            *ac += pc / taps as f32;
                        }
                    }
                    a
                })
            } else {
                // spin blur, `samples` taps along an arc of `angle` turns around the centre
                let n = f("samples", 64.0).round().clamp(1.0, 128.0) as usize;
                let arc = f("angle", 0.125).clamp(0.0, 0.5) * TAU;
                Tex::from_fn(t.w, t.h, |u, v| {
                    let (dx, dy) = (ops::wrapd(u - cx), ops::wrapd(v - cy));
                    // the blur fades out towards the image border so the result still tiles
                    let fade = ((0.5 - (dx * dx + dy * dy).sqrt()) / 0.15).clamp(0.0, 1.0);
                    let mut a = [0.0f32; 4];
                    for k in 0..n {
                        let th = if n > 1 { (k as f32 / (n - 1) as f32 - 0.5) * arc * fade } else { 0.0 };
                        let (sn, cs) = math::sin_cos(th);
                        let p = t.sample(cx + dx * cs - dy * sn, cy + dx * sn + dy * cs);
                        for (ac, pc) in a.iter_mut().zip(p) {
                            *ac += pc / n as f32;
                        }
                    }
                    a
                })
            }
        }
        "anisotropic_blur" => {
            let t = need("in")?;
            let (inten, an) = (f("intensity", 8.0).clamp(0.0, 64.0), f("anisotropy", 0.8).clamp(0.0, 1.0));
            let a = ops::blur(t, inten * (1.0 - an) * rs);
            ops::directional_blur(&a, f("angle", 0.0), inten * an * 4.0 / 512.0)
        }
        // ------------------------------------------------ transforms
        "symmetry" => {
            let t = need("in")?;
            let (axis, c) = (f("axis", 0.0) as u32, f("offset", 0.0));
            let sg = if f("flip", 0.0) > 0.5 { -1.0 } else { 1.0 };
            let tm = f("transfer", 0.0).round() as u32; // 0 copy, 1 add, 2 subtract, 3 multiply, 4 add sub, 5 max, 6 min
            let fold = move |p: f32| 0.5 + c + sg * ops::wrapd(p - 0.5 - c).abs();
            let diag = move |u: f32, v: f32| {
                // mirror across the line u = v
                let (x, y) = (ops::wrapd(u - v), u + v);
                let x = x.abs() * sg;
                ((y + x) * 0.5, (y - x) * 0.5)
            };
            Tex::from_fn(t.w, t.h, |u, v| {
                let (a, b) = match axis {
                    0 => (fold(u), v),
                    1 => (u, fold(v)),
                    2 => (fold(u), fold(v)),
                    3 => diag(u, v),
                    4 => {
                        // anti-diagonal: mirror across the line u + v = 0 (mod 1)
                        let (x, y) = (ops::wrapd(u + v), u - v);
                        let x = x.abs() * sg;
                        ((y + x) * 0.5, (x - y) * 0.5)
                    }
                    _ => diag(fold(u), fold(v)), // 8-fold: both axes plus the diagonal
                };
                let m = t.sample(a, b);
                if tm == 0 {
                    return m;
                }
                let o = t.sample(u, v);
                let mut r = o;
                for ((rk, ok), mk) in r.iter_mut().zip(o).zip(m).take(3) {
                    *rk = match tm {
                        1 => ok + mk,
                        2 => ok - mk,
                        3 => ok * mk,
                        4 => ok + (mk - 0.5) * 2.0,
                        5 => ok.max(mk),
                        _ => ok.min(mk),
                    }
                    .clamp(0.0, 1.0);
                }
                r
            })
        }
        "skew" => {
            let t = need("in")?;
            // whole numbers keep the lattice periodic: the skewed texture still tiles
            let (kx, ky) = (f("skew_x", 1.0).round().clamp(-32.0, 32.0), f("skew_y", 0.0).round().clamp(-32.0, 32.0));
            // align = the line the shear pivots around: 0 centre, 1 top left (default), 2 bottom right
            let o = match f("align", 1.0).round() as u32 {
                0 => 0.5,
                2 => 1.0,
                _ => 0.0,
            };
            Tex::from_fn(t.w, t.h, |u, v| t.sample_cubic(u + kx * (v - o), v + ky * (u - o)))
        }
        // ------------------------------------------------ adjustments
        "auto_levels" => {
            let t = need("in")?;
            let per_ch = f("per_channel", 0.0) > 0.5;
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for p in &t.px {
                for ((l, h), &pc) in lo.iter_mut().zip(hi.iter_mut()).zip(p) {
                    *l = l.min(pc);
                    *h = h.max(pc);
                }
            }
            if !per_ch {
                let (l, h) = (lo[0].min(lo[1]).min(lo[2]), hi[0].max(hi[1]).max(hi[2]));
                lo = [l; 3];
                hi = [h; 3];
            }
            t.map(|p| {
                let mut o = p;
                for ((oc, pc), (l, h)) in o.iter_mut().zip(p).zip(lo.into_iter().zip(hi)) {
                    *oc = if h - l > 1e-6 { (pc - l) / (h - l) } else { pc };
                }
                o
            })
        }
        "channel_mixer" => {
            let t = need("in")?;
            let m =
                [[f("rr", 1.0), f("rg", 0.0), f("rb", 0.0)], [f("gr", 0.0), f("gg", 1.0), f("gb", 0.0)], [f("br", 0.0), f("bg", 0.0), f("bb", 1.0)]];
            let mono = f("mono", 0.0) > 0.5;
            t.map(|p| {
                let mut o = p;
                for (oc, mc) in o.iter_mut().zip(&m) {
                    *oc = (mc[0] * p[0] + mc[1] * p[1] + mc[2] * p[2]).clamp(-8.0, 8.0);
                }
                if mono {
                    let g = lum(o);
                    o = [g, g, g, p[3]];
                }
                o
            })
        }
        "alpha_merge" => {
            let rgb = need("rgb")?;
            match opt("alpha") {
                Some(a) => Tex::from_fn(rgb.w, rgb.h, |u, v| {
                    let p = rgb.sample(u, v);
                    [p[0], p[1], p[2], lum(a.sample(u, v))]
                }),
                None => rgb.clone(),
            }
        }
        "color_to_mask" => {
            let t = need("in")?;
            let (cr, cg, cb) = (f("r", 1.0), f("g", 0.0), f("b", 0.0));
            let (range, soft) = (f("range", 0.2).max(0.0), f("softness", 0.1).max(0.0));
            t.map(|p| {
                let d = ((math::powi(p[0] - cr, 2) + math::powi(p[1] - cg, 2) + math::powi(p[2] - cb, 2)) / 3.0).sqrt();
                let m = 1.0 - smooth(range, range + soft.max(1e-4), d);
                [m, m, m, 1.0]
            })
        }
        "replace_color" => {
            let t = need("in")?;
            let src = [f("src_r", 1.0), f("src_g", 0.0), f("src_b", 0.0)];
            let dst = [f("dst_r", 0.0), f("dst_g", 0.0), f("dst_b", 1.0)];
            let (range, soft) = (f("range", 0.25).max(0.0), f("softness", 0.15).max(0.0));
            t.map(|p| {
                let d = ((math::powi(p[0] - src[0], 2) + math::powi(p[1] - src[1], 2) + math::powi(p[2] - src[2], 2)) / 3.0).sqrt();
                let m = 1.0 - smooth(range, range + soft.max(1e-4), d);
                [p[0] + (dst[0] - src[0]) * m, p[1] + (dst[1] - src[1]) * m, p[2] + (dst[2] - src[2]) * m, p[3]]
            })
        }
        "pow" => {
            let t = need("in")?;
            let e = f("exponent", 2.0).clamp(0.0, 32.0);
            t.map(|p| [math::powf(p[0].max(0.0), e), math::powf(p[1].max(0.0), e), math::powf(p[2].max(0.0), e), p[3]])
        }
        "histogram_shift" => {
            let t = need("in")?;
            let d = (f("position", 0.5) - 0.5) * 2.0;
            t.map(|p| [(p[0] + d).clamp(0.0, 1.0), (p[1] + d).clamp(0.0, 1.0), (p[2] + d).clamp(0.0, 1.0), p[3]])
        }
        "grayscale_conversion" => {
            let t = need("in")?;
            let (wr, wg, wb, wa) = (f("r", 0.3), f("g", 0.59), f("b", 0.11), f("a", 0.0));
            let (lo, hi) = if f("limit", 0.0) > 0.5 { (0.0, 1.0) } else { (f32::MIN, f32::MAX) };
            t.map(|p| {
                let g = (p[0] * wr + p[1] * wg + p[2] * wb + p[3] * wa).clamp(lo, hi);
                [g, g, g, p[3]]
            })
        }
        // ------------------------------------------------ generators
        "cells" => {
            let mut cells = noise::Cells::new(f("variant", 0.0) as u32, per("scale", 8.0), seed);
            cells.jitter = f("jitter", 1.0).clamp(0.0, 1.0);
            (cells.luminance_random, cells.edge_width, cells.hardness) =
                (f("luminance_random", 0.0).clamp(0.0, 1.0), f("edge_width", 0.25).clamp(0.01, 1.0), f("hardness", 2.0).clamp(0.1, 16.0));
            let (src, pseed) = (f("color_source", 0.0) as u32, f("pseudorandom_seed", 0.0) as u32);
            // cell value of Cells 4: random, pseudorandom (re-rolled with its own seed) or the input image at the cell centre
            cells.source = if src == 1 { noise::CellSource::Pseudorandom(pseed) } else { noise::CellSource::Random };
            let img = if src == 2 { opt("color") } else { None };
            cells.output = f("_out", 0.0).round().max(0.0) as u32;
            if f("square_cells", 0.0) > 0.5 {
                cells.rows = ((cells.scale as f32 * hs as f32 / s as f32).round().max(1.0)) as i32;
            }
            let image = img.map(|t| move |x: f32, y: f32| lum(t.sample(x, y)));
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (u, v) = dz.warp(u, v);
                cells.value_with(u, v, image.as_ref().map(|i| i as &dyn Fn(f32, f32) -> f32))
            }))
        }
        "fractal_sum" => {
            let (p, lo, hi) = (per("scale", 2.0), f("min_level", 0.0).clamp(0.0, 10.0) as u32, f("max_level", 6.0).clamp(0.0, 10.0) as u32);
            let rough = f("roughness", 0.5).clamp(0.0, 1.5);
            let mode = f("mode", 0.0) as u32;
            let hi = hi.max(lo);
            let grids: Vec<(noise::GradGrid, f32)> = (lo..=hi)
                .map(|o| {
                    let q = (p as i64 * (1i64 << o)).clamp(1, 1024) as i32;
                    (noise::GradGrid::new(q, q, seed.wrapping_add(o * 31)), math::powi(rough, (o - lo) as i32))
                })
                .collect();
            let norm: f32 = grids.iter().map(|g| g.1).sum::<f32>().max(1e-6);
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (u, v) = dz.warp(u, v);
                let mut acc = 0.0;
                for (g, a) in &grids {
                    let x = g.sample(u, v);
                    acc += a * match mode {
                        1 => (x * 2.0 - 1.0).abs(),
                        2 => 1.0 - (x * 2.0 - 1.0).abs(),
                        _ => x,
                    };
                }
                (acc / norm).clamp(0.0, 1.0)
            }))
        }
        "waveform" => {
            let (a, b) = ops::tileable_wave(f("count", 8.0), f("angle", 0.0));
            let (warp, w) = (f("warp", 0.3).clamp(0.0, 4.0), per("warp_scale", 4.0));
            let shape = f("shape", 0.0) as u32;
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (u, v) = dz.warp(u, v);
                let ph = (a as f32 * u + b as f32 * v) + (noise::perlin(u, v, w, w, seed) - 0.5) * warp;
                let x = ph - ph.floor();
                match shape {
                    1 => 1.0 - (x * 2.0 - 1.0).abs(),
                    2 => x,
                    3 => {
                        if x < 0.5 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    _ => 0.5 + 0.5 * math::sin(x * TAU),
                }
            }))
        }
        "gaussian_spots" => {
            let spots = noise::GaussianSpots {
                scale: per("scale", 8.0),
                seed,
                size: f("size", 0.5).clamp(0.05, 1.0),
                size_variation: f("size_variation", 0.5).clamp(0.0, 1.0),
            };
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (u, v) = dz.warp(u, v);
                spots.value(u, v)
            }))
        }
        // ------------------------------------------------ normals and surface
        "normal_invert" => {
            let t = need("in")?;
            let (ix, iy, iz, ia) = (f("invert_x", 0.0) > 0.5, f("invert_y", 1.0) > 0.5, f("invert_z", 0.0) > 0.5, f("invert_a", 0.0) > 0.5);
            t.map(|p| {
                [
                    if ix { 1.0 - p[0] } else { p[0] },
                    if iy { 1.0 - p[1] } else { p[1] },
                    if iz { 1.0 - p[2] } else { p[2] },
                    if ia { 1.0 - p[3] } else { p[3] },
                ]
            })
        }
        "normal_normalize" => need("in")?.map(|p| {
            let (x, y, z) = ((p[0] - 0.5) * 2.0, (p[1] - 0.5) * 2.0, (p[2] - 0.5) * 2.0);
            let l = (x * x + y * y + z * z).sqrt().max(1e-6);
            [x / l * 0.5 + 0.5, y / l * 0.5 + 0.5, z / l * 0.5 + 0.5, p[3]]
        }),
        "normal_vector_rotation" => {
            let t = need("in")?;
            let (ang, gs) = (f("angle", 0.0) * TAU, if f("directx", 0.0) > 0.5 { -1.0f32 } else { 1.0 });
            let rot = opt("rotation");
            Tex::from_fn(t.w, t.h, |u, v| {
                let p = t.sample(u, v);
                // the optional rotation map scales the angle per pixel
                let a = ang * rot.map(|m| lum(m.sample(u, v))).unwrap_or(1.0);
                let (sn, cs) = math::sin_cos(a);
                let (x, y) = ((p[0] - 0.5) * 2.0, gs * (p[1] - 0.5) * 2.0);
                let (rx, ry) = (cs * x - sn * y, sn * x + cs * y);
                [rx * 0.5 + 0.5, gs * ry * 0.5 + 0.5, p[2], p[3]]
            })
        }
        "normal_to_height" => {
            normal_to_height(need("in")?, f("directx", 0.0) > 0.5, f("relief_balance", 0.5).clamp(0.0, 1.0), f("opacity", 1.0).clamp(0.0, 1.0))
        }
        "sobel" => {
            let t = need("in")?;
            let k = f("intensity", 1.0).clamp(0.0, 64.0) * (t.w as f32 / 512.0).max(0.25);
            Tex::from_fn(t.w, t.h, |u, v| {
                let (x, y) = ((u * t.w as f32) as i64, (v * t.h as f32) as i64);
                let g = |i: i64, j: i64| t.gray_at(x + i, y + j);
                let gx = g(1, -1) + 2.0 * g(1, 0) + g(1, 1) - g(-1, -1) - 2.0 * g(-1, 0) - g(-1, 1);
                let gy = g(-1, 1) + 2.0 * g(0, 1) + g(1, 1) - g(-1, -1) - 2.0 * g(0, -1) - g(1, -1);
                let m = ((gx * gx + gy * gy).sqrt() * k * 0.25).clamp(0.0, 1.0);
                [m, m, m, 1.0]
            })
        }
        // ------------------------------------------------ effects
        "glow" => {
            let t = need("in")?;
            let (w, inten) = (f("width", 8.0).clamp(0.0, 128.0) * rs, f("intensity", 1.0).clamp(0.0, 8.0));
            let b = ops::blur(t, w);
            // clear amount cuts the faint tail of the glow off; the colour tints it (white = unchanged)
            let (clear, gc) = (f("clear", 0.0).clamp(0.0, 0.99), [f("glow_r", 1.0), f("glow_g", 1.0), f("glow_b", 1.0)]);
            Tex::from_fn(t.w, t.h, |u, v| {
                let (p, q) = (t.sample(u, v), b.sample(u, v));
                let cut = |x: f32| ((x - clear) / (1.0 - clear)).max(0.0);
                [
                    (p[0] + cut(q[0]) * gc[0] * inten).clamp(0.0, 1.0),
                    (p[1] + cut(q[1]) * gc[1] * inten).clamp(0.0, 1.0),
                    (p[2] + cut(q[2]) * gc[2] * inten).clamp(0.0, 1.0),
                    p[3],
                ]
            })
        }
        "mosaic" => {
            let t = need("in")?;
            if let Some(m) = opt("mosaic_map") {
                ops::mosaic_map(t, m, f("intensity", 1.0))
            } else {
                // no map: a plain grid of `count` cells per side
                let c = per("count", 16.0) as f32;
                Tex::from_fn(t.w, t.h, |u, v| t.sample(((u * c).floor() + 0.5) / c, ((v * c).floor() + 0.5) / c))
            }
        }
        "shape_drop_shadow" => {
            let t = need("in")?;
            let (ang, dist) = (f("angle", 0.125) * TAU, f("distance", 0.03).clamp(0.0, 0.5));
            let (sn, cs) = math::sin_cos(ang);
            let shifted = Tex::from_fn(t.w, t.h, |u, v| t.sample(u - cs * dist, v - sn * dist));
            let sh = ops::blur(&shifted, f("blur", 6.0).clamp(0.0, 128.0) * rs);
            let op = f("opacity", 0.8).clamp(0.0, 1.0);
            let spread = f("spread", 0.0).clamp(0.0, 1.0);
            Tex::from_fn(t.w, t.h, |u, v| {
                // spread raises the blur cut-off so the shadow stays solid for longer
                let sv = (lum(sh.sample(u, v)) / (1.0 - spread * 0.99)).min(1.0);
                let m = lum(t.sample(u, v)).max(sv * op).clamp(0.0, 1.0);
                [m, m, m, 1.0]
            })
        }
        // ------------------------------------------------ batch 1b: warps, patterns, strokes
        "non_uniform_directional_warp" => {
            let (t, m) = (need("in")?, need("intensity")?);
            let (k, base) = (f("intensity", 0.1).clamp(-2.0, 2.0), f("angle", 0.0) * TAU);
            let am = opt("angle");
            Tex::from_fn(t.w, t.h, |u, v| {
                let a = base + am.map(|a| (lum(a.sample(u, v)) - 0.5) * TAU).unwrap_or(0.0);
                let d = (lum(m.sample(u, v)) - 0.5) * 2.0 * k;
                t.sample_cubic(u + math::cos(a) * d, v + math::sin(a) * d)
            })
        }
        "multi_directional_warp" => {
            let (t, m) = (need("in")?, need("warp")?);
            let (k, a0) = (f("intensity", 0.1).clamp(-2.0, 2.0), f("angle", 0.0));
            // Directions: 0 = 1 (the angle and its opposite), 1 = 2 (plus the perpendicular axis), 2 = 4 (plus the 45 degree
            // steps). Larger values from older graphs keep meaning "that many samples".
            let dv = f("directions", 2.0).round().max(0.0) as usize;
            let n = match dv {
                0 => 2,
                1 => 4,
                2 => 8,
                _ => dv.min(16),
            };
            let mode = f("mode", 0.0).round() as u32; // 0 average, 1 max, 2 min, 3 chain
            if mode == 3 {
                // each direction warps the result of the previous one
                let mut cur = t.clone();
                for i in 0..n {
                    let a = (a0 + i as f32 / n as f32) * TAU;
                    let src = cur.clone();
                    cur = Tex::from_fn(t.w, t.h, |u, v| {
                        let d = (lum(m.sample(u, v)) - 0.5) * 2.0 * k / n as f32;
                        src.sample(u + math::cos(a) * d, v + math::sin(a) * d)
                    });
                }
                cur
            } else {
                Tex::from_fn(t.w, t.h, |u, v| {
                    let d = (lum(m.sample(u, v)) - 0.5) * 2.0 * k;
                    let mut acc = if mode == 1 {
                        [f32::MIN; 4]
                    } else if mode == 2 {
                        [f32::MAX; 4]
                    } else {
                        [0.0f32; 4]
                    };
                    for i in 0..n {
                        let a = (a0 + i as f32 / n as f32) * TAU;
                        let p = t.sample(u + math::cos(a) * d, v + math::sin(a) * d);
                        for (ac, pc) in acc.iter_mut().zip(p) {
                            match mode {
                                1 => *ac = ac.max(pc),
                                2 => *ac = ac.min(pc),
                                _ => *ac += pc / n as f32,
                            }
                        }
                    }
                    acc
                })
            }
        }
        "normal_combine" => {
            // technique 0 whiteout, 1 channel mixer (partial derivatives added), 2 detail oriented (reoriented normal mapping:
            // the detail normal is rotated into the surface of the base normal)
            let (b, d) = (need("base")?, need("detail")?);
            let tech = f("technique", 2.0).round() as u32;
            Tex::from_fn(b.w, b.h, |u, v| {
                let (pb, pd) = (b.sample(u, v), d.sample(u, v));
                if tech < 2 {
                    let (bx, by, bz) = ((pb[0] - 0.5) * 2.0, (pb[1] - 0.5) * 2.0, ((pb[2] - 0.5) * 2.0).max(0.05));
                    let (dx, dy, dz) = ((pd[0] - 0.5) * 2.0, (pd[1] - 0.5) * 2.0, ((pd[2] - 0.5) * 2.0).max(0.05));
                    let r = if tech == 0 { [bx + dx, by + dy, bz * dz] } else { [bx / bz + dx / dz, by / bz + dy / dz, 1.0] };
                    let l = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt().max(1e-6);
                    return [r[0] / l * 0.5 + 0.5, r[1] / l * 0.5 + 0.5, r[2] / l * 0.5 + 0.5, 1.0];
                }
                let tt = [(pb[0] - 0.5) * 2.0, (pb[1] - 0.5) * 2.0, (pb[2] - 0.5) * 2.0 + 1.0];
                let uu = [-(pd[0] - 0.5) * 2.0, -(pd[1] - 0.5) * 2.0, (pd[2] - 0.5) * 2.0];
                let dot = tt[0] * uu[0] + tt[1] * uu[1] + tt[2] * uu[2];
                let tz = if tt[2].abs() < 1e-4 { 1e-4 } else { tt[2] };
                let r = [tt[0] * dot / tz - uu[0], tt[1] * dot / tz - uu[1], tt[2] * dot / tz - uu[2]];
                let l = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt().max(1e-6);
                [r[0] / l * 0.5 + 0.5, r[1] / l * 0.5 + 0.5, r[2] / l * 0.5 + 0.5, 1.0]
            })
        }
        "triangle_grid" if f("style", 0.0) >= 0.5 => {
            let lattice = noise::TriangleLattice {
                nx: per("x_amount", 16.0).clamp(1, 64),
                ny: per("y_amount", 16.0).clamp(1, 64),
                amp: f("random_position_multiplier", 1.0).clamp(0.0, 2.0).min(1.0) * 0.32,
                quincunx: (f("quincunx_offset_x", 0.0).clamp(0.0, 1.0), f("quincunx_offset_y", 0.0).clamp(0.0, 1.0)),
                height_random: f("height_random", 1.0).clamp(0.0, 1.0),
                triangulation: f("triangulation", 0.0).round() as i32,
                output: f("output", 0.0).round() as i32,
                line_thickness: f("distance_to_edge_thickness", 0.025).clamp(0.0, 0.1),
                line_color: f("thin_line_color", 1.0),
                line_background: f("thin_line_background", 0.0),
                lines: f("thin_lines", 0.0) > 0.5,
                seed,
            };
            fin(Tex::gray_fn(s, hs, move |u, v| lattice.value(u, v)))
        }
        "triangle_grid" => {
            let grid = noise::TriangleGrid {
                count: per("count", 8.0),
                border: f("border", 0.06).clamp(0.0, 0.5),
                variation: f("variation", 1.0).clamp(0.0, 1.0),
                seed,
            };
            fin(Tex::gray_fn(s, hs, move |u, v| grid.value(u, v)))
        }
        "weave_generator" => {
            let mut weave = noise::WeaveGenerator::new(
                (per("tile_x", 1.0).clamp(1, 20), per("tile_y", 1.0).clamp(1, 20)),
                per("weave", 2.0).clamp(1, 10),
                f("shape", 0.5).clamp(0.0, 1.0),
                (f("gap_x", 0.1).clamp(0.0, 0.9), f("gap_y", 0.1).clamp(0.0, 0.9)),
            );
            weave.output = f("_out", 0.0).round().max(0.0) as u32;
            let t = Tex::gray_fn(s, hs, move |u, v| weave.value(u, v));
            if weave.output == 0 { fin(t) } else { t }
        }
        "starburst" => {
            let star = noise::Starburst {
                count: per("count", 3.0),
                rays: f("rays", 8.0).round().clamp(2.0, 64.0),
                sharpness: f("sharpness", 2.0).clamp(0.1, 16.0),
                size: f("size", 1.0).clamp(0.05, 2.0),
                rotation: f("rotation", 0.0),
                rotation_random: f("rotation_random", 0.0).clamp(0.0, 1.0),
                seed,
            };
            fin(Tex::gray_fn(s, hs, move |u, v| star.value(u, v)))
        }
        "gradient_reflected" => {
            let (a, b) = ops::tileable_wave(f("count", 1.0), f("angle", 0.0));
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let ph = a as f32 * u + b as f32 * v;
                1.0 - (2.0 * (ph - ph.floor()) - 1.0).abs()
            }))
        }
        "shape_stroke" | "shape_glow" => {
            let t = need("in")?;
            let w = (f("width", 8.0).clamp(0.5, 256.0) * rs).max(2.0);
            let mask: Vec<bool> = t.px.iter().map(|p| lum(*p) > f("threshold", 0.5)).collect();
            let inside = ops::distance_field(&mask, t.w, t.h);
            let inv: Vec<bool> = mask.iter().map(|m| !m).collect();
            let outside = ops::distance_field(&inv, t.w, t.h);
            let stroke = kind == "shape_stroke";
            let (off, soft, inten) = (f("offset", 0.0).clamp(-1.0, 1.0), f("softness", 0.5).clamp(0.0, 1.0), f("intensity", 1.0).clamp(0.0, 4.0));
            let (opacity, spread) = (f("opacity", 1.0).clamp(0.0, 1.0), f("spread", 0.0).clamp(0.0, 1.0));
            let col = [f("color_r", 1.0), f("color_g", 1.0), f("color_b", 1.0)];
            let px = mask
                .iter()
                .enumerate()
                .map(|(i, &mi)| {
                    let (ins, outs) = (inside.get(i).copied().unwrap_or(0.0), outside.get(i).copied().unwrap_or(0.0));
                    // signed distance in pixels: positive outside the shape
                    let sd = if mi { -(ins.min(1e8) - 0.5) } else { outs.min(1e8) - 0.5 };
                    let g = if stroke {
                        let c = off * w * 0.5;
                        1.0 - smooth(w * 0.5 * (1.0 - soft), w * 0.5 + 0.5, (sd - c).abs())
                    } else if mi {
                        1.0
                    } else {
                        math::powf(1.0 - (sd / w).clamp(0.0, 1.0), 1.0 + soft * 3.0) * inten
                    };
                    let g = if stroke { g } else { g / (1.0 - spread * 0.99) };
                    let g = g.clamp(0.0, 1.0) * opacity;
                    [g * col[0], g * col[1], g * col[2], 1.0]
                })
                .collect();
            Tex { w: t.w, h: t.h, px }
        }
        "chrominance_extract" => need("in")?.map(|p| {
            let l = lum(p).max(1e-3);
            [(p[0] / l * 0.5).clamp(0.0, 1.0), (p[1] / l * 0.5).clamp(0.0, 1.0), (p[2] / l * 0.5).clamp(0.0, 1.0), p[3]]
        }),
        _ => return Ok(None),
    };
    Ok(Some(out))
}
