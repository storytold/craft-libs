//! Polylines cut by a region: each segment is split where it crosses the region's edges, and the
//! pieces whose middle lies inside are kept, joined where they meet.

use kurbo::{Point, Rect};

use crate::hpgl::plot::FillRule;

/// A clipping region: closed rings (each closes implicitly) and the rule that decides inside.
pub struct Region {
    rings: Vec<Vec<Point>>,
    rule: FillRule,
    bounds: Rect,
}

impl Region {
    /// The region of `rings` under `rule`; `None` when it has no ring of three points or more.
    pub fn new(rings: Vec<Vec<Point>>, rule: FillRule) -> Option<Self> {
        let rings: Vec<Vec<Point>> = rings.into_iter().filter(|r| r.len() >= 3).collect();
        let bounds = rings.iter().flatten().fold(None, |b: Option<Rect>, p| Some(b.map_or(Rect::from_points(*p, *p), |b| b.union_pt(*p))))?;
        Some(Self { rings, rule, bounds })
    }

    /// The edges this region has, which [`Region::cut`] tests each segment against.
    fn edges(&self) -> u64 {
        self.rings.iter().map(|r| r.len() as u64).sum()
    }

    /// Does `p` lie on one of the region's edges (within a millionth of a point)?
    fn on_edge(&self, p: Point) -> bool {
        self.rings.iter().any(|ring| {
            ring.iter().enumerate().any(|(i, &a)| {
                let b = ring[(i + 1) % ring.len()];
                let ab = b - a;
                let len2 = ab.hypot2();
                let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
                (a + ab * t).distance(p) <= 1e-6
            })
        })
    }

    /// Is `p` inside (non-zero winding or an odd number of crossings), or on an edge?
    fn contains(&self, p: Point) -> bool {
        if self.on_edge(p) {
            return true;
        }
        let mut winding = 0i64;
        for ring in &self.rings {
            for (i, &a) in ring.iter().enumerate() {
                let b = ring[(i + 1) % ring.len()];
                if a.y <= p.y {
                    if b.y > p.y && cross(a, b, p) > 0.0 {
                        winding += 1;
                    }
                } else if b.y <= p.y && cross(a, b, p) < 0.0 {
                    winding -= 1;
                }
            }
        }
        match self.rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        }
    }

    /// Where segment `a`–`b` crosses this region's edges, as fractions of its length strictly
    /// between 0 and 1, sorted.
    fn crossings(&self, a: Point, b: Point) -> Vec<f64> {
        let mut ts = vec![];
        for ring in &self.rings {
            for (i, &c) in ring.iter().enumerate() {
                let d = ring[(i + 1) % ring.len()];
                if let Some(t) = intersect(a, b, c, d)
                    && t > 1e-12
                    && t < 1.0 - 1e-12
                {
                    ts.push(t);
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() <= 1e-12);
        ts
    }

    /// The parts of polyline `line` inside the region, each a polyline of two points or more.
    /// Every test of a point or a segment against an edge takes one from `budget`; `None` when
    /// the budget runs out first.
    pub fn cut(&self, line: &[Point], budget: &mut u64) -> Option<Vec<Vec<Point>>> {
        let edges = self.edges();
        let mut out: Vec<Vec<Point>> = vec![];
        let mut cur: Vec<Point> = vec![];
        for w in line.windows(2) {
            let (a, b) = (w[0], w[1]);
            // A segment wholly off the region's box lies outside it.
            let near = overlaps(Rect::from_points(a, b), self.bounds);
            let ts = if near {
                spend(budget, edges)?;
                self.crossings(a, b)
            } else {
                vec![]
            };
            let mut from = 0.0;
            for to in ts.into_iter().chain([1.0]) {
                let (p, q) = (a.lerp(b, from), a.lerp(b, to));
                // Each piece between crossings is tested on its own: its edge and winding tests.
                let inside = if near {
                    spend(budget, edges.saturating_mul(2))?;
                    self.contains(p.midpoint(q))
                } else {
                    false
                };
                if inside {
                    if cur.last().is_none_or(|l| l.distance(p) > 1e-9) {
                        flush(&mut out, &mut cur);
                        cur.push(p);
                    }
                    cur.push(q);
                } else {
                    flush(&mut out, &mut cur);
                }
                from = to;
            }
        }
        flush(&mut out, &mut cur);
        // A closed line that leaves the region and comes back: its last piece ends where its
        // first starts, at the line's start, and the two are one piece.
        if line.len() > 2
            && line.first() == line.last()
            && out.len() >= 2
            && let (Some(end), Some(start)) = (out.last().and_then(|l| l.last()), out.first().and_then(|l| l.first()))
            && end.distance(*start) <= 1e-9
        {
            let first = out.remove(0);
            if let Some(last) = out.last_mut() {
                last.extend(first.into_iter().skip(1));
            }
        }
        Some(out)
    }
}

/// Take `n` from `budget`; `None` when it holds less.
fn spend(budget: &mut u64, n: u64) -> Option<()> {
    *budget = budget.checked_sub(n)?;
    Some(())
}

fn flush(out: &mut Vec<Vec<Point>>, cur: &mut Vec<Point>) {
    let line = std::mem::take(cur);
    if line.len() >= 2 {
        out.push(line);
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Which side of line `a`–`b` point `p` lies on (positive: left).
fn cross(a: Point, b: Point, p: Point) -> f64 {
    (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y)
}

/// Where segment `a`–`b` crosses segment `c`–`d`, as a fraction of `a`–`b`; `None` when they
/// are parallel or don't meet.
fn intersect(a: Point, b: Point, c: Point, d: Point) -> Option<f64> {
    let (r, s) = (b - a, d - c);
    let den = r.x * s.y - r.y * s.x;
    if den.abs() <= f64::EPSILON * (r.hypot() * s.hypot()).max(f64::MIN_POSITIVE) {
        return None;
    }
    let ac = c - a;
    let t = (ac.x * s.y - ac.y * s.x) / den;
    let u = (ac.x * r.y - ac.y * r.x) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
        vec![Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)]
    }

    /// `line` cut by `r` with no limit on the work.
    fn cut(r: &Region, line: &[Point]) -> Vec<Vec<Point>> {
        let mut unlimited = u64::MAX;
        r.cut(line, &mut unlimited).unwrap()
    }

    #[test]
    fn a_line_across_a_square_keeps_its_middle() {
        let r = Region::new(vec![square(0.0, 0.0, 10.0, 10.0)], FillRule::NonZero).unwrap();
        let pieces = cut(&r, &[Point::new(-5.0, 5.0), Point::new(15.0, 5.0)]);
        assert_eq!(pieces, vec![vec![Point::new(0.0, 5.0), Point::new(10.0, 5.0)]]);
        // Wholly outside, nothing; wholly inside, the line as it is.
        assert!(cut(&r, &[Point::new(20.0, 0.0), Point::new(30.0, 0.0)]).is_empty());
        let inside = vec![Point::new(1.0, 1.0), Point::new(9.0, 1.0), Point::new(9.0, 9.0)];
        assert_eq!(cut(&r, &inside), vec![inside]);
    }

    /// Each piece between crossings costs a pass over the edges: a line through the teeth of a
    /// comb spends far more than its two points times the edges.
    #[test]
    fn every_piece_between_crossings_is_paid_for() {
        let mut ring = vec![];
        for i in 0..100 {
            let x = f64::from(i) * 4.0;
            ring.extend([Point::new(x, 0.0), Point::new(x + 2.0, 0.0), Point::new(x + 2.0, 10.0), Point::new(x + 4.0, 10.0)]);
        }
        ring.extend([Point::new(400.0, 20.0), Point::new(0.0, 20.0)]);
        let r = Region::new(vec![ring], FillRule::NonZero).unwrap();
        let line = [Point::new(-1.0, 5.0), Point::new(401.0, 5.0)];
        // 402 edges, 200 crossings, 201 pieces: 402 + 201 × 2 × 402 tests.
        let need = 402 + 201 * 2 * 402;
        let mut budget = need;
        assert_eq!(r.cut(&line, &mut budget).map(|p| p.len()), Some(100));
        assert_eq!(budget, 0);
        let mut short = need - 1;
        assert!(r.cut(&line, &mut short).is_none());
        // A segment off the region's box costs nothing.
        assert_eq!(r.cut(&[Point::new(0.0, 30.0), Point::new(9.0, 30.0)], &mut 0), Some(vec![]));
    }

    /// A line along any of the region's edges is inside it.
    #[test]
    fn a_line_on_an_edge_is_kept() {
        let r = Region::new(vec![square(0.0, 0.0, 10.0, 10.0)], FillRule::NonZero).unwrap();
        for line in [
            [Point::new(0.0, 0.0), Point::new(10.0, 0.0)],
            [Point::new(10.0, 0.0), Point::new(10.0, 10.0)],
            [Point::new(10.0, 10.0), Point::new(0.0, 10.0)],
            [Point::new(0.0, 10.0), Point::new(0.0, 0.0)],
        ] {
            assert_eq!(cut(&r, &line), vec![line.to_vec()], "{line:?}");
        }
    }

    #[test]
    fn a_line_leaving_and_coming_back_is_two_pieces() {
        let r = Region::new(vec![square(0.0, 0.0, 10.0, 10.0)], FillRule::NonZero).unwrap();
        let line = [Point::new(5.0, 5.0), Point::new(15.0, 5.0), Point::new(15.0, 8.0), Point::new(5.0, 8.0)];
        assert_eq!(cut(&r, &line), vec![vec![Point::new(5.0, 5.0), Point::new(10.0, 5.0)], vec![Point::new(10.0, 8.0), Point::new(5.0, 8.0)]]);
    }

    /// A closed line that starts inside, leaves and comes back is one piece: the part after it
    /// comes back runs on into the part before it left.
    #[test]
    fn a_closed_line_cut_once_stays_one_piece() {
        let r = Region::new(vec![square(0.0, 0.0, 10.0, 10.0)], FillRule::NonZero).unwrap();
        let line = [Point::new(5.0, 5.0), Point::new(15.0, 5.0), Point::new(15.0, 8.0), Point::new(5.0, 8.0), Point::new(5.0, 5.0)];
        assert_eq!(cut(&r, &line), vec![vec![Point::new(10.0, 8.0), Point::new(5.0, 8.0), Point::new(5.0, 5.0), Point::new(10.0, 5.0)]]);
    }

    #[test]
    fn the_fill_rule_decides_a_hole() {
        let rings = vec![square(0.0, 0.0, 10.0, 10.0), square(3.0, 3.0, 7.0, 7.0)];
        let line = [Point::new(-1.0, 5.0), Point::new(11.0, 5.0)];
        // Both rings wind the same way: even-odd cuts a hole, non-zero doesn't.
        let even_odd = cut(&Region::new(rings.clone(), FillRule::EvenOdd).unwrap(), &line);
        assert_eq!(even_odd, vec![vec![Point::new(0.0, 5.0), Point::new(3.0, 5.0)], vec![Point::new(7.0, 5.0), Point::new(10.0, 5.0)]]);
        let non_zero = cut(&Region::new(rings, FillRule::NonZero).unwrap(), &line);
        assert_eq!(non_zero.len(), 1);
        assert!(Region::new(vec![vec![Point::ZERO, Point::new(1.0, 1.0)]], FillRule::NonZero).is_none());
    }
}
