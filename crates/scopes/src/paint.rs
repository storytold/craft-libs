//! Count grids to pixels: the trace images a UI uploads as textures.
//!
//! A cell's intensity grows with the logarithm of its count relative to a reference count (a
//! "full" cell: a flat color puts a whole column of samples on one waveform cell), so sparse
//! detail stays visible next to flat areas, like a phosphor trace. Traces of several grids add
//! up (R + G + B traces that coincide read white).
//!
//! The `*_into` functions write into buffers the caller keeps ([`Scratch`] and the output `Vec`),
//! so painting the same size again does not allocate.

use crate::{Grid, MAX_ROWS, MAX_SIGNAL_W, MAX_VECTOR_SIZE, Vectorscope};

/// Intensity 0..1 of a cell holding `count` samples (`reference` = a full cell, `gain` = the
/// Brightness setting).
#[inline]
pub fn intensity(count: u32, reference: f32, gain: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let r = (1.0 + reference.max(1.0)).ln();
    (gain * (0.12 + 0.88 * (1.0 + count as f32).ln() / r)).clamp(0.0, 1.0)
}

/// Working buffers of the painters, kept between frames.
#[derive(Clone, Debug, Default)]
pub struct Scratch {
    acc: Vec<[f32; 3]>,
    level: Vec<f32>,
    tint: Vec<[f32; 3]>,
}

fn reset<T: Clone>(v: &mut Vec<T>, n: usize, value: T) {
    v.clear();
    v.resize(n, value);
}

/// Paint grids of equal size with colors (0..1 RGB), additively, into premultiplied RGBA8 with the
/// highest row at the top. Empty cells are transparent. Grids of another size than the first are
/// skipped.
pub fn grids(layers: &[(&Grid, [f32; 3])], reference: f32, gain: f32) -> (usize, usize, Vec<u8>) {
    let mut out = Vec::new();
    let (w, h) = grids_into(layers, reference, gain, &mut Scratch::default(), &mut out);
    (w, h, out)
}

/// [`grids`] into `out` (resized to `w * h * 4`), returning `(w, h)`. A first grid wider than
/// [`MAX_SIGNAL_W`] or taller than [`MAX_ROWS`] paints nothing and gives `(0, 0)`.
pub fn grids_into(layers: &[(&Grid, [f32; 3])], reference: f32, gain: f32, scratch: &mut Scratch, out: &mut Vec<u8>) -> (usize, usize) {
    let Some((first, _)) = layers.first().filter(|(g, _)| g.cols <= MAX_SIGNAL_W && g.rows <= MAX_ROWS) else {
        out.clear();
        return (0, 0);
    };
    let (w, h) = (first.cols, first.rows);
    let n = w.saturating_mul(h);
    reset(out, n.saturating_mul(4), 0u8);
    reset(&mut scratch.acc, n, [0f32; 3]);
    for (g, col) in layers {
        if g.cols != w || g.rows != h || g.data.len() < n {
            continue;
        }
        for (row, counts) in g.data.chunks_exact(w.max(1)).take(h).enumerate() {
            let dst = (h - 1 - row) * w;
            let Some(acc) = scratch.acc.get_mut(dst..dst + w) else { continue };
            for (a, &c) in acc.iter_mut().zip(counts) {
                if c > 0 {
                    let i = intensity(c, reference, gain);
                    for k in 0..3 {
                        a[k] += col[k] * i;
                    }
                }
            }
        }
    }
    for (o, a) in out.as_chunks_mut::<4>().0.iter_mut().zip(&scratch.acc) {
        let m = a[0].max(a[1]).max(a[2]);
        if m <= 0.0 {
            continue;
        }
        o[0] = (a[0].min(1.0) * 255.0).round() as u8;
        o[1] = (a[1].min(1.0) * 255.0).round() as u8;
        o[2] = (a[2].min(1.0) * 255.0).round() as u8;
        o[3] = (m.min(1.0) * 255.0).round() as u8;
    }
    (w, h)
}

/// The reference count of a waveform cell: half a column.
pub fn waveform_reference(per_column: usize) -> f32 {
    (per_column as f32 / 2.0).max(1.0)
}

/// The 3 × 3 spread of a vectorscope point: (dy, dx, weight).
const SPREAD: [(i64, i64, f32); 9] =
    [(0, 0, 1.0), (-1, 0, 0.6), (1, 0, 0.6), (0, -1, 0.6), (0, 1, 0.6), (-1, -1, 0.35), (-1, 1, 0.35), (1, -1, 0.35), (1, 1, 0.35)];

/// Paint a vectorscope: each point takes the color of its own position (hue = angle) when
/// `colorize`, else `col`, and spreads over its 3 × 3 neighborhood (falling off), so isolated
/// points stay visible when the scope is drawn smaller than its grid. Premultiplied RGBA8, row
/// 0 = +Cr.
pub fn vectorscope(v: &Vectorscope, col: [f32; 3], gain: f32, colorize: bool) -> (usize, usize, Vec<u8>) {
    let mut out = Vec::new();
    let (w, h) = vectorscope_into(v, col, gain, colorize, &mut Scratch::default(), &mut out);
    (w, h, out)
}

/// [`vectorscope`] into `out` (resized to `size² * 4`), returning `(size, size)`. A vectorscope
/// larger than [`MAX_VECTOR_SIZE`] paints nothing and gives `(0, 0)`.
pub fn vectorscope_into(v: &Vectorscope, col: [f32; 3], gain: f32, colorize: bool, scratch: &mut Scratch, out: &mut Vec<u8>) -> (usize, usize) {
    if v.size > MAX_VECTOR_SIZE {
        out.clear();
        return (0, 0);
    }
    let n = v.size;
    let cells = n.saturating_mul(n);
    let reference = (v.samples as f32 / 64.0).max(1.0);
    reset(&mut scratch.level, cells, 0f32);
    reset(&mut scratch.tint, cells, [0f32; 3]);
    for y in 0..n {
        for x in 0..n {
            let c = v.at(x, y);
            if c == 0 {
                continue;
            }
            let i = intensity(c, reference, gain);
            let t = if colorize {
                let [px, py] = v.point(x, y);
                // the color at this point with mid lightness, desaturated towards white
                let [r, g, b] = if v.hls {
                    let hue = ((crate::angle_deg(px, py) - crate::red_angle(v.matrix)) / 360.0).rem_euclid(1.0);
                    let sat = (px.hypot(py) / 0.5).clamp(0.0, 1.0);
                    craft_color::hsl_to_rgb(hue, sat * 0.8, 0.5)
                } else {
                    craft_color::ycbcr_to_rgb(0.6, px * 0.8, py * 0.8, v.matrix)
                };
                [0.55 + 0.45 * r.clamp(0.0, 1.0), 0.55 + 0.45 * g.clamp(0.0, 1.0), 0.55 + 0.45 * b.clamp(0.0, 1.0)]
            } else {
                col
            };
            for (dy, dx, w) in SPREAD {
                let (yy, xx) = (y as i64 + dy, x as i64 + dx);
                if yy < 0 || xx < 0 || yy >= n as i64 || xx >= n as i64 {
                    continue;
                }
                let k = yy as usize * n + xx as usize;
                if let (Some(l), Some(tk)) = (scratch.level.get_mut(k), scratch.tint.get_mut(k))
                    && i * w > *l
                {
                    *l = i * w;
                    *tk = t;
                }
            }
        }
    }
    reset(out, cells.saturating_mul(4), 0u8);
    for ((o, l), t) in out.as_chunks_mut::<4>().0.iter_mut().zip(&scratch.level).zip(&scratch.tint) {
        if *l > 0.0 {
            o.copy_from_slice(&[
                (t[0] * l * 255.0).round() as u8,
                (t[1] * l * 255.0).round() as u8,
                (t[2] * l * 255.0).round() as u8,
                (l * 255.0).round() as u8,
            ]);
        }
    }
    (n, n)
}
