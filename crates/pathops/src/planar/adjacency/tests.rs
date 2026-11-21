use super::*;
use craft_geom::{FillRule, Rect, shapes};
use kurbo::{ParamCurveNearest, Vec2};
use proptest::prelude::*;

fn line(a: (f64, f64), b: (f64, f64)) -> Shape {
    Shape::new(PathData::single(SubPath::polyline(&[Point::new(a.0, a.1), Point::new(b.0, b.1)], false)), FillRule::NonZero, 0)
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
    Shape::new(shapes::rectangle(Rect::new(x0, y0, x1, y1)), FillRule::NonZero, 0)
}

fn inside(f: &MapFace, p: Point) -> bool {
    f.path.to_bezpath().winding(p) != 0
}

/// The edges whose middle lies within 0.01 of `p`.
fn edges_at(a: &Adjacency, p: Point) -> Vec<&MapEdge> {
    a.edges
        .iter()
        .filter(|e| {
            let bp = e.path.to_bezpath();
            bp.segments().any(|s| {
                let n = s.nearest(p, 1e-6);
                n.distance_sq < 1e-4
            })
        })
        .collect()
}

#[test]
fn a_line_across_a_rectangle_divides_two_faces_it_lies_between() {
    let a = adjacency(&[rect(0.0, 0.0, 100.0, 50.0), line((40.0, -10.0), (40.0, 60.0))], DEFAULT_PRECISION).unwrap();
    assert_eq!(a.faces.len(), 2, "{:?}", a.faces);
    let left_face = a.faces.iter().position(|f| inside(f, Point::new(20.0, 25.0))).unwrap();
    let right_face = a.faces.iter().position(|f| inside(f, Point::new(70.0, 25.0))).unwrap();
    assert_ne!(left_face, right_face);
    assert!((a.faces[left_face].area - 2000.0).abs() < 1e-6 && (a.faces[right_face].area - 3000.0).abs() < 1e-6);
    let shared = edges_at(&a, Point::new(40.0, 25.0));
    assert_eq!(shared.len(), 1, "{shared:?}");
    let e = shared[0];
    assert_eq!(e.sources, vec![1]);
    let mut sides = [e.left.unwrap(), e.right.unwrap()];
    sides.sort_unstable();
    let mut want = [left_face, right_face];
    want.sort_unstable();
    assert_eq!(sides, want);
    // The rectangle's own sides have a face on one side only; the line's ends stick out into no face.
    let side = edges_at(&a, Point::new(70.0, 0.0));
    assert_eq!(side.len(), 1);
    assert_eq!([side[0].left, side[0].right].iter().flatten().count(), 1);
    let stub = edges_at(&a, Point::new(40.0, -5.0));
    assert_eq!((stub[0].left, stub[0].right), (None, None));
    assert!(a.faces.iter().all(|f| f.piece == 0) && a.edges.iter().all(|e| e.piece == 0));
}

#[test]
fn a_line_that_ends_inside_a_face_has_that_face_on_both_sides() {
    let a = adjacency(&[rect(0.0, 0.0, 100.0, 50.0), line((50.0, 0.0), (50.0, 30.0)), line((20.0, 10.0), (30.0, 20.0))], DEFAULT_PRECISION).unwrap();
    assert_eq!(a.faces.len(), 1);
    for at in [Point::new(50.0, 15.0), Point::new(25.0, 15.0)] {
        let e = edges_at(&a, at);
        assert_eq!(e.len(), 1, "{at:?}");
        assert_eq!((e[0].left, e[0].right), (Some(0), Some(0)), "{at:?}");
    }
}

#[test]
fn loose_lines_in_many_faces_look_only_at_the_faces_near_them() {
    // A 40 × 40 grid of separate squares, then the same with a loose line in each.
    let squares: Vec<Shape> =
        (0..1600).map(|k| (f64::from(k % 40) * 20.0, f64::from(k / 40) * 20.0)).map(|(x, y)| rect(x, y, x + 10.0, y + 10.0)).collect();
    let started = super::super::index::work_done();
    assert_eq!(adjacency(&squares, DEFAULT_PRECISION).unwrap().faces.len(), 1600);
    let plain = super::super::index::work_done() - started;
    let mut all = squares.clone();
    all.extend((0..1600).map(|k| (f64::from(k % 40) * 20.0, f64::from(k / 40) * 20.0)).map(|(x, y)| line((x + 5.0, y), (x + 5.0, y + 5.0))));
    let started = super::super::index::work_done();
    let a = adjacency(&all, DEFAULT_PRECISION).unwrap();
    let work = super::super::index::work_done() - started - plain;
    let loose: Vec<&MapEdge> = a.edges.iter().filter(|e| e.sources.len() == 1 && e.sources[0] >= 1600).collect();
    assert_eq!(loose.len(), 1600);
    for e in loose {
        let mid = e.path.to_bezpath().segments().next().unwrap().eval(0.5);
        let face = a.faces.iter().position(|f| inside(f, mid));
        assert_eq!((e.left, e.right), (face, face));
        assert!(face.is_some());
    }
    // Testing every face for each loose line would read 1,600 faces per line.
    assert!(work <= 64 * 1600, "{work} reads");
}

#[test]
fn a_line_on_a_rectangle_side_keeps_both_inputs_in_either_order() {
    for flip in [false, true] {
        let (r, l) = (rect(0.0, 0.0, 100.0, 50.0), line((-10.0, 50.0), (110.0, 50.0)));
        let shapes = if flip { vec![l, r] } else { vec![r, l] };
        let a = adjacency(&shapes, DEFAULT_PRECISION).unwrap();
        let on = edges_at(&a, Point::new(30.0, 50.0));
        assert_eq!(on.len(), 1, "flip {flip}: {on:?}");
        assert_eq!(on[0].sources, vec![0, 1], "flip {flip}");
    }
}

#[test]
fn each_side_names_the_face_seen_on_that_side_of_the_edge() {
    // A tray-like outline with a crease grid, edges drawn in every direction.
    let a = adjacency(
        &[
            rect(0.0, 0.0, 90.0, 60.0),
            line((30.0, 0.0), (30.0, 60.0)),
            line((60.0, 60.0), (60.0, 0.0)),
            line((90.0, 20.0), (0.0, 20.0)),
            line((0.0, 40.0), (90.0, 40.0)),
        ],
        DEFAULT_PRECISION,
    )
    .unwrap();
    assert_eq!(a.faces.len(), 9);
    let mut checked = 0;
    for e in &a.edges {
        let bp = e.path.to_bezpath();
        let Some(seg) = bp.segments().next() else { continue };
        let (p, d) = (seg.eval(0.5), seg.end() - seg.start());
        let n = d.normalize();
        // On the page (y down), the left of a direction (dx, dy) is (dy, -dx).
        let (left_at, right_at) = (p + Vec2::new(n.y, -n.x) * 0.5, p + Vec2::new(-n.y, n.x) * 0.5);
        for (side, at) in [(e.left, left_at), (e.right, right_at)] {
            match side {
                Some(f) => assert!(inside(&a.faces[f], at), "{e:?}"),
                None => assert!(a.faces.iter().all(|f| !inside(f, at)), "{e:?}"),
            }
            checked += 1;
        }
    }
    assert!(checked >= 24, "{checked}");
}

#[test]
fn a_slot_narrower_than_the_precision_is_a_sliver() {
    let a = adjacency(&[rect(0.0, 0.0, 100.0, 50.0), rect(50.0, 10.0, 50.004, 40.0)], DEFAULT_PRECISION).unwrap();
    assert_eq!(a.slivers.len(), 1, "{:?}", a.slivers);
    assert_eq!(a.faces.len(), 1);
    let b = adjacency(&[rect(0.0, 0.0, 100.0, 50.0), rect(50.0, 10.0, 51.0, 40.0)], DEFAULT_PRECISION).unwrap();
    assert!(b.slivers.is_empty());
    assert_eq!(b.faces.len(), 2, "the slot is a face of its own");
}

#[test]
fn a_ring_is_a_face_with_a_hole_and_two_pieces() {
    let a = adjacency(&[rect(0.0, 0.0, 100.0, 100.0), rect(30.0, 30.0, 70.0, 70.0)], DEFAULT_PRECISION).unwrap();
    assert_eq!(a.faces.len(), 2);
    let ring = a.faces.iter().find(|f| inside(f, Point::new(10.0, 10.0))).unwrap();
    assert!(!inside(ring, Point::new(50.0, 50.0)), "the inner square is a hole in the ring");
    assert!((ring.area - 8400.0).abs() < 1e-6, "{}", ring.area);
    assert_eq!(a.outlines.len(), 2);
    assert_ne!(a.outlines[0].piece, a.outlines[1].piece);
    let inner = a.faces.iter().find(|f| inside(f, Point::new(50.0, 50.0))).unwrap();
    assert_ne!(inner.piece, ring.piece);
    let inner_side = edges_at(&a, Point::new(50.0, 30.0));
    let mut sides = [inner_side[0].left, inner_side[0].right];
    sides.sort_unstable();
    assert!(sides.iter().all(Option::is_some), "a ring face on one side, the inner square on the other: {sides:?}");
}

#[test]
fn bad_input_gives_no_map() {
    assert!(adjacency(&[], DEFAULT_PRECISION).is_none());
    assert!(adjacency(&[line((0.0, 0.0), (f64::NAN, 1.0))], DEFAULT_PRECISION).is_none());
    assert!(adjacency(&[rect(0.0, 0.0, 10.0, 10.0)], 0.0).is_none());
    assert!(adjacency(&[rect(0.0, 0.0, 10.0, 10.0)], f64::INFINITY).is_none());
    let mut points: Vec<Point> = (0..=SHAPE_BUILDER_MAX_SEGMENTS + 1).map(|i| Point::new(i as f64, (i % 2) as f64)).collect();
    let open = Shape::new(PathData::single(SubPath::polyline(&points, false)), FillRule::NonZero, 0);
    assert!(adjacency(&[open], DEFAULT_PRECISION).is_none(), "over the segment limit");
    points.truncate(64);
    let open = Shape::new(PathData::single(SubPath::polyline(&points, false)), FillRule::NonZero, 0);
    assert!(adjacency(&[open], DEFAULT_PRECISION).is_some());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    /// Random lines and boxes never panic, and every side an edge names is a face.
    #[test]
    fn random_lines_give_a_consistent_map(
        segs in proptest::collection::vec((-50.0f64..50.0, -50.0f64..50.0, -50.0f64..50.0, -50.0f64..50.0), 1..12),
        boxed in any::<bool>(),
    ) {
        let mut shapes: Vec<Shape> = segs.iter().map(|&(a, b, c, d)| line((a, b), (c, d))).collect();
        if boxed {
            shapes.push(rect(-40.0, -40.0, 40.0, 40.0));
        }
        if let Some(a) = adjacency(&shapes, DEFAULT_PRECISION) {
            for e in &a.edges {
                prop_assert!(e.left.is_none_or(|f| f < a.faces.len()) && e.right.is_none_or(|f| f < a.faces.len()));
                prop_assert!(e.sources.iter().all(|&s| s < shapes.len()));
            }
        }
    }
}
