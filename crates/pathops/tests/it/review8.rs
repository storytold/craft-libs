//! Claims made in the eighth review of the pull request, each as the smallest test that fails when the
//! claim is true.

use craft_pathops::*;
use kurbo::{BezPath, ParamCurveArclen, Point, Rect, Shape as _};

use crate::common::{poly, rectangle};
use crate::review4::deviation;

const NZ: FillRule = FillRule::NonZero;

fn length(path: &BezPath) -> f64 {
    path.segments().map(|s| s.arclen(1e-9)).sum()
}

/// The default limits with `set` applied.
fn limits(set: impl FnOnce(&mut Limits)) -> Limits {
    let mut l = Limits::default();
    set(&mut l);
    l
}

// ---- the frame of a planar map

/// Eleven lines 9e-6 long, one level in each of the ten shears and one in that of 3/8, bar every shear
/// from 1/1024 to 1/2 (each the shears within about 0.12 of its own), and there was no map: Outline of
/// them and a line 100 long beside them gave nothing. The shears below 0 are free, and one of them is
/// the frame.
#[test]
fn a_map_is_made_in_a_negative_shear_when_the_positive_ones_are_barred() {
    let ks = [1.0 / 32.0, 1.0 / 64.0, 1.0 / 16.0, 1.0 / 128.0, 1.0 / 8.0, 1.0 / 256.0, 1.0 / 512.0, 1.0 / 4.0, 1.0 / 1024.0, 1.0 / 2.0, 3.0 / 8.0];
    let mut shapes: Vec<Shape> = ks
        .iter()
        .enumerate()
        .map(|(i, k)| Shape::new(poly(&[(0.0, 100.0 * i as f64), (9e-6, 100.0 * i as f64 - 9e-6 * k)], false), NZ, i as u64))
        .collect();
    let long = poly(&[(-500.0, -500.0), (-400.0, -450.0)], false);
    shapes.push(Shape::new(long.clone(), NZ, 99));
    let edges = pathfinder(PathfinderOp::Outline, &shapes);
    let kept: f64 = edges.iter().filter(|e| e.key == 99).map(|e| length(&e.path)).sum();
    assert!((kept - length(&long)).abs() < 1e-6, "the long line's outline: {kept} of {}", length(&long));
}

// ---- a map that is not made, told to the caller

/// Rectangles nested in one shape, with a line across them, are more than a fill limit of 1,000 lets
/// the planar map wind: `shape_builder` and Divide give the filled areas instead, unsplit by the line,
/// and `try_shape_builder` and `try_pathfinder` say so. With room, the two give what the others do.
#[test]
fn a_planar_map_past_its_limit_is_an_error_to_the_try_forms() {
    let mut nested = BezPath::new();
    for j in 0..20 {
        let d = 2.0 * f64::from(j);
        nested.extend(rectangle(Rect::new(d, d, 100.0 - d, 100.0 - d)).iter());
    }
    let shapes = [Shape::new(nested, NZ, 0), Shape::new(poly(&[(-10.0, 50.0), (110.0, 50.0)], false), NZ, 1)];
    let few = limits(|l| l.fill_work = 1_000);
    let fallback = with_limits(few, || shape_builder(&shapes, false));
    assert!(!fallback.regions.is_empty() && fallback.lines.is_empty(), "the filled areas, as before");
    assert_eq!(with_limits(few, || try_shape_builder(&shapes, false)), Err(PathOpsError::TooComplex));
    assert!(!with_limits(few, || pathfinder(PathfinderOp::Divide, &shapes)).is_empty());
    assert_eq!(with_limits(few, || try_pathfinder(PathfinderOp::Divide, &shapes)), Err(PathOpsError::TooComplex));
    // Past the segments the map is made of, the same; and Outline, which takes its cuts from the map,
    // gives nothing where `try_pathfinder` says why.
    let tight = limits(|l| l.shape_builder_segments = 8);
    assert!(!with_limits(tight, || shape_builder(&shapes, false)).regions.is_empty());
    assert_eq!(with_limits(tight, || try_shape_builder(&shapes, false)), Err(PathOpsError::TooComplex));
    assert!(with_limits(tight, || pathfinder(PathfinderOp::Outline, &shapes)).is_empty());
    assert_eq!(with_limits(tight, || try_pathfinder(PathfinderOp::Outline, &shapes)), Err(PathOpsError::TooComplex));
    // With the default limits the map is made, and the try forms give what the others give.
    assert_eq!(try_shape_builder(&shapes, true), Ok(shape_builder(&shapes, true)));
    for op in [PathfinderOp::Divide, PathfinderOp::Outline, PathfinderOp::Merge, PathfinderOp::Trim, PathfinderOp::Unite, PathfinderOp::Intersect] {
        assert_eq!(try_pathfinder(op, &shapes), Ok(pathfinder(op, &shapes)), "{op:?}");
    }
}

// ---- remove_redundant_points

/// Two smooth cubics merged into one are held to the tolerance between their samples too: a pair 1000
/// across, at a tolerance of 0.5, came out 1.0010 of it from the one cubic that replaced them (16
/// samples each); and a pair 40 across, at a tolerance of 0.25, 1.0013 of it, sampled as finely as a
/// merge now is but held to the samples only.
#[test]
fn a_merge_of_two_cubics_is_within_the_tolerance_between_its_samples() {
    let pairs = [
        (
            "M0,0 C141.48565563,-124.36162298999997 312.90930577343585,-163.5862231244835 477.52033205396543,-137.46956168238205 \
             C688.7299417556214,-103.95971732665808 888.5487988006399,37.63319206383998 1000,244.1965",
            0.5,
        ),
        (
            "M0,0 C6.742826980031999,7.7453082462079985 11.518474538498094,11.507438463691496 15.938384902008348,12.826348453191539 \
             C22.758219066796393,14.861400449026378 28.71872842696,10.949475244927001 40,7.0587399999999985",
            0.25,
        ),
    ];
    for (svg, tol) in pairs {
        let pair = BezPath::from_svg(svg).expect("an SVG path");
        let simpler = remove_redundant_points(&pair, tol);
        let dev = deviation(&pair, &simpler).max(deviation(&simpler, &pair));
        assert!(dev <= tol, "{dev} from the pair at a tolerance of {tol}: {simpler:?}");
    }
}

// ---- remove_redundant_points on finely cut curves

/// A circular arc: its radius, centre, the angle it starts at and how far it goes round (counter clockwise).
type Arc = (f64, (f64, f64), f64, f64);

/// The point of `arc` a fraction `t` of the way round it.
fn arc_point(&(r, (cx, cy), from, sweep): &Arc, t: f64) -> Point {
    let a = from + sweep * t;
    Point::new(cx + r * a.cos(), cy + r * a.sin())
}

/// `arc` in `n` cubics, each the standard arc of its angle; closed for the whole circle.
fn cut_arc(arc: &Arc, n: usize) -> BezPath {
    let mut path = BezPath::new();
    let &(r, _, from, sweep) = arc;
    for i in 0..n {
        let (t0, t1) = (i as f64 / n as f64, (i + 1) as f64 / n as f64);
        let (a0, a1) = (from + sweep * t0, from + sweep * t1);
        let k = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan() * r;
        let (p0, p3) = (arc_point(arc, t0), arc_point(arc, t1));
        if i == 0 {
            path.move_to(p0);
        }
        path.curve_to(p0 + kurbo::Vec2::new(-a0.sin(), a0.cos()) * k, p3 - kurbo::Vec2::new(-a1.sin(), a1.cos()) * k, p3);
    }
    if sweep >= std::f64::consts::TAU {
        path.close_path();
    }
    path
}

/// How far `p` is from `arc`.
fn distance_to_arc(arc: &Arc, p: Point) -> f64 {
    let &(r, (cx, cy), from, sweep) = arc;
    let along = (p.y - cy).atan2(p.x - cx) - from;
    if along.rem_euclid(std::f64::consts::TAU) <= sweep {
        (p.distance(Point::new(cx, cy)) - r).abs()
    } else {
        p.distance(arc_point(arc, 0.0)).min(p.distance(arc_point(arc, 1.0)))
    }
}

/// How far the curves of `path` are from `arcs` (the nearest of them, at 2,000 points of each curve), and
/// how far the arcs are from the curves (at 500 points of each arc, from the nearest curve).
fn arcs_deviation(path: &BezPath, arcs: &[Arc]) -> (f64, f64) {
    use kurbo::{ParamCurve, ParamCurveNearest};
    let curves: Vec<_> = path.segments().collect();
    let mut out: f64 = 0.0;
    for seg in &curves {
        for k in 0..=2_000 {
            let p = seg.eval(f64::from(k) / 2_000.0);
            out = out.max(arcs.iter().map(|a| distance_to_arc(a, p)).fold(f64::INFINITY, f64::min));
        }
    }
    let mut back: f64 = 0.0;
    for arc in arcs {
        for k in 0..=500 {
            let q = arc_point(arc, f64::from(k) / 500.0);
            back = back.max(curves.iter().map(|s| s.nearest(q, 1e-12).distance_sq.sqrt()).fold(f64::INFINITY, f64::min));
        }
    }
    (out, back)
}

/// A circle of radius 1,000 cut into many cubics comes down to a few, however finely it is cut. Merging
/// one cubic at a time added the whole worst case of each merge to what a run had used of the tolerance,
/// so a run of a few hundred pieces had none left: a hundred thousand cubics, at 0.01, came down to 508
/// elements, where a hundred came down to 36 and the carve before gave 29, though each was within a
/// hundred-thousandth of the tolerance of the circle. The elements here are no more than 5/4 of those of
/// the carve before, which merged on the error at its samples and so was not held to the tolerance
/// between them (it ended a run where that error passed the tolerance, this ends it where the run's own
/// bound does), and the result is within the tolerance of the circle and the circle of it.
#[test]
fn a_finely_cut_circle_comes_down_to_as_few_elements_as_a_coarsely_cut_one() {
    let circle: Arc = (1_000.0, (0.0, 0.0), 0.0, std::f64::consts::TAU);
    // (cubics, tolerance, elements before)
    for (n, tol, before) in [(100, 0.01, 27usize), (1_000, 0.01, 29), (100_000, 0.01, 29), (1_000, 0.1, 17), (100_000, 0.1, 17)] {
        let simpler = remove_redundant_points(&cut_arc(&circle, n), tol);
        let (out, back) = arcs_deviation(&simpler, &[circle]);
        assert!(out <= tol && back <= tol, "{n} cubics at {tol}: the result is {out} from the circle and the circle {back} from it");
        let elements = simpler.elements().len();
        assert!(elements <= before + before / 4, "{n} cubics at {tol}: {elements} elements, where there were {before}");
    }
}

/// A corner between two long smooth arcs stays, with each arc in a few cubics: a run is held to the
/// tolerance by what it is made of, so one that takes in the corner is refused however many pieces it has.
/// Two arcs of radius 500, a quarter circle each, the second turning away from the first by 60 degrees
/// where it starts.
#[test]
fn a_corner_between_two_finely_cut_arcs_is_kept() {
    let quarter = std::f64::consts::FRAC_PI_2;
    let first: Arc = (500.0, (0.0, 0.0), 0.0, quarter);
    // The first arrives at (0, 500) going along -x. The second leaves it 60 degrees to the left of that, and
    // curves left too: its centre is 500 to the left of where it leaves.
    let corner = arc_point(&first, 1.0);
    let leaves = std::f64::consts::PI + std::f64::consts::FRAC_PI_3;
    let centre = (corner.x + 500.0 * (leaves + quarter).cos(), corner.y + 500.0 * (leaves + quarter).sin());
    let second: Arc = (500.0, centre, (corner.y - centre.1).atan2(corner.x - centre.0), quarter);
    let mut both = cut_arc(&first, 4_000);
    both.extend(cut_arc(&second, 4_000).iter().skip(1));
    let tol = 0.1;
    let simpler = remove_redundant_points(&both, tol);
    let curves = simpler.segments().count();
    assert!(curves <= 6, "{curves} curves: {simpler:?}");
    let ends: Vec<Point> = simpler.segments().map(|s| kurbo::ParamCurve::eval(&s, 1.0)).collect();
    assert!(ends.iter().any(|p| p.distance(corner) < 1e-9), "no curve ends at the corner: {ends:?}");
    let (out, back) = arcs_deviation(&simpler, &[first, second]);
    assert!(out <= tol && back <= tol, "the result is {out} from the arcs and they {back} from it");
}

// ---- interior points

/// A rectangle 1 wide at x = 1e15 and 2 high at y = 1e16, where a unit in the last place of y is 2,
/// has no height strictly inside it: every height tried rounds onto its bottom or top edge, and a
/// point on an edge, which the winding counts as inside, was returned.
#[test]
fn an_interior_point_is_never_on_a_horizontal_edge() {
    let tall = rectangle(Rect::new(1e15, 1e16, 1e15 + 1.0, 1e16 + 2.0));
    let got = interior_point(&tall);
    assert!(got.is_none_or(|p| p.y > 1e16 && p.y < 1e16 + 2.0), "{got:?}, on an edge");
    // The same rectangle where the heights are representable has its point.
    let plain = rectangle(Rect::new(1e15, 1e6, 1e15 + 1.0, 1e6 + 2.0));
    assert!(interior_point(&plain).is_some_and(|p| p.y > 1e6 && p.y < 1e6 + 2.0));
}

// ---- unions

/// `unite_all` leaves out a path `try_unite_all` refuses, whole: a square with a loop of one cubic whose
/// handle is no number is no part of the union, where its square was.
#[test]
fn unite_all_leaves_out_a_refused_path_whole() {
    let mut bad = rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    bad.move_to((20.0, 0.0));
    bad.curve_to((f64::NAN, 0.0), (20.0, 10.0), (20.0, 0.0));
    bad.close_path();
    let good = rectangle(Rect::new(100.0, 0.0, 104.0, 4.0));
    let inputs = [(&bad, NZ), (&good, NZ)];
    assert_eq!(try_unite_all(&inputs), Err(PathOpsError::NonFinite));
    let union = unite_all(&inputs);
    assert!((area(&union, NZ) - 16.0).abs() < 1e-9, "{}", area(&union, NZ));
    assert!(union.winding(Point::new(5.0, 5.0)) == 0);
}
