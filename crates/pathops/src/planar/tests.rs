// Carved from VectorCraft's crates/pathops (src/planar/tests.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.
// Brought up to storytold/vectorcraft 0945eaf2b3d4e93963aaa6d796b9efe53feaf624 by porting its changes to this file (3fcd157..0945eaf; see the README).

use super::*;
use crate::path::subs;
use crate::testutil::{anchor_count, ellipse, length, polyline, polyline_pts, rect as rect_path, subpath_count};
use crate::work::{self, Work};
use kurbo::{ParamCurveNearest, Rect};
use proptest::prelude::*;

/// Sparse crossings add, compare and read like the dense counts they stand for.
#[test]
fn crossings_are_sparse_counts() {
    let mut c = Crossings::single(7, true);
    c += Crossings::single(2, false);
    c += Crossings::single(7, true);
    assert_eq!((c.get(14), c.get(5), c.get(15), c.get(0)), (2, 1, 0, 0));
    assert_eq!(c.0.len(), 2, "only the slots that aren't 0");
    assert_eq!(c.of_tag(7), Crossings(vec![(14, 2)]));
    assert_eq!(Crossings::single(2, false) + Crossings::single(7, true) + Crossings::single(7, true), c);
    assert_eq!(Crossings::default() + Crossings::default(), Crossings::default());
}

fn line(a: (f64, f64), b: (f64, f64), key: u64) -> Shape {
    Shape::new(polyline(&[a, b], false), FillRule::NonZero, key)
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64, key: u64) -> Shape {
    Shape::new(rect_path(x0, y0, x1, y1), FillRule::NonZero, key)
}

fn area(r: &Region) -> f64 {
    r.path.area()
}

fn at(faces: &[Region], x: f64, y: f64) -> Vec<&Region> {
    faces.iter().filter(|f| f.contains(Point::new(x, y))).collect()
}

/// The first and last points of a path of one open subpath.
fn ends(path: &BezPath) -> (Point, Point) {
    let sub = subs(path).into_iter().next().expect("a subpath");
    (sub.start, sub.end())
}

/// Whether the path has a closed subpath.
fn has_closed(path: &BezPath) -> bool {
    subs(path).iter().any(|s| s.closed)
}

fn bounds(path: &BezPath) -> Option<Rect> {
    (!path.elements().is_empty()).then(|| path.bounding_box())
}

/// The distance from `p` to the outline of `path`.
fn distance_to(path: &BezPath, p: Point) -> f64 {
    path.segments().map(|s| s.nearest(p, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
}

/// Each piece lies on the input it is keyed by and none is a speck.
fn assert_pieces_lie_on_inputs(shapes: &[Shape], arr: &BuilderArrangement) {
    use kurbo::ParamCurve;
    for e in arr.edges.iter().chain(&arr.lines) {
        let len = length(&e.path);
        assert!(len > 1e-3, "a speck of a piece, keyed {}: {:?}", e.key, e.path);
        let input = &shapes[e.key as usize].path;
        for seg in e.path.segments() {
            for k in 0..=4 {
                let p = seg.eval(f64::from(k) / 4.0);
                assert!(distance_to(input, p) < 0.05, "a piece keyed {} strays from its input at {p:?}", e.key);
            }
        }
    }
}

#[test]
fn crossing_lines_enclose_a_face() {
    // Three lines crossing pairwise, their ends sticking out: one triangle.
    let s = [line((0.0, 0.0), (100.0, 100.0), 0), line((100.0, 0.0), (0.0, 100.0), 1), line((-10.0, 80.0), (110.0, 80.0), 2)];
    let (faces, edges) = live_paint(&s);
    assert_eq!(faces.len(), 1, "{faces:?}");
    let f = &faces[0];
    assert!((area(f) - 900.0).abs() < 1e-6, "triangle (20,80) (50,50) (80,80): {}", area(f));
    assert!(f.sources.is_empty(), "nothing fills it yet");
    assert_eq!(subpath_count(&f.path), 1);
    assert_eq!(anchor_count(&f.path), 3, "the loose ends are not part of it");
    // Each line is cut where the others cross it: 3 + 3 + 3 pieces.
    assert_eq!(edges.len(), 9);
    assert_eq!(edges.iter().filter(|e| e.key == 2).count(), 3);
}

#[test]
fn a_grid_of_lines_makes_its_cells() {
    let mut s = vec![];
    for k in 0..4 {
        let v = f64::from(k) * 10.0;
        s.push(line((v, -5.0), (v, 35.0), 0));
        s.push(line((-5.0, v), (35.0, v), 1));
    }
    let (faces, _) = live_paint(&s);
    assert_eq!(faces.len(), 9);
    assert!(faces.iter().all(|f| (area(f) - 100.0).abs() < 1e-6));
    assert_eq!(at(&faces, 15.0, 15.0).len(), 1);
}

#[test]
fn a_line_across_a_shape_splits_it() {
    let s = [rect(0.0, 0.0, 100.0, 50.0, 7), line((40.0, -20.0), (60.0, 70.0), 8)];
    let (faces, edges) = live_paint(&s);
    assert_eq!(faces.len(), 2);
    assert!(faces.iter().all(|f| f.sources == [0]), "both halves keep the rectangle's fill");
    assert!((faces.iter().map(area).sum::<f64>() - 5000.0).abs() < 1e-6);
    assert_eq!(at(&faces, 10.0, 25.0).len(), 1);
    assert_eq!(at(&faces, 90.0, 25.0).len(), 1);
    // The rectangle's outline in two pieces, the line in three.
    assert_eq!(edges.iter().filter(|e| e.key == 7).count(), 2);
    assert_eq!(edges.iter().filter(|e| e.key == 8).count(), 3);
}

#[test]
fn overlapping_shapes_keep_their_coverage() {
    let s = [rect(0.0, 0.0, 100.0, 100.0, 0), rect(50.0, 0.0, 150.0, 100.0, 1)];
    let (faces, _) = live_paint(&s);
    assert_eq!(faces.len(), 3);
    assert_eq!(at(&faces, 25.0, 50.0)[0].sources, [0]);
    assert_eq!(at(&faces, 75.0, 50.0)[0].sources, [0, 1]);
    assert_eq!(at(&faces, 125.0, 50.0)[0].sources, [1]);
    // Same faces as the Pathfinder regions.
    let regions = crate::regions(&s);
    assert_eq!(regions.len(), 3);
    for f in &faces {
        assert!(regions.iter().any(|r| r.sources == f.sources && (area(r).abs() - area(f)).abs() < 1e-6));
    }
}

#[test]
fn a_closed_loop_of_lines_inside_a_shape_is_a_hole_and_a_face() {
    let mut s = vec![rect(0.0, 0.0, 100.0, 100.0, 0)];
    let sq = [(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0), (40.0, 40.0)];
    for w in sq.windows(2) {
        s.push(line(w[0], w[1], 1));
    }
    // A loose line inside: no face of its own, no hole.
    s.push(line((10.0, 10.0), (20.0, 20.0), 2));
    let (faces, _) = live_paint(&s);
    assert_eq!(faces.len(), 2, "{faces:?}");
    let outer = at(&faces, 5.0, 5.0);
    assert_eq!(outer.len(), 1);
    assert_eq!(subpath_count(&outer[0].path), 2, "the inner square is a hole in it");
    assert!((area(outer[0]) - 9600.0).abs() < 1e-6);
    let inner = at(&faces, 50.0, 50.0);
    assert_eq!(inner.len(), 1, "the hole doesn't count as the outer face");
    assert_eq!(inner[0].sources, [0], "the rectangle fills it");
    assert!((area(inner[0]) - 400.0).abs() < 1e-6);
}

#[test]
fn an_area_enclosed_by_shapes_is_an_unfilled_face() {
    // A frame of four bars around a 50 × 50 gap.
    let s = [rect(0.0, 0.0, 70.0, 10.0, 0), rect(0.0, 60.0, 70.0, 70.0, 0), rect(0.0, 0.0, 10.0, 70.0, 0), rect(60.0, 0.0, 70.0, 70.0, 0)];
    let (faces, _) = live_paint(&s);
    let gap = at(&faces, 35.0, 35.0);
    assert_eq!(gap.len(), 1);
    assert!(gap[0].sources.is_empty());
    assert!((area(gap[0]) - 2500.0).abs() < 1e-6);
}

#[test]
fn curves_and_a_circle_crossed_by_a_line() {
    let circle = Shape::new(ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), FillRule::NonZero, 0);
    let (faces, edges) = live_paint(&[circle, line((-10.0, 50.0), (110.0, 50.0), 1)]);
    assert_eq!(faces.len(), 2);
    for f in &faces {
        assert!((area(f) - std::f64::consts::PI * 2500.0 / 2.0).abs() < 2.0, "{}", area(f));
        assert!(anchor_count(&f.path) <= 4, "no extra anchors from the sweep's splits");
    }
    assert_eq!(edges.iter().filter(|e| e.key == 0).count(), 2, "two arcs");
}

fn circle(cx: f64, cy: f64, r: f64, key: u64) -> Shape {
    Shape::new(ellipse(Rect::new(cx - r, cy - r, cx + r, cy + r)), FillRule::NonZero, key)
}

/// Coincident curved boundaries retain the fill coverage of the regions they separate.
#[test]
fn coincident_curves_keep_their_source_faces() {
    fn check(diameter: f64, offset: f64, rule: FillRule, reverse: bool, swap: bool) {
        let mut ring = circle(0.5 * diameter, 0.5 * diameter, 0.5 * diameter, 0);
        let hole = circle(0.5 * diameter, 0.5 * diameter, 0.3 * diameter, 1).path.reverse_subpaths();
        ring.path.extend(hole.iter());
        ring.rule = rule;
        let mut disk = circle(0.5 * diameter + offset, 0.5 * diameter, 0.3 * diameter, 1);
        disk.rule = rule;
        if reverse {
            ring.path = ring.path.reverse_subpaths();
            disk.path = disk.path.reverse_subpaths();
        }
        let shapes = if swap { vec![disk, ring] } else { vec![ring, disk] };
        let (faces, _) = live_paint(&shapes);
        let label = format!("diameter={diameter}, offset={offset}, rule={rule:?}, reverse={reverse}, swap={swap}");
        for (x, source) in [(0.1 * diameter, usize::from(swap)), (0.5 * diameter, usize::from(!swap))] {
            let covering = at(&faces, x, 0.5 * diameter);
            assert_eq!(covering.len(), 1, "{label}: {covering:?}");
            assert_eq!(covering[0].sources, [source], "{label}");
        }
        if offset == 0.0 {
            assert_eq!(faces.len(), 2, "{label}");
            let ring_face = at(&faces, 0.1 * diameter, 0.5 * diameter)[0];
            assert_eq!(subpath_count(&ring_face.path), 2, "{label}: the ring keeps its hole");
        }
    }
    for diameter in [1.0, 0.5, 2.0, 100.0] {
        for offset in [0.0, 0.025, -0.025] {
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                for reverse in [false, true] {
                    for swap in [false, true] {
                        check(diameter, offset, rule, reverse, swap);
                    }
                }
            }
        }
    }
}

#[test]
fn a_line_ending_on_an_edge_splits_the_shape() {
    let s = [line((90.0, 20.0), (70.0, 10.0), 0), rect(80.0, 10.0, 90.0, 50.0, 1)];
    let (faces, _) = live_paint(&s);
    assert_eq!(faces.len(), 2);
    assert!((area(&faces[0]) + area(&faces[1]) - 400.0).abs() < 1e-6);
    for f in &faces {
        let p = crate::interior_point(&f.path).unwrap();
        assert_eq!(at(&faces, p.x, p.y).len(), 1, "{p:?}");
        assert_eq!(f.sources, [1]);
    }
}

#[test]
fn nothing_or_garbage_gives_an_empty_map() {
    assert_eq!(live_paint(&[]), (vec![], vec![]));
    assert!(live_paint(&[line((0.0, 0.0), (f64::NAN, 1.0), 0)]).0.is_empty());
    let (faces, edges) = live_paint(&[line((0.0, 0.0), (10.0, 10.0), 0)]);
    assert!(faces.is_empty());
    assert_eq!(edges.len(), 1);
}

/// `interior_point` answers None, and returns, for a path it cannot place a point in.
#[test]
fn interior_point_of_hostile_paths_is_none() {
    assert_eq!(crate::interior_point(&BezPath::new()), None);
    // Points that are not numbers place nothing (not the origin, not any point).
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let p = polyline(&[(0.0, 0.0), (v, 0.0), (v, v), (0.0, v)], true);
        assert_eq!(crate::interior_point(&p), None, "{v}");
    }
    // Finite but huge ones return, with a point or without.
    for v in [f64::MAX, 1e300] {
        let p = polyline(&[(0.0, 0.0), (v, 0.0), (v, v), (0.0, v)], true);
        let _ = crate::interior_point(&p);
    }
    // A line has no inside.
    assert_eq!(crate::interior_point(&polyline(&[(0.0, 0.0), (10.0, 10.0)], false)), None);
}

/// Faces meeting at a vertex share its exact coordinates (an axonometric grid of lines): no corner
/// has two positions a hair apart, which a union of the faces could not take apart again.
#[test]
fn faces_share_their_corners_exactly() {
    const LINES: &[[f64; 4]] = &[
        [416.57797027866155, 268.45362144397683, 416.5779702786616, 811.5463785560231],
        [243.12292300414873, 511.1134238473054, 713.4550472745127, 782.6598024033286],
        [836.8770769958512, 511.1134238473054, 366.5449527254871, 782.6598024033286],
        [457.718646852441, 268.45362144397683, 457.718646852441, 811.5463785560231],
        [263.6932612910385, 475.4845528055331, 734.0253855614023, 747.0309313615563],
        [816.3067387089613, 475.4845528055331, 345.9746144385975, 747.0309313615563],
        [498.85932342622044, 268.45362144397683, 498.85932342622044, 811.5463785560231],
        [284.26359957792823, 439.8556817637607, 754.5957238482922, 711.4020603197839],
        [795.7364004220717, 439.8556817637607, 325.40427615170773, 711.4020603197839],
        [539.9999999999998, 268.45362144397683, 539.9999999999998, 811.5463785560231],
        [304.83393786481787, 404.22681072198844, 775.1660621351818, 675.7731892780117],
        [775.1660621351821, 404.22681072198844, 304.833937864818, 675.7731892780117],
        [581.1406765737793, 268.45362144397683, 581.1406765737793, 811.5463785560231],
        [325.4042761517076, 368.59793968021614, 795.7364004220715, 640.1443182362394],
        [754.5957238482922, 368.59793968021614, 284.26359957792835, 640.1443182362394],
        [622.2813531475588, 268.45362144397683, 622.2813531475588, 811.5463785560231],
        [345.97461443859737, 332.9690686384438, 816.3067387089613, 604.515447194467],
        [734.0253855614026, 332.9690686384438, 263.6932612910385, 604.515447194467],
        [663.422029721338, 268.45362144397683, 663.422029721338, 811.5463785560231],
        [366.54495272548706, 297.3401975966715, 836.877076995851, 568.8865761526947],
        [713.4550472745128, 297.3401975966715, 243.12292300414884, 568.8865761526947],
    ];
    let s: Vec<Shape> = LINES.iter().enumerate().map(|(i, &[x0, y0, x1, y1])| line((x0, y0), (x1, y1), i as u64)).collect();
    let (faces, _) = live_paint(&s);
    let corners: Vec<Point> = faces.iter().flat_map(|f| f.path.elements().iter().filter_map(kurbo::PathEl::end_point)).collect();
    for p in &corners {
        for q in &corners {
            assert!(p == q || p.distance(*q) > 1e-6, "one corner, two positions: {p:?} {q:?}");
        }
    }
    // Down the strip between the lines x ≈ 498.9 and x = 540: 11 triangles.
    let mut strip: Vec<&Region> = vec![];
    for k in 0..=240 {
        if let Some(f) = faces.iter().find(|f| f.contains(Point::new(519.43, 420.0 + f64::from(k))))
            && !strip.iter().any(|g| std::ptr::eq(*g, f))
        {
            strip.push(f);
        }
    }
    assert_eq!(strip.len(), 11);
    let merged = crate::merge_regions(&strip);
    assert_eq!(subpath_count(&merged), 1, "{merged:?}");
}

#[test]
fn shape_builder_lines_cut_regions() {
    // A line across a rectangle: two regions, the line in three pieces keyed by its index.
    let s = [rect(0.0, 0.0, 100.0, 50.0, 7), line((40.0, -20.0), (60.0, 70.0), 8)];
    let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
    assert_eq!(regions.len(), 2);
    assert!(regions.iter().all(|r| r.sources == [0]));
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|l| l.key == 1 && !has_closed(&l.path)));
    // Lines alone enclose a region nothing fills.
    let s = [line((0.0, 0.0), (100.0, 100.0), 0), line((100.0, 0.0), (0.0, 100.0), 0), line((-10.0, 80.0), (110.0, 80.0), 0)];
    let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].sources.is_empty() && regions[0].top().is_none());
    assert_eq!(lines.len(), 9);
}

#[test]
fn shape_builder_keeps_the_pieces_of_a_line_under_an_outline_in_front() {
    // A line whose left half lies under the bottom edge of a rectangle in front of it.
    let s = [line((0.0, 50.0), (200.0, 50.0), 0), rect(0.0, 0.0, 100.0, 50.0, 1)];
    let BuilderArrangement { lines, edges, lines_under, .. } = shape_builder(&s, true);
    let span = |l: &Shape| bounds(&l.path).map(|b| (b.x0, b.x1));
    assert_eq!(lines.iter().map(span).collect::<Vec<_>>(), [Some((100.0, 200.0))], "{lines:?}");
    assert_eq!(lines_under.iter().map(span).collect::<Vec<_>>(), [Some((0.0, 100.0))], "{lines_under:?}");
    assert!(lines.iter().chain(&lines_under).all(|l| l.key == 0));
    // The rectangle's outline keeps its edge there.
    assert!(!edges.is_empty() && edges.iter().all(|e| e.key == 1 && bounds(&e.path) == Some(Rect::new(0.0, 0.0, 100.0, 50.0))), "{edges:?}");
    // The planar map keeps one edge there, the front-most input's.
    let (_, pieces) = live_paint(&s);
    assert!(!pieces.iter().any(|p| p.key == 0 && bounds(&p.path).is_some_and(|b| b.x1 <= 100.0)), "{pieces:?}");
    // A line in front of the outline has no pieces under it.
    let s = [rect(0.0, 0.0, 100.0, 50.0, 0), line((0.0, 50.0), (200.0, 50.0), 1)];
    assert!(shape_builder(&s, true).lines_under.is_empty());
}

#[test]
fn shape_builder_gives_each_coincident_line_its_pieces() {
    // Two lines along the same stretch, whose left half lies under the bottom edge of a rectangle
    // in front of them: each has its own pieces, under the edge and beyond it. Beyond the edge the
    // front line's piece is the edge, and the back line's lies under it.
    let s = [line((0.0, 50.0), (200.0, 50.0), 0), line((0.0, 50.0), (200.0, 50.0), 1), rect(0.0, 0.0, 100.0, 50.0, 2)];
    let BuilderArrangement { lines, lines_under, .. } = shape_builder(&s, false);
    let spans = |pieces: &[Shape], key: u64| {
        let mut v: Vec<_> = pieces.iter().filter(|l| l.key == key).filter_map(|l| bounds(&l.path).map(|b| (b.x0, b.x1))).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v
    };
    assert_eq!(spans(&lines, 1), [(100.0, 200.0)], "{lines:?}");
    assert_eq!(spans(&lines_under, 1), [(0.0, 100.0)], "{lines_under:?}");
    assert_eq!(spans(&lines, 0), [], "{lines:?}");
    assert_eq!(spans(&lines_under, 0), [(0.0, 100.0), (100.0, 200.0)], "{lines_under:?}");
    // Without the rectangle, both lines are cut where the line across them crosses.
    let s = [line((0.0, 50.0), (200.0, 50.0), 0), line((0.0, 50.0), (200.0, 50.0), 1), line((100.0, 0.0), (100.0, 100.0), 2)];
    let BuilderArrangement { lines, lines_under, .. } = shape_builder(&s, false);
    assert_eq!(spans(&lines, 1), [(0.0, 100.0), (100.0, 200.0)], "{lines:?}");
    assert_eq!(spans(&lines_under, 0), [(0.0, 100.0), (100.0, 200.0)], "{lines_under:?}");
    // The planar map's edges stay one per stretch, the front line's.
    assert_eq!(live_paint(&s).1.iter().filter(|e| e.key == 0).count(), 0);
}

#[test]
fn shape_builder_closed_only_or_degenerate_is_the_filled_arrangement() {
    let s = [rect(0.0, 0.0, 100.0, 100.0, 0), rect(50.0, 0.0, 150.0, 100.0, 1)];
    assert_eq!(shape_builder(&s, false), BuilderArrangement { regions: crate::regions(&s), ..Default::default() });
    // A broken line can't be swept and fills nothing: the filled areas still give their regions.
    let s = [rect(0.0, 0.0, 100.0, 100.0, 0), line((0.0, 0.0), (f64::NAN, 1.0), 1)];
    assert_eq!(shape_builder(&s, false), BuilderArrangement { regions: crate::regions(&s[..1]), ..Default::default() });
    // Zero-length and doubled-back lines don't break it either.
    let s = [
        rect(0.0, 0.0, 100.0, 100.0, 0),
        line((50.0, 50.0), (50.0, 50.0), 1),
        line((0.0, 0.0), (100.0, 0.0), 2),
        line((50.0, 0.0), (50.0, 100.0), 3),
    ];
    let regions = shape_builder(&s, false).regions;
    assert_eq!(regions.len(), 2);
    assert!((regions.iter().map(area).sum::<f64>() - 10000.0).abs() < 1e-6);
}

#[test]
fn shape_builder_offers_the_gaps_closed_shapes_enclose() {
    // Four circles in a 2×2 grid, each touching its neighbours (VectorCraft issue 1691): the four
    // circles and the diamond between them, which nothing fills.
    let s = [circle(50.0, 50.0, 50.0, 0), circle(150.0, 50.0, 50.0, 1), circle(50.0, 150.0, 50.0, 2), circle(150.0, 150.0, 50.0, 3)];
    let regions = shape_builder(&s, false).regions;
    assert_eq!(regions.len(), 5, "{regions:?}");
    let gaps: Vec<&Region> = regions.iter().filter(|r| r.sources.is_empty()).collect();
    assert_eq!(gaps.len(), 1);
    assert!(gaps[0].contains(Point::new(100.0, 100.0)) && !gaps[0].contains(Point::new(50.0, 50.0)));
    let diamond = 10000.0 - std::f64::consts::PI * 2500.0;
    assert!((area(gaps[0]).abs() - diamond).abs() < 0.01 * diamond, "{}", area(gaps[0]));
    assert_eq!(shape_builder(&s, true).regions, regions, "the outline edges don't change them");
    // A ring of four bars that overlap at the corners: the eight pieces of the bars and the
    // square they enclose.
    let s = [rect(0.0, 0.0, 300.0, 100.0, 0), rect(200.0, 0.0, 300.0, 300.0, 1), rect(0.0, 200.0, 300.0, 300.0, 2), rect(0.0, 0.0, 100.0, 300.0, 3)];
    let regions = shape_builder(&s, false).regions;
    assert_eq!(regions.len(), 9, "{regions:?}");
    let gaps: Vec<&Region> = regions.iter().filter(|r| r.sources.is_empty()).collect();
    assert_eq!(gaps.len(), 1);
    assert!((area(gaps[0]).abs() - 10000.0).abs() < 1e-6 && gaps[0].contains(Point::new(150.0, 150.0)));
    // Shapes apart or overlapping without a hole enclose no gap.
    let s = [circle(50.0, 50.0, 50.0, 0), circle(250.0, 50.0, 50.0, 1), rect(80.0, 0.0, 220.0, 60.0, 2)];
    assert!(shape_builder(&s, false).regions.iter().all(|r| !r.sources.is_empty()));
}

/// A ring of `n` points on a wavy circle, closed.
fn wavy_ring(n: usize, waves: f64) -> BezPath {
    let ring: Vec<Point> = (0..n)
        .map(|k| {
            let t = k as f64 / n as f64 * std::f64::consts::TAU;
            let r = 100.0 + 5.0 * (t * waves).sin();
            Point::new(r * t.cos(), r * t.sin())
        })
        .collect();
    polyline_pts(&ring, true)
}

#[test]
fn shape_builder_at_its_cap() {
    // A wavy ring of nearly the most segments the map takes, crossed by a few lines.
    let n = SHAPE_BUILDER_MAX_SEGMENTS - 8;
    let mut s = vec![Shape::new(wavy_ring(n, 200.0), FillRule::NonZero, 0)];
    // Closed paths alone are swept for the gaps they enclose.
    crate::work::reset();
    assert_eq!(shape_builder(&s, false).regions.len(), 1, "the ring encloses no gap");
    // Reads of the index stay in proportion to the ring: scanning the ring for each edge's source would read about `n` entries per edge
    // (VectorCraft 4cf4ea3 counts these reads where it timed the call).
    let most = 384 * n as u64;
    let reads = || (crate::work::take(crate::work::Work::IndexCandidates) + crate::work::take(crate::work::Work::IndexNodes)) as u64;
    let work = reads();
    assert!(work <= most, "{work} reads");
    for k in 0..4 {
        let y = -60.0 + 40.0 * f64::from(k);
        s.push(line((-150.0, y), (150.0, y + 7.0), 1));
    }
    let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
    assert!(regions.len() >= 5, "the ring cut in five bands at least (the waves make pockets)");
    assert!(lines.len() >= 12, "{}", lines.len());
    let work = reads();
    assert!(work <= most, "{work} reads");
    // Past the cap, only the filled areas count.
    s.push(line((0.0, 0.0), (1.0, 1.0), 2));
    let mut over = s[0].path.clone();
    over.move_to((100.0, 0.0));
    for k in 1..=16 {
        over.line_to((100.0, f64::from(k)));
    }
    s[0].path = over;
    assert!(shape_builder(&s, false).lines.is_empty());
}

#[test]
fn a_traced_outline_of_many_segments_is_cut_by_lines() {
    // VectorCraft issue 937: art traced to thousands of segments was past the old cap (4096), so
    // lines stopped cutting it and `shape_builder` offered the whole shapes. A blob of 20 000
    // segments crossed by two long lines (far wider than any of its segments: the index of the
    // outlines keeps those apart) splits into four regions, each line in four pieces.
    let s = [Shape::new(wavy_ring(20_000, 37.0), FillRule::NonZero, 0), line((-150.0, 0.0), (150.0, 3.0), 1), line((0.0, -150.0), (3.0, 150.0), 2)];
    let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
    assert_eq!(regions.len(), 4, "{}", regions.len());
    assert!(regions.iter().all(|r| r.sources == [0]));
    let whole = crate::area(&s[0].path, FillRule::NonZero);
    assert!((regions.iter().map(area).sum::<f64>() - whole).abs() < 1e-6 * whole);
    assert_eq!(lines.iter().filter(|l| l.key == 1).count(), 4, "cut twice by the blob and once by the other line");
    assert_eq!(lines.iter().filter(|l| l.key == 2).count(), 4);
}

#[test]
fn a_hatch_of_parallel_lines_is_cut_line_by_line() {
    // Hundreds of long horizontal lines over a square, all alike in x (the weak axis of an index
    // by x alone), and one vertical line: every strip is cut in two.
    let n: u32 = 300;
    let mut s = vec![rect(0.0, 0.0, 100.0, 100.0, 0)];
    for k in 1..n {
        let y = 100.0 * f64::from(k) / f64::from(n);
        s.push(line((-10.0, y), (110.0, y), u64::from(k)));
    }
    s.push(line((50.0, -10.0), (50.0, 110.0), u64::from(n)));
    let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
    assert_eq!(regions.len(), 2 * n as usize, "{}", regions.len());
    assert!(regions.iter().all(|r| r.sources == [0]));
    assert!((regions.iter().map(area).sum::<f64>() - 10000.0).abs() < 1e-6);
    // Each horizontal line: a loose end outside, two pieces inside, a loose end outside.
    assert_eq!(lines.iter().filter(|l| l.key == 1).count(), 4);
    // The vertical line: cut by the square's sides and every hatch line.
    assert_eq!(lines.iter().filter(|l| l.key == u64::from(n)).count(), n as usize + 2);
}

#[test]
fn rect_index_finds_every_rectangle_holding_a_point() {
    // Narrow in x, narrow in y only, and wide both ways: each lookup matches a scan of them all.
    let mut items = vec![(kurbo::Rect::new(0.0, 0.0, 100.0, 100.0), 0usize)];
    for k in 1..50 {
        let a = f64::from(k as u32) * 2.0;
        items.push((kurbo::Rect::new(a, 0.0, a + 1.0, 100.0), k));
        items.push((kurbo::Rect::new(0.0, a, 100.0, a + 1.0), 100 + k));
    }
    items.push((kurbo::Rect::new(f64::NAN, 0.0, 1.0, 1.0), 999));
    let index = RectIndex::new(items.clone());
    for p in [Point::new(4.5, 6.5), Point::new(50.0, 50.0), Point::new(-1.0, 3.0), Point::new(99.5, 0.5)] {
        let mut got: Vec<usize> = index.at(p, 0.1).copied().collect();
        got.sort_unstable();
        let mut want: Vec<usize> = items.iter().filter(|(r, _)| r.inflate(0.1, 0.1).contains(p)).map(|(_, k)| *k).collect();
        want.sort_unstable();
        assert_eq!(got, want, "{p:?}");
    }
    assert_eq!(RectIndex::<usize>::new(Vec::new()).at(Point::ZERO, 1.0).count(), 0);
}

#[test]
fn rect_index_reads_a_diagonal_hatch_only_near_the_point() {
    // 2,000 parallel diagonals a tenth of a point apart: the bounds of every line left of a
    // point hold it, so an index of the bounds alone handed half the hatch to each lookup
    // (quadratic in the hatch, VectorCraft issue 937). Filed by pieces, a lookup gets the lines of its
    // neighbourhood (a few cells' worth), among them every line through the point.
    let n = 2000;
    let items: Vec<_> = (0..n)
        .map(|k| {
            let d = f64::from(k) * 0.1;
            let s = PathSeg::Line(kurbo::Line::new((d, 0.0), (d + 200.0, 200.0)));
            (s.bounding_box(), (s, k))
        })
        .collect();
    let index = RectIndex::with_footprint(items.clone(), |r, (s, _), cell| segment_footprint(r, s, cell));
    for p in [Point::new(100.0, 100.0), Point::new(130.25, 60.0), Point::new(250.0, 150.0), Point::new(399.9, 200.0)] {
        let got: Vec<u32> = index.at(p, 0.1).map(|(_, k)| *k).collect();
        assert!(got.len() <= n as usize / 4, "{p:?}: {} candidates", got.len());
        let through: Vec<u32> =
            items.iter().filter(|(_, (s, _))| kurbo::ParamCurveNearest::nearest(s, p, 1e-9).distance_sq <= 0.01).map(|(_, (_, k))| *k).collect();
        assert!(!through.is_empty() && through.iter().all(|k| got.contains(k)), "{p:?}: {through:?} vs {got:?}");
    }
    // Faces a hatch cuts a square into: thin stripes across the whole grid, each filed under
    // its own column of cells, so a lookup reads a few of them, not the hatch.
    let stripes: Vec<_> = (0..n).map(|k| (kurbo::Rect::new(f64::from(k) * 0.1, 0.0, f64::from(k) * 0.1 + 0.1, 200.0), k)).collect();
    let index = RectIndex::new(stripes);
    let got: Vec<u32> = index.at(Point::new(100.05, 50.0), 0.0).copied().collect();
    assert_eq!(got, [1000]);
    assert!(index.rest.is_empty());
    // A footprint still finds a point on the item but off the grid's cells' edges, and a
    // degenerate grid (collinear items) is divided along its length.
    let line = PathSeg::Line(kurbo::Line::new((0.0, 0.0), (1000.0, 0.0)));
    let items: Vec<_> = (0..10).map(|k| (line.bounding_box(), (line, k))).collect();
    let index = RectIndex::with_footprint(items, |r, (s, _), cell| segment_footprint(r, s, cell));
    assert_eq!(index.at(Point::new(333.3, 0.0), 0.1).count(), 10);
    assert_eq!(index.at(Point::new(1000.0, 0.0), 0.1).count(), 10);
    assert_eq!(index.at(Point::new(-5.0, 0.0), 0.1).count(), 0);
}

/// The index's memory stays linear in the item count: a segment is filed as the bounds of its
/// pieces, and coincident segments (each long as the whole column) are filed under their bounds once
/// the index holds its budget of rectangles, and are still all found, where their bounds hold the point.
/// A lookup counts each entry it reads, an entry filed under several of the places it spans once for each (VectorCraft 500f74b: the
/// count was made after the repeats were removed, so a line filed under several cells counted once): ten identical long lines, each
/// filed in pieces, are ten found and more than ten read.
#[test]
fn rect_index_counts_each_entry_it_reads() {
    let line = PathSeg::Line(kurbo::Line::new((0.0, 0.0), (1000.0, 0.0)));
    let items: Vec<_> = (0..10).map(|k| (line.bounding_box(), (line, k))).collect();
    let index = RectIndex::with_footprint(items, |r, (s, _), cell| segment_footprint(r, s, cell));
    work::reset();
    assert_eq!(index.at(Point::new(500.0, 0.0), 300.0).count(), 10);
    let (candidates, nodes) = (work::take(Work::IndexCandidates), work::take(Work::IndexNodes));
    // Each line is filed under 8 of the places along it (80 entries read in 8 nodes): all of them are counted, not each line once.
    assert!(nodes >= 1 && candidates > 4 * 10, "{candidates} candidates read in {nodes} nodes");
}

#[test]
fn rect_index_holds_a_bounded_number_of_references() {
    // Ten collinear horizontal lines, cut into pieces a tenth of their length.
    let line = PathSeg::Line(kurbo::Line::new((0.0, 0.0), (1000.0, 0.0)));
    let items: Vec<_> = (0..10).map(|k| (line.bounding_box(), (line, k))).collect();
    let index = RectIndex::with_footprint(items, |r, (s, _), cell| segment_footprint(r, s, cell));
    assert!(index.rest.is_empty(), "every line is filed, {} scanned", index.rest.len());
    assert!((10..=1000).contains(&index.entries.len()), "{} rectangles", index.entries.len());
    assert_eq!(index.at(Point::new(500.0, 0.0), 0.1).count(), 10);
    // 4,096 coincident vertical segments, each of 4,096 pieces: 16 million rectangles filed whole;
    // the budget keeps it to a few hundred thousand, and the segments past it are filed as their bounds.
    let n = 4096u32;
    let seg = PathSeg::Line(kurbo::Line::new((50.0, 0.0), (50.0, 200.0)));
    let items: Vec<_> = (0..n).map(|k| (seg.bounding_box(), (seg, k))).collect();
    let index = RectIndex::with_footprint(items, |r, (s, _), cell| segment_footprint(r, s, cell));
    assert!(index.entries.len() <= n as usize * 65 + 4096, "{} rectangles", index.entries.len());
    assert!(index.rest.is_empty(), "{} scanned", index.rest.len());
    let mut got: Vec<u32> = index.at(Point::new(50.0, 100.0), 0.1).map(|(_, k)| *k).collect();
    got.sort_unstable();
    assert_eq!(got, (0..n).collect::<Vec<_>>());
    // A point their bounds do not hold reads none of them (scanned, each was read at every lookup).
    work::reset();
    assert_eq!(index.at(Point::new(60.0, 100.0), 0.1).count(), 0);
    assert!(work::take(Work::IndexCandidates) < 64, "the segments past the budget are not read where they are not");
}

/// One shape far from the rest, or a second cluster far from the first, does not make a lookup
/// read more: the tree follows where the rectangles are (a uniform grid over their union put the
/// 10,000 squares below in one cell, and a lookup read them all).
#[test]
fn rect_index_is_not_stretched_by_shapes_far_away() {
    let square = |x: f64, y: f64| kurbo::Rect::new(x, y, x + 1.0, y + 1.0);
    let mut items = Vec::new();
    for i in 0..100 {
        for j in 0..100 {
            items.push((square(f64::from(i) * 2.0, f64::from(j) * 2.0), (i, j)));
        }
    }
    let far = [(1.0e9, -1.0e9), (-3.0e12, 2.0e12), (1.0e15, 5.0)];
    let count = |items: Vec<(kurbo::Rect, (i32, i32))>| {
        let index = RectIndex::new(items);
        work::reset();
        let mut found = 0;
        for k in 0..200 {
            let p = Point::new(f64::from(k % 100) * 2.0 + 0.5, f64::from(k / 2) * 2.0 + 0.5);
            found += index.at(p, 0.0).count();
        }
        assert_eq!(found, 200, "each point is in one square");
        (work::take(Work::IndexNodes), work::take(Work::IndexCandidates))
    };
    let (nodes, read) = count(items.clone());
    // A lookup visits a path of the tree and a few boxes beside it, and reads a few leaves.
    assert!(nodes <= 200 * 60 && read <= 200 * 200, "{nodes} nodes, {read} read");
    for (k, &(x, y)) in far.iter().enumerate() {
        let mut with_far = items.clone();
        with_far.push((square(x, y), (-1, k as i32)));
        let (far_nodes, far_read) = count(with_far.clone());
        assert!(far_nodes <= 2 * nodes + 400 && far_read <= 2 * read + 400, "{x}: {far_nodes} nodes, {far_read} read, against {nodes}, {read}");
        // And the far one is found.
        let index = RectIndex::new(with_far);
        assert_eq!(index.at(Point::new(x + 0.5, y + 0.5), 0.0).copied().collect::<Vec<_>>(), [(-1, k as i32)]);
    }
    // A second cluster: the same grid a billion away.
    let mut two = items.clone();
    two.extend(items.iter().map(|(r, (i, j))| (*r + kurbo::Vec2::new(1.0e9, 1.0e9), (i + 1000, *j))));
    let (two_nodes, two_read) = count(two);
    assert!(two_nodes <= 2 * nodes + 400 && two_read <= 2 * read + 400, "{two_nodes} nodes, {two_read} read");
}

/// The side a long segment is cut at follows the middle of the art, not its extent, so a hatch
/// with one segment far away is still read by its neighbourhood.
#[test]
fn rect_index_files_a_hatch_by_pieces_when_a_segment_is_far_away() {
    let n = 2000u32;
    let mut items: Vec<_> = (0..n)
        .map(|k| {
            let d = f64::from(k) * 0.1;
            let s = PathSeg::Line(kurbo::Line::new((d, 0.0), (d + 200.0, 200.0)));
            (s.bounding_box(), (s, k))
        })
        .collect();
    let s = PathSeg::Line(kurbo::Line::new((1.0e9, 1.0e9), (1.0e9 + 1.0, 1.0e9 + 1.0)));
    items.push((s.bounding_box(), (s, n)));
    let index = RectIndex::with_footprint(items, |r, (s, _), cell| segment_footprint(r, s, cell));
    let got: Vec<u32> = index.at(Point::new(100.0, 100.0), 0.1).map(|(_, k)| *k).collect();
    assert!(!got.is_empty() && got.len() <= n as usize / 4, "{} candidates", got.len());
    assert_eq!(index.at(Point::new(1.0e9 + 0.5, 1.0e9 + 0.5), 0.1).map(|(_, k)| *k).collect::<Vec<_>>(), [n]);
}

#[test]
fn a_line_crossed_where_its_two_passes_part_is_cut_once() {
    // The shrunk case of `pieces_lie_on_their_inputs`: the sweep cut the first line's way there
    // and its way back by the third line a rounding error (1.2e-6) apart, joined by a stub. The
    // line came back as a speck keyed by the third line and a piece running from the crossing out
    // to (10, 70) and back to the other cut, which erasing took twice over.
    let s = [line((10.0, 70.0), (80.0, 20.0), 0), line((10.0, 40.0), (70.0, 90.0), 0), line((20.0, 80.0), (20.0, 50.0), 0)];
    let arr = shape_builder(&s, true);
    assert_pieces_lie_on_inputs(&s, &arr);
    // The first line cut by both others, the other two once each.
    let count = |k: u64| arr.lines.iter().filter(|l| l.key == k).count();
    assert_eq!((count(0), count(1), count(2)), (3, 2, 2), "{:?}", arr.lines);
    let at = Point::new(20.0, 70.0 - 50.0 / 7.0);
    for k in [0, 2] {
        assert!(arr.lines.iter().filter(|l| l.key == k).all(|l| {
            let (a, b) = ends(&l.path);
            a.distance(b) > 1.0 && l.path.segments().count() == 1
        }));
        assert_eq!(arr.lines.iter().filter(|l| l.key == k && [ends(&l.path).0, ends(&l.path).1].iter().any(|p| p.distance(at) < 1e-9)).count(), 2);
    }
}

#[test]
fn a_run_of_tiny_level_segments_is_not_contracted_as_stubs() {
    // 2000 segments of 1e-4 (0.2 in all), nearly level: each one alone is as short as a
    // stub the sweep leaves between two cuts, but chained they are real geometry, and the
    // stub contraction must not pull them into one vertex.
    let pts: Vec<(f64, f64)> = (0..=2000).map(|k| (f64::from(k) * 1e-4, f64::from(k) * 1e-8)).collect();
    let run = Shape::new(polyline(&pts, false), FillRule::NonZero, 0);
    let whole = length(&run.path);
    let s = [run, line((0.1003, -1.0), (0.1003, 1.0), 0)];
    let arr = shape_builder(&s, false);
    let pieces: Vec<&Shape> = arr.lines.iter().filter(|l| l.key == 0).collect();
    assert_eq!(pieces.len(), 2, "cut once by the line: {}", pieces.len());
    let total: f64 = pieces.iter().map(|l| length(&l.path)).sum();
    // Where the line crosses, one segment may be taken for a stub (it is as short as one),
    // moving the cut by less than a stub's reach; the rest of the run stays.
    assert!((total - whole).abs() <= DEFAULT_PRECISION / 16.0, "{total} of {whole}");
    for l in &pieces {
        let (a, b) = ends(&l.path);
        assert!(a.distance(b) > 0.09, "a piece collapsed: {a:?} {b:?}");
    }
    assert_eq!(arr.lines.iter().filter(|l| l.key == 1).count(), 2);
}

#[test]
fn a_dense_hatch_at_exact_binary_steps_cuts_each_line_once_per_crossing() {
    // Lines every 0.25 or 0.5 (exact in binary) cut a crossing line or a circle where two
    // passes of each line met it a rounding error apart, adding a speck and a piece there.
    let hatch = |n: i32, step: f64, f: &dyn Fn(f64) -> Shape| -> Vec<Shape> { (1..=n).map(|k| f(step * f64::from(k))).collect() };
    // Horizontal lines across a diagonal: each in two pieces, the diagonal in n + 1.
    let mut s = vec![line((-3.0, -2.0), (33.0, 31.0), 0)];
    s.extend(hatch(119, 0.25, &|y| line((-5.0, y), (35.0, y), 0)));
    let arr = shape_builder(&s, false);
    assert_pieces_lie_on_inputs(&s, &arr);
    assert_eq!(arr.lines.iter().filter(|l| l.key == 0).count(), 120);
    assert!((1..=119).all(|k| arr.lines.iter().filter(|l| l.key == k).count() == 2));
    assert_eq!(arr.lines.len(), 120 + 2 * 119);
    // Sloping lines across a 45° one.
    let mut s = vec![line((-3.0, -3.0), (33.0, 33.0), 0)];
    s.extend(hatch(59, 0.5, &|v| line((v - 40.0, -5.0), (v + 40.0, 35.0), 0)));
    let arr = shape_builder(&s, false);
    assert_pieces_lie_on_inputs(&s, &arr);
    assert_eq!(arr.lines.len(), 60 + 2 * 59, "{}", arr.lines.len());
    // Vertical lines across a circle: n + 1 strips, each line in three pieces, the circle's
    // outline in two per line.
    let mut s = vec![Shape::new(ellipse(Rect::new(0.0, 0.0, 30.0, 30.0)), FillRule::NonZero, 0)];
    s.extend(hatch(119, 0.25, &|x| line((x, -5.0), (x, 35.0), 0)));
    let arr = shape_builder(&s, true);
    assert_pieces_lie_on_inputs(&s, &arr);
    assert_eq!(arr.regions.len(), 120);
    assert!(arr.regions.iter().all(|r| r.sources == [0]));
    assert!((1..=119).all(|k| arr.lines.iter().filter(|l| l.key == k).count() == 3));
    assert_eq!(arr.lines.len(), 3 * 119);
    assert_eq!(arr.edges.len(), 2 * 119);
}

#[test]
fn shape_builder_edges_are_the_outlines_cut_where_paths_meet() {
    // Two overlapping squares: each outline in two pieces, cut where the other crosses it.
    let s = [rect(0.0, 0.0, 100.0, 100.0, 0), rect(50.0, 50.0, 150.0, 150.0, 1)];
    let with = shape_builder(&s, true);
    assert_eq!(with.regions, shape_builder(&s, false).regions, "the regions don't change");
    assert!(with.lines.is_empty() && shape_builder(&s, false).edges.is_empty());
    assert_eq!(with.edges.len(), 4);
    for key in [0, 1] {
        assert_eq!(with.edges.iter().filter(|e| e.key == key && !has_closed(&e.path)).count(), 2);
    }
    assert_pieces_lie_on_inputs(&s, &with);
    let inner = with.edges.iter().find(|e| e.key == 0 && bounds(&e.path) == Some(Rect::new(50.0, 50.0, 100.0, 100.0))).expect("A inside B");
    let (a, b) = ends(&inner.path);
    assert!([a, b].contains(&Point::new(100.0, 50.0)) && [a, b].contains(&Point::new(50.0, 100.0)), "{a:?} {b:?}");
    assert!((length(&inner.path) - 100.0).abs() < 1e-6, "{}", length(&inner.path));
    // A lone square's edge is its whole outline.
    let lone = shape_builder(&s[..1], true);
    assert_eq!(lone.edges.len(), 1);
    assert!((length(&lone.edges[0].path) - 400.0).abs() < 1e-6);
}

/// Proptest shrank a failure of `pieces_lie_on_their_inputs` to this input with linesweeper 0.5.
/// The sweep cuts lines 0 and 1 at points of their crossing up to 1.4 `eps` apart, and `vertex_id`
/// merges a point only into a vertex within `eps` of it. Line 0's loose end in the square is
/// still one piece. No face runs out along it and back.
#[test]
fn a_loose_end_is_one_piece_when_the_sweep_cuts_its_passes_apart() {
    let s = [line((30.0, 20.0), (70.0, 0.0), 0), line((40.0, 0.0), (0.0, 90.0), 1), rect(10.0, 0.0, 40.0, 30.0, 2)];
    let arr = shape_builder(&s, true);
    for key in 0..3 {
        assert_eq!(arr.edges.iter().chain(&arr.lines).filter(|e| e.key == key).count(), 3, "pieces of {key}");
    }
    assert_pieces_lie_on_inputs(&s, &arr);
    for key in 0..2 {
        let input = length(&s[key as usize].path);
        let pieces: f64 = arr.edges.iter().chain(&arr.lines).filter(|e| e.key == key).map(|e| length(&e.path)).sum();
        assert!((input - pieces).abs() < 1e-6, "the pieces of {key} add up to it: {pieces} of {input}");
    }
    // From (30,20) to the crossing at (220/7, 135/7).
    let loose =
        arr.lines.iter().find(|l| ends(&l.path).0 == Point::new(30.0, 20.0) || ends(&l.path).1 == Point::new(30.0, 20.0)).expect("the loose end");
    assert_eq!(loose.path.segments().count(), 1, "{:?}", loose.path);
    assert!((length(&loose.path) - 2000f64.sqrt() / 28.0).abs() < 1e-6, "{}", length(&loose.path));
    let (faces, _) = live_paint(&s);
    let left = at(&faces, 15.0, 15.0);
    assert_eq!(left.len(), 1);
    assert_eq!(anchor_count(&left[0].path), 4, "(10,0) (40,0) (26.67,30) (10,30): {:?}", left[0].path);
}

fn arb_segment() -> impl Strategy<Value = Shape> {
    let c = || (0..10i32).prop_map(|v| f64::from(v) * 10.0);
    prop_oneof![
        (c(), c(), c(), c()).prop_map(|(a, b, x, y)| line((a, b), (x, y), 0)),
        (c(), c(), 1..5i32, 1..5i32).prop_map(|(x, y, w, h)| rect(x, y, x + f64::from(w) * 10.0, y + f64::from(h) * 10.0, 0)),
        (c(), c(), 1..5i32).prop_map(|(x, y, r)| {
            let r = f64::from(r) * 10.0;
            Shape::new(ellipse(Rect::new(x, y, x + r, y + r)), FillRule::EvenOdd, 0)
        }),
    ]
}

/// Whether `p` is inside the filled area of `s` (open paths are not filled here).
fn fill_contains(s: &Shape, p: Point) -> bool {
    subs(&s.path).iter().all(|sp| sp.closed) && crate::boolean::inside(s.rule, s.path.winding(p))
}

/// A property check's verdict: what is wrong, or nothing.
type Check = Result<(), String>;

/// Faces never overlap, and a face's sources are exactly the inputs whose fill holds a point
/// inside it.
fn check_faces_are_disjoint_and_know_what_covers_them(shapes: &[Shape]) -> Check {
    let (faces, _) = live_paint(shapes);
    for f in &faces {
        if area(f) <= 0.0 {
            return Err(format!("a face of area {}", area(f)));
        }
        let Some(p) = crate::interior_point(&f.path) else { continue };
        let in_faces = faces.iter().filter(|g| g.contains(p)).count();
        if in_faces != 1 {
            return Err(format!("{p:?} is in {in_faces} faces"));
        }
        let covering: Vec<usize> = shapes.iter().enumerate().filter(|(_, s)| fill_contains(s, p)).map(|(i, _)| i).collect();
        if f.sources != covering {
            return Err(format!("at {p:?}: sources {:?}, covering {covering:?}", f.sources));
        }
    }
    Ok(())
}

/// Closed shapes alone: the filled faces cover exactly the Pathfinder regions.
fn check_filled_faces_are_the_pathfinder_regions(shapes: &[Shape]) -> Check {
    let closed: Vec<Shape> = shapes.iter().filter(|s| subs(&s.path).iter().all(|sp| sp.closed)).cloned().collect();
    let (faces, _) = live_paint(&closed);
    let filled: f64 = faces.iter().filter(|f| !f.sources.is_empty()).map(area).sum();
    let regions: f64 = crate::regions(&closed).iter().map(|r| area(r).abs()).sum();
    // Both refit split curves to [`DEFAULT_PRECISION`], so curved areas may differ by up to that
    // much along every outline.
    let outlines: f64 = closed.iter().map(|s| length(&s.path)).sum();
    if (filled - regions).abs() <= 1e-9 + DEFAULT_PRECISION * outlines { Ok(()) } else { Err(format!("{filled} vs {regions}")) }
}

/// The length of `input` within `tol` of `piece`, measured at samples every 0.02 along it, each
/// weighing the distance between the ends of its stretch (a Bézier circle is not drawn at a
/// steady speed, so equal steps of the parameter are not equal steps along it).
fn length_near(input: &BezPath, piece: &BezPath, tol: f64) -> f64 {
    use kurbo::{ParamCurve, ParamCurveArclen};
    let mut near = 0.0;
    for seg in input.segments() {
        let n = ((seg.arclen(1e-6) / 0.02).ceil() as usize).clamp(1, 100_000);
        let at = |k: usize| seg.eval(k as f64 / n as f64);
        near += (0..n).filter(|&k| distance_to(piece, seg.eval((k as f64 + 0.5) / n as f64)) < tol).map(|k| at(k).distance(at(k + 1))).sum::<f64>();
    }
    near
}

/// Does the length of input within reach of a piece (`taken`) fit a piece of length `len`? The
/// input runs on a little past each end of the piece (0.05 over the sine of the angle it crosses
/// another path at), which is why `taken` may exceed `len` by more than rounding; a piece that
/// runs out and back is caught by [`retraces`], not by this.
fn taken_fits(taken: f64, len: f64) -> bool {
    taken + 0.1 + 0.005 * len >= len && taken <= len + 3.0 + 0.1 * len
}

/// Does a subpath of `path` run back over itself: some point of it, `tol` or more further along it
/// than another that lies within half of `tol` of that point, with the path heading the opposite
/// way (a turn of over 154 degrees)? Found at samples `tol / 2` apart, each compared with those in
/// its own square of `tol` and the eight around, so that it costs what the path's length does, and
/// any out-and-back stub longer than `tol` is found, whatever its length or the length of the
/// piece it is in (one of a quarter of `tol` may be missed, between the samples).
fn retraces(path: &BezPath, tol: f64) -> bool {
    use kurbo::{ParamCurve, ParamCurveArclen};
    // The segments of each subpath.
    let mut subpaths: Vec<Vec<kurbo::PathSeg>> = Vec::new();
    let (mut start, mut current) = (Point::ZERO, Point::ZERO);
    for el in path.iter() {
        let seg = match el {
            kurbo::PathEl::MoveTo(p) => {
                subpaths.push(Vec::new());
                (start, current) = (p, p);
                continue;
            }
            kurbo::PathEl::ClosePath => {
                let closing = (current != start).then(|| kurbo::PathSeg::Line(kurbo::Line::new(current, start)));
                current = start;
                match closing {
                    Some(seg) => seg,
                    None => continue,
                }
            }
            kurbo::PathEl::LineTo(p) => kurbo::PathSeg::Line(kurbo::Line::new(current, p)),
            kurbo::PathEl::QuadTo(a, p) => kurbo::PathSeg::Quad(kurbo::QuadBez::new(current, a, p)),
            kurbo::PathEl::CurveTo(a, b, p) => kurbo::PathSeg::Cubic(kurbo::CubicBez::new(current, a, b, p)),
        };
        current = seg.end();
        if let Some(sub) = subpaths.last_mut() {
            sub.push(seg);
        }
    }
    for segs in &subpaths {
        // (position along the path, where, which way) of every sample.
        let mut samples: Vec<(f64, Point, kurbo::Vec2)> = Vec::new();
        let mut along = 0.0;
        for seg in segs {
            let len = seg.arclen(1e-6);
            let n = ((len / (tol / 2.0)).ceil() as usize).clamp(1, 1_000_000);
            for k in 0..=n {
                let t = k as f64 / n as f64;
                let (p, q) = (seg.eval(t), seg.eval((t + 1e-4).min(1.0)));
                let back = seg.eval((t - 1e-4).max(0.0));
                let heading = q - back;
                let norm = heading.hypot();
                if norm > 0.0 {
                    samples.push((along + len * t, p, heading / norm));
                }
            }
            along += len;
        }
        let cell = |p: Point| ((p.x / tol).floor() as i64, (p.y / tol).floor() as i64);
        let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
        for (k, (_, p, _)) in samples.iter().enumerate() {
            grid.entry(cell(*p)).or_default().push(k);
        }
        for (a, (pos, p, heading)) in samples.iter().enumerate() {
            let (cx, cy) = cell(*p);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for &b in grid.get(&(cx + dx, cy + dy)).into_iter().flatten().filter(|&&b| b > a) {
                        let Some((other_pos, q, other_heading)) = samples.get(b) else { continue };
                        if other_pos - pos >= tol && p.distance(*q) < tol / 2.0 && heading.dot(*other_heading) < -0.9 {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

#[test]
fn a_piece_out_and_back_along_its_input_does_not_fit_and_one_along_it_does() {
    let input = polyline(&[(0.0, 0.0), (10.0, 0.0)], false);
    let along = polyline(&[(2.0, 0.0), (8.0, 0.0)], false);
    let out_and_back = polyline(&[(2.0, 0.0), (8.0, 0.0), (2.0, 0.0)], false);
    let (taken_along, taken_out_and_back) = (length_near(&input, &along, 0.05), length_near(&input, &out_and_back, 0.05));
    assert!((taken_along - 6.1).abs() < 0.05, "{taken_along}");
    assert!(taken_fits(taken_along, length(&along)));
    assert!((taken_out_and_back - 6.1).abs() < 0.05, "{taken_out_and_back}");
    assert!(!taken_fits(taken_out_and_back, length(&out_and_back)), "12 long, taking 6.1 of the input");
}

/// An out-and-back stub of any length over the tolerance is found, in a piece of any length, at
/// its end or in the middle, and a piece that does not turn back is not: along a line, round a
/// circle, round a closed path that returns to its start, and round a corner.
#[test]
fn retracing_is_found_for_any_stub_over_the_tolerance_and_nothing_else() {
    for stub in [0.06, 0.1, 0.3, 0.9, 5.0] {
        for (name, pts) in [
            ("at the end", vec![(0.0, 0.0), (40.0, 0.0), (40.0 + stub, 0.0), (40.0, 0.0)]),
            ("in the middle", vec![(0.0, 0.0), (20.0, 0.0), (20.0 + stub, 0.0), (20.0, 0.0), (40.0, 0.0)]),
            ("at the start", vec![(stub, 0.0), (0.0, 0.0), (40.0, 0.0)]),
            ("a diagonal", vec![(0.0, 0.0), (30.0, 30.0), (30.0 + stub * 0.6, 30.0 + stub * 0.8), (30.0, 30.0), (50.0, 50.0)]),
        ] {
            assert!(retraces(&polyline(&pts, false), 0.05), "a stub of {stub} {name}");
        }
    }
    assert!(retraces(&polyline(&[(0.0, 0.0), (8.0, 0.0), (0.0, 0.0)], false), 0.05), "out and back whole");
    for (name, path) in [
        ("a line", polyline(&[(0.0, 0.0), (40.0, 0.0)], false)),
        ("a corner", polyline(&[(0.0, 0.0), (40.0, 0.0), (40.0, 30.0)], false)),
        ("a right turn of a closed path", rect_path(0.0, 0.0, 30.0, 20.0)),
        ("a circle", ellipse(Rect::new(0.0, 0.0, 30.0, 30.0))),
        ("a stub of a hundredth, under the tolerance", polyline(&[(0.0, 0.0), (20.0, 0.0), (20.01, 0.0), (20.0, 0.0), (40.0, 0.0)], false)),
    ] {
        assert!(!retraces(&path, 0.05), "{name}");
    }
}

/// One piece, of `len` (over 0.05) long, against the path `input` it is keyed by: it lies on it,
/// takes its own length of it, and does not run back over itself.
fn check_piece_fits_input(input: &BezPath, piece: &BezPath, key: u64) -> Check {
    use kurbo::ParamCurve;
    for seg in piece.segments() {
        for k in 0..=4 {
            let p = seg.eval(f64::from(k) / 4.0);
            if distance_to(input, p) >= 0.05 {
                return Err(format!("a piece keyed {key} strays from its input at {p:?}"));
            }
        }
    }
    let len = length(piece);
    // The input within 0.05 of the piece is the piece's length of it, and a little more: at each
    // end it runs on for 0.05 over the sine of the angle it crosses the other path at.
    let taken = length_near(input, piece, 0.05);
    if !taken_fits(taken, len) {
        return Err(format!("the input {key} takes {taken} for a piece {len} long"));
    }
    // A piece out and back along it takes only its way out of it, and any stub over the tolerance
    // is a retrace, which the length alone passes while the stub is short against the piece.
    if retraces(piece, 0.05) {
        return Err(format!("a piece keyed {key} runs back over itself ({len} long)"));
    }
    Ok(())
}

#[test]
fn a_piece_with_a_stub_of_any_length_over_the_tolerance_does_not_fit_its_input() {
    let input = polyline(&[(0.0, 0.0), (100.0, 0.0)], false);
    assert_eq!(check_piece_fits_input(&input, &polyline(&[(10.0, 0.0), (90.0, 0.0)], false), 7), Ok(()));
    for stub in [0.06, 0.2, 0.5, 0.99, 2.0, 10.0] {
        let piece = polyline(&[(10.0, 0.0), (50.0, 0.0), (50.0 + stub, 0.0), (50.0, 0.0), (90.0, 0.0)], false);
        let verdict = check_piece_fits_input(&input, &piece, 7);
        assert!(verdict.is_err(), "a stub of {stub}: {verdict:?}");
        assert!(retraces(&piece, 0.05), "a stub of {stub} is found by itself, not through the length");
    }
}

/// Every edge and line piece lies on the input it is keyed by and takes exactly its own length
/// of it (a piece that ran out and back along it would take half), a piece of no length is where
/// two paths touch, and the pieces of one input are no longer in all than the input.
fn check_pieces_lie_on_their_inputs(shapes: &[Shape]) -> Check {
    let arr = shape_builder(shapes, true);
    let mut per_key: std::collections::BTreeMap<u64, (f64, usize)> = std::collections::BTreeMap::new();
    for e in arr.edges.iter().chain(&arr.lines).chain(&arr.lines_under) {
        let Some(s) = shapes.get(e.key as usize) else { continue };
        let len = length(&e.path);
        if len <= 0.05 {
            // A speck of a piece is where two paths touch, and lies on its input.
            let at = ends(&e.path).0;
            if distance_to(&s.path, at) >= 0.05 {
                return Err(format!("a piece keyed {} strays from its input at {at:?}", e.key));
            }
            let touching = shapes.iter().filter(|s| distance_to(&s.path, at) < 0.05).count();
            if touching < 2 {
                return Err(format!("a speck of a piece keyed {} at {at:?}, where no other path is", e.key));
            }
            continue;
        }
        check_piece_fits_input(&s.path, &e.path, e.key)?;
        let entry = per_key.entry(e.key).or_default();
        *entry = (entry.0 + len, entry.1 + 1);
    }
    for (key, (sum, count)) in per_key {
        let input = shapes.get(key as usize).map_or(0.0, |s| length(&s.path));
        if sum > input + 0.005 * count as f64 + 1e-6 {
            return Err(format!("the pieces of {key} are {sum} long in all, more than the input's {input}"));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, rng_seed: proptest::test_runner::RngSeed::Fixed(0x5eed_9a7f), ..ProptestConfig::default() })]

    #[test]
    fn faces_are_disjoint_and_know_what_covers_them(shapes in prop::collection::vec(arb_segment(), 1..7)) {
        check_faces_are_disjoint_and_know_what_covers_them(&shapes).map_err(TestCaseError::fail)?;
    }

    #[test]
    fn filled_faces_are_the_pathfinder_regions(shapes in prop::collection::vec(arb_segment(), 1..6)) {
        check_filled_faces_are_the_pathfinder_regions(&shapes).map_err(TestCaseError::fail)?;
    }

    #[test]
    fn pieces_lie_on_their_inputs(shapes in prop::collection::vec(arb_segment(), 1..6)) {
        check_pieces_lie_on_their_inputs(&shapes).map_err(TestCaseError::fail)?;
    }

    /// Lines anywhere (not on a grid): faces stay disjoint, and nothing panics.
    #[test]
    fn free_lines_make_disjoint_faces(pts in prop::collection::vec((-50.0..150.0f64, -50.0..150.0f64, -50.0..150.0f64, -50.0..150.0f64), 2..9)) {
        let lines: Vec<Shape> = pts.into_iter().map(|(a, b, x, y)| line((a, b), (x, y), 0)).collect();
        let (faces, edges) = live_paint(&lines);
        prop_assert!(edges.len() >= lines.len().min(1));
        for f in &faces {
            prop_assert!(f.sources.is_empty());
            if let Some(p) = crate::interior_point(&f.path) {
                prop_assert_eq!(faces.iter().filter(|g| g.contains(p)).count(), 1);
            }
        }
    }
}

/// The case proptest saved in `proptest-regressions` when it first failed (three even-odd circles,
/// the two small ones touching at (30, 85) inside the big one), run through every property here.
#[test]
fn the_saved_proptest_case_of_three_touching_circles() {
    let circle = |cx: f64, cy: f64, r: f64| Shape::new(ellipse(Rect::new(cx - r, cy - r, cx + r, cy + r)), FillRule::EvenOdd, 0);
    let shapes = [circle(25.0, 85.0, 5.0), circle(30.0, 80.0, 20.0), circle(35.0, 85.0, 5.0)];
    check_faces_are_disjoint_and_know_what_covers_them(&shapes).unwrap();
    check_filled_faces_are_the_pathfinder_regions(&shapes).unwrap();
    check_pieces_lie_on_their_inputs(&shapes).unwrap();
}

/// A comb: a bar and `n` teeth one wide with gaps nine wide between them, as one polygon.
fn comb(n: usize) -> BezPath {
    let mut b = BezPath::new();
    b.move_to((0.0, 0.0));
    for i in 0..n {
        let x = i as f64 * 10.0;
        b.line_to((x, 100.0));
        b.line_to((x + 1.0, 100.0));
        b.line_to((x + 1.0, 1.0));
        b.line_to((x + 10.0, 1.0));
    }
    b.line_to((n as f64 * 10.0, 0.0));
    b.close_path();
    b
}

/// A point inside a comb takes a handful of windings of the path, however many teeth it has (it
/// took one for each wider gap of the outside, each one a pass over the whole path).
#[test]
fn an_interior_point_of_a_comb_tests_a_handful_of_windings() {
    for n in [50, 500, 5000] {
        let b = comb(n);
        crate::work::reset();
        let p = crate::interior_point(&b).expect("a point");
        assert!(b.winding(p) != 0, "{p:?} is not inside");
        let tests = crate::work::take(crate::work::Work::Winding);
        assert!((1..=map::MAX_WINDING_TESTS).contains(&tests), "n={n}: {tests} windings");
    }
}

/// The faces of a comb are found with a few windings each too: one face, one point, not one
/// pass over the path for each gap.
#[test]
fn the_faces_of_a_comb_take_a_handful_of_windings() {
    let b = comb(2000);
    crate::work::reset();
    let (faces, _) = live_paint(&[Shape::new(b, FillRule::NonZero, 0)]);
    assert_eq!(faces.len(), 1);
    let tests = crate::work::take(crate::work::Work::Winding);
    assert!(tests <= map::MAX_WINDING_TESTS * faces.len(), "{tests} windings for {} faces", faces.len());
}

#[test]
fn many_lines_make_many_faces() {
    // 60 lines through a 100 unit square: about 900 crossings.
    let lines: Vec<Shape> = (0..60)
        .map(|k| {
            let t = f64::from(k) * 0.37;
            line((50.0 + 80.0 * t.cos(), 50.0 + 80.0 * t.sin()), (50.0 - 80.0 * (t + 1.1).cos(), 50.0 - 80.0 * (t + 1.1).sin()), 0)
        })
        .collect();
    crate::work::reset();
    let (faces, _) = live_paint(&lines);
    assert!(faces.len() > 500, "{}", faces.len());
    // Counted, not timed (VectorCraft 4cf4ea3): the reads of the index of the map's outlines.
    let reads = crate::work::take(crate::work::Work::IndexCandidates) + crate::work::take(crate::work::Work::IndexNodes);
    assert!(reads <= 256_000, "{reads} reads (226,485 measured)");
}

#[test]
fn holes_touching_at_a_point_stay_holes() {
    // Two small circles inside a big one, touching each other at (30,85): the big circle's face
    // has both as holes, and every face and region is one of the three.
    let circle = |cx: f64, cy: f64, r: f64| Shape::new(ellipse(Rect::new(cx - r, cy - r, cx + r, cy + r)), FillRule::EvenOdd, 0);
    let closed = [circle(25.0, 85.0, 5.0), circle(30.0, 80.0, 20.0), circle(35.0, 85.0, 5.0)];
    let (big, small) = (400.0 * std::f64::consts::PI, 25.0 * std::f64::consts::PI);
    let (faces, _) = live_paint(&closed);
    let regions = crate::regions(&closed);
    for found in [&faces, &regions] {
        let mut sizes: Vec<(Vec<usize>, f64)> = found.iter().map(|r| (r.sources.clone(), area(r))).collect();
        sizes.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(sizes.iter().map(|s| s.0.clone()).collect::<Vec<_>>(), [vec![0, 1], vec![1], vec![1, 2]], "{sizes:?}");
        for ((_, a), want) in sizes.iter().zip([small, big - 2.0 * small, small]) {
            assert!((a - want).abs() < 0.5, "{a} vs {want}: {sizes:?}");
        }
    }
}

/// An edge that starts with a segment far longer than the way the edges at its vertex leave (a
/// short chord at the vertex makes `r` small, and the long segment is the edge's first) is read
/// for the start of it that gets `r` away, not flattened whole: that was up to a billion lines.
#[test]
fn the_angle_an_edge_leaves_a_vertex_at_is_read_from_the_start_of_a_huge_segment() {
    use super::graph::{Edge, WHOLE_LINES_IN_ALL, leaving_angle};
    use crate::path::Seg;
    let edge = |c: kurbo::CubicBez| Edge { segs: vec![Seg { c, line: false }], ends: [0, 1], source: 0, sources: vec![0] };
    let (v, r) = (Point::new(0.0, 0.0), 1e-3);
    let mut whole = WHOLE_LINES_IN_ALL;
    // Leaves along +x at first, then turns up and away 1e8 out.
    let huge = edge(kurbo::CubicBez::new((0.0, 0.0), (1e8, 0.0), (1e8, 1e8), (0.0, 1e8)));
    work::reset();
    let angle = leaving_angle(&[huge], 0, v, r, &mut whole);
    let lines = work::take(Work::FlattenLines);
    assert!(angle.abs() < 1e-6, "{angle}");
    assert!(lines <= 2_000, "{lines} lines for the start of one segment");
    assert_eq!(whole, WHOLE_LINES_IN_ALL, "nothing was flattened whole");
    // The same, leaving along a diagonal and from the other end of the edge (the half-edge back).
    let diagonal = edge(kurbo::CubicBez::new((0.0, 1e8), (1e8, 1e8), (1e8, 0.0), (0.0, 0.0)));
    work::reset();
    let back = leaving_angle(&[diagonal], 1, v, r, &mut whole);
    // The edge reversed is the same cubic, and leaves along +x like the first (not up or down, as a
    // half-edge put in the wrong place would).
    assert!(back.abs() < 1e-6, "{back}");
    assert!(work::take(Work::FlattenLines) <= 2_000);
    // A segment that is small beside r is flattened whole, as before, and gives its tangent.
    let small = edge(kurbo::CubicBez::new((0.0, 0.0), (4e-3, 4e-3), (8e-3, 4e-3), (1.2e-2, 0.0)));
    let tangent = leaving_angle(&[small], 0, v, r, &mut whole);
    assert!((tangent - std::f64::consts::FRAC_PI_4).abs() < 0.2, "{tangent}");
}

/// A segment a thousand times `r` is flattened whole (the same lines as ever, which the order of
/// edges along one tangent depends on) until the map has spent its allowance, and cut after.
#[test]
fn segments_are_flattened_whole_until_the_map_has_spent_its_lines() {
    use super::graph::{Edge, leaving_angle};
    use crate::path::Seg;
    let edge = |c: kurbo::CubicBez| Edge { segs: vec![Seg { c, line: false }], ends: [0, 1], source: 0, sources: vec![0] };
    let (v, r) = (Point::new(0.0, 0.0), 1e-3);
    let medium = edge(kurbo::CubicBez::new((0.0, 0.0), (1e3, 0.0), (1e3, 1e3), (0.0, 1e3)));
    let mut whole = 3e5;
    work::reset();
    let first = leaving_angle(std::slice::from_ref(&medium), 0, v, r, &mut whole);
    let whole_lines = work::take(Work::FlattenLines);
    assert!(whole_lines > 20_000 && whole < 3e5, "{whole_lines} lines whole, {whole} left");
    // Spent: the same edge is cut.
    let mut spent = 0.0;
    work::reset();
    let after = leaving_angle(std::slice::from_ref(&medium), 0, v, r, &mut spent);
    let cut_lines = work::take(Work::FlattenLines);
    assert!(cut_lines <= 2_000, "{cut_lines} lines when the allowance is spent, against {whole_lines} whole");
    // Whole or cut, the edge leaves the same way to within the tolerance of the flattening.
    assert!((first - after).abs() < 2e-4, "{first} whole, {after} cut");
}

/// The start of a segment kept for the angle is the segment itself when its control polygon is
/// within 4 `r` of the vertex or flattening it whole is cheap, and else a part that reaches `r`
/// and whose control polygon is within a few dozen `r`, however long the segment.
#[test]
fn the_start_of_a_segment_within_reach_of_a_vertex_reaches_r_and_stays_close() {
    use super::graph::start_within_reach;
    let (v, r) = (Point::new(0.0, 0.0), 1e-3);
    let small = kurbo::CubicBez::new((0.0, 0.0), (1e-3, 1e-3), (2e-3, 1e-3), (3e-3, 0.0));
    let mut none = 0.0;
    assert_eq!(start_within_reach(&small, v, r, &mut none), (small, 1.0));
    let cut = |c: &kurbo::CubicBez, whole: &mut f64| {
        let (part, end) = start_within_reach(c, v, r, whole);
        let reach = [part.p0, part.p1, part.p2, part.p3].iter().map(|p| p.distance(v)).fold(0.0, f64::max);
        assert!(
            part.p3.distance(v) >= r && reach <= 40.0 * r && end > 0.0 && end < 1.0,
            "reaches {}, control polygon {reach}, ends at {end}",
            part.p3.distance(v)
        );
    };
    for scale in [1e-2, 1.0, 1e4] {
        // Cut when there is no allowance left...
        cut(&kurbo::CubicBez::new((0.0, 0.0), (scale, 0.0), (scale, scale), (0.0, scale)), &mut none);
    }
    for scale in [1e8, 1e12] {
        // ... and when the segment is past what is flattened whole whatever the allowance.
        let mut plenty = f64::INFINITY;
        cut(&kurbo::CubicBez::new((0.0, 0.0), (scale, 0.0), (scale, scale), (0.0, scale)), &mut plenty);
    }
    // With an allowance, a segment a thousand times r is whole and is charged.
    let mut allowance = 1e9;
    let medium = kurbo::CubicBez::new((0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0));
    assert_eq!(start_within_reach(&medium, v, r, &mut allowance), (medium, 1.0));
    assert!(allowance < 1e9);
}

/// The inputs that cover a face are found among the fills whose bounds hold its interior point, not
/// by testing the point against every fill: faces apart, each in a fill of its own, cost a few
/// tests each, not one for every fill.
#[test]
fn the_fills_a_face_is_tested_against_are_those_near_it() {
    let tests = |n: u32| {
        let shapes: Vec<Shape> = (0..n)
            .map(|i| Shape::new(rect_path(0.0, f64::from(i) * 20.0, 10.0, f64::from(i) * 20.0 + 10.0), FillRule::NonZero, u64::from(i)))
            .collect();
        work::reset();
        let (faces, _) = live_paint(&shapes);
        assert_eq!(faces.len(), n as usize);
        assert!(faces.iter().enumerate().all(|(i, f)| f.sources == [i]), "sources in order");
        work::take(Work::FillTests)
    };
    let (small, large) = (tests(100), tests(400));
    assert!(small > 0 && small <= 8 * 100, "{small} tests for 100 faces");
    assert!(large <= 8 * 400, "{large} tests for 400 faces, against {} for a scan", 400 * 400);
}

/// A fill of many subpaths (squares in a column, one shape) is tested for a point against the subpaths
/// whose bounds hold it: the cost of a face is the few near it, so the cost of all the faces grows
/// with their number, where it was the square of it (4 s for 8,000 squares, 15 s for 16,000).
#[test]
fn a_fill_of_many_subpaths_is_tested_against_the_few_that_hold_the_point() {
    let column = |n: usize| {
        let mut col = BezPath::new();
        for j in 0..n {
            col.extend(rect_path(5_000.0, 3.0 * j as f64, 5_001.0, 3.0 * j as f64 + 1.0).iter());
        }
        Shape::new(col, FillRule::NonZero, 0)
    };
    let mut counts = Vec::new();
    for n in [4_000usize, 16_000] {
        let shape = column(n);
        let fill = super::graph::Fill::new(&shape);
        let budget = super::graph::FillWork::new();
        work::reset();
        // A point in the middle of every square is in its fill, and one between two is not.
        for j in 0..n {
            assert_eq!(fill.contains(Point::new(5_000.5, 3.0 * j as f64 + 0.5), &budget), Some(true), "square {j}");
            assert_eq!(fill.contains(Point::new(5_000.5, 3.0 * j as f64 + 2.0), &budget), Some(false), "gap {j}");
        }
        let subpaths = work::take(Work::FillSubpaths);
        assert!(subpaths <= 2 * n, "{subpaths} subpaths wound for {n} squares and as many gaps, against {} for a scan", 2 * n * n);
        counts.push(subpaths);
    }
    assert!(counts[1] <= 5 * counts[0], "4,000 squares {}, 16,000 squares {}: linear, four times as many", counts[0], counts[1]);
    // And through the map: 2,000 squares of one shape are 2,000 faces, each in the shape.
    work::reset();
    let (faces, _) = live_paint(&[column(2_000)]);
    assert_eq!(faces.len(), 2_000);
    assert!(faces.iter().all(|f| f.sources == [0]));
    assert!(work::take(Work::FillSubpaths) <= 2 * 2_000, "a face winds the few subpaths near it");
}

/// A path of parts far apart, each a unit high, is missed by the first heights tried; the search
/// goes on over a bounded number of the stretches between the heights of its vertices, and so finds
/// a part, whatever the number of parts.
#[test]
fn an_interior_point_is_found_among_parts_far_apart_in_a_bounded_search() {
    let squares = |n: u32| {
        let mut p = BezPath::new();
        for k in 0..n {
            let y = f64::from(k) * 100.0;
            p.extend(polyline(&[(0.0, y), (1.0, y), (1.0, y + 1.0), (0.0, y + 1.0)], true).iter());
        }
        p
    };
    for n in [2, 50, 1_000] {
        let p = squares(n);
        work::reset();
        let found = crate::interior_point(&p);
        let heights = work::take(Work::InteriorHeights);
        let at = found.unwrap_or_else(|| panic!("no point in {n} squares"));
        assert!(kurbo::Shape::winding(&p, at) != 0, "{at:?} is outside the {n} squares");
        assert!((9..=9 + 64).contains(&heights), "{heights} heights for {n} squares");
    }
    // One part, found at the first heights: the search goes no further.
    work::reset();
    assert!(crate::interior_point(&squares(1)).is_some());
    assert_eq!(work::take(Work::InteriorHeights), 9, "the first heights are all it looks at");
    // And a path with nothing in it (a line with many vertices) is None, in a bounded search too.
    work::reset();
    let diagonal: Vec<(f64, f64)> = (0..200).map(|i| (f64::from(i), f64::from(i))).collect();
    assert_eq!(crate::interior_point(&polyline(&diagonal, false)), None);
    assert!(work::take(Work::InteriorHeights) <= 9 + 64);
}

/// The sweep's frame is sheared by 1/32 unless an open path slopes -1/32 in the document (so would
/// be level in it); then by the next power of two that leaves none level.
#[test]
fn the_shear_is_changed_only_for_an_open_path_that_would_be_level_in_it() {
    use super::index::{SHEARS, frame_for};
    let shear_of = |shapes: &[Shape]| frame_for(shapes).map(|f| f.k());
    let shape = |p: BezPath| Shape::new(p, FillRule::NonZero, 0);
    let ordinary = [shape(line_path((0.0, 0.0), (100.0, 0.0))), shape(line_path((0.0, 0.0), (10.0, 10.0)))];
    assert_eq!(shear_of(&ordinary), Some(1.0 / 32.0));
    // A closed shape with such an edge is no trouble: only open paths are.
    let closed = [shape(polyline(&[(0.0, 0.0), (64.0, -2.0), (64.0, 10.0)], true))];
    assert_eq!(shear_of(&closed), Some(1.0 / 32.0));
    let level = line_path((-10.0, 50.3125), (110.0, 46.5625));
    assert_eq!(shear_of(&[shape(level.clone())]), Some(1.0 / 64.0));
    // Also a curve with a side of its control polygon that slopes so.
    let mut curve = BezPath::new();
    curve.move_to((0.0, 0.0));
    curve.curve_to((32.0, -1.0), (50.0, 20.0), (60.0, 10.0));
    assert_eq!(shear_of(&[shape(curve)]), Some(1.0 / 64.0));
    // And one that is level in the next too moves on to the one after.
    let second = line_path((0.0, 0.0), (64.0, -1.0));
    assert_eq!(shear_of(&[shape(level), shape(second)]), Some(1.0 / 16.0));
    assert_eq!(SHEARS[2], 1.0 / 16.0);
}

/// The test of whether an open path is level is made on the paths as the sweep gets them, snapped to
/// its tolerance, which a distant shape widens: a line that rises 5e-4 in the usual frame is level
/// there once a shape at 1e11 makes the tolerance 1.5e-3, so the frame moves on.
#[test]
fn an_open_path_that_snapping_levels_moves_the_shear_on() {
    use super::index::frame_for;
    let line = line_path((0.0, 0.0), (100.0, -3.1245));
    let shape = |p: BezPath| Shape::new(p, FillRule::NonZero, 0);
    let alone = frame_for(&[shape(line.clone())]).expect("a frame");
    assert_eq!(alone.k(), 1.0 / 32.0, "by itself it is not level (it rises 5e-4 in 100)");
    let far = rect_path(1e11, 1e11, 1e11 + 1.0, 1e11 + 1.0);
    let both = frame_for(&[shape(line), shape(far)]).expect("a frame");
    assert!(both.eps > 1e-3, "{}", both.eps);
    assert_eq!(both.k(), 1.0 / 64.0, "the next shear, in which the line rises 1.6");
}

/// A side of an open path no longer than two tolerances of the sweep is level in every shear (the
/// steepest raises its far end by less than one), and is not looked at: a drawing with one is framed at
/// the first try. One of a few tolerances (up to eight) is short: it does not make a frame fail, but a
/// frame in which it is not level is looked for, and used where there is one. A side longer is long.
#[test]
fn a_side_within_a_few_tolerances_does_not_fail_a_frame_and_is_kept_where_a_frame_can() {
    use super::index::{SHEARS, frame_for};
    let shape = |p: BezPath, key: u64| Shape::new(p, FillRule::NonZero, key);
    let square = rect_path(0.0, 0.0, 100.0, 100.0);
    let frame_with = |stub: BezPath| {
        work::reset();
        let frame = frame_for(&[shape(square.clone(), 0), shape(line_path((-10.0, 50.0), (110.0, 50.0)), 1), shape(stub, 2)]);
        (frame.map(|f| f.k()), work::take(Work::FrameTries))
    };
    // The sweep's tolerance here is 1e-6. Stubs along the height of the frame and across it.
    for len in [1e-9, 1e-7, 1.9e-6] {
        for stub in [line_path((300.0, 300.0), (300.0 + len, 300.0)), line_path((300.0, 300.0), (300.0 + len, 300.0 + len))] {
            assert_eq!(frame_with(stub), (Some(1.0 / 32.0), 1), "a stub {len} long: framed at the first try");
        }
    }
    // One 7e-6 long along the height of the frame rises 7e-6 k, under the tolerance in the first seven
    // shears and over it in the eighth, 1/4: that is the frame. Across it, it is level in none.
    assert_eq!(frame_with(line_path((300.0, 300.0), (300.0 + 7e-6, 300.0))), (Some(1.0 / 4.0), 8));
    assert_eq!(frame_with(line_path((300.0, 300.0), (300.0 + 7e-6, 300.0 + 7e-6))), (Some(1.0 / 32.0), 1));
    // One 3.6e-6 long sloping -1/4 is within a tolerance of level in every shear: each is tried, and
    // the first used (the stub may be lost from the map, the rest is kept).
    assert_eq!(frame_with(line_path((300.0, 300.0), (300.0 + 3.5e-6, 300.0 - 3.5e-6 / 4.0))), (Some(1.0 / 32.0), SHEARS.len()));
    // A stub of 1e-4 along the height of the frame is long: level in the usual shear, and the next works.
    work::reset();
    let stub = line_path((300.0, 300.0), (300.0 + 1e-4, 300.0 - 1e-4 / 32.0));
    let frame = frame_for(&[shape(square, 0), shape(stub, 2)]).expect("a frame");
    assert!(frame.k() != 1.0 / 32.0 && work::take(Work::FrameTries) > 1, "k = {}", frame.k());
}

/// A line of segments each shorter than 16 tolerances of the sweep is lost from the map in any frame,
/// level in it or not: the map takes pieces whose ends lie within that of each other for copies of one
/// edge (the sweep cuts the two passes of an open path, or coincident outlines, a little apart) and
/// joins their ends. A line of segments a little longer is kept. This is what no choice of frame changes.
#[test]
fn a_line_of_segments_under_sixteen_tolerances_is_lost_in_any_frame() {
    // The sweep's tolerance is 1e-6 here; the segments are 1.4e-5 and 2.8e-5 long, on a diagonal level in no frame.
    for (step, whole) in [(1e-5, false), (2e-5, true)] {
        let pts: Vec<(f64, f64)> = (0..=1_000).map(|i| (f64::from(i) * step, f64::from(i) * step)).collect();
        let line = polyline(&pts, false);
        let (_, edges) = live_paint(&[rect(0.0, 0.0, 1.0, 1.0, 0), Shape::new(line.clone(), FillRule::NonZero, 2)]);
        let kept: f64 = edges.iter().filter(|e| e.key == 2).map(|e| length(&e.path)).sum();
        if whole {
            assert!((kept - length(&line)).abs() < 1e-9, "{kept} of {}", length(&line));
        } else {
            assert!(kept.abs() < 1e-12, "{kept} of {}", length(&line));
        }
    }
}

/// Ten lines, each level in one of the ten shears, are framed by an eleventh shear between their slopes,
/// in which none is level, and kept whole; lines whose slopes bar every shear from 1/1024 to 1/2 by
/// one from -1/2 to -1/1024. Lines whose slopes leave no shear free in either give no frame and no
/// map, rather than a map with one of them lost.
#[test]
fn open_paths_level_in_each_of_the_ten_shears_are_framed_by_one_between_their_slopes() {
    use super::index::{SHEARS, frame_for};
    let lines: Vec<Shape> = SHEARS
        .iter()
        .enumerate()
        .map(|(i, k)| Shape::new(line_path((0.0, 100.0 * i as f64), (1024.0, 100.0 * i as f64 - k * 1024.0)), FillRule::NonZero, i as u64))
        .collect();
    work::reset();
    let k = frame_for(&lines).map(|f| f.k()).expect("a frame between their slopes");
    assert_eq!(work::take(Work::FrameTries), SHEARS.len() + 1, "each shear is tried once, and one more");
    assert!(!SHEARS.contains(&k) && k > 1.0 / 1024.0 && k < 0.5, "k = {k}");
    let (_, edges) = live_paint(&lines);
    for (i, s) in lines.iter().enumerate() {
        let kept: f64 = edges.iter().filter(|e| e.key == i as u64).map(|e| length(&e.path)).sum();
        assert!((kept - length(&s.path)).abs() < 1e-6, "line {i}: {kept} of {}", length(&s.path));
    }
    // Nine of them leave the tenth.
    work::reset();
    assert_eq!(frame_for(&lines[..9]).map(|f| f.k()), Some(SHEARS[9]));
    assert_eq!(work::take(Work::FrameTries), SHEARS.len());
    // Lines 9e-6 long (nine tolerances), one level in each shear: each is level in the shears within
    // about 0.12 of its own, which leave the shears about 3/8 free.
    let short: Vec<Shape> = SHEARS
        .iter()
        .enumerate()
        .map(|(i, k)| Shape::new(line_path((0.0, 100.0 * i as f64), (9e-6, 100.0 * i as f64 - k * 9e-6)), FillRule::NonZero, i as u64))
        .collect();
    work::reset();
    assert_eq!(frame_for(&short).map(|f| f.k()), Some(3.0 / 8.0));
    assert_eq!(work::take(Work::FrameTries), SHEARS.len() + 1);
    // With an open cubic whose first control side, a thousandth across, is level in the shear of 0.37:
    // it bars the shears within three tolerances over its width of that (a side to a handle), 0.3668
    // to 0.3732, which leaves the stretch above it to 0.382; within one it would leave a wider one
    // below, and a shear in that.
    let mut curved = short.clone();
    let mut cubic = BezPath::new();
    cubic.move_to((0.0, 2000.0));
    cubic.curve_to((0.001, 1999.99963), (1.0, 2020.0), (2.0, 2040.0));
    curved.push(Shape::new(cubic, FillRule::NonZero, 10));
    // The middle of 0.3732 to 0.3820, to a multiple of 2^-14: 6186 of them.
    assert_eq!(frame_for(&curved).map(|f| f.k()), Some(6186.0 / 16384.0));
    // Lines that long sloping 0, -1/10, ..., -1/2: each of the ten shears levels one, and between
    // them they level every shear from 1/1024 to 1/2; a shear from -1/2 to -1/1024 frames them.
    let level: Vec<Shape> = (0u8..6)
        .map(|i| {
            Shape::new(line_path((0.0, 100.0 * f64::from(i)), (9e-6, 100.0 * f64::from(i) - f64::from(i) * 9e-7)), FillRule::NonZero, u64::from(i))
        })
        .collect();
    work::reset();
    let k = frame_for(&level).map(|f| f.k()).expect("a frame among the shears below 0");
    assert!(k < -1.0 / 1024.0 && k > -0.5, "k = {k}");
    assert_eq!(work::take(Work::FrameTries), SHEARS.len() + 1);
    // With their mirror images, sloping up as much, they bar the shears below 0 too: none is free.
    let mirrored: Vec<Shape> = (0u8..6)
        .flat_map(|i| {
            let (y, rise) = (100.0 * f64::from(i), f64::from(i) * 9e-7);
            [line_path((0.0, y), (9e-6, y - rise)), line_path((50.0, y), (50.0 + 9e-6, y + rise))]
        })
        .enumerate()
        .map(|(i, p)| Shape::new(p, FillRule::NonZero, i as u64))
        .collect();
    work::reset();
    assert!(frame_for(&mirrored).is_none(), "no shear is free");
    assert_eq!(work::take(Work::FrameTries), SHEARS.len(), "and none more is tried");
    assert_eq!(live_paint(&mirrored), (Vec::new(), Vec::new()));
    // Ordinary art is framed at the first try.
    work::reset();
    assert!(frame_for(&[Shape::new(line_path((0.0, 0.0), (10.0, 10.0)), FillRule::NonZero, 0)]).is_some());
    assert_eq!(work::take(Work::FrameTries), 1);
}

/// Two shapes under curves that rise from (0, 0) and come back to (159.7, 0) a few hundredths apart: the lens between
/// the curves, up to 0.07 wide and 0.028 thick (over the precision of 0.01), and under the upper shape alone.
fn lens_of_two_arches() -> [Shape; 2] {
    let end = 159.71479927270926;
    let under = |c1: (f64, f64), c2: (f64, f64)| {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to(c1, c2, (end, 0.0));
        p.line_to((end, -50.0));
        p.line_to((0.0, -50.0));
        p.close_path();
        p
    };
    let (c1, c2) = ((34.39808760979988, 8.807766709864676), (111.74858077097721, 15.954246590150165));
    let lower = under(c1, c2);
    let upper = under((c1.0, c1.1 + 0.03758453957627678), (c2.0, c2.1 + 0.06841421252966735));
    [Shape::new(lower, FillRule::NonZero, 0), Shape::new(upper, FillRule::NonZero, 1)]
}

/// The point deep in a face is looked for on its outline flattened to a thousandth of its size, and a face
/// that is thinner than that has none of the middles of its spans in it: a lens 0.03 wide between two curves a
/// hundred and sixty long. It was then listed with no source, a gap that nothing fills (and once more, as the filled
/// area it is, by `shape_builder`), though it is covered by the upper shape; the point is looked for again
/// on an outline flattened finer, whatever the precision (it was so for a precision under the default only).
#[test]
fn a_face_thinner_than_the_first_flattening_has_its_sources_from_a_finer_one() {
    let shapes = lens_of_two_arches();
    let (faces, _) = live_paint(&shapes);
    let lens = faces.iter().filter(|f| f.path.area().abs() < 10.0).collect::<Vec<_>>();
    assert_eq!(
        lens.len(),
        1,
        "the lens between the curves is a face: {:?}",
        faces.iter().map(|f| (f.path.area().abs(), &f.sources)).collect::<Vec<_>>()
    );
    assert!(interior_point(&lens[0].path).is_none(), "the first search finds no point in it");
    assert_eq!(lens[0].sources, [1], "it is under the upper curve alone");
    assert!(faces.iter().all(|f| !f.sources.is_empty()), "no face is a gap");
    // And `shape_builder` lists the regions of the filled areas, with no gap beside them.
    let built = shape_builder(&shapes, false);
    assert_eq!(
        built.regions.len(),
        2,
        "the area under both, and the lens: {:?}",
        built.regions.iter().map(|r| (r.path.area().abs(), &r.sources)).collect::<Vec<_>>()
    );
    assert!(built.regions.iter().all(|r| !r.sources.is_empty()));
}

/// Where no point is found in a face thicker than the precision, what covers it is not known, and it would be listed
/// as a gap that nothing fills: `try_shape_builder` and `try_pathfinder` say so ([`crate::PathOpsError::Degenerate`]) where
/// the others list it with no source. A face thinner than the precision is debris and no error. (A test hook
/// makes the search find nothing: no face the flattening finer than the first leaves is without a point.)
#[test]
fn a_face_with_no_point_that_is_thicker_than_the_precision_is_an_error_to_the_strict_forms() {
    let shapes = lens_of_two_arches();
    super::map::NO_POINTS_BELOW.with(|f| f.set(10.0));
    let strict = try_shape_builder(&shapes, false);
    let (faces, _) = live_paint(&shapes);
    let precision_over = crate::pathfinder::try_pathfinder(
        crate::PathfinderOp::Divide,
        &[shapes[0].clone(), shapes[1].clone(), Shape::new(line_path((-10.0, 5.0), (110.0, 5.0)), FillRule::NonZero, 2)],
    );
    // A precision above the lens's thickness (0.028) leaves it out of the map, as debris: no error.
    let loose = crate::planar::try_builder(&shapes, false, 1.0, true);
    super::map::NO_POINTS_BELOW.with(|f| f.set(0.0));
    assert_eq!(strict.map(|_| ()), Err(crate::PathOpsError::Degenerate));
    assert_eq!(precision_over.map(|_| ()), Err(crate::PathOpsError::Degenerate), "with an open path, the planar map is the Pathfinder's too");
    assert!(faces.iter().any(|f| f.sources.is_empty()), "the infallible forms list it with no source, as they did");
    assert!(loose.is_ok(), "a sliver is no error: {:?}", loose.map(|_| ()));
    // With the hook off the same shapes are mapped.
    assert!(try_shape_builder(&shapes, false).is_ok());
}

/// Where the art is near the largest coordinates the sweep takes, the shears that widen them past it are no
/// frame, and the search for a free one is made among those that do not: the middle of the widest stretch
/// the long sides leave free could be in the ones that do (a frame no good), where a narrower stretch of
/// the others is. A shape at (4e13, 6e13) takes in shears to about 0.26; nine lines 40 long, each level in
/// one of the shears 1/1024 to 1/4, leave free the stretches 0.0864 to 0.101, 0.149 to 0.226 and 0.274 on;
/// twelve with slopes up bar the shears below 0. The frame is in the middle of 0.149 to 0.226, not of
/// 0.274 to 0.5 where the sheared art is too far out.
#[test]
fn a_free_shear_is_looked_for_where_the_coordinates_are_valid() {
    use super::index::{SHEARS, frame_for};
    let line =
        |i: usize, slope: f64| Shape::new(line_path((0.0, 100.0 * i as f64), (40.0, 100.0 * i as f64 + 40.0 * slope)), FillRule::NonZero, i as u64);
    let far = Shape::new(polyline(&[(3.9e13, 5.9e13), (4.0e13, 5.9e13), (4.0e13, 6.0e13), (3.9e13, 6.0e13)], true), FillRule::NonZero, 99);
    let mut shapes = vec![far];
    // Level in each shear of the ten but the last (1/2 widens the art past what the sweep takes).
    shapes.extend(SHEARS[..9].iter().enumerate().map(|(i, k)| line(i, -k)));
    // The shears below 0, each barred by a line sloping up as much.
    shapes.extend((0..12).map(|i| line(20 + i, 0.02 + 0.045 * i as f64)));
    work::reset();
    let k = frame_for(&shapes).map(|f| f.k()).expect("a frame where the coordinates are valid");
    assert!(k > 0.149 && k < 0.226, "k = {k}");
    assert_eq!(work::take(Work::FrameTries), SHEARS.len() + 1, "the ten, and one more");
    // The art in that frame is within what the sweep takes: no coordinate is past where its tolerance is a unit.
    let extent = 6.0e13 + k * 4.0e13;
    assert!(extent <= crate::boolean::largest_extent(), "{extent}");
    // With the stretches in the shears that the coordinates allow barred as well (lines level in 0.0925,
    // 0.17, 0.2 and 0.215) there is no frame, as before: a shear past the coordinates is no frame.
    let mut barred = shapes.clone();
    barred.extend([0.0925, 0.17, 0.2, 0.215].iter().enumerate().map(|(i, k)| line(40 + i, -k)));
    assert!(frame_for(&barred).is_none(), "no shear is free where the coordinates are valid");
}

/// The search for a shear among those of one sign takes the tolerance of that range from the coordinates at its ends,
/// not the widest the other range's frames had: a shape at 1e11 widens the sweep's tolerance in a shear of +1/2
/// (1.5e11 in the frame) a half more than in one of -1/2 (1e11, the shape's x), and the wider tolerance bars a
/// wider stretch of shears about each long side. Eleven lines 0.03 long (21 tolerances), level in the ten shears
/// and in 3/8, bar every shear from 1/1024 to 1/2; five more, sloping up, each bar a stretch of the shears
/// below 0 that is 0.1 wide at the tolerance of the shears below 0 and 0.15 at the widest of the others, and
/// are spaced 0.105 apart: four gaps of 0.004 open between them at the first and none at the second.
#[test]
fn a_range_of_shears_is_searched_at_its_own_tolerance() {
    use super::index::{SHEARS, frame_for};
    let far = Shape::new(polyline(&[(1e11, 1e11), (1e11 + 1.0, 1e11), (1e11 + 1.0, 1e11 + 1.0), (1e11, 1e11 + 1.0)], true), FillRule::NonZero, 99);
    let dx = 0.03;
    let line =
        |i: usize, slope: f64| Shape::new(line_path((0.0, 100.0 * i as f64), (dx, 100.0 * i as f64 + dx * slope)), FillRule::NonZero, i as u64);
    let mut shapes = vec![far];
    // Level in each of the ten shears, and in 3/8 (which the ten leave free).
    shapes.extend(SHEARS.iter().enumerate().map(|(i, k)| line(i, -k)));
    shapes.push(line(10, -0.375));
    // Five that slope up by 0.05, 0.155, ...: each bars the shears within about 0.05 of its negative.
    shapes.extend((0..5).map(|i| line(20 + i, 0.05 + 0.104_67 * i as f64)));
    work::reset();
    let frame = frame_for(&shapes).expect("a frame in the shears below 0, where the tolerance is narrower");
    assert!(frame.k() < -0.1 && frame.k() > -0.5, "k = {}", frame.k());
    assert_eq!(work::take(Work::FrameTries), SHEARS.len() + 1, "the ten, and one more");
    // The map is made, and the lines are in it.
    let (faces, edges) = live_paint(&shapes);
    assert_eq!(faces.len(), 1);
    for i in (0..=10).chain(20..25) {
        let kept: f64 = edges.iter().filter(|e| e.key == i as u64).map(|e| length(&e.path)).sum();
        assert!(kept > 0.9 * dx, "line {i}: {kept} of {dx}");
    }
    // The same lines with the gaps closed (six, spaced 0.095 apart, under the 0.1 each bars) leave no shear.
    let mut closed: Vec<Shape> = shapes[..12].to_vec();
    closed.extend((0..6).map(|i| line(20 + i, 0.05 + 0.095 * i as f64)));
    assert!(frame_for(&closed).is_none(), "no shear is free with the gaps closed");
}

/// Edges that run back over each other cancel in the winding, as many of each way as the difference
/// of their numbers leaves, and the rest keep their order. Exactly: ends a hair apart are not the same edge.
#[test]
fn edges_that_run_back_over_each_other_cancel_and_the_rest_keep_their_order() {
    use super::map::without_retraces;
    let [a, b, c, d, e] = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (5.0, 5.0), (6.0, 5.0)].map(|(x, y)| Point::new(x, y));
    // Out and back through the same points leaves nothing.
    assert_eq!(without_retraces(vec![(a, b), (b, c), (c, b), (b, a)]), vec![]);
    // Two one way and one the other leaves one; the others keep their order.
    assert_eq!(without_retraces(vec![(d, e), (a, b), (e, d), (b, c), (a, b), (b, a)]), vec![(a, b), (b, c)]);
    assert_eq!(without_retraces(vec![(b, a), (b, a), (a, b), (c, d)]), vec![(b, a), (c, d)]);
    // An edge of no length bounds nothing.
    assert_eq!(without_retraces(vec![(a, a), (a, b)]), vec![(a, b)]);
    // And an edge back to a point a hair off is another edge.
    let near = Point::new(1.0 + 1e-12, 0.0);
    assert_eq!(without_retraces(vec![(a, b), (near, a)]).len(), 2);
}

/// A retraced stub costs the heights it would have been given: a square beside a zigzag that goes out
/// and back through the same points, turning at a vertex at every integer height, is found at the first
/// heights tried once the retrace is dropped. A stub out and back along a straight line, through the
/// same vertices or others, puts no heights in the search but its two ends'.
#[test]
fn a_retraced_path_does_not_use_up_the_heights_of_the_search() {
    let square = polyline(&[(0.0, 400.3), (1.0, 400.3), (1.0, 401.3), (0.0, 401.3)], true);
    let zigzag: Vec<(f64, f64)> = (0..=1000).map(|y| (5.0 + 0.5 * f64::from(y % 2), f64::from(y))).collect();
    let back: Vec<(f64, f64)> = zigzag.iter().rev().skip(1).copied().collect();
    let mut p = square.clone();
    p.extend(polyline(&[zigzag, back].concat(), false).iter());
    work::reset();
    let at = crate::interior_point(&p).expect("a point in the square");
    assert!(kurbo::Shape::winding(&p, at) != 0 && at.x < 1.0);
    // The heights of the whole path (nine, and the sixty-four more) find nothing in 2,001 stretches of a
    // thousand heights; those of what does not cancel, in its own bounds, find the square at the first.
    assert_eq!(work::take(Work::InteriorHeights), 9 + 64 + 9, "the first search, then the second");
    // Without the retrace it is the same square.
    assert_eq!(crate::interior_point(&p), crate::interior_point(&square));
    // Straight out and back through the same vertices: the stretches are those of its two ends and the
    // square's, three of them, and the one in the middle finds the square.
    let out: Vec<(f64, f64)> = (0..=1000).map(|y| (5.0, f64::from(y))).collect();
    let back: Vec<(f64, f64)> = (0..1000).rev().map(|y| (5.0, f64::from(y))).collect();
    let mut q = square.clone();
    q.extend(polyline(&[out, back].concat(), false).iter());
    work::reset();
    let at = crate::interior_point(&q).expect("a point in the square");
    assert!(kurbo::Shape::winding(&q, at) != 0 && at.x < 1.0);
    assert_eq!(work::take(Work::InteriorHeights), 9 + 3);
    // A path that is nothing but a retrace has no inside.
    let stub = polyline(&[(0.0, 0.0), (0.0, 5.0), (0.0, 10.0), (0.0, 5.0), (0.0, 0.0)], false);
    assert_eq!(crate::interior_point(&stub), None);
    // Nor one along a slanted line, whose points are rounded each way: its slivers are rounding.
    let slant = polyline(&[(0.0, 0.0), (0.3, 1.0), (0.9, 3.0), (0.6, 2.0), (0.0, 0.0)], false);
    assert_eq!(crate::interior_point(&slant), None);
}

/// A retrace whose way back has zeros of the other sign (`-0.0`, as a mirror or a negated coordinate
/// gives) is the same edge the other way: the bits of a zero of either sign are one key.
#[test]
fn a_retrace_through_zeros_of_the_other_sign_cancels() {
    use super::map::without_retraces;
    let (a, b) = (Point::new(0.0, 0.0), Point::new(0.0, 1.0));
    let (a_neg, b_neg) = (Point::new(-0.0, -0.0), Point::new(-0.0, 1.0));
    assert_eq!(without_retraces(vec![(a, b), (b_neg, a_neg)]), vec![]);
    assert_eq!(without_retraces(vec![(a, b), (b_neg, a_neg), (a_neg, b)]), vec![(a_neg, b)]);
    // And through `interior_point`: a square beside a zigzag along x = 0 that comes back through -0.0.
    let square = polyline(&[(10.0, 400.3), (11.0, 400.3), (11.0, 401.3), (10.0, 401.3)], true);
    let zigzag: Vec<(f64, f64)> = (0..=1000).map(|y| (0.5 * f64::from(y % 2), f64::from(y))).collect();
    let back: Vec<(f64, f64)> = zigzag.iter().rev().skip(1).map(|&(x, y)| (if x == 0.0 { -0.0 } else { x }, y)).collect();
    let mut p = square;
    p.extend(polyline(&[zigzag, back].concat(), false).iter());
    let at = crate::interior_point(&p).expect("a point in the square");
    assert!(kurbo::Shape::winding(&p, at) != 0 && at.x > 10.0, "{at:?}");
}

fn line_path(a: (f64, f64), b: (f64, f64)) -> BezPath {
    polyline(&[a, b], false)
}

/// `n` rectangles nested in one shape, each 1,000 times as tall as it is wide, as one shape.
fn tall_nested(n: usize) -> Shape {
    let mut one = BezPath::new();
    for j in 0..n {
        let (j, m) = (j as f64, (2 * n) as f64);
        one.extend(rect_path(10.0 * j, 10_000.0 * j, 10.0 * (m - j), 10_000.0 * (m - j)).iter());
    }
    Shape::new(one, FillRule::NonZero, 0)
}

/// The winding of points against the fills and faces of a planar map is charged before each test,
/// and a map that would charge more than the limit is not built. Rectangles nested n deep in one shape
/// cost each face the subpaths round it, and each hole the faces round it: about 6 n² elements.
#[test]
fn the_fill_work_of_a_map_is_charged_before_each_test_and_held_to_its_limit() {
    // The faces, the elements charged, the subpaths of the fill wound against, and the elements wound
    // against: of fills and of faces (for holes), counted at the windings themselves, not at the charges.
    let measured = |n: usize, limit: usize| {
        let l = crate::Limits { fill_work: limit, ..crate::Limits::default() };
        work::reset();
        let (faces, _) = crate::with_limits(l, || live_paint(&[tall_nested(n)]));
        let charged = (faces.len(), work::take(Work::FillElements), work::take(Work::FillSubpaths));
        (charged, (work::take(Work::FillWound), work::take(Work::HoleWound)))
    };
    // Every element wound was charged first, and no more than the limit is wound: of fills and of holes.
    let charged = |n: usize, limit: usize| {
        let (charged, (fills, holes)) = measured(n, limit);
        assert!(fills + holes == charged.1 && charged.1 <= limit, "{fills} + {holes} wound, {} charged, of {limit}", charged.1);
        charged
    };
    let ((faces, w100, wound), (fills, holes)) = measured(100, usize::MAX);
    assert_eq!(faces, 100);
    assert!((5 * 100 * 100..=7 * 100 * 100).contains(&w100), "{w100}");
    assert!(fills > 0 && holes > 0 && fills + holes == w100, "{fills} and {holes} wound of {w100} charged");
    // Short of the work, each winding charged before the budget ran out is made, and none after.
    for limit in [w100 / 2, w100 / 10, 1] {
        assert_eq!(charged(100, limit).0, 0, "{limit}");
    }
    let (faces, w200, _) = charged(200, usize::MAX);
    assert_eq!(faces, 200);
    assert!(w200 > 39 * w100 / 10 && w200 < 41 * w100 / 10, "{w100} and {w200}: twice as deep, four times the work");
    // At the work it needs, the same map; one under it, none, and no more charged than the limit.
    assert_eq!(charged(100, w100), (100, w100, wound));
    let (faces, w, fewer) = charged(100, w100 - 1);
    assert_eq!(faces, 0);
    assert!(w < w100 && fewer < wound, "{w} charged, {fewer} subpaths wound");
    // With none, nothing is wound.
    assert_eq!(charged(100, 0), (0, 0, 0));
    // Ordinary art is far from it: 2,000 squares in a column, one shape, charge each face its square.
    let mut col = BezPath::new();
    for j in 0..2_000 {
        col.extend(rect_path(0.0, 3.0 * f64::from(j), 1.0, 3.0 * f64::from(j) + 1.0).iter());
    }
    work::reset();
    assert_eq!(live_paint(&[Shape::new(col, FillRule::NonZero, 0)]).0.len(), 2_000);
    assert!(work::take(Work::FillElements) <= 6 * 2_000, "a few elements a face");
}
