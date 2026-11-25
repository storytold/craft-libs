//! Vanishing Point's planes as data and the app's scene geometry.

use serde::{Deserialize, Serialize};

use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::Point;

/// The most planes a session holds (PhotoCraft `MAX_PLANES`, so the control channel can't make it
/// allocate without bound).
pub const MAX_PLANES: usize = 64;

/// The most grid lines drawn per direction (PhotoCraft `MAX_LINES`).
pub const MAX_GRID_LINES: usize = 200;

/// A plane's four corners in document pixels, in the scene's order (top-left, top-right,
/// bottom-right, bottom-left after [`VanishingScene::normalized`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VanishingPlane {
    /// The corners in document pixels, `[x, y]` each.
    pub corners: [[f64; 2]; 4],
}

impl VanishingPlane {
    /// The plane with these corners.
    pub const fn new(corners: [[f64; 2]; 4]) -> Self {
        Self { corners }
    }

    /// The map from the unit square to the plane (`Homography::unit_to_quad`), `None` for a
    /// degenerate plane.
    pub fn homography(&self) -> Option<Homography> {
        // pc: crates/algo/src/vanishing.rs:77 `VpPlane::homography`.
        Homography::unit_to_quad(self.corners.map(|c| Point::new(c[0], c[1])))
    }

    /// The middles of the four edges (0 top, 1 right, 2 bottom, 3 left): the edge nodes.
    ///
    /// The middles are the plane's own (the image of the unit square's edge middles, which
    /// perspective moves off the corners' midpoints). A degenerate plane has no map: its nodes are
    /// the corners' midpoints.
    pub fn edge_nodes(&self) -> [[f64; 2]; 4] {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:649 `edge_nodes` (`None` there for a degenerate
        // plane; the array here falls back to the midpoints).
        let q = &self.corners;
        let Some(h) = self.homography() else {
            return std::array::from_fn(|i| [(q[i][0] + q[(i + 1) % 4][0]) / 2.0, (q[i][1] + q[(i + 1) % 4][1]) / 2.0]);
        };
        [(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)].map(|(u, v)| {
            let at = h.apply(Point::new(u, v));
            [at.x, at.y]
        })
    }
}

/// An edge of a plane, as the engine names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum PlaneEdge {
    /// The top edge.
    Top,
    /// The right edge.
    Right,
    /// The bottom edge.
    Bottom,
    /// The left edge.
    Left,
}

impl PlaneEdge {
    /// The four edges, clockwise from the top.
    pub const ALL: [PlaneEdge; 4] = [PlaneEdge::Top, PlaneEdge::Right, PlaneEdge::Bottom, PlaneEdge::Left];
}

/// A plane torn off an edge of an earlier plane (the engine's `{"from", "edge", "angle",
/// "depth"}`): it follows its parent when that is edited.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TornPlane {
    /// The parent plane's index, always below this plane's.
    pub from: usize,
    /// The parent's edge the plane grows from.
    pub edge: PlaneEdge,
    /// Degrees between the planes (90: perpendicular).
    pub angle: f64,
    /// Extent as a multiple of the parent's adjacent side.
    pub depth: f64,
}

impl TornPlane {
    /// A plane torn off `edge` of plane `from`, `angle` degrees from it and `depth` times its adjacent side deep.
    pub const fn new(from: usize, edge: PlaneEdge, angle: f64, depth: f64) -> Self {
        Self { from, edge, angle, depth }
    }
}

/// How believable a plane is: the grid and outline in `accent` (valid), `status.warn.fg`
/// (questionable) or `status.bad.fg` (invalid; drawn as its outline only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlaneCheck {
    /// The perspective holds.
    Valid,
    /// The perspective is doubtful.
    Questionable,
    /// The perspective cannot hold.
    Invalid,
}

/// The app's perspective geometry (lead call 45; PhotoCraft's `photocraft_algo::vanishing`): the
/// camera solved from the first plane, torn planes, the check, the plane under a point, the
/// foreshortened stamp ring and the clone source.
pub trait VanishingScene {
    /// The plane with its corners in the scene's order.
    fn normalized(&self, plane: VanishingPlane) -> VanishingPlane;

    /// The planes with every torn plane's corners rebuilt from its parent (a torn plane that
    /// cannot be rebuilt has all-zero corners). `planes[0]` is always drawn.
    fn resolve(&self, planes: &[VanishingPlane], torn: &[Option<TornPlane>]) -> Vec<VanishingPlane>;

    /// How believable plane `index` is. [`PlaneCheck::Invalid`] depends on the plane alone, not on
    /// the others or the camera ([`slide_edge`] asks it of a lone plane).
    fn check(&self, planes: &[VanishingPlane], index: usize) -> PlaneCheck;

    /// The topmost plane under `at`.
    fn plane_at(&self, planes: &[VanishingPlane], at: [f64; 2]) -> Option<usize>;

    /// Plane `index`'s frame: the unit square onto the plane, and its sides' lengths in the
    /// scene (for the grid's spacing and the paste's extent).
    fn frame(&self, planes: &[VanishingPlane], index: usize) -> Option<(Homography, f64, f64)>;

    /// The stamp brush's ring of `diameter` at `at`, foreshortened on the plane there, in
    /// document points; `None` off every plane.
    fn stamp_ring(&self, planes: &[VanishingPlane], at: [f64; 2], diameter: f64) -> Option<Vec<Point>>;

    /// Where the clone source is for the brush at `at`, a stroke that began at `anchor` with
    /// `source` (PhotoCraft `vanishing::clone_source`).
    fn clone_source(&self, planes: &[VanishingPlane], source: [f64; 2], anchor: [f64; 2], at: [f64; 2]) -> Option<[f64; 2]>;
}

/// Fits torn plane `index`'s depth (or with `rotate` its angle, 1..=179 degrees) so the middle of
/// its far edge comes as near `at` as it can: a search over the values with
/// [`VanishingScene::resolve`]. `None` when nothing fits.
pub fn fit_torn(
    scene: &dyn VanishingScene,
    planes: &[VanishingPlane],
    torn: &[Option<TornPlane>],
    index: usize,
    rotate: bool,
    at: [f64; 2],
) -> Option<TornPlane> {
    // pc: crates/ui-egui/src/dialogs/vp_ui.rs:430 `fit_torn`. PhotoCraft rebuilds the scene by hand;
    // here `resolve` over the planes up to `index` (later planes cannot change this one).
    let t = torn.get(index).copied().flatten().filter(|t| t.from < index && index < planes.len())?;
    let upto = &planes[..=index];
    let mut chain: Vec<Option<TornPlane>> = (0..=index).map(|i| torn.get(i).copied().flatten()).collect();
    let edge = PlaneEdge::ALL.iter().position(|e| *e == t.edge)?;
    let mut miss = |angle: f64, depth: f64| -> f64 {
        chain[index] = Some(TornPlane { angle, depth, ..t });
        let Some(c) = scene.resolve(upto, &chain).get(index).map(|p| p.corners) else { return f64::MAX };
        if c.iter().flatten().all(|v| *v == 0.0) {
            return f64::MAX;
        }
        let (a, b) = (c[edge], c[(edge + 1) % 4]);
        let d = ((a[0] + b[0]) / 2.0 - at[0]).hypot((a[1] + b[1]) / 2.0 - at[1]);
        if d.is_finite() { d } else { f64::MAX }
    };
    let best = |values: &mut dyn Iterator<Item = f64>, miss: &mut dyn FnMut(f64) -> f64| {
        values.map(|v| (miss(v), v)).filter(|m| m.0 < f64::MAX).min_by(|a, b| a.0.total_cmp(&b.0)).map(|m| m.1)
    };
    if rotate {
        let coarse = best(&mut (1..180).map(f64::from), &mut |a| miss(a, t.depth))?;
        let fine = best(&mut (-10..=10).map(|k| coarse + f64::from(k) / 10.0), &mut |a| miss(a, t.depth)).unwrap_or(coarse);
        Some(TornPlane { angle: fine.clamp(1.0, 179.0), ..t })
    } else {
        let coarse = best(&mut (0..=160).map(|k| 0.02 * 1.06f64.powi(k)), &mut |d| miss(t.angle, d))?;
        let fine = best(&mut (-10..=10).map(|k| coarse * (1.0 + f64::from(k) * 0.006)), &mut |d| miss(t.angle, d)).unwrap_or(coarse);
        Some(TornPlane { depth: fine.clamp(0.02, 1000.0), ..t })
    }
}

/// The plane after dragging edge node `edge` to `at`: the edge slides along the plane's own
/// perspective as it was at the press (`at_press`, the plane's map then). `None` when part of the
/// plane would lie on or beyond its horizon, or when the scene's [`VanishingScene::check`] calls
/// the result [`PlaneCheck::Invalid`] (PhotoCraft's `slid_quad` asks `VpPlane::check`; the test is
/// the scene's, not a copy).
pub fn slide_edge(scene: &dyn VanishingScene, at_press: &Homography, edge: usize, at: [f64; 2]) -> Option<VanishingPlane> {
    // pc: crates/ui-egui/src/dialogs/vp_ui.rs:770 `slid_quad`, :718 `drag_to` (Edge).
    let inv = at_press.inverse()?;
    let p = inv.apply(Point::new(at[0], at[1]));
    let (u, v) = (p.x, p.y);
    // The plane's (u, v) rectangle after the move.
    let (u0, v0, u1, v1) = match edge {
        0 => (0.0, v.min(0.99), 1.0, 1.0),
        1 => (0.0, 0.0, u.max(0.01), 1.0),
        2 => (0.0, 0.0, 1.0, v.max(0.01)),
        _ => (u.min(0.99), 0.0, 1.0, 1.0),
    };
    // Part of the rectangle on or beyond the plane's horizon cannot follow the perspective.
    let [g, h, c] = at_press.row(2);
    let w = |u: f64, v: f64| g * u + h * v + c;
    let side = w(0.5, 0.5).signum();
    let uv = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
    if uv.iter().any(|q| w(q[0], q[1]) * side <= 1e-9 * w(0.5, 0.5).abs()) {
        return None;
    }
    let corners = uv.map(|q| {
        let at = at_press.apply(Point::new(q[0], q[1]));
        [at.x, at.y]
    });
    let slid = VanishingPlane { corners };
    (scene.check(&[slid], 0) != PlaneCheck::Invalid).then_some(slid)
}
