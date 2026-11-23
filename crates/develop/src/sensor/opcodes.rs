//! DNG opcode lists (`OpcodeList1/2/3`, DNG 1.7 chapter 7). Opcode lists are always big-endian.
//!
//! Parsed (by `craft-raw`): every opcode (unknown ones are kept as [`Opcode::Unknown`]). Applied: `WarpRectilinear`,
//! `WarpRectilinear2` (opcode 14, DNG 1.6; see [`Opcode::WarpRectilinear2`]), `FixVignetteRadial`,
//! `FixBadPixelsConstant`, `FixBadPixelsList`, `MapTable`, `MapPolynomial`, `GainMap`, `DeltaPerRow/Column`,
//! `ScalePerRow/Column`. `TrimBounds` and `WarpFisheye` are recorded but not applied.
//!
//! Opcodes run in list order, except for the DNG 1.6 skip rule ([`applied_ops`]): a `WarpRectilinear` or
//! `WarpFisheye` that immediately follows an *optional* `WarpRectilinear2` is skipped (the older opcode is the
//! fallback for readers that don't know opcode 14).
//!
//! Value convention: list 1 runs on raw sample values (16-bit scale), lists 2 and 3 on values normalised to
//! [0, 1]; table/polynomial/delta opcodes are defined on the normalised range and are rescaled accordingly.

pub use craft_raw::opcodes::{Area, Opcode, OpcodeLists, WR2_LEN, applied_ops, parse_list, wr2_factor, write_list};

use super::{Cfa, Rgb32f};

/// Whether [`apply_list`] / [`apply_list3`] implement `op`.
pub fn is_applied(op: &Opcode) -> bool {
    !matches!(op, Opcode::Unknown { .. } | Opcode::TrimBounds { .. } | Opcode::WarpFisheye { .. })
}

fn area_iter(a: &Area, w: usize, h: usize, cpp: usize) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    let (t, l) = (a.top as usize, a.left as usize);
    let (b, r) = ((a.bottom as usize).min(h), (a.right as usize).min(w));
    let (p0, p1) = (a.plane as usize, (a.plane as usize + a.planes.max(1) as usize).min(cpp));
    (t..b.max(t))
        .step_by(a.row_pitch.max(1) as usize)
        .flat_map(move |y| (l..r.max(l)).step_by(a.col_pitch.max(1) as usize).flat_map(move |x| (p0..p1.max(p0)).map(move |p| (x, y, p))))
}

struct GainMapGrid<'a> {
    points: [usize; 2],
    spacing: [f64; 2],
    origin: [f64; 2],
    planes: usize,
    gains: &'a [f32],
}

impl GainMapGrid<'_> {
    fn sample_at(&self, position: [f64; 2], plane: usize) -> f32 {
        let [pv, ph] = self.points;
        let mp = self.planes;
        let p = plane.min(mp - 1);
        let fy = if self.spacing[0] > 0.0 { ((position[0] - self.origin[0]) / self.spacing[0]).clamp(0.0, (pv - 1) as f64) } else { 0.0 };
        let fx = if self.spacing[1] > 0.0 { ((position[1] - self.origin[1]) / self.spacing[1]).clamp(0.0, (ph - 1) as f64) } else { 0.0 };
        let (y0, x0) = (fy.floor() as usize, fx.floor() as usize);
        let (y1, x1) = ((y0 + 1).min(pv - 1), (x0 + 1).min(ph - 1));
        let (ty, tx) = ((fy - y0 as f64) as f32, (fx - x0 as f64) as f32);
        let g = |y: usize, x: usize| self.gains[(y * ph + x) * mp + p];
        let top = g(y0, x0) + (g(y0, x1) - g(y0, x0)) * tx;
        let bot = g(y1, x0) + (g(y1, x1) - g(y1, x0)) * tx;
        top + (bot - top) * ty
    }
}

/// Radius normaliser: distance from the centre to the farthest image corner.
pub(crate) fn max_radius(cx: f64, cy: f64, w: f64, h: f64) -> f64 {
    [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].iter().map(|&(x, y)| (x - cx).hypot(y - cy)).fold(0.0, f64::max).max(1e-9)
}

/// Most pixels one bad-pixel opcode repairs (the list is 64 MB, `fix_pixels`' set about twice that). Uncapped, a
/// mosaic equal to `FixBadPixelsConstant`'s constant, or one `FixBadPixelsList` rectangle over the image, lists
/// every pixel: tens of GB at [`crate::MAX_SAMPLES`]. Pixels past the cap, in list order, are left as they are.
const MAX_BAD_PIXELS: usize = 1 << 22;

/// Apply a list to an interleaved `w × h × cpp` buffer. `cfa` (when the data is a mosaic) is used by the
/// bad-pixel opcodes. `scale` is the value of 1.0 in the buffer: 65535 for list 1 (raw 16-bit values), 1 for
/// lists 2 and 3 (normalised values).
/// Whole-image passes one opcode list may cost, all its ops together (a real DNG's list is a
/// handful of gain maps and tables). Ops past it, in list order, are left out: a list repeating a
/// full-image polynomial thousands of times was billions of evaluations on an ordinary raw.
const MAX_LIST_PASSES: usize = 32;

/// Sample visits `op` costs on a `w × h × cpp` buffer (coefficient evaluations for a polynomial).
fn op_cost(op: &Opcode, w: usize, h: usize, cpp: usize) -> usize {
    let visits = |a: &Area| {
        let rows = (a.bottom as usize).min(h).saturating_sub(a.top as usize).div_ceil(a.row_pitch.max(1) as usize);
        let cols = (a.right as usize).min(w).saturating_sub(a.left as usize).div_ceil(a.col_pitch.max(1) as usize);
        let planes = (a.plane as usize).saturating_add(a.planes.max(1) as usize).min(cpp).saturating_sub(a.plane as usize);
        rows.saturating_mul(cols).saturating_mul(planes)
    };
    match op {
        Opcode::MapPolynomial { area, coefficients } => visits(area).saturating_mul(coefficients.len().max(1)),
        Opcode::MapTable { area, .. }
        | Opcode::GainMap { area, .. }
        | Opcode::DeltaPerRow { area, .. }
        | Opcode::DeltaPerColumn { area, .. }
        | Opcode::ScalePerRow { area, .. }
        | Opcode::ScalePerColumn { area, .. } => visits(area),
        // `apply_list` does nothing for these, so they spend none of the budget
        Opcode::Unknown { .. }
        | Opcode::TrimBounds { .. }
        | Opcode::WarpFisheye { .. }
        | Opcode::WarpRectilinear { .. }
        | Opcode::WarpRectilinear2 { .. } => 0,
        _ => w.saturating_mul(h).saturating_mul(cpp),
    }
}

pub fn apply_list(list: &[Opcode], buf: &mut [f32], w: usize, h: usize, cpp: usize, cfa: Option<&Cfa>, scale: f32) {
    let mut budget = w.saturating_mul(h).saturating_mul(cpp).max(1).saturating_mul(MAX_LIST_PASSES);
    for op in list {
        let cost = op_cost(op, w, h, cpp);
        if cost > budget {
            continue;
        }
        budget -= cost;
        match op {
            Opcode::MapTable { area, table } => {
                let n = table.len();
                for (x, y, p) in area_iter(area, w, h, cpp) {
                    let v = &mut buf[(y * w + x) * cpp + p];
                    let idx = ((*v / scale * 65535.0).round().max(0.0) as usize).min(n - 1);
                    *v = table[idx] as f32 / 65535.0 * scale;
                }
            }
            Opcode::MapPolynomial { area, coefficients } => {
                for (x, y, p) in area_iter(area, w, h, cpp) {
                    let v = &mut buf[(y * w + x) * cpp + p];
                    let t = (*v / scale) as f64;
                    let r = coefficients.iter().rev().fold(0.0, |acc, &c| acc * t + c);
                    *v = r as f32 * scale;
                }
            }
            Opcode::GainMap { area, points_v, points_h, spacing, origin, map_planes, gains } => {
                let grid = GainMapGrid {
                    points: [*points_v as usize, *points_h as usize],
                    spacing: *spacing,
                    origin: *origin,
                    planes: *map_planes as usize,
                    gains,
                };
                for (x, y, p) in area_iter(area, w, h, cpp) {
                    let rv = y as f64 / h as f64;
                    let rh = x as f64 / w as f64;
                    let g = grid.sample_at([rv, rh], p - area.plane as usize);
                    buf[(y * w + x) * cpp + p] *= g;
                }
            }
            Opcode::DeltaPerRow { area, deltas } | Opcode::DeltaPerColumn { area, deltas } => {
                let per_row = matches!(op, Opcode::DeltaPerRow { .. });
                for (x, y, p) in area_iter(area, w, h, cpp) {
                    let i =
                        if per_row { (y - area.top as usize) / area.row_pitch as usize } else { (x - area.left as usize) / area.col_pitch as usize };
                    if let Some(d) = deltas.get(i) {
                        buf[(y * w + x) * cpp + p] += d * scale;
                    }
                }
            }
            Opcode::ScalePerRow { area, scales } | Opcode::ScalePerColumn { area, scales } => {
                let per_row = matches!(op, Opcode::ScalePerRow { .. });
                for (x, y, p) in area_iter(area, w, h, cpp) {
                    let i =
                        if per_row { (y - area.top as usize) / area.row_pitch as usize } else { (x - area.left as usize) / area.col_pitch as usize };
                    if let Some(s) = scales.get(i) {
                        buf[(y * w + x) * cpp + p] *= s;
                    }
                }
            }
            Opcode::FixVignetteRadial { k, center } => {
                let (cx, cy) = (center[0] * w as f64, center[1] * h as f64);
                let m = max_radius(cx, cy, w as f64, h as f64);
                for y in 0..h {
                    for x in 0..w {
                        let r2 = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)) / (m * m);
                        let g = 1.0 + r2 * (k[0] + r2 * (k[1] + r2 * (k[2] + r2 * (k[3] + r2 * k[4]))));
                        for p in 0..cpp {
                            buf[(y * w + x) * cpp + p] *= g as f32;
                        }
                    }
                }
            }
            Opcode::FixBadPixelsConstant { constant, .. } => {
                let c = (*constant as f64 / 65535.0 * scale as f64) as f32;
                let bad: Vec<(usize, usize)> =
                    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| cpp == 1 && buf[y * w + x] == c).take(MAX_BAD_PIXELS).collect();
                fix_pixels(buf, w, h, cpp, cfa, &bad);
            }
            Opcode::FixBadPixelsList { points, rects, .. } => {
                let mut bad: Vec<(usize, usize)> =
                    points.iter().map(|&(r, c)| (c as usize, r as usize)).filter(|&(x, y)| x < w && y < h).take(MAX_BAD_PIXELS).collect();
                'rects: for r in rects {
                    let [t, l, b, rr] = r.map(|v| v as usize);
                    for y in t..b.min(h) {
                        for x in l..rr.min(w) {
                            if bad.len() >= MAX_BAD_PIXELS {
                                break 'rects;
                            }
                            bad.push((x, y));
                        }
                    }
                }
                fix_pixels(buf, w, h, cpp, cfa, &bad);
            }
            _ => {}
        }
    }
}

/// Replace listed pixels with the mean of the nearest same-colour neighbours that are not themselves bad.
fn fix_pixels(buf: &mut [f32], w: usize, h: usize, cpp: usize, cfa: Option<&Cfa>, bad: &[(usize, usize)]) {
    if bad.is_empty() {
        return;
    }
    let mut is_bad = std::collections::HashSet::with_capacity(bad.len());
    for &b in bad {
        is_bad.insert(b);
    }
    let step: isize = match cfa {
        Some(c) if c.width == 2 && c.height == 2 => 2,
        Some(c) => c.width.max(c.height) as isize,
        None => 1,
    };
    for &(x, y) in bad {
        for p in 0..cpp {
            let mut sum = 0.0;
            let mut n = 0;
            for (dx, dy) in [(-step, 0), (step, 0), (0, -step), (0, step), (-step, -step), (step, step), (-step, step), (step, -step)] {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize || is_bad.contains(&(nx as usize, ny as usize)) {
                    continue;
                }
                sum += buf[(ny as usize * w + nx as usize) * cpp + p];
                n += 1;
            }
            if n > 0 {
                buf[(y * w + x) * cpp + p] = sum / n as f32;
            }
        }
    }
}

/// Most full-image passes of one `OpcodeList3`: every warp record is a full-image allocation and resample
/// pass and every bad-pixel record a full flatten-and-rebuild, and a hostile record may repeat either
/// thousands of times. Real DNGs carry at most a couple; passes past the cap are skipped.
const MAX_LIST3_PASSES: usize = 8;

/// Apply `OpcodeList3` (on the demosaiced RGB image, values normalised to [0, 1]).
pub fn apply_list3(list: &[Opcode], img: &mut Rgb32f) {
    if list.is_empty() {
        return;
    }
    let (w, h) = (img.width, img.height);
    let mut passes = 0usize;
    for op in applied_ops(list) {
        match op {
            Opcode::WarpRectilinear { planes, center } => {
                if passes < MAX_LIST3_PASSES {
                    passes += 1;
                    *img = warp_rectilinear(img, planes, *center);
                }
            }
            Opcode::WarpRectilinear2 { planes, center, reciprocal, .. } => {
                if passes < MAX_LIST3_PASSES {
                    passes += 1;
                    *img = warp_rectilinear2(img, planes, *center, *reciprocal);
                }
            }
            Opcode::WarpFisheye { .. } | Opcode::TrimBounds { .. } | Opcode::Unknown { .. } => {}
            other => {
                if passes >= MAX_LIST3_PASSES {
                    continue;
                }
                passes += 1;
                let mut flat: Vec<f32> = img.data.iter().flat_map(|p| *p).collect();
                apply_list(std::slice::from_ref(other), &mut flat, w, h, 3, None, 1.0);
                for (d, c) in img.data.iter_mut().zip(flat.as_chunks::<3>().0) {
                    *d = [c[0], c[1], c[2]];
                }
            }
        }
    }
}

/// DNG `WarpRectilinear`: for each output pixel, the source position is the radial + tangential model around the
/// optical centre, with coordinates normalised by the distance to the farthest corner.
pub fn warp_rectilinear(img: &Rgb32f, planes: &[[f64; 6]], center: [f64; 2]) -> Rgb32f {
    let (w, h) = (img.width, img.height);
    let (cx, cy) = (center[0] * (w as f64 - 1.0), center[1] * (h as f64 - 1.0));
    let m = max_radius(cx, cy, w as f64 - 1.0, h as f64 - 1.0);
    let mut out = Rgb32f::new(w, h);
    let coef = |p: usize| planes[p.min(planes.len() - 1)];
    crate::raster::par_rows(&mut out.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            for (c, v) in px.iter_mut().enumerate() {
                let [kr0, kr1, kr2, kr3, kt0, kt1] = coef(c);
                let dx = (x as f64 - cx) / m;
                let dy = (y as f64 - cy) / m;
                let r2 = dx * dx + dy * dy;
                let f = kr0 + r2 * (kr1 + r2 * (kr2 + r2 * kr3));
                let sx = dx * f + kt0 * 2.0 * dx * dy + kt1 * (r2 + 2.0 * dx * dx);
                let sy = dy * f + kt1 * 2.0 * dx * dy + kt0 * (r2 + 2.0 * dy * dy);
                let (fx, fy) = (cx + m * sx, cy + m * sy);
                *v = img.sample_bilinear(fx as f32 + 0.5, fy as f32 + 0.5)[c];
            }
        }
    });
    out
}

/// DNG 1.6 `WarpRectilinear2`: the `WarpRectilinear` geometry (centre and normalisation as there), with the radial
/// factor of [`wr2_factor`] in place of the even polynomial in `r²`. A pixel whose factor is not finite stays put.
pub fn warp_rectilinear2(img: &Rgb32f, planes: &[[f64; WR2_LEN]], center: [f64; 2], reciprocal: bool) -> Rgb32f {
    let (w, h) = (img.width, img.height);
    let Some(last) = planes.len().checked_sub(1) else { return img.clone() };
    let mut out = Rgb32f::new(w, h);
    let (cx, cy) = (center[0] * (w as f64 - 1.0), center[1] * (h as f64 - 1.0));
    let m = max_radius(cx, cy, w as f64 - 1.0, h as f64 - 1.0);
    crate::raster::par_rows(&mut out.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            for (c, v) in px.iter_mut().enumerate() {
                let Some(k) = planes.get(c.min(last)) else { continue };
                let dx = (x as f64 - cx) / m;
                let dy = (y as f64 - cy) / m;
                let r2 = dx * dx + dy * dy;
                let (sx, sy) = match wr2_factor(k, reciprocal, r2.sqrt()) {
                    Some(f) => {
                        (dx * f + k[15] * 2.0 * dx * dy + k[16] * (r2 + 2.0 * dx * dx), dy * f + k[16] * 2.0 * dx * dy + k[15] * (r2 + 2.0 * dy * dy))
                    }
                    None => (dx, dy),
                };
                let (fx, fy) = (cx + m * sx, cy + m * sy);
                *v = img.sample_bilinear(fx as f32 + 0.5, fy as f32 + 0.5)[c];
            }
        }
    });
    out
}

#[cfg(test)]
pub(crate) mod tests;
