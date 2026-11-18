//! Divide into Dashes: a subpath (or some of its segments) cut into open pieces of a given length
//! measured along the curve, with gaps of a given length between them.

use std::collections::BTreeSet;

use craft_geom::{Anchor, AnchorKind, SubPath};
use kurbo::{CubicBez, ParamCurve, ParamCurveArclen};

use super::{ACCURACY, TINY};

/// What a piece [`divide_dashes`] cuts is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    Dash,
    /// The stretch between two dashes.
    Gap,
    /// A stretch of segments that weren't to be divided, as it was.
    Rest,
}

/// How [`divide_dashes`] measures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DashSpec {
    /// Each dash's length (pt), more than 0.
    pub dash: f64,
    /// Each gap's length (pt), 0 or more (0: the dashes touch).
    pub gap: f64,
    /// Fit to corners and ends: every run between corners and ends holds a whole number of dashes
    /// (and the gaps between them), all stretched or squeezed alike so it starts and ends on a
    /// dash; a closed subpath with no corner holds whole periods. Otherwise the lengths are exact
    /// and the last piece of each run is what is left.
    pub fit: bool,
}

/// `sp` cut into open pieces in order along it: the segments `segments` picks (all when `None`)
/// cut into dashes and gaps as `spec` says, the dash pattern running on across the picked
/// segments that follow one another, and each stretch of segments not picked as one piece. `None`
/// when that would make more than `room` pieces, or `spec` is no pattern (lengths not finite, a
/// dash not more than 0, a negative gap).
pub fn divide_dashes(sp: &SubPath, segments: Option<&BTreeSet<usize>>, spec: DashSpec, room: usize) -> Option<Vec<(SubPath, Piece)>> {
    let DashSpec { dash, gap, fit } = spec;
    if !(dash.is_finite() && gap.is_finite() && dash > TINY && gap >= 0.0) {
        return None;
    }
    let n = sp.segment_count();
    let picked = |k: usize| segments.is_none_or(|s| s.contains(&k));
    if !(0..n).any(picked) {
        return Some(vec![(sp.clone(), Piece::Rest)]);
    }
    // An anchor between two segments where the outline turns: a corner a fitted run ends at.
    let corner = |i: usize| sp.anchors.get(i).is_some_and(|a| Anchor::with_handles(a.p, a.h_in, a.h_out).kind != AnchorKind::Smooth);
    // A closed subpath starts where picked and unpicked segments meet, else (fitting) at a corner.
    let start = if sp.closed {
        (0..n).find(|&k| picked(k) != picked((k + n - 1) % n)).or_else(|| (0..n).find(|&k| fit && corner(k))).unwrap_or(0)
    } else {
        0
    };
    // Runs of segments alike; a fitted picked run also ends at each corner.
    let mut runs: Vec<(bool, Vec<usize>)> = vec![];
    for k in (0..n).map(|i| (start + i) % n) {
        let p = picked(k);
        match runs.last_mut() {
            Some((q, segs)) if *q == p && !(p && fit && corner(k)) => segs.push(k),
            _ => runs.push((p, vec![k])),
        }
    }
    let whole_loop = sp.closed && runs.len() == 1 && !(fit && corner(start));
    let mut out = vec![];
    for (p, segs) in runs {
        let curves: Vec<CubicBez> = segs.iter().map(|&k| sp.segment(k)).collect();
        if !p {
            out.extend(from_cubics(&curves).map(|s| (s, Piece::Rest)));
            continue;
        }
        let lens: Vec<f64> = curves.iter().map(|c| c.arclen(ACCURACY)).collect();
        let len: f64 = lens.iter().sum();
        if !(len.is_finite() && len > TINY) {
            continue;
        }
        let cuts = spans(len, spec, whole_loop, room.checked_sub(out.len())?)?;
        let mut cursor = 0;
        let mut from = 0.0;
        let starts: Vec<f64> = lens
            .iter()
            .map(|l| {
                let s = from;
                from += l;
                s
            })
            .collect();
        for (a, b, kind) in cuts {
            let mut piece = vec![];
            let mut j = cursor;
            while let (Some(&s0), Some(&l), Some(c)) = (starts.get(j), lens.get(j), curves.get(j)) {
                if s0 >= b - TINY {
                    break;
                }
                let (t0, t1) = ((a - s0).max(0.0), (b - s0).min(l));
                if t1 - t0 > TINY {
                    let at = |d: f64| {
                        if d <= 0.0 {
                            0.0
                        } else if d >= l {
                            1.0
                        } else {
                            c.inv_arclen(d, ACCURACY)
                        }
                    };
                    piece.push(c.subsegment(at(t0)..at(t1)));
                }
                if s0 + l > b + TINY {
                    break;
                }
                j += 1;
            }
            cursor = j;
            out.extend(from_cubics(&piece).map(|s| (s, kind)));
        }
    }
    Some(out)
}

/// The pieces (start, end along the run, kind) of a run `len` long; `None` past `room` pieces.
fn spans(len: f64, DashSpec { dash, gap, fit }: DashSpec, whole_loop: bool, room: usize) -> Option<Vec<(f64, f64, Piece)>> {
    let period = dash + gap;
    // A loop holds whole periods; an open run one gap fewer than dashes.
    let scale = if !fit {
        1.0
    } else if whole_loop {
        len / ((len / period).round().max(1.0) * period)
    } else {
        let count = ((len + gap) / period).round().max(1.0);
        len / (count * dash + (count - 1.0) * gap)
    };
    let (dash, gap) = (dash * scale, gap * scale);
    // Rounding (of the length measured, over many pieces): what is left past `end` is no piece.
    let end = len - (len * 1e-9).max(TINY);
    let most = (end / (dash + gap)).ceil() * if gap > 0.0 { 2.0 } else { 1.0 };
    if !(most.is_finite() && most <= room as f64) {
        return None;
    }
    let mut out = vec![];
    let (mut at, mut on) = (0.0, true);
    while at < end {
        let step = if on { dash } else { gap };
        if step > 0.0 {
            let to = (at + step).min(len);
            out.push((at, if to >= end { len } else { to }, if on { Piece::Dash } else { Piece::Gap }));
            at = to;
        }
        on = !on;
    }
    Some(out)
}

/// An open subpath through cubic segments that follow one another; `None` for none.
fn from_cubics(cs: &[CubicBez]) -> Option<SubPath> {
    let (first, last) = (cs.first()?, cs.last()?);
    let mut anchors = vec![Anchor::with_handles(first.p0, first.p0, first.p1)];
    anchors.extend(cs.windows(2).filter_map(|w| match w {
        [a, b] => Some(Anchor::with_handles(b.p0, a.p2, b.p1)),
        _ => None,
    }));
    anchors.push(Anchor::with_handles(last.p3, last.p2, last.p3));
    Some(SubPath::new(anchors, false))
}

#[cfg(test)]
mod tests {
    use kurbo::Point;

    use super::*;

    fn line(len: f64) -> SubPath {
        SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(len, 0.0)], false)
    }

    fn length(sp: &SubPath) -> f64 {
        (0..sp.segment_count()).map(|k| sp.segment(k).arclen(ACCURACY)).sum()
    }

    fn lengths(pieces: &[(SubPath, Piece)]) -> Vec<(i64, Piece)> {
        pieces.iter().map(|(s, k)| ((length(s) * 1000.0).round() as i64, *k)).collect()
    }

    const EXACT: DashSpec = DashSpec { dash: 10.0, gap: 0.0, fit: false };

    #[test]
    fn a_line_is_cut_into_dashes_and_gaps() {
        let p = divide_dashes(&line(35.0), None, EXACT, 100).unwrap();
        assert_eq!(lengths(&p), [(10_000, Piece::Dash), (10_000, Piece::Dash), (10_000, Piece::Dash), (5_000, Piece::Dash)]);
        assert_eq!(p[0].0.anchors[1].p, p[1].0.anchors[0].p, "touching");
        assert!(p[1].0.anchors[0].p.distance(Point::new(10.0, 0.0)) < 1e-6);
        let p = divide_dashes(&line(35.0), None, DashSpec { gap: 5.0, ..EXACT }, 100).unwrap();
        let kinds: Vec<Piece> = p.iter().map(|(_, k)| *k).collect();
        assert_eq!(kinds, [Piece::Dash, Piece::Gap, Piece::Dash, Piece::Gap, Piece::Dash]);
        assert_eq!(lengths(&p)[4], (5_000, Piece::Dash));
    }

    #[test]
    fn fitting_stretches_the_pattern_to_start_and_end_on_a_dash() {
        // 35 with 10 + 5: three dashes and two gaps, squeezed from 40 to 35.
        let p = divide_dashes(&line(35.0), None, DashSpec { dash: 10.0, gap: 5.0, fit: true }, 100).unwrap();
        let (d, g) = ((8_750, Piece::Dash), (4_375, Piece::Gap));
        assert_eq!(lengths(&p), [d, g, d, g, d]);
        // A square: each side its own run, each starting and ending on a dash.
        let sq = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(30.0, 0.0), Point::new(30.0, 30.0), Point::new(0.0, 30.0)], true);
        let p = divide_dashes(&sq, None, DashSpec { dash: 7.0, gap: 0.0, fit: true }, 100).unwrap();
        assert_eq!(p.len(), 16, "four dashes of 7.5 on each side");
        assert!(lengths(&p).iter().all(|(l, k)| *l == 7_500 && *k == Piece::Dash));
    }

    #[test]
    fn a_curve_is_measured_along_it() {
        let arc = SubPath::new(
            vec![
                Anchor::with_handles(Point::new(0.0, 0.0), Point::new(0.0, 0.0), Point::new(0.0, 55.0)),
                Anchor::with_handles(Point::new(100.0, 100.0), Point::new(45.0, 100.0), Point::new(100.0, 100.0)),
            ],
            false,
        );
        let total = length(&arc);
        let p = divide_dashes(&arc, None, EXACT, 1000).unwrap();
        assert_eq!(p.len(), (total / 10.0).ceil() as usize);
        assert!(p.iter().rev().skip(1).all(|(s, _)| (length(s) - 10.0).abs() < 1e-4));
        assert!((p.iter().map(|(s, _)| length(s)).sum::<f64>() - total).abs() < 1e-4);
    }

    #[test]
    fn only_the_picked_segments_are_divided() {
        // Three sides of 20; the middle one picked.
        let u = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(20.0, 20.0), Point::new(0.0, 20.0)], false);
        let p = divide_dashes(&u, Some(&BTreeSet::from([1])), EXACT, 100).unwrap();
        assert_eq!(lengths(&p), [(20_000, Piece::Rest), (10_000, Piece::Dash), (10_000, Piece::Dash), (20_000, Piece::Rest)]);
        // Closed: the unpicked rest is one piece round the start.
        let sq = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(20.0, 20.0), Point::new(0.0, 20.0)], true);
        let p = divide_dashes(&sq, Some(&BTreeSet::from([1])), EXACT, 100).unwrap();
        assert_eq!(lengths(&p), [(10_000, Piece::Dash), (10_000, Piece::Dash), (60_000, Piece::Rest)]);
        assert!(p.iter().all(|(s, _)| !s.closed));
    }

    #[test]
    fn too_many_pieces_or_no_pattern_is_none() {
        assert!(divide_dashes(&line(1000.0), None, EXACT, 99).is_none());
        assert!(divide_dashes(&line(1000.0), None, EXACT, 100).is_some());
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(divide_dashes(&line(10.0), None, DashSpec { dash: bad, ..EXACT }, 100).is_none());
        }
        assert!(divide_dashes(&line(10.0), None, DashSpec { gap: -1.0, ..EXACT }, 100).is_none());
        assert!(divide_dashes(&line(1e6), None, DashSpec { dash: 1e-6, ..EXACT }, 100_000).is_none());
    }
}
