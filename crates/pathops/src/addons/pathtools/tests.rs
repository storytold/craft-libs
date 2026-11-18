use std::collections::BTreeSet;

use craft_geom::{Anchor, AnchorKind, PathData, SubPath};
use kurbo::{ParamCurve, ParamCurveArclen, Point, Vec2};

use super::*;
use crate::addons::simplify_anchors_keep_shape;

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn line(pts: &[(f64, f64)], closed: bool) -> SubPath {
    SubPath::polyline(&pts.iter().map(|&(x, y)| pt(x, y)).collect::<Vec<_>>(), closed)
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn length(sp: &SubPath) -> f64 {
    (0..sp.segment_count()).map(|i| sp.segment(i).arclen(1e-6)).sum()
}

/// The farthest any sampled point of `sp` lies off the circle about `c` of radius `r`.
fn off_circle(sp: &SubPath, c: Point, r: f64) -> f64 {
    (0..sp.segment_count())
        .flat_map(|i| (0..=16).map(move |k| (i, k as f64 / 16.0)))
        .map(|(i, t)| (sp.segment(i).eval(t).distance(c) - r).abs())
        .fold(0.0, f64::max)
}

#[test]
fn auto_smooth_turns_a_square_into_a_circle() {
    let mut sq = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
    for i in 0..4 {
        assert!(auto_smooth(&mut sq, i, SMOOTH_RATIO));
    }
    let a = sq.anchors[1];
    assert_eq!(a.kind, AnchorKind::Smooth);
    // Each handle is the ratio × the side long, at right angles to the corner's bisector.
    assert!(near((a.h_out - a.p).hypot(), 39.05, 1e-9) && near((a.h_in - a.p).hypot(), 39.05, 1e-9));
    assert!(near((a.h_out - a.p).dot(Vec2::new(-1.0, 1.0)), 0.0, 1e-9));
    // The four corners make a circle through them, to within its cubic approximation.
    assert!(off_circle(&sq, pt(50.0, 50.0), 50f64.hypot(50.0)) < 0.2, "{}", off_circle(&sq, pt(50.0, 50.0), 50f64.hypot(50.0)));
    // A smooth anchor stays as it is.
    assert!(!auto_smooth(&mut sq, 1, SMOOTH_RATIO));
}

#[test]
fn auto_smooth_mirrors_a_lone_handle_and_aims_open_ends() {
    let mut sp = SubPath::new(
        vec![Anchor::corner(pt(0.0, 0.0)), Anchor::with_handles(pt(50.0, 0.0), pt(40.0, -10.0), pt(50.0, 0.0)), Anchor::corner(pt(100.0, 0.0))],
        false,
    );
    assert!(auto_smooth(&mut sp, 1, SMOOTH_RATIO));
    assert_eq!(sp.anchors[1].h_out, pt(60.0, 10.0));
    // An open end with no handle aims at its neighbour's facing handle.
    assert!(auto_smooth(&mut sp, 0, SMOOTH_RATIO));
    let h = sp.anchors[0].h_out;
    assert!(near(h.distance(pt(0.0, 0.0)), 50.0 * SMOOTH_RATIO, 1e-9));
    assert!(near((h - pt(0.0, 0.0)).cross(pt(40.0, -10.0) - pt(0.0, 0.0)), 0.0, 1e-9), "{h:?}");
    // Two handles out of line are brought into line, keeping their lengths.
    let mut bent = SubPath::new(
        vec![Anchor::corner(pt(0.0, 0.0)), Anchor::with_handles(pt(50.0, 0.0), pt(40.0, 0.0), pt(50.0, 20.0)), Anchor::corner(pt(100.0, 0.0))],
        false,
    );
    assert!(auto_smooth(&mut bent, 1, SMOOTH_RATIO));
    let a = bent.anchors[1];
    assert!(
        near((a.h_in - a.p).cross(a.h_out - a.p), 0.0, 1e-9) && near((a.h_out - a.p).hypot(), 20.0, 1e-9) && near((a.h_in - a.p).hypot(), 10.0, 1e-9)
    );
}

#[test]
fn equalise_averages_lengths_and_makes_a_missing_handle() {
    let mut sp = SubPath::new(
        vec![Anchor::corner(pt(0.0, 0.0)), Anchor::with_handles(pt(50.0, 0.0), pt(40.0, 0.0), pt(80.0, 0.0)), Anchor::corner(pt(100.0, 0.0))],
        false,
    );
    assert!(equalize_handles(&mut sp, 1));
    assert_eq!((sp.anchors[1].h_in, sp.anchors[1].h_out), (pt(30.0, 0.0), pt(70.0, 0.0)));
    assert!(!equalize_handles(&mut sp, 1), "already equal");
    let mut one = SubPath::new(
        vec![Anchor::corner(pt(0.0, 0.0)), Anchor::with_handles(pt(50.0, 0.0), pt(50.0, 0.0), pt(50.0, 20.0)), Anchor::corner(pt(100.0, 0.0))],
        false,
    );
    assert!(equalize_handles(&mut one, 1));
    assert_eq!(one.anchors[1].h_in, pt(50.0, -20.0));
    assert!(!equalize_handles(&mut one, 0), "no handles to equalise");
}

#[test]
fn make_corner_retracts_or_keeps_handles() {
    let a = Anchor::smooth(pt(50.0, 0.0), pt(60.0, 0.0));
    let mut sp = SubPath::new(vec![Anchor::corner(pt(0.0, 0.0)), a, Anchor::corner(pt(100.0, 0.0))], false);
    assert!(make_corner(&mut sp, 1, false));
    assert_eq!((sp.anchors[1].kind, sp.anchors[1].h_out), (AnchorKind::Corner, pt(60.0, 0.0)));
    assert!(make_corner(&mut sp, 1, true));
    assert!(!sp.anchors[1].has_in() && !sp.anchors[1].has_out());
}

#[test]
fn add_points_splits_segments_into_equal_lengths() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 40.0)], false);
    let (out, added) = add_points(&sp, Some(&BTreeSet::from([0])), 3);
    let xs: Vec<f64> = out.anchors.iter().map(|a| a.p.x).collect();
    for (x, want) in xs.iter().zip([0.0, 25.0, 50.0, 75.0, 100.0, 100.0]) {
        assert!(near(*x, want, 1e-4), "{xs:?}");
    }
    assert_eq!((xs.len(), added), (6, BTreeSet::from([1, 2, 3])));
    // On a curve the pieces have the same length and the shape stays.
    let mut sq = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
    for i in 0..4 {
        auto_smooth(&mut sq, i, SMOOTH_RATIO);
    }
    let (out, added) = add_points(&sq, None, 2);
    assert_eq!((out.anchors.len(), added.len()), (12, 8));
    let lens: Vec<f64> = (0..3).map(|i| out.segment(i).arclen(1e-6)).collect();
    assert!(near(lens[0], lens[1], 1e-3) && near(lens[1], lens[2], 1e-3), "{lens:?}");
    assert!(near(length(&out), length(&sq), 1e-4));
}

#[test]
fn close_joins_ends_three_ways() {
    let open = SubPath::new(
        vec![Anchor::with_handles(pt(0.0, 0.0), pt(-10.0, 10.0), pt(10.0, -10.0)), Anchor::corner(pt(100.0, 0.0)), Anchor::corner(pt(50.0, 80.0))],
        false,
    );
    let mut kept = open.clone();
    assert!(close(&mut kept, CloseMode::Handles, SMOOTH_RATIO));
    assert!(kept.closed && kept.anchors[0].h_in == pt(-10.0, 10.0));
    let mut straight = open.clone();
    assert!(close(&mut straight, CloseMode::Straight, SMOOTH_RATIO));
    assert!(straight.closed && straight.segment_is_line(2));
    let mut smooth = open.clone();
    assert!(close(&mut smooth, CloseMode::Smooth, SMOOTH_RATIO));
    assert!(smooth.anchors[2].has_in() && smooth.anchors[2].has_out(), "the last anchor is made smooth");
    assert!(!close(&mut smooth, CloseMode::Smooth, SMOOTH_RATIO), "closed already");
    assert_eq!(CloseMode::parse("straight"), Some(CloseMode::Straight));
}

#[test]
fn extend_straight_lengthens_and_shortens_either_end() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0)], false);
    let end = extend_open(&sp, false, 50.0, ExtendMode::Straight).unwrap();
    assert_eq!(end.anchors.last().unwrap().p, pt(150.0, 0.0));
    let start = extend_open(&sp, true, 20.0, ExtendMode::Straight).unwrap();
    assert_eq!(start.anchors.first().unwrap().p, pt(-20.0, 0.0));
    let short = extend_open(&sp, false, -30.0, ExtendMode::Straight).unwrap();
    assert_eq!(short.anchors.len(), 2);
    assert!(near(short.anchors[1].p.x, 70.0, 1e-6));
    // Shortening past an anchor drops it.
    let bent = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 10.0)], false);
    let cut = extend_open(&bent, false, -40.0, ExtendMode::Straight).unwrap();
    assert_eq!(cut.anchors.len(), 2);
    assert!(near(cut.anchors[1].p.x, 70.0, 1e-6));
    // Nothing is left of a path shortened by its whole length; closed paths have no ends.
    assert!(extend_open(&sp, false, -100.0, ExtendMode::Straight).is_none());
    assert!(extend_open(&line(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)], true), false, 5.0, ExtendMode::Straight).is_none());
    assert!(extend_open(&sp, false, f64::NAN, ExtendMode::Arc).is_none());
}

/// A quarter of the circle of radius 100 about the origin, from (100, 0) to (0, 100).
fn quarter() -> SubPath {
    let k = 4.0 / 3.0 * (std::f64::consts::FRAC_PI_8).tan() * 100.0;
    SubPath::new(
        vec![Anchor::with_handles(pt(100.0, 0.0), pt(100.0, 0.0), pt(100.0, k)), Anchor::with_handles(pt(0.0, 100.0), pt(k, 100.0), pt(0.0, 100.0))],
        false,
    )
}

#[test]
fn extend_arc_follows_the_circle_the_end_curves_along() {
    let sp = quarter();
    let len = std::f64::consts::FRAC_PI_2 * 100.0;
    let out = extend_open(&sp, false, len, ExtendMode::Arc).unwrap();
    // The circle matches the curvature the (cubic) quarter ends with, within a percent or so of
    // the true circle's.
    assert!(off_circle(&out, Point::ORIGIN, 100.0) < 3.0, "{}", off_circle(&out, Point::ORIGIN, 100.0));
    let end = out.anchors.last().unwrap().p;
    assert!(end.distance(pt(-100.0, 0.0)) < 5.0, "{end:?}");
    assert!(near(length(&out), 2.0 * len, 0.2));
    // At most half way round.
    let long = extend_open(&sp, false, 10.0 * len, ExtendMode::Arc).unwrap();
    assert!(near(length(&long) - len, std::f64::consts::PI * 100.0, 6.0), "{}", length(&long) - len);
    // A straight end carries on straight.
    let straight = extend_open(&line(&[(0.0, 0.0), (10.0, 0.0)], false), false, 5.0, ExtendMode::Arc).unwrap();
    assert_eq!(straight.anchors.last().unwrap().p, pt(15.0, 0.0));
}

#[test]
fn extend_bezier_stretches_the_last_curve_without_new_anchors() {
    let sp = quarter();
    let before = length(&sp);
    let out = extend_open(&sp, false, 30.0, ExtendMode::Bezier).unwrap();
    assert_eq!(out.anchors.len(), 2);
    assert!(near(length(&out), before + 30.0, 1e-3), "{}", length(&out));
    let p = out.segment(0).eval(0.5);
    assert!((p.to_vec2().hypot() - 100.0).abs() < 15.0);
}

#[test]
fn the_length_a_drag_asks_for() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0)], false);
    assert!(near(extend_length_to(&sp, false, pt(130.0, 10.0), ExtendMode::Straight).unwrap(), 30.0, 1e-6));
    assert!(near(extend_length_to(&sp, false, pt(40.0, 5.0), ExtendMode::Straight).unwrap(), -60.0, 1e-6));
    assert!(near(extend_length_to(&sp, true, pt(-25.0, 3.0), ExtendMode::Straight).unwrap(), 25.0, 1e-6));
    // Never the whole path.
    assert!(extend_length_to(&sp, false, pt(-50.0, 0.0), ExtendMode::Straight).unwrap() > -100.0);
    let arc = extend_length_to(&quarter(), false, pt(-100.0, 0.0), ExtendMode::Arc).unwrap();
    assert!(near(arc, std::f64::consts::FRAC_PI_2 * 100.0, 3.0), "{arc}");
    assert!(extend_length_to(&sp, false, pt(f64::NAN, 0.0), ExtendMode::Straight).is_none());
}

#[test]
fn crossings_are_found_on_both_paths() {
    let a = PathData::single(line(&[(0.0, 0.0), (100.0, 100.0)], false));
    let b = PathData::single(line(&[(0.0, 100.0), (100.0, 0.0)], false));
    let hits = crossings(&[&a, &b], false).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].len(), 1);
    assert!(near(hits[0][0].2, 0.5, 1e-6) && near(hits[1][0].2, 0.5, 1e-6));
    // A circle crossed by a line: twice.
    let mut circle = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
    for i in 0..4 {
        auto_smooth(&mut circle, i, SMOOTH_RATIO);
    }
    let across = PathData::single(line(&[(-40.0, 50.0), (140.0, 50.0)], false));
    let circle = PathData::single(circle);
    let hits = crossings(&[&circle, &across], false).unwrap();
    assert_eq!((hits[0].len(), hits[1].len()), (2, 2), "{hits:?}");
    // A path doesn't cross itself where its segments meet, and only crosses itself when asked.
    let bow = PathData::single(line(&[(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)], false));
    assert!(crossings(&[&bow], false).unwrap()[0].is_empty());
    assert_eq!(crossings(&[&bow], true).unwrap()[0].len(), 2, "one crossing, on two of its segments");
    // Two identical paths lying on each other don't tangle the search.
    let sq = PathData::single(circle.subpaths[0].clone());
    assert!(crossings(&[&sq, &circle], false).is_some());
    // A path ending on another meets it there: at its end anchor, and part way along the other.
    let t = PathData::single(line(&[(50.0, 50.0), (50.0, 150.0)], false));
    let hits = crossings(&[&b, &t], false).unwrap();
    assert_eq!((hits[0].len(), hits[1].len()), (1, 1), "{hits:?}");
    assert!(near(hits[1][0].2, 0.0, 1e-6) && near(b.subpaths[0].segment(0).eval(hits[0][0].2).x, 50.0, 1e-6));
}

#[test]
fn a_shallow_crossing_is_placed_where_the_curves_meet() {
    // Lines square to the middle of a quarter circle of radius 100, just inside it: each crosses the
    // arc twice, at under a degree for the nearest one.
    let arc = PathData::single(quarter());
    let k = std::f64::consts::FRAC_1_SQRT_2;
    for gap in [0.5, 0.05, 0.005, 0.0005] {
        let mid = pt((100.0 - gap) * k, (100.0 - gap) * k);
        let (from, to) = (mid + Vec2::new(-k, k) * 60.0, mid + Vec2::new(k, -k) * 60.0);
        let across = PathData::single(line(&[(from.x, from.y), (to.x, to.y)], false));
        let exact: Vec<Point> = kurbo::PathSeg::Cubic(arc.subpaths[0].segment(0))
            .intersect_line(kurbo::Line::new(from, to))
            .iter()
            .map(|x| from.lerp(to, x.line_t))
            .collect();
        assert_eq!(exact.len(), 2, "{gap}");
        let on = |p: &PathData, other: &PathData| -> Vec<Point> {
            crossings_on(p, &[other]).unwrap().iter().map(|q| p.subpaths[0].segment(q.1).eval(q.2)).collect()
        };
        let both = crossings(&[&arc, &across], false).unwrap();
        let found = [
            on(&arc, &across),
            on(&across, &arc),
            both[0].iter().map(|q| arc.subpaths[0].segment(q.1).eval(q.2)).collect(),
            both[1].iter().map(|q| across.subpaths[0].segment(q.1).eval(q.2)).collect(),
        ];
        for places in found {
            assert_eq!(places.len(), 2, "{gap}: {places:?}");
            for x in &exact {
                let off = places.iter().map(|p| p.distance(*x)).fold(f64::INFINITY, f64::min);
                assert!(off < 1e-6, "{gap}: {off} pt from {x:?}");
            }
        }
    }
}

#[test]
fn insert_at_keeps_the_shape_and_names_the_new_anchors() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false);
    let (out, marks) = insert_at(&sp, &[(1, 0.5), (0, 0.25), (0, 0.75), (1, 1.0), (7, 0.5)]);
    let pts: Vec<Point> = out.anchors.iter().map(|a| a.p).collect();
    let c = sp.segment(0);
    let want = [pt(0.0, 0.0), c.eval(0.25), c.eval(0.75), pt(100.0, 0.0), pt(100.0, 50.0), pt(100.0, 100.0)];
    assert_eq!(pts.len(), want.len());
    assert!(pts.iter().zip(want).all(|(p, w)| p.distance(w) < 1e-9), "{pts:?}");
    assert_eq!(marks, BTreeSet::from([1, 2, 4, 5]));
    // The closing segment of a closed path.
    let tri = line(&[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)], true);
    let (out, marks) = insert_at(&tri, &[(2, 0.5)]);
    assert_eq!((out.anchors.len(), out.anchors[3].p, marks), (4, pt(0.0, 50.0), BTreeSet::from([3])));
}

#[test]
fn inspect_reads_lengths_area_angles_and_curvature() {
    let rect = PathData::single(line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)], true));
    let i = inspect(&rect, pt(50.0, 1.0), 3.0).unwrap();
    assert_eq!((i.closed, i.anchors, i.segment), (true, 4, 0));
    assert!(near(i.length, 300.0, 1e-6) && near(i.area, 5000.0, 1e-6) && near(i.segment_length, 100.0, 1e-6));
    assert_eq!((i.radius, i.anchor.is_none()), (None, true));
    let corner = inspect(&rect, pt(99.0, 1.0), 3.0).unwrap().anchor.unwrap();
    assert_eq!(corner.index, 1);
    assert!(near(corner.angle.unwrap(), 90.0, 1e-9));
    let r = inspect(&PathData::single(quarter()), pt(71.0, 71.0), 1.0).unwrap();
    assert!(near(r.radius.unwrap(), 100.0, 1.0), "{:?}", r.radius);
    assert!(near(r.tangent, -135.0, 1.0), "{}", r.tangent);
    let open = inspect(&PathData::single(quarter()), pt(100.0, 0.0), 1.0).unwrap().anchor.unwrap();
    assert_eq!(open.angle, None, "an open end has no angle");
    assert!(inspect(&PathData::default(), pt(0.0, 0.0), 1.0).is_none());
    // Handle readouts: length and direction counter-clockwise on the page.
    assert_eq!(handle_polar(pt(0.0, 0.0), pt(0.0, -10.0)), Some((10.0, 90.0)));
    assert!(handle_at(pt(0.0, 0.0), 10.0, 90.0).distance(pt(0.0, -10.0)) < 1e-9);
}

#[test]
fn the_brush_kernel_removes_only_marked_anchors() {
    let pts: Vec<(f64, f64)> = (0..=10).map(|i| (i as f64 * 10.0, 0.0)).collect();
    let sp = line(&pts, false);
    let marked: Vec<bool> = (0..=10).map(|i| (3..=5).contains(&i)).collect();
    let out = simplify_anchors_keep_shape(&sp, 0.1, &marked);
    let xs: Vec<f64> = out.anchors.iter().map(|a| a.p.x).collect();
    assert_eq!(xs, [0.0, 10.0, 20.0, 60.0, 70.0, 80.0, 90.0, 100.0]);
    // Off the line, a removal within tolerance only.
    let bump = line(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 5.0), (40.0, 0.0)], false);
    assert_eq!(simplify_anchors_keep_shape(&bump, 0.1, &[true; 5]).anchors.len(), 4, "only the anchor on the straight run goes");
    assert_eq!(simplify_anchors_keep_shape(&bump, f64::INFINITY, &[true; 5]).anchors.len(), 2, "unless tolerance is ignored");
}

/// A circle about (0, 0) of radius 100 with `n` smooth anchors, each arc a cubic.
fn circle(n: usize) -> SubPath {
    let step = std::f64::consts::TAU / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan() * 100.0;
    let anchors = (0..n).map(|i| {
        let a = i as f64 * step;
        let (p, tan) = (pt(a.cos() * 100.0, a.sin() * 100.0), Vec2::new(-a.sin(), a.cos()));
        Anchor { p, h_in: p - tan * k, h_out: p + tan * k, kind: AnchorKind::Smooth }
    });
    SubPath::new(anchors.collect(), true)
}

#[test]
fn a_slid_anchor_keeps_the_shape_its_count_and_its_index() {
    let sp = circle(8);
    for (index, t) in [(2, 0.5), (2, -0.5), (0, -0.25), (0, 0.25), (7, 0.9), (7, -0.9)] {
        let (out, strayed) = slide_anchor(&sp, index, t).unwrap();
        let seg = if t > 0.0 { sp.segment(index) } else { sp.segment((index + 7) % 8) };
        let want = seg.eval(if t > 0.0 { t } else { 1.0 + t });
        assert_eq!(out.anchors.len(), 8, "{index} {t}");
        assert!(out.anchors[index].p.distance(want) < 1e-9, "{index} {t}: the anchor is where it was sent");
        assert!(strayed < 0.05 && off_circle(&out, pt(0.0, 0.0), 100.0) < 0.05, "{index} {t}: {strayed}");
        // The other anchors stay.
        assert!((0..8).filter(|i| *i != index).all(|i| out.anchors[i].p == sp.anchors[i].p), "{index} {t}");
    }
}

#[test]
fn sliding_along_straight_runs_stays_straight_and_ends_do_not_slide() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (200.0, 100.0)], false);
    let (out, strayed) = slide_anchor(&sp, 1, 0.5).unwrap();
    assert_eq!(out.anchors[1].p, pt(150.0, 0.0));
    assert!(strayed < 1e-9 && out.segment_is_line(0) && out.segment_is_line(1));
    assert_eq!(slide_anchor(&sp, 1, 0.0).unwrap().0, sp, "no move");
    assert_eq!(slide_anchor(&sp, 1, 1e-9).unwrap().0, sp, "too small a move");
    for (i, t) in [(0, 0.5), (3, -0.5), (9, 0.5), (1, 0.99), (1, -1.0), (1, f64::NAN), (1, f64::INFINITY)] {
        assert!(slide_anchor(&sp, i, t).is_none(), "{i} {t}");
    }
    let mut bad = sp.clone();
    bad.anchors[1].h_out = pt(f64::NAN, 0.0);
    assert!(slide_anchor(&bad, 1, 0.5).is_none());
}

#[test]
fn the_slide_a_pointer_asks_for_and_where_the_tangent_stops() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0)], false);
    assert!(near(slide_t_at(&sp, 1, pt(150.0, 30.0)).unwrap(), 0.5, 1e-6));
    let back = slide_t_at(&sp, 1, pt(75.0, -30.0)).unwrap();
    assert!(back < 0.0 && sp.segment(0).eval(1.0 + back).distance(pt(75.0, 0.0)) < 1e-3, "{back}");
    assert!(near(slide_t_at(&sp, 1, pt(900.0, 0.0)).unwrap(), MAX_SLIDE, 1e-9), "held short of the next anchor");
    assert!(slide_t_at(&sp, 0, pt(50.0, 0.0)).is_none());
    assert!(slide_stops(&sp, 1, 0.0).is_empty(), "a straight run has no tangent to reach");
    let slanted = line(&[(0.0, 0.0), (100.0, 100.0), (250.0, 250.0)], false);
    assert!(slide_stops(&slanted, 1, 0.0).is_empty(), "nor one that lies on a multiple of 45°");
    // Round a circle of 4 anchors the tangent is at a multiple of 45° at each anchor and half way.
    let round = circle(4);
    let stops = slide_stops(&round, 0, 0.0);
    assert_eq!(stops.len(), 2, "{stops:?}");
    assert!(near(stops[0], -0.5, 1e-6) && near(stops[1], 0.5, 1e-6), "{stops:?}");
    // From a constrain angle of 22.5°, between those.
    let stops = slide_stops(&round, 0, 22.5);
    assert_eq!(stops.len(), 4, "{stops:?}");
    assert!(stops.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn add_anchors_spaces_them_by_count_or_distance() {
    let sq = line(&[(0.0, 0.0), (90.0, 0.0), (90.0, 90.0), (0.0, 90.0)], true);
    let out = add_anchors(&sq, SegmentKind::All, AnchorSpacing::Count(2), 100, 100).unwrap();
    assert_eq!(out.anchors.len(), 12);
    assert!(near(out.anchors[1].p.x, 30.0, 1e-6) && near(out.anchors[2].p.x, 60.0, 1e-6));
    assert!(out.closed);
    let out = add_anchors(&sq, SegmentKind::All, AnchorSpacing::Distance(40.0), 100, 100).unwrap();
    assert_eq!(out.anchors.len(), 12, "at 40 and 80, none past the segment");
    assert!(near(out.anchors[2].p.x, 80.0, 1e-6));
    assert_eq!(add_anchors(&sq, SegmentKind::All, AnchorSpacing::Distance(1.0), 3, 100).unwrap().anchors.len(), 16, "the cap");
    assert_eq!(add_anchors(&sq, SegmentKind::All, AnchorSpacing::Distance(0.0), 3, 100).unwrap(), sq);
    assert_eq!(add_anchors(&sq, SegmentKind::Curved, AnchorSpacing::Count(1), 3, 100).unwrap(), sq, "no curves");
    assert_eq!(add_anchors(&sq, SegmentKind::All, AnchorSpacing::Count(3), 9, 12).unwrap().anchors.len(), 16, "room for all of them");
    assert!(add_anchors(&sq, SegmentKind::All, AnchorSpacing::Count(3), 9, 11).is_none(), "no room");
    // A curve keeps its shape, and a zero-length segment gets nothing.
    let bent = SubPath::new(
        vec![
            Anchor { h_out: pt(0.0, 50.0), ..Anchor::corner(pt(0.0, 0.0)) },
            Anchor { h_in: pt(100.0, 50.0), ..Anchor::corner(pt(100.0, 0.0)) },
            Anchor::corner(pt(100.0, 0.0)),
        ],
        false,
    );
    let out = add_anchors(&bent, SegmentKind::Curved, AnchorSpacing::Count(3), 9, 9).unwrap();
    assert_eq!(out.anchors.len(), 6);
    assert!(near(length(&out), length(&bent), 1e-4));
    assert_eq!(add_anchors(&bent, SegmentKind::Straight, AnchorSpacing::Count(3), 9, 9).unwrap(), bent);
}

#[test]
fn smoothing_every_anchor_reads_the_same_either_way_along_the_path() {
    let zig = line(&[(0.0, 0.0), (30.0, 40.0), (70.0, 0.0), (100.0, 50.0)], false);
    let mut there = zig.clone();
    smooth_anchors(&mut there, SMOOTH_RATIO);
    let mut back = zig.clone();
    back.reverse();
    smooth_anchors(&mut back, SMOOTH_RATIO);
    back.reverse();
    for (a, b) in there.anchors.iter().zip(&back.anchors) {
        assert!(a.h_in.distance(b.h_in) < 1e-9 && a.h_out.distance(b.h_out) < 1e-9, "{a:?} {b:?}");
    }
    assert!(there.anchors[1].kind == AnchorKind::Smooth && there.anchors[0].has_out() && there.anchors[3].has_in());
    // Nothing to smooth.
    for mut sp in [SubPath::default(), line(&[(1.0, 1.0)], false), line(&[(1.0, 1.0)], true)] {
        let before = sp.clone();
        smooth_anchors(&mut sp, SMOOTH_RATIO);
        assert_eq!(sp, before);
    }
}

#[test]
fn splitting_reads_an_anchors_kind_from_its_handles() {
    // Smooth by its handles though it is called a corner, as a path read back from an outline is.
    let run = SubPath::new(
        vec![
            Anchor::corner(pt(0.0, 0.0)),
            Anchor { h_in: pt(40.0, 0.0), h_out: pt(60.0, 0.0), ..Anchor::corner(pt(50.0, 0.0)) },
            Anchor::corner(pt(100.0, 0.0)),
            Anchor::corner(pt(100.0, 50.0)),
        ],
        false,
    );
    assert_eq!(split_at_anchors(&run, false, true).len(), 2);
    assert_eq!(split_at_anchors(&run, true, false).len(), 2);
    assert_eq!(split_at_anchors(&run, true, true).len(), 3);
    assert_eq!(split_at_anchors(&run, false, false), std::slice::from_ref(&run));
    let pieces = split_at_anchors(&run, true, true);
    assert!(near(pieces.iter().map(length).sum::<f64>(), length(&run), 1e-6), "the shape stays");
    // A closed subpath opens at its first cut.
    let sq = line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true);
    assert_eq!(split_at_anchors(&sq, true, false).len(), 3);
    assert!(split_at_anchors(&sq, true, false).iter().all(|p| !p.closed));
}

#[test]
fn crossings_on_finds_one_paths_places_among_many() {
    // A horizontal line crossed by two uprights, with a self-crossing bow far away.
    let across = PathData::single(line(&[(0.0, 50.0), (300.0, 50.0)], false));
    let up1 = PathData::single(line(&[(100.0, 0.0), (100.0, 100.0)], false));
    let up2 = PathData::single(line(&[(200.0, 0.0), (200.0, 100.0)], false));
    let far = PathData::single(line(&[(1000.0, 0.0), (1100.0, 100.0), (1100.0, 0.0), (1000.0, 100.0)], false));
    let places = crossings_on(&across, &[&up1, &up2, &far]).unwrap();
    assert_eq!(places.len(), 2, "{places:?}");
    let x = |p: &Place| across.subpaths[p.0].segment(p.1).eval(p.2).x;
    assert!(near(x(&places[0]), 100.0, 1e-6) && near(x(&places[1]), 200.0, 1e-6));
    // A path's crossings with itself count, once on each of the two segments.
    let own = crossings_on(&far, &[&across]).unwrap();
    assert_eq!(own.iter().map(|p| p.1).collect::<Vec<_>>(), [0, 2], "{own:?}");
    // A path that ends on another marks the other there, and its own end.
    let stub = PathData::single(line(&[(150.0, 50.0), (150.0, 90.0)], false));
    assert_eq!(crossings_on(&across, &[&stub]).unwrap().len(), 1);
    assert!(near(crossings_on(&stub, &[&across]).unwrap()[0].2, 0.0, 1e-6));
    // Lines lying along each other don't cross, and a path with a coordinate that isn't finite is refused.
    let along = PathData::single(line(&[(50.0, 50.0), (250.0, 50.0)], false));
    assert!(crossings_on(&across, &[&along]).unwrap().is_empty());
    let broken = PathData::single(line(&[(0.0, f64::NAN), (10.0, 10.0)], false));
    assert!(crossings_on(&broken, &[&across]).is_none());
    assert_eq!(crossings_on(&across, &[&broken, &up1]).unwrap().len(), 1, "a broken path beside it is left out");
    assert!(crossings_on(&PathData::default(), &[&across]).unwrap().is_empty());
}

#[test]
fn segments_that_share_an_anchor_still_cross_elsewhere() {
    // A straight segment, then a curve from its end that swings back across it near x = 17.4.
    let hook = SubPath::new(
        vec![Anchor::corner(pt(0.0, 0.0)), Anchor { h_out: pt(-50.0, 50.0), ..Anchor::corner(pt(100.0, 0.0)) }, Anchor::corner(pt(50.0, -50.0))],
        false,
    );
    let path = PathData::single(hook.clone());
    let places = crossings_on(&path, &[]).unwrap();
    assert_eq!(places.iter().map(|p| p.1).collect::<Vec<_>>(), [0, 1], "{places:?}");
    let x = hook.segment(0).eval(places[0].2).x;
    assert!(near(x, 17.4, 0.01), "{x}");
    assert!(hook.segment(1).eval(places[1].2).distance(pt(x, 0.0)) < 1e-3);
    // Split at Crossings finds it too, and neither counts the anchor the segments share.
    assert_eq!(crossings(&[&path], true).unwrap()[0], places);
    // The two segments of a closed subpath share both ends: a lens has no crossing.
    let lens = SubPath::new(
        vec![
            Anchor { h_out: pt(30.0, -40.0), h_in: pt(30.0, 40.0), ..Anchor::corner(pt(0.0, 0.0)) },
            Anchor { h_in: pt(70.0, -40.0), h_out: pt(70.0, 40.0), ..Anchor::corner(pt(100.0, 0.0)) },
        ],
        true,
    );
    assert!(crossings_on(&PathData::single(lens.clone()), &[]).unwrap().is_empty());
    assert!(crossings(&[&PathData::single(lens)], true).unwrap()[0].is_empty());
}

#[test]
fn segments_with_no_length_between_them_still_share_their_anchor() {
    // An open path with its middle anchor doubled, and a closed one whose last anchor sits on its
    // first: the segments either side of the one with no length meet there and don't cross.
    let doubled = PathData::single(line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false));
    let ring = PathData::single(line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)], true));
    for p in [&doubled, &ring] {
        assert!(crossings_on(p, &[]).unwrap().is_empty(), "{:?}", crossings_on(p, &[]));
        assert!(crossings(&[p], true).unwrap()[0].is_empty(), "{:?}", crossings(&[p], true));
    }
    // A segment that crosses them elsewhere still does.
    let bow = PathData::single(line(&[(0.0, 0.0), (100.0, 100.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)], false));
    assert_eq!(crossings_on(&bow, &[]).unwrap().iter().map(|p| p.1).collect::<Vec<_>>(), [0, 3]);
}

#[test]
fn one_work_budget_bounds_several_calls_together() {
    // 1,000 segments in a straight row: 500,000 pairs a call, so 120 calls spend 60,000,000.
    let row: Vec<(f64, f64)> = (0..=1000).map(|i| (i as f64, 0.0)).collect();
    let path = PathData::single(line(&row, false));
    let mut work = Work::default();
    let calls = (0..200).take_while(|_| crossings_on_within(&path, &[], &mut work).is_some()).count();
    assert_eq!(calls, 120);
    assert!(crossings_on(&path, &[]).is_some(), "a call of its own has its own budget");
}

#[test]
fn work_done_once_can_be_drawn_again_from_another_budget() {
    let row: Vec<(f64, f64)> = (0..=1000).map(|i| (i as f64, 0.0)).collect();
    let path = PathData::single(line(&row, false));
    let start = Work::default();
    let mut work = start;
    crossings_on_within(&path, &[], &mut work).unwrap();
    let cost = work.spent_since(start);
    // Drawing a call's cost leaves what the call itself leaves, 120 times and no more.
    let mut again = Work::default();
    assert_eq!(again.draw(cost), Some(()));
    assert_eq!(again, work);
    let draws = (1..200).take_while(|_| again.draw(cost).is_some()).count();
    assert_eq!(draws, 119);
    let left = again;
    assert_eq!(again.draw(cost), None);
    assert_eq!(again, left, "a draw that fails takes nothing");
    // A twentieth of the budget pays for the call 6 times.
    assert_eq!((Work::share(0), Work::share(1)), (Work::default(), Work::default()));
    let mut part = Work::share(20);
    assert_eq!((0..200).take_while(|_| part.draw(cost).is_some()).count(), 6);
}

#[test]
fn a_segment_that_loops_over_itself_crosses_itself() {
    // One cubic that meets itself at t = 0.25 and t = 0.75.
    let looped = PathData::single(SubPath::new(
        vec![Anchor { h_out: pt(13.0 / 9.0, 1.0), ..Anchor::corner(pt(0.0, 0.0)) }, Anchor { h_in: pt(0.0, 1.0), ..Anchor::corner(pt(1.0, 0.0)) }],
        false,
    ));
    let places = crossings_on(&looped, &[]).unwrap();
    assert_eq!(places.len(), 2, "{places:?}");
    assert!(near(places[0].2, 0.25, 1e-9) && near(places[1].2, 0.75, 1e-9), "{places:?}");
    assert_eq!(crossings(&[&looped], true).unwrap()[0], places);
    assert!(crossings(&[&looped], false).unwrap()[0].is_empty());
    // Its two visits to that point divide it into three stretches, and the loop can go alone.
    let sp = &looped.subpaths[0];
    let three = spans(sp, &places.iter().map(|p| (p.1, p.2)).collect::<Vec<_>>());
    assert_eq!(three.len(), 3, "{three:?}");
    let left = without_spans(sp, &three, &[false, true, false]).unwrap();
    assert_eq!(left.len(), 2);
    let x = sp.segment(0).eval(0.25);
    assert_eq!((left[0].anchors[0].p, left[1].anchors.last().unwrap().p), (pt(0.0, 0.0), pt(1.0, 0.0)));
    assert!(left[0].anchors.last().unwrap().p.distance(x) < 1e-3 && left[1].anchors[0].p.distance(x) < 1e-3);
    // The same loop squashed to a 10⁻¹² of its height.
    let flat = PathData::single(SubPath::new(
        vec![
            Anchor { h_out: pt(13.0 / 9.0, 1e-12), ..Anchor::corner(pt(0.0, 0.0)) },
            Anchor { h_in: pt(0.0, 1e-12), ..Anchor::corner(pt(1.0, 0.0)) },
        ],
        false,
    ));
    assert_eq!(crossings_on(&flat, &[]).unwrap().iter().map(|p| (p.2 * 4.0).round()).collect::<Vec<_>>(), [1.0, 3.0]);
    // An arch, an S and a cusp don't cross themselves.
    for (h1, h2) in [((0.0, 1.0), (1.0, 1.0)), ((0.3, 1.0), (0.7, -1.0)), ((1.0, 1.0), (0.0, 1.0))] {
        let c = PathData::single(SubPath::new(
            vec![Anchor { h_out: pt(h1.0, h1.1), ..Anchor::corner(pt(0.0, 0.0)) }, Anchor { h_in: pt(h2.0, h2.1), ..Anchor::corner(pt(1.0, 0.0)) }],
            false,
        ));
        assert!(crossings_on(&c, &[]).unwrap().is_empty(), "{h1:?} {h2:?}");
    }
    // Nor does a straight curve that doubles back along itself, at a slant.
    let back = PathData::single(SubPath::new(
        vec![Anchor { h_out: pt(1.6, 1.2), ..Anchor::corner(pt(0.0, 0.0)) }, Anchor { h_in: pt(-0.8, -0.6), ..Anchor::corner(pt(0.8, 0.6)) }],
        false,
    ));
    assert!(crossings_on(&back, &[]).unwrap().is_empty());
}

#[test]
fn spans_divide_a_subpath_at_its_places() {
    let open = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false);
    assert_eq!(spans(&open, &[]), [(0.0, 2.0)]);
    assert_eq!(spans(&open, &[(1, 0.5), (0, 0.25)]), [(0.0, 0.25), (0.25, 1.5), (1.5, 2.0)]);
    // Places at an open end, off the subpath, or twice at one point, add no stretch.
    assert_eq!(spans(&open, &[(0, 0.0), (1, 1.0), (7, 0.5), (0, 0.5), (0, 0.5 + 1e-9), (0, f64::NAN)]), [(0.0, 0.5), (0.5, 2.0)]);
    // A place at an anchor is the same from either segment.
    assert_eq!(spans(&open, &[(0, 1.0), (1, 0.0)]), [(0.0, 1.0), (1.0, 2.0)]);
    let ring = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
    assert_eq!(spans(&ring, &[]), [(0.0, 4.0)]);
    assert_eq!(spans(&ring, &[(1, 0.5)]), [(1.5, 5.5)], "one place: from it round to it again");
    assert_eq!(spans(&ring, &[(3, 0.5), (0, 0.5)]), [(0.5, 3.5), (3.5, 4.5)]);
    // The closing segment's end is the first anchor.
    assert_eq!(spans(&ring, &[(3, 1.0), (0, 0.0), (2, 0.0)]), [(0.0, 2.0), (2.0, 4.0)]);
    assert!(spans(&SubPath::new(vec![Anchor::corner(pt(1.0, 1.0))], false), &[(0, 0.5)]).is_empty());
    // Which stretch a place is on: the one that starts there, the last one at an open end.
    let three = spans(&open, &[(1, 0.5), (0, 0.25)]);
    assert_eq!(span_at(&open, &three, 0, 0.1), Some(0));
    assert_eq!(span_at(&open, &three, 0, 0.25), Some(1));
    assert_eq!(span_at(&open, &three, 1, 1.0), Some(2));
    assert_eq!(span_at(&open, &three, 0, f64::NAN), None);
    let two = spans(&ring, &[(3, 0.5), (0, 0.5)]);
    assert_eq!((span_at(&ring, &two, 0, 0.25), span_at(&ring, &two, 3, 0.75), span_at(&ring, &two, 2, 0.0)), (Some(1), Some(1), Some(0)));
    // The closing segment's end is the first anchor: on the stretch that starts there, or runs through it.
    let at_first = spans(&ring, &[(0, 0.0), (2, 0.0)]);
    assert_eq!((span_at(&ring, &at_first, 3, 1.0), span_at(&ring, &at_first, 0, 0.0)), (Some(0), Some(0)));
    assert_eq!((span_at(&ring, &two, 3, 1.0), span_at(&ring, &spans(&ring, &[]), 3, 1.0)), (Some(1), Some(0)));
}

#[test]
fn a_subpath_without_some_stretches_keeps_the_rest_as_it_was() {
    // An open line of three segments, crossed twice on its middle one: the middle stretch goes.
    let open = line(&[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (300.0, 0.0)], false);
    let three = spans(&open, &[(1, 0.25), (1, 0.75)]);
    let left = without_spans(&open, &three, &[false, true, false]).unwrap();
    let xs = |sp: &SubPath| sp.anchors.iter().map(|a| a.p.x).collect::<Vec<_>>();
    // A straight segment's parameter doesn't run evenly along it: the cuts are where it puts them.
    let (q1, q3) = (open.segment(1).eval(0.25).x, open.segment(1).eval(0.75).x);
    assert!(100.0 < q1 && q1 < q3 && q3 < 200.0);
    assert_eq!(left.iter().map(xs).collect::<Vec<_>>(), [vec![0.0, 100.0, q1], vec![q3, 200.0, 300.0]]);
    assert!(left.iter().all(|p| !p.closed && p.anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p)), "straight pieces have no handles");
    // Nothing marked: unchanged. All marked: nothing left. An end stretch: one piece.
    assert!(without_spans(&open, &three, &[false, false, false]).is_none());
    assert!(without_spans(&open, &three, &[]).is_none());
    assert_eq!(without_spans(&open, &three, &[true, true, true]), Some(vec![]));
    assert_eq!(without_spans(&open, &three, &[true, false, false]).unwrap().iter().map(xs).collect::<Vec<_>>(), [vec![q1, 200.0, 300.0]]);
    // Stretches left side by side stay one piece with no anchor where they meet.
    let four = spans(&open, &[(0, 0.5), (1, 0.5), (2, 0.5)]);
    assert_eq!(
        without_spans(&open, &four, &[false, false, false, true]).unwrap().iter().map(xs).collect::<Vec<_>>(),
        [vec![0.0, 100.0, 200.0, 250.0]]
    );
    // A closed square that loses a stretch opens, and its first anchor stays inside the piece.
    let ring = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
    let two = spans(&ring, &[(1, 0.5), (2, 0.5)]);
    let open_ring = without_spans(&ring, &two, &[true, false]).unwrap();
    assert_eq!(open_ring.len(), 1);
    let pts: Vec<Point> = open_ring[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(pts, [pt(50.0, 100.0), pt(0.0, 100.0), pt(0.0, 0.0), pt(100.0, 0.0), pt(100.0, 50.0)]);
    assert!(!open_ring[0].closed);
    // The same when the removed stretch is the one through the first anchor.
    let through = without_spans(&ring, &two, &[false, true]).unwrap();
    assert_eq!(through[0].anchors.iter().map(|a| a.p).collect::<Vec<_>>(), [pt(100.0, 50.0), pt(100.0, 100.0), pt(50.0, 100.0)]);
    // Three places: the two stretches left on both sides of the first anchor are one piece through it.
    let three = spans(&ring, &[(0, 0.5), (1, 0.5), (2, 0.5)]);
    assert_eq!(three, [(0.5, 1.5), (1.5, 2.5), (2.5, 4.5)]);
    let around = without_spans(&ring, &three, &[false, true, false]).unwrap();
    assert_eq!(around.len(), 1, "{around:?}");
    let pts: Vec<Point> = around[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(pts, [pt(50.0, 100.0), pt(0.0, 100.0), pt(0.0, 0.0), pt(100.0, 0.0), pt(100.0, 50.0)]);
    // A closed subpath with one place loses all of itself.
    assert_eq!(without_spans(&ring, &spans(&ring, &[(0, 0.5)]), &[true]), Some(vec![]));
}

#[test]
fn a_slice_of_a_curve_keeps_its_shape_and_the_anchors_it_runs_through() {
    let mut circle = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
    for i in 0..4 {
        auto_smooth(&mut circle, i, SMOOTH_RATIO);
    }
    let piece = slice(&circle, (0.5, 2.5));
    assert_eq!(piece.anchors.len(), 4);
    assert!(!piece.closed);
    // Its ends are corners with the handle off the piece retracted; the anchors between stay smooth, and
    // the whole segment between them keeps its handles.
    assert_eq!((piece.anchors[0].kind, piece.anchors[3].kind), (AnchorKind::Corner, AnchorKind::Corner));
    assert_eq!((piece.anchors[0].h_in, piece.anchors[3].h_out), (piece.anchors[0].p, piece.anchors[3].p));
    assert_eq!((piece.anchors[1].p, piece.anchors[2].p), (circle.anchors[1].p, circle.anchors[2].p));
    assert_eq!((piece.anchors[1].h_out, piece.anchors[2].h_in), (circle.anchors[1].h_out, circle.anchors[2].h_in));
    assert_eq!((piece.anchors[1].kind, piece.anchors[2].kind), (AnchorKind::Smooth, AnchorKind::Smooth));
    // Every point of the piece is on the circle's outline.
    for i in 0..piece.segment_count() {
        for k in 0..=8 {
            let p = piece.segment(i).eval(k as f64 / 8.0);
            let d = PathData::single(circle.clone()).nearest(p).unwrap().4;
            assert!(d < 1e-6, "{p:?} is {d} off");
        }
    }
    assert!(near(length(&piece), length(&circle) / 2.0, 1e-3));
    // No length, backwards or not finite: nothing.
    assert!(slice(&circle, (1.0, 1.0)).anchors.is_empty());
    assert!(slice(&circle, (2.0, 1.0)).anchors.is_empty());
    assert!(slice(&circle, (f64::NAN, 1.0)).anchors.is_empty());
}

#[test]
fn an_open_end_a_hair_off_another_path_still_meets_it() {
    // A bow from (0, 0) to (100, 100), and where a line across at y = 50 crosses it.
    let bow = PathData::single(SubPath::new(
        vec![Anchor { h_out: pt(0.0, 55.0), ..Anchor::corner(pt(0.0, 0.0)) }, Anchor { h_in: pt(45.0, 100.0), ..Anchor::corner(pt(100.0, 100.0)) }],
        false,
    ));
    let across = PathData::single(line(&[(-50.0, 50.0), (150.0, 50.0)], false));
    let at = crossings_on(&across, &[&bow]).unwrap();
    assert_eq!(at.len(), 1);
    let p = across.subpaths[0].segment(0).eval(at[0].2);
    // A line ending 0.00005 pt short of the bow, or past it, as a line cut where the bow crosses it
    // can: the bow is crossed there, on both searches.
    let on_bow = |l: &PathData| crossings_on(&bow, &[l]).unwrap().iter().map(|q| bow.subpaths[0].segment(q.1).eval(q.2)).collect::<Vec<_>>();
    for off in [-5e-5, 5e-5] {
        let stub = PathData::single(line(&[(-50.0, 50.0), (p.x + off, 50.0)], false));
        let found = on_bow(&stub);
        assert!(found.len() == 1 && found[0].distance(p) < 1e-3, "{off}: {found:?}");
        let both = crossings(&[&stub, &bow], false).unwrap();
        assert_eq!((both[0].len(), both[1].len()), (1, 1), "{off}: {both:?}");
    }
    // 0.01 pt short is not on it.
    assert!(on_bow(&PathData::single(line(&[(-50.0, 50.0), (p.x - 0.01, 50.0)], false))).is_empty());
    // The bow's own open end 0.00005 pt short of an upright meets it there.
    let upright = PathData::single(line(&[(100.00005, 50.0), (100.00005, 150.0)], false));
    let found = crossings_on(&upright, &[&bow]).unwrap();
    assert!(found.len() == 1 && near(upright.subpaths[0].segment(0).eval(found[0].2).y, 100.0, 1e-3), "{found:?}");
    // Two ends that each stop 0.0009 pt short of a corner lie 0.0013 pt apart: they don't meet.
    let flat = PathData::single(line(&[(0.0, 0.0), (99.9991, 0.0)], false));
    let post = PathData::single(line(&[(100.0, 0.0009), (100.0, 50.0)], false));
    assert!(crossings_on(&flat, &[&post]).unwrap().is_empty() && crossings_on(&post, &[&flat]).unwrap().is_empty());
    // Two ends that each stop 0.0004 pt short of a corner lie 0.0006 pt apart: they meet.
    let flat = PathData::single(line(&[(0.0, 0.0), (99.9996, 0.0)], false));
    let post = PathData::single(line(&[(100.0, 0.0004), (100.0, 50.0)], false));
    assert_eq!((crossings_on(&flat, &[&post]).unwrap().len(), crossings_on(&post, &[&flat]).unwrap().len()), (1, 1));
    // A level line that crosses a quarter circle near its top, where the two meet at under 3
    // degrees, cut 0.0015 pt short along its length: its end lies under 0.0001 pt from the arc and
    // meets it there.
    use kurbo::ParamCurveNearest;
    let arc = PathData::single(quarter());
    let level = PathData::single(line(&[(50.0, 99.9), (-50.0, 99.9)], false));
    let at = crossings_on(&level, &[&arc]).unwrap();
    assert_eq!(at.len(), 1);
    let end = level.subpaths[0].segment(0).eval(at[0].2) + Vec2::new(0.0015, 0.0);
    assert!(arc.subpaths[0].segment(0).nearest(end, 1e-12).distance_sq.sqrt() < 1e-4);
    let stub = PathData::single(line(&[(50.0, 99.9), (end.x, end.y)], false));
    let found = crossings_on(&arc, &[&stub]).unwrap();
    assert!(found.len() == 1 && arc.subpaths[0].segment(0).eval(found[0].2).distance(end) < 1e-3, "{found:?}");
    assert_eq!(crossings(&[&stub, &arc], false).unwrap().iter().map(Vec::len).collect::<Vec<_>>(), [1, 1]);
    // A corner of a closed path as near a line is not an open end: it doesn't reach the line.
    let wedge = PathData::single(line(&[(180.0, 0.0), (220.0, 0.0), (200.0, 49.99995)], true));
    let floor = PathData::single(line(&[(150.0, 50.0), (250.0, 50.0)], false));
    assert!(crossings_on(&floor, &[&wedge]).unwrap().is_empty() && crossings_on(&wedge, &[&floor]).unwrap().is_empty());
}
