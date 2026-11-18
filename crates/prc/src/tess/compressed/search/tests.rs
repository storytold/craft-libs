#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::cell::Cell;

use super::*;

/// Edge statuses, points, slots, references and normal angles.
type Case<'a> = (&'a [i32], Vec<i64>, usize, &'a [u32], &'a [i32]);

/// Walks the whole mesh oriented from the first triangle, inverting each of `flips`, with no
/// retries: what a resumed [`Resume::attempt`] must equal.
fn fresh(a: &Arrays<'_>, n: &NormalArrays<'_>, flips: &[Choice]) -> Outcome {
    let mut w = Walk::new(a.triangles, a.limit, Some(NormalReader::new(n)));
    let after = flips.iter().map(|c| c.at() + 1).max().unwrap_or(0);
    let mut steps = Vec::new();
    for i in 0..a.triangles {
        let start = w.next.is_none();
        w.turn = flips.contains(&Choice::Turn(i));
        if w.step(a, a.edge_status[i], flips.contains(&Choice::Fold(i))).is_err() {
            let back = steps.iter().enumerate().rev();
            return Outcome::Failed(if start { Vec::new() } else { window(back.take_while(|&(j, _)| j >= after)) });
        }
        steps.push(Step::of(&w, start));
    }
    if !w.used_up(a) {
        return Outcome::Dead;
    }
    match w.normals.as_ref().and_then(|r| r.finished(&w.pos)) {
        Some(stored) if flat(a, &w.pos, &w.tris, 0) => Outcome::Fit(rebuilt(w.pos.clone(), w.tris.clone(), stored)),
        _ => Outcome::Dead,
    }
}

/// A comparable digest of an outcome.
fn digest(o: Outcome) -> (u8, Vec<Choice>, Vec<[u32; 3]>, Vec<V>) {
    match o {
        Outcome::Fit(m) => (0, Vec::new(), m.triangles, m.positions),
        Outcome::Failed(c) => (1, c, Vec::new(), Vec::new()),
        Outcome::Dead => (2, Vec::new(), Vec::new(), Vec::new()),
    }
}

/// A step whose fold is a candidate.
fn fold_step(signalled: bool) -> Step {
    Step { start: false, fold: true, signalled, turn: false }
}

/// One unsignalled fold followed by a run of signalled ones longer than the window: the window
/// keeps the unsignalled fold and the nearest [`SECONDARIES`] signalled ones, so a run of weak
/// triangles cannot hide the one sliver whose fold is wrong.
#[test]
fn a_run_of_signalled_folds_does_not_push_an_unsignalled_one_out() {
    let mut steps = vec![fold_step(false)];
    steps.extend((0..PRIMARIES + SECONDARIES).map(|_| fold_step(true)));
    let c = window(steps.iter().enumerate().rev());
    let n = steps.len();
    assert_eq!(c.len(), SECONDARIES + 1);
    assert_eq!(c[0], Choice::Fold(0));
    assert_eq!(c[1], Choice::Fold(n - SECONDARIES));
    assert_eq!(c[SECONDARIES], Choice::Fold(n - 1));
    let mut steps = vec![fold_step(false); PRIMARIES + 1];
    steps[0].start = true;
    assert_eq!(window(steps.iter().enumerate().rev()).len(), PRIMARIES);
    steps.truncate(3);
    assert_eq!(window(steps.iter().enumerate().rev()).len(), 3);
}

/// Every set of at most two choices over `t` triangles, in an order that moves the first
/// differing choice both ways between attempts.
fn flip_sets(t: usize) -> Vec<Vec<Choice>> {
    let all: Vec<Choice> = (0..t).flat_map(|i| [Choice::Fold(i), Choice::Turn(i)]).collect();
    let mut sets = vec![Vec::new()];
    for (k, &x) in all.iter().enumerate() {
        sets.push(vec![x]);
        for &y in all.iter().skip(k + 1) {
            sets.push(vec![x, y]);
        }
    }
    let back: Vec<_> = sets.iter().rev().cloned().collect();
    sets.extend(back);
    sets
}

/// A resumed attempt comes to exactly what a walk from the first triangle does, whatever
/// attempt preceded it, with a snapshot at every triangle, every [`SNAP_EVERY`], or (memory
/// short) only the first.
#[test]
fn a_resumed_attempt_matches_a_fresh_walk() {
    let tri0 = [0, 0, 0, 4, 0, 0, 2, 4, 0];
    let cases: [Case<'_>; 2] = [
        (&[1, 3, 1, 1], [&tri0[..], &[0, 0, 0, -4, -4, 2]].concat(), 6, &[2], &[470, 724]),
        (&[1, 3, 2, 1, 2], [&tri0[..], &[-4, 0, 0, 3, 3, 3, -1, 0, 0]].concat(), 7, &[0], &[837, 402]),
    ];
    let steps = Cell::new(u64::MAX);
    for (status, pts, slots, refs, angles) in cases {
        let t = status.len();
        let mut is_ref = vec![false; slots];
        is_ref[5] = true;
        let face_of = vec![0; t];
        for limit in [usize::MAX, 400] {
            let a = Arrays {
                tolerance: 0.5,
                origin: [10.0, 0.0, 0.0],
                points: &pts,
                edge_status: status,
                triangles: t,
                is_reference: &is_ref,
                references: refs,
                normals: Some(NormalArrays { bits: 10, binary: &[false; 4], angles, planar: &[true], face_of: &face_of }),
                ortho_turned: false,
                limit,
                steps: &steps,
            };
            let n = a.normals.as_ref().unwrap();
            for every in [1, SNAP_EVERY] {
                let mut r = Resume::new(&a, n, every);
                let mut kinds = [0; 3];
                for flips in flip_sets(t) {
                    let got = digest(r.attempt(&a, &flips));
                    let want = digest(fresh(&a, n, &flips));
                    kinds[usize::from(want.0)] += 1;
                    assert!(got == want, "flips {:?}", flips.iter().map(|c| c.at()).collect::<Vec<_>>());
                }
                assert!(kinds[0] > 0 && kinds[1] > 0, "{kinds:?}");
            }
        }
    }
}

/// The same on seeded arbitrary arrays, valid or not, whose walks branch, pop stacked edges
/// and start new components.
#[test]
fn a_resumed_attempt_matches_a_fresh_walk_on_arbitrary_arrays() {
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = |m: u64| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % m
    };
    let steps = Cell::new(u64::MAX);
    let mut kinds = [0; 3];
    for _ in 0..300 {
        let t = 6 + usize::try_from(next(5)).unwrap();
        let status: Vec<i32> = (0..3 * t).map(|_| i32::try_from(next(4)).unwrap()).collect();
        let pts: Vec<i64> = (0..6 * t).map(|_| i64::try_from(next(9)).unwrap() - 4).collect();
        let is_ref: Vec<bool> = (0..3 * t).map(|_| next(3) == 0).collect();
        let refs: Vec<u32> = (0..t).map(|_| u32::try_from(next(6)).unwrap()).collect();
        let angles: Vec<i32> = (0..2 * t).map(|_| i32::try_from(next(1024)).unwrap()).collect();
        let face_of = vec![0; t];
        let a = Arrays {
            tolerance: 0.5,
            origin: [0.0; 3],
            points: &pts,
            edge_status: &status,
            triangles: t,
            is_reference: &is_ref,
            references: &refs,
            normals: Some(NormalArrays { bits: 10, binary: &[false; 4], angles: &angles, planar: &[true], face_of: &face_of }),
            ortho_turned: false,
            limit: usize::MAX,
            steps: &steps,
        };
        let n = a.normals.as_ref().unwrap();
        let mut r = Resume::new(&a, n, 1);
        for flips in flip_sets(t) {
            let got = digest(r.attempt(&a, &flips));
            let want = digest(fresh(&a, n, &flips));
            kinds[usize::from(want.0)] += 1;
            assert!(got == want);
        }
    }
    assert!(kinds[1] > 0 && kinds[2] > 0, "{kinds:?}");
}

/// A search that runs out of the stream's steps stops undecided, before its own budget.
#[test]
fn a_search_stops_when_the_stream_steps_run_out() {
    let tri0 = [0, 0, 0, 4, 0, 0, 2, 4, 0];
    let pts = [&tri0[..], &[0, 0, 0, -4, -4, 2]].concat();
    let status = [1, 3, 1, 1, 1, 3, 0, 0, 1, 3, 2, 0];
    let mut is_ref = vec![false; 6];
    is_ref[5] = true;
    let steps = Cell::new(u64::MAX);
    let a = Arrays {
        tolerance: 0.5,
        origin: [10.0, 0.0, 0.0],
        points: &pts,
        edge_status: &status,
        triangles: 4,
        is_reference: &is_ref,
        references: &[2],
        normals: Some(NormalArrays { bits: 10, binary: &[false; 4], angles: &[470, 724], planar: &[true], face_of: &[0; 4] }),
        ortho_turned: false,
        limit: usize::MAX,
        steps: &steps,
    };
    assert!(unique_fit_reading(&a).0.is_some());
    let spent = u64::MAX - steps.get();
    assert!(spent > 4);
    steps.set(spent - 1);
    assert!(unique_fit_reading(&a).0.is_none());
    assert_eq!(steps.get(), 0);
}

/// A finished planar face is judged against the plane through its centroid: two triangles
/// folded along their shared edge are refused, the same pair laid flat is kept, and a face not
/// flagged planar is not judged.
#[test]
fn a_finished_planar_face_must_lie_in_one_plane() {
    let steps = Cell::new(u64::MAX);
    let tris = [[0, 1, 2], [2, 1, 3]];
    let judge = |apex: V, planar: bool| {
        let a = Arrays {
            tolerance: 0.5,
            origin: [0.0; 3],
            points: &[],
            edge_status: &[],
            triangles: 2,
            is_reference: &[],
            references: &[],
            normals: Some(NormalArrays { bits: 10, binary: &[], angles: &[], planar: &[planar], face_of: &[0, 0] }),
            ortho_turned: false,
            limit: usize::MAX,
            steps: &steps,
        };
        flat(&a, &[[0.0; 3], [80.0, 0.0, 0.0], [40.0, 80.0, 0.0], apex], &tris, 0)
    };
    assert!(!judge([120.0, 80.0, 60.0], true));
    assert!(judge([120.0, 80.0, 0.0], true));
    assert!(judge([120.0, 80.0, 60.0], false));
}

/// A first reading that refused two fits stays refused: the turned reading is not searched,
/// and its unique fit is not returned. Only a true no-fit hands over to the turned reading.
#[test]
fn a_refused_reading_is_not_answered_by_the_turned_one() {
    let mesh = || Box::new(rebuilt(vec![[0.0; 3]; 3], vec![[0, 1, 2]], (Vec::new(), Vec::new())));
    let searched = Cell::new(false);
    let turned = || {
        searched.set(true);
        Search::Unique(mesh())
    };
    assert!(then_turned((Search::Refused, true), turned).is_none());
    assert!(!searched.get());
    assert!(then_turned((Search::NoFit, false), turned).is_none());
    assert!(!searched.get());
    assert!(then_turned((Search::NoFit, true), turned).is_some());
    assert!(searched.get());
}

/// A search whose budget runs out before any fit, with combinations left, is exhausted, not a
/// no-fit, and the turned reading is not searched after it. The same search given room to
/// finish settles one way or the other.
#[test]
fn a_search_cut_short_before_any_fit_is_exhausted() {
    let tri0 = [0, 0, 0, 4, 0, 0, 2, 4, 0];
    let pts = [&tri0[..], &[-4, 0, 0, 3, 3, 3, -1, 0, 0]].concat();
    let status = [1, 3, 2, 1, 2];
    let mut is_ref = vec![false; 7];
    is_ref[5] = true;
    let steps = Cell::new(u64::MAX);
    let a = Arrays {
        tolerance: 0.5,
        origin: [10.0, 0.0, 0.0],
        points: &pts,
        edge_status: &status,
        triangles: status.len(),
        is_reference: &is_ref,
        references: &[0],
        normals: Some(NormalArrays { bits: 10, binary: &[false; 4], angles: &[837, 402], planar: &[true], face_of: &[0; 5] }),
        ortho_turned: false,
        limit: usize::MAX,
        steps: &steps,
    };
    let n = a.normals.as_ref().unwrap();
    assert!(matches!(fresh(&a, n, &[]), Outcome::Failed(c) if !c.is_empty()));
    let plan = |budget| Plan { turns: true, depth: 2, budget };
    assert!(matches!(search_reading(&a, n, plan(1)).0, Search::Exhausted));
    assert!(!matches!(search_reading(&a, n, plan(STEP_BUDGET)).0, Search::Exhausted));
    let searched = Cell::new(false);
    let turned = || {
        searched.set(true);
        Search::NoFit
    };
    assert!(then_turned((Search::Exhausted, true), turned).is_none());
    assert!(!searched.get());
}
