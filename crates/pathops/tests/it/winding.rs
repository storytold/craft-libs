//! The winding number of the curves the planar map reads what covers a face from: `kurbo`'s `BezPath::winding` tells
//! a point wrongly for a cubic whose handles are level (an arch, as a circle cut at its top and bottom is made of: the
//! cubic term of its height is rounding noise, and the cubic formula puts the crossing of a height where it is not), and
//! the six places the crate wound a path against a point ([`Region::contains`], the groups of a Merge, the sources of
//! a face, the holes of a face, the point inside a face, the point deep inside an offset's face) took its answer.
//! Each test is the smallest input found for one of them that fails with it.

use craft_pathops::*;
use kurbo::{BezPath, Point, Shape as _};

use crate::common::rectangle;

const NZ: FillRule = FillRule::NonZero;

/// The arch from `(0, y0)` to `(len, y0)` through handles at `(len fx1, h)` and `(len fx2, h)`: level handles, and the
/// ends at one height.
fn arch_curve(len: f64, h: f64, fx1: f64, fx2: f64, y0: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.0, y0));
    p.curve_to((len * fx1, h), (len * fx2, h), (len, y0));
    p
}

/// The arch closed by its chord.
fn closed_arch(len: f64, h: f64, fx1: f64, fx2: f64, y0: f64) -> BezPath {
    let mut p = arch_curve(len, h, fx1, fx2, y0);
    p.close_path();
    p
}

/// The area under the arch, down to a line 50 below its ends: the shape the lenses are made of.
fn under_arch(len: f64, h: f64, fx1: f64, fx2: f64) -> BezPath {
    let mut p = arch_curve(len, h, fx1, fx2, 0.0);
    p.line_to((len, -50.0));
    p.line_to((0.0, -50.0));
    p.close_path();
    p
}

/// A circle of `n` cubic arcs (the first from angle 0), each the cubic that fits its arc: for an even number the arcs at
/// the top and the bottom are level.
fn circle_of_arcs(n: usize, r: f64) -> BezPath {
    let step = std::f64::consts::TAU / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let at = |a: f64| Point::new(r * a.cos(), r * a.sin());
    let along = |a: f64| kurbo::Vec2::new(-r * a.sin(), r * a.cos());
    let mut bp = BezPath::new();
    bp.move_to(at(0.0));
    for i in 0..n {
        let (a0, a1) = (step * i as f64, step * (i + 1) as f64);
        let (p0, p3) = (at(a0), if i + 1 == n { at(0.0) } else { at(a1) });
        bp.curve_to(p0 + along(a0) * k, p3 - along(a1) * k, p3);
    }
    bp.close_path();
    bp
}

/// The faces of `live_paint` as their areas and sources, the smallest first.
fn faces_by_area(shapes: &[Shape]) -> Vec<(f64, Vec<usize>)> {
    let mut v: Vec<(f64, Vec<usize>)> = live_paint(shapes).0.iter().map(|f| (f.path.area().abs(), f.sources.clone())).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v
}

// ---- Region::contains

/// The fault in its plainest form: an arch of handles at 10.05 (closed by its chord) is `y = 3h u (1 - u)` for
/// `u = x / 100`, and a point is in it where it is over the chord and under that. `kurbo` told 1,440 of the 16,000 points
/// of a grid wrongly (the point (2.929, 2.385), 0.86 above the arch there, was inside) and the others for handles at
/// 10.0, 9.0 and 7.0 right. [`Region::contains`] is the winding number of its path.
#[test]
fn a_region_of_level_handles_contains_what_is_under_its_arch() {
    for h in [10.05, 10.0, 9.0, 7.0, 10.1, 3.3] {
        let region = Region { path: closed_arch(100.0, h, 1.0 / 3.0, 2.0 / 3.0, 0.0), sources: vec![0] };
        let (mut wrong, mut tested) = (0, 0);
        for i in 0..160 {
            for j in 0..100 {
                let p = Point::new(-5.0 + 110.0 * (f64::from(i) + 0.5) / 160.0, -1.0 + (3.0 * h + 2.0) * (f64::from(j) + 0.5) / 100.0);
                let top = 3.0 * h * (p.x / 100.0) * (1.0 - p.x / 100.0);
                if (p.y - top).abs() < 1e-6 || p.y.abs() < 1e-6 {
                    continue;
                }
                let inside = p.x > 0.0 && p.x < 100.0 && p.y > 0.0 && p.y < top;
                tested += 1;
                wrong += usize::from(region.contains(p) != inside);
            }
        }
        assert!(tested > 15_000);
        assert_eq!(wrong, 0, "h = {h}: {wrong} of {tested} points are told wrongly");
    }
    let region = Region { path: closed_arch(100.0, 10.05, 1.0 / 3.0, 2.0 / 3.0, 0.0), sources: Vec::new() };
    assert!(!region.contains(Point::new(2.929, 2.385)));
}

// ---- the sources of a face (Fill::contains)

/// Two shapes under arches from the same two points, the upper a few hundredths above the lower: the lens between them is
/// covered by the upper shape alone, and the rest by both. The lens was listed under both shapes, under the lower, and
/// under none, for the arches below (`live_paint` and `shape_builder` took each face's sources from the winding of a
/// point in it against each shape).
#[test]
fn a_lens_between_two_arches_is_covered_by_the_upper_shape_alone() {
    // (length, height of the lower, how much higher the upper is, where the handles are).
    for (len, h, d, fx) in [
        (100.0, 2.0, 0.03, 0.2),
        (100.0, 2.0, 0.06, 0.2),
        (100.0, 2.0, 0.06, 0.3),
        (100.0, 2.0, 0.3, 1.0 / 3.0),
        (100.0, 5.0, 0.06, 0.3),
        (100.0, 5.0, 0.1, 1.0 / 3.0),
        (100.0, 5.0, 0.1, 0.2),
        (100.0, 10.05, 0.06, 1.0 / 3.0),
        (100.0, 10.05, 0.06, 0.2),
        (100.0, 10.05, 0.1, 1.0 / 3.0),
        (100.0, 10.05, 0.1, 0.2),
    ] {
        let lower = under_arch(len, h, fx, 1.0 - fx);
        let upper = under_arch(len, h + d, fx, 1.0 - fx);
        let shapes = [Shape::new(lower, NZ, 0), Shape::new(upper, NZ, 1)];
        let faces = faces_by_area(&shapes);
        let label = format!("length {len}, height {h}, lens {d} high, handles at {fx}: {faces:?}");
        assert_eq!(faces.len(), 2, "{label}");
        assert_eq!(faces[0].1, [1], "the lens: {label}");
        assert_eq!(faces[1].1, [0, 1], "under both: {label}");
        // The regions of the filled areas: the same two, and none without a source.
        let built = shape_builder(&shapes, false);
        let mut regions: Vec<(f64, Vec<usize>)> = built.regions.iter().map(|r| (r.path.area().abs(), r.sources.clone())).collect();
        regions.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(regions.iter().map(|r| r.1.clone()).collect::<Vec<_>>(), [vec![1], vec![0, 1]], "shape_builder: {label}: {regions:?}");
    }
}

/// Two concentric circles cut into arcs (6 and 10 of them, so that the arcs at the top and bottom are level), the second
/// smaller by a sliver: the ring between them is covered by the first shape alone and the disc inside by both. The ring was
/// listed under the second shape, or under both.
#[test]
fn the_ring_between_two_circles_cut_into_arcs_is_covered_by_the_outer_one_alone() {
    // (arcs of the outer circle, arcs of the inner, radius, the inner smaller by this share of it in percent).
    for (outer, inner, r, gap) in [
        (6, 6, 100.0, 1.0),
        (6, 6, 37.3, 0.3),
        (6, 6, 37.3, 0.05),
        (6, 6, 100.0, 0.03),
        (10, 6, 100.0, 1.0),
        (10, 6, 37.3, 0.3),
        (10, 6, 37.3, 0.05),
        (10, 6, 100.0, 0.03),
        (6, 10, 37.3, 0.03),
        (6, 10, 100.0, 0.02),
        (6, 12, 100.0, 0.05),
        (6, 14, 100.0, 0.1),
    ] {
        let shapes = [Shape::new(circle_of_arcs(outer, r), NZ, 0), Shape::new(circle_of_arcs(inner, r * (1.0 - gap / 100.0)), NZ, 1)];
        let faces = faces_by_area(&shapes);
        let label = format!("{outer} arcs of radius {r} and {inner} of {gap}% less: {faces:?}");
        assert_eq!(faces.len(), 2, "{label}");
        assert_eq!(faces[0].1, [0], "the ring: {label}");
        assert_eq!(faces[1].1, [0, 1], "the disc: {label}");
    }
}

// ---- the holes of a face (place_holes)

/// A small square inside an arch whose handles are level: the arch's face has the square for a hole, whose first point was
/// wound against the face to place it (and the square's face is covered by both).
#[test]
fn a_hole_in_the_face_under_an_arch_is_placed_in_it() {
    let arch = closed_arch(100.0, 20.1, 1.0 / 3.0, 2.0 / 3.0, 0.0);
    let area = arch.area().abs();
    let square = rectangle(kurbo::Rect::new(15.455, 7.024, 15.855, 7.424));
    let faces = faces_by_area(&[Shape::new(arch, NZ, 0), Shape::new(square, NZ, 1)]);
    assert_eq!(faces.len(), 2, "{faces:?}");
    assert!((faces[0].0 - 0.16).abs() < 1e-9 && faces[0].1 == [0, 1], "the square: {faces:?}");
    assert!((faces[1].0 - (area - 0.16)).abs() < 1e-9 && faces[1].1 == [0], "the arch less the square, as its face with a hole: {faces:?}");
}

// ---- the groups of a Merge (face_groups_by_winding)

/// Two squares inside an arch that touch at a corner, one shape of one key: the sweep's tree of contours is wrong for the
/// two holes (the second comes out an island in the first), and the holes are grouped under the contour that winds
/// round a point of each. They were faces of their own, and the arch's face had none.
#[test]
fn holes_that_touch_inside_an_arch_stay_in_its_face() {
    let arch = closed_arch(100.0, 10.05, 1.0 / 3.0, 2.0 / 3.0, 0.0);
    let area = arch.area().abs();
    let side = 0.4;
    let (x, y) = (20.155, 3.519);
    let first = rectangle(kurbo::Rect::new(x, y, x + side, y + side));
    let second = rectangle(kurbo::Rect::new(x + side, y + side, x + 2.0 * side, y + 2.0 * side));
    let shapes = [Shape::new(arch, NZ, 0), Shape::new(first, NZ, 0), Shape::new(second, NZ, 0)];
    let mut got: Vec<(Vec<usize>, f64)> = regions(&shapes).iter().map(|r| (r.sources.clone(), r.path.area().abs())).collect();
    got.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let want = [(vec![0], area - 2.0 * side * side), (vec![0, 1], side * side), (vec![0, 2], side * side)];
    assert_eq!(got.len(), 3, "{got:?}");
    for (g, w) in got.iter().zip(&want) {
        assert!(g.0 == w.0 && (g.1 - w.1).abs() < 1e-9, "{got:?} against {want:?}");
    }
}

// ---- the point inside a face (interior_point)

/// The crescent between two arches a few hundredths apart: a point is looked for in the middle of the widest span of its
/// flattening, and tested against the winding of the path itself. Where that was wrong the point taken was outside it
/// (the crescent's two arches are `y = 3h u (1 - u)` and `y = 3 (h + 0.05) u (1 - u)`, for handles at a third of its length).
#[test]
fn the_point_found_in_a_crescent_is_in_it() {
    let mut found = 0;
    for (h, y0) in [(10.05, 0.0), (10.05, 1.5), (10.05, -3.0), (20.1, 0.0), (55.5, 0.0), (3.3, 0.0), (10.0, 0.0)] {
        let mut crescent = BezPath::new();
        crescent.move_to((0.0, y0));
        crescent.curve_to((100.0 / 3.0, y0 + h + 0.05), (200.0 / 3.0, y0 + h + 0.05), (100.0, y0));
        crescent.curve_to((200.0 / 3.0, y0 + h), (100.0 / 3.0, y0 + h), (0.0, y0));
        crescent.close_path();
        let Some(p) = interior_point(&crescent) else { continue };
        let u = p.x / 100.0;
        let (low, high) = (y0 + 3.0 * h * u * (1.0 - u), y0 + 3.0 * (h + 0.05) * u * (1.0 - u));
        // The upper arch's x is not 100 t when its handles are not a third apart, but they are here: u is x / 100.
        assert!(p.y > low && p.y < high, "h = {h}, y0 = {y0}: {p:?} is not between the arches ({low} and {high})");
        found += 1;
    }
    assert!(found >= 5, "a point was found in {found} of the 7");
}
