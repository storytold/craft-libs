use craft_geom::{Anchor, PathData, SubPath};
use kurbo::{ParamCurve, Point};

use super::*;

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// A closed shape above y = 0 whose lower edge runs right to left from (100, 0) to (0, 0), curved
/// down by `bulge` control-point height.
fn above(bulge: f64) -> SubPath {
    SubPath::new(
        vec![
            Anchor::corner(pt(0.0, -50.0)),
            Anchor::corner(pt(100.0, -50.0)),
            Anchor::with_handles(pt(100.0, 0.0), pt(100.0, 0.0), pt(70.0, bulge)),
            Anchor::with_handles(pt(0.0, 0.0), pt(30.0, bulge), pt(0.0, 0.0)),
        ],
        true,
    )
}

/// A closed shape below y = 0 whose upper edge runs left to right, curved up by `bulge`.
fn below(bulge: f64) -> SubPath {
    SubPath::new(
        vec![
            Anchor::with_handles(pt(0.0, 0.0), pt(0.0, 0.0), pt(30.0, -bulge)),
            Anchor::with_handles(pt(100.0, 0.0), pt(70.0, -bulge), pt(100.0, 0.0)),
            Anchor::corner(pt(100.0, 50.0)),
            Anchor::corner(pt(0.0, 50.0)),
        ],
        true,
    )
}

fn single(sp: SubPath) -> PathData {
    PathData::single(sp)
}

/// The plan at `at` with a 20 pt brush.
fn plan(a: &PathData, b: &PathData, at: Point) -> MergePlan {
    stretches_at(a, b, at, 10.0, COINCIDENCE).expect("a stretch")
}

/// The outline of `chain` run one way, sampled.
fn samples(chain: &[CubicBez]) -> Vec<Point> {
    let n = chain.len() * 16;
    let at = |k: usize| {
        let f = k as f64 / n as f64 * chain.len() as f64;
        let i = (f.floor() as usize).min(chain.len() - 1);
        chain[i].eval(f - i as f64)
    };
    (0..=n).map(at).collect()
}

/// The edge as `path` now has it between anchors `from` (inclusive) and `to`, along the path.
fn edge(path: &PathData, from: usize, segments: usize) -> Vec<CubicBez> {
    let sp = &path.subpaths[0];
    (0..segments).map(|k| sp.segment((from + k) % sp.segment_count())).collect()
}

/// The farthest apart two chains are, sampled alike.
fn apart(x: &[CubicBez], y: &[CubicBez]) -> f64 {
    samples(x).iter().zip(samples(y)).map(|(p, q)| p.distance(q)).fold(0.0, f64::max)
}

#[test]
fn the_merged_edge_lies_half_way_and_both_paths_keep_their_topology() {
    let (a, b) = (single(above(6.0)), single(below(2.0)));
    let plan = plan(&a, &b, pt(50.0, 0.0));
    assert_eq!(
        plan,
        MergePlan::Edge {
            a: Stretch { subpath: 0, start: 2, segments: 1, reversed: false },
            b: Stretch { subpath: 0, start: 0, segments: 1, reversed: true },
        }
    );
    let (na, nb) = apply_merge(&a, &b, &plan, Follow::Average).unwrap();
    assert!(na.subpaths[0].closed && nb.subpaths[0].closed);
    assert_eq!((na.anchor_count(), nb.anchor_count()), (4, 4));
    // Everything else is where it was.
    assert_eq!((na.subpaths[0].anchors[0], na.subpaths[0].anchors[1]), (a.subpaths[0].anchors[0], a.subpaths[0].anchors[1]));
    assert_eq!((nb.subpaths[0].anchors[2], nb.subpaths[0].anchors[3]), (b.subpaths[0].anchors[2], b.subpaths[0].anchors[3]));
    // One edge, run opposite ways by the two paths, between the two it replaced.
    let (ea, eb) = (edge(&na, 2, 1), edge(&nb, 0, 1));
    assert!(apart(&ea, &backward(&eb)) < 1e-9);
    let (upper, lower) = (a.subpaths[0].segment(2), b.subpaths[0].segment(0));
    for k in 0..=16 {
        let t = k as f64 / 16.0;
        let (p, u, l) = (ea[0].eval(t), upper.eval(t), lower.eval(1.0 - t));
        assert!(p.distance(u.midpoint(l)) < 1e-9, "{p:?} is half way between {u:?} and {l:?}");
        assert!(l.y - 1e-9 <= p.y && p.y <= u.y + 1e-9);
    }
    // Merged: there is nothing left to do, and it says so.
    let again = stretches_at(&na, &nb, pt(50.0, 0.0), 10.0, COINCIDENCE).unwrap();
    assert!(matches!(merge_preview(&na, &nb, &again, Follow::Average), Some(MergePreview::Edge { done: true, .. })));
    let (ma, mb) = apply_merge(&na, &nb, &again, Follow::Average).unwrap();
    assert!(apart(&edge(&ma, 2, 1), &edge(&na, 2, 1)) < 1e-9 && apart(&edge(&mb, 0, 1), &edge(&nb, 0, 1)) < 1e-9);
}

#[test]
fn following_a_path_keeps_it_and_the_other_takes_its_edge() {
    let (a, b) = (single(above(6.0)), single(below(2.0)));
    let plan = plan(&a, &b, pt(50.0, 0.0));
    let (na, nb) = apply_merge(&a, &b, &plan, Follow::First).unwrap();
    assert_eq!(na, a);
    assert!(apart(&edge(&nb, 0, 1), &backward(&edge(&a, 2, 1))) < 1e-9);
    let (na, nb) = apply_merge(&a, &b, &plan, Follow::Second).unwrap();
    assert_eq!(nb, b);
    assert!(apart(&edge(&na, 2, 1), &backward(&edge(&b, 0, 1))) < 1e-9);
}

#[test]
fn stretches_of_different_anchor_counts_get_the_same_anchors() {
    let (with_two, _) = insert_at(&above(6.0), &[(2, 0.3)]);
    assert_eq!(with_two.anchors.len(), 5);
    let (a, b) = (single(with_two), single(below(2.0)));
    let plan = plan(&a, &b, pt(50.0, 0.0));
    let (na, nb) = apply_merge(&a, &b, &plan, Follow::Average).unwrap();
    assert_eq!((na.anchor_count(), nb.anchor_count()), (5, 5));
    let (ea, eb) = (edge(&na, 2, 2), edge(&nb, 0, 2));
    assert!(apart(&ea, &backward(&eb)) < 1e-9);
    assert_eq!(ea[0].p3, eb[0].p3);
    // The anchors were smooth on the one and a point of a curve on the other: the joint stays smooth.
    assert_eq!(na.subpaths[0].anchors[3].kind, AnchorKind::Smooth);
    // Anchors at nearly the same place stand for one.
    let (b2, _) = insert_at(&below(2.0), &[(0, 0.7)]);
    let (a2, b2) = (a.clone(), single(b2));
    let (na, nb) = apply_merge(&a2, &b2, &stretches_at(&a2, &b2, pt(50.0, 0.0), 10.0, COINCIDENCE).unwrap(), Follow::Average).unwrap();
    assert_eq!((na.anchor_count(), nb.anchor_count()), (5, 5), "one anchor inside, not two");
}

#[test]
fn open_paths_stay_open_and_keep_what_lies_beyond_the_stretch() {
    let a = single(SubPath::polyline(&[pt(-50.0, 0.0), pt(0.0, 0.0), pt(50.0, 4.0), pt(100.0, 0.0), pt(150.0, 40.0)], false));
    let b = single(SubPath::polyline(&[pt(100.0, 0.0), pt(50.0, -4.0), pt(0.0, 0.0), pt(-20.0, 60.0)], false));
    let plan = plan(&a, &b, pt(25.0, 0.0));
    assert_eq!(
        plan,
        MergePlan::Edge {
            a: Stretch { subpath: 0, start: 1, segments: 2, reversed: false },
            b: Stretch { subpath: 0, start: 0, segments: 2, reversed: true },
        }
    );
    let (na, nb) = apply_merge(&a, &b, &plan, Follow::Average).unwrap();
    assert!(!na.subpaths[0].closed && !nb.subpaths[0].closed);
    let (pa, pb) = (&na.subpaths[0].anchors, &nb.subpaths[0].anchors);
    assert_eq!(pa.iter().map(|x| x.p).collect::<Vec<_>>(), [pt(-50.0, 0.0), pt(0.0, 0.0), pt(50.0, 0.0), pt(100.0, 0.0), pt(150.0, 40.0)]);
    assert_eq!(pb.iter().map(|x| x.p).collect::<Vec<_>>(), [pt(100.0, 0.0), pt(50.0, 0.0), pt(0.0, 0.0), pt(-20.0, 60.0)]);
}

#[test]
fn a_stretch_over_the_start_of_a_closed_path_stays_closed_and_whole() {
    // The edge along y = 0 runs from anchor 3 over anchor 0, which has no partner, to anchor 1.
    let a = single(SubPath::polyline(&[pt(50.0, 1.0), pt(100.0, 0.0), pt(100.0, -50.0), pt(0.0, 0.0)], true));
    let b = single(SubPath::polyline(&[pt(100.0, 0.0), pt(0.0, 0.0), pt(0.0, 50.0), pt(100.0, 50.0)], true));
    let plan = plan(&a, &b, pt(25.0, 0.5));
    let MergePlan::Edge { a: sa, .. } = plan else { panic!("{plan:?}") };
    assert_eq!((sa.start, sa.segments), (3, 2));
    let (na, nb) = apply_merge(&a, &b, &plan, Follow::Average).unwrap();
    assert!(na.subpaths[0].closed && nb.subpaths[0].closed);
    assert_eq!((na.anchor_count(), nb.anchor_count()), (4, 5));
    // Every anchor of the new outlines is where one was, or on the merged edge at y 0.5.
    let at = |path: &PathData, p: Point| path.anchors().any(|(_, _, x)| x.p.distance(p) < 1e-4);
    assert!(at(&na, pt(100.0, -50.0)) && at(&nb, pt(0.0, 50.0)) && at(&nb, pt(100.0, 50.0)));
    assert!(at(&na, pt(50.0, 0.5)) && at(&nb, pt(50.0, 0.5)), "{na:?}");
}

#[test]
fn the_stretch_under_the_brush_is_the_one_along_the_shared_edge_not_the_long_way_round() {
    let square = |x: f64| single(SubPath::polyline(&[pt(x, 0.0), pt(x + 100.0, 0.0), pt(x + 100.0, 100.0), pt(x, 100.0)], true));
    let (a, b) = (square(0.0), square(100.0));
    // Above the corner, each path's top edge is nearest: their long ways round face each other.
    let MergePlan::Edge { a: sa, b: sb } = plan(&a, &b, pt(99.0, -2.0)) else { panic!() };
    assert_eq!((sa.start, sa.segments), (1, 1));
    assert_eq!((sb.start, sb.segments), (3, 1));
}

#[test]
fn stretches_too_unlike_or_without_two_coincident_ends_are_left_alone() {
    let (a, b) = (single(above(6.0)), single(below(2.0)));
    assert_eq!(stretches_at(&a, &b, pt(50.0, 40.0), 10.0, COINCIDENCE), None, "the brush doesn't touch both");
    // One coincident anchor only.
    let c = single(SubPath::polyline(&[pt(0.0, 0.0), pt(101.0, 0.0)], false));
    assert_eq!(stretches_at(&a, &c, pt(50.0, 0.0), 10.0, COINCIDENCE), None);
    // Coincident within a wider tolerance.
    assert!(stretches_at(&a, &c, pt(50.0, 2.0), 10.0, 1.5).is_some());
    // One side twice as long as the other, both ends coincident.
    let hump = single(SubPath::polyline(&[pt(0.0, 0.0), pt(50.0, 5.0), pt(50.0, 60.0), pt(100.0, 0.0)], false));
    let flat = single(SubPath::polyline(&[pt(0.0, 0.0), pt(100.0, 0.0)], false));
    assert_eq!(stretches_at(&hump, &flat, pt(25.0, 1.0), 10.0, COINCIDENCE), None);
    // Past the limits, or not finite.
    let mut bad = a.clone();
    bad.subpaths[0].anchors[0].p.x = f64::NAN;
    assert_eq!(stretches_at(&bad, &b, pt(50.0, 0.0), 10.0, COINCIDENCE), None);
    assert_eq!(ends_at(&bad, &b, pt(50.0, 0.0), 10.0), None);
    assert_eq!(stretches_at(&a, &b, pt(f64::NAN, 0.0), 10.0, COINCIDENCE), None);
    assert_eq!(stretches_at(&PathData::default(), &b, pt(50.0, 0.0), 10.0, COINCIDENCE), None);
    assert_eq!(ends_at(&PathData::default(), &PathData::default(), pt(50.0, 0.0), 10.0), None);
}

#[test]
fn near_anchors_meet_half_way_or_on_the_path_followed() {
    let a = single(SubPath::polyline(&[pt(0.0, 0.0), pt(100.0, 0.0)], false));
    let b = single(SubPath::polyline(&[pt(104.0, 2.0), pt(200.0, 0.0)], false));
    let plan = ends_at(&a, &b, pt(102.0, 1.0), 10.0).unwrap();
    let ends = |f| {
        let (na, nb) = apply_merge(&a, &b, &plan, f).unwrap();
        (na.subpaths[0].anchors[1].p, nb.subpaths[0].anchors[0].p)
    };
    assert_eq!(ends(Follow::Average), (pt(102.0, 1.0), pt(102.0, 1.0)));
    assert_eq!(ends(Follow::First), (pt(100.0, 0.0), pt(100.0, 0.0)));
    assert_eq!(ends(Follow::Second), (pt(104.0, 2.0), pt(104.0, 2.0)));
    assert_eq!(merge_preview(&a, &b, &plan, Follow::Average), Some(MergePreview::Ends { a: pt(100.0, 0.0), b: pt(104.0, 2.0), to: pt(102.0, 1.0) }));
    // Together already: nothing to do.
    let c = single(SubPath::polyline(&[pt(100.0, 0.0), pt(100.0, 90.0)], false));
    assert_eq!(ends_at(&a, &c, pt(100.0, 1.0), 10.0), None);
    // Nothing there.
    assert_eq!(ends_at(&a, &b, pt(50.0, 1.0), 10.0), None);
}

#[test]
fn an_anchor_one_path_lacks_is_added_to_match() {
    let a = single(SubPath::polyline(&[pt(0.0, 0.0), pt(100.0, 2.0), pt(100.0, 90.0)], false));
    let b = single(SubPath::polyline(&[pt(0.0, 0.0), pt(200.0, 0.0)], false));
    let plan = ends_at(&a, &b, pt(100.0, 1.0), 10.0).unwrap();
    assert!(matches!(plan, MergePlan::Ends { a: EndSide::Anchor { anchor: 1, .. }, b: EndSide::Insert { .. } }), "{plan:?}");
    let (na, nb) = apply_merge(&a, &b, &plan, Follow::Average).unwrap();
    assert_eq!(na, a, "the anchor stays");
    assert_eq!(nb.anchor_count(), 3);
    assert!(nb.subpaths[0].anchors[1].p.distance(pt(100.0, 2.0)) < 1e-9);
    // Following the path that lacks it, the anchor comes to it.
    let (na, nb) = apply_merge(&a, &b, &plan, Follow::Second).unwrap();
    assert!(na.subpaths[0].anchors[1].p.distance(pt(100.0, 0.0)) < 1e-6);
    assert!(nb.subpaths[0].anchors[1].p.distance(na.subpaths[0].anchors[1].p) < 1e-9);
    // The same the other way round.
    let swapped = ends_at(&b, &a, pt(100.0, 1.0), 10.0).unwrap();
    let (nb, na) = apply_merge(&b, &a, &swapped, Follow::Average).unwrap();
    assert_eq!((na.anchor_count(), nb.anchor_count()), (3, 3));
    // With both paths' ends made, the stretch between can merge.
    let c = single(SubPath::polyline(&[pt(100.0, 2.0), pt(100.0, 90.0)], false));
    let d = single(SubPath::polyline(&[pt(95.0, 2.0), pt(95.0, 90.0)], false));
    assert_eq!(stretches_at(&c, &d, pt(97.0, 50.0), 10.0, COINCIDENCE), None);
    let first = ends_at(&c, &d, pt(97.0, 3.0), 10.0).unwrap();
    let (c, d) = apply_merge(&c, &d, &first, Follow::Average).unwrap();
    let second = ends_at(&c, &d, pt(97.0, 89.0), 10.0).unwrap();
    let (c, d) = apply_merge(&c, &d, &second, Follow::Average).unwrap();
    assert!(stretches_at(&c, &d, pt(97.5, 50.0), 10.0, COINCIDENCE).is_some());
}

#[test]
fn a_smooth_end_whose_handles_part_turns_into_a_corner() {
    let a = single(SubPath::new(
        vec![Anchor::smooth(pt(0.0, 0.0), pt(30.0, 10.0)), Anchor::with_handles(pt(100.0, 0.0), pt(70.0, 10.0), pt(100.0, 0.0))],
        true,
    ));
    let b = single(SubPath::polyline(&[pt(0.0, 0.0), pt(100.0, 0.0)], false));
    let plan = plan(&a, &b, pt(50.0, 3.0));
    let (na, _) = apply_merge(&a, &b, &plan, Follow::Second).unwrap();
    assert_eq!(na.subpaths[0].anchors[0].kind, AnchorKind::Corner);
}

#[test]
fn follow_names_round_trip() {
    for f in Follow::ALL {
        assert_eq!(Follow::parse(f.id()), Some(f));
    }
    assert_eq!(Follow::parse("up"), None);
}

/// A plan made before the path was edited must be refused, never wrapped: an open subpath has no
/// way on past its last segment, and the splice would edit the wrong anchors.
#[test]
fn a_stale_plan_for_a_shortened_open_path_is_refused() {
    let shortened = single(SubPath::polyline(&[pt(0.0, 0.0), pt(10.0, 0.0), pt(20.0, 0.0)], false));
    let b = single(SubPath::polyline(&[pt(0.0, -1.0), pt(10.0, -1.0), pt(20.0, -1.0)], false));
    let plan = MergePlan::Edge {
        a: Stretch { subpath: 0, start: 1, segments: 2, reversed: false },
        b: Stretch { subpath: 0, start: 0, segments: 2, reversed: true },
    };
    assert!(merge_preview(&shortened, &b, &plan, Follow::Average).is_none());
    assert!(apply_merge(&shortened, &b, &plan, Follow::Average).is_none());
}
