// SPDX-License-Identifier: MIT OR Apache-2.0
//! More image operations with the conventions of compiled node graphs, each measured against the original renderer:
//! see the crate's decompiler for how compiled nodes map onto them. `compute` returns Ok(None) for kinds it does not
//! know.

use crate::graph::{Error, Node, Spec};
use crate::math;
use crate::measured::INTENSITY_SPAN;
use crate::tex::Tex;
use std::collections::BTreeMap;
use std::sync::Arc as Rc;

/// What a node's text leaves out. Distance: 10, measurement "Operations identified, by fitting" (0x2a "Default distance 10").
/// Normal: 1, the same section (0x25 "default 1"). Emboss: 1, no measurement, a guess.
const DISTANCE_DEFAULT: f32 = 10.0;
const NORMAL_DEFAULT_INTENSITY: f32 = 1.0;
const EMBOSS_DEFAULT_INTENSITY: f32 = 1.0;
/// A distance mask's pixel is set from this value on. Measurement "Operations identified, by fitting" (0x2a: "to the nearest
/// pixel >= 0.5").
const DISTANCE_SET_FROM: f32 = 0.5;
/// An aspect this small counts as this, so the axis reaches nowhere instead of dividing by zero (a guard).
const ASPECT_FLOOR_SQ: f32 = 1e-6;
/// Format data: a vector shape's vertices are 16-bit fixed point, `k / 65535` of the image. Measurement "Operations identified,
/// by fitting" (0x0a: "n u16 x,y pairs").
const STRIP_COORD_STEPS: f32 = 65535.0;

pub const SPECS: &[Spec] = &[
    Spec {
        kind: "curve_keys",
        req: &["in"],
        opt: &[],
        doc: "tone curve from Bezier keys: n, then per key k xk yk (point) lxk lyk (left handle) rxk ryk (right handle); ramp=1 draws the curve left to right instead of applying it",
    },
    Spec {
        kind: "triangle_strips",
        req: &[],
        opt: &[],
        doc: "white triangle strips on black: strips=\"...\", each strip hex digits, 8 per vertex (yyyyxxxx, 0..ffff across the image), strips apart by a space",
    },
    Spec {
        kind: "distance_map",
        req: &["in"],
        opt: &["source"],
        doc: "distance from the mask's bright pixels: distance (reach in 1/256 of the image, default 10), aspect_x aspect_y, tile_x tile_y (1, the default: across the image's edge); with a source, the colour of the nearest mask pixel, fade=1 fading it out with distance",
    },
    Spec {
        kind: "normal_map",
        req: &["in"],
        opt: &[],
        doc: "tangent-space normals from a height: intensity (default 1, scaled by width/256), flip_y, alpha=1 keeps the height in alpha, aspect_x aspect_y",
    },
    Spec {
        kind: "emboss_map",
        req: &["in", "map"],
        opt: &[],
        doc: "adds the slope of map along a direction: intensity (default 1, scaled by width/256), angle (turns), aspect_x aspect_y",
    },
    Spec {
        kind: "gradient_dynamic",
        req: &["in", "grad"],
        opt: &[],
        doc: "look up the grad image with the input: in picks the column (vertical=1 the row), position the other coordinate",
    },
];

/// Run a node of this module. Ok(None) = not one of ours.
pub fn compute(kind: &str, id: u32, n: &Node, ins: &BTreeMap<String, Rc<Tex>>, size: [usize; 2]) -> Result<Option<Tex>, Error> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let need = |slot: &str| -> Result<&Tex, Error> { ins.get(slot).map(|r| r.as_ref()).ok_or_else(|| Error::MissingInput(id, slot.to_string())) };
    Ok(Some(match kind {
        "curve_keys" => {
            let keys: Vec<CurveKey> = (0..f("n", 0.0).max(0.0) as usize)
                .map(|k| {
                    let g = |name: &str| f(&format!("{name}{k}"), 0.0);
                    CurveKey { point: [g("x"), g("y")], left: [g("lx"), g("ly")], right: [g("rx"), g("ry")] }
                })
                .collect();
            let table = curve_table(&keys);
            let t = need("in")?;
            if f("ramp", 0.0) > 0.5 {
                let (w, h) = (t.w, t.h);
                Tex::gray_fn(w, h, |u, _| table_ramp(&table, u, w))
            } else {
                t.map(|p| [table_lookup(&table, p[0]), table_lookup(&table, p[1]), table_lookup(&table, p[2]), p[3]])
            }
        }
        "triangle_strips" => {
            let strips = parse_strips(n.strs.get("strips").map_or("", String::as_str)).ok_or_else(|| Error::BadParam(id, "bad strips".into()))?;
            let (w, h) = (size[0], size[1]);
            let c = strips_coverage(&strips, w, h);
            let mut t = Tex::solid(w, h, [0.0, 0.0, 0.0, 1.0]);
            for (p, v) in t.px.iter_mut().zip(c) {
                *p = [v, v, v, 1.0];
            }
            t
        }
        "distance_map" => {
            let t = need("in")?;
            let reach = f("distance", DISTANCE_DEFAULT) * t.w as f32 / INTENSITY_SPAN;
            // the distance is in the units across, so on a node of another shape a pixel down counts width/height pixels (measured:
            // leather_strip #546, 512 x 128, falls off over 112.57/256 of the height down; pipe_rack #47, 256 x 32, over 32/256 of the
            // width across)
            let (ax, ay) = (f("aspect_x", 1.0), f("aspect_y", 1.0) * t.h as f32 / t.w as f32);
            let field = NearestField::new(
                t.w,
                t.h,
                |x, y| t.px_at(y * t.w + x)[0] >= DISTANCE_SET_FROM,
                ax,
                ay,
                [f("tile_x", 1.0) > 0.5, f("tile_y", 1.0) > 0.5],
            );
            let source = ins.get("source").map(|r| r.as_ref());
            let fade = f("fade", 0.0) > 0.5;
            let mut out = Tex::solid(t.w, t.h, [0.0, 0.0, 0.0, 1.0]);
            for y in 0..t.h {
                for x in 0..t.w {
                    let Some((d2, nx, ny)) = field.at(x, y) else { continue };
                    let d = d2.sqrt();
                    let falloff = if reach > 0.0 { (1.0 - d / reach).clamp(0.0, 1.0) } else { f32::from(u8::from(d2 == 0.0)) };
                    let v = match source {
                        None => [falloff, falloff, falloff, 1.0],
                        Some(s) => {
                            let c = s.px_at((ny * s.h / t.h) * s.w + nx * s.w / t.w);
                            if fade {
                                [c[0] * falloff, c[1] * falloff, c[2] * falloff, c[3]]
                            } else if d < reach || d2 == 0.0 {
                                c
                            } else {
                                [0.0, 0.0, 0.0, 1.0]
                            }
                        }
                    };
                    out.set_px(y * t.w + x, v);
                }
            }
            out
        }
        "normal_map" => {
            let t = need("in")?;
            let k = f("intensity", NORMAL_DEFAULT_INTENSITY) * t.w as f32 / INTENSITY_SPAN;
            // on a node of another shape the slope down is in the same units as across, height over width per pixel (measured on
            // 512 x 128 nodes: leather_strip #363 at intensity 64 has 64 across and 16 down per pixel step; bandage #42 and
            // fabric_edge_wear #26 with their aspect expressions come out equal, ours 4 times that down: means 15 to 21 off)
            let (kx, ky) = (k * f("aspect_x", 1.0), k * f("aspect_y", 1.0) * t.h as f32 / t.w as f32);
            let sign_y = if f("flip_y", 0.0) > 0.5 { 1.0 } else { -1.0 };
            let alpha = f("alpha", 0.0) > 0.5;
            per_pixel(t, |x, y| {
                let (gx, gy) = backward_slope(t, x, y, 0);
                let (nx, ny) = (-kx * gx, sign_y * ky * gy);
                let len = (nx * nx + ny * ny + 1.0).sqrt();
                [0.5 * nx / len + 0.5, 0.5 * ny / len + 0.5, 0.5 / len + 0.5, if alpha { t.px_at(y * t.w + x)[0] } else { 1.0 }]
            })
        }
        "emboss_map" => {
            let (t, map) = (need("in")?, need("map")?);
            let k = f("intensity", EMBOSS_DEFAULT_INTENSITY) * t.w as f32 / INTENSITY_SPAN;
            let (sn, cs) = math::sin_cos(f("angle", 0.0) * std::f32::consts::TAU);
            let (kx, ky) = (k * f("aspect_x", 1.0) * cs, k * f("aspect_y", 1.0) * sn);
            per_pixel(t, |x, y| {
                let (gx, gy) = backward_slope(map, x * map.w / t.w, y * map.h / t.h, 0);
                let add = kx * gx + ky * gy;
                let p = t.px_at(y * t.w + x);
                [(p[0] + add).clamp(0.0, 1.0), (p[1] + add).clamp(0.0, 1.0), (p[2] + add).clamp(0.0, 1.0), p[3]]
            })
        }
        "gradient_dynamic" => {
            let (t, grad) = (need("in")?, need("grad")?);
            let vertical = f("vertical", 0.0) > 0.5;
            let position = f("position", 0.0).clamp(0.0, 1.0);
            let (gw, gh) = (grad.w.saturating_sub(1) as f32, grad.h.saturating_sub(1) as f32);
            t.map(|p| {
                let v = p[0].clamp(0.0, 1.0);
                if vertical { texel_lerp(grad, position * gw, v * gh) } else { texel_lerp(grad, v * gw, position * gh) }
            })
        }
        _ => return Ok(None),
    }))
}

/// Vertices (0..1) of the strips in the text form of `triangle_strips`.
fn parse_strips(text: &str) -> Option<Vec<Vec<[f32; 2]>>> {
    text.split_whitespace()
        .map(|strip| {
            let bytes = strip.as_bytes();
            bytes
                .chunks(8)
                .map(|c| {
                    let v = u32::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()?;
                    Some([(v & 0xffff) as f32 / STRIP_COORD_STEPS, (v >> 16) as f32 / STRIP_COORD_STEPS])
                })
                .collect()
        })
        .collect()
}

/// Fraction of each pixel covered by the strips: their union, sampled 4 x 4 per pixel (a choice: measurement "Operations identified,
/// by fitting" (0x0a) has 99.2 to 99.8% of pixels, the edges off). A pixel's samples are at
/// (i + 0.5) / 4 of it. Triangles without area leave no trace (strips join their parts with such triangles).
fn strips_coverage(strips: &[Vec<[f32; 2]>], w: usize, h: usize) -> Vec<f32> {
    const SS: usize = 4;
    let (sw, sh) = (w * SS, h * SS);
    let rows: Vec<Vec<u8>> = crate::tex::par_map(h, |py| {
        let mut row = vec![0u8; w];
        let mut mask = vec![false; sw];
        for sy in py * SS..(py + 1) * SS {
            mask.fill(false);
            let y = sy as f32 + 0.5;
            for strip in strips {
                for tri in strip.windows(3) {
                    let &[ta, tb, tc] = tri else { continue };
                    let p = |q: [f32; 2]| [q[0] * sw as f32, q[1] * sh as f32];
                    let (a, b, c) = (p(ta), p(tb), p(tc));
                    // the span of this sample row inside the triangle
                    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                    for (s, e) in [(a, b), (b, c), (c, a)] {
                        if (s[1] <= y) != (e[1] <= y) {
                            let x = s[0] + (y - s[1]) / (e[1] - s[1]) * (e[0] - s[0]);
                            lo = lo.min(x);
                            hi = hi.max(x);
                        }
                    }
                    if lo < hi {
                        let from = (lo - 0.5).ceil().max(0.0) as usize;
                        let to = ((hi - 0.5).floor() + 1.0).clamp(0.0, sw as f32) as usize;
                        if from < to
                            && let Some(m) = mask.get_mut(from..to)
                        {
                            m.fill(true);
                        }
                    }
                }
            }
            for (cell, m) in row.iter_mut().zip(mask.as_chunks::<SS>().0) {
                *cell += m.iter().filter(|m| **m).count() as u8;
            }
        }
        row
    });
    rows.into_iter().flatten().map(|v| f32::from(v) / (SS * SS) as f32).collect()
}

/// A texture from a function of the pixel. Runs multi-threaded.
fn per_pixel<F: Fn(usize, usize) -> [f32; 4] + Sync>(t: &Tex, f: F) -> Tex {
    let (w, h) = (t.w, t.h);
    Tex::from_fn(w, h, |u, v| f(((u * w as f32) as usize).min(w - 1), ((v * h as f32) as usize).min(h - 1)))
}

/// The change of channel `c` from the pixel before, along x and y, wrapped.
fn backward_slope(t: &Tex, x: usize, y: usize, c: usize) -> (f32, f32) {
    let at = |x: usize, y: usize| t.px_at(y * t.w + x).get(c).copied().unwrap_or(0.0);
    (at(x, y) - at((x + t.w - 1) % t.w, y), at(x, y) - at(x, (y + t.h - 1) % t.h))
}

/// Bilinear between texels, by texel coordinates, stopping at the edges.
fn texel_lerp(t: &Tex, x: f32, y: f32) -> [f32; 4] {
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(t.w.saturating_sub(1)), (y0 + 1).min(t.h.saturating_sub(1)));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: usize, y: usize| t.px_at(y * t.w + x);
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    let mix = |a: f32, b: f32, c: f32, d: f32| (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy;
    [mix(a[0], b[0], c[0], d[0]), mix(a[1], b[1], c[1], d[1]), mix(a[2], b[2], c[2], d[2]), mix(a[3], b[3], c[3], d[3])]
}

/// The nearest set pixel of a mask for every pixel, with a distance that may weigh the axes differently, across the edges on
/// the axes that repeat.
struct NearestField {
    w: usize,
    /// Per pixel: squared distance and the nearest set pixel; `None` everywhere when no pixel is set.
    cells: Vec<Option<(f32, usize, usize)>>,
}

impl NearestField {
    /// Exact Euclidean distances in two passes: along each column, then the lower envelope of parabolas along each
    /// row, over three periods so that the nearest pixel may lie across the wrap (one when the axis doesn't repeat).
    fn new(w: usize, h: usize, set: impl Fn(usize, usize) -> bool + Sync, ax: f32, ay: f32, tile: [bool; 2]) -> NearestField {
        // for each column: distance (in rows, wrapped) to the nearest set pixel and that pixel's row
        let columns: Vec<Vec<Option<(usize, usize)>>> = crate::tex::par_map(w, |x| {
            let rows: Vec<usize> = (0..h).filter(|&y| set(x, y)).collect();
            let Some((&first, &last)) = rows.first().zip(rows.last()) else { return vec![None; h] };
            let mut out = vec![None; h];
            let mut next = 0;
            for (y, cell) in out.iter_mut().enumerate() {
                while rows.get(next).is_some_and(|&r| r < y) {
                    next += 1;
                }
                // past the last or first set pixel the nearest one is across the edge, or else very far
                let far = (usize::MAX / 4, first);
                let below = rows.get(next).copied().map_or(if tile[1] { (first + h - y, first) } else { far }, |r| (r - y, r));
                let above = if let Some(&r) = next.checked_sub(1).and_then(|i| rows.get(i)) {
                    (y - r, r)
                } else if tile[1] {
                    (y + h - last, last)
                } else {
                    far
                };
                *cell = Some(if above.0 < below.0 { above } else { below });
            }
            out
        });
        // the aspect divides: at 0 an axis doesn't reach across at all
        let weight = |a: f32| 1.0 / (a * a).max(ASPECT_FLOOR_SQ);
        let (ax2, ay2) = (weight(ax), weight(ay));
        let rows: Vec<Vec<Option<(f32, usize, usize)>>> = crate::tex::par_map(h, |y| {
            // parabolas at x + k w with height g^2, k = -1, 0, 1; without repeating only those of k = 0
            let (n, base) = if tile[0] { (3 * w, 0) } else { (2 * w, w) };
            let mut site = vec![0usize; n];
            let mut from = vec![0f32; n];
            let mut top = 0usize;
            let mut any = false;
            let column = |i: usize| columns.get(i % w).and_then(|c| c.get(y)).copied().flatten();
            let height = |i: usize| column(i).map(|(g, _)| (g * g) as f32 * ay2);
            let site_at = |site: &[usize], k: usize| site.get(k).copied().unwrap_or(0);
            let from_at = |from: &[f32], k: usize| from.get(k).copied().unwrap_or(f32::NEG_INFINITY);
            let put = |v: &mut [usize], k: usize, x: usize| {
                if let Some(e) = v.get_mut(k) {
                    *e = x;
                }
            };
            for i in base..n {
                let Some(hi) = height(i) else { continue };
                if !any {
                    any = true;
                    put(&mut site, 0, i);
                    if let Some(e) = from.first_mut() {
                        *e = f32::NEG_INFINITY;
                    }
                    continue;
                }
                let cross = |p: usize| {
                    let hp = height(p).unwrap_or(0.0);
                    let (pf, qf) = (p as f32, i as f32);
                    (hi - hp + ax2 * (qf * qf - pf * pf)) / (2.0 * ax2 * (qf - pf))
                };
                let mut s = cross(site_at(&site, top));
                while s <= from_at(&from, top) && top > 0 {
                    top -= 1;
                    s = cross(site_at(&site, top));
                }
                if s <= from_at(&from, top) {
                    put(&mut site, top, i);
                } else {
                    top += 1;
                    put(&mut site, top, i);
                    if let Some(e) = from.get_mut(top) {
                        *e = s;
                    }
                }
            }
            if !any {
                return vec![None; w];
            }
            let mut k = 0;
            (0..w)
                .map(|x| {
                    let q = (x + w) as f32;
                    while k < top && from.get(k + 1).is_some_and(|&f| f < q) {
                        k += 1;
                    }
                    let i = site_at(&site, k);
                    let (g, row) = column(i)?;
                    let dx = q - i as f32;
                    Some((ax2 * dx * dx + (g * g) as f32 * ay2, i % w, row))
                })
                .collect()
        });
        NearestField { w, cells: rows.into_iter().flatten().collect() }
    }

    /// Squared distance and the nearest set pixel of pixel (x, y).
    fn at(&self, x: usize, y: usize) -> Option<(f32, usize, usize)> {
        self.cells.get(y * self.w + x).copied().flatten()
    }
}

/// One key of a tone curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveKey {
    pub point: [f32; 2],
    pub left: [f32; 2],
    pub right: [f32; 2],
}

/// Entries of the table a curve is evaluated through; measured, it shows where a curve is very steep. Measurement "Operations
/// identified, by fitting" (0x2c: "Bezier keys through a 1024-entry table").
const CURVE_TABLE: usize = 1024;

/// The curve's value at `CURVE_TABLE` evenly spaced inputs from 0 to 1. Before the first key and after the last the
/// value stays that key's.
pub fn curve_table(keys: &[CurveKey]) -> Vec<f32> {
    let bezier = |a: f32, b: f32, c: f32, d: f32, t: f32| {
        let s = 1.0 - t;
        s * s * s * a + 3.0 * s * s * t * b + 3.0 * s * t * t * c + t * t * t * d
    };
    (0..CURVE_TABLE)
        .map(|i| {
            let x = i as f32 / (CURVE_TABLE - 1) as f32;
            let (Some(first), Some(last)) = (keys.first(), keys.last()) else { return x };
            if x <= first.point[0] {
                return first.point[1];
            }
            if x >= last.point[0] {
                return last.point[1];
            }
            let Some((a, b)) = keys.windows(2).find_map(|p| match *p {
                [a, b] if x < b.point[0] => Some((a, b)),
                _ => None,
            }) else {
                return last.point[1];
            };
            // the x of a segment rises with t, so bisection finds the t of this input
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..32 {
                let t = 0.5 * (lo + hi);
                if bezier(a.point[0], a.right[0], b.left[0], b.point[0], t) > x {
                    hi = t;
                } else {
                    lo = t;
                }
            }
            bezier(a.point[1], a.right[1], b.left[1], b.point[1], 0.5 * (lo + hi))
        })
        .collect()
}

/// Linear interpolation in the table, for an input clamped to 0..1.
fn table_lookup(table: &[f32], v: f32) -> f32 {
    let p = v.clamp(0.0, 1.0) * table.len().saturating_sub(1) as f32;
    let i = (p as usize).min(table.len().saturating_sub(2));
    let t = p - i as f32;
    let at = |i: usize| table.get(i).copied().unwrap_or(0.0);
    at(i) * (1.0 - t) + at(i + 1) * t
}

/// The table stretched over an image `w` pixels wide, by texels: u = (x + 0.5) / w.
fn table_ramp(table: &[f32], u: f32, w: usize) -> f32 {
    let n = table.len();
    let p = u * n as f32 - 0.5;
    if w >= n {
        let i = p.floor().clamp(0.0, n.saturating_sub(2) as f32);
        let t = (p - i).clamp(0.0, 1.0);
        let i = i as usize;
        let at = |i: usize| table.get(i).copied().unwrap_or(0.0);
        return at(i) * (1.0 - t) + at(i + 1) * t;
    }
    // shrinking by a whole factor is a box filter
    let k = n / w;
    let start = ((u * n as f32) as usize).saturating_sub(k / 2).min(n - k);
    table.get(start..start + k).unwrap_or_default().iter().sum::<f32>() / k as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    #[test]
    fn a_straight_curve_is_the_identity_and_outside_the_keys_it_holds() {
        let k = |x: f32, y: f32, dx: f32, dy: f32| CurveKey { point: [x, y], left: [x - dx, y - dy], right: [x + dx, y + dy] };
        let table = curve_table(&[k(0.25, 0.25, 0.1, 0.1), k(0.75, 0.75, 0.1, 0.1)]);
        assert!((table_lookup(&table, 0.5) - 0.5).abs() < 1e-3);
        assert_eq!(table_lookup(&table, 0.1), 0.25);
        assert_eq!(table_lookup(&table, 0.9), 0.75);
    }

    #[test]
    fn handles_bend_the_curve_as_a_cubic() {
        // (0,0) to (1,1) with handles (0,1) and (1,0): at t = 1/2 the point is (0.5, 0.5), at t = 1/4 x = 0.15625, y = 0.4375
        let a = CurveKey { point: [0.0, 0.0], left: [0.0, 0.0], right: [0.0, 1.0] };
        let b = CurveKey { point: [1.0, 1.0], left: [1.0, 0.0], right: [1.0, 1.0] };
        let table = curve_table(&[a, b]);
        assert!((table_lookup(&table, 0.5) - 0.5).abs() < 1e-3);
        assert!((table_lookup(&table, 0.15625) - 0.4375).abs() < 2e-3);
    }

    #[test]
    fn ramp_draws_the_curve_from_left_to_right_and_ignores_the_input() {
        let g = "size 64\nnode 1 color r=0 g=0 b=0\nnode 2 curve_keys in=@1 ramp=1 n=2 x0=0 y0=1 lx0=0 ly0=1 rx0=0.25 ry0=0.75 x1=1 y1=0 lx1=0.75 ly1=0.25 rx1=1 ry1=0\noutput o 2\n";
        let t = &Graph::parse(g).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(64)).unwrap()["o"];
        assert!(t.px[10 * 64 + 1][0] > 0.9 && t.px[10 * 64 + 62][0] < 0.1, "{:?}", t.px[10 * 64 + 1]);
        assert_eq!(t.px[3 * 64 + 40], t.px[60 * 64 + 40]);
    }

    fn eval(body: &str) -> Tex {
        Graph::parse(&format!("size 64\n{body}")).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(64)).unwrap()["o"].clone()
    }

    #[test]
    fn distance_falls_off_linearly_and_wraps() {
        // a white column at the left edge of a texture; reach 16/256 of 64 = 4 pixels
        let field = NearestField::new(64, 64, |x, _| x == 0, 1.0, 1.0, [true, true]);
        assert_eq!(field.at(3, 5), Some((9.0, 0, 5)));
        // across the wrap: pixel 62 is 2 from pixel 0
        assert_eq!(field.at(62, 5), Some((4.0, 0, 5)));
        // everything is mask here, so everything is at distance 0
        let t = eval("node 1 color r=1 g=1 b=1\nnode 2 distance_map in=@1 distance=16\noutput o 2\n");
        assert_eq!(t.px[9][0], 1.0);
        // an empty mask has nothing near it
        let t = eval("node 1 color r=0 g=0 b=0\nnode 2 distance_map in=@1 distance=16\noutput o 2\n");
        assert_eq!(t.px[9][0], 0.0);
    }

    #[test]
    fn distance_does_not_cross_an_edge_that_does_not_repeat() {
        let field = NearestField::new(64, 64, |x, y| x == 0 && y == 0, 1.0, 1.0, [false, false]);
        assert_eq!(field.at(62, 0), Some((62.0 * 62.0, 0, 0)));
        assert_eq!(field.at(0, 62), Some((62.0 * 62.0, 0, 0)));
        let wrapped = NearestField::new(64, 64, |x, y| x == 0 && y == 0, 1.0, 1.0, [true, false]);
        assert_eq!(wrapped.at(62, 0), Some((4.0, 0, 0)));
        assert_eq!(wrapped.at(0, 62), Some((62.0 * 62.0, 0, 0)));
    }

    #[test]
    fn distance_weighs_the_axes_by_the_aspect() {
        let field = NearestField::new(32, 32, |x, y| x == 0 && y == 0, 0.5, 2.0, [true, true]);
        // 2 pixels along x weigh 4, 8 along y weigh 4
        assert_eq!(field.at(2, 8), Some((32.0, 0, 0)));
    }

    #[test]
    fn distance_down_a_non_square_node_is_in_the_units_across() {
        // a white row at the top of a 64 x 16 node; reach 64/256 of the width across is 16 pixels, a quarter of the height
        // down, so 2 rows below the line (an eighth of the height) is half way
        let g = "size 64\nsubgraph top\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"y\"\nnode 4 f_float v=0.05\nnode 5 f_lt a=@3 b=@4\nnode 1 f_out in=@5\nend\nnode 1 pixel_processor fn=\"top\" osize_log2_x=6 osize_log2_y=4\nnode 2 distance_map in=@1 distance=64 tile_y=0 osize_log2_x=6 osize_log2_y=4\noutput o 2\n";
        let t = &Graph::parse(g).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(64)).unwrap()["o"];
        assert_eq!((t.w, t.h), (64, 16));
        assert!((t.px[2 * 64 + 10][0] - 0.5).abs() < 1e-5, "{:?}", t.px[2 * 64 + 10]);
    }

    #[test]
    fn distance_gives_the_nearest_source_within_reach_only() {
        let g = "node 1 color r=1 g=1 b=1\nnode 2 color r=0.25 g=0.5 b=0.75\nnode 3 distance_map in=@1 source=@2 distance=4\noutput o 3\n";
        let t = eval(g);
        assert_eq!(t.px[10], [0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn normal_points_away_from_a_slope_and_flat_is_up() {
        // a uv ramp along x rises 1/64 per pixel; intensity 64 at width 64 gives k = 16, nx = -16/64 = -0.25 before normalizing
        let ramp = "subgraph c\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"c\"\n";
        let t = eval(&format!("{ramp}node 2 normal_map in=@1 intensity=64\noutput o 2\n"));
        let n = t.px[5 * 64 + 20];
        let expect = -0.25 / (1.0f32 + 0.0625).sqrt() * 0.5 + 0.5;
        assert!((n[0] - expect).abs() < 1e-3, "{n:?}");
        assert!((n[1] - 0.5).abs() < 1e-5);
        let flat = eval("node 1 color r=0.7 g=0.7 b=0.7\nnode 2 normal_map in=@1 intensity=64 alpha=1\noutput o 2\n");
        assert_eq!(flat.px[3], [0.5, 0.5, 1.0, 0.7]);
    }

    #[test]
    fn a_normal_down_a_non_square_node_is_the_square_one() {
        // a uv ramp along y on a 64 x 16 node steps 1/16 per pixel, four times the square node's 1/64, and gives the same normal
        let ramp = "subgraph c\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"y\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"c\"";
        let at = |size: &str, w: usize| eval(&format!("{ramp} {size}\nnode 2 normal_map in=@1 intensity=64 {size}\noutput o 2\n")).px[5 * w + 20];
        let (square, wide) = (at("", 64), at("osize_log2_x=6 osize_log2_y=4", 64));
        assert!((square[1] - wide[1]).abs() < 1e-4, "{square:?} {wide:?}");
        assert!((square[1] - 0.5).abs() > 0.05);
    }

    #[test]
    fn emboss_adds_the_slope_along_the_angle() {
        let ramp = "subgraph c\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"y\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"c\"\nnode 2 color r=0.5 g=0.5 b=0.5\n";
        // along y, slope 1/64 per pixel, gain 16 * 64/256 = 4: adds 1/16
        let t = eval(&format!("{ramp}node 3 emboss_map in=@2 map=@1 intensity=16 angle=0.25\noutput o 3\n"));
        assert!((t.px[20 * 64 + 5][0] - 0.5625).abs() < 1e-3, "{:?}", t.px[20 * 64 + 5]);
        // along x the ramp has no slope
        let t = eval(&format!("{ramp}node 3 emboss_map in=@2 map=@1 intensity=16\noutput o 3\n"));
        assert!((t.px[20 * 64 + 5][0] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn gradient_dynamic_maps_the_input_to_the_last_texel_at_one() {
        let ramp = "subgraph c\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"c\"\nnode 2 color r=1 g=1 b=1\n";
        let t = eval(&format!("{ramp}node 3 gradient_dynamic in=@2 grad=@1\noutput o 3\n"));
        assert!((t.px[100][0] - 63.5 / 64.0).abs() < 1e-4, "{:?}", t.px[100]);
    }

    #[test]
    fn strips_cover_their_triangles_and_no_more() {
        // a strip over the left half (with a zero-area triangle joining a second triangle at the far corner)
        let g = "node 1 triangle_strips strips=\"0000000000007fffffff0000ffff7fff\"\noutput o 1\n";
        let t = eval(g);
        assert_eq!(t.px[10 * 64 + 10][0], 1.0);
        assert_eq!(t.px[10 * 64 + 50][0], 0.0);
        // the exact edge at x = 0.5 falls between pixels
        assert_eq!(t.px[10 * 64 + 31][0], 1.0);
        let thin = strips_coverage(&[vec![[0.0, 0.0], [1.0, 1.0], [1.0, 1.0]]], 8, 8);
        assert!(thin.iter().all(|v| *v == 0.0), "a triangle without area draws nothing");
    }
}
