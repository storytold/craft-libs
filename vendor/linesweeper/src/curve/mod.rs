//! Curve comparison utilities.
use arrayvec::ArrayVec;
use kurbo::{CubicBez, Line, ParamCurve, PathSeg, Point, QuadBez, Shape, Vec2};
use polycool::{Cubic, Poly, Quadratic};
use smallvec::SmallVec;

pub mod line;
pub(crate) mod transverse;

#[derive(Clone, Debug, PartialEq)]
struct CurveEntry<T> {
    end: f64,
    order: T,
}

type CurveOrderEntry = CurveEntry<Order>;

impl CurveOrderEntry {
    fn flip(&self) -> Self {
        Self {
            end: self.end,
            order: match self.order {
                Order::Right => Order::Left,
                Order::Ish => Order::Ish,
                Order::Left => Order::Right,
            },
        }
    }
}

/// Observability for the curve comparison algorithm, recording recursion
/// depth by y-interval.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Metrics {
    /// Each interval records a single recursion depth measurement.
    pub entries: Vec<MetricsEntry>,
}

/// A single observability entry.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricsEntry {
    /// The y endpoint of this entry. (The start point is the endpoint
    /// of the previous entry.)
    pub end: f64,
    /// The recursion depth of this entry, starting from zero.
    pub depth: u32,
}

impl MetricsEntry {
    fn new(end: f64, status: MetricsStatus) -> Self {
        Self {
            end,
            depth: status.depth,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
struct MetricsStatus {
    depth: u32,
}

impl MetricsStatus {
    fn bump(&self) -> Self {
        Self {
            depth: self.depth + 1,
        }
    }
}

/// The outcome of analyzing two curves for horizontal ordering.
///
/// We assume that the two curves are monotonic in y, and we partition their
/// common y extent into sub-intervals, each of which gets assigned an order.
///
/// For example, consider the curves c0 and c1 below.
///
/// ```text
///       c0     c1
/// y=0    \      /
///         \    /
///          |  /
/// y=1     ─┼─
///        / │
///       /  │
/// y=2  /   │
/// ```
///
/// They start out with c0 to the left, cross over, and end up with c0 to the
/// right. We might represent this situation with a `CurveOrder` that says
///
/// - `Order::Left` on the interval `[0, 0.99]`
/// - `Order::Ish` in the interval `[0.99, 1.01]`, and
/// - `Order::Right` on the interval `[1.01, 2]`.
///
/// Note that this representation has some slack around the intersection point,
/// because we do everything numerically and there might be some errors.
#[derive(Clone, Debug)]
pub struct CurveOrder {
    start: f64,
    cmps: SmallVec<[CurveOrderEntry; 3]>,
    /// Observability metrics for the intersection algorithm.
    pub metrics: Metrics,
}

/// An iterator over intervals in a [`CurveOrder`].
pub struct CurveOrderIter<'a, T> {
    next: f64,
    iter: std::slice::Iter<'a, CurveEntry<T>>,
}

/// The different ways in which two curves can interact.
#[derive(Clone, Copy, Debug)]
pub enum CurveInteraction {
    /// The curves cross between two heights.
    Cross(f64, f64),
    /// The curves touch between two heights, but don't actually cross.
    Touch(f64, f64),
}

impl<T: Copy> Iterator for CurveOrderIter<'_, T> {
    type Item = (f64, f64, T);

    fn next(&mut self) -> Option<Self::Item> {
        let next_entry = self.iter.next()?;
        let start = self.next;
        self.next = next_entry.end;
        Some((start, next_entry.end, next_entry.order))
    }
}

// TODO: we've grown a weird set of convenience methods. Design them better.
impl CurveOrder {
    fn new(start: f64) -> Self {
        CurveOrder {
            start,
            cmps: SmallVec::new(),
            metrics: Metrics::default(),
        }
    }

    fn push(&mut self, end: f64, order: Order, status: MetricsStatus) {
        self.metrics.entries.push(MetricsEntry::new(end, status));
        if let Some(last) = self.cmps.last_mut() {
            match (last.order, order) {
                (Order::Right, Order::Right)
                | (Order::Left, Order::Left)
                | (Order::Ish, Order::Ish) => {
                    debug_assert!(end >= last.end);
                    last.end = end;
                }
                (Order::Right, Order::Left) | (Order::Left, Order::Right) => {
                    // It would be nice if we could forbid this case, but the
                    // combination of almost-horizontal segments and errors in
                    // y position make it hard to avoid.
                    //
                    // What we do is insert a zero-height "ish" interval. Then
                    // adding y-slop will turn it into a taller interval.
                    debug_assert!(end >= last.end);
                    self.cmps.push(CurveOrderEntry {
                        end,
                        order: Order::Ish,
                    });
                    self.cmps.push(CurveOrderEntry { end, order });
                }
                _ => {
                    debug_assert!(end >= last.end);
                    if end > last.end {
                        self.cmps.push(CurveOrderEntry { end, order });
                    }
                }
            }
        } else {
            self.cmps.push(CurveOrderEntry { end, order });
        }
    }

    /// Returns an iterator over the ordering intervals.
    pub fn iter(&self) -> CurveOrderIter<'_, Order> {
        CurveOrderIter {
            next: self.start,
            iter: self.cmps.iter(),
        }
    }

    /// Returns a `CurveOrder` with all the orderings flipped.
    pub fn flip(&self) -> Self {
        Self {
            start: self.start,
            cmps: self.cmps.iter().map(CurveOrderEntry::flip).collect(),
            metrics: self.metrics.clone(),
        }
    }

    /// Imagine that our curve is to the left of the other curve at height `y`.
    /// What's the next height at which they touch?
    ///
    /// We "imagine" that our curve is to the left, but we don't actually insist
    /// on it: if our curve is actually to the right at `y` we just say that
    /// they cross immediately.
    pub fn next_touch_after(
        &self,
        y: f64,
    ) -> Option<CurveInteraction> {
        let mut iter = self
            .iter()
            .skip_while(|(_start, end, _order)| end <= &y)
            .skip_while(|(_start, _end, order)| *order == Order::Left);

        let (y0, y1, order) = iter.next()?;
        if order == Order::Right {
            return Some(CurveInteraction::Cross(y, y));
        }

        let Some((_, next_y1, next_order)) = iter.next() else {
            return Some(CurveInteraction::Touch(y0, y1));
        };

        match next_order {
            Order::Right => Some(CurveInteraction::Cross(y0, y1)),
            Order::Left => {
                if y < y0 {
                    Some(CurveInteraction::Touch(y0, y1))
                } else {
                    self.next_touch_after(next_y1)
                }
            }
            Order::Ish => {
                // The current order is Ish and adjacent orders get merged,
                // so the next one isn't Ish.
                unreachable!();
            }
        }
    }

    /// Adds some vertical imprecision to the order comparison.
    ///
    /// To understand why we need this, note that
    /// - All our computations are approximate, and so the computed `y` values where orders
    ///   change will not be exact. When curves are almost horizontal, this error in `y`
    ///   can lead to a large error in `x`.
    /// - For the sweep-line algorithm to work, we need the strong order to have no loops:
    ///   if at some height `y`, `c0` is left of `c1` and `c1` is left of `c2` then `c0`
    ///   is not allowed to be left of `c2`.
    ///
    /// Together, these two properties present a problem: the approximation
    /// error in `y` can lead to ordering loops for some specific heights `y`
    /// where there's a lot of crossing action. (To be honest, I'm not sure that this ever
    /// actually happens. Fuzzing didn't find an example, and I didn't try too hard to
    /// find one manually. But the possibility of ordering loops keeps me up at night,
    /// so let's just assume that they might happen.)
    ///
    /// Our solution to this issue is to admit that all of our `y` values are imprecise,
    /// by expanding the "ish" regions. This method expands all the "ish" regions by `slop`
    /// in both directions. Geometrically, after applying y-slop, you end up with a comparison
    /// where `c0` is declared "left" of `c1` at `y` if a small square around the point on `c0`
    /// at height `y` stays to the left of `c1`. (If one segment is shorter than the other,
    /// this is not quite true near the endpoints of the shorter segment. But close enough.)
    pub fn with_y_slop(self, slop: f64) -> CurveOrder {
        if slop == 0.0 {
            return self;
        }

        let mut ret = SmallVec::<[CurveOrderEntry; 3]>::new();

        // unwrap: cmps is always non-empty
        let last_end = self.cmps.last().unwrap().end;

        if self.start == last_end || self.cmps.len() <= 1 {
            return self;
        }

        for (_start, end, order) in self.iter() {
            if order == Order::Ish {
                let new_end = (end + slop).min(last_end);
                if let Some(last) = ret.last_mut()
                    && last.order == Order::Ish
                {
                    // The input order shouldn't have two "ish"es in a row, but in a previous
                    // iteration we may have deleted a non-ish entry if it was too short. In
                    // that case, we extend the previos "ish" instead of making a new one.
                    last.end = new_end;
                } else {
                    ret.push(CurveOrderEntry {
                        end: new_end,
                        order: Order::Ish,
                    });
                }
            } else {
                let new_end = end - slop;
                if new_end > self.start && ret.last().is_none_or(|last| last.end < new_end) {
                    ret.push(CurveOrderEntry {
                        end: new_end,
                        order,
                    });
                }
            }
        }

        // If we ended with a definite order, the loop above will have shortened it
        // but we actually want it to extend to the end.
        if let Some(last) = ret.last_mut()
            && last.end < last_end
        {
            last.end = last_end;
        }
        if ret.is_empty() {
            ret.push(CurveOrderEntry {
                end: last_end,
                order: Order::Ish,
            });
        }

        CurveOrder {
            start: self.start,
            cmps: ret,
            metrics: Metrics::default(),
        }
    }

    /// Asserts that we satisfy our internal invariants. For testing only.
    pub fn check_invariants(&self) {
        let mut cmps = self.cmps.iter();
        let mut last = cmps.next().unwrap();
        for cmp in cmps {
            assert!(last.end <= cmp.end);
            assert!(last.order != cmp.order);
            assert!(last.order == Order::Ish || cmp.order == Order::Ish);
            last = cmp;
        }
    }

    /// What's the order at `y`?
    ///
    /// If `y` is at the boundary of two intervals, takes the first one.
    ///
    /// TODO: maybe it would be more consistent with the other methods if
    /// we took the second one. But then we need to be careful about what
    /// happens at the endpoint...
    ///
    /// # Panics
    ///
    /// Panics if `y` is outside the range of our ordering.
    pub fn order_at(&self, y: f64) -> Order {
        self.iter()
            .find(|(_start, end, _order)| end >= &y)
            .unwrap()
            .2
    }

    /// Returns the first order entry ending after `y`.:w
    ///
    /// # Panics
    ///
    /// Panics if our comparison range ends at or before `y`.
    pub fn entry_at(&self, y: f64) -> (f64, f64, Order) {
        self.iter().find(|(_start, end, _order)| *end > y).unwrap()
    }

    /// What's the next definite (`Left` or `Right`) ordering after `y`?
    ///
    /// If there is no definite ordering (everything after `y` is just `Ish`),
    /// returns `Ish`.
    ///
    /// As a corner case, if this comparison ends exactly at `y` then we return
    /// the ordering exactly at `y`.
    pub fn order_after(&self, y: f64) -> Order {
        if y == self.cmps.last().unwrap().end {
            self.cmps.last().unwrap().order
        } else {
            self.iter()
                .skip_while(|(_start, end, _order)| end <= &y)
                .find(|(_start, _end, order)| *order != Order::Ish)
                .map_or(Order::Ish, |(_start, _end, order)| order)
        }
    }
}

/// Find the parameter `t` at which `c` crosses height `y`.
pub fn solve_t_for_y(c: CubicBez, y: f64) -> f64 {
    debug_assert!(
        c.p0.y <= y && y <= c.p3.y && c.p0.y < c.p3.y,
        "invalid y ({y}) for curve ({c:?})"
    );

    if y == c.p0.y {
        return 0.0;
    }
    if y == c.p3.y {
        return 1.0;
    }

    let c3 = c.p3.y - 3.0 * c.p2.y + 3.0 * c.p1.y - c.p0.y;
    let c2 = 3.0 * (c.p2.y - 2.0 * c.p1.y + c.p0.y);
    let c1 = 3.0 * (c.p1.y - c.p0.y);
    let c0 = c.p0.y - y;
    let cubic = Cubic::new([c0, c1, c2, c3]);

    let eps = 1e-12;
    let roots = cubic.roots_between(0.0, 1.0, eps);
    if !roots.is_empty() {
        return roots[0];
    }

    // There are situations (discovered by fuzzing) where because of rounding
    // the cubic doesn't actually change signs. (Mathematically, it does change
    // signs because it's `c.p0.y - y` at zero and `c.p3.y - y` at one, and we
    // checked that those have opposite signs.)
    //
    // In this situation, it must be that the cubic was very close to zero
    // at some endpoint, but after rounding the sign flipped.
    debug_assert_eq!(cubic.eval(0.0).signum(), cubic.eval(1.0).signum());
    if (y - c.p0.y).abs() <= (y - c.p3.y).abs() {
        0.0
    } else {
        1.0
    }
}

/// Finds the x coordinate at which `c` crosses through `y`.
pub fn solve_x_for_y(c: CubicBez, y: f64) -> f64 {
    c.eval(solve_t_for_y(c, y)).x
}

/// Restricts a Bézier curve to a vertical range.
///
/// The input curve should be monotonic in `y`, and its range should include
/// `y0` and `y1` (which should be ordered).
pub fn y_subsegment(c: CubicBez, y0: f64, y1: f64) -> CubicBez {
    debug_assert!(y0 < y1);
    debug_assert!(c.p0.y <= y0 && y1 <= c.p3.y);
    let t0 = solve_t_for_y(c, y0);
    let t1 = solve_t_for_y(c, y1);
    let mut ret = c.subsegment(t0..t1);
    ret.p0.y = y0;
    ret.p3.y = y1;
    ret
}

/// Specifies the roots of a quadratic, along with the signs taken before, after, and between the roots.
#[derive(Clone, Copy, Debug)]
pub struct QuadraticSigns {
    /// The smaller root.
    pub smaller_root: f64,
    /// The bigger root.
    pub bigger_root: f64,
    /// The sign of the quadratic in the limit at `f64::NEG_INFINITY`.
    ///
    /// If this is zero, the quadratic is (approximately) zero everywhere.
    /// Otherwise, the quadratic takes this sign until the first root, then the
    /// opposite sign until the second root, then this sign again.
    pub initial_sign: f64,
}

/// An approximate horizontal ordering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// The first thing is to the right of the second thing.
    Right,
    /// The two things are close.
    Ish,
    /// The first thing is to the left of the second thing.
    Left,
}

impl Order {
    /// Returns the opposite ordering.
    pub fn flip(self) -> Order {
        match self {
            Order::Right => Order::Left,
            Order::Ish => Order::Ish,
            Order::Left => Order::Right,
        }
    }
}

/// An approximate horizontal ordering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaybeOrder {
    /// The first thing is to the right of the second thing.
    Right,
    /// The two things are close.
    Ish,
    /// The first thing is to the left of the second thing.
    Left,
    /// We don't have enough information to tell yet.
    Unknown,
}

impl From<Order> for MaybeOrder {
    fn from(ord: Order) -> Self {
        match ord {
            Order::Right => MaybeOrder::Right,
            Order::Ish => MaybeOrder::Ish,
            Order::Left => MaybeOrder::Left,
        }
    }
}

impl TryFrom<MaybeOrder> for Order {
    type Error = ();

    fn try_from(ord: MaybeOrder) -> Result<Self, Self::Error> {
        match ord {
            MaybeOrder::Right => Ok(Order::Right),
            MaybeOrder::Ish => Ok(Order::Ish),
            MaybeOrder::Left => Ok(Order::Left),
            MaybeOrder::Unknown => Err(()),
        }
    }
}

/// What direction does the curve go?
#[derive(Clone, Copy, Debug)]
pub enum Direction {
    /// The curve's `x` coordinate is increasing in `t`.
    Increasing,
    /// The curve's `x` coordinate is decreasing in `t`.
    Decreasing,
}

#[derive(Clone, Copy, Debug)]
enum Bound {
    Left,
    Right,
}

/// A buffer for accumulating ordering comparisons from a [`QuadraticApprox`].
#[derive(Clone, Debug, Default)]
pub struct ApproxComparison<const CAP: usize> {
    /// The order entries.
    entries: ArrayVec<CurveEntry<MaybeOrder>, CAP>,
}

/// There are up to 27 of them, because each quadratic comparison can
/// produce five entries (left, unknown, ish, unknown, right, unknown, ish, unknown, left). And when the
/// almost-horizontal robustness code kicks in, a curve comparison can
/// trigger up to three quadratic comparisons.
type QuadApproxComparison = ApproxComparison<27>;

struct MergedRangeIter<S, T, I, J> {
    a_iter: I,
    b_iter: J,
    a_cur: Option<(f64, S)>,
    b_cur: Option<(f64, T)>,
}
impl<S, T, I, J> MergedRangeIter<S, T, I, J>
where
    I: Iterator<Item = (f64, S)>,
    J: Iterator<Item = (f64, T)>,
    S: Clone,
    T: Clone,
{
    fn new(mut a: I, mut b: J) -> Self {
        Self {
            a_cur: a.next(),
            b_cur: b.next(),
            a_iter: a,
            b_iter: b,
        }
    }
}

impl<S, T, I, J> Iterator for MergedRangeIter<S, T, I, J>
where
    I: Iterator<Item = (f64, S)>,
    J: Iterator<Item = (f64, T)>,
    S: Clone,
    T: Clone,
{
    type Item = (f64, S, T);

    fn next(&mut self) -> Option<Self::Item> {
        if let (Some((a, a_payload)), Some((b, b_payload))) =
            (self.a_cur.clone(), self.b_cur.clone())
        {
            let end = a.min(b);
            if a == end {
                self.a_cur = self.a_iter.next();
            }
            if b == end {
                self.b_cur = self.b_iter.next();
            }
            Some((end, a_payload, b_payload))
        } else {
            None
        }
    }
}

impl<const CAP: usize> ApproxComparison<CAP> {
    fn push(&mut self, end: f64, order: MaybeOrder) {
        debug_assert!(self.entries.last().is_none_or(|e| e.end < end));
        if let Some(e) = self.entries.last_mut()
            && e.order == order
        {
            e.end = end;
        } else {
            self.entries.push(CurveEntry { end, order });
        }
    }

    fn merge_from<const A: usize, const B: usize>(
        a: &ApproxComparison<A>,
        b: &ApproxComparison<B>,
        merge: impl Fn(MaybeOrder, MaybeOrder) -> MaybeOrder,
    ) -> Self {
        let mut a_iter = a.entries.iter();
        let mut b_iter = b.entries.iter();
        let mut ret = Self::default();
        let mut a_cur = a_iter.next();
        let mut b_cur = b_iter.next();

        while let (Some(a), Some(b)) = (a_cur, b_cur) {
            let end = a.end.min(b.end);
            ret.push(end, merge(a.order, b.order));
            if a.end == end {
                a_cur = a_iter.next();
            }
            if b.end == end {
                b_cur = b_iter.next();
            }
        }
        ret
    }

    fn iter(&self, start: f64) -> CurveOrderIter<'_, MaybeOrder> {
        CurveOrderIter {
            next: start,
            iter: self.entries.iter(),
        }
    }
}

/// Partitions the real line according to the sign of `q`. The return value is
/// `(initially_positive, r0, r1)`, where `r0 < r1`. If `initially_positive`,
/// the quadratic is positive up to `r0`, then negative up to `r1`, and then
/// positive again up to infinity.
///
/// It is perfectly normal for `r0` or `r1` (or both) to be infinite. For example,
/// if `r0` is `f64::NEG_INFINITY` and `initially_positive` is `true` it means that
/// the quadratic is positive up to `f64::NEG_INFINITY` (which is vacuous, really)
/// and then negative from there until `r1`.
pub(crate) fn quadratic_signs(q: Quadratic) -> (bool, f64, f64) {
    let &[c, b, a] = q.coeffs();
    let disc = b * b - 4.0 * a * c;
    // The handling of b here is a little confusing, and it has to do with how
    // the "roots" come out in the linear case. If a is zero, and b is not we
    // find roots at `-a.signum() * b.signum() * f64::INFINITY` and something
    // finite. So if a is positive zero and b is positive, the first root we
    // find will be negative infinity and so we want the sign *after* that first
    // root to be negative, and so the initial sign should be positive. And if a
    // is positive zero and b is negative, our first root will be finite and we
    // want the sign before it to be positive. If a is negative zero then things
    // swap around, and the summary is that if a is zero and b is not, the initial
    // sign should be the sign of a. Hooray for signed zero!
    let initially_negative = a < 0.0
        || (a == 0.0 && b != 0.0 && a.signum() == -1.0)
        || (a == 0.0 && b == 0.0 && c < 0.0);
    let numerator = if disc.is_finite() {
        if disc <= 0.0 {
            // There are no sign changes: the sign of `q` is always the sign of `a`.
            return (!initially_negative, f64::NEG_INFINITY, f64::NEG_INFINITY);
        } else {
            // There are two choices for the sign here. This choice gives the
            // root with larger magnitude, which is better for numerical stability
            // because we're going to divide by it to find the other root.
            //
            // See https://math.stackexchange.com/questions/866331 for details.
            -0.5 * (b + disc.sqrt().copysign(b))
        }
    } else {
        // We're trying to compute
        //
        // b + sqrt(b^2 - 4 a c)
        //
        // but the discriminant overflowed, so rewrite it as
        //
        // b (1 + sqrt(1 - 4 a c / b^2))
        //
        // If the discriminant here overflows it must be because a * c dominates,
        // especially if we do the computation in this weird order:
        let scaled_disc = 1.0 - (c / b / b * a) * 4.0;
        if !scaled_disc.is_finite() {
            if c * a > 0.0 {
                // disc must have overflowed to -infinity. There are no real roots.
                return (!initially_negative, f64::NEG_INFINITY, f64::NEG_INFINITY);
            } else {
                // disc must have overflowed to +infinity, meaning that
                // there are two roots but they're so far out that the
                // equation is effectively constant. The sign is the opposite
                // of the leading term's sign: the roots are super far out,
                // so the effective sign most of the time is opposite of the
                // sign at infinity.
                return (initially_negative, f64::NEG_INFINITY, f64::NEG_INFINITY);
            }
        }
        -0.5 * b * (1.0 + scaled_disc.sqrt())
    };

    let r1 = numerator / a;
    let r2 = c / numerator;
    let (r1, r2) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
    (!initially_negative, r1, r2)
}

/// Populates `out` according to the sign of `q`, restricted to
/// the interval between `y0` and `y1`. `neg` is the value to put
/// in `out` when `q` is negative, and `pos` is the value to put in
/// `out` when `q` is positive.
fn push_quadratic_sign<const CAP: usize, T>(
    q: Quadratic,
    neg: Order,
    pos: Order,
    y0: f64,
    y1: f64,
    out: &mut ArrayVec<CurveEntry<T>, CAP>,
) where
    T: PartialEq,
    Order: Into<T>,
{
    let mut push = |end: f64, order: Order| {
        let order = order.into();
        debug_assert!(out.last().is_none_or(|e| e.end < end));
        if let Some(e) = out.last_mut()
            && e.order == order
        {
            e.end = end;
        } else {
            out.push(CurveEntry { end, order });
        }
    };

    let (initially_positive, r1, r2) = quadratic_signs(q);
    let (initial_sign, next_sign) = if initially_positive {
        (pos, neg)
    } else {
        (neg, pos)
    };

    let r1 = r1.min(y1);
    let r2 = r2.min(y1);
    if r1 > y0 {
        push(r1, initial_sign);
    }
    if r2 > r1 && r2 > y0 {
        push(r2, next_sign);
    }
    if y1 > r2 {
        push(y1, initial_sign);
    }
}

/// Analyze the sign of a quadratic.
///
/// Consider the quadratic equation `q(y)` for `y` between `y0` and `y1`, and some
/// thresholds `lower < upper`. We consider the quadratic "less" if it's smaller than `lower`,
/// "greater" if it's bigger than `upper`, and "unknown" if it's between the two thresholds.
/// We push the results of this sign analysis to `out`.
fn push_two_level_quadratic_signs(
    q: Quadratic,
    lower: f64,
    upper: f64,
    y0: f64,
    y1: f64,
    out: &mut QuadApproxComparison,
) {
    debug_assert!(lower < upper);

    let mut push = |end: f64, order: MaybeOrder| {
        if end > y0
            && out
                .entries
                .last()
                .is_none_or(|last| last.end < y1 && last.end < end)
        {
            out.push(end.min(y1), order);
        }
    };
    let lower_q = raise_quadratic(q, -lower);
    let upper_q = raise_quadratic(q, -upper);

    let (lower_initially_positive, r_lower, s_lower) = quadratic_signs(lower_q);
    let (upper_initially_positive, r_upper, s_upper) = quadratic_signs(upper_q);
    // dbg!(r_lower, s_lower, r_upper, s_upper);

    use MaybeOrder::*;
    if upper_initially_positive {
        debug_assert!(r_upper <= r_lower || r_lower.is_infinite());
        debug_assert!(s_lower <= s_upper);

        push(r_upper, Left);
        push(r_lower, Unknown);
        push(s_lower, Right);
        push(s_upper, Unknown);
    } else {
        debug_assert!(r_upper >= r_lower || r_upper.is_infinite());
        debug_assert!(s_lower >= s_upper);

        push(r_lower, Right);
        push(r_upper, Unknown);
        push(s_upper, Left);
        push(s_lower, Unknown);
    }
    let final_sign = if upper_initially_positive && lower_initially_positive {
        Left
    } else if !upper_initially_positive && !lower_initially_positive {
        Right
    } else {
        Unknown
    };
    push(y1, final_sign);
}

/// An axis-aligned quadratic approximation to a cubic Bézier.
///
/// The quadratic here gives `y` as a function of `x`.
#[derive(Clone, Copy, Debug)]
pub struct QuadraticApprox {
    /// The quadratic approximation.
    pub q: Quadratic,
    /// The amount by which the approximation undershoots the real
    /// value. Always non-positive.
    ///
    /// To be precise, the true `x` coordinate of the curve we're
    /// approximating will be at least `<quadratic approx> + dmin`.
    ///
    /// Note that while this value is intended to bound the error,
    /// this isn't strictly a guarantee: we use floating-point to
    /// compute it and we aren't careful with rounding direction.
    /// But because the basic algorithm has some slack built in,
    /// it's very likely that the slack dominates the floating-point
    /// error. So this *probably* is a bound, and our sweep-line
    /// algorithm treats it as one.
    pub dmin: f64,
    /// The amount by which the approximation overshoots the real
    /// value. Always non-negative.
    ///
    /// To be precise, the true `x` coordinate of the curve we're
    /// approximating will be at most `<quadratic approx> + dmax`.
    pub dmax: f64,
    /// If non-`None`, this is an approximation to part of curve that is
    /// almost-horizontal, and which needs a little extra care in collision
    /// detection to account for the fact that a small `y` error in subdividing
    /// the curve might have led to a large `x` error.
    pub almost_horiz: Option<(Direction, f64, f64)>,
}

fn raise_quadratic(q: Quadratic, c: f64) -> Quadratic {
    let &[c_old, b, a] = q.coeffs();
    Quadratic::new([c_old + c, b, a])
}

// Returns the quadratic representing `q(y - shift)`.
fn shift_quadratic(q: Quadratic, shift: f64) -> Quadratic {
    let &[c, b, a] = q.coeffs();
    Quadratic::new([c + a * shift * shift - b * shift, b - 2.0 * shift * a, a])
}

impl QuadraticApprox {
    /// Compute an approximation for a cubic Bézier, which we assume to be
    /// monotonically increasing in `y`.
    ///
    /// Note that this can produce very large coefficients if
    ///
    /// - the Bézier's y range is small, or
    /// - any of the y coordinates is large
    ///
    /// The second effect can be mitigated by translation (especially if
    /// the y range is small *and* the y coordinates are large, because then
    /// translation would make them all small), and the first effect can be
    /// mitigated by rescaling.
    pub fn from_cubic(c: CubicBez, almost_horiz: Option<Direction>) -> Self {
        let q = fit_quadratic(c);
        let &[c0, c1, c2] = q.coeffs();

        // To compare the polynomial c0 + c1 y + c2 y^2 to our Bezier curve,
        // we reparametrize it in t by expanding out the cubic y(t). This
        // gives a degree-6 polynomial, and then we subtract off x(t) to
        // find the error.
        let y = cubic_from_bez_y(c);
        let x = cubic_from_bez_x(c);
        let [y0, y1, y2, y3] = *y.coeffs();
        let [x0, x1, x2, x3] = *x.coeffs();
        let error = Poly::new([
            c0 + c1 * y0 + c2 * y0 * y0 - x0,
            c1 * y1 + c2 * (2.0 * y0 * y1) - x1,
            c1 * y2 + c2 * (2.0 * y0 * y2 + y1 * y1) - x2,
            c1 * y3 + c2 * 2.0 * (y0 * y3 + y1 * y2) - x3,
            c2 * (2.0 * y1 * y3 + y2 * y2),
            c2 * 2.0 * y2 * y3,
            c2 * y3 * y3,
        ]);
        let extrema = error.deriv().roots_between(0.0, 1.0, 1e-8);

        // Errors at the endpoints. These could also be computed more slowly
        // as -error.eval(0.0) and -error.eval(1.0).
        let err0 = c.p0.x - q.eval(c.p0.y);
        let err1 = c.p3.x - q.eval(c.p3.y);

        let mut dmin = 0.0f64.min(err0).min(err1);
        let mut dmax = 0.0f64.max(err0).max(err1);

        for t in extrema {
            let e = -error.eval(t);
            dmin = dmin.min(e);
            dmax = dmax.max(e);
        }

        QuadraticApprox {
            q,
            dmin,
            dmax,
            almost_horiz: almost_horiz.map(|d| (d, c.p0.x, c.p3.x)),
        }
    }

    /// Translate this quadratic approximation in y.
    pub fn shift_y(&self, amount: f64) -> Self {
        Self {
            q: shift_quadratic(self.q, amount),
            ..*self
        }
    }

    fn initial_bound(&self, which: Bound, tolerance: f64) -> Option<Quadratic> {
        match (self.almost_horiz, which) {
            (Some((Direction::Increasing, x, _)), Bound::Left) => {
                Some(Quadratic::new([x - tolerance, 0.0, 0.0]))
            }
            (Some((Direction::Decreasing, x, _)), Bound::Right) => {
                Some(Quadratic::new([x + tolerance, 0.0, 0.0]))
            }
            _ => None,
        }
    }

    fn final_bound(&self, which: Bound, tolerance: f64) -> Option<Quadratic> {
        match (self.almost_horiz, which) {
            (Some((Direction::Increasing, _, x)), Bound::Right) => {
                Some(Quadratic::new([x + tolerance, 0.0, 0.0]))
            }
            (Some((Direction::Decreasing, _, x)), Bound::Left) => {
                Some(Quadratic::new([x - tolerance, 0.0, 0.0]))
            }
            _ => None,
        }
    }

    fn middle_bound(&self, which: Bound, tolerance: f64, outer_bound: bool) -> Quadratic {
        let shift = match (self.almost_horiz, which) {
            (Some((Direction::Increasing, ..)), Bound::Right)
            | (Some((Direction::Decreasing, ..)), Bound::Left) => -tolerance,
            (Some((Direction::Increasing, ..)), Bound::Left)
            | (Some((Direction::Decreasing, ..)), Bound::Right) => tolerance,
            _ => 0.0,
        };
        let factor = if outer_bound { 1.0 } else { -1.0 };
        let raise = match which {
            Bound::Left => factor * self.dmin - tolerance,
            Bound::Right => factor * self.dmax + tolerance,
        };
        shift_quadratic(raise_quadratic(self.q, raise), shift)
    }

    #[allow(dead_code)]
    fn compare(
        &self,
        other: &QuadraticApprox,
        sure_tolerance: f64,
        ish_tolerance: f64,
        y0: f64,
        y1: f64,
    ) -> QuadApproxComparison {
        if self.almost_horiz.is_none() && other.almost_horiz.is_none() {
            let diff = *other - *self;
            let mut out = QuadApproxComparison::default();
            push_two_level_quadratic_signs(
                diff.q,
                -diff.dmax - sure_tolerance,
                -diff.dmin + sure_tolerance,
                y0,
                y1,
                &mut out,
            );
            return out;
        }

        use MaybeOrder::*;
        let our_right_their_left = self.compare_one_side(other, sure_tolerance, y0, y1, true);
        let their_right_our_left = other.compare_one_side(self, sure_tolerance, y0, y1, true);
        let sure_order = QuadApproxComparison::merge_from(
            &our_right_their_left,
            &their_right_our_left,
            |ortl, trol| match (ortl, trol) {
                (Left, _) => {
                    debug_assert_eq!(trol, Ish);
                    Left
                }
                (_, Left) => {
                    debug_assert_eq!(ortl, Ish);
                    Right
                }
                _ => Ish,
            },
        );

        let our_right_their_left = self.compare_one_side(other, ish_tolerance, y0, y1, false);
        let their_right_our_left = other.compare_one_side(self, ish_tolerance, y0, y1, false);
        let ish_order = QuadApproxComparison::merge_from(
            &our_right_their_left,
            &their_right_our_left,
            |ortl, trol| match (ortl, trol) {
                (Left, _) => Left,
                (_, Left) => Right,
                _ => Ish,
            },
        );

        QuadApproxComparison::merge_from(&sure_order, &ish_order, |sure, ish| match (sure, ish) {
            (MaybeOrder::Left | MaybeOrder::Right, _) => sure,
            (_, MaybeOrder::Ish) => MaybeOrder::Ish,
            _ => MaybeOrder::Unknown,
        })
    }

    fn compare_one_side(
        &self,
        other: &QuadraticApprox,
        tolerance: f64,
        mut y0: f64,
        y1: f64,
        outer_bound: bool,
    ) -> QuadApproxComparison {
        let mut buf = ApproxComparison::default();
        let r = self.initial_bound(Bound::Right, tolerance);
        let l = other.initial_bound(Bound::Left, tolerance);
        if r.is_some() || l.is_some() {
            let r = r.unwrap_or_else(|| self.middle_bound(Bound::Right, tolerance, outer_bound));
            let l = l.unwrap_or_else(|| other.middle_bound(Bound::Left, tolerance, outer_bound));
            let end = (y0 + tolerance).min(y1);
            push_quadratic_sign(r - l, Order::Left, Order::Ish, y0, end, &mut buf.entries);
            y0 = end;
        }

        let r = self.final_bound(Bound::Right, tolerance);
        let l = other.final_bound(Bound::Left, tolerance);
        let end = if r.is_some() || l.is_some() {
            (y1 - tolerance).max(y0)
        } else {
            y1
        };

        if end > y0 {
            let r = self.middle_bound(Bound::Right, tolerance, outer_bound);
            let l = other.middle_bound(Bound::Left, tolerance, outer_bound);
            push_quadratic_sign(r - l, Order::Left, Order::Ish, y0, end, &mut buf.entries);
            y0 = end;
        }

        if y0 < y1 {
            let r = r.unwrap_or_else(|| self.middle_bound(Bound::Right, tolerance, outer_bound));
            let l = l.unwrap_or_else(|| other.middle_bound(Bound::Left, tolerance, outer_bound));
            push_quadratic_sign(r - l, Order::Left, Order::Ish, y0, y1, &mut buf.entries);
        }

        buf
    }

    #[cfg(test)]
    fn eval(&self, y: f64) -> f64 {
        self.q.eval(y)
    }
}

impl std::ops::Sub for QuadraticApprox {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        // Subtracting quadratic approximations is only a valid way of comparing
        // them if the extra almost-horizontal robustness stuff is inactive.
        debug_assert!(self.almost_horiz.is_none() && other.almost_horiz.is_none());

        QuadraticApprox {
            q: self.q - other.q,
            dmin: self.dmin - other.dmax,
            dmax: self.dmax - other.dmin,
            almost_horiz: None,
        }
    }
}

// Find a quadratic passing through the given points. The x's are assuming to be
// in strictly increasing order (and if they are close to one another, this will
// be numerically unstable).
fn interpolate_quadratic(x0: f64, y0: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> Quadratic {
    debug_assert!(x0 < x1 && x1 < x2);

    // If we took reciprocals here, we could eliminate some divisions (at the cost of some accuracy).
    let dx10 = x1 - x0;
    let dx21 = x2 - x1;
    let dx20 = x2 - x0;

    // We use the Lagrange basis functions. These are their coefficients, constant terms first.
    // We pre-scale by their interpolation coefficients, just to group multiplications more efficiently.
    // As a result, ell0 is the polynomial with ell0(x0) = y0, ell0(x1) = 0, ell0(x2) = 0.
    // ell0 = y0 (x - x1) (x - x2) / ((x0 - x1) (x0 - x2))
    let denom0 = dx10 * dx20;
    let factor0 = y0 / denom0;
    let ell0 = [x1 * x2 * factor0, -(x1 + x2) * factor0, factor0];

    // ell1 = y1 (x - x0) (x - x2) / ((x1 - x0) (x1 - x2))
    let denom1 = -dx10 * dx21;
    let factor1 = y1 / denom1;
    let ell1 = [x0 * x2 * factor1, -(x0 + x2) * factor1, factor1];

    // ell2 = y2 (x - x0) (x - x1) / ((x2 - x0) (x2 - x1))
    let denom2 = dx21 * dx20;
    let factor2 = y2 / denom2;
    let ell2 = [x0 * x1 * factor2, -(x0 + x1) * factor2, factor2];

    Quadratic::new([
        ell0[0] + ell1[0] + ell2[0],
        ell0[1] + ell1[1] + ell2[1],
        ell0[2] + ell1[2] + ell2[2],
    ])
}

fn fit_quadratic(c: CubicBez) -> Quadratic {
    let t0 = 0.0;
    let t1 = 0.3;
    let t2 = 0.7;
    let t3 = 1.0;

    let p0 = c.eval(t0);
    let p1 = c.eval(t1);
    let p2 = c.eval(t2);
    let p3 = c.eval(t3);

    // For very short (or almost horizontal) segments the interpolation is too
    // unstable. In that case, just take a chord.
    if p1.y - p0.y < 1e-12 || p2.y - p1.y < 1e-12 {
        // p0.x * (y - p3.y) / (p0.y - p3.y)
        // + p3.x * (y - p0.y) / (p3.y - p0.y)
        return Quadratic::new([
            (p0.x * p3.y - p3.x * p0.y) / (p3.y - p0.y),
            (p3.x - p0.x) / (p3.y - p0.y),
            0.0,
        ]);
    }

    let p = interpolate_quadratic(p0.y, p0.x, p1.y, p1.x, p2.y, p2.x);
    let q = interpolate_quadratic(p0.y, 1.0, p1.y, -1.0, p2.y, 1.0);

    // Note that the denominator won't be small (assuming that p0.y, p1.y, p2.y,
    // p3.y are ordered), because q(p2.y) is 1, and q has already had two sign
    // changes before p2.y, so it can't change sign again.
    let e = (p.eval(p3.y) - p3.x) / (q.eval(p3.y) + 1.0);

    let p = p.coeffs();
    let q = q.coeffs();
    Quadratic::new([p[0] - q[0] * e, p[1] - q[1] * e, p[2] - q[2] * e])
}

#[allow(dead_code)]
fn fit_quadratic_with_endpoints_and_area(c: CubicBez) -> Quadratic {
    let seg = PathSeg::Cubic(c);
    let close_seg = PathSeg::Line(Line::new(c.p3, c.p0));
    let area = seg.area() + close_seg.area();
    let dy = c.p3.y - c.p0.y;
    // Note: this solution gives 0 error at endpoints. Arguably
    // a better solution would be based on mean / moments.
    let c2 = -6. * area / dy.powi(3);
    let c1 = (c.p3.x - c.p0.x - (c.p3.y.powi(2) - c.p0.y.powi(2)) * c2) / dy;
    let c0 = c.p0.x - c1 * c.p0.y - c2 * c.p0.y.powi(2);
    Quadratic::new([c0, c1, c2])
}

/// Get the parameters such that the curve can be represented by the following formula:
///     B(t) = c0 + c1 * t + c2 * t^2
pub fn quad_parameters(q: QuadBez) -> (Vec2, Vec2, Vec2) {
    let c0 = q.p0.to_vec2();
    let c1 = (q.p1 - q.p0) * 2.0;
    let c2 = c0 - q.p1.to_vec2() * 2.0 + q.p2.to_vec2();
    (c0, c1, c2)
}

fn l_infty_distance(p0: Point, p1: Point) -> f64 {
    (p0.x - p1.x).abs().max((p0.y - p1.y).abs())
}

fn max_distance(c0: CubicBez, c1: CubicBez) -> f64 {
    l_infty_distance(c0.p0, c1.p0)
        .max(l_infty_distance(c0.p1, c1.p1))
        .max(l_infty_distance(c0.p2, c1.p2))
        .max(l_infty_distance(c0.p3, c1.p3))
}

#[derive(Clone, Debug)]
struct CubicParams {
    c: CubicBez,
    dir: Option<Direction>,
    approx: QuadraticApprox,
}

#[allow(clippy::too_many_arguments)]
fn intersect_cubics_rec(
    cubic0: &CubicParams,
    cubic1: &CubicParams,
    use_precomputed_approx: bool,
    y0: f64,
    y1: f64,
    sure_tolerance: f64,
    ish_tolerance: f64,
    out: &mut CurveOrder,
    status: MetricsStatus,
) {
    // eprintln!("recursing to {y0}..{y1}");
    let mut c0 = y_subsegment(cubic0.c, y0, y1);
    let mut c1 = y_subsegment(cubic1.c, y0, y1);
    // dbg!(c0, orig_c0);
    // dbg!(c1, orig_c1);

    if max_distance(c0, c1) <= ish_tolerance {
        out.push(y1, Order::Ish, status);
        return;
    }
    if y1 - y0 < ish_tolerance {
        // For very short intervals there's some numerical instability in constructing the
        // approximating quadratics, so we just do a coarser comparison based on bounding
        // boxes.
        let b0 = Shape::bounding_box(&c0);
        let b1 = Shape::bounding_box(&c1);
        let order = if b1.min_x() >= b0.max_x() + ish_tolerance {
            Order::Left
        } else if b0.min_x() >= b1.max_x() + ish_tolerance {
            Order::Right
        } else {
            Order::Ish
        };
        out.push(y1, order, status);
        return;
    }

    // If the y coordinates are very off-center, the quadratic coefficients become
    // large and cause instability. So we re-center and then compensate afterwards.
    let y_mid = (y0 + y1) / 2.0;
    c0.p0.y -= y_mid;
    c0.p1.y -= y_mid;
    c0.p2.y -= y_mid;
    c0.p3.y -= y_mid;
    c1.p0.y -= y_mid;
    c1.p1.y -= y_mid;
    c1.p2.y -= y_mid;
    c1.p3.y -= y_mid;

    // TODO: document the precomputed approximation
    let (ep0, ep1) = if use_precomputed_approx {
        let orig_y_mid0 = (cubic0.c.p0.y + cubic0.c.p3.y) / 2.0;
        let orig_y_mid1 = (cubic1.c.p0.y + cubic1.c.p3.y) / 2.0;
        (
            cubic0.approx.shift_y(orig_y_mid0 - y_mid),
            cubic1.approx.shift_y(orig_y_mid1 - y_mid),
        )
    } else {
        (
            QuadraticApprox::from_cubic(c0, cubic0.dir),
            QuadraticApprox::from_cubic(c1, cubic1.dir),
        )
    };

    //let mut scratch = CurveOrder::new((y0 - y_mid) * y_scale);
    let mut signs = ep0.compare(&ep1, sure_tolerance, ish_tolerance, y0 - y_mid, y1 - y_mid);

    // dbg!(orig_c0, orig_c1, ep0, ep1, y0, y1, &signs);
    // for (y0, y1, order) in signs.iter((y0 - y_mid) * y_scale) {
    //     dbg!(y0, y1, order);
    //     match order {
    //         MaybeOrder::Right => {
    //             debug_assert!(solve_x_for_y(c0, y0) > solve_x_for_y(c1, y0));
    //             debug_assert!(dbg!(solve_x_for_y(c0, y1)) > dbg!(solve_x_for_y(c1, y1)));
    //         }
    //         MaybeOrder::Left => {
    //             debug_assert!(solve_x_for_y(c0, y0) < solve_x_for_y(c1, y0));
    //             debug_assert!(solve_x_for_y(c0, y1) < solve_x_for_y(c1, y1));
    //         }
    //         _ => {}
    //     }
    // }

    // Re-center the roots. It's important that the starting and ending positions
    // have no rounding error, so deal with them separately.
    for entry in &mut signs.entries {
        entry.end = (entry.end + y_mid).clamp(y0, y1);
    }
    signs.entries.last_mut().unwrap().end = y1;

    for (y0, y1, order) in signs.iter(y0) {
        match order {
            MaybeOrder::Right => {
                debug_assert!(solve_x_for_y(cubic0.c, y0) > solve_x_for_y(cubic1.c, y0));
                debug_assert!(solve_x_for_y(cubic0.c, y1) > solve_x_for_y(cubic1.c, y1));
            }
            MaybeOrder::Left => {
                debug_assert!(solve_x_for_y(cubic0.c, y0) < solve_x_for_y(cubic1.c, y0));
                debug_assert!(solve_x_for_y(cubic0.c, y1) < solve_x_for_y(cubic1.c, y1));
            }
            _ => {}
        }
    }

    for (new_y0, new_y1, order) in signs.iter(y0) {
        // It's possible to end up with empty intervals: even though they're non-empty
        // when we construct them, scaling and rounding can make them empty. This case
        // could do with more thought, but for now we just ignore empty ones. The main
        // effect here is probably that we delete a tiny "ish" interval around a crossing.
        if new_y0 == new_y1 {
            continue;
        }
        // dbg!(new_y0, new_y1, order);
        // dbg!(
        //     solve_x_for_y(orig_c0, new_y0),
        //     solve_x_for_y(orig_c1, new_y0)
        // );
        // dbg!(
        //     solve_x_for_y(orig_c0, new_y1),
        //     solve_x_for_y(orig_c1, new_y1)
        // );
        if let Ok(order) = order.try_into() {
            out.push(new_y1, order, status);
        } else {
            let mid = 0.5 * (new_y0 + new_y1);

            // We test the difference between y1 and y0, not new_y1 and new_y0.
            // This help reduce false positives where the error in dep is substantial
            // but it just barely had a root and so we picked up a very small
            // "ish" interval that's just an artifact. In this case, new_y0 and new_y1
            // will be very close, but we'll recurse one more time to get a better
            // quadratic approximation.
            if y1 - y0 <= ish_tolerance
                || (ep0.dmax + ep1.dmax <= 2.0 * ish_tolerance
                    && ep0.dmin + ep1.dmin >= -2.0 * ish_tolerance)
            {
                out.push(new_y1, Order::Ish, status);
            } else if new_y1 - new_y0 < 0.5 * (y1 - y0) {
                intersect_cubics_rec(
                    cubic0,
                    cubic1,
                    false,
                    new_y0,
                    new_y1,
                    sure_tolerance,
                    ish_tolerance,
                    out,
                    status.bump(),
                );
            } else {
                // eprintln!(
                //     "recursing because interval didn't shrink: {y0} - {new_y0} - {new_y1} - {y1}, error = {}, scratch {:?}, x slope {}",
                //     dep.dmax - dep.dmin, scratch,
                //     (c0.p3.x - c0.p0.x) / (y1 - y0)
                // );
                intersect_cubics_rec(
                    cubic0,
                    cubic1,
                    false,
                    new_y0,
                    mid,
                    sure_tolerance,
                    ish_tolerance,
                    out,
                    status.bump(),
                );
                intersect_cubics_rec(
                    cubic0,
                    cubic1,
                    false,
                    mid,
                    new_y1,
                    sure_tolerance,
                    ish_tolerance,
                    out,
                    status.bump(),
                );
            }
        }
    }
}

/// A decomposition of a cubic Bezier into almost-horizontal and not-almost-horizontal pieces.
#[derive(Clone, Debug, Default)]
pub struct AlmostHorizontalDecomposition {
    pieces: ArrayVec<CubicParams, 6>,
}

impl AlmostHorizontalDecomposition {
    /// Decompose a cubic into almost-horizontal and not-almost-horizontal pieces.
    ///
    /// The cubic `c` must be (approximately) y-monotone and must not be completely horizontal.
    pub fn from_monotone_cubic(c: CubicBez) -> Self {
        const THRESHOLD: f64 = 16.0;
        let dx = cubic_from_bez_x(c).deriv();
        let dy = cubic_from_bez_y(c).deriv();

        // We want to solve |dx/dy| > THRESHOLD. Since dy is non-negative, this is equivalent to
        // dx - dy * THRESHOLD > 0 or
        // -dx - dy * THRESHOLD > 0.
        let increasing = dx - dy * THRESHOLD;
        let decreasing = dy * (-THRESHOLD) - dx;

        let mut increasing_buf = ApproxComparison::<3>::default();
        let mut decreasing_buf = ApproxComparison::<3>::default();
        push_quadratic_sign(
            increasing,
            Order::Ish,
            Order::Right,
            0.0,
            1.0,
            &mut increasing_buf.entries,
        );

        push_quadratic_sign(
            decreasing,
            Order::Ish,
            Order::Left,
            0.0,
            1.0,
            &mut decreasing_buf.entries,
        );

        use MaybeOrder::*;
        let subdivision =
            ApproxComparison::<6>::merge_from(&increasing_buf, &decreasing_buf, |inc, dec| match (
                inc, dec,
            ) {
                (Right, _) => Right,
                (_, Left) => Left,
                _ => Ish,
            });

        // The subdivision can produce degenerate things (like, horizontal
        // segments or zero-length segments). We filter them out, because the
        // curve comparison expects all y-intervals to be non-empty. Our filtering
        // mechanism works in the t-parameter space: if a subinterval would give
        // a horizontal segment, just drop it and tack that t-interval onto the
        // next one.
        let mut last_y = c.p0.y;
        let mut last_end = 0.0;
        let orig_c = c;
        let mut ret = ArrayVec::default();
        for (_, end, order) in subdivision.iter(0.0) {
            let start = last_end;
            // If there's no actual subdivision, skip the subsegment operation.
            // Although `subsegment` perserves p0 and p3 exactly, it can perturb p1
            // and p2.
            let mut c = if start == 0.0 && end == 1.0 {
                orig_c
            } else {
                orig_c.subsegment(start..end)
            };

            // Enforce y-monotonicity, from the previous curve to this one, and
            // also for the initial and final tangents of this subsegment.
            c.p0.y = c.p0.y.clamp(last_y, orig_c.p3.y);
            c.p1.y = c.p1.y.clamp(c.p0.y, orig_c.p3.y);
            c.p3.y = c.p3.y.clamp(last_y, orig_c.p3.y);
            c.p2.y = c.p2.y.clamp(last_y, c.p3.y);
            last_y = c.p3.y;
            let approx_horiz = match order {
                Right => {
                    // This is supposed to be increasing in x, so fix up the initial final tangents just in case.
                    // They're very likely to already be in the right direction, but if the cubic is very wild
                    // then a small error in y_subsegment could mess things up.
                    c.p1.x = c.p1.x.max(c.p0.x);
                    c.p2.x = c.p2.x.min(c.p3.x);
                    Some(Direction::Increasing)
                }
                Ish | Unknown => None,
                Left => {
                    c.p1.x = c.p1.x.min(c.p0.x);
                    c.p2.x = c.p2.x.max(c.p3.x);
                    Some(Direction::Decreasing)
                }
            };
            if c.p0.y != c.p3.y {
                let mut recentered_c = c;
                let y_mid = (c.p0.y + c.p3.y) / 2.0;
                recentered_c.p0.y -= y_mid;
                recentered_c.p1.y -= y_mid;
                recentered_c.p2.y -= y_mid;
                recentered_c.p3.y -= y_mid;
                let p = CubicParams {
                    c,
                    dir: approx_horiz,
                    approx: QuadraticApprox::from_cubic(recentered_c, approx_horiz),
                };
                ret.push(p);
                last_end = end;
            }
        }
        // In case the last interval got filtered out, make sure we end at the right place.
        ret.last_mut().unwrap().c.p3 = orig_c.p3;

        Self { pieces: ret }
    }
}

/// Compute the horizontal order between two cubics, for the vertical range that they have in common.
///
/// The returned order breaks the vertical range into regions, and in each region either specifies
/// a definite order or says that the curves are close. For example, for the following two curves
/// we might identify five vertical regions.
///
/// ```text
/// c0      c1
///  \      /
///   \    /    left
///    \  /     ______
///     \/      close
///     /\      ______
///    /  \
///   (    \    right
///    \    )
///     \  /    ______
///      \/     close
///      /\     ______
///     /  \    left
/// ```
///
/// There are two "closeness" parameters: `sure_tolerance` and `ish_tolerance`,
/// of which `ish_tolerance` should be larger. The `sure_tolerance` parameter
/// tells us when we are allowed to say that there's a definite order: if `c0`
/// is more than `sure_tolerance` to the left of `c1`, we can say that it's to
/// the left. The purpose of this parameter is to protect against numerical
/// errors. In the future, maybe we can determine it automatically based on the
/// magnitude of the inputs.
///
/// The `ish_tolerance` parameter tells us when we're allowed to say that
/// the two curves are close: if they're definitely within `ish_tolerance`
/// of one another, we can say that they're close.
pub fn intersect_cubics(
    c0: CubicBez,
    c1: CubicBez,
    sure_tolerance: f64,
    ish_tolerance: f64,
) -> CurveOrder {
    intersect_cubics_with_precomputed_decomp(
        c0,
        &AlmostHorizontalDecomposition::from_monotone_cubic(c0),
        c1,
        &AlmostHorizontalDecomposition::from_monotone_cubic(c1),
        sure_tolerance,
        ish_tolerance,
    )
}

/// A version of [intersect_cubics] that uses a precomputed almost-horizontal
/// decomposition.
///
/// Pre-computing the almost-horizontal decomposition can be much faster, because
/// then you can do it per-curve instead of per-comparison.
pub fn intersect_cubics_with_precomputed_decomp(
    c0: CubicBez,
    c0_div: &AlmostHorizontalDecomposition,
    c1: CubicBez,
    c1_div: &AlmostHorizontalDecomposition,
    sure_tolerance: f64,
    ish_tolerance: f64,
) -> CurveOrder {
    let y0 = c0.p0.y.max(c1.p0.y);
    let y1 = c0.p3.y.min(c1.p3.y);

    let mut ret = CurveOrder::new(y0);
    if y0 < y1 {
        let c0_div = c0_div
            .pieces
            .iter()
            .filter(|cp| cp.c.p3.y > y0)
            .map(|cp| (cp.c.p3.y, cp));
        let c1_div = c1_div
            .pieces
            .iter()
            .filter(|cp| cp.c.p3.y > y0)
            .map(|cp| (cp.c.p3.y, cp));
        for (_, c0, c1) in MergedRangeIter::new(c0_div, c1_div) {
            let y0 = c0.c.p0.y.max(c1.c.p0.y);
            let y1 = c0.c.p3.y.min(c1.c.p3.y);
            if y0 < y1 {
                intersect_cubics_rec(
                    c0,
                    c1,
                    true,
                    y0,
                    y1,
                    sure_tolerance,
                    ish_tolerance,
                    &mut ret,
                    MetricsStatus::default(),
                );
            }
        }
    } else if y0 == y1 {
        // Neither of the curves should be purely horizontal, so it must be
        // that they just overlap at a single point.
        debug_assert!(c0.p0.y == c1.p3.y || c0.p3.y == c1.p0.y);

        let (x0, x1) = if c0.p0.y == c1.p3.y {
            (c0.p0.x, c1.p3.x)
        } else {
            (c0.p3.x, c1.p0.x)
        };
        let order = if x0 < x1 - ish_tolerance {
            Order::Left
        } else if x0 > x1 + ish_tolerance {
            Order::Right
        } else {
            Order::Ish
        };

        ret.push(y1, order, MetricsStatus::default());
    }
    //fix_up_endpoints(&mut ret, c0, c1, eps);
    debug_assert_eq!(ret.cmps.last().unwrap().end, y1);
    ret
}

fn cubic_from_bez(a0: f64, a1: f64, a2: f64, a3: f64) -> Cubic {
    let c3 = a3 - 3.0 * a2 + 3.0 * a1 - a0;
    let c2 = 3.0 * (a2 - 2.0 * a1 + a0);
    let c1 = 3.0 * (a1 - a0);
    let c0 = a0;
    Cubic::new([c0, c1, c2, c3])
}

fn cubic_from_bez_x(c: CubicBez) -> Cubic {
    cubic_from_bez(c.p0.x, c.p1.x, c.p2.x, c.p3.x)
}

fn cubic_from_bez_y(c: CubicBez) -> Cubic {
    cubic_from_bez(c.p0.y, c.p1.y, c.p2.y, c.p3.y)
}

#[cfg(any(test, feature = "arbitrary"))]
#[doc(hidden)]
pub mod arbtests {
    use super::{cubic_from_bez_x, cubic_from_bez_y};
    use arbitrary::Unstructured;
    use kurbo::ParamCurve as _;

    use super::solve_t_for_y;

    pub fn solve_for_t(u: &mut Unstructured) -> Result<(), arbitrary::Error> {
        let c = crate::arbitrary::monotonic_bezier(u)?;
        if c.p0.y == c.p3.y {
            return Ok(());
        }

        // How much relative accuracy do we expect?
        // This was determined empirically: 1e-10 fails fuzz tests.
        let accuracy = 1e-9;
        let max_coeff = cubic_from_bez_x(c)
            .max_abs_coefficient()
            .max(cubic_from_bez_y(c).max_abs_coefficient());
        let threshold = accuracy * max_coeff.max(1.0);

        let y = crate::arbitrary::float_in_range(c.p0.y, c.p3.y, u)?;
        let t = solve_t_for_y(c, y);
        assert!((c.eval(t).y - y).abs() <= threshold);

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn solve_for_t_accuracy() {
        let c = CubicBez {
            p0: (0.5, 0.0).into(),
            p1: (0.19531249655301508, 0.6666666864572713).into(),
            p2: (0.00781250000181899, 1.0).into(),
            p3: (0.0, 1.0).into(),
        };
        let y0 = 0.0;
        let y1 = 0.9999999406281859;
        let c0 = c.subsegment(solve_t_for_y(c, y0)..solve_t_for_y(c, y1));
        assert!((dbg!(c0.p3.y) - dbg!(y1)).abs() < 1e-8);
        let y1 = 0.9999999;
        let c0 = c.subsegment(solve_t_for_y(c, y0)..solve_t_for_y(c, y1));
        assert!((dbg!(c0.p3.y) - dbg!(y1)).abs() < 1e-8);
    }

    #[test]
    fn solve_for_t_accuracy2() {
        let c = CubicBez {
            p0: (0.9999999406281859, 0.0).into(),
            p1: (0.011764705882352941, 0.011764705882352941).into(),
            p2: (0.011764705882352941, 0.023391003460207612).into(),
            p3: (0.011764705882352941, 0.03488052106655811).into(),
        };
        let y = 0.012468608207852864;
        let t = solve_t_for_y(c, y);
        assert!(dbg!(c.eval(t).y - y).abs() < 1e-8);
    }

    #[test]
    fn solve_for_t_accuracy3() {
        let c = CubicBez {
            p0: (0.1443023801753, -0.00034678801186988073).into(),
            p1: (0.3760345290602, -0.00011491438194505266).into(),
            p2: (0.6070068772783, 0.00011680717653411721).into(),
            p3: (0.8372203215247, 0.0003483767633017387).into(),
        };

        let y = 0.00016929232880729566;
        let t = solve_t_for_y(c, y);
        assert!(dbg!(c.eval(t).y - y).abs() < 1e-8);
    }

    #[test]
    fn solve_for_t_arbtest() {
        arbtest::arbtest(arbtests::solve_for_t);
    }

    #[test]
    fn non_touching() {
        let sure_tol = 0.01;
        let ish_tol = 0.03;
        let a = Line::new((-1e6, 0.0), (0.0, 1.0));
        let b = Line::new((ish_tol * 1.5, 0.0), (ish_tol * 1.5, 1.0));
        let a = PathSeg::Line(a).to_cubic();
        let b = PathSeg::Line(b).to_cubic();

        let cmp = intersect_cubics(a, b, sure_tol, ish_tol);
        assert_eq!(cmp.order_at(1.0), Order::Left);
    }

    #[test]
    fn almost_touching() {
        let sure_tol = 0.25;
        let ish_tol = 0.3;
        let a = CubicBez::new((-1.0, 0.0), (0.5, 0.6), (0.5, 0.4), (-1.0, 1.0));
        let b = CubicBez::new((0.3, 0.0), (0.3, 1.0 / 3.0), (0.3, 2.0 / 3.0), (0.3, 1.0));
        let cmp = intersect_cubics(a, b, sure_tol, ish_tol);
        assert_eq!(cmp.iter().count(), 3);
    }

    #[test]
    fn quad_approximation() {
        let c = CubicBez {
            p0: (0.1443023801753, -0.00034678801186988073).into(),
            p1: (0.3760345290602, -0.00011491438194505266).into(),
            p2: (0.6070068772783, 0.00011680717653411721).into(),
            p3: (0.8372203215247, 0.0003483767633017387).into(),
        };

        let y = 0.00016929232880729566;
        let t = solve_t_for_y(c, y);
        dbg!(t, c.eval(t));

        let ep = QuadraticApprox::from_cubic(c, None);
        dbg!(y);
        dbg!(ep);
        dbg!(ep.eval(y));
        dbg!(solve_x_for_y(c, y));

        let diff = solve_x_for_y(c, y) - ep.eval(y);
        assert!(diff <= ep.dmax);
        assert!(diff >= ep.dmin);
    }

    // At some point, this test case looped infinitely.
    #[test]
    fn test_loop() {
        let c0 = CubicBez {
            p0: (0.5, 0.0).into(),
            p1: (0.0014551791831513819, 0.9999847770668531).into(),
            p2: (0.9999999850988388, 1.0).into(),
            p3: (0.0, 1.0).into(),
        };
        let c1 = CubicBez {
            p0: (0.5, 0.0).into(),
            p1: (0.0014551791831513819, 0.9999847770668531).into(),
            p2: (1.0, 0.9999999999999717).into(),
            p3: (0.0, 0.9999999999999717).into(),
        };

        dbg!(intersect_cubics(c0, c1, 0.5e-6, 1.5e-6));
    }

    #[test]
    fn y_slop_single_seg() {
        let order = CurveOrder {
            start: 0.0,
            cmps: [CurveOrderEntry {
                end: 1.0,
                order: Order::Right,
            }]
            .into_iter()
            .collect(),
            metrics: Default::default(),
        };

        assert_eq!(order.clone().with_y_slop(0.5).cmps, order.cmps);
    }

    #[test]
    fn y_slop_single_short_seg() {
        let order = CurveOrder {
            start: 0.0,
            cmps: [CurveOrderEntry {
                end: 0.2,
                order: Order::Right,
            }]
            .into_iter()
            .collect(),
            metrics: Default::default(),
        };

        assert_eq!(order.clone().with_y_slop(0.5).cmps, order.cmps);
    }

    #[test]
    fn y_slop_short_start_order() {
        let order = CurveOrder {
            start: 0.0,
            cmps: [
                CurveOrderEntry {
                    end: 0.1,
                    order: Order::Right,
                },
                CurveOrderEntry {
                    end: 1.0,
                    order: Order::Ish,
                },
            ]
            .into_iter()
            .collect(),
            metrics: Default::default(),
        };

        assert_eq!(
            order.clone().with_y_slop(0.5).cmps.as_slice(),
            &[CurveOrderEntry {
                end: 1.0,
                order: Order::Ish
            }]
        );
    }

    #[test]
    fn y_slop_short_end_order() {
        let order = CurveOrder {
            start: 0.0,
            cmps: [
                CurveOrderEntry {
                    end: 0.9,
                    order: Order::Ish,
                },
                CurveOrderEntry {
                    end: 1.0,
                    order: Order::Left,
                },
            ]
            .into_iter()
            .collect(),
            metrics: Default::default(),
        };

        assert_eq!(
            order.clone().with_y_slop(0.5).cmps.as_slice(),
            &[CurveOrderEntry {
                end: 1.0,
                order: Order::Ish
            }]
        );
    }

    #[test]
    fn y_slop_no_height() {
        let order = CurveOrder {
            start: 1.0,
            cmps: [CurveOrderEntry {
                end: 1.0,
                order: Order::Right,
            }]
            .into_iter()
            .collect(),
            metrics: Default::default(),
        };

        assert_eq!(order.clone().with_y_slop(0.5).cmps, order.cmps);
    }

    // Cases where two curves share just a single y coordinate.
    #[test]
    fn curve_order_no_overlap() {
        let c0 = CubicBez {
            p0: (0.0, 0.0).into(),
            p1: (0.0, 0.0).into(),
            p2: (0.0, 1.0).into(),
            p3: (0.0, 1.0).into(),
        };
        let c1 = CubicBez {
            p0: (1.0, 1.0).into(),
            p1: (1.0, 1.0).into(),
            p2: (1.0, 2.0).into(),
            p3: (1.0, 2.0).into(),
        };
        let order = intersect_cubics(c0, c1, 0.125, 0.25);
        assert_eq!(order.start, 1.0);
        assert_eq!(
            &order.cmps[..],
            &[CurveOrderEntry {
                end: 1.0,
                order: Order::Left
            }]
        );

        let c1 = CubicBez {
            p0: (1.0, 1.0).into(),
            p1: (1.0, 1.0).into(),
            p2: (0.0, 2.0).into(),
            p3: (0.0, 2.0).into(),
        };
        let order = intersect_cubics(c0, c1, 0.125, 0.25);
        assert_eq!(order.start, 1.0);
        assert_eq!(
            &order.cmps[..],
            &[CurveOrderEntry {
                end: 1.0,
                order: Order::Left
            }]
        );

        let c1 = CubicBez {
            p0: (0.1, 1.0).into(),
            p1: (0.1, 1.0).into(),
            p2: (0.0, 2.0).into(),
            p3: (0.0, 2.0).into(),
        };
        let order = intersect_cubics(c0, c1, 0.125, 0.25);
        assert_eq!(order.start, 1.0);
        assert_eq!(
            &order.cmps[..],
            &[CurveOrderEntry {
                end: 1.0,
                order: Order::Ish
            }]
        );
    }

    // This test recurses a bit too deeply. The two curves are quite close,
    // and near the end they're just a bit more than eps apart.
    #[test]
    fn painted_dreams_deep_recursion() {
        let sure_tol = 0.5e-7;
        let ish_tol = 1.5e-7;
        let c0 = CubicBez::new(
            (268.2700181987094, 465.9600610772311),
            (261.5036543284458, 474.04917281766745),
            (255.01588390924064, 481.16474559246694),
            (248.76144328725377, 487.4191862144538),
        );
        let c1 = CubicBez::new(
            (268.2700181987094, 465.9600610772311),
            (261.508835610275, 474.04297901254995),
            (255.02582000674386, 481.1538483487484),
            (248.77581162941254, 487.404817872295),
        );

        dbg!(intersect_cubics(c0, c1, sure_tol, ish_tol));
    }

    #[test]
    fn quadratic_comparison() {
        let c0 = CubicBez::new((0.0, 0.0), (10.0, 10.0), (20.0, 20.0), (30.0, 30.0));
        let c1 = CubicBez::new((30.0, 0.0), (20.0, 10.0), (10.0, 20.0), (0.0, 30.0));
        let sure_tolerance = 1.0;
        let ish_tolerance = 2.0;

        let qa0 = QuadraticApprox::from_cubic(c0, None);
        let qa1 = QuadraticApprox::from_cubic(c1, None);
        assert_eq!(
            3,
            qa0.compare(&qa1, sure_tolerance, ish_tolerance, 0.0, 30.0)
                .entries
                .len()
        );

        let c0 = CubicBez::new((0.0, 0.0), (100.0, 10.0), (200.0, 20.0), (300.0, 30.0));
        let c1 = CubicBez::new((3.0, 0.0), (103.0, 10.0), (203.0, 20.0), (303.0, 30.0));

        let qa0 = QuadraticApprox::from_cubic(c0, Some(Direction::Increasing));
        let qa1 = QuadraticApprox::from_cubic(c1, Some(Direction::Increasing));
        assert_eq!(
            1,
            qa0.compare(&qa1, sure_tolerance, ish_tolerance, 0.0, 30.0)
                .entries
                .len()
        );

        let c1 = CubicBez::new((150.0, 0.0), (150.0, 10.0), (150.0, 20.0), (150.0, 30.0));

        let qa0 = QuadraticApprox::from_cubic(c0, Some(Direction::Increasing));
        let qa1 = QuadraticApprox::from_cubic(c1, Some(Direction::Increasing));
        assert_eq!(
            3,
            qa0.compare(&qa1, sure_tolerance, ish_tolerance, 0.0, 30.0)
                .entries
                .len()
        );
    }

    // TODO: improve this test
    #[test]
    fn break_horizontal() {
        let c0 = CubicBez::new((0.0, 0.0), (10.0, 10.0), (20.0, 20.0), (30.0, 30.0));
        dbg!(AlmostHorizontalDecomposition::from_monotone_cubic(c0));

        let c0 = CubicBez::new((0.0, 0.0), (10.0, 10.0), (20.0, 20.0), (500.0, 30.0));
        dbg!(AlmostHorizontalDecomposition::from_monotone_cubic(c0));

        let c0 = CubicBez::new((0.0, 0.0), (2000.0, 10.0), (-1000.0, 20.0), (1000.0, 30.0));
        dbg!(AlmostHorizontalDecomposition::from_monotone_cubic(c0));

        let c0 = CubicBez::new((0.0, 0.0), (2000.0, 0.0), (-1000.0, 30.0), (1000.0, 30.0));
        dbg!(AlmostHorizontalDecomposition::from_monotone_cubic(c0));
    }
}
