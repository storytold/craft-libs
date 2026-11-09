//! A Pathfinder Outline cuts an open path where it meets another path or itself, and nowhere else. The planar map it reads its junctions from
//! traces an open path out and back, and where the path runs nearly level for a stretch the two ways part by more than the map's precision
//! and left vertices in the middle of the path, where nothing meets it: one open arc, 100,000 across, crossed once by a line came back in
//! 6 or 7 pieces and not 4, and a circle of radius 100 whose path does not end in a close and is crossed by a line in 6 pieces and not 5.

use craft_pathops::{FillRule, PathfinderOp, Shape, try_pathfinder_with};
use kurbo::{Arc, BezPath, Circle, PathEl, Shape as _};

const PRECISION: f64 = 1e-5;

fn line(a: (f64, f64), b: (f64, f64)) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(a);
    p.line_to(b);
    p
}

/// An open arc from (0, 0) to (`chord`, 0) that bows up by a twentieth of its chord, as cubics within `tolerance` of the circle.
fn arc(chord: f64, tolerance: f64) -> BezPath {
    let sag = chord / 20.0;
    let radius = (chord * chord / 4.0 + sag * sag) / (2.0 * sag);
    let half = (chord / 2.0).atan2(radius - sag);
    let arc = Arc::new((chord / 2.0, sag - radius), (radius, radius), std::f64::consts::FRAC_PI_2 + half, -2.0 * half, 0.0);
    let mut path = BezPath::new();
    path.move_to((0.0, 0.0));
    for el in arc.append_iter(tolerance) {
        path.push(el);
    }
    path
}

/// The Outline of `paths` (each its own shape, keyed by its place), as the numbers of the pieces of each.
fn pieces(paths: Vec<BezPath>) -> Vec<usize> {
    let shapes: Vec<Shape> = paths.iter().enumerate().map(|(i, p)| Shape::new(p.clone(), FillRule::EvenOdd, i as u64)).collect();
    let out = try_pathfinder_with(PathfinderOp::Outline, &shapes, PRECISION).expect("within the limits");
    (0..paths.len() as u64).map(|key| out.iter().filter(|s| s.key == key).count()).collect()
}

/// The path with its closing element dropped, so that it is open and comes back to where it started.
fn unclosed(path: &BezPath) -> BezPath {
    BezPath::from_vec(path.elements().iter().copied().filter(|el| !matches!(el, PathEl::ClosePath)).collect())
}

/// One open arc, crossed once in its middle by a line, is cut once there: two pieces of the arc and two of the line, whatever the arc is cut
/// into (the cubics it is flattened to, within a thousandth, a millionth and a billionth of its circle) and however long it is.
#[test]
fn an_arc_crossed_once_is_cut_once() {
    for chord in [1_000.0, 1e5] {
        for tolerance in [1e-3, 1e-6, 1e-9] {
            let cut = line((chord / 2.0, -chord / 10.0), (chord / 2.0, chord / 10.0));
            assert_eq!(pieces(vec![arc(chord, tolerance), cut]), vec![2, 2], "an arc {chord} across, to {tolerance}");
        }
    }
}

/// An arc that nothing crosses is one piece.
#[test]
fn an_arc_that_nothing_crosses_is_one_piece() {
    for tolerance in [1e-3, 1e-6, 1e-9] {
        assert_eq!(pieces(vec![arc(1e5, tolerance)]), vec![1], "to {tolerance}");
    }
}

/// A circle crossed twice by a line is two pieces and the line three, drawn with a close or without one (a run of cubics that ends where
/// it started is a ring either way), of radius 100 and 1,000.
#[test]
fn a_circle_crossed_twice_is_cut_twice_with_a_close_or_without_one() {
    for radius in [100.0, 1_000.0] {
        let circle = Circle::new((0.0, 0.0), radius).to_path(1e-9);
        let cut = line((-3.0 * radius, 0.3 * radius), (3.0 * radius, 0.3 * radius));
        assert_eq!(pieces(vec![circle.clone(), cut.clone()]), vec![2, 3], "a circle of radius {radius}");
        assert_eq!(pieces(vec![unclosed(&circle), cut]), vec![2, 3], "a circle of radius {radius}, with no close");
    }
}

/// Where paths do meet they are cut: two lines that cross (two pieces each), an arc whose middle another path ends on (the arc two, the
/// ender one), and a path that crosses itself (three pieces: the way to the crossing, the loop, the way on).
#[test]
fn the_places_where_paths_meet_are_still_cuts() {
    assert_eq!(pieces(vec![line((0.0, 0.0), (10.0, 10.0)), line((0.0, 10.0), (10.0, 0.0))]), vec![2, 2], "two lines that cross");
    let chord = 1_000.0;
    for tolerance in [1e-3, 1e-9] {
        let ender = line((chord / 2.0, -chord / 10.0), (chord / 2.0, chord / 20.0));
        assert_eq!(pieces(vec![arc(chord, tolerance), ender]), vec![2, 1], "an arc with a line ending on it, to {tolerance}");
    }
    let mut figure = BezPath::new();
    for (i, p) in [(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)].into_iter().enumerate() {
        if i == 0 {
            figure.move_to(p);
        } else {
            figure.line_to(p);
        }
    }
    assert_eq!(pieces(vec![figure]), vec![3], "a path that crosses itself");
}
