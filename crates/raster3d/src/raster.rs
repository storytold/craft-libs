//! The triangle rasterizer.
//!
//! Triangles are clipped against the depth range (`0 ≤ z ≤ w`) and a guard band in x and y,
//! set up in screen space and scanned per band of rows in parallel with edge
//! functions (pixel centers, top-left fill rule) and perspective-correct barycentrics. Opaque
//! triangles resolve visibility first (reversed depth: larger ndc z is nearer) and each pixel is
//! shaded once; transparent triangles (already sorted back to front) are then shaded per
//! fragment and blended over, depth-tested against the opaque surfaces.

use rayon::prelude::*;

use crate::scene::{Scene, Vertex, validate, validate_passes, validate_shadow_refs};
use crate::shade::{Frag, shade};
use crate::{Limits, RenderError};

/// Half-width of the clip guard band in units of clip w: a triangle reaching past
/// `|x| > GUARD_BAND × w` or `|y| > GUARD_BAND × w` is clipped there before the perspective
/// divide, so screen coordinates stay within about 512 viewports of the raster. A triangle
/// inside the band is not clipped in x or y.
pub const GUARD_BAND: f64 = 1024.0;

/// A rendered target at raster size.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub width: u32,
    pub height: u32,
    /// Premultiplied linear RGBA, row-major from the top row.
    pub color: Vec<[f32; 4]>,
    /// Camera-space depth of the nearest opaque surface (∞ where empty).
    pub depth: Vec<f32>,
}

/// A clipped triangle in screen space; `w` holds, per corner, its weights over the original
/// triangle's three vertices.
#[derive(Clone, Copy)]
struct STri {
    p: [[f64; 2]; 3],
    z: [f32; 3],
    inv_w: [f32; 3],
    /// The corners' 1/w in `f64` when one of them does not fit `f32` exactly (the scaled
    /// reciprocals of the wide projection path), so a ratio below `f32`'s range is not dropped.
    inv_w_wide: Option<[f64; 3]>,
    w: [[f32; 3]; 3],
    tri: u32,
    front: bool,
    bbox: [i32; 4],
}

/// A polygon corner: clip coordinates and weights over the original triangle's vertices.
type Corner = ([f32; 4], [f32; 3]);

/// A screen-space corner: position, ndc z, 1/w and weights.
type ScreenCorner = ([f64; 2], f32, f64, [f32; 3]);

#[inline]
fn mat_mul(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 4] {
    let mut o = [0.0; 4];
    for r in 0..4 {
        o[r] = m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3];
    }
    o
}

/// The signed distance of a clip-space point inside clip plane `plane`: 0 and 1 are the depth
/// range `0 ≤ z ≤ w`, 2 to 5 the guard band `|x| ≤ GUARD_BAND × w` and `|y| ≤ GUARD_BAND × w`.
fn plane_distance(c: &[f64; 4], plane: usize) -> f64 {
    let g = GUARD_BAND * c[3];
    match plane {
        0 => c[2],
        1 => c[3] - c[2],
        2 => g + c[0],
        3 => g - c[0],
        4 => g + c[1],
        _ => g - c[1],
    }
}

/// The depth-range planes followed by the guard-band planes.
const ALL_PLANES: [usize; 6] = [0, 1, 2, 3, 4, 5];

/// The guard-band planes.
const GUARD_PLANES: [usize; 4] = [2, 3, 4, 5];

/// Clip a polygon against the guard band. A polygon inside the band is returned unchanged;
/// any other is clipped in `f64`.
fn clip_guard(poly: Vec<Corner>) -> Vec<Corner> {
    if poly.iter().all(|(c, _)| GUARD_PLANES.iter().all(|&plane| plane_distance(&c.map(f64::from), plane) >= 0.0)) {
        return poly;
    }
    clip_f64(&poly, &GUARD_PLANES)
}

/// The share of an edge's length within which an `f32` crossing is interpolated from the inside
/// corner instead of from the edge's first corner.
const NEAR_CORNER: f32 = 1.0 / 4096.0;

/// Where the edge from a corner at plane distance `da` to one at `db` crosses the plane, for an
/// `f32` clip: `(false, t)` for `a + (b − a) × t`, or `(true, t)` for `b + (a − b) × t` when the
/// crossing lies within [`NEAR_CORNER`] of the edge's length from `b` and `da` is finite. A
/// distance that overflowed keeps the first form, whose corner is then not finite. A difference
/// of finite distances that overflows `f32` is taken in `f64`.
pub(crate) fn f32_crossing(da: f32, db: f32) -> (bool, f32) {
    let share = |p: f32, q: f32| {
        let d = p - q;
        if d.is_finite() || !p.is_finite() || !q.is_finite() { p / d } else { (f64::from(p) / (f64::from(p) - f64::from(q))) as f32 }
    };
    if da.is_finite() && db.abs() < da.abs() * NEAR_CORNER { (true, share(db, da)) } else { (false, share(da, db)) }
}

/// Clip a polygon against `planes` (see [`plane_distance`]), interpolating in `f64` from the
/// corner nearer the plane.
fn clip_f64(poly: &[Corner], planes: &[usize]) -> Vec<Corner> {
    let mut p: Vec<([f64; 4], [f64; 3])> = poly.iter().map(|(c, w)| (c.map(f64::from), w.map(f64::from))).collect();
    for &plane in planes {
        if p.is_empty() {
            break;
        }
        let d = |c: &[f64; 4]| plane_distance(c, plane);
        let mut out = Vec::with_capacity(p.len() + 2);
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            let (da, db) = (d(&a.0), d(&b.0));
            if da >= 0.0 {
                out.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let (p, q, t) = if da.abs() <= db.abs() { (a, b, da / (da - db)) } else { (b, a, db / (db - da)) };
                let c: [f64; 4] = std::array::from_fn(|k| p.0[k] + (q.0[k] - p.0[k]) * t);
                let w: [f64; 3] = std::array::from_fn(|k| p.1[k] + (q.1[k] - p.1[k]) * t);
                out.push((c, w));
            }
        }
        p = out;
    }
    p.iter().map(|(c, w)| (c.map(|v| v as f32), w.map(|v| v as f32))).collect()
}

/// Clip a polygon (clip coords + original weights) against `0 ≤ z ≤ w`.
fn clip_poly(poly: Vec<Corner>) -> Vec<Corner> {
    let mut p = poly;
    for plane in 0..2 {
        if p.is_empty() {
            break;
        }
        let d = |c: &[f32; 4]| if plane == 0 { c[2] } else { c[3] - c[2] };
        let mut out = Vec::with_capacity(p.len() + 2);
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            let (da, db) = (d(&a.0), d(&b.0));
            if da >= 0.0 {
                out.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let (from_b, t) = f32_crossing(da, db);
                let (p, q) = if from_b { (b, a) } else { (a, b) };
                let mut c = [0.0; 4];
                let mut w = [0.0; 3];
                for k in 0..4 {
                    c[k] = p.0[k] + (q.0[k] - p.0[k]) * t;
                }
                for k in 0..3 {
                    w[k] = p.1[k] + (q.1[k] - p.1[k]) * t;
                }
                out.push((c, w));
            }
        }
        p = out;
    }
    p
}

/// The clipped polygon of triangle `t` (the index of its first index) projected to the screen,
/// or `None` when nothing of it is left or a corner is not finite.
fn project(s: &Scene, clip: &[[f32; 4]], t: usize) -> Option<Vec<ScreenCorner>> {
    let (w, h) = (s.width as f64, s.height as f64);
    let ids = [s.indices[t] as usize, s.indices[t + 1] as usize, s.indices[t + 2] as usize];
    let tri = vec![(clip[ids[0]], [1.0, 0.0, 0.0]), (clip[ids[1]], [0.0, 1.0, 0.0]), (clip[ids[2]], [0.0, 0.0, 1.0])];
    let finite = |p: &[Corner]| p.iter().all(|(c, wt)| c.iter().chain(wt).all(|v| v.is_finite()));
    // The depth range in f32; when that overflows between far-apart corners, every plane in f64.
    let poly = clip_poly(tri.clone());
    let poly = if finite(&poly) { clip_guard(poly) } else { clip_f64(&tri, &ALL_PLANES) };
    if poly.len() < 3 || !finite(&poly) {
        return None;
    }
    let scr: Vec<ScreenCorner> = poly
        .iter()
        .map(|(c, wt)| {
            let iw = 1.0 / c[3];
            let (nx, ny) = (c[0] * iw, c[1] * iw);
            ([(nx as f64 + 1.0) * 0.5 * w, (1.0 - ny as f64) * 0.5 * h], c[2] * iw, f64::from(iw), *wt)
        })
        .collect();
    let ok = |s: &ScreenCorner| s.0.iter().all(|v| v.is_finite()) && s.1.is_finite() && s.2.is_finite();
    if scr.iter().all(ok) {
        return Some(scr);
    }
    // A tiny w overflows 1/w in f32: project in f64, with 1/w scaled by a common factor so the
    // largest is 1 (the interpolation weights use only their ratios). The ratios stay in f64
    // until the weights are normalized, since a small one can be below f32's range.
    let iw: Vec<f64> = poly.iter().map(|(c, _)| 1.0 / f64::from(c[3])).collect();
    let top = iw.iter().fold(0.0f64, |a, &b| a.max(b));
    let scr: Vec<ScreenCorner> = poly
        .iter()
        .zip(&iw)
        .map(|((c, wt), &iw)| {
            let (nx, ny) = (f64::from(c[0]) * iw, f64::from(c[1]) * iw);
            ([(nx + 1.0) * 0.5 * w, (1.0 - ny) * 0.5 * h], (f64::from(c[2]) * iw) as f32, iw / top, *wt)
        })
        .collect();
    scr.iter().all(ok).then_some(scr)
}

/// The screen triangles of a projected polygon, fanned from its first corner and clamped to the
/// viewport, appended to `out`.
fn fan(s: &Scene, scr: &[ScreenCorner], tri: u32, out: &mut Vec<STri>) {
    // Facing from the polygon's screen-space winding (y down: negative = front).
    let mut area = 0.0;
    for i in 0..scr.len() {
        let (a, b) = (scr[i].0, scr[(i + 1) % scr.len()].0);
        area += a[0] * b[1] - b[0] * a[1];
    }
    if area == 0.0 || !area.is_finite() {
        return;
    }
    let front = area < 0.0;
    for k in 1..scr.len() - 1 {
        let c = [scr[0], scr[k], scr[k + 1]];
        let mut x0 = f64::INFINITY;
        let mut y0 = f64::INFINITY;
        let mut x1 = f64::NEG_INFINITY;
        let mut y1 = f64::NEG_INFINITY;
        for v in &c {
            x0 = x0.min(v.0[0]);
            y0 = y0.min(v.0[1]);
            x1 = x1.max(v.0[0]);
            y1 = y1.max(v.0[1]);
        }
        // Clamped on both sides to the viewport before narrowing to i32.
        let (w, h) = (i64::from(s.width.min(i32::MAX as u32)), i64::from(s.height.min(i32::MAX as u32)));
        let bbox = [
            (x0.floor() as i64).clamp(0, w) as i32,
            (y0.floor() as i64).clamp(0, h) as i32,
            (x1.ceil() as i64).clamp(0, w) as i32,
            (y1.ceil() as i64).clamp(0, h) as i32,
        ];
        if bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
            continue;
        }
        out.push(STri {
            p: [c[0].0, c[1].0, c[2].0],
            z: [c[0].1, c[1].1, c[2].1],
            inv_w: [c[0].2 as f32, c[1].2 as f32, c[2].2 as f32],
            inv_w_wide: c.iter().any(|v| f64::from(v.2 as f32) != v.2).then_some([c[0].2, c[1].2, c[2].2]),
            w: [c[0].3, c[1].3, c[2].3],
            tri,
            front,
            bbox,
        });
    }
}

/// Set up the screen triangles of index range `range`, adding each one's coverage tests (its
/// bounding-box pixels, plus one) to `work`. Stops with [`RenderError::TooMuchWork`] as soon as
/// `work` passes `limit`, before more triangles are kept.
fn setup(s: &Scene, clip: &[[f32; 4]], range: std::ops::Range<usize>, work: &mut u64, limit: u64) -> Result<Vec<STri>, RenderError> {
    setup_with_reserve(s, clip, range, work, limit, reserve_screen_triangles)
}

fn reserve_screen_triangles(out: &mut Vec<STri>, additional: usize) -> Result<(), RenderError> {
    out.try_reserve(additional).map_err(|_| RenderError::OutOfMemory)
}

fn setup_with_reserve(
    s: &Scene,
    clip: &[[f32; 4]],
    range: std::ops::Range<usize>,
    work: &mut u64,
    limit: u64,
    mut reserve: impl FnMut(&mut Vec<STri>, usize) -> Result<(), RenderError>,
) -> Result<Vec<STri>, RenderError> {
    let mut out = vec![];
    for t in range.step_by(3) {
        let mat = s.vertices[s.indices[t] as usize].material as usize;
        if !s.materials.get(mat).is_some_and(|m| m.visible) {
            continue;
        }
        if let Some(scr) = project(s, clip, t) {
            let first = out.len();
            reserve(&mut out, scr.len())?;
            fan(s, &scr, (t / 3) as u32, &mut out);
            for st in &out[first..] {
                let area = (st.bbox[2] - st.bbox[0]) as u64 * (st.bbox[3] - st.bbox[1]) as u64;
                *work = work.saturating_add(area + 1);
            }
            if *work > limit {
                return Err(RenderError::TooMuchWork { needed: *work, limit });
            }
        }
    }
    Ok(out)
}

/// Coverage of a pixel center: (ndc z, perspective-correct weights over the screen corners).
#[inline]
fn cover(t: &STri, px: f64, py: f64) -> Option<(f32, [f32; 3])> {
    let [a, b, c] = t.p;
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    // Orient so the area is positive.
    let (b, c, swap) = if area < 0.0 { (c, b, true) } else { (b, c, false) };
    let area = area.abs();
    let edge = |p: [f64; 2], q: [f64; 2]| -> f64 { (q[0] - p[0]) * (py - p[1]) - (q[1] - p[1]) * (px - p[0]) };
    let top_left = |p: [f64; 2], q: [f64; 2]| -> bool {
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        dy < 0.0 || (dy == 0.0 && dx > 0.0)
    };
    let e0 = edge(b, c); // opposite a
    let e1 = edge(c, a); // opposite b
    let e2 = edge(a, b); // opposite c
    let inside = |e: f64, p: [f64; 2], q: [f64; 2]| e > 0.0 || (e == 0.0 && top_left(p, q));
    if !(inside(e0, b, c) && inside(e1, c, a) && inside(e2, a, b)) {
        return None;
    }
    let (l0, l1, l2) = ((e0 / area) as f32, (e1 / area) as f32, (e2 / area) as f32);
    // Back to the stored corner order.
    let l = if swap { [l0, l2, l1] } else { [l0, l1, l2] };
    let z = l[0] * t.z[0] + l[1] * t.z[1] + l[2] * t.z[2];
    if let Some(iw) = t.inv_w_wide {
        let (m0, m1, m2) = (e0 / area, e1 / area, e2 / area);
        let m = if swap { [m0, m2, m1] } else { [m0, m1, m2] };
        let q = [m[0] * iw[0], m[1] * iw[1], m[2] * iw[2]];
        let sum = q[0] + q[1] + q[2];
        if sum <= 0.0 {
            return None;
        }
        return Some((z, q.map(|v| (v / sum) as f32)));
    }
    let q = [l[0] * t.inv_w[0], l[1] * t.inv_w[1], l[2] * t.inv_w[2]];
    let sum = q[0] + q[1] + q[2];
    if sum <= 0.0 {
        return None;
    }
    Some((z, [q[0] / sum, q[1] / sum, q[2] / sum]))
}

/// Interpolate the original triangle's vertex attributes at screen-corner weights `l`.
fn frag(s: &Scene, t: &STri, l: [f32; 3]) -> Frag {
    let base = t.tri as usize * 3;
    let v: [&Vertex; 3] = [0, 1, 2].map(|k| &s.vertices[s.indices[base + k] as usize]);
    // Weights over the original vertices.
    let mut wv = [0.0f32; 3];
    for c in 0..3 {
        for k in 0..3 {
            wv[k] += l[c] * t.w[c][k];
        }
    }
    let mut f = Frag { pos: [0.0; 3], normal: [0.0; 3], uv: [0.0; 2], tangent: [0.0; 4], material: v[0].material, front: t.front };
    for k in 0..3 {
        let x = wv[k];
        for i in 0..3 {
            f.pos[i] += v[k].pos[i] * x;
            f.normal[i] += v[k].normal[i] * x;
        }
        for i in 0..2 {
            f.uv[i] += v[k].uv[i] * x;
        }
        for i in 0..4 {
            f.tangent[i] += v[k].tangent[i] * x;
        }
    }
    f.tangent[3] = v[0].tangent[3];
    f
}

const BAND: i32 = 16;

/// The clip coordinates of every vertex, or the first vertex whose clip position is not finite.
fn clip_positions(s: &Scene) -> Result<Vec<[f32; 4]>, RenderError> {
    s.vertices
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let c = mat_mul(&s.clip, v.pos);
            if c.iter().all(|x| x.is_finite()) { Ok(c) } else { Err(RenderError::BadClipPosition(i)) }
        })
        .collect()
}

/// The screen triangles of each band of `BAND` rows, by index into `ts`.
fn bin(ts: &[STri], bands: i32) -> Vec<Vec<u32>> {
    let mut b = vec![vec![]; bands.max(0) as usize];
    for (i, t) in ts.iter().enumerate() {
        for band in (t.bbox[1] / BAND)..((t.bbox[3] + BAND - 1) / BAND).min(bands) {
            b[band as usize].push(i as u32);
        }
    }
    b
}

/// One band of rows being rendered: its first row, its row count and the raster width.
struct Band<'a> {
    s: &'a Scene,
    y0: i32,
    rows: usize,
    w: usize,
}

/// The screen triangles of one pass and the indices of those in the current band.
#[derive(Clone, Copy)]
struct Pass<'a> {
    tris: &'a [STri],
    bins: &'a [u32],
}

/// Per pixel: (screen triangle, ndc z, weights); `u32::MAX` and [`EMPTY`] where no triangle
/// covers it.
type Visibility = Vec<(u32, f32, [f32; 3])>;

/// The visibility depth of a pixel with no opaque surface: below the far plane's 0, so a
/// fragment on the far plane is drawn.
const EMPTY: f32 = -1.0;

/// Whether ndc depth `z` is inside the depth range and nearer than `than`.
#[inline]
fn nearer(z: f32, than: f32) -> bool {
    z > than && (0.0..=1.0).contains(&z)
}

impl Band<'_> {
    /// Camera-space depth of a world point, NaN when it overflows `f32` (infinity marks an
    /// empty pixel, and [`try_render`] rejects NaN).
    fn cam_z(&self, p: [f32; 3]) -> f32 {
        let view = self.s.view;
        let z = view[2][0] * p[0] + view[2][1] * p[1] + view[2][2] * p[2] + view[2][3];
        if z.is_finite() { z } else { f32::NAN }
    }

    /// The nearest opaque triangle covering each pixel of the band.
    fn visibility(&self, opaque: Pass) -> Visibility {
        let (w, y0, rows) = (self.w, self.y0, self.rows);
        let mut vis: Visibility = vec![(u32::MAX, EMPTY, [0.0; 3]); rows * w];
        for &ti in opaque.bins {
            let t = &opaque.tris[ti as usize];
            for y in t.bbox[1].max(y0)..t.bbox[3].min(y0 + rows as i32) {
                let r = (y - y0) as usize * w;
                for x in t.bbox[0]..t.bbox[2] {
                    if let Some((z, l)) = cover(t, x as f64 + 0.5, y as f64 + 0.5) {
                        let v = &mut vis[r + x as usize];
                        if nearer(z, v.1) {
                            *v = (ti, z, l);
                        }
                    }
                }
            }
        }
        vis
    }

    /// Shade each pixel's visible opaque fragment. A discarded fragment (alpha test, back face)
    /// gives way to the nearest surviving fragment under the pixel, and the pixel's visibility
    /// depth becomes that fragment's ([`EMPTY`] when none survives).
    fn shade_opaque(&self, opaque: Pass, vis: &mut Visibility, crow: &mut [[f32; 4]], drow: &mut [f32]) {
        let s = self.s;
        let mut discarded = vec![false; self.rows * self.w];
        for i in 0..self.rows * self.w {
            let (ti, _, l) = vis[i];
            if ti == u32::MAX {
                continue;
            }
            let f = frag(s, &opaque.tris[ti as usize], l);
            match shade(s, &f) {
                Some(c) => {
                    crow[i] = c;
                    drow[i] = self.cam_z(f.pos);
                }
                None => discarded[i] = true,
            }
        }
        if !discarded.contains(&true) {
            return;
        }
        for (i, best) in self.fallback(opaque, vis, &discarded).into_iter().enumerate() {
            if !discarded[i] {
                continue;
            }
            match best {
                Some((z, c, d)) => {
                    crow[i] = c;
                    drow[i] = d;
                    vis[i].1 = z;
                }
                None => vis[i].1 = EMPTY,
            }
        }
    }

    /// For each discarded pixel of the band, the nearest surviving opaque fragment of a screen
    /// triangle other than the pixel's visible one, inside the depth range as in
    /// [`Band::visibility`]: (ndc z, color, camera depth). Triangles are
    /// visited in index order over their bounding boxes, so this pass tests no more pixels than
    /// [`Band::visibility`], and among fragments at equal depth the first triangle's is kept.
    #[allow(clippy::type_complexity)]
    fn fallback(&self, opaque: Pass, vis: &Visibility, discarded: &[bool]) -> Vec<Option<(f32, [f32; 4], f32)>> {
        let (s, w, y0, rows) = (self.s, self.w, self.y0, self.rows);
        let mut best: Vec<Option<(f32, [f32; 4], f32)>> = vec![None; rows * w];
        for &tj in opaque.bins {
            let t2 = &opaque.tris[tj as usize];
            for y in t2.bbox[1].max(y0)..t2.bbox[3].min(y0 + rows as i32) {
                let r = (y - y0) as usize * w;
                for x in t2.bbox[0]..t2.bbox[2] {
                    let i = r + x as usize;
                    if !discarded[i] || vis[i].0 == tj {
                        continue;
                    }
                    if let Some((z, l2)) = cover(t2, x as f64 + 0.5, y as f64 + 0.5)
                        && nearer(z, best[i].map_or(EMPTY, |b| b.0))
                    {
                        let f2 = frag(s, t2, l2);
                        if let Some(c) = shade(s, &f2) {
                            best[i] = Some((z, c, self.cam_z(f2.pos)));
                        }
                    }
                }
            }
        }
        best
    }

    /// Transparent fragments in order, blended over the opaque result where they are nearer.
    fn blend_transparent(&self, trans: Pass, vis: &Visibility, crow: &mut [[f32; 4]]) {
        let (s, w, y0, rows) = (self.s, self.w, self.y0, self.rows);
        for &ti in trans.bins {
            let t = &trans.tris[ti as usize];
            for y in t.bbox[1].max(y0)..t.bbox[3].min(y0 + rows as i32) {
                let r = (y - y0) as usize * w;
                for x in t.bbox[0]..t.bbox[2] {
                    let i = r + x as usize;
                    if let Some((z, l)) = cover(t, x as f64 + 0.5, y as f64 + 0.5)
                        && nearer(z, vis[i].1)
                        && let Some(c) = shade(s, &frag(s, t, l))
                    {
                        let d = crow[i];
                        let k = 1.0 - c[3];
                        crow[i] = [c[0] + d[0] * k, c[1] + d[1] * k, c[2] + d[2] * k, c[3] + d[3] * k];
                    }
                }
            }
        }
    }
}

/// Rasterize and shade set-up screen triangles into a target of `pixels` pixels.
fn raster(s: &Scene, opaque: &[STri], trans: &[STri], pixels: usize) -> Target {
    let (w, h) = (s.width as usize, s.height as usize);
    let bands = (h as i32 + BAND - 1) / BAND;
    let ob = bin(opaque, bands);
    let tb = bin(trans, bands);
    let mut color = vec![[0.0f32; 4]; pixels];
    let mut depth = vec![f32::INFINITY; pixels];
    color.par_chunks_mut(w * BAND as usize).zip(depth.par_chunks_mut(w * BAND as usize)).enumerate().for_each(|(band, (crow, drow))| {
        let b = Band { s, y0: band as i32 * BAND, rows: crow.len() / w, w };
        let (op, tp) = (Pass { tris: opaque, bins: &ob[band] }, Pass { tris: trans, bins: &tb[band] });
        let mut vis = b.visibility(op);
        b.shade_opaque(op, &mut vis, crow, drow);
        b.blend_transparent(tp, &vis, crow);
    });
    Target { width: s.width, height: s.height, color, depth }
}

/// Adds the tests of the search under discarded opaque pixels ([`Band::fallback`]) to `work`,
/// which holds the opaque pass's tests: the same count again when an opaque triangle's material
/// can discard a fragment (single-sided, or an alpha mode other than 0), since the search visits
/// each opaque triangle's bounding box once more. Stops with [`RenderError::TooMuchWork`] when
/// `work` passes `limit`.
fn charge_fallback(s: &Scene, opaque: &[STri], work: &mut u64, limit: u64) -> Result<(), RenderError> {
    let discards = |t: &STri| {
        let first = s.indices.get(t.tri as usize * 3).and_then(|&i| s.vertices.get(i as usize));
        first.and_then(|v| s.materials.get(v.material as usize)).is_some_and(|m| !m.double_sided || m.alpha_mode != 0)
    };
    if opaque.iter().any(discards) {
        *work = work.saturating_add(*work);
        if *work > limit {
            return Err(RenderError::TooMuchWork { needed: *work, limit });
        }
    }
    Ok(())
}

/// Rasterize and shade the scene at its raster size.
///
/// The scene is checked first (see [`Limits`]); an empty scene gives a transparent target with
/// infinite depth. A triangle whose clipped polygon has zero area draws nothing.
/// Screen-triangle reservation failures return [`RenderError::OutOfMemory`].
pub fn try_render(s: &Scene, limits: &Limits) -> Result<Target, RenderError> {
    let pixels = validate(s, limits)?;
    validate_passes(s)?;
    validate_shadow_refs(s)?;
    let clip = clip_positions(s)?;
    let mut work = 0;
    let opaque = setup(s, &clip, 0..s.opaque_count as usize, &mut work, limits.max_work)?;
    charge_fallback(s, &opaque, &mut work, limits.max_work)?;
    let trans = setup(s, &clip, s.opaque_count as usize..s.indices.len(), &mut work, limits.max_work)?;
    let t = raster(s, &opaque, &trans, pixels);
    // Finite inputs near the top of the f32 range can still overflow in shading.
    if let Some(i) = t.color.iter().zip(&t.depth).position(|(c, d)| !c.iter().all(|v| v.is_finite()) || d.is_nan()) {
        return Err(RenderError::BadPixel(i));
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::{f32_crossing, reserve_screen_triangles, setup, setup_with_reserve};
    use crate::{Material, RenderError, Scene, Vertex};

    #[test]
    fn screen_triangle_reservation_failure_is_returned() {
        let scene = Scene {
            width: 1,
            height: 1,
            vertices: vec![Vertex::default(); 3],
            materials: vec![Material { double_sided: true, ..Material::default() }],
            indices: vec![0, 1, 2],
            opaque_count: 3,
            ..Scene::default()
        };
        let clip = [[-1.0, -1.0, 0.5, 1.0], [1.0, -1.0, 0.5, 1.0], [0.0, 1.0, 0.5, 1.0]];
        let mut work = 0;
        let failed = setup_with_reserve(&scene, &clip, 0..3, &mut work, u64::MAX, |out, _| reserve_screen_triangles(out, usize::MAX));
        assert_eq!(failed.err(), Some(RenderError::OutOfMemory));
        assert_eq!(work, 0);
        let normal = setup(&scene, &clip, 0..3, &mut work, u64::MAX);
        assert!(normal.is_ok_and(|triangles| !triangles.is_empty()));
        assert!(work > 0);
    }

    #[test]
    fn a_crossing_whose_distance_difference_overflows_is_taken_in_f64() {
        // −3e38 − 1e38 overflows f32; the plane lies three quarters of the way along the edge.
        assert_eq!(f32_crossing(-3e38, 1e38), (false, 0.75));
        assert_eq!(f32_crossing(1e38, -3e38), (false, 0.25));
    }
}
