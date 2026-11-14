//! A path that begins with a `ClosePath` (a closed subpath with nothing before it, which a caller can build with `Extend` or in a release
//! build): it closes nothing, and every entry point takes it as that. kurbo's `segments`, and its bounding box with it, panic on such a path;
//! `try_stroke_region`, `stroke_region` and `interior_point` let that panic out.

use craft_pathops::*;
use kurbo::{BezPath, PathEl, Point};

const NZ: FillRule = FillRule::NonZero;

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// The elements `rest` after `closes` `ClosePath`s: `extend` takes what `push` refuses in a test build (a path that does not begin with a move).
fn after_closes(closes: usize, rest: &[PathEl]) -> BezPath {
    let mut path = BezPath::new();
    path.extend(std::iter::repeat_n(PathEl::ClosePath, closes));
    path.extend(rest.iter().copied());
    path
}

fn triangle() -> Vec<PathEl> {
    vec![PathEl::MoveTo(p(0.0, 0.0)), PathEl::LineTo(p(10.0, 0.0)), PathEl::LineTo(p(10.0, 10.0)), PathEl::ClosePath]
}

/// Paths that begin with a close, and the path each is without it (`None`: nothing). What follows the closes begins with a move, as a path
/// does that kurbo accepts in a build for tests (it checks that in those builds, and in the crate's `push` calls).
fn cases() -> Vec<(&'static str, BezPath, Option<BezPath>)> {
    vec![
        ("a triangle after a close", after_closes(1, &triangle()), Some(after_closes(0, &triangle()))),
        ("a triangle after two closes", after_closes(2, &triangle()), Some(after_closes(0, &triangle()))),
        ("a close alone", after_closes(1, &[]), None),
    ]
}

/// A long, low outline (3,000 vertices): past the 2,048 segments where the sweep of a union or a stroke is turned if that is cheaper, so
/// the turn reads every outline's segments.
fn zigzag() -> BezPath {
    let mut path = BezPath::new();
    path.move_to((0.0, 0.0));
    for i in 1..3000 {
        path.line_to((f64::from(i) * 1.6, if i % 2 == 0 { 0.0 } else { 2.0 }));
    }
    path.line_to((3000.0 * 1.6, -5.0));
    path.line_to((0.0, -5.0));
    path.close_path();
    path
}

fn nothing() -> BezPath {
    BezPath::new()
}

#[test]
fn a_stroke_region_of_a_path_that_begins_with_a_close_is_that_of_the_path_without_it() {
    for (name, odd, clean) in cases() {
        let clean = clean.unwrap_or_else(nothing);
        // What the outline covers is what the path without its closes covers (50 square units for the triangle, nothing for a close alone).
        let covers = area(&clean, NZ).abs();
        let (made, tried) = (stroke_region(std::slice::from_ref(&odd)), try_stroke_region(std::slice::from_ref(&odd)).unwrap());
        assert!((area(&made, NZ).abs() - covers).abs() < 1e-6 && (area(&tried, NZ).abs() - covers).abs() < 1e-6, "{name}: {covers} square units");
        for beside in [vec![], vec![zigzag()]] {
            let with = |first: &BezPath| beside.iter().cloned().chain([first.clone()]).collect::<Vec<_>>();
            let (odd_in, clean_in) = (with(&odd), with(&clean));
            let label = format!("{name}, beside {} long outline(s)", beside.len());
            assert_eq!(stroke_region(&odd_in), stroke_region(&clean_in), "{label}: stroke_region");
            assert_eq!(
                try_stroke_region(&odd_in).map_err(|e| e.to_string()),
                try_stroke_region(&clean_in).map_err(|e| e.to_string()),
                "{label}: try_stroke_region"
            );
            // Alone, and first of two.
            assert_eq!(stroke_region(std::slice::from_ref(&odd)), stroke_region(std::slice::from_ref(&clean)), "{name}: alone");
            assert_eq!(
                try_stroke_region(&[odd.clone(), clean.clone()]).map_err(|e| e.to_string()),
                try_stroke_region(&[clean.clone(), clean.clone()]).map_err(|e| e.to_string()),
                "{name}: doubled"
            );
        }
    }
}

#[test]
fn an_interior_point_of_a_path_that_begins_with_a_close_is_that_of_the_path_without_it() {
    for (name, odd, clean) in cases() {
        assert_eq!(interior_point(&odd), clean.as_ref().and_then(interior_point), "{name}");
    }
}

/// Every entry point that takes a path or shapes, on every path of [`cases`], alone and beside a long outline: none panics (a panic fails the test).
#[test]
fn no_entry_point_panics_on_a_path_that_begins_with_a_close() {
    for (name, odd, _) in cases() {
        for far in [BezPath::new(), zigzag()] {
            let other = if far.elements().is_empty() { after_closes(0, &triangle()) } else { far };
            let shapes = vec![Shape::new(odd.clone(), NZ, 0), Shape::new(other.clone(), NZ, 1)];
            let label = format!("{name}, beside {} elements", other.elements().len());
            let _ = (stroke_region(&[odd.clone(), other.clone()]), try_stroke_region(&[odd.clone(), other.clone()]));
            let _ = (try_unite_all(&[(&odd, NZ), (&other, NZ)]), unite_all(&[(&odd, NZ), (&other, NZ)]));
            let _ = try_boolean_n(&[(&odd, NZ), (&other, NZ)], |c| !c.is_empty());
            let _ = (boolean(&odd, NZ, &other, NZ, BoolOp::Union), try_boolean(&odd, NZ, &other, NZ, BoolOp::Union, DEFAULT_PRECISION));
            let _ = (normalize(&odd, NZ), try_normalize(&odd, NZ), area(&odd, NZ), try_area(&odd, NZ));
            let _ = (simplify(&odd, 1.0), simplify_with(&odd, &SimplifyOptions::default().tolerance(1.0).smooth(1.0)));
            let _ = (remove_redundant_points(&odd, 1.0), join(&[odd.clone(), other.clone()], 1.0));
            let _ =
                (offset_path(&odd, 1.0, Join::Miter, 4.0), offset_path(&odd, -1.0, Join::Miter, 4.0), try_offset_path(&odd, 1.0, Join::Miter, 4.0));
            let _ = (outline_stroke(&odd, 2.0, Cap::Butt, Join::Miter, 4.0), try_outline_stroke(&odd, 2.0, Cap::Butt, Join::Miter, 4.0));
            for op in [PathfinderOp::Unite, PathfinderOp::MinusFront, PathfinderOp::Intersect, PathfinderOp::Exclude, PathfinderOp::Divide] {
                let _ = (pathfinder(op, &shapes), try_pathfinder(op, &shapes));
            }
            let _ = (regions(&shapes), regions_within(&shapes, 100), try_regions_within(&shapes, 100), region_at(&shapes, p(1.0, 0.5)));
            let _ = (has_open_path(&shapes), interior_point(&odd), live_paint(&shapes));
            let _ = (shape_builder(&shapes, true), try_shape_builder(&shapes, true));
            assert!(!label.is_empty());
        }
    }
}

/// Paths with no `MoveTo` before the first segment: a line, a quad and a cubic first, then two lines and a close. kurbo reads such a path as
/// starting at the end of its first element, and says in a build for tests (by an assertion in `push`) that it is not a path; a release
/// build takes it, and so does every function but two in a build for tests. `stroke_region` and `try_stroke_region` added its elements with
/// `push` and `close_path`, and panicked in a build for tests.
fn no_move_cases() -> Vec<(&'static str, BezPath, BezPath)> {
    let rest = [PathEl::LineTo(p(4.0, 0.0)), PathEl::LineTo(p(4.0, 4.0)), PathEl::ClosePath];
    let firsts = [
        ("a line first", PathEl::LineTo(p(0.0, 0.0))),
        ("a quad first", PathEl::QuadTo(p(1.0, 1.0), p(0.0, 0.0))),
        ("a cubic first", PathEl::CurveTo(p(1.0, 1.0), p(2.0, 2.0), p(0.0, 0.0))),
    ];
    firsts
        .into_iter()
        .map(|(name, first)| {
            let mut odd = BezPath::new();
            odd.extend(std::iter::once(first).chain(rest));
            // The same path with its first element a move to where kurbo takes it to start.
            let clean = BezPath::from_vec(std::iter::once(PathEl::MoveTo(p(0.0, 0.0))).chain(rest).collect());
            (name, odd, clean)
        })
        .collect()
}

/// The stroke region of a path with no move first is that of the path with one: the triangle it draws, 8 square units, alone and beside a long
/// outline (which makes the turn happen), by both functions. In a debug build both panicked.
#[test]
fn a_stroke_region_of_a_path_with_no_move_first_is_that_of_the_path_with_one() {
    for (name, odd, clean) in no_move_cases() {
        let covers = area(&clean, NZ).abs();
        assert!((covers - 8.0).abs() < 1e-9, "{name}: the triangle covers {covers}");
        for beside in [vec![], vec![zigzag()]] {
            let with = |first: &BezPath| beside.iter().cloned().chain([first.clone()]).collect::<Vec<_>>();
            let (odd_in, clean_in) = (with(&odd), with(&clean));
            let label = format!("{name}, beside {} long outline(s)", beside.len());
            let (made, tried) = (stroke_region(&odd_in), try_stroke_region(&odd_in).unwrap());
            let (want, want_tried) = (stroke_region(&clean_in), try_stroke_region(&clean_in).unwrap());
            // Alone it covers the triangle; beside the long outline it covers what the outline does and the triangle's part outside it.
            let floor = if beside.is_empty() { covers } else { area(&zigzag(), NZ).abs() };
            for (path, kind) in [(&made, "stroke_region"), (&tried, "try_stroke_region")] {
                let got = area(path, NZ).abs();
                assert!(got >= floor - 1e-3 && (!beside.is_empty() || (got - covers).abs() < 1e-9), "{label}: {kind} covers {got}");
            }
            // The same region as the path with the move: the shape is the same, though a degenerate first curve can change where the contour starts.
            for (path, expected, kind) in [(&made, &want, "stroke_region"), (&tried, &want_tried, "try_stroke_region")] {
                assert!(
                    (area(path, NZ) - area(expected, NZ)).abs() < 1e-6,
                    "{label}: {kind} covers {} where the path with a move covers {}",
                    area(path, NZ),
                    area(expected, NZ)
                );
            }
        }
    }
}
