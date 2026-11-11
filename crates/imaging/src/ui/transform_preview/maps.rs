//! [`PreviewMap`]: where a transform preview's source goes, and the mesh it is drawn with (task
//! TR-T10).

use craft_behavior::geometry::Homography;
use craft_behavior::kurbo::{self, Point, Rect};

use super::PreviewQuality;
use crate::ui::warp::WarpMesh;
use crate::ui::warp::pin::PinMeshView;

/// Where the preview's source goes, in document units. The texture covers the source frame; each
/// map says where every point of it lands.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum PreviewMap<'a> {
    /// Free Transform: the source frame's corners onto a box's corners (clockwise from the top
    /// left), through the homography; a `QUAD_CELLS` × `QUAD_CELLS` mesh keeps perspective
    /// straight, cells with a corner past the horizon left out.
    Quad([Point; 4]),
    /// Warp: the source frame's unit square through a warp mesh's surface, sampled finer where the
    /// warp pulls hard (about every `SEGMENT_PX` screen points, `MIN_PER_CELL` to `MAX_PER_CELL`
    /// per patch, at most `MAX_PER_AXIS` per axis). The displacement order measures from `source`,
    /// the unwarped frame.
    #[non_exhaustive]
    Mesh {
        /// The warp mesh.
        mesh: &'a WarpMesh,
        /// The source frame.
        source: Rect,
    },
    /// Puppet Warp: the solver's triangles, each vertex at its deformed position and its texel at
    /// its rest position within `source` (the layer's bounds), drawn in the solver's order.
    #[non_exhaustive]
    Pins {
        /// The solver's mesh.
        mesh: PinMeshView<'a>,
        /// The source frame.
        source: Rect,
    },
    /// Perspective Warp and other engine maps: a `cells` × `cells` grid over `source` pushed
    /// through the app's function (PhotoCraft's piecewise homography, LC20).
    #[non_exhaustive]
    Function {
        /// The source frame.
        source: Rect,
        /// Cells per side.
        cells: usize,
        /// The app's map.
        map: &'a dyn Fn(Point) -> Point,
    },
}

impl std::fmt::Debug for PreviewMap<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewMap::Quad(quad) => f.debug_tuple("Quad").field(quad).finish(),
            PreviewMap::Mesh { mesh, source } => f.debug_struct("Mesh").field("mesh", mesh).field("source", source).finish(),
            PreviewMap::Pins { mesh, source } => f.debug_struct("Pins").field("mesh", mesh).field("source", source).finish(),
            PreviewMap::Function { source, cells, .. } => {
                f.debug_struct("Function").field("source", source).field("cells", cells).finish_non_exhaustive()
            }
        }
    }
}

impl<'a> PreviewMap<'a> {
    /// A box: the source frame's corners onto `quad`.
    pub fn quad(quad: [Point; 4]) -> Self {
        PreviewMap::Quad(quad)
    }

    /// A warp mesh over `source`, the source frame.
    pub fn mesh(mesh: &'a WarpMesh, source: Rect) -> Self {
        PreviewMap::Mesh { mesh, source }
    }

    /// A pin solver's mesh over `source`, the layer's bounds.
    pub fn pins(mesh: PinMeshView<'a>, source: Rect) -> Self {
        PreviewMap::Pins { mesh, source }
    }

    /// The app's map over a `cells` × `cells` grid of `source` (Perspective Warp: 40,
    /// `PreviewQuality::PLANE_CELLS`).
    pub fn function(source: Rect, cells: usize, map: &'a dyn Fn(Point) -> Point) -> Self {
        PreviewMap::Function { source, cells, map }
    }

    /// How much the map enlarges a source pixel, for the texture level: the quad's largest side
    /// ratio to the source frame for a box, 1 for the other maps (PhotoCraft `quad_scale`).
    pub fn source_scale(&self, source: [usize; 2]) -> f32 {
        // pc: crates/ui-egui/src/tools/transform_tool/options_bar.rs:9 (`quad_scale`).
        let PreviewMap::Quad(q) = self else { return 1.0 };
        let (w0, h0) = ((source[0] as f64).max(1e-9), (source[1] as f64).max(1e-9));
        let len = |a: Point, b: Point| (b - a).hypot();
        let s = (len(q[0], q[1]) / w0).max(len(q[3], q[2]) / w0).max(len(q[0], q[3]) / h0).max(len(q[1], q[2]) / h0);
        if s.is_finite() { s as f32 } else { 1.0 }
    }

    /// The mesh to draw at `zoom` screen points per document unit: positions, rest positions,
    /// texture coordinates within `uv` (the level's extent covering the source) and triangles in
    /// drawing order.
    pub fn build(&self, zoom: f64, uv: [f32; 2]) -> PreviewMesh {
        // pc: crates/ui-egui/src/tools/transform_tool/draw.rs:113 (quad), :83 (`front_vertices`);
        // crates/ui-egui/src/tools/warp_preview.rs:20 (`samples`), :58 (`triangle_order`);
        // crates/ui-egui/src/tools/puppet_ui.rs:348 (pins); perspective_ui.rs:263 (function).
        match self {
            PreviewMap::Quad(quad) => quad_mesh(quad, uv),
            PreviewMap::Mesh { mesh, source } => warp_mesh(mesh, *source, zoom, uv),
            PreviewMap::Pins { mesh, source } => pin_mesh(mesh, *source, uv),
            PreviewMap::Function { source, cells, map } => function_mesh(*source, *cells, map, uv),
        }
    }
}

/// A preview mesh in document units, before it is clipped and mapped to the screen.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PreviewMesh {
    /// Where each vertex is drawn.
    pub points: Vec<Point>,
    /// Where each vertex sits with no transform (the displacement order reads it).
    pub rest: Vec<Point>,
    /// The texture coordinate of each vertex.
    pub uv: Vec<[f32; 2]>,
    /// Triangles, three indices each, in drawing order.
    pub indices: Vec<u32>,
}

impl PreviewMesh {
    /// The mesh clipped to a document of `canvas` pixels: triangles past its edges are cut in
    /// document space, also under a turned view.
    #[must_use]
    pub fn clipped(self, canvas: [u32; 2]) -> PreviewMesh {
        clip_to_canvas(self, canvas)
    }
}

/// Two triangles per cell of a `row`-wide grid of vertices, in row order, for the cells whose four
/// corners `keep`.
fn grid_triangles(row: usize, rows: usize, keep: impl Fn(usize) -> bool) -> Vec<u32> {
    let mut out = Vec::new();
    for j in 0..rows.saturating_sub(1) {
        for i in 0..row.saturating_sub(1) {
            let a = j * row + i;
            if ![a, a + 1, a + row, a + row + 1].into_iter().all(&keep) {
                continue;
            }
            out.extend([a, a + 1, a + row + 1, a, a + row + 1, a + row].map(|k| k as u32));
        }
    }
    out
}

fn quad_mesh(quad: &[Point; 4], uv: [f32; 2]) -> PreviewMesh {
    let Some(h) = Homography::unit_to_quad(*quad) else { return PreviewMesh::default() };
    let n = PreviewQuality::QUAD_CELLS;
    let ahead = front_vertices(&h, n);
    let mut mesh = PreviewMesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let unit = Point::new(i as f64 / n as f64, j as f64 / n as f64);
            mesh.points.push(h.apply(unit));
            mesh.rest.push(unit);
            mesh.uv.push([unit.x as f32 * uv[0], unit.y as f32 * uv[1]]);
        }
    }
    // A cell with a corner past the horizon would stretch across the canvas.
    mesh.indices = grid_triangles(n + 1, n + 1, |k| ahead.get(k).copied().unwrap_or(false));
    mesh
}

fn warp_mesh(mesh: &WarpMesh, source: Rect, zoom: f64, uv: [f32; 2]) -> PreviewMesh {
    if !mesh.is_valid() {
        return PreviewMesh::default();
    }
    let (us, vs) = samples(mesh, zoom);
    // The rest is the unwarped frame, so the displacement order ranks a pulled part by how far it
    // moved from where the picture was (PhotoCraft's rest grid).
    let mut out = PreviewMesh::default();
    for &t in &vs {
        for &s in &us {
            out.points.push(surface_point(mesh, s, t));
            out.rest.push(Point::new(source.x0 + s * source.width(), source.y0 + t * source.height()));
            out.uv.push([s as f32 * uv[0], t as f32 * uv[1]]);
        }
    }
    out.indices = triangle_order(&out.points, &out.rest, us.len());
    out
}

fn pin_mesh(view: &PinMeshView<'_>, source: Rect, uv: [f32; 2]) -> PreviewMesh {
    let n = view.rest.len().min(view.deformed.len());
    let (w, h) = (source.width(), source.height());
    let unit = |v: f64, size: f64| if size > 0.0 { v / size } else { 0.0 };
    let mut mesh = PreviewMesh {
        points: view.deformed[..n].to_vec(),
        rest: view.rest[..n].to_vec(),
        uv: view.rest[..n].iter().map(|r| [unit(r.x - source.x0, w) as f32 * uv[0], unit(r.y - source.y0, h) as f32 * uv[1]]).collect(),
        indices: Vec::new(),
    };
    for &t in view.order {
        let Some(tri) = view.triangles.get(t as usize) else { continue };
        if tri.iter().all(|&k| (k as usize) < n) {
            mesh.indices.extend(tri);
        }
    }
    mesh
}

fn function_mesh(source: Rect, cells: usize, map: &dyn Fn(Point) -> Point, uv: [f32; 2]) -> PreviewMesh {
    let n = cells.max(1);
    let mut mesh = PreviewMesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
            let rest = Point::new(source.x0 + u * source.width(), source.y0 + v * source.height());
            mesh.points.push(map(rest));
            mesh.rest.push(rest);
            mesh.uv.push([u as f32 * uv[0], v as f32 * uv[1]]);
        }
    }
    let finite = |k: usize| mesh.points.get(k).is_some_and(|p| p.x.is_finite() && p.y.is_finite());
    mesh.indices = grid_triangles(n + 1, n + 1, finite);
    mesh
}

/// The sample positions `(us, vs)` along a warp mesh's two axes at `zoom` screen points per
/// document unit: every knot included, increasing, from 0 to 1. Each patch gets
/// `ceil(extent × zoom / SEGMENT_PX)` segments clamped to `MIN_PER_CELL..=MAX_PER_CELL`, where
/// `extent` is the larger side of its control points' bounding box; an axis over `MAX_PER_AXIS`
/// is scaled down together (each interval keeping at least 2). A zoom that is not finite or not
/// positive counts as 1.
pub(super) fn samples(mesh: &WarpMesh, zoom: f64) -> (Vec<f64>, Vec<f64>) {
    // pc: crates/ui-egui/src/tools/warp_preview.rs:20 (`samples`), :84 (`axis`).
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    let (cols, rows) = (mesh.columns(), mesh.rows());
    let width = mesh.width();
    let mut cu = vec![PreviewQuality::MIN_PER_CELL; cols];
    let mut cv = vec![PreviewQuality::MIN_PER_CELL; rows];
    // The screen size of each patch's control points bounds how far its surface can stretch.
    for (b, cv) in cv.iter_mut().enumerate() {
        for (a, cu) in cu.iter_mut().enumerate() {
            let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
            for j in 0..4 {
                for i in 0..4 {
                    if let Some(p) = mesh.points.get((3 * b + j) * width + 3 * a + i) {
                        lo = (lo.0.min(p.x), lo.1.min(p.y));
                        hi = (hi.0.max(p.x), hi.1.max(p.y));
                    }
                }
            }
            let extent = ((hi.0 - lo.0).max(hi.1 - lo.1) * zoom).abs();
            let n = if extent.is_finite() { (extent / PreviewQuality::SEGMENT_PX).ceil() as usize } else { PreviewQuality::MIN_PER_CELL };
            let n = n.clamp(PreviewQuality::MIN_PER_CELL, PreviewQuality::MAX_PER_CELL);
            *cu = (*cu).max(n);
            *cv = (*cv).max(n);
        }
    }
    (axis(&mesh.us, &mut cu), axis(&mesh.vs, &mut cv))
}

/// Samples along one axis: `counts[k]` segments in knot interval `k`, scaled down together when
/// the total passes `MAX_PER_AXIS`.
fn axis(knots: &[f64], counts: &mut [usize]) -> Vec<f64> {
    let total: usize = counts.iter().sum();
    if total > PreviewQuality::MAX_PER_AXIS {
        for c in counts.iter_mut() {
            *c = (*c * PreviewQuality::MAX_PER_AXIS / total).max(2);
        }
    }
    let mut out = Vec::with_capacity(counts.iter().sum::<usize>() + 1);
    for (pair, &n) in knots.windows(2).zip(counts.iter()) {
        out.extend((0..n).map(|i| pair[0] + (pair[1] - pair[0]) * i as f64 / n as f64));
    }
    out.push(1.0);
    out
}

/// The point of a warp mesh's surface at mesh parameters `(s, t)` (bicubic Bézier patches,
/// `WarpMesh::locate`).
pub(super) fn surface_point(mesh: &WarpMesh, s: f64, t: f64) -> Point {
    // new: the library's `WarpMesh` has no evaluation yet; PhotoCraft's is `BezierMesh::eval` in
    // `photocraft-geom` (engine), the de Casteljau patch evaluation.
    let (column, row, u, v) = mesh.locate(s, t);
    let bernstein = |x: f64| {
        let y = 1.0 - x;
        [y * y * y, 3.0 * y * y * x, 3.0 * y * x * x, x * x * x]
    };
    let (bu, bv) = (bernstein(u), bernstein(v));
    let mut sum = kurbo::Vec2::ZERO;
    for (j, wv) in bv.iter().enumerate() {
        for (i, wu) in bu.iter().enumerate() {
            sum += mesh.point(3 * column + i, 3 * row + j).to_vec2() * (wu * wv);
        }
    }
    sum.to_point()
}

/// Triangle indices for a `row`-wide vertex grid, two per cell, cells ordered from the most
/// displaced (mean of its four corners' distance from rest) to the least. Empty when `row < 2` or
/// the lengths differ.
pub(super) fn triangle_order(points: &[Point], rest: &[Point], row: usize) -> Vec<u32> {
    // pc: crates/ui-egui/src/tools/warp_preview.rs:58 (`triangle_order`).
    if row < 2 || points.len() != rest.len() {
        return Vec::new();
    }
    let rows = points.len() / row;
    let moved = |k: usize| match (points.get(k), rest.get(k)) {
        (Some(p), Some(r)) => (*p - *r).hypot(),
        _ => 0.0,
    };
    let mut cells: Vec<(f64, usize)> = Vec::with_capacity(rows * row);
    for j in 0..rows.saturating_sub(1) {
        for i in 0..row - 1 {
            let a = j * row + i;
            cells.push(((moved(a) + moved(a + 1) + moved(a + row) + moved(a + row + 1)) / 4.0, a));
        }
    }
    cells.sort_by(|x, y| y.0.total_cmp(&x.0));
    let mut out = Vec::with_capacity(cells.len() * 6);
    for (_, a) in cells {
        out.extend([a, a + 1, a + row + 1, a, a + row + 1, a + row].map(|k| k as u32));
    }
    out
}

/// The homogeneous `w` of `h` at the unit-square point `(u, v)`.
fn homogeneous_w(h: &Homography, u: f64, v: f64) -> f64 {
    let [g, k, i] = h.row(2);
    g * u + k * v + i
}

/// For an `n` × `n` grid of cells over the unit square mapped by `h` (`(n + 1)²` vertices, row by
/// row): whether each vertex lands on the same side of the horizon as the unit square (its
/// homogeneous `w` has the sign of the square's, beyond 1e-9). A Distort dragged past the horizon
/// sends the rest of the image to the far side, where its cells would wrap round infinity.
pub(super) fn front_vertices(h: &Homography, n: usize) -> Vec<bool> {
    // pc: crates/ui-egui/src/tools/transform_tool/draw.rs:83 (`front_vertices`).
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let ahead = corners.iter().filter(|(x, y)| homogeneous_w(h, *x, *y) > 0.0).count();
    // The side most of the square's corners are on; a split square follows its centre.
    let front = match ahead {
        3 | 4 => 1.0,
        0 | 1 => -1.0,
        _ if homogeneous_w(h, 0.5, 0.5) >= 0.0 => 1.0,
        _ => -1.0,
    };
    (0..=n)
        .flat_map(|j| (0..=n).map(move |i| (i as f64 / n as f64, j as f64 / n as f64)))
        .map(|(u, v)| front * homogeneous_w(h, u, v) > 1e-9)
        .collect()
}

/// `mesh` with every triangle cut to the document `canvas` (0, 0)–(width, height) in document
/// space, so the cut follows a turned view.
pub(super) fn clip_to_canvas(mesh: PreviewMesh, canvas: [u32; 2]) -> PreviewMesh {
    // pc: crates/ui-egui/src/tools/transform_tool/draw.rs:395 (`clip_to_canvas`).
    let size = (f64::from(canvas[0]), f64::from(canvas[1]));
    let inside = |p: &Point| p.x >= 0.0 && p.y >= 0.0 && p.x <= size.0 && p.y <= size.1;
    if mesh.points.iter().all(inside) {
        return mesh;
    }
    #[derive(Clone, Copy)]
    struct Vertex {
        point: Point,
        rest: Point,
        uv: [f32; 2],
    }
    let vertex = |k: u32| {
        let k = k as usize;
        Some(Vertex { point: *mesh.points.get(k)?, rest: *mesh.rest.get(k)?, uv: *mesh.uv.get(k)? })
    };
    let mut out = PreviewMesh::default();
    for tri in mesh.indices.as_chunks::<3>().0 {
        let Some(mut poly) = tri.iter().map(|&k| vertex(k)).collect::<Option<Vec<_>>>() else { continue };
        // Sutherland–Hodgman against the four canvas edges (`dist` >= 0 is the canvas side).
        for edge in 0..4 {
            let dist = |v: &Vertex| match edge {
                0 => v.point.x,
                1 => size.0 - v.point.x,
                2 => v.point.y,
                _ => size.1 - v.point.y,
            };
            let mut next = Vec::with_capacity(poly.len() + 2);
            for (k, a) in poly.iter().enumerate() {
                let Some(b) = poly.get((k + 1) % poly.len()) else { continue };
                let (da, db) = (dist(a), dist(b));
                if da >= 0.0 {
                    next.push(*a);
                }
                if (da >= 0.0) != (db >= 0.0) && da != db {
                    let f = da / (da - db);
                    let f32f = f as f32;
                    next.push(Vertex {
                        point: a.point.lerp(b.point, f),
                        rest: a.rest.lerp(b.rest, f),
                        uv: [a.uv[0] + (b.uv[0] - a.uv[0]) * f32f, a.uv[1] + (b.uv[1] - a.uv[1]) * f32f],
                    });
                }
            }
            poly = next;
        }
        if poly.len() < 3 {
            continue;
        }
        let base = out.points.len() as u32;
        for v in &poly {
            out.points.push(v.point);
            out.rest.push(v.rest);
            out.uv.push(v.uv);
        }
        for k in 1..poly.len() as u32 - 1 {
            out.indices.extend([base, base + k, base + k + 1]);
        }
    }
    out
}
