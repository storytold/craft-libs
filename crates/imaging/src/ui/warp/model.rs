//! The pure model of a warp: [`WarpMesh`], its edits and the geometry the grid reads.

use std::borrow::Cow;

use kurbo::{BezPath, Point, Rect, Vec2};

/// A warp's control mesh: bicubic Bézier patches sharing their edges, as
/// `(3 columns + 1) × (3 rows + 1)` control points row by row, in document units.
///
/// `us` and `vs` are the patch boundaries in the mesh's global parameter space (PhotoCraft's
/// `BezierMesh::us` and `vs`): the knots of the columns and rows, increasing from 0 to 1, one more
/// than there are columns or rows. A patch's local `(u, v)` and the mesh's `(s, t)` are related by
/// them, `s = us[column] + u * (us[column + 1] - us[column])`; the patches need not be even.
///
/// A point on a row and a column that are both multiples of 3 is an anchor, where patches meet.
/// The other points on those rows and columns are the anchors' handles; the points inside a patch
/// shape its inside and are neither drawn nor dragged.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct WarpMesh {
    /// Column patch boundaries in the mesh's parameter space, increasing from 0 to 1.
    pub us: Vec<f64>,
    /// Row patch boundaries in the mesh's parameter space, increasing from 0 to 1.
    pub vs: Vec<f64>,
    /// The control points row by row, in document units.
    pub points: Vec<Point>,
}

/// The side of its anchor a handle leaves on, in the mesh's own order: up and down along a
/// column, left and right along a row (in the document, unmirrored).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WarpSide {
    /// Above the anchor.
    Up,
    /// Left of the anchor.
    Left,
    /// Right of the anchor.
    Right,
    /// Below the anchor.
    Down,
}

impl WarpMesh {
    /// An unwarped mesh over `rect`: `columns` × `rows` patches with even knots.
    pub fn identity(rect: Rect, columns: usize, rows: usize) -> Self {
        let even = |n: usize| (0..=n).map(|i| i as f64 / n as f64).collect();
        Self::straight(rect, even(columns.max(1)), even(rows.max(1)))
    }

    /// An unwarped mesh over `rect` with patch boundaries `us` and `vs` (see [`WarpMesh`]), as
    /// PhotoCraft's `BezierMesh::fit` of the identity. `None` when the knots are not valid.
    pub fn identity_with_knots(rect: Rect, us: Vec<f64>, vs: Vec<f64>) -> Option<Self> {
        (Self::knots_valid(&us) && Self::knots_valid(&vs)).then(|| Self::straight(rect, us, vs))
    }

    /// The unwarped mesh over `rect` for valid knots.
    fn straight(rect: Rect, us: Vec<f64>, vs: Vec<f64>) -> Self {
        // Control points sit at thirds of each patch's span, which is exact for a straight mesh.
        let along = |knots: &[f64]| -> Vec<f64> {
            let mut out: Vec<f64> = knots.windows(2).flat_map(|k| (0..3).map(move |i| k[0] + (k[1] - k[0]) * i as f64 / 3.0)).collect();
            out.push(1.0);
            out
        };
        let (xs, ys) = (along(&us), along(&vs));
        let points = ys.iter().flat_map(|&v| xs.iter().map(move |&u| Point::new(rect.x0 + u * rect.width(), rect.y0 + v * rect.height()))).collect();
        Self { us, vs, points }
    }

    /// A mesh from PhotoCraft's parts: knots `us`, `vs` and the control points row by row. `None`
    /// when they do not make a [valid](Self::is_valid) mesh.
    pub fn new(us: Vec<f64>, vs: Vec<f64>, points: Vec<Point>) -> Option<Self> {
        let mesh = Self { us, vs, points };
        mesh.is_valid().then_some(mesh)
    }

    /// Whether `knots` are patch boundaries: at least two, from 0 to 1, strictly increasing.
    fn knots_valid(knots: &[f64]) -> bool {
        knots.len() >= 2 && knots[0] == 0.0 && knots[knots.len() - 1] == 1.0 && knots.windows(2).all(|k| k[1] > k[0])
    }

    /// The number of patch columns.
    pub fn columns(&self) -> usize {
        self.us.len().saturating_sub(1)
    }

    /// The number of patch rows.
    pub fn rows(&self) -> usize {
        self.vs.len().saturating_sub(1)
    }

    /// The mesh parameters `(s, t)` of the point `(u, v)` of patch (`column`, `row`).
    pub fn params(&self, column: usize, row: usize, u: f64, v: f64) -> (f64, f64) {
        (self.us[column] + u * (self.us[column + 1] - self.us[column]), self.vs[row] + v * (self.vs[row + 1] - self.vs[row]))
    }

    /// The patch under the mesh parameters `(s, t)` (each clamped to 0 to 1) and the parameters
    /// within it: the inverse of [`params`](Self::params). A parameter on a knot belongs to the
    /// patch before it, as in PhotoCraft's `BezierMesh::eval`.
    pub fn locate(&self, s: f64, t: f64) -> (usize, usize, f64, f64) {
        let at = |knots: &[f64], x: f64| {
            let x = if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 };
            let k = knots[1..knots.len() - 1].partition_point(|&k| k < x);
            (k, (x - knots[k]) / (knots[k + 1] - knots[k]))
        };
        let ((column, u), (row, v)) = (at(&self.us, s), at(&self.vs, t));
        (column, row, u, v)
    }

    /// Control points per row: `3 columns + 1`.
    pub fn width(&self) -> usize {
        3 * self.columns() + 1
    }

    /// Control points per column: `3 rows + 1`.
    pub fn height(&self) -> usize {
        3 * self.rows() + 1
    }

    /// Whether the knots are valid (see [`WarpMesh`]) and there are as many points as the patches
    /// need.
    pub fn is_valid(&self) -> bool {
        Self::knots_valid(&self.us) && Self::knots_valid(&self.vs) && self.points.len() == self.width() * self.height()
    }

    /// The control point at column `i`, row `j`.
    pub fn point(&self, i: usize, j: usize) -> Point {
        self.points[j * self.width() + i]
    }

    /// The anchor the point `index` belongs to: its own index for an anchor, its anchor's for a
    /// handle, `None` for a point inside a patch.
    pub fn anchor_of(&self, index: usize) -> Option<usize> {
        let w = self.width();
        let (i, j) = (index % w, index / w);
        let near = |k: usize| match k % 3 {
            0 => k,
            1 => k - 1,
            _ => k + 1,
        };
        match (i % 3 == 0, j % 3 == 0) {
            (true, true) => Some(index),
            (true, false) => Some(near(j) * w + i),
            (false, true) => Some(j * w + near(i)),
            (false, false) => None,
        }
    }

    /// The handles of the anchor `index`, each with its side, in the mesh's order.
    pub fn handles(&self, index: usize) -> Vec<(usize, WarpSide)> {
        let w = self.width();
        let (i, j) = (index % w, index / w);
        let mut out = Vec::with_capacity(4);
        if j > 0 {
            out.push((index - w, WarpSide::Up));
        }
        if i > 0 {
            out.push((index - 1, WarpSide::Left));
        }
        if i + 1 < w {
            out.push((index + 1, WarpSide::Right));
        }
        if j + 1 < self.height() {
            out.push((index + w, WarpSide::Down));
        }
        out
    }

    /// The anchors row by row.
    pub(crate) fn anchors(&self) -> impl Iterator<Item = usize> + '_ {
        let w = self.width();
        (0..self.height()).step_by(3).flat_map(move |j| (0..w).step_by(3).map(move |i| j * w + i))
    }

    /// The chain of cubic segments along control row `j`.
    pub(crate) fn row_path(&self, j: usize) -> BezPath {
        chain((0..self.width()).map(|i| self.point(i, j)).collect())
    }

    /// The chain of cubic segments along control column `i`.
    pub(crate) fn column_path(&self, i: usize) -> BezPath {
        chain((0..self.height()).map(|j| self.point(i, j)).collect())
    }

    /// The patch boundaries: each a chain of cubic segments along a row or a column of anchors.
    pub(crate) fn boundaries(&self) -> Vec<BezPath> {
        let rows = (0..self.height()).step_by(3).map(|j| self.row_path(j));
        let columns = (0..self.width()).step_by(3).map(|i| self.column_path(i));
        rows.chain(columns).collect()
    }

    /// The mesh with new lines of anchors cut through patch (`column`, `row`) at `(u, v)` (each 0
    /// to 1 across the patch): [`Vertical`](WarpSplit::Vertical) cuts the patch column at `u`,
    /// [`Horizontal`](WarpSplit::Horizontal) the patch row at `v` and [`Cross`](WarpSplit::Cross)
    /// both. The patches on a cut line are cut by de Casteljau, so the surface is unchanged, and
    /// the new knot is the mesh parameter of the cut ([`params`](Self::params)). A line whose
    /// patch index is past the mesh or whose parameter is not strictly between 0 and 1 is not
    /// cut.
    pub fn split(&self, split: WarpSplit, column: usize, row: usize, u: f64, v: f64) -> WarpMesh {
        if !self.is_valid() {
            return self.clone();
        }
        let mut out = self.clone();
        let inside = |x: f64| x > 0.0 && x < 1.0;
        if split != WarpSplit::Horizontal && column < self.columns() && inside(u) {
            out = out.split_column(column, u, self.params(column, 0, u, 0.0).0);
        }
        if split != WarpSplit::Vertical && row < self.rows() && inside(v) {
            out = out.split_row(row, v, self.params(0, row, 0.0, v).1);
        }
        out
    }

    /// A one-patch mesh with its rule-of-thirds lines made real: cut into 3 × 3 patches at a
    /// third and two thirds across each way, the surface unchanged. PhotoCraft does this before
    /// the first split of a custom warp so that the cut divides the grid on screen instead of
    /// replacing it. Any other mesh is returned as it is.
    pub fn commit_thirds(&self) -> WarpMesh {
        if !self.is_valid() || self.columns() != 1 || self.rows() != 1 {
            return self.clone();
        }
        // The second cut is at half of what is left after the first. The knots are 0, 1/3, 2/3, 1.
        self.split_column(0, 1.0 / 3.0, 1.0 / 3.0).split_column(1, 0.5, 2.0 / 3.0).split_row(0, 1.0 / 3.0, 1.0 / 3.0).split_row(1, 0.5, 2.0 / 3.0)
    }

    /// The de Casteljau cut of a cubic at `t`: the five new control points after its first.
    fn cut(p: [Point; 4], t: f64) -> [Point; 5] {
        let lerp = |a: Point, b: Point| Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        let (a, b, c) = (lerp(p[0], p[1]), lerp(p[1], p[2]), lerp(p[2], p[3]));
        let (d, e) = (lerp(a, b), lerp(b, c));
        [a, d, lerp(d, e), e, c]
    }

    /// A new row of anchors through patch row `index` at `t`, at mesh parameter `knot`.
    fn split_row(&self, index: usize, t: f64, knot: f64) -> WarpMesh {
        let (w, h) = (self.width(), self.height());
        let mut rows: Vec<Vec<Point>> = (0..h).map(|j| (0..w).map(|i| self.point(i, j)).collect()).collect();
        let at = 3 * index;
        let cuts: Vec<[Point; 5]> = (0..w).map(|i| Self::cut(std::array::from_fn(|k| self.point(i, at + k)), t)).collect();
        let inserted: Vec<Vec<Point>> = (0..5).map(|k| cuts.iter().map(|c| c[k]).collect()).collect();
        rows.splice(at + 1..at + 3, inserted);
        let mut vs = self.vs.clone();
        vs.insert(index + 1, knot);
        WarpMesh { us: self.us.clone(), vs, points: rows.concat() }
    }

    /// A new column of anchors through patch column `index` at `t`, at mesh parameter `knot`.
    fn split_column(&self, index: usize, t: f64, knot: f64) -> WarpMesh {
        let (w, h) = (self.width(), self.height());
        let at = 3 * index;
        let mut points = Vec::with_capacity((w + 3) * h);
        for j in 0..h {
            let row: [Point; 4] = std::array::from_fn(|k| self.point(at + k, j));
            points.extend((0..=at).map(|i| self.point(i, j)));
            points.extend(Self::cut(row, t));
            points.extend((at + 3..w).map(|i| self.point(i, j)));
        }
        let mut us = self.us.clone();
        us.insert(index + 1, knot);
        WarpMesh { us, vs: self.vs.clone(), points }
    }

    /// The split a quick split (Alt or Ctrl with no tool set) makes at `(u, v)` of patch
    /// (`column`, `row`), PhotoCraft's rule: a [`Cross`](WarpSplit::Cross) in the open, and the
    /// perpendicular line when the pointer is close to an existing line of one direction. A
    /// one-patch mesh's rule-of-thirds lines count as existing lines. "Close" is 8 document units
    /// of the warp's source rectangle `bounds` across, within 2 to 12 % of the mesh.
    pub fn quick_split(&self, bounds: Rect, column: usize, row: usize, u: f64, v: f64) -> WarpSplit {
        let tolerance = |extent: f64| (8.0 / extent.abs().max(1.0)).clamp(0.02, 0.12);
        let thirds = self.columns() == 1 && self.rows() == 1;
        let near = |knots: &[f64], at: f64, tolerance: f64| {
            let inner = knots[1..knots.len() - 1].iter().copied();
            let thirds = thirds.then_some([1.0 / 3.0, 2.0 / 3.0]).into_iter().flatten();
            inner.chain(thirds).any(|k| (k - at).abs() < tolerance)
        };
        let (s, t) = self.params(column, row, u, v);
        match (near(&self.us, s, tolerance(bounds.width())), near(&self.vs, t, tolerance(bounds.height()))) {
            (true, false) => WarpSplit::Horizontal,
            (false, true) => WarpSplit::Vertical,
            _ => WarpSplit::Cross,
        }
    }

    /// The point of the surface nearest to document point `p`: its patch column and row, its
    /// parameters `(u, v)` in that patch and its distance from `p`.
    pub(crate) fn nearest(&self, p: Point) -> (usize, usize, f64, f64, f64) {
        const COARSE: usize = 8;
        const ROUNDS: usize = 10;
        let mut best = (0, 0, 0.0, 0.0, f64::INFINITY);
        for row in 0..self.rows() {
            for column in 0..self.columns() {
                let at = |u: f64, v: f64| self.patch_point(column, row, u, v).distance(p);
                let mut found = (0.5, 0.5, f64::INFINITY);
                for j in 0..=COARSE {
                    for i in 0..=COARSE {
                        let (u, v) = (i as f64 / COARSE as f64, j as f64 / COARSE as f64);
                        let d = at(u, v);
                        if d < found.2 {
                            found = (u, v, d);
                        }
                    }
                }
                let mut reach = 1.0 / COARSE as f64;
                for _ in 0..ROUNDS {
                    let (cu, cv) = (found.0, found.1);
                    for j in -2..=2 {
                        for i in -2..=2 {
                            let (u, v) = ((cu + f64::from(i) * reach / 2.0).clamp(0.0, 1.0), (cv + f64::from(j) * reach / 2.0).clamp(0.0, 1.0));
                            let d = at(u, v);
                            if d < found.2 {
                                found = (u, v, d);
                            }
                        }
                    }
                    reach /= 2.0;
                }
                if found.2 < best.4 {
                    best = (column, row, found.0, found.1, found.2);
                }
            }
        }
        best
    }

    /// The surface point of patch (`column`, `row`) at `(u, v)`.
    pub(crate) fn patch_point(&self, column: usize, row: usize, u: f64, v: f64) -> Point {
        let across: [Point; 4] = std::array::from_fn(|j| cubic(std::array::from_fn(|i| self.point(3 * column + i, 3 * row + j)), u));
        cubic(across, v)
    }

    /// A one-patch mesh's rule-of-thirds curves: the patch at a third and two thirds across, each
    /// way.
    pub(crate) fn thirds(&self) -> Vec<BezPath> {
        let curve = |q: [Point; 4]| {
            let mut path = BezPath::new();
            path.move_to(q[0]);
            path.curve_to(q[1], q[2], q[3]);
            path
        };
        let mut out = Vec::with_capacity(4);
        for t in [1.0 / 3.0, 2.0 / 3.0] {
            out.push(curve(std::array::from_fn(|j| cubic(std::array::from_fn(|i| self.point(i, j)), t))));
            out.push(curve(std::array::from_fn(|i| cubic(std::array::from_fn(|j| self.point(i, j)), t))));
        }
        out
    }
    fn bernstein(t: f64) -> [f64; 4] {
        let s = 1.0 - t;
        [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
    }

    /// The surface point at the mesh parameters `(s, t)`.
    pub fn eval(&self, s: f64, t: f64) -> Point {
        let (column, row, u, v) = self.locate(s, t);
        let rows: [Point; 4] = std::array::from_fn(|b| cubic(std::array::from_fn(|a| self.point(3 * column + a, 3 * row + b)), u));
        cubic(rows, v)
    }

    /// The mesh parameters of the surface point at `p`, `None` off the surface: the nearest of a
    /// sampled grid refined by Newton steps. The grid's default surface rule.
    pub fn surface_at(&self, p: Point) -> Option<(f64, f64)> {
        let n = 16 * self.columns().max(self.rows());
        let (mut s, mut t) = (0..=n)
            .flat_map(|i| (0..=n).map(move |j| (i as f64 / n as f64, j as f64 / n as f64)))
            .min_by(|a, b| self.eval(a.0, a.1).distance(p).total_cmp(&self.eval(b.0, b.1).distance(p)))?;
        let h = 1e-5;
        for _ in 0..12 {
            let q = self.eval(s, t);
            let ds = (self.eval(s + h, t) - self.eval(s - h, t)) / (2.0 * h);
            let dt = (self.eval(s, t + h) - self.eval(s, t - h)) / (2.0 * h);
            let det = ds.x * dt.y - ds.y * dt.x;
            if det.abs() < 1e-12 {
                break;
            }
            let r = p - q;
            s = (s + (r.x * dt.y - r.y * dt.x) / det).clamp(0.0, 1.0);
            t = (t + (ds.x * r.y - ds.y * r.x) / det).clamp(0.0, 1.0);
        }
        (self.eval(s, t).distance(p) < 1.0).then_some((s, t))
    }

    /// The mesh with `edit` applied, as PhotoCraft's Warp does in outline: an anchor carries its
    /// handles, a pull moves the patch's control points so the surface point follows the pointer.
    /// The grid's default rule.
    pub fn edited(&self, edit: &WarpEdit) -> WarpMesh {
        let mut out = self.clone();
        let w = self.width();
        match *edit {
            WarpEdit::Point { index, by } if index < self.points.len() => {
                out.points[index] += by;
                if self.anchor_of(index) == Some(index) {
                    // An anchor carries its handles and the twist points beside it.
                    let (i, j) = ((index % w) as isize, (index / w) as isize);
                    for (di, dj) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                        let (x, y) = (i + di, j + dj);
                        if (0..w as isize).contains(&x) && (0..self.height() as isize).contains(&y) {
                            out.points[y as usize * w + x as usize] += by;
                        }
                    }
                }
            }
            WarpEdit::Point { .. } => {}
            WarpEdit::Split { split, column, row, u, v, thirds } => {
                let base = if thirds { self.commit_thirds() } else { self.clone() };
                return base.split(split, column, row, u, v);
            }
            WarpEdit::Surface { s, t, by } => {
                // The least change of the patch's control points that moves the surface point by `by`.
                let (column, row, u, v) = self.locate(s, t);
                let (bu, bv) = (Self::bernstein(u), Self::bernstein(v));
                let sum: f64 = bu.iter().flat_map(|a| bv.iter().map(move |b| (a * b).powi(2))).sum();
                for (b, wv) in bv.iter().enumerate() {
                    for (a, wu) in bu.iter().enumerate() {
                        out.points[(3 * row + b) * w + 3 * column + a] += by * (wu * wv / sum);
                    }
                }
            }
        }
        out
    }

    /// The mesh a split cuts and whether its thirds were made real first: a custom (not `preset`)
    /// one-patch mesh is cut into thirds first.
    pub(crate) fn split_base(&self, preset: bool) -> (Cow<'_, WarpMesh>, bool) {
        let thirds = !preset && self.columns() == 1 && self.rows() == 1;
        (if thirds { Cow::Owned(self.commit_thirds()) } else { Cow::Borrowed(self) }, thirds)
    }

    /// What the split would insert at document point `p`: the armed `tool`'s lines, or with
    /// `quick` the quick split's. A point off the surface splits at the nearest point of it.
    /// `None` where every line of the split lies on an existing one; a cross with one line on an
    /// existing one inserts the other alone. `bounds` is the warp's source rectangle.
    pub(crate) fn split_at(&self, preset: bool, bounds: Rect, tool: Option<WarpSplit>, quick: bool, p: Point) -> Option<SplitAt> {
        let (mesh, thirds) = self.split_base(preset);
        let (column, row, u, v, _) = mesh.nearest(p);
        let kind = tool.filter(|_| !quick).unwrap_or_else(|| mesh.quick_split(bounds, column, row, u, v));
        let free = |c: f64| (SPLIT_EDGE..=1.0 - SPLIT_EDGE).contains(&c);
        let split = match (kind != WarpSplit::Horizontal && free(u), kind != WarpSplit::Vertical && free(v)) {
            (true, true) => WarpSplit::Cross,
            (true, false) => WarpSplit::Vertical,
            (false, true) => WarpSplit::Horizontal,
            (false, false) => return None,
        };
        // The way a line moves as its parameter grows, for the arrow across it.
        let step = 1e-3;
        let across = |along_u: bool| {
            let at = |d: f64| if along_u { mesh.patch_point(column, row, d, v) } else { mesh.patch_point(column, row, u, d) };
            let c = if along_u { u } else { v };
            at((c + step).min(1.0)) - at((c - step).max(0.0))
        };
        let motion = match kind {
            WarpSplit::Cross => None,
            WarpSplit::Vertical => Some(across(true)),
            WarpSplit::Horizontal => Some(across(false)),
        };
        Some(SplitAt { split, column, row, u, v, thirds, motion })
    }
}

/// How close to a patch's edge, as a share of the patch, a split line may lie before it counts as
/// on the existing line.
const SPLIT_EDGE: f64 = 0.01;

/// The lines a pointer would insert ([`WarpMesh::split_at`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SplitAt {
    split: WarpSplit,
    column: usize,
    row: usize,
    u: f64,
    v: f64,
    thirds: bool,
    /// The way the line across the pointer runs, in document units; `None` for a cross.
    pub(crate) motion: Option<Vec2>,
}

impl SplitAt {
    /// The edit the split inserts.
    pub(crate) fn edit(&self) -> WarpEdit {
        WarpEdit::Split { split: self.split, column: self.column, row: self.row, u: self.u, v: self.v, thirds: self.thirds }
    }

    /// The new lines of `mesh`, in document units.
    pub(crate) fn lines(&self, mesh: &WarpMesh) -> Vec<BezPath> {
        let base = if self.thirds { mesh.commit_thirds() } else { mesh.clone() };
        let cut = base.split(self.split, self.column, self.row, self.u, self.v);
        let column = || cut.column_path(3 * self.column + 3);
        let row = || cut.row_path(3 * self.row + 3);
        match self.split {
            WarpSplit::Vertical => vec![column()],
            WarpSplit::Horizontal => vec![row()],
            WarpSplit::Cross => vec![column(), row()],
        }
    }
}

/// The cubic segments through `points`, the first the start and every three after the next
/// segment's controls and end.
fn chain(points: Vec<Point>) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for &[a, b, c] in points[1..].as_chunks::<3>().0 {
        path.curve_to(a, b, c);
    }
    path
}

/// The cubic Bézier through control points `p` at `t`.
pub(crate) fn cubic(p: [Point; 4], t: f64) -> Point {
    let s = 1.0 - t;
    let b = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
    Point::new(p.iter().zip(b).map(|(p, b)| p.x * b).sum(), p.iter().zip(b).map(|(p, b)| p.y * b).sum())
}

/// Which lines a split puts through a point of a warp mesh ([`WarpMesh::split`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WarpSplit {
    /// Both lines through the point: a column and a row of anchors.
    Cross,
    /// A line down the mesh, along its columns: it cuts a patch column in two and adds a column
    /// of anchors.
    Vertical,
    /// A line across the mesh, along its rows: it cuts a patch row in two and adds a row of
    /// anchors.
    Horizontal,
}

/// A warp drag or nudge, in document units. A drag's `by` is the whole move from its press, which
/// the host applies to the mesh it had at the press; a nudge's (a [`Set`](EditEvent::Set)) is
/// one step from the mesh it has.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum WarpEdit {
    /// Move the control point `index` (an anchor carries its handles along: the host's rule).
    #[non_exhaustive]
    Point {
        /// The index of the point.
        index: usize,
        /// The move, in document units.
        by: Vec2,
    },
    /// Pull the surface at the host's parameters `(s, t)` ([`WarpGrid::surface`](super::WarpGrid)).
    #[non_exhaustive]
    Surface {
        /// The mesh parameter across the columns.
        s: f64,
        /// The mesh parameter down the rows.
        t: f64,
        /// The move, in document units.
        by: Vec2,
    },
    /// Insert lines of anchors: the host's mesh becomes [`WarpMesh::split`] of itself with these
    /// arguments. Always a [`Set`](EditEvent::Set).
    ///
    /// `thirds` is set when the cut is of a one-patch mesh that PhotoCraft's custom warp first
    /// divides at its thirds ([`WarpMesh::commit_thirds`]): the host applies that to its mesh
    /// before `split`, and `column`, `row`, `u` and `v` name a patch of the result.
    #[non_exhaustive]
    Split {
        /// Which lines to insert.
        split: WarpSplit,
        /// The patch column cut.
        column: usize,
        /// The patch row cut.
        row: usize,
        /// The cut across the patch, 0 to 1.
        u: f64,
        /// The cut down the patch, 0 to 1.
        v: f64,
        /// Whether the thirds are made real first.
        thirds: bool,
    },
}
