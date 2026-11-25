use std::collections::HashMap;

use craft_geom::{Anchor, AnchorKind, EPS, SubPath};
use kurbo::{Point, Vec2};

use super::{
    End, Rejoined, chains, close_nearly_closed, merge_duplicate_anchors, near_point_groups, outward, rejoin, remove_collinear_anchors,
    retract_tiny_handles, root,
};

fn line(pts: &[(f64, f64)], closed: bool) -> SubPath {
    SubPath::polyline(&pts.iter().map(|&p| p.into()).collect::<Vec<Point>>(), closed)
}

fn points(sp: &SubPath) -> Vec<(f64, f64)> {
    sp.anchors.iter().map(|a| (a.p.x, a.p.y)).collect()
}

#[test]
fn collinear_anchors_go_and_corners_stay() {
    let mut sp = line(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 5.0)], true);
    assert_eq!(remove_collinear_anchors(&mut sp, 0.005), 2);
    assert_eq!(points(&sp), [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    assert_eq!(remove_collinear_anchors(&mut sp, 0.005), 0, "clean art is left alone");
    // A turn back on itself is not a straight run; an open end stays.
    let mut back = line(&[(0.0, 0.0), (10.0, 0.0), (5.0, 0.0)], false);
    assert_eq!(remove_collinear_anchors(&mut back, 0.005), 0);
    // An anchor with a handle is not a corner on a straight run.
    let mut curved = line(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0)], false);
    curved.anchors[1] = Anchor::smooth((5.0, 0.0).into(), (7.0, 0.0).into());
    assert_eq!(remove_collinear_anchors(&mut curved, 0.005), 0);
    // A slight bend within the tolerance goes; a larger one stays.
    let mut bend = line(&[(0.0, 0.0), (100.0, 0.1), (200.0, 0.0)], false);
    assert_eq!(remove_collinear_anchors(&mut bend.clone(), 0.0001), 0);
    assert_eq!(remove_collinear_anchors(&mut bend, 0.005), 1);
}

/// The collinear-anchor removal done one `Vec::remove` at a time, testing the anchor that moves
/// into the gap next: the result [`remove_collinear_anchors`] gives.
fn one_at_a_time(sp: &mut SubPath, tolerance: f64) -> usize {
    let min = if sp.closed { 3 } else { 2 };
    let mut removed = 0;
    let mut i = 0;
    while i < sp.anchors.len() && sp.anchors.len() > min {
        let n = sp.anchors.len();
        let inner = sp.closed || (i > 0 && i + 1 < n);
        let (prev, cur, next) = (sp.anchors[(i + n - 1) % n], sp.anchors[i], sp.anchors[(i + 1) % n]);
        let straight = !prev.has_out() && !cur.has_in() && !cur.has_out() && !next.has_in();
        let (a, b) = (cur.p - prev.p, next.p - cur.p);
        let len = a.hypot() * b.hypot();
        if inner && straight && len > EPS && a.dot(b) > 0.0 && a.cross(b).abs() / len <= tolerance {
            sp.anchors.remove(i);
            removed += 1;
        } else {
            i += 1;
        }
    }
    removed
}

/// A subpath's closed flag and every anchor's coordinates bit for bit, with its kind.
fn bits(sp: &SubPath) -> (bool, Vec<([u64; 6], AnchorKind)>) {
    let anchors = sp.anchors.iter().map(|a| ([a.p.x, a.p.y, a.h_in.x, a.h_in.y, a.h_out.x, a.h_out.y].map(f64::to_bits), a.kind));
    (sp.closed, anchors.collect())
}

/// Check that [`remove_collinear_anchors`] gives `sp` the count and anchors [`one_at_a_time`] does.
fn same_as_one_at_a_time(sp: &SubPath, tolerance: f64) {
    let (mut fast, mut slow) = (sp.clone(), sp.clone());
    let got = remove_collinear_anchors(&mut fast, tolerance);
    let want = one_at_a_time(&mut slow, tolerance);
    assert_eq!(got, want, "count at tolerance {tolerance} for {sp:?}");
    assert_eq!(bits(&fast), bits(&slow), "anchors at tolerance {tolerance} for {sp:?}");
}

/// xorshift64: the same numbers on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[(self.next() % xs.len() as u64) as usize]
    }
}

/// A subpath of `n` anchors stepping mostly straight along one direction, with repeated points,
/// small and large bends, reversals, short and long handles and non-finite or huge coordinates.
fn generated(r: &mut Rng, n: usize, closed: bool) -> SubPath {
    let dir = r.pick(&[Vec2::new(1.0, 0.0), Vec2::new(0.0, -1.0), Vec2::new(0.6, 0.8), Vec2::new(-1.0, 1.0)]);
    let side = Vec2::new(-dir.y, dir.x);
    let mut at = Point::new(r.pick(&[0.0, 3.0, -7.5]), r.pick(&[0.0, 2.0]));
    let anchors = (0..n)
        .map(|_| {
            at += dir * r.pick(&[1.0, 1.0, 1.0, 2.0, 0.5, 0.0, -1.0]);
            let bend = r.pick(&[0.0, 0.0, 0.0, 0.0, 1e-3, 0.004, 0.005, 0.006, 1.0]);
            let mut p = at + side * bend;
            match r.next() % 40 {
                0 => p.x = f64::NAN,
                1 => p.y = f64::INFINITY,
                2 => p.x = f64::NEG_INFINITY,
                3 => p = Point::new(p.x * 1e300, p.y * 1e300),
                4 => p = Point::new(1e-300, -1e-300),
                _ => {}
            }
            let handle = |r: &mut Rng| p + Vec2::new(r.pick(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1e-10, 1.0]), 0.0);
            let (h_in, h_out) = (handle(r), handle(r));
            Anchor { p, h_in, h_out, kind: r.pick(&[AnchorKind::Corner, AnchorKind::Corner, AnchorKind::Smooth]) }
        })
        .collect();
    SubPath::new(anchors, closed)
}

const TOLERANCES: [f64; 12] = [-1.0, -0.0, 0.0, 1e-9, 0.001, 0.005, 0.5, 1.0, 2.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY];

#[test]
fn collinear_anchors_go_as_when_removed_one_at_a_time() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    for n in (0..40).chain([64, 100, 257]) {
        for _ in 0..40 {
            for closed in [false, true] {
                let sp = generated(&mut r, n, closed);
                for tolerance in TOLERANCES {
                    same_as_one_at_a_time(&sp, tolerance);
                }
            }
        }
    }
}

#[test]
fn collinear_anchors_keep_their_neighbors_order_and_ends() {
    let ring = |pts: &[(f64, f64)]| line(pts, true);
    let cases = [
        // A closed run whose first anchor goes: the last anchor's next neighbor is then the second.
        ring(&[(5.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)]),
        ring(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (4.0, 0.0), (5.0, 0.0), (2.0, 4.0)]),
        // Every anchor on one line: the path stops at three (closed) or two (open) anchors.
        ring(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (4.0, 0.0)]),
        line(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (4.0, 0.0)], false),
        // The last anchor lies between the first one kept and the one before it.
        ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 5.0)]),
        // Repeated and zero-length points.
        line(&[(0.0, 0.0), (0.0, 0.0), (1.0, 0.0), (1.0, 0.0), (2.0, 0.0)], false),
        ring(&[(0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]),
        // Products of huge coordinates overflow; tiny ones underflow.
        line(&[(0.0, 0.0), (1e200, 1e200), (2e200, 2e200), (f64::MAX, f64::MAX)], false),
        line(&[(0.0, 0.0), (1e-200, 0.0), (2e-200, 0.0)], false),
        line(&[(0.0, 0.0), (f64::NAN, 0.0), (2.0, 0.0), (f64::INFINITY, 0.0), (5.0, 0.0)], false),
    ];
    for sp in &cases {
        for tolerance in TOLERANCES {
            same_as_one_at_a_time(sp, tolerance);
        }
    }
    let mut first = cases[0].clone();
    assert_eq!(remove_collinear_anchors(&mut first, 0.0), 1);
    assert_eq!(points(&first), [(10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)]);
    let (mut closed, mut open) = (cases[2].clone(), cases[3].clone());
    assert_eq!((remove_collinear_anchors(&mut closed, 0.0), closed.anchors.len()), (2, 3));
    assert_eq!(remove_collinear_anchors(&mut open, 0.0), 3);
    assert_eq!(points(&open), [(0.0, 0.0), (4.0, 0.0)], "open ends stay");
    // A handle on either side of either segment keeps the anchor between them.
    for side in 0..4 {
        let mut sp = line(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)], false);
        let (i, h) = [(0, 1), (1, 0), (1, 1), (2, 0)][side];
        for kind in [AnchorKind::Corner, AnchorKind::Smooth] {
            let a = &mut sp.anchors[i];
            a.kind = kind;
            *(if h == 0 { &mut a.h_in } else { &mut a.h_out }) = a.p + Vec2::new(0.0, 1.0);
            same_as_one_at_a_time(&sp, 0.005);
            assert_eq!(remove_collinear_anchors(&mut sp.clone(), 0.005), 0, "handle {h} on anchor {i}");
        }
    }
}

#[test]
fn collinear_anchors_at_the_tolerance_go_and_just_past_it_stay() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.1), (200.0, 0.0)], false);
    let (a, b) = (sp.anchors[1].p - sp.anchors[0].p, sp.anchors[2].p - sp.anchors[1].p);
    let sine = a.cross(b).abs() / (a.hypot() * b.hypot());
    for (tolerance, removed) in [(sine, 1), (sine.next_down(), 0), (sine.next_up(), 1)] {
        same_as_one_at_a_time(&sp, tolerance);
        assert_eq!(remove_collinear_anchors(&mut sp.clone(), tolerance), removed, "{tolerance}");
    }
}

#[test]
fn collinear_anchors_go_in_linear_work() {
    for (n, closed) in [(10_000, false), (20_000, false), (20_000, true)] {
        let pts: Vec<Point> = (0..n).map(|i| Point::new(i as f64, 0.0)).collect();
        let mut sp = SubPath::polyline(&pts, closed);
        let removed = remove_collinear_anchors(&mut sp, 0.005);
        // A closed straight run stops at three anchors; an open one keeps its two ends.
        assert_eq!((removed, sp.anchors.len()), if closed { (n - 3, 3) } else { (n - 2, 2) });
        let work = super::COLLINEAR_WORK.with(std::cell::Cell::get);
        assert!(work <= 2 * n, "{n} anchors took {work} steps");
    }
}

#[test]
fn duplicate_anchors_merge_keeping_the_outer_handles() {
    let mut sp = SubPath::new(
        vec![
            Anchor::with_handles((0.0, 0.0).into(), (0.0, 0.0).into(), (0.0, 0.0).into()),
            Anchor::with_handles((10.0, 0.0).into(), (8.0, 0.0).into(), (10.0, 0.0).into()),
            Anchor::with_handles((10.005, 0.0).into(), (10.005, 0.0).into(), (10.005, 3.0).into()),
            Anchor::corner((10.0, 10.0).into()),
        ],
        false,
    );
    assert_eq!(merge_duplicate_anchors(&mut sp, 0.01), 1);
    assert_eq!(sp.anchors.len(), 3);
    let a = sp.anchors[1];
    assert_eq!((a.p, a.h_in), (Point::new(10.0, 0.0), Point::new(8.0, 0.0)));
    assert!(a.h_out.distance(Point::new(10.0, 3.0)) < 1e-9, "{:?}", a.h_out);
    assert_eq!(merge_duplicate_anchors(&mut sp, 0.01), 0);
    // A closed path's last anchor on its first merges into it.
    let mut ring = line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.0)], true);
    assert_eq!(merge_duplicate_anchors(&mut ring, 0.01), 1);
    assert_eq!(points(&ring), [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
}

#[test]
fn tiny_handles_retract() {
    let p = Point::new(5.0, 5.0);
    let mut sp = SubPath::new(vec![Anchor { p, h_in: p - Vec2::new(0.004, 0.0), h_out: p + Vec2::new(3.0, 0.0), kind: AnchorKind::Smooth }], false);
    assert_eq!(retract_tiny_handles(&mut sp, 0.01), 1);
    let a = sp.anchors[0];
    assert_eq!((a.h_in, a.kind), (p, AnchorKind::Corner));
    assert_eq!(a.h_out, p + Vec2::new(3.0, 0.0), "a long handle stays");
    assert_eq!(retract_tiny_handles(&mut sp, 0.01), 0);
}

#[test]
fn nearly_closed_paths_close() {
    let mut sp = line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.005, 0.0)], false);
    assert!(close_nearly_closed(&mut sp, 0.01));
    assert!(sp.closed);
    assert_eq!(points(&sp), [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
    let mut apart = line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (1.0, 0.0)], false);
    assert!(!close_nearly_closed(&mut apart, 0.01));
    assert!(!apart.closed);
    let mut two = line(&[(0.0, 0.0), (0.0, 0.0)], false);
    assert!(!close_nearly_closed(&mut two, 0.01), "two anchors make no shape");
}

#[test]
fn near_points_group_but_linked_and_equal_ones_do_not() {
    let pts: Vec<Point> = [(0.0, 0.0), (0.005, 0.0), (5.0, 5.0), (5.0, 5.0), (9.0, 9.0), (9.004, 9.0), (f64::NAN, 0.0)].map(Into::into).to_vec();
    let groups = near_point_groups(&pts, 0.01, |a, b| (a, b) == (4, 5));
    assert_eq!(groups, [vec![0, 1]], "equal points and linked ones are not snapped");
    assert!(near_point_groups(&pts, 0.0, |_, _| false).is_empty());
    assert!(near_point_groups(&pts, f64::NAN, |_, _| false).is_empty());
    // Far coordinates don't overflow the grid.
    let far: Vec<Point> = [(1e300, 1e300), (-1e300, 1e300)].map(Into::into).to_vec();
    assert!(near_point_groups(&far, 1e-300, |_, _| false).is_empty());
}

/// Nearly Coincident Anchors' grouping comparing every pair of points in touching grid cells: the
/// groups [`near_point_groups`] gives.
fn near_groups_pair_by_pair(points: &[Point], distance: f64, linked: impl Fn(usize, usize) -> bool) -> Vec<Vec<usize>> {
    if !(distance > 0.0 && distance.is_finite()) {
        return vec![];
    }
    let cell = |p: Point| ((p.x / distance).floor() as i64, (p.y / distance).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in points.iter().enumerate().filter(|(_, p)| p.is_finite()) {
        grid.entry(cell(*p)).or_default().push(i);
    }
    let mut parent: Vec<usize> = (0..points.len()).collect();
    for (&(cx, cy), members) in &grid {
        for (dx, dy) in [(0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let Some(others) = grid.get(&(cx.saturating_add(dx), cy.saturating_add(dy))) else { continue };
            for &i in members {
                for &j in others.iter().filter(|&&j| (dx, dy) != (0, 0) || j > i) {
                    let d = points[i].distance(points[j]);
                    if d <= distance && d > 0.0 && !linked(i.min(j), i.max(j)) {
                        let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                        parent[ri.max(rj)] = ri.min(rj);
                    }
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..points.len() {
        groups.entry(root(&mut parent, i)).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() > 1).collect();
    out.sort_unstable();
    out
}

/// Points around a few spots on a lattice, many at the same place, with signed zeros, huge, tiny
/// and non-finite coordinates.
fn generated_points(r: &mut Rng, n: usize, distance: f64) -> Vec<Point> {
    let spots: Vec<Point> = (0..1 + r.next() % 6).map(|_| Point::new((r.next() % 5) as f64 * distance, (r.next() % 5) as f64 * distance)).collect();
    (0..n)
        .map(|_| {
            let jitter = |r: &mut Rng| r.pick(&[0.0, 0.0, -0.0, 0.1, 0.3, 0.5, 0.7, 0.99, 1.0, 1.01, 2.0, -0.5, -1.0]) * distance;
            let at = r.pick(&spots);
            let p = Point::new(at.x + jitter(r), at.y + jitter(r));
            match r.next() % 50 {
                0 => Point::new(f64::NAN, p.y),
                1 => Point::new(p.x, f64::INFINITY),
                2 => Point::new(-0.0, -0.0),
                3 => Point::new(p.x * 1e300, p.y),
                4 => Point::new(1e-300, 1e-300),
                _ => p,
            }
        })
        .collect()
}

#[test]
fn near_points_group_as_when_every_pair_is_compared() {
    let mut r = Rng(0x2545_f491_4f6c_dd1d);
    let linked: [fn(usize, usize) -> bool; 5] =
        [|_, _| false, |a, b| b == a + 1, |a, b| (a * 31 + b * 17) % 5 == 0, |a, b| (a + b) % 2 == 0, |_, _| true];
    for n in (0..30).chain([64, 150, 300]) {
        for _ in 0..12 {
            for distance in [0.01, 0.5, 1.0, 3.0, 1e-200, 1e300] {
                let pts = generated_points(&mut r, n, distance);
                for (k, linked) in linked.iter().enumerate() {
                    let want = near_groups_pair_by_pair(&pts, distance, linked);
                    assert_eq!(near_point_groups(&pts, distance, linked), want, "linked {k} at {distance} for {pts:?}");
                }
            }
        }
    }
}

/// A tight cluster of `n` points around `at`, no two at the same place.
fn cluster(at: Point, radius: f64, n: usize) -> Vec<Point> {
    (0..n).map(|i| at + Vec2::from_angle(i as f64 * 2.399_963) * (radius * ((i + 1) as f64 / n as f64).sqrt())).collect()
}

#[test]
fn near_points_in_a_dense_cell_group_in_linear_work() {
    let n = 4_000;
    let neighbours = |a: usize, b: usize| b == a + 1;
    let one_cluster = cluster(Point::new(0.005, 0.005), 0.004, n);
    let mut two_clusters = cluster(Point::new(0.001, 0.001), 0.0009, n / 2);
    two_clusters.extend(cluster(Point::new(0.019, 0.019), 0.0009, n / 2));
    let same_place = vec![Point::new(0.005, 0.005); n];
    let cases = [(&one_cluster, 1, n), (&two_clusters, 2, n / 2), (&same_place, 0, 0)];
    for (pts, count, size) in cases {
        let groups = near_point_groups(pts, 0.01, neighbours);
        let work = super::NEAR_WORK.with(std::cell::Cell::get);
        assert_eq!(groups, near_groups_pair_by_pair(pts, 0.01, neighbours));
        assert_eq!(groups.len(), count);
        assert!(groups.iter().all(|g| g.len() == size), "{:?}", groups.iter().map(Vec::len).collect::<Vec<_>>());
        assert!(work <= 4 * n, "{n} points took {work} steps");
    }
}

/// One pair of ends (`a` < `b` in the list of ends) for [`rejoin_sorting_every_pair`]: the turn,
/// the distance, and the two ends.
type Pair = (f64, f64, End, End);

/// Rejoin listing every pair of ends in touching grid cells and sorting them by turn and then
/// distance: the joins [`rejoin`] makes.
fn rejoin_sorting_every_pair(pieces: &[SubPath], gap: f64) -> Vec<Rejoined> {
    if !(gap >= 0.0 && gap.is_finite()) {
        return vec![];
    }
    let usable = |sp: &&SubPath| {
        !sp.closed && sp.anchors.len() >= 2 && sp.anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite())
    };
    let ends: Vec<(End, Point, Vec2)> = pieces
        .iter()
        .enumerate()
        .filter(|(_, sp)| usable(sp))
        .flat_map(|(i, sp)| [(i, sp, false), (i, sp, true)])
        .filter_map(|(i, sp, last)| {
            let a = if last { sp.anchors.last() } else { sp.anchors.first() }?;
            Some(((i, last), a.p, outward(sp, last)?))
        })
        .collect();
    let size = gap.max(1e-9);
    let cell = |p: Point| ((p.x / size).floor() as i64, (p.y / size).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, e) in ends.iter().enumerate() {
        grid.entry(cell(e.1)).or_default().push(i);
    }
    let mut pairs: Vec<Pair> = vec![];
    for (a, &(ea, pa, oa)) in ends.iter().enumerate() {
        let (cx, cy) = cell(pa);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(others) = grid.get(&(cx.saturating_add(dx), cy.saturating_add(dy))) else { continue };
                for &b in others.iter().filter(|&&b| b > a) {
                    let (eb, pb, ob) = ends[b];
                    if ea.0 == eb.0 && pieces[ea.0].anchors.len() < 3 {
                        continue;
                    }
                    let d = pa.distance(pb);
                    if d <= gap {
                        pairs.push((oa.dot(-ob).clamp(-1.0, 1.0).acos(), d, ea, eb));
                    }
                }
            }
        }
    }
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)));
    let mut link: HashMap<End, End> = HashMap::new();
    for (_, _, a, b) in pairs {
        if !link.contains_key(&a) && !link.contains_key(&b) {
            link.insert(a, b);
            link.insert(b, a);
        }
    }
    chains(pieces, &link)
}

/// Check that [`rejoin`] joins `pieces` as [`rejoin_sorting_every_pair`] does, bit for bit.
fn same_as_sorting_every_pair(pieces: &[SubPath], gap: f64) {
    let (got, want) = (rejoin(pieces, gap), rejoin_sorting_every_pair(pieces, gap));
    let shape = |out: &[Rejoined]| out.iter().map(|j| (j.members.clone(), bits(&j.path))).collect::<Vec<_>>();
    assert_eq!(shape(&got), shape(&want), "gap {gap} for {pieces:?}");
}

/// Pieces of two to four anchors whose ends sit on or near a few lattice points, often at the
/// same place, some curving, closed or non-finite.
fn generated_pieces(r: &mut Rng, n: usize, gap: f64) -> Vec<SubPath> {
    let spot = |r: &mut Rng| {
        let near = r.pick(&[0.0, 0.0, 0.0, 0.25, 0.5, 1.0, 1.5]) * gap;
        Point::new((r.next() % 3) as f64 + near, (r.next() % 3) as f64 + r.pick(&[0.0, 0.0, near, -near]))
    };
    (0..n)
        .map(|_| {
            let count = 2 + (r.next() % 3) as usize;
            let mut pts: Vec<Point> = (0..count).map(|_| spot(r)).collect();
            if r.next().is_multiple_of(30) {
                pts[0].x = f64::NAN;
            }
            let mut sp = SubPath::polyline(&pts, r.next().is_multiple_of(15));
            if r.next().is_multiple_of(4) {
                let a = &mut sp.anchors[0];
                a.h_out = a.p + r.pick(&[Vec2::new(0.3, 0.1), Vec2::new(0.0, -0.2), Vec2::new(-0.1, -0.1)]);
            }
            sp
        })
        .collect()
}

#[test]
fn rejoin_joins_as_when_every_pair_is_sorted() {
    let mut r = Rng(0xd1b5_4a32_d192_ed03);
    for n in (0..25).chain([40, 80, 160]) {
        for _ in 0..15 {
            for gap in [0.0, 0.05, 0.1, 0.5, 1.0, 3.0] {
                same_as_sorting_every_pair(&generated_pieces(&mut r, n, gap), gap);
            }
        }
    }
    // Ends meeting from directions that tie: a plus sign, a doubled star and stacked copies.
    let plus: Vec<SubPath> = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)].map(|(x, y)| line(&[(0.0, 0.0), (x, y)], false)).to_vec();
    let mut doubled = plus.clone();
    doubled.extend(plus.iter().map(|sp| {
        let mut sp = sp.clone();
        sp.reverse();
        sp
    }));
    let stacked = vec![line(&[(0.0, 0.0), (1.0, 0.0), (2.0, 1.0)], false); 7];
    for pieces in [&plus, &doubled, &stacked] {
        for gap in [0.0, 0.1, 5.0] {
            same_as_sorting_every_pair(pieces, gap);
        }
    }
}

#[test]
fn rejoin_pairs_coincident_ends_in_near_linear_work() {
    let n = 2_000;
    let ray = |angle: f64| {
        let tip = Vec2::from_angle(angle) * 100.0;
        line(&[(0.0, 0.0), (tip.x, tip.y)], false)
    };
    let star: Vec<SubPath> = (0..n).map(|i| ray(i as f64 * std::f64::consts::TAU / n as f64)).collect();
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let scattered: Vec<SubPath> = (0..n).map(|_| ray((r.next() % 1_000_000) as f64 * 1e-6 * std::f64::consts::TAU)).collect();
    let stacked = vec![line(&[(0.0, 0.0), (10.0, 0.0)], false); n];
    for (name, pieces) in [("star", &star), ("scattered", &scattered), ("stacked", &stacked)] {
        let out = rejoin(pieces, 0.1);
        let work = super::REJOIN_WORK.with(std::cell::Cell::get);
        let want = rejoin_sorting_every_pair(pieces, 0.1);
        assert_eq!(
            out.iter().map(|j| (j.members.clone(), bits(&j.path))).collect::<Vec<_>>(),
            want.iter().map(|j| (j.members.clone(), bits(&j.path))).collect::<Vec<_>>(),
            "{name}"
        );
        assert!(work <= 20 * n, "{name}: {n} pieces took {work} steps");
    }
}

#[test]
fn rejoin_welds_pieces_within_the_gap() {
    // Three pieces of one zig-zag, the middle one drawn backwards, plus one too far away.
    let pieces = [
        line(&[(0.0, 0.0), (10.0, 0.0)], false),
        line(&[(20.0, 10.0), (10.05, 0.0)], false),
        line(&[(20.0, 10.0), (30.0, 10.0)], false),
        line(&[(31.0, 10.0), (40.0, 10.0)], false),
    ];
    let out = rejoin(&pieces, 0.1);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].members, [0, 1, 2]);
    assert_eq!(points(&out[0].path), [(0.0, 0.0), (10.0, 0.0), (20.0, 10.0), (30.0, 10.0)]);
    assert!(!out[0].path.closed);
    // With a wider gap the last piece joins too.
    assert_eq!(rejoin(&pieces, 1.0)[0].members, [0, 1, 2, 3]);
    // Nothing within the gap: nothing joins.
    assert!(rejoin(&pieces[2..], 0.5).is_empty());
}

#[test]
fn rejoin_takes_the_straightest_continuation_and_closes_loops() {
    // Piece 0 ends at the origin; piece 1 goes straight on, piece 2 turns back.
    let pieces = [line(&[(-10.0, 0.0), (0.0, 0.0)], false), line(&[(0.0, 0.0), (0.0, 10.0)], false), line(&[(0.0, 0.0), (10.0, 0.0)], false)];
    let out = rejoin(&pieces, 0.1);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].members, [0, 2]);
    // Four sides of a square close into one path.
    let sides = [
        line(&[(0.0, 0.0), (10.0, 0.0)], false),
        line(&[(10.0, 0.0), (10.0, 10.0)], false),
        line(&[(0.0, 10.0), (10.0, 10.0)], false),
        line(&[(0.0, 10.0), (0.0, 0.0)], false),
    ];
    let out = rejoin(&sides, 0.1);
    assert_eq!(out.len(), 1);
    assert!(out[0].path.closed);
    assert_eq!(out[0].path.anchors.len(), 4);
    // Closed, single-anchor and non-finite pieces stay out; bad gaps join nothing.
    let odd = [line(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)], true), line(&[(1.0, 0.0)], false), line(&[(f64::NAN, 0.0), (0.0, 1.0)], false)];
    assert!(rejoin(&odd, 1.0).is_empty());
    assert!(rejoin(&sides, f64::NAN).is_empty() && rejoin(&sides, -1.0).is_empty());
}
