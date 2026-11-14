//! Claims made in the review of the first pull request, each as the smallest test that fails when
//! the claim is true. A claim that did not hold has no test here.

use craft_pathops::*;
use kurbo::{BezPath, ParamCurve, Point, Rect, Shape as _};

use crate::common::{distance_to, poly, polyline, rectangle, rel_close as close};

const NZ: FillRule = FillRule::NonZero;

fn open(pts: &[(f64, f64)]) -> BezPath {
    poly(pts, false)
}

fn closed(pts: &[(f64, f64)]) -> BezPath {
    poly(pts, true)
}

fn length(path: &BezPath) -> f64 {
    use kurbo::ParamCurveArclen;
    path.segments().map(|s| s.arclen(1e-9)).sum()
}

/// The largest distance from a sample of `a` to the outline of `b`.
fn deviation(a: &BezPath, b: &BezPath) -> f64 {
    let mut worst: f64 = 0.0;
    for seg in a.segments() {
        for k in 0..=8 {
            worst = worst.max(distance_to(b, seg.eval(f64::from(k) / 8.0)));
        }
    }
    worst
}

/// The self-crossing operand of the first review's 5478 fixture, with the triangle it meets.
fn fixture() -> (BezPath, BezPath) {
    (closed(&[(3.0, 1.0), (3.0, 3.0), (0.0, 1.0)]), closed(&[(3.0, 0.0), (0.0, 2.0), (3.0, 2.0), (0.0, 0.0), (0.0, 1.0)]))
}

#[test]
fn claim_02_the_n_ary_booleans_resolve_operands_whose_windings_do_not_add_up() {
    let (a, b) = fixture();
    for rule in [NZ, FillRule::EvenOdd] {
        for (name, want, pred) in [
            ("union", 4.375, (|c: &[usize]| !c.is_empty()) as fn(&[usize]) -> bool),
            ("intersection", 1.125, |c| c.len() == 2),
            ("difference", 1.875, |c| c == [0]),
            ("xor", 3.25, |c| c.len() == 1),
        ] {
            let got = area(&boolean_n(&[(&a, rule), (&b, rule)], pred), NZ);
            assert!(close(got, want, 1e-6), "boolean_n {name} {rule:?}: {got}, want {want}");
            let tried = try_boolean_n(&[(&a, rule), (&b, rule)], pred).unwrap();
            assert!(close(area(&tried, NZ), want, 1e-6), "try_boolean_n {name} {rule:?}");
        }
    }
}

#[test]
fn claim_04_straight_simplification_keeps_an_excursion_past_the_ends() {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.curve_to((10.0, 0.0), (10.0, 0.0), (1.0, 0.0));
    let s = simplify_with(&p, &SimplifyOptions::default().tolerance(0.1).straight_lines(true));
    assert!(deviation(&p, &s) <= 0.11, "the result is {} from the curve", deviation(&p, &s));
    assert!(s.bounding_box().x1 > 6.0, "{:?}", s.bounding_box());
}

#[test]
fn claim_05_removing_redundant_points_keeps_a_finely_sampled_arc() {
    let n = 10_000;
    let pts: Vec<Point> = (0..=n)
        .map(|i| {
            let a = std::f64::consts::PI * f64::from(i) / f64::from(n);
            Point::new(a.cos(), a.sin())
        })
        .collect();
    let arc = polyline(&pts, false);
    let cleaned = remove_redundant_points(&arc, 0.001);
    assert!(deviation(&arc, &cleaned) <= 0.0011, "the arc is {} from the result", deviation(&arc, &cleaned));
    assert!(cleaned.elements().len() > 5, "{} elements", cleaned.elements().len());
}

#[test]
fn claim_06_an_inset_of_nested_same_winding_rectangles_has_no_moat() {
    let mut nested = rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
    nested.extend(rectangle(Rect::new(30.0, 30.0, 70.0, 70.0)).iter());
    let inset = offset_path(&nested, -5.0, Join::Miter, 4.0);
    assert!(close(area(&inset, NZ), 90.0 * 90.0, 1e-3), "area {}", area(&inset, NZ));
}

#[test]
fn claim_07_a_negative_offset_outlines_an_open_line() {
    let line = open(&[(0.0, 0.0), (100.0, 0.0)]);
    for delta in [2.0, -2.0] {
        let o = offset_path(&line, delta, Join::Miter, 4.0);
        assert!(close(area(&o, NZ), 400.0, 1e-6), "offset {delta}: area {}", area(&o, NZ));
        assert_eq!(try_offset_path(&line, delta, Join::Miter, 4.0).map(|p| area(&p, NZ).round()), Ok(400.0));
    }
    // Beside a closed shape the inset keeps both: the shape inset, and the line outlined.
    let mut both = rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
    both.extend(open(&[(200.0, 0.0), (300.0, 0.0)]).iter());
    let o = offset_path(&both, -5.0, Join::Miter, 4.0);
    assert!(close(area(&o, NZ), 90.0 * 90.0 + 1000.0, 1e-6), "area {}", area(&o, NZ));
    // Where they cross, it is the union: the inset square (8,100) and the 55 units of the stroke, 10 wide, outside it at either end.
    let mut crossing = rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
    crossing.extend(open(&[(-50.0, 50.0), (150.0, 50.0)]).iter());
    let o = offset_path(&crossing, -5.0, Join::Miter, 4.0);
    assert!(close(area(&o, NZ), 90.0 * 90.0 + 2.0 * 55.0 * 10.0, 1e-6), "area {}", area(&o, NZ));
}

#[test]
fn claim_08_outline_splits_a_rectangle_and_an_open_line_at_both_crossings() {
    let shapes = [Shape::new(rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), NZ, 1), Shape::new(open(&[(-10.0, 50.0), (110.0, 50.0)]), NZ, 2)];
    let pieces = pathfinder(PathfinderOp::Outline, &shapes);
    let of = |key: u64| pieces.iter().filter(|s| s.key == key).collect::<Vec<_>>();
    assert_eq!(of(1).len(), 2, "the rectangle is cut at (0, 50) and (100, 50)");
    assert!(close(of(1).iter().map(|s| length(&s.path)).sum(), 400.0, 1e-6));
    assert_eq!(of(2).len(), 3, "the line is cut at the same two points");
    assert!(close(of(2).iter().map(|s| length(&s.path)).sum(), 120.0, 1e-6));
}

#[test]
fn claim_10_outline_of_an_open_path_has_only_its_own_edges() {
    let v = open(&[(0.0, 0.0), (10.0, 20.0), (20.0, 0.0)]);
    let pieces = pathfinder(PathfinderOp::Outline, &[Shape::new(v, NZ, 1)]);
    let total: f64 = pieces.iter().map(|s| length(&s.path)).sum();
    assert!(close(total, 2.0 * 500f64.sqrt(), 1e-9), "{} pieces, {total} long", pieces.len());
}

#[test]
fn claim_11_a_cutter_that_is_level_in_the_sweeps_frame_keeps_its_loose_end() {
    let shapes = [Shape::new(rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), NZ, 0), Shape::new(open(&[(-10.0, 50.3125), (110.0, 46.5625)]), NZ, 1)];
    let built = shape_builder(&shapes, false);
    let cutter: Vec<&Shape> = built.lines.iter().filter(|l| l.key == 1).collect();
    assert_eq!(cutter.len(), 3, "loose end, inside, loose end: {:?}", cutter.iter().map(|s| length(&s.path)).collect::<Vec<_>>());
    let (_, edges) = live_paint(&shapes);
    let cut: Vec<&Shape> = edges.iter().filter(|l| l.key == 1).collect();
    assert_eq!(cut.len(), 3, "live_paint: {:?}", cut.iter().map(|s| length(&s.path)).collect::<Vec<_>>());
}

#[test]
fn claim_12_an_interior_point_is_found_in_separated_components() {
    let mut two = closed(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
    two.extend(closed(&[(0.0, 100.0), (1.0, 100.0), (1.0, 101.0), (0.0, 101.0)]).iter());
    let p = interior_point(&two).expect("a point inside one of the squares");
    assert!(two.winding(p) != 0, "{p:?}");
}

#[test]
fn claim_13_the_shape_builder_fallback_fills_no_open_subpath() {
    let few = {
        let mut l = Limits::default();
        l.shape_builder_segments = 2;
        l
    };
    // An open triangle (a cutter) fills nothing, by itself and beside a closed rectangle.
    let triangle = open(&[(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)]);
    let mut mixed = rectangle(Rect::new(20.0, 0.0, 30.0, 10.0));
    mixed.extend(triangle.iter());
    let shapes = [Shape::new(triangle.clone(), NZ, 0), Shape::new(mixed, NZ, 1)];
    let built = with_limits(few, || shape_builder(&shapes, false));
    let filled: f64 = built.regions.iter().map(|r| area(&r.path, NZ)).sum();
    assert!(close(filled, 100.0, 1e-9), "{filled} filled, the rectangle is 100: {:?}", built.regions.iter().map(|r| &r.sources).collect::<Vec<_>>());
    assert!(built.regions.iter().all(|r| r.sources == [1]));
    let divided = with_limits(few, || pathfinder(PathfinderOp::Divide, &shapes));
    assert!(divided.iter().all(|s| s.key == 1), "{:?}", divided.iter().map(|s| s.key).collect::<Vec<_>>());
}

#[test]
fn claim_15_short_edges_survive_beside_a_very_long_one() {
    let bumped = closed(&[(0.0, 0.0), (1e9, 0.0), (1e9, 1.0), (3.0, 1.0), (3.0, 2.0), (2.0, 2.0), (2.0, 1.0), (0.0, 1.0)]);
    let got = area(&unite_all(&[(&bumped, NZ)]), NZ);
    assert!(close(got, 1e9 + 1.0, 1e-12), "{got}");
    let normalized = area(&normalize(&bumped, NZ), NZ);
    assert!(close(normalized, 1e9 + 1.0, 1e-12), "{normalized}");
}

#[test]
fn claim_20_a_closed_single_cubic_loop_has_its_area() {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.curve_to((100.0, 0.0), (0.0, 100.0), (0.0, 0.0));
    p.close_path();
    let want = p.area().abs();
    assert!(want > 1000.0, "{want}");
    assert!(close(try_area(&p, NZ).unwrap(), want, 1e-9), "{:?}", try_area(&p, NZ));
    let square = rectangle(Rect::new(-50.0, -50.0, 150.0, 150.0));
    let union = area(&boolean(&p, NZ, &square, NZ, BoolOp::Intersect), NZ);
    assert!(close(union, want, 1e-6), "{union}");
    let shapes = [Shape::new(p.clone(), NZ, 0)];
    let faces = regions(&shapes);
    assert_eq!(faces.len(), 1, "{} regions", faces.len());
    assert!(close(area(&faces[0].path, NZ), want, 1e-6));
    let (live, _) = live_paint(&shapes);
    assert!(live.iter().any(|f| f.sources == [0] && close(area(&f.path, NZ), want, 1e-6)), "{} faces", live.len());
    assert!(offset_path(&p, 2.0, Join::Round, 4.0).elements().len() > 3);
    // A loop of a line or of a curve with no inside has none.
    let flat = {
        let mut f = BezPath::new();
        f.move_to((0.0, 0.0));
        f.curve_to((10.0, 0.0), (20.0, 0.0), (0.0, 0.0));
        f.close_path();
        f
    };
    assert_eq!(try_area(&flat, NZ), Ok(0.0));
}
