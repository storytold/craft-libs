//! Mesh shadings (types 4 to 7) as Coons patches in ink space, for [`Paint::Mesh`].
//!
//! Each source patch becomes a 4 × 4 control net in page space. A triangle is a Coons patch
//! whose fourth side is collapsed onto its third vertex, which shades exactly as the triangle
//! does; a Coons patch is the net its boundary implies (ISO 32000-2 §8.7.4.5.8); a tensor patch
//! is its own net. A net is halved in u and v (de Casteljau, exact for both kinds) until its
//! colors at the edge midpoints and the center are within [`TOLERANCE`] of the bilinear blend
//! of its converted corners, and its inner points are within [`NET_TOLERANCE`] of the ones its
//! boundary implies. Each piece is written as the Coons patch of its boundary.
//!
//! A piece still out at [`MAX_DEPTH`], or more than [`MAX_MESH_PATCHES`] pieces, gives `None`:
//! the caller reads the shading as an image.
//!
//! [`Paint::Mesh`]: super::Paint::Mesh

use craft_pdf::interpret::color::ColorComponents;
use craft_pdf::interpret::shading::{CoonsPatch, ShadingType, TensorProductPatch, Triangle};
use kurbo::{Affine, Point};

use super::shade::{self, STOP_TOLERANCE, StopInks};
use super::{Mesh, MeshPatch};

/// The most patches a mesh paint holds.
pub(super) const MAX_MESH_PATCHES: usize = 1 << 16;

/// How far a converted color may be from the blend of its piece's corners, per ink.
const TOLERANCE: f32 = STOP_TOLERANCE;

/// How far, in points, a tensor net's inner points may be from a Coons patch's.
const NET_TOLERANCE: f64 = 0.01;

/// The most times a source patch is halved in each direction.
const MAX_DEPTH: u8 = 8;

/// Control points `net[i][j]`: `i` steps along u, `j` along v.
type Net = [[Point; 4]; 4];

/// Corners in Coons order: (u, v) = (0, 0), (0, 1), (1, 1), (1, 0).
type Corners = [Vec<f32>; 4];

struct Piece {
    net: Net,
    source: Corners,
    ink: Corners,
    depth: u8,
}

/// The mesh of a type 4 to 7 shading under `matrix` (shading space to page space). `convert`
/// maps source components (after the shading's function, if any) to ink values; `kind` says
/// what the values mean. `None` for other types and when the mesh must stay an image.
pub(super) fn mesh(shading: &ShadingType, matrix: Affine, kind: &StopInks, convert: &mut dyn FnMut(&[f32]) -> Option<Vec<f32>>) -> Option<Mesh> {
    let sources = source_nets(shading, matrix)?;
    let mut patches = Vec::new();
    let mut pending = sources.len();
    for (net, source) in sources {
        pending = pending.saturating_sub(1);
        let ink = convert_all(&source, convert)?;
        let mut stack = vec![Piece { net, source, ink, depth: 0 }];
        while let Some(piece) = stack.pop() {
            match refine(piece, convert)? {
                Refined::Done(piece) => patches.push(coons(&piece, kind)),
                Refined::Split(children) => {
                    if patches.len().saturating_add(stack.len()).saturating_add(pending).saturating_add(children.len()) > MAX_MESH_PATCHES {
                        return None;
                    }
                    // Popped in order: (u low, v low) first.
                    stack.extend(children.into_iter().rev());
                }
            }
        }
    }
    (!patches.is_empty()).then_some(Mesh { patches })
}

/// Every source patch as a page-space net with its corner components; `None` past
/// [`MAX_MESH_PATCHES`], for a point that isn't finite, or for other shading types.
fn source_nets(shading: &ShadingType, matrix: Affine) -> Option<Vec<(Net, Corners)>> {
    let nets = page_nets(shading, matrix)?;
    nets.iter().all(|(net, _)| net.iter().flatten().all(|p| p.is_finite())).then_some(nets)
}

fn page_nets(shading: &ShadingType, matrix: Affine) -> Option<Vec<(Net, Corners)>> {
    let to_page = |net: Net| net.map(|row| row.map(|p| matrix * p));
    let fits = |n: usize| (n <= MAX_MESH_PATCHES).then_some(());
    match shading {
        ShadingType::TriangleMesh { triangles, .. } => {
            fits(triangles.len())?;
            Some(triangles.iter().map(|t| (to_page(triangle_net(t)), triangle_corners(t))).collect())
        }
        ShadingType::CoonsPatchMesh { patches, .. } => {
            fits(patches.len())?;
            Some(patches.iter().map(|p| (to_page(coons_net(p)), corners(&p.colors))).collect())
        }
        ShadingType::TensorProductPatchMesh { patches, .. } => {
            fits(patches.len())?;
            Some(patches.iter().map(|p| (to_page(tensor_net(p)), corners(&p.colors))).collect())
        }
        ShadingType::FunctionBased { .. } | ShadingType::RadialAxial { .. } | ShadingType::Dummy => None,
    }
}

fn corners(colors: &[ColorComponents; 4]) -> Corners {
    colors.clone().map(|c| c.to_vec())
}

/// Triangle ABC as the Coons patch A → B → C → C → A: `S(u, v) = (1−u)(1−v)A + (1−u)vB + uC`,
/// with its colors blended by the same weights.
fn triangle_net(t: &Triangle) -> Net {
    let (a, b, c) = (t.p0.point, t.p1.point, t.p2.point);
    let third = |p: Point, q: Point, k: f64| p.lerp(q, k / 3.0);
    let boundary = [a, third(a, b, 1.0), third(a, b, 2.0), b, third(b, c, 1.0), third(b, c, 2.0), c, c, c, c, third(c, a, 1.0), third(c, a, 2.0)];
    implied_net(&boundary)
}

fn triangle_corners(t: &Triangle) -> Corners {
    let c = t.p2.colors.to_vec();
    [t.p0.colors.to_vec(), t.p1.colors.to_vec(), c.clone(), c]
}

fn coons_net(p: &CoonsPatch) -> Net {
    implied_net(&p.control_points)
}

/// The net of a tensor patch's points, in the order of ISO 32000-2 §8.7.4.5.8.
fn tensor_net(p: &TensorProductPatch) -> Net {
    const ORDER: [[usize; 4]; 4] = [[0, 1, 2, 3], [11, 12, 13, 4], [10, 15, 14, 5], [9, 8, 7, 6]];
    let cp = &p.control_points;
    ORDER.map(|row| row.map(|k| cp.get(k).copied().unwrap_or(Point::ORIGIN)))
}

/// The 12 boundary points of a Coons patch, from the u = 0 side's start, as a net.
const BOUNDARY: [(usize, usize); 12] = [(0, 0), (0, 1), (0, 2), (0, 3), (1, 3), (2, 3), (3, 3), (3, 2), (3, 1), (3, 0), (2, 0), (1, 0)];

/// The net of the Coons patch with `boundary`: its inner points are the ones the boundary implies.
fn implied_net(boundary: &[Point; 12]) -> Net {
    let mut net = [[Point::ORIGIN; 4]; 4];
    for (&(i, j), p) in BOUNDARY.iter().zip(boundary) {
        net[i][j] = *p;
    }
    for (i, j) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
        net[i][j] = implied(&net, i, j);
    }
    net
}

/// Inner point `(i, j)` of the Coons patch on `net`'s boundary: the two ruled surfaces less the
/// bilinear one.
fn implied(net: &Net, i: usize, j: usize) -> Point {
    let (s, t) = (i as f64 / 3.0, j as f64 / 3.0);
    let r1 = net[i][0].to_vec2() * (1.0 - t) + net[i][3].to_vec2() * t;
    let r2 = net[0][j].to_vec2() * (1.0 - s) + net[3][j].to_vec2() * s;
    let bl = net[0][0].to_vec2() * ((1.0 - s) * (1.0 - t))
        + net[0][3].to_vec2() * ((1.0 - s) * t)
        + net[3][0].to_vec2() * (s * (1.0 - t))
        + net[3][3].to_vec2() * (s * t);
    (r1 + r2 - bl).to_point()
}

/// How far `net`'s inner points are from a Coons patch's.
fn net_deviation(net: &Net) -> f64 {
    [(1, 1), (1, 2), (2, 1), (2, 2)].iter().map(|&(i, j)| net[i][j].distance(implied(net, i, j))).fold(0.0, f64::max)
}

enum Refined {
    Done(Box<Piece>),
    Split(Vec<Piece>),
}

/// `piece` as it is, or its four halves; `None` when it must be split past [`MAX_DEPTH`] or a
/// conversion fails.
fn refine(piece: Piece, convert: &mut dyn FnMut(&[f32]) -> Option<Vec<f32>>) -> Option<Refined> {
    let [c0, c1, c2, c3] = &piece.source;
    // Edge midpoints (u = 0, v = 1, u = 1, v = 0) and the center, in source components.
    let source = [blend(&[c0, c1]), blend(&[c1, c2]), blend(&[c2, c3]), blend(&[c3, c0]), blend(&[c0, c1, c2, c3])];
    let ink = convert_all(&source, convert)?;
    let [i0, i1, i2, i3] = &piece.ink;
    let expected = [blend(&[i0, i1]), blend(&[i1, i2]), blend(&[i2, i3]), blend(&[i3, i0]), blend(&[i0, i1, i2, i3])];
    let mut off = 0.0_f32;
    for (got, want) in ink.iter().zip(&expected) {
        if got.len() != want.len() {
            return None;
        }
        off = got.iter().zip(want).map(|(g, w)| (g - w).abs()).fold(off, f32::max);
    }
    let within = off <= TOLERANCE && net_deviation(&piece.net) <= NET_TOLERANCE;
    if within {
        return Some(Refined::Done(Box::new(piece)));
    }
    if piece.depth >= MAX_DEPTH {
        return None;
    }
    Some(Refined::Split(halves(piece, source, ink)))
}

/// The four quarters of `piece`, with the midpoint and center samples as their corners.
fn halves(piece: Piece, source: [Vec<f32>; 5], ink: [Vec<f32>; 5]) -> Vec<Piece> {
    let Piece { net, source: [s0, s1, s2, s3], ink: [i0, i1, i2, i3], depth } = piece;
    let [s01, s12, s23, s30, sc] = source;
    let [i01, i12, i23, i30, ic] = ink;
    let (lo, hi) = split_u(&net);
    let ((lo_lo, lo_hi), (hi_lo, hi_hi)) = (split_v(&lo), split_v(&hi));
    let depth = depth.saturating_add(1);
    let piece = |net, source, ink| Piece { net, source, ink, depth };
    vec![
        piece(lo_lo, [s0, s01.clone(), sc.clone(), s30.clone()], [i0, i01.clone(), ic.clone(), i30.clone()]),
        piece(lo_hi, [s01, s1, s12.clone(), sc.clone()], [i01, i1, i12.clone(), ic.clone()]),
        piece(hi_hi, [sc.clone(), s12, s2, s23.clone()], [ic.clone(), i12, i2, i23.clone()]),
        piece(hi_lo, [s30, sc, s23, s3], [i30, ic, i23, i3]),
    ]
}

/// Every one of `source` converted; `None` when one fails or gives a value that isn't finite.
fn convert_all<const N: usize>(source: &[Vec<f32>; N], convert: &mut dyn FnMut(&[f32]) -> Option<Vec<f32>>) -> Option<[Vec<f32>; N]> {
    let mut out: [Vec<f32>; N] = std::array::from_fn(|_| Vec::new());
    for (o, s) in out.iter_mut().zip(source) {
        *o = convert(s).filter(|v| v.iter().all(|x| x.is_finite()))?;
    }
    Some(out)
}

/// The mean of `values`, component by component; missing components read as 0.
fn blend(values: &[&Vec<f32>]) -> Vec<f32> {
    let len = values.iter().map(|v| v.len()).max().unwrap_or(0);
    let n = values.len().max(1) as f32;
    (0..len).map(|k| values.iter().map(|v| v.get(k).copied().unwrap_or(0.0)).sum::<f32>() / n).collect()
}

/// A cubic Bézier halved at t = 0.5.
fn half(c: [Point; 4]) -> ([Point; 4], [Point; 4]) {
    let (p01, p12, p23) = (c[0].midpoint(c[1]), c[1].midpoint(c[2]), c[2].midpoint(c[3]));
    let (p012, p123) = (p01.midpoint(p12), p12.midpoint(p23));
    let mid = p012.midpoint(p123);
    ([c[0], p01, p012, mid], [mid, p123, p23, c[3]])
}

fn split_u(net: &Net) -> (Net, Net) {
    let (mut lo, mut hi) = (*net, *net);
    for j in 0..4 {
        let (a, b) = half([net[0][j], net[1][j], net[2][j], net[3][j]]);
        for i in 0..4 {
            lo[i][j] = a[i];
            hi[i][j] = b[i];
        }
    }
    (lo, hi)
}

fn split_v(net: &Net) -> (Net, Net) {
    let (mut lo, mut hi) = (*net, *net);
    for i in 0..4 {
        (lo[i], hi[i]) = half(net[i]);
    }
    (lo, hi)
}

/// `piece` as the Coons patch of its boundary, its corners as ink colors.
fn coons(piece: &Piece, kind: &StopInks) -> MeshPatch {
    MeshPatch { points: BOUNDARY.map(|(i, j)| piece.net[i][j]), colors: piece.ink.clone().map(|v| shade::stop_color(kind, &v)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink::{InkColor, InkId, Process};

    fn square(size: f64) -> [Point; 12] {
        BOUNDARY.map(|(i, j)| Point::new(i as f64 * size / 3.0, j as f64 * size / 3.0))
    }

    #[test]
    fn a_coons_net_halves_into_coons_nets() {
        let mut boundary = square(90.0);
        boundary[1] = Point::new(-20.0, 40.0);
        boundary[5] = Point::new(70.0, 110.0);
        boundary[10] = Point::new(50.0, -30.0);
        let net = implied_net(&boundary);
        let (lo, hi) = split_u(&net);
        // They meet where the patch's v = 0 side is at u = 1/2.
        let side = kurbo::CubicBez::new(net[0][0], net[1][0], net[2][0], net[3][0]);
        assert!(lo[3][0].distance(kurbo::ParamCurve::eval(&side, 0.5)) < 1e-9 && lo[3][0] == hi[0][0]);
        for n in [lo, hi, split_v(&lo).0, split_v(&hi).1] {
            assert!(net_deviation(&n) < 1e-9, "{}", net_deviation(&n));
        }
    }

    #[test]
    fn a_curved_tensor_net_is_not_a_coons_net() {
        let mut net = implied_net(&square(90.0));
        net[1][1] += kurbo::Vec2::new(5.0, 0.0);
        assert!(net_deviation(&net) > 4.0);
    }

    fn patches(n: usize, colors: [f32; 4]) -> ShadingType {
        let patch = CoonsPatch { control_points: square(10.0), colors: colors.map(|c| ColorComponents::from_slice(&[c])) };
        ShadingType::CoonsPatchMesh { patches: vec![patch; n], function: None }
    }

    fn values(c: &InkColor) -> Vec<f32> {
        match c {
            InkColor::Tints(t) => t.iter().map(|(_, v)| *v).collect(),
            _ => Vec::new(),
        }
    }

    const BLACK: [InkId; 1] = [InkId::process(Process::Black)];

    #[test]
    fn a_linear_conversion_keeps_every_patch_whole() {
        let kind = StopInks::Inks(BLACK.to_vec());
        let found = mesh(&patches(3, [0.0, 0.2, 0.9, 1.0]), Affine::IDENTITY, &kind, &mut |s| Some(vec![s[0] * 0.5])).unwrap();
        assert_eq!(found.patches.len(), 3);
        assert_eq!(values(&found.patches[0].colors[2]), [0.45]);
    }

    #[test]
    fn a_bending_conversion_splits_until_each_piece_is_within_tolerance() {
        let kind = StopInks::Inks(BLACK.to_vec());
        let square_of = |s: &[f32]| Some(vec![s[0] * s[0]]);
        let found = mesh(&patches(1, [0.0, 1.0, 1.0, 0.0]), Affine::IDENTITY, &kind, &mut |s| square_of(s)).unwrap();
        assert!(found.patches.len() > 4, "{}", found.patches.len());
        for patch in &found.patches {
            // Colors run along v only (u = 0 and u = 1 match), so v is the corner color's root.
            let [c0, c1, ..] = patch.colors.each_ref().map(values);
            let mid = ((c0[0].sqrt() + c1[0].sqrt()) / 2.0).powi(2);
            assert!((mid - (c0[0] + c1[0]) / 2.0).abs() <= TOLERANCE + 1e-6, "{c0:?} {c1:?}");
        }
    }

    #[test]
    fn meshes_past_the_patch_limit_or_the_depth_are_left_to_the_image_path() {
        let kind = StopInks::Inks(BLACK.to_vec());
        let linear = &mut |s: &[f32]| Some(vec![s[0]]);
        assert!(mesh(&patches(MAX_MESH_PATCHES, [0.0; 4]), Affine::IDENTITY, &kind, linear).is_some());
        assert!(mesh(&patches(MAX_MESH_PATCHES + 1, [0.0; 4]), Affine::IDENTITY, &kind, linear).is_none());
        let step = &mut |s: &[f32]| Some(vec![if s[0] < 0.3 { 0.0 } else { 1.0 }]);
        assert!(mesh(&patches(1, [0.0, 1.0, 1.0, 0.0]), Affine::IDENTITY, &kind, step).is_none());
        let failing = &mut |_: &[f32]| None;
        assert!(mesh(&patches(1, [0.0; 4]), Affine::IDENTITY, &kind, failing).is_none());
        // One patch failing is the whole mesh failing.
        let ShadingType::CoonsPatchMesh { patches: mut two, .. } = patches(2, [0.0; 4]) else { panic!() };
        two[1].colors = [0.9; 4].map(|c| ColorComponents::from_slice(&[c]));
        let partly = &mut |s: &[f32]| (s[0] < 0.5).then(|| vec![s[0]]);
        assert!(mesh(&ShadingType::CoonsPatchMesh { patches: two, function: None }, Affine::IDENTITY, &kind, partly).is_none());
    }

    #[test]
    fn values_and_points_that_arent_finite_are_left_to_the_image_path() {
        let kind = StopInks::Inks(BLACK.to_vec());
        let nan_in_the_middle = &mut |s: &[f32]| Some(vec![if s[0] == 0.5 { f32::NAN } else { s[0] }]);
        assert!(mesh(&patches(1, [0.0, 1.0, 1.0, 0.0]), Affine::IDENTITY, &kind, nan_in_the_middle).is_none());
        let ShadingType::CoonsPatchMesh { patches: mut one, .. } = patches(1, [0.0; 4]) else { panic!() };
        one[0].control_points[4].x = f64::NAN;
        let linear = &mut |s: &[f32]| Some(vec![s[0]]);
        assert!(mesh(&ShadingType::CoonsPatchMesh { patches: one, function: None }, Affine::IDENTITY, &kind, linear).is_none());
    }

    /// A tensor patch on a 90 pt square whose (1, 1) point is pulled in by `pull`.
    fn tensor(pull: f64) -> TensorProductPatch {
        let mut net = implied_net(&square(90.0));
        net[1][1] += kurbo::Vec2::new(pull, pull / 2.0);
        // Stream order (ISO 32000-2 §8.7.4.5.8): the boundary, then p11, p12, p22, p21.
        let inner = [(1, 1), (1, 2), (2, 2), (2, 1)];
        let order = BOUNDARY.iter().chain(&inner);
        let mut control_points = [Point::ORIGIN; 16];
        for (p, &(i, j)) in control_points.iter_mut().zip(order) {
            *p = net[i][j];
        }
        TensorProductPatch { control_points, colors: [0.0, 0.3, 1.0, 0.6].map(|c| ColorComponents::from_slice(&[c])) }
    }

    #[test]
    fn a_tensor_patch_is_read_as_the_surface_its_points_give() {
        let patch = tensor(25.0);
        let net = tensor_net(&patch);
        for (u, v) in [(0.25, 0.25), (0.5, 0.1), (0.8, 0.6)] {
            let rows = net.map(|row| kurbo::ParamCurve::eval(&kurbo::CubicBez::new(row[0], row[1], row[2], row[3]), v));
            let got = kurbo::ParamCurve::eval(&kurbo::CubicBez::new(rows[0], rows[1], rows[2], rows[3]), u);
            let want = patch.map_coordinate(Point::new(u, v));
            assert!(got.distance(want) < 1e-9, "{got:?} {want:?}");
        }
    }

    #[test]
    fn a_curved_tensor_patch_splits_until_each_piece_is_a_coons_patch() {
        let kind = StopInks::Inks(BLACK.to_vec());
        let linear = &mut |s: &[f32]| Some(vec![s[0]]);
        let patch = tensor(25.0);
        let net = tensor_net(&patch);
        let found =
            mesh(&ShadingType::TensorProductPatchMesh { patches: vec![patch.clone()], function: None }, Affine::IDENTITY, &kind, linear).unwrap();
        // The same refinement, keeping each piece's net.
        let (mut stack, mut done) = (vec![Piece { net, source: corners(&patch.colors), ink: corners(&patch.colors), depth: 0 }], Vec::new());
        while let Some(piece) = stack.pop() {
            match refine(piece, linear).unwrap() {
                Refined::Done(piece) => done.push(piece),
                Refined::Split(children) => stack.extend(children),
            }
        }
        assert!(done.len() > 4 && done.len() == found.patches.len(), "{} {}", done.len(), found.patches.len());
        for piece in &done {
            assert!(net_deviation(&piece.net) <= NET_TOLERANCE, "{}", net_deviation(&piece.net));
            // Each piece's corner is on the source surface.
            let corner = piece.net[0][0];
            let on = (0..=256).flat_map(|a| (0..=256).map(move |b| (a as f64 / 256.0, b as f64 / 256.0)));
            assert!(on.map(|(u, v)| patch.map_coordinate(Point::new(u, v)).distance(corner)).fold(f64::MAX, f64::min) < 1e-6);
        }
        // A flat tensor patch is one Coons patch.
        let flat = ShadingType::TensorProductPatchMesh { patches: vec![tensor(0.0)], function: None };
        assert_eq!(mesh(&flat, Affine::IDENTITY, &kind, linear).unwrap().patches.len(), 1);
    }

    #[test]
    fn the_implied_inner_point_is_the_one_the_standard_gives() {
        let mut boundary = square(30.0);
        boundary[2] = Point::new(3.0, 25.0);
        boundary[7] = Point::new(33.0, 18.0);
        let net = implied_net(&boundary);
        let g = |i: usize, j: usize| net[i][j].to_vec2();
        let p11 = (g(0, 0) * -4.0 + (g(0, 1) + g(1, 0)) * 6.0 - (g(0, 3) + g(3, 0)) * 2.0 + (g(3, 1) + g(1, 3)) * 3.0 - g(3, 3)) / 9.0;
        let p12 = (g(0, 3) * -4.0 + (g(0, 2) + g(1, 3)) * 6.0 - (g(0, 0) + g(3, 3)) * 2.0 + (g(3, 2) + g(1, 0)) * 3.0 - g(3, 0)) / 9.0;
        let p21 = (g(3, 0) * -4.0 + (g(3, 1) + g(2, 0)) * 6.0 - (g(3, 3) + g(0, 0)) * 2.0 + (g(0, 1) + g(2, 3)) * 3.0 - g(0, 3)) / 9.0;
        let p22 = (g(3, 3) * -4.0 + (g(3, 2) + g(2, 3)) * 6.0 - (g(3, 0) + g(0, 3)) * 2.0 + (g(0, 2) + g(2, 0)) * 3.0 - g(0, 0)) / 9.0;
        for (got, want) in [(net[1][1], p11), (net[1][2], p12), (net[2][1], p21), (net[2][2], p22)] {
            assert!(got.distance(want.to_point()) < 1e-9, "{got:?} {want:?}");
        }
    }
}
