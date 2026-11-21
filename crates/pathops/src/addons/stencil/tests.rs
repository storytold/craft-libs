use craft_geom::{FillRule, PathData, SubPath};
use kurbo::{BezPath, Point, Shape};

use super::*;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> SubPath {
    SubPath::polyline(&[Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)], true)
}

/// A square ring: `outer` with the hole `inner`, each (x0, y0, x1, y1).
fn ring(outer: (f64, f64, f64, f64), inner: (f64, f64, f64, f64)) -> PathData {
    PathData::new(vec![rect(outer.0, outer.1, outer.2, outer.3), rect(inner.0, inner.1, inner.2, inner.3)])
}

fn spec(width: f64, count: usize, direction: BridgeDirection) -> BridgeSpec {
    BridgeSpec { width, count, direction }
}

fn run(art: &PathData, spec: BridgeSpec) -> Bridged {
    stencil_bridges(&[(art, FillRule::EvenOdd)], spec).unwrap()
}

/// Is `(x, y)` inside the area `p` draws?
fn inside(p: &PathData, x: f64, y: f64) -> bool {
    p.to_bezpath().winding(Point::new(x, y)) != 0
}

/// The islands left once the bridges are cut.
fn islands_left(b: &Bridged) -> usize {
    run(&b.path, spec(1.0, 1, BridgeDirection::Shortest)).islands
}

/// A letter O made square: 100 pt across with bars 20 pt thick.
fn letter_o() -> PathData {
    ring((0.0, 0.0, 100.0, 100.0), (20.0, 20.0, 80.0, 80.0))
}

#[test]
fn an_o_gets_a_bridge_and_its_island_joins_the_sheet() {
    let b = run(&letter_o(), spec(4.0, 1, BridgeDirection::Vertical));
    assert_eq!((b.islands, b.bridges, b.unbridged), (1, 1, 0));
    assert_eq!(islands_left(&b), 0, "the inside of the O is joined to the sheet");
    // The bridge crosses the top bar at the middle of the hole; the rest of the O stays.
    assert!(!inside(&b.path, 50.0, 10.0));
    assert!(inside(&b.path, 30.0, 10.0) && inside(&b.path, 50.0, 90.0) && inside(&b.path, 10.0, 50.0));
}

#[test]
fn the_bridge_is_as_wide_as_asked() {
    for width in [2.0, 6.0] {
        let b = run(&letter_o(), spec(width, 1, BridgeDirection::Vertical));
        let half = width / 2.0;
        assert!(!inside(&b.path, 50.0 - half * 0.9, 10.0) && !inside(&b.path, 50.0 + half * 0.9, 10.0), "{width}");
        assert!(inside(&b.path, 50.0 - half * 1.2, 10.0) && inside(&b.path, 50.0 + half * 1.2, 10.0), "{width}");
    }
}

#[test]
fn horizontal_bridges_cross_the_sides() {
    let b = run(&letter_o(), spec(4.0, 1, BridgeDirection::Horizontal));
    assert_eq!((b.bridges, islands_left(&b)), (1, 0));
    assert!(!inside(&b.path, 10.0, 50.0) || !inside(&b.path, 90.0, 50.0));
    assert!(inside(&b.path, 50.0, 10.0) && inside(&b.path, 50.0, 90.0));
}

#[test]
fn the_shortest_crossing_takes_the_thinnest_bar() {
    // The left bar is 5 pt thick, the others 20.
    let art = ring((0.0, 0.0, 100.0, 100.0), (5.0, 20.0, 80.0, 80.0));
    let b = run(&art, spec(4.0, 1, BridgeDirection::Shortest));
    assert_eq!((b.bridges, islands_left(&b)), (1, 0));
    assert!(!inside(&b.path, 2.5, 50.0), "the bridge crosses the thin bar");
    assert!(inside(&b.path, 50.0, 10.0) && inside(&b.path, 90.0, 50.0));
    // Vertical bridges cross a 20 pt bar instead.
    let v = run(&art, spec(4.0, 1, BridgeDirection::Vertical));
    assert!(inside(&v.path, 2.5, 50.0) && !inside(&v.path, 42.5, 10.0));
}

#[test]
fn a_shape_with_two_holes_gets_a_bridge_for_each() {
    let art = PathData::new(vec![rect(0.0, 0.0, 100.0, 50.0), rect(10.0, 10.0, 40.0, 40.0), rect(60.0, 10.0, 90.0, 40.0)]);
    let b = run(&art, spec(3.0, 1, BridgeDirection::Vertical));
    assert_eq!((b.islands, b.bridges, b.unbridged), (2, 2, 0));
    assert_eq!(islands_left(&b), 0);
    assert!(!inside(&b.path, 25.0, 5.0) && !inside(&b.path, 75.0, 5.0));
}

#[test]
fn more_bridges_per_island_spread_around_it() {
    let b = run(&letter_o(), spec(4.0, 2, BridgeDirection::Vertical));
    assert_eq!(b.bridges, 2);
    assert!(!inside(&b.path, 50.0, 10.0) && !inside(&b.path, 50.0, 90.0), "one at the top, one at the bottom");
}

#[test]
fn a_short_crossing_too_narrow_for_a_bridge_does_not_limit_the_others() {
    // Hole A has a 2 pt slot B just above it: the 5 pt crossing from A to B is the shortest, but
    // no 4 pt bridge fits there. A's 20 pt crossings to the sheet take its two bridges.
    let art = PathData::new(vec![rect(0.0, 0.0, 200.0, 100.0), rect(20.0, 20.0, 80.0, 80.0), rect(49.0, 5.0, 51.0, 15.0)]);
    let b = run(&art, spec(4.0, 2, BridgeDirection::Vertical));
    assert_eq!((b.islands, b.bridges, b.unbridged), (2, 2, 1));
}

#[test]
fn islands_that_only_reach_each_other_get_no_bridge() {
    // Holes Y and X, one above the other, with combs of 2 pt slots above Y and below X: no 4 pt
    // vertical bridge reaches the sheet from either, so the bridge that fits between them would
    // join nothing to the sheet.
    let mut subpaths = vec![rect(0.0, 0.0, 100.0, 100.0), rect(20.0, 20.0, 80.0, 30.0), rect(30.0, 40.0, 70.0, 60.0)];
    for k in 0..27 {
        let x = 10.0 + 3.0 * k as f64;
        subpaths.push(rect(x, 5.0, x + 2.0, 15.0));
        subpaths.push(rect(x, 85.0, x + 2.0, 95.0));
    }
    let b = run(&PathData::new(subpaths), spec(4.0, 1, BridgeDirection::Vertical));
    assert_eq!((b.islands, b.bridges, b.unbridged), (56, 0, 56));
}

#[test]
fn an_island_inside_an_island_is_joined_through_it() {
    // A ring in the hole of a ring: two islands, the gap between the rings and the inner hole.
    let art =
        PathData::new(vec![rect(0.0, 0.0, 100.0, 100.0), rect(10.0, 10.0, 90.0, 90.0), rect(30.0, 30.0, 70.0, 70.0), rect(40.0, 40.0, 60.0, 60.0)]);
    let b = run(&art, spec(2.0, 1, BridgeDirection::Vertical));
    assert_eq!((b.islands, b.bridges, b.unbridged), (2, 2, 0));
    assert_eq!(islands_left(&b), 0);
}

#[test]
fn an_island_too_narrow_for_the_width_is_counted() {
    let art = ring((0.0, 0.0, 100.0, 100.0), (49.0, 20.0, 51.0, 80.0));
    let b = run(&art, spec(4.0, 1, BridgeDirection::Vertical));
    assert_eq!((b.islands, b.bridges, b.unbridged), (1, 0, 1));
    assert_eq!(islands_left(&b), 1);
}

#[test]
fn art_without_islands_is_left_whole() {
    let art = PathData::new(vec![rect(0.0, 0.0, 50.0, 50.0), rect(100.0, 0.0, 150.0, 50.0)]);
    let b = run(&art, spec(4.0, 1, BridgeDirection::Shortest));
    assert_eq!((b.islands, b.bridges, b.unbridged), (0, 0, 0));
    assert!(inside(&b.path, 25.0, 25.0) && inside(&b.path, 125.0, 25.0));
}

#[test]
fn the_shortest_crossing_may_run_at_an_angle() {
    // A diamond ring: its walls are 20 pt across horizontally and vertically but about 14 pt
    // across at 45°, so only a diagonal line finds the shortest crossing.
    let diamond = |r: f64| {
        let pts = [(100.0, 100.0 - r), (100.0 + r, 100.0), (100.0, 100.0 + r), (100.0 - r, 100.0)].map(|(x, y)| Point::new(x, y));
        SubPath::polyline(&pts, true)
    };
    let art = PathData::new(vec![diamond(60.0), diamond(40.0)]);
    let b = run(&art, spec(2.0, 1, BridgeDirection::Shortest));
    assert_eq!((b.bridges, islands_left(&b)), (1, 0));
    // The middles of the four diagonal walls: one is cut. The middles of the walls straight
    // above, below and beside the hole all stay.
    let diagonal = [(125.0, 125.0), (75.0, 75.0), (125.0, 75.0), (75.0, 125.0)];
    assert!(diagonal.iter().any(|&(x, y)| !inside(&b.path, x, y)), "a diagonal bridge");
    for (x, y) in [(150.0, 100.0), (50.0, 100.0), (100.0, 150.0), (100.0, 50.0)] {
        assert!(inside(&b.path, x, y), "({x}, {y}) stays");
    }
    let h = run(&art, spec(2.0, 1, BridgeDirection::Horizontal));
    assert!(diagonal.iter().all(|&(x, y)| inside(&h.path, x, y)));
}

#[test]
fn a_hole_touching_another_outline_at_a_corner_is_still_an_island() {
    // A diamond hole whose bottom corner, where its outline starts, touches the bottom edge of the
    // square around it.
    let hole = SubPath::polyline(&[(50.0, 100.0), (70.0, 70.0), (50.0, 40.0), (30.0, 70.0)].map(|(x, y)| Point::new(x, y)), true);
    let region = PathData::new(vec![rect(0.0, 0.0, 100.0, 100.0), hole]);
    let found = contours(&region, 1.0).unwrap();
    assert_eq!(found.iter().filter(|c| c.hole).count(), 1);
}

#[test]
fn a_sliver_too_small_to_see_counts_as_art() {
    // A hole 10 pt long and 0.005 pt high between the island and the top edge: a bridge that
    // reaches only the sliver would join nothing to the sheet.
    let region = PathData::new(vec![rect(0.0, 0.0, 100.0, 100.0), rect(40.0, 40.0, 60.0, 60.0), rect(45.0, 20.0, 55.0, 20.005)]);
    let b = bridge_region(region, spec(5.0, 1, BridgeDirection::Vertical)).unwrap();
    assert_eq!((b.islands, b.bridges, b.unbridged), (1, 1, 0));
    assert_eq!(islands_left(&b), 0);
}

#[test]
fn extra_bridges_stay_two_widths_apart() {
    // Sixteen 10 pt bridges have no room around a hole 60 pt across with 20 pt between them.
    let width = 10.0;
    let found = contours(&letter_o(), width).unwrap();
    let (bridges, unjoined) = choose(&found, spec(width, 16, BridgeDirection::Shortest), MAX_WORK).unwrap();
    assert!(bridges.len() > 4 && bridges.len() < 16, "{}", bridges.len());
    assert_eq!(unjoined, 0);
    for (i, a) in bridges.iter().enumerate() {
        for b in &bridges[i + 1..] {
            assert!(a.middle.distance(b.middle) >= width * 2.0, "{:?} {:?}", a.middle, b.middle);
        }
    }
}

#[test]
fn bridges_found_before_the_budget_runs_out_are_kept() {
    let found = contours(&letter_o(), 4.0).unwrap();
    // The least work that joins the island with one bridge also gives one with up to 16 asked
    // for: the extra bridges stop where the budget ends.
    let least = (1..100_000).find(|&work| choose(&found, spec(4.0, 1, BridgeDirection::Vertical), work).is_ok()).unwrap();
    let (bridges, unjoined) = choose(&found, spec(4.0, 16, BridgeDirection::Vertical), least).unwrap();
    assert_eq!((bridges.len(), unjoined), (1, 0));
    assert!(matches!(choose(&found, spec(4.0, 1, BridgeDirection::Vertical), least - 1), Err(PathOpsError::TooComplex)));
}

#[test]
fn joined_art_is_kept_when_lines_in_one_direction_run_out() {
    // A ring whose thin left bar has edges of many short zigzag segments: lines across it in the
    // horizontal direction look at many more segments than vertical lines across the top bar.
    let zigzag = |x: f64, from: f64, to: f64, out: &mut Vec<Point>| {
        let n = ((to - from).abs() / 0.05) as usize;
        out.extend((1..n).map(|i| Point::new(x + if i % 2 == 1 { 0.02 } else { 0.0 }, from + (to - from) * i as f64 / n as f64)));
    };
    let mut outer = vec![Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(200.0, 100.0), Point::new(0.0, 100.0)];
    zigzag(0.0, 100.0, 0.0, &mut outer);
    let mut hole = vec![Point::new(10.0, 20.0), Point::new(170.0, 20.0), Point::new(170.0, 80.0), Point::new(10.0, 80.0)];
    zigzag(9.98, 80.0, 20.0, &mut hole);
    let art = PathData::new(vec![SubPath::polyline(&outer, true), SubPath::polyline(&hole, true)]);
    let found = contours(&art, 4.0).unwrap();
    let shortest = spec(4.0, 1, BridgeDirection::Shortest);
    let views = (0..180).step_by(SHORTEST_STEP_DEG).count();
    // A share of the work that runs out in the horizontal direction while another direction joins
    // the island.
    let share = (1..5000)
        .find(|&budget| {
            let all: Vec<View> =
                (0..180).step_by(SHORTEST_STEP_DEG).map(|d| View::new(&found, (d as f64).to_radians(), 4.0, budget).unwrap()).collect();
            let mut picker = Picker::new(&found, &all, 4.0);
            picker.connect();
            all.iter().any(View::exhausted) && picker.unjoined() == 0
        })
        .expect("a share of the work that runs out with the island joined");
    let (bridges, unjoined) = choose(&found, shortest, share * views).unwrap();
    assert_eq!((bridges.len(), unjoined), (1, 0));
}

#[test]
fn curves_are_followed_within_a_twentieth_of_the_width() {
    let circle = PathData::from_bezpath(&kurbo::Circle::new((0.0, 0.0), 100.0).to_path(0.001));
    let mut counts = vec![];
    for (width, tolerance) in [(0.01, 0.002), (1.0, 0.05), (20.0, 0.25)] {
        let found = contours(&circle, width).unwrap();
        let points = &found[0].points;
        // The sag of each chord below the circle.
        let sag = (0..points.len()).map(|i| 100.0 - points[i].midpoint(points[(i + 1) % points.len()]).to_vec2().hypot()).fold(0.0, f64::max);
        assert!(sag <= tolerance + 0.002, "{width}: {sag}");
        counts.push(points.len());
    }
    assert!(counts[0] > counts[1] && counts[1] > counts[2], "{counts:?}");
}

#[test]
fn open_outlines_are_closed_as_a_fill_closes_them() {
    let mut art = letter_o();
    art.subpaths[0].closed = false;
    let b = run(&art, spec(4.0, 1, BridgeDirection::Vertical));
    assert_eq!((b.islands, b.bridges, b.unbridged), (1, 1, 0));
}

#[test]
fn art_too_large_is_refused_before_it_is_combined() {
    let far = PathData::new(vec![rect(0.0, 0.0, 1e8, 1e8)]);
    let points: Vec<Point> = (0..=MAX_STENCIL_SEGMENTS).map(|i| Point::new(i as f64 * 0.01, (i % 2) as f64)).collect();
    let dense = PathData::new(vec![SubPath::polyline(&points, true)]);
    assert!(too_large(&[(&far, FillRule::EvenOdd)]) && too_large(&[(&dense, FillRule::EvenOdd)]));
    assert!(!too_large(&[(&letter_o(), FillRule::EvenOdd), (&letter_o(), FillRule::NonZero)]));
}

#[test]
fn outlines_nested_too_deep_to_place_are_refused() {
    // Rings of 190-sided polygons, one inside the next: placing each inside the others takes
    // about 95 million tests of a point against an edge with 1000 of them, and 8 million with 300.
    let polygon = |r: f64| {
        let pts: Vec<Point> = (0..190).map(|i| (i as f64 / 190.0 * std::f64::consts::TAU).sin_cos()).map(|(s, c)| Point::new(c * r, s * r)).collect();
        SubPath::polyline(&pts, true)
    };
    let nested = |n: usize| PathData::new((0..n).map(|i| polygon(10.0 + i as f64)).collect());
    assert!(contours(&nested(300), 1.0).is_ok());
    assert!(matches!(contours(&nested(1000), 1.0), Err(PathOpsError::TooComplex)));
}

#[test]
fn a_curved_o_is_bridged_at_its_thinnest() {
    // Two circles, the hole 8 pt above the middle: the O is 2 pt thick at the top and 18 pt at
    // the bottom.
    let circle = |cy: f64, r: f64| PathData::from_bezpath(&kurbo::Circle::new((100.0, cy), r).to_path(0.01));
    let mut art = circle(100.0, 60.0);
    art.subpaths.extend(circle(92.0, 50.0).subpaths);
    let b = run(&art, spec(1.0, 1, BridgeDirection::Vertical));
    assert_eq!((b.islands, b.bridges, islands_left(&b)), (1, 1, 0));
    assert!(!inside(&b.path, 100.0, 41.0), "it crosses the thin top");
    assert!(inside(&b.path, 100.0, 151.0), "the thick bottom stays");
}

#[test]
fn art_with_too_many_outlines_is_refused() {
    let squares = (0..MAX_STENCIL_CONTOURS + 1)
        .map(|i| {
            let (x, y) = ((i % 100) as f64 * 3.0, (i / 100) as f64 * 3.0);
            rect(x, y, x + 1.0, y + 1.0)
        })
        .collect();
    let region = PathData::new(squares);
    assert!(matches!(contours(&region, 1.0), Err(PathOpsError::TooComplex)));
    let mut fewer = region.clone();
    fewer.subpaths.pop();
    assert_eq!(contours(&fewer, 1.0).unwrap().len(), MAX_STENCIL_CONTOURS);
}

#[test]
fn art_as_large_as_a_document_holds_is_followed_and_larger_art_is_refused() {
    // An O 8000000 pt across, the most a document holds, with a 60 pt hole near its edge, followed
    // within 0.002 pt.
    let circle = |c: (f64, f64), r: f64| PathData::from_bezpath(&kurbo::Circle::new(c, r).to_path(0.01));
    let mut art = circle((0.0, 0.0), 4e6);
    art.subpaths.extend(circle((0.0, 0.0), 2e6).subpaths);
    art.subpaths.extend(circle((4e6 - 100.0, 0.0), 30.0).subpaths);
    let found = contours(&art, 0.01).unwrap();
    assert_eq!(found.iter().filter(|c| c.hole).count(), 2);
    let far = PathData::new(vec![rect(0.0, 0.0, 1e8, 1e8)]);
    assert!(matches!(contours(&far, 0.01), Err(PathOpsError::TooComplex)));
    let r = stencil_bridges(&[(&far, FillRule::EvenOdd)], spec(4.0, 1, BridgeDirection::Vertical));
    assert!(matches!(r, Err(PathOpsError::TooComplex)));
}

#[test]
fn art_with_too_many_anchors_is_refused() {
    let points: Vec<Point> = (0..=MAX_STENCIL_SEGMENTS).map(|i| Point::new(i as f64 * 0.01, (i % 2) as f64)).collect();
    let art = PathData::new(vec![SubPath::polyline(&points, true)]);
    let r = stencil_bridges(&[(&art, FillRule::EvenOdd)], spec(4.0, 1, BridgeDirection::Vertical));
    assert!(matches!(r, Err(PathOpsError::TooComplex)));
}

#[test]
fn following_stops_past_the_limit_and_allows_the_closing_point() {
    // A closed square: its four corners and the start again.
    let square = rect(0.0, 0.0, 10.0, 10.0);
    let mut bp = BezPath::new();
    square.to_bezpath_into(&mut bp);
    assert_eq!(follow(&bp, 0.01, 4).unwrap().len(), 5);
    assert!(matches!(follow(&bp, 0.01, 3), Err(PathOpsError::TooComplex)));
}

#[test]
fn bad_widths_and_counts_are_refused() {
    let art = letter_o();
    for s in [spec(0.0, 1, BridgeDirection::Vertical), spec(f64::NAN, 1, BridgeDirection::Vertical), spec(4.0, 0, BridgeDirection::Vertical)] {
        assert!(stencil_bridges(&[(&art, FillRule::EvenOdd)], s).is_err(), "{s:?}");
    }
}
