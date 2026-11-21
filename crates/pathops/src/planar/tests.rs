use super::*;
use craft_geom::{Rect, shapes};
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
    Shape::new(PathData::single(SubPath::polyline(&[Point::new(a.0, a.1), Point::new(b.0, b.1)], false)), FillRule::NonZero, key)
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64, key: u64) -> Shape {
    Shape::new(shapes::rectangle(Rect::new(x0, y0, x1, y1)), FillRule::NonZero, key)
}

fn area(r: &Region) -> f64 {
    r.path.to_bezpath().area()
}

fn at(faces: &[Region], x: f64, y: f64) -> Vec<&Region> {
    faces.iter().filter(|f| f.contains(Point::new(x, y))).collect()
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
    assert_eq!(f.path.subpaths.len(), 1);
    assert_eq!(f.path.subpaths[0].anchors.len(), 3, "the loose ends are not part of it");
    // Each line is cut where the others cross it: 3 + 3 + 3 pieces.
    assert_eq!(edges.len(), 9);
    assert_eq!(edges.iter().filter(|e| e.key == 2).count(), 3);
}

fn open(points: &[(f64, f64)], key: u64) -> Shape {
    let pts: Vec<Point> = points.iter().map(|&(x, y)| Point::new(x, y)).collect();
    Shape::new(PathData::single(SubPath::polyline(&pts, false)), FillRule::NonZero, key)
}

#[test]
fn gap_detection_closes_a_square_drawn_with_a_gap() {
    // One open path round a square, its end 2 pt short of its start.
    let s = [open(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 2.0)], 0)];
    assert!(live_paint(&s).0.is_empty(), "paint leaks through the gap");
    let (faces, edges, gaps) = live_paint_with_gaps(&s, 3.0);
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    let g = gaps[0];
    assert!(g[0].distance(Point::new(0.0, 0.0)) < 1e-9 && g[1].distance(Point::new(0.0, 2.0)) < 1e-6, "{g:?}");
    assert_eq!(faces.len(), 1, "{faces:?}");
    assert!((area(&faces[0]) - 10000.0).abs() < 1e-6, "{}", area(&faces[0]));
    assert_eq!(edges, live_paint(&s).1, "the gap adds no edge");
    // A gap wider than the size stays open.
    let (faces, _, gaps) = live_paint_with_gaps(&s, 1.5);
    assert!(faces.is_empty() && gaps.is_empty());
}

#[test]
fn gaps_close_to_curves_and_between_paths() {
    // A line across a circle, each end 2 pt short of it: the circle splits in two.
    let circle = Shape::new(shapes::ellipse(Rect::new(50.0, 0.0, 150.0, 100.0)), FillRule::NonZero, 0);
    let s = [circle, line((52.0, 50.0), (148.0, 50.0), 1)];
    assert_eq!(live_paint(&s).0.len(), 1);
    let (faces, _, gaps) = live_paint_with_gaps(&s, 3.0);
    assert_eq!(gaps.len(), 2, "{gaps:?}");
    for g in &gaps {
        assert!((g[0].distance(g[1]) - 2.0).abs() < 1e-6, "{g:?}");
    }
    assert_eq!(faces.len(), 2, "{faces:?}");
    assert_eq!(at(&faces, 100.0, 25.0).len(), 1);
    assert_eq!(at(&faces, 100.0, 75.0).len(), 1);
    // Four lines round a square whose corners miss by 1 pt: the gaps are listed once each.
    let s = [
        line((0.0, 0.0), (99.0, 0.0), 0),
        line((100.0, 1.0), (100.0, 99.0), 1),
        line((99.0, 100.0), (1.0, 100.0), 2),
        line((0.0, 99.0), (0.0, 1.0), 3),
    ];
    let (faces, _, gaps) = live_paint_with_gaps(&s, 3.0);
    assert_eq!(gaps.len(), 4, "{gaps:?}");
    assert_eq!(faces.len(), 1, "{faces:?}");
    // One curve that comes back round to 2 pt from its start.
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    bp.curve_to((100.0, 0.0), (100.0, 100.0), (0.0, 2.0));
    let s = [Shape::new(PathData::from_bezpath(&bp), FillRule::NonZero, 0)];
    let (faces, _, gaps) = live_paint_with_gaps(&s, 3.0);
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert!((gaps[0][0].distance(gaps[0][1]) - 2.0).abs() < 1e-9, "{gaps:?}");
    assert_eq!(faces.len(), 1, "{faces:?}");
    // A line far longer than the gap, and many lines far from each other.
    let mut s = vec![line((0.0, 0.0), (100000.0, 0.0), 0), line((50000.0, 2.0), (50000.0, 100.0), 1)];
    s.extend((0..2000).map(|k| {
        let (x, y) = (f64::from(k % 50) * 40.0, 1000.0 + f64::from(k / 50) * 40.0);
        line((x, y), (x + 10.0, y), 2)
    }));
    let gaps = find_gaps(&s, 3.0);
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert!(gaps[0][0].distance(Point::new(50000.0, 2.0)) < 1e-9 && gaps[0][1].distance(Point::new(50000.0, 0.0)) < 1e-9, "{gaps:?}");
    // Paths far from the origin: one gap between two loose ends, listed once.
    let s = [line((1e20, 0.0), (1e20, 100.0), 0), line((1e20, 102.0), (1e20, 200.0), 1)];
    assert_eq!(find_gaps(&s, 3.0).len(), 1, "{:?}", find_gaps(&s, 3.0));
}

#[test]
fn overshoots_and_joined_ends_are_no_gaps() {
    // Ends that cross a line just before they stop, square to it and at 30 degrees.
    let t = 30f64.to_radians();
    let s = [
        line((-10.0, 0.0), (110.0, 0.0), 0),
        line((20.0, 100.0), (20.0, -1.0), 1),
        line((80.0 + 100.0 * t.cos(), 100.0 * t.sin()), (80.0 - t.cos(), -t.sin()), 2),
    ];
    assert!(find_gaps(&s, 3.0).is_empty(), "{:?}", find_gaps(&s, 3.0));
    // An end on another path, or on its own, is joined there.
    let s = [line((0.0, 0.0), (100.0, 0.0), 0), line((50.0, 0.0), (50.0, 100.0), 1)];
    assert!(find_gaps(&s, 3.0).is_empty(), "{:?}", find_gaps(&s, 3.0));
    // Another path 2 pt away does not make a gap from the joined end.
    let s = [line((0.0, 0.0), (100.0, 0.0), 0), line((50.0, 0.0), (50.0, 100.0), 1), line((40.0, -2.0), (60.0, -2.0), 2)];
    let gaps = find_gaps(&s, 3.0);
    assert!(!gaps.is_empty() && gaps.iter().all(|g| g[0].distance(Point::new(50.0, 0.0)) > 1e-9), "{gaps:?}");
    let six = [open(&[(0.0, 0.0), (0.0, 100.0), (50.0, 100.0), (50.0, 50.0), (0.0, 50.0)], 0)];
    assert!(find_gaps(&six, 3.0).is_empty(), "{:?}", find_gaps(&six, 3.0));
    // Sizes that are not a length find nothing.
    let square = [open(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 2.0)], 0)];
    for size in [0.0, -3.0, f64::NAN, f64::INFINITY] {
        assert!(find_gaps(&square, size).is_empty(), "{size}");
    }
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
    assert_eq!(outer[0].path.subpaths.len(), 2, "the inner square is a hole in it");
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
    let circle = Shape::new(shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), FillRule::NonZero, 0);
    let (faces, edges) = live_paint(&[circle, line((-10.0, 50.0), (110.0, 50.0), 1)]);
    assert_eq!(faces.len(), 2);
    for f in &faces {
        assert!((area(f) - std::f64::consts::PI * 2500.0 / 2.0).abs() < 2.0, "{}", area(f));
        assert!(f.path.subpaths[0].anchors.len() <= 4, "no extra anchors from the sweep's splits");
    }
    assert_eq!(edges.iter().filter(|e| e.key == 0).count(), 2, "two arcs");
}

/// Coincident curved boundaries retain the fill coverage of the regions they separate.
#[test]
fn coincident_curves_keep_their_source_faces() {
    fn check(diameter: f64, offset: f64, rule: FillRule, reverse: bool, swap: bool) {
        let mut ring = circle(0.5 * diameter, 0.5 * diameter, 0.5 * diameter, 0);
        let mut hole = circle(0.5 * diameter, 0.5 * diameter, 0.3 * diameter, 1).path;
        hole.reverse();
        ring.path.subpaths.extend(hole.subpaths);
        ring.rule = rule;
        let mut disk = circle(0.5 * diameter + offset, 0.5 * diameter, 0.3 * diameter, 1);
        disk.rule = rule;
        if reverse {
            ring.path.reverse();
            disk.path.reverse();
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
            assert_eq!(ring_face.path.subpaths.len(), 2, "{label}: the ring keeps its hole");
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

/// A strip merged across an axonometric grid of lines (#893, the reporter's file): faces meeting
/// at a vertex share its exact coordinates, so the merge is one outline with no edge left inside.
#[test]
fn faces_share_their_corners_exactly_so_merging_them_leaves_no_seam() {
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
    let corners: Vec<Point> = faces.iter().flat_map(|f| f.path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p))).collect();
    for p in &corners {
        for q in &corners {
            assert!(p == q || p.distance(*q) > 1e-6, "one corner, two positions: {p:?} {q:?}");
        }
    }
    // Down the strip between the lines x ≈ 498.9 and x = 540: 11 triangles, one outline.
    let mut strip: Vec<&Region> = vec![];
    for k in 0..=240 {
        if let Some(f) = faces.iter().find(|f| f.contains(Point::new(519.43, 420.0 + f64::from(k))))
            && !strip.iter().any(|g| std::ptr::eq(*g, f))
        {
            strip.push(f);
        }
    }
    assert_eq!(strip.len(), 11);
    // As the Shape Builder merges them: in the map, so no sweep can round the corners apart.
    let merged = crate::FaceMerger::new(&s).merge(&strip);
    assert_eq!(merged.subpaths.len(), 1, "{merged:?}");
    let want: f64 = strip.iter().map(|f| area(f)).sum();
    assert!((crate::area(&merged, FillRule::NonZero) - want).abs() < 1e-6 * want);
}

/// Merging faces of a grid of lines in the map: a ring keeps its hole, cells that only touch at
/// a corner stay two outlines, a line's loose end inside the merge leaves no spur.
#[test]
fn faces_merge_in_the_map_with_holes_corners_and_loose_ends() {
    let mut s = vec![];
    for k in 0..4 {
        let v = f64::from(k) * 10.0;
        s.push(line((v, -5.0), (v, 35.0), 0));
        s.push(line((-5.0, v), (35.0, v), 1));
    }
    // A loose end poking into the middle cell.
    s.push(line((15.0, 10.0), (15.0, 15.0), 2));
    let (faces, _) = live_paint(&s);
    let cell = |x: f64, y: f64| at(&faces, x, y)[0];
    let merger = crate::FaceMerger::new(&s);
    let m = map(&s, true).unwrap();
    let check = |picked: &[&Region], subpaths: usize, area_want: f64| {
        // Merged in the map itself, not by the fallback union.
        let inside: Vec<Point> = picked.iter().map(|f| interior_point(&f.path).unwrap()).collect();
        assert_eq!(m.merge(&inside, &crate::pathfinder::cover(picked, s.len()), None).as_ref(), Some(&merger.merge(picked)));
        let m = merger.merge(picked);
        assert_eq!(m.subpaths.len(), subpaths, "{m:?}");
        assert!((crate::area(&m, FillRule::NonZero) - area_want).abs() < 1e-6, "{}", crate::area(&m, FillRule::NonZero));
        m
    };
    let all: Vec<&Region> = faces.iter().collect();
    let whole = check(&all, 1, 900.0);
    assert_eq!(whole.subpaths[0].anchors.len(), 4, "one square, no anchors left from the inner lines or the loose end");
    let ring: Vec<&Region> =
        [(5., 5.), (15., 5.), (25., 5.), (5., 15.), (25., 15.), (5., 25.), (15., 25.), (25., 25.)].iter().map(|&(x, y)| cell(x, y)).collect();
    check(&ring, 2, 800.0);
    check(&[cell(5.0, 5.0), cell(15.0, 15.0)], 2, 200.0);
}

/// Merging in the map takes in a sliver next to the merged faces that only their shapes fill,
/// such as the overlap of two squares that overlap by 0.001, and leaves a thin cutout that
/// nothing fills as it is.
#[test]
fn faces_merge_in_the_map_with_the_slivers_their_shapes_fill() {
    let far = line((400.0, 400.0), (450.0, 450.0), 2);
    let s = [rect(100.0, 100.0, 200.0, 200.0, 0), rect(199.999, 100.0, 299.999, 200.0, 1), far.clone()];
    let regions = shape_builder(&s, false).regions;
    assert_eq!(regions.len(), 2, "the overlap is a sliver: {regions:?}");
    let both: Vec<&Region> = regions.iter().collect();
    let merged = crate::FaceMerger::new(&s).merge(&both);
    assert_eq!(merged.subpaths.len(), 1, "{merged:?}");
    assert!((crate::area(&merged, FillRule::NonZero) - 19999.9).abs() < 1e-6, "{}", crate::area(&merged, FillRule::NonZero));
    // Live Paint's map does not test which inputs cover the sliver.
    let covered = |cover: bool| map(&s, cover).unwrap().slivers.into_iter().map(|(_, sources)| sources).collect::<Vec<_>>();
    assert_eq!(covered(true), [vec![0, 1]]);
    assert_eq!(covered(false), [Vec::<usize>::new()]);
    // A square with a cutout 0.005 wide.
    let mut frame = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
    frame.subpaths.extend(shapes::rectangle(Rect::new(50.0, 10.0, 50.005, 90.0)).subpaths);
    let s = [Shape::new(frame, FillRule::EvenOdd, 0), far];
    let regions = shape_builder(&s, false).regions;
    assert_eq!(regions.len(), 1, "{regions:?}");
    let merged = crate::FaceMerger::new(&s).merge(&[&regions[0]]);
    assert_eq!(merged.subpaths.len(), 2, "the cutout stays: {merged:?}");
    assert!((crate::area(&merged, FillRule::EvenOdd) - 9999.6).abs() < 1e-6, "{}", crate::area(&merged, FillRule::EvenOdd));
}

#[test]
fn shape_builder_lines_cut_regions() {
    // A line across a rectangle: two regions, the line in three pieces keyed by its index.
    let s = [rect(0.0, 0.0, 100.0, 50.0, 7), line((40.0, -20.0), (60.0, 70.0), 8)];
    let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
    assert_eq!(regions.len(), 2);
    assert!(regions.iter().all(|r| r.sources == [0]));
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|l| l.key == 1 && !l.path.is_closed()));
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
    let span = |l: &Shape| l.path.bounds().map(|b| (b.x0, b.x1));
    assert_eq!(lines.iter().map(span).collect::<Vec<_>>(), [Some((100.0, 200.0))], "{lines:?}");
    assert_eq!(lines_under.iter().map(span).collect::<Vec<_>>(), [Some((0.0, 100.0))], "{lines_under:?}");
    assert!(lines.iter().chain(&lines_under).all(|l| l.key == 0));
    // The rectangle's outline keeps its edge there.
    assert!(!edges.is_empty() && edges.iter().all(|e| e.key == 1 && e.path.bounds() == Some(Rect::new(0.0, 0.0, 100.0, 50.0))), "{edges:?}");
    // Live Paint keeps one edge there, the front-most input's.
    let (_, pieces) = live_paint(&s);
    assert!(!pieces.iter().any(|p| p.key == 0 && p.path.bounds().is_some_and(|b| b.x1 <= 100.0)), "{pieces:?}");
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
        let mut v: Vec<_> = pieces.iter().filter(|l| l.key == key).filter_map(|l| l.path.bounds().map(|b| (b.x0, b.x1))).collect();
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
    // Live Paint's edges stay one per stretch, the front line's.
    assert_eq!(live_paint(&s).1.iter().filter(|e| e.key == 0).count(), 0);
}

#[test]
fn shape_builder_merges_adjacent_gap_closed_faces_without_keeping_bridges() {
    let outline = Shape::new(
        PathData::single(SubPath::polyline(
            &[Point::new(0.0, 4.0), Point::new(0.0, 100.0), Point::new(100.0, 100.0), Point::new(100.0, 0.0), Point::new(0.0, 0.0)],
            false,
        )),
        FillRule::NonZero,
        0,
    );
    let shapes = [outline, line((50.0, 0.0), (50.0, 100.0), 1)];
    let arr = shape_builder_with_gap(&shapes, true, 6.0);
    assert_eq!(arr.regions.len(), 2);
    let faces: Vec<_> = arr.regions.iter().collect();
    let merged = crate::FaceMerger::with_gap(&shapes, 6.0).merge(&faces);
    assert!((crate::area(&merged, FillRule::NonZero).abs() - 10000.0).abs() < 1e-6);
    assert_eq!(merged.subpaths.len(), 1, "the shared edge disappears");
    assert!(arr.lines.iter().chain(&arr.edges).all(|p| p.key < 2), "bridges are not art");
    // A request over 72 pt cannot close a gap of 80 pt.
    let large = [Shape::new(
        PathData::single(SubPath::polyline(
            &[Point::new(0.0, 80.0), Point::new(0.0, 200.0), Point::new(200.0, 200.0), Point::new(200.0, 0.0), Point::new(0.0, 0.0)],
            false,
        )),
        FillRule::NonZero,
        0,
    )];
    assert!(shape_builder_with_gap(&large, false, 1000.0).regions.is_empty());
}

#[test]
fn shape_builder_gap_detection_closes_small_gaps() {
    // Three sides of a 100 x 100 square, the fourth open by 4: no region without Gap Detection.
    let open = |gap: f64| {
        let pts = [(0.0, gap), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0), (0.0, 0.0)];
        let pts: Vec<Point> = pts.iter().map(|&(x, y)| Point::new(x, y)).collect();
        vec![Shape::new(PathData::single(SubPath::polyline(&pts, false)), FillRule::NonZero, 0)]
    };
    let s = open(4.0);
    assert!(shape_builder(&s, false).regions.is_empty());
    assert!(shape_builder_with_gap(&s, false, 3.0).regions.is_empty(), "a gap longer than the length stays open");
    let arr = shape_builder_with_gap(&s, true, 6.0);
    assert_eq!(arr.regions.len(), 1);
    assert!((area(&arr.regions[0]) - 10000.0).abs() < 1e-6);
    // The bridge is no art: the path comes back as it was, in its own pieces only.
    assert!(arr.lines.iter().chain(&arr.edges).all(|p| p.key == 0));
    let len: f64 = arr.lines.iter().map(|p| p.path.to_bezpath().perimeter(1e-6)).sum();
    assert!((len - 396.0).abs() < 1e-6, "{len}");
    // Two paths whose ends miss each other: a U and a lid 3 above it.
    let u = Shape::new(
        PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(0.0, 100.0), Point::new(100.0, 100.0), Point::new(100.0, 0.0)], false)),
        FillRule::NonZero,
        0,
    );
    let s = [u, line((0.0, -3.0), (100.0, -3.0), 1)];
    assert!(shape_builder_with_gap(&s, false, 2.0).regions.is_empty());
    let arr = shape_builder_with_gap(&s, false, 3.5);
    assert_eq!(arr.regions.len(), 1);
    assert!((area(&arr.regions[0]) - 10300.0).abs() < 1e-6);
    // An end close to the side of another path bridges to the nearest point on it.
    let s = [rect(0.0, 0.0, 100.0, 100.0, 0), line((50.0, 140.0), (50.0, 104.0), 1)];
    assert_eq!(shape_builder_with_gap(&s, false, 5.0).regions.len(), 1, "nothing is enclosed, nothing breaks");
    // Bad gaps are no gap.
    let s = open(4.0);
    for gap in [f64::NAN, f64::INFINITY, -3.0] {
        assert!(shape_builder_with_gap(&s, false, gap).regions.is_empty());
    }
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

fn circle(cx: f64, cy: f64, r: f64, key: u64) -> Shape {
    Shape::new(shapes::ellipse(Rect::new(cx - r, cy - r, cx + r, cy + r)), FillRule::NonZero, key)
}

#[test]
fn shape_builder_offers_the_gaps_closed_shapes_enclose() {
    // Four circles in a 2×2 grid, each touching its neighbours (#1691): the four circles and
    // the diamond between them, which nothing fills.
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
    // The gap merges with a bar in the planar map: the bar's piece and the square, no seam.
    let merger = crate::FaceMerger::new(&s);
    let bar = regions.iter().find(|r| r.sources == [0] && r.contains(Point::new(150.0, 50.0))).unwrap();
    let merged = merger.merge(&[bar, gaps[0]]);
    assert_eq!(merged.subpaths.len(), 1, "{merged:?}");
    assert!((crate::area(&merged, FillRule::NonZero) - 20000.0).abs() < 1e-6);
    // Shapes apart or overlapping without a hole enclose no gap.
    let s = [circle(50.0, 50.0, 50.0, 0), circle(250.0, 50.0, 50.0, 1), rect(80.0, 0.0, 220.0, 60.0, 2)];
    assert!(shape_builder(&s, false).regions.iter().all(|r| !r.sources.is_empty()));
}

#[test]
fn shape_builder_stays_quick_up_to_its_cap() {
    // A wavy ring of nearly the most segments the map takes, crossed by a few lines.
    let n = SHAPE_BUILDER_MAX_SEGMENTS - 8;
    let ring: Vec<Point> = (0..n)
        .map(|k| {
            let t = k as f64 / n as f64 * std::f64::consts::TAU;
            let r = 100.0 + 5.0 * (t * 200.0).sin();
            Point::new(r * t.cos(), r * t.sin())
        })
        .collect();
    let mut s = vec![Shape::new(PathData::single(SubPath::polyline(&ring, true)), FillRule::NonZero, 0)];
    // Closed paths alone are swept for the gaps they enclose.
    let started = work_done();
    assert_eq!(shape_builder(&s, false).regions.len(), 1, "the ring encloses no gap");
    // Reads of indexed items stay in proportion to the ring. Scanning the ring for each edge's
    // source would read about `n` items per edge.
    let most = 64 * n as u64;
    let work = work_done() - started;
    assert!(work <= most, "{work} reads");
    for k in 0..4 {
        let y = -60.0 + 40.0 * f64::from(k);
        s.push(line((-150.0, y), (150.0, y + 7.0), 1));
    }
    let started = work_done();
    let BuilderArrangement { regions, lines, .. } = shape_builder(&s, false);
    assert!(regions.len() >= 5, "the ring cut in five bands at least (the waves make pockets)");
    assert!(lines.len() >= 12, "{}", lines.len());
    let work = work_done() - started;
    assert!(work <= most, "{work} reads");
    // Past the cap, only the filled areas count.
    s.push(line((0.0, 0.0), (1.0, 1.0), 2));
    s[0].path.subpaths[0].anchors.extend((0..16).map(|k| craft_geom::Anchor::corner(Point::new(100.0, f64::from(k)))));
    assert!(shape_builder(&s, false).lines.is_empty());
}

#[test]
fn a_traced_outline_of_many_segments_is_cut_by_lines() {
    // #937: art traced to thousands of segments was past the old cap (4096), so lines stopped
    // cutting it and the Shape Builder offered the whole shapes. A blob of 20 000 segments
    // crossed by two long lines (far wider than any of its segments: the index of the
    // outlines keeps those apart) splits into four regions, each line in four pieces.
    let n = 20_000;
    let blob: Vec<Point> = (0..n)
        .map(|k| {
            let t = k as f64 / n as f64 * std::f64::consts::TAU;
            let r = 100.0 + 5.0 * (t * 37.0).sin();
            Point::new(r * t.cos(), r * t.sin())
        })
        .collect();
    let s = [
        Shape::new(PathData::single(SubPath::polyline(&blob, true)), FillRule::NonZero, 0),
        line((-150.0, 0.0), (150.0, 3.0), 1),
        line((0.0, -150.0), (3.0, 150.0), 2),
    ];
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
    // (quadratic in the hatch, #937). Filed by pieces, a lookup gets the lines of its
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

#[test]
fn rect_index_counts_each_entry_it_reads() {
    // Ten collinear lines filed under each of the 10 cells along them: a lookup 300 pt either side
    // reads every line from each of the cells it spans, more than once per line.
    let line = PathSeg::Line(kurbo::Line::new((0.0, 0.0), (1000.0, 0.0)));
    let items: Vec<_> = (0..10).map(|k| (line.bounding_box(), (line, k))).collect();
    let index = RectIndex::with_footprint(items, |r, (s, _), cell| segment_footprint(r, s, cell));
    let started = work_done();
    assert_eq!(index.at(Point::new(500.0, 0.0), 300.0).count(), 10);
    let work = work_done() - started;
    assert!(work >= 50, "{work} reads");
}

/// The grid's memory stays linear in the item count: pieces of one segment falling in the same
/// cell file it once, and coincident segments (each along every cell of a one-column grid) go
/// to the scanned list once the grid holds its budget of references, and are still all found.
#[test]
fn rect_index_holds_a_bounded_number_of_references() {
    // Ten collinear horizontal lines: a grid of 10 cells a hundred points wide and a hair tall,
    // so each line's 4096 pieces fall about 400 per cell; filed once per cell.
    let line = PathSeg::Line(kurbo::Line::new((0.0, 0.0), (1000.0, 0.0)));
    let items: Vec<_> = (0..10).map(|k| (line.bounding_box(), (line, k))).collect();
    let index = RectIndex::with_footprint(items, |r, (s, _), cell| segment_footprint(r, s, cell));
    assert!(index.rest.is_empty(), "every line is in the grid, {} scanned", index.rest.len());
    assert!(index.cells.iter().all(|c| c.len() <= 10), "{:?}", index.cells.iter().map(Vec::len).collect::<Vec<_>>());
    assert!(index.cells.iter().map(Vec::len).sum::<usize>() >= 10, "the lines are filed");
    assert_eq!(index.at(Point::new(500.0, 0.0), 0.1).count(), 10);
    // 4,096 coincident vertical segments: one column, a row per segment, every segment along
    // every row would be 16 million references; the budget keeps it to a few hundred per item.
    let n = 4096u32;
    let seg = PathSeg::Line(kurbo::Line::new((50.0, 0.0), (50.0, 200.0)));
    let items: Vec<_> = (0..n).map(|k| (seg.bounding_box(), (seg, k))).collect();
    let index = RectIndex::with_footprint(items, |r, (s, _), cell| segment_footprint(r, s, cell));
    let references: usize = index.cells.iter().map(Vec::len).sum();
    assert!(references <= reference_budget(n as usize, index.cells.len()), "{references} references");
    assert!(references < 2_000_000, "{references} references");
    assert!(!index.rest.is_empty() && index.rest.len() < n as usize, "{} scanned", index.rest.len());
    let mut got: Vec<u32> = index.at(Point::new(50.0, 100.0), 0.1).map(|(_, k)| *k).collect();
    got.sort_unstable();
    assert_eq!(got.len(), n as usize);
    assert_eq!(got, (0..n).collect::<Vec<_>>());
    assert_eq!(index.at(Point::new(60.0, 100.0), 0.1).count(), 0);
}

fn poly(pts: &[(f64, f64)], closed: bool) -> SubPath {
    SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed)
}

fn ends(sp: &SubPath) -> (Point, Point) {
    (sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p)
}

/// Each piece cuts out of the input it is keyed by, losing just its own length (what erasing
/// it does), and none is a speck.
fn assert_pieces_cut_out(shapes: &[Shape], arr: &BuilderArrangement) {
    for e in arr.edges.iter().chain(&arr.lines) {
        let len = e.path.length();
        assert!(len > 1e-3, "a speck of a piece, keyed {}: {:?}", e.key, e.path);
        let s = &shapes[e.key as usize];
        let cut = cut_out(&s.path, &e.path.subpaths[0], false, 0.05).unwrap_or_else(|| panic!("{:?} not on input {}", e.path, e.key));
        let lost = s.path.length() - cut.length();
        assert!((lost - len).abs() <= 1e-6 * s.path.length(), "input {} lost {lost} for a piece {len} long", e.key);
    }
}

#[test]
fn a_line_crossed_where_its_two_passes_part_is_cut_once() {
    // The shrunk case of `pieces_cut_out_of_their_inputs`: the sweep cut the first line's way
    // there and its way back by the third line a rounding error (1.2e-6) apart, joined by a
    // stub. The line came back as a speck keyed by the third line and a piece running from
    // the crossing out to (10, 70) and back to the other cut, which erasing took twice over.
    let s = [line((10.0, 70.0), (80.0, 20.0), 0), line((10.0, 40.0), (70.0, 90.0), 0), line((20.0, 80.0), (20.0, 50.0), 0)];
    let arr = shape_builder(&s, true);
    assert_pieces_cut_out(&s, &arr);
    // The first line cut by both others, the other two once each.
    let count = |k: u64| arr.lines.iter().filter(|l| l.key == k).count();
    assert_eq!((count(0), count(1), count(2)), (3, 2, 2), "{:?}", arr.lines);
    let at = Point::new(20.0, 70.0 - 50.0 / 7.0);
    for k in [0, 2] {
        assert!(arr.lines.iter().filter(|l| l.key == k).all(|l| {
            let (a, b) = ends(&l.path.subpaths[0]);
            a.distance(b) > 1.0 && l.path.subpaths[0].anchors.len() == 2
        }));
        assert_eq!(
            arr.lines
                .iter()
                .filter(|l| l.key == k && [ends(&l.path.subpaths[0]).0, ends(&l.path.subpaths[0]).1].iter().any(|p| p.distance(at) < 1e-9))
                .count(),
            2
        );
    }
}

#[test]
fn a_run_of_tiny_level_segments_is_not_contracted_as_stubs() {
    // 2000 segments of 1e-4 (0.2 pt in all), nearly level: each one alone is as short as a
    // stub the sweep leaves between two cuts, but chained they are real geometry, and the
    // stub contraction must not pull them into one vertex.
    let pts: Vec<Point> = (0..=2000).map(|k| Point::new(f64::from(k) * 1e-4, f64::from(k) * 1e-8)).collect();
    let run = Shape::new(PathData::single(SubPath::polyline(&pts, false)), FillRule::NonZero, 0);
    let whole = run.path.length();
    let s = [run, line((0.1003, -1.0), (0.1003, 1.0), 0)];
    let arr = shape_builder(&s, false);
    let pieces: Vec<&Shape> = arr.lines.iter().filter(|l| l.key == 0).collect();
    assert_eq!(pieces.len(), 2, "cut once by the line: {}", pieces.len());
    let total: f64 = pieces.iter().map(|l| l.path.length()).sum();
    // Where the line crosses, one segment may be taken for a stub (it is as short as one),
    // moving the cut by less than a stub's reach; the rest of the run stays.
    assert!((total - whole).abs() <= DEFAULT_PRECISION / 16.0, "{total} of {whole}");
    for l in &pieces {
        let (a, b) = ends(&l.path.subpaths[0]);
        assert!(a.distance(b) > 0.09, "a piece collapsed: {a:?} {b:?}");
    }
    assert_eq!(arr.lines.iter().filter(|l| l.key == 1).count(), 2);
}

#[test]
fn a_dense_hatch_at_exact_binary_steps_cuts_each_line_once_per_crossing() {
    // Lines every 0.25 or 0.5 pt (exact in binary) cut a crossing line or a circle where two
    // passes of each line met it a rounding error apart, adding a speck and a piece there.
    let hatch = |n: i32, step: f64, f: &dyn Fn(f64) -> Shape| -> Vec<Shape> { (1..=n).map(|k| f(step * f64::from(k))).collect() };
    // Horizontal lines across a diagonal: each in two pieces, the diagonal in n + 1.
    let mut s = vec![line((-3.0, -2.0), (33.0, 31.0), 0)];
    s.extend(hatch(119, 0.25, &|y| line((-5.0, y), (35.0, y), 0)));
    let arr = shape_builder(&s, false);
    assert_pieces_cut_out(&s, &arr);
    assert_eq!(arr.lines.iter().filter(|l| l.key == 0).count(), 120);
    assert!((1..=119).all(|k| arr.lines.iter().filter(|l| l.key == k).count() == 2));
    assert_eq!(arr.lines.len(), 120 + 2 * 119);
    // Sloping lines across a 45° one.
    let mut s = vec![line((-3.0, -3.0), (33.0, 33.0), 0)];
    s.extend(hatch(59, 0.5, &|v| line((v - 40.0, -5.0), (v + 40.0, 35.0), 0)));
    let arr = shape_builder(&s, false);
    assert_pieces_cut_out(&s, &arr);
    assert_eq!(arr.lines.len(), 60 + 2 * 59, "{}", arr.lines.len());
    // Vertical lines across a circle: n + 1 strips, each line in three pieces, the circle's
    // outline in two per line.
    let mut s = vec![Shape::new(shapes::ellipse(Rect::new(0.0, 0.0, 30.0, 30.0)), FillRule::NonZero, 0)];
    s.extend(hatch(119, 0.25, &|x| line((x, -5.0), (x, 35.0), 0)));
    let arr = shape_builder(&s, true);
    assert_pieces_cut_out(&s, &arr);
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
        assert_eq!(with.edges.iter().filter(|e| e.key == key && !e.path.is_closed()).count(), 2);
    }
    let inner = with.edges.iter().find(|e| e.key == 0 && e.path.bounds() == Some(Rect::new(50.0, 50.0, 100.0, 100.0))).expect("A inside B");
    // Erasing it opens A there.
    let cut = cut_out(&s[0].path, &inner.path.subpaths[0], false, 1e-3).expect("on A");
    assert_eq!(cut.subpaths.len(), 1);
    let sp = &cut.subpaths[0];
    assert!(!sp.closed && sp.anchors.len() == 5, "{sp:?}");
    let (a, b) = ends(sp);
    assert!([a, b].contains(&Point::new(100.0, 50.0)) && [a, b].contains(&Point::new(50.0, 100.0)), "{a:?} {b:?}");
    assert!((cut.length() - 300.0).abs() < 1e-6, "{}", cut.length());
    // Not on B.
    assert_eq!(cut_out(&s[1].path, &inner.path.subpaths[0], false, 1e-3), None);
    // A lone square's edge is its whole outline: erasing it leaves nothing.
    let lone = shape_builder(&s[..1], true);
    assert_eq!(lone.edges.len(), 1);
    assert!(cut_out(&s[0].path, &lone.edges[0].path.subpaths[0], false, 1e-3).unwrap().is_empty());
}

#[test]
fn cut_out_splits_open_paths_and_keeps_closing_edges_invisible() {
    let open = PathData::single(poly(&[(0.0, 0.0), (100.0, 0.0)], false));
    let cut = cut_out(&open, &poly(&[(60.0, 0.0), (30.0, 0.0)], false), false, 1e-3).unwrap();
    let mut parts: Vec<(Point, Point)> = cut.subpaths.iter().map(ends).collect();
    parts.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    assert_eq!(parts, vec![(Point::new(0.0, 0.0), Point::new(30.0, 0.0)), (Point::new(60.0, 0.0), Point::new(100.0, 0.0))]);
    // An end piece only shortens it.
    let cut = cut_out(&open, &poly(&[(70.0, 0.0), (100.0, 0.0)], false), false, 1e-3).unwrap();
    assert_eq!(cut.subpaths.iter().map(ends).collect::<Vec<_>>(), vec![(Point::new(0.0, 0.0), Point::new(70.0, 0.0))]);

    // A filled open path runs on round its invisible closing edge (100,100) → (0,0).
    let corner = PathData::single(poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false));
    let piece = poly(&[(100.0, 50.0), (100.0, 100.0), (50.0, 50.0)], false);
    assert_eq!(cut_out(&corner, &piece, false, 1e-3), None, "without the fill it isn't on the path");
    let cut = cut_out(&corner, &piece, true, 1e-3).unwrap();
    assert_eq!(cut.subpaths.len(), 1);
    assert_eq!(cut.subpaths[0].anchors.iter().map(|a| a.p).collect::<Vec<_>>(), [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)].map(Point::from));
    // Cut elsewhere, the closing edge stays invisible: two open runs, not one through it.
    let cut = cut_out(&corner, &poly(&[(20.0, 0.0), (60.0, 0.0)], false), true, 1e-3).unwrap();
    assert_eq!(cut.subpaths.len(), 2);
    assert!(cut.subpaths.iter().all(|sp| !sp.closed));
    assert!((cut.length() - 160.0).abs() < 1e-6, "{}", cut.length());
}

#[test]
fn run_around_stops_at_the_stops_either_side() {
    // A lone square's edge is its whole outline; with its corners as stops, a point by any side
    // gives that side.
    let lone = shape_builder(&[rect(0.0, 0.0, 100.0, 100.0, 0)], true);
    let edge = &lone.edges[0].path.subpaths[0];
    let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)].map(Point::from);
    let side = |at: (f64, f64)| {
        run_around(edge, &corners, Point::from(at), 1e-3).map(|sp| (sp.closed, PathData::single(sp.clone()).length(), PathData::single(sp).bounds()))
    };
    assert_eq!(side((50.0, 1.0)), Some((false, 100.0, Some(Rect::new(0.0, 0.0, 100.0, 0.0)))));
    for at in [(0.5, 50.0), (99.0, 50.0), (50.0, 100.0)] {
        let (closed, length, _) = side(at).expect("a side");
        assert!(!closed && (length - 100.0).abs() < 1e-6, "{at:?}: {length}");
    }
    // Without two stops on it, the run would be all of the outline.
    assert_eq!(run_around(edge, &corners[..1], Point::new(50.0, 0.0), 1e-3), None);
    // An open piece's run ends at the piece's own ends where no stop comes first; (40, 5) lies
    // off the line and does not stop the run.
    let line = poly(&[(0.0, 0.0), (100.0, 0.0)], false);
    let run = run_around(&line, &[Point::new(40.0, 0.0), Point::new(40.0, 5.0)], Point::new(70.0, 0.0), 1e-3).unwrap();
    assert_eq!(ends(&run), (Point::new(40.0, 0.0), Point::new(100.0, 0.0)));
    let run = run_around(&line, &[Point::new(40.0, 0.0)], Point::new(10.0, 0.0), 1e-3).unwrap();
    assert_eq!(ends(&run), (Point::new(0.0, 0.0), Point::new(40.0, 0.0)));
    // A stop counts only where it is nearest the piece: the folded line's last anchor lies
    // 0.02 from its first side and does not stop the run there.
    let folded = poly(&[(0.0, 0.0), (100.0, 0.0), (50.0, 0.02)], false);
    let stops = folded.anchors.iter().map(|a| a.p).collect::<Vec<_>>();
    let run = run_around(&folded, &stops, Point::new(25.0, 0.0), 0.05).unwrap();
    assert_eq!(ends(&run), (Point::new(0.0, 0.0), Point::new(100.0, 0.0)));
}

#[test]
fn run_around_a_ring_of_4000_sides_measures_only_the_stops_it_reaches() {
    // Every anchor of the ring is a stop, so each run is one side. Measuring every stop along
    // the whole ring would take 16 million nearest-point searches a call.
    let n = 4000;
    let pts: Vec<(f64, f64)> = (0..n)
        .map(|k| {
            let a = k as f64 * std::f64::consts::TAU / n as f64;
            (500.0 * a.cos(), 500.0 * a.sin())
        })
        .collect();
    let ring = poly(&pts, true);
    let stops: Vec<Point> = ring.anchors.iter().map(|a| a.p).collect();
    let started = work_done();
    for k in (0..n).step_by(200) {
        let run = run_around(&ring, &stops, ring.segment(k).eval(0.5), 1e-3).expect("a side");
        assert_eq!(ends(&run), (stops[k], stops[(k + 1) % n]), "side {k}");
    }
    let work = work_done() - started;
    assert!(work <= 1_000_000, "20 runs made {work} searches");
}

/// Proptest shrank a failure of `pieces_cut_out_of_their_inputs` to this input with linesweeper
/// 0.5. The sweep cuts lines 0 and 1 at points of their crossing up to 1.4 `eps` apart, and
/// `vertex_id` merges a point only into a vertex within `eps` of it. Line 0's loose end in the
/// square is still one piece, and erasing it shortens line 0 by its length only. No face or
/// merged outline runs out along it and back.
#[test]
fn a_loose_end_is_one_piece_when_the_sweep_cuts_its_passes_apart() {
    let s = [line((30.0, 20.0), (70.0, 0.0), 0), line((40.0, 0.0), (0.0, 90.0), 1), rect(10.0, 0.0, 40.0, 30.0, 2)];
    let arr = shape_builder(&s, true);
    for key in 0..3 {
        assert_eq!(arr.edges.iter().chain(&arr.lines).filter(|e| e.key == key).count(), 3, "pieces of {key}");
    }
    for e in arr.edges.iter().chain(&arr.lines) {
        let input = &s[e.key as usize].path;
        let cut = cut_out(input, &e.path.subpaths[0], false, 0.05).expect("on its input");
        assert!((input.length() - cut.length() - e.path.length()).abs() < 1e-6, "{:?}", e.path);
    }
    // From (30,20) to the crossing at (220/7, 135/7).
    let loose = arr.lines.iter().find(|l| l.path.subpaths[0].anchors.iter().any(|a| a.p == Point::new(30.0, 20.0))).expect("the loose end");
    assert_eq!(loose.path.subpaths[0].anchors.len(), 2, "{:?}", loose.path);
    assert!((loose.path.length() - 2000f64.sqrt() / 28.0).abs() < 1e-6, "{}", loose.path.length());
    let (faces, _) = live_paint(&s);
    let left = at(&faces, 15.0, 15.0);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].path.subpaths[0].anchors.len(), 4, "(10,0) (40,0) (26.67,30) (10,30): {:?}", left[0].path);
    let merged = crate::FaceMerger::new(&s).merge(&faces.iter().collect::<Vec<_>>());
    assert_eq!(merged.subpaths.len(), 1, "the square: {merged:?}");
    assert_eq!(merged.subpaths[0].anchors.len(), 4, "{merged:?}");
}

fn arb_segment() -> impl Strategy<Value = Shape> {
    let c = || (0..10i32).prop_map(|v| f64::from(v) * 10.0);
    prop_oneof![
        (c(), c(), c(), c()).prop_map(|(a, b, x, y)| line((a, b), (x, y), 0)),
        (c(), c(), 1..5i32, 1..5i32).prop_map(|(x, y, w, h)| rect(x, y, x + f64::from(w) * 10.0, y + f64::from(h) * 10.0, 0)),
        (c(), c(), 1..5i32).prop_map(|(x, y, r)| {
            let r = f64::from(r) * 10.0;
            Shape::new(shapes::ellipse(Rect::new(x, y, x + r, y + r)), FillRule::EvenOdd, 0)
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Faces never overlap, and a face's sources are exactly the inputs whose fill holds a
    /// point inside it.
    #[test]
    fn faces_are_disjoint_and_know_what_covers_them(shapes in prop::collection::vec(arb_segment(), 1..7)) {
        let (faces, _) = live_paint(&shapes);
        for f in &faces {
            prop_assert!(area(f) > 0.0);
            let Some(p) = crate::interior_point(&f.path) else { continue };
            prop_assert_eq!(faces.iter().filter(|g| g.contains(p)).count(), 1, "{:?} in more than one face", p);
            let covering: Vec<usize> = shapes
                .iter()
                .enumerate()
                .filter(|(_, s)| s.path.is_closed() && craft_geom::hit::fill_contains(&s.path.to_bezpath(), s.rule, p))
                .map(|(i, _)| i)
                .collect();
            prop_assert_eq!(&f.sources, &covering, "at {:?}", p);
        }
    }

    /// Closed shapes alone: the filled faces cover exactly the Pathfinder regions.
    #[test]
    fn filled_faces_are_the_pathfinder_regions(shapes in prop::collection::vec(arb_segment(), 1..6)) {
        let closed: Vec<Shape> = shapes.into_iter().filter(|s| s.path.is_closed()).collect();
        let (faces, _) = live_paint(&closed);
        let filled: f64 = faces.iter().filter(|f| !f.sources.is_empty()).map(area).sum();
        let regions: f64 = crate::regions(&closed).iter().map(|r| area(r).abs()).sum();
        // Both refit split curves to [`DEFAULT_PRECISION`], so curved areas may differ by up
        // to that much along every outline.
        let outlines: f64 = closed.iter().map(|s| s.path.to_bezpath().perimeter(1e-6)).sum();
        prop_assert!((filled - regions).abs() <= 1e-9 + DEFAULT_PRECISION * outlines, "{} vs {}", filled, regions);
    }

    /// Every edge and line piece cuts out of the input it is keyed by, which loses its length
    /// and nothing else.
    #[test]
    fn pieces_cut_out_of_their_inputs(shapes in prop::collection::vec(arb_segment(), 1..6)) {
        let arr = shape_builder(&shapes, true);
        for e in arr.edges.iter().chain(&arr.lines) {
            let (Some(s), Some(piece)) = (shapes.get(e.key as usize), e.path.subpaths.first()) else { continue };
            let len = e.path.length();
            let cut = cut_out(&s.path, piece, false, 0.05);
            if len <= 0.05 {
                // A speck where two paths touch.
                prop_assert!(cut.is_none());
                continue;
            }
            prop_assert!(cut.is_some(), "{:?} not on {:?}", piece, s.path);
            let lost = s.path.length() - cut.map_or(0.0, |c| c.length());
            prop_assert!((lost - len).abs() <= 0.05 + 1e-3 * len, "lost {} for a piece {} long", lost, len);
        }
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

#[test]
fn many_lines_stay_quick() {
    // 60 lines through a 100 pt square: about 900 crossings.
    let lines: Vec<Shape> = (0..60)
        .map(|k| {
            let t = f64::from(k) * 0.37;
            line((50.0 + 80.0 * t.cos(), 50.0 + 80.0 * t.sin()), (50.0 - 80.0 * (t + 1.1).cos(), 50.0 - 80.0 * (t + 1.1).sin()), 0)
        })
        .collect();
    let started = work_done();
    let (faces, _) = live_paint(&lines);
    assert!(faces.len() > 500, "{}", faces.len());
    let work = work_done() - started;
    assert!(work <= 150_000, "{work} reads");
}

#[test]
fn holes_touching_at_a_point_stay_holes() {
    // Two small circles inside a big one, touching each other at (30,85): the big circle's face
    // has both as holes, and every face and region is one of the three.
    let circle = |cx: f64, cy: f64, r: f64| Shape::new(shapes::ellipse(Rect::new(cx - r, cy - r, cx + r, cy + r)), FillRule::EvenOdd, 0);
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
