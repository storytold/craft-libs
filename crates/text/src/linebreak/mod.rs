//! Line breaking on Knuth and Plass's box/glue/penalty model, independent of fonts.
//!
//! A composer turns a paragraph's shaped glyphs into [`Item`]s (boxes for glyphs or words, glue
//! for spaces, penalties for other break opportunities), calls a breaker, and places the glyphs of
//! each returned [`Line`]. Widths are in the caller's units.
//!
//! - [`knuth_plass`]: one total-fit pass at [`Params::tolerance`].
//! - [`break_paragraph`]: Knuth–Plass passes at rising tolerances, the last accepting overfull
//!   lines, then [`greedy`] if no pass succeeds. [`Breaks::method`] says which produced the lines.
//! - [`greedy`] / [`greedy_with`]: first fit, each line takes as much as fits.
//! - [`balanced`]: the line count of [`break_paragraph`] with lines as even as possible.
//! - [`overlong`]: runs of boxes wider than the measure, where a composer may add
//!   [`Item::emergency`] breaks.
//!
//! Elasticity comes in [`TIERS`] tiers, used in order: tier 0 (word spaces), then tier 1 (letter
//! spacing), then tier 2 (glyph scaling). Each tier is charged [`Params::tier_cost`] per unit of
//! its ratio, so a breaker prefers word-space changes, then letter spacing, then glyph scaling.
//!
//! Breakers take the measure of line `k` from `width(k)`. They clamp each measure to
//! `0.0..=f64::MAX` (NaN counts as 0) and treat lines from [`Params::uniform_from`] on as having
//! the same measure.

mod greedy;
mod knuth_plass;
#[cfg(test)]
mod tests;

use std::ops::Range;

pub use greedy::{greedy, greedy_with};
pub use knuth_plass::knuth_plass;

/// Number of elasticity tiers: word space, letter space, glyph scale.
pub const TIERS: usize = 3;

/// Stretch or shrink per tier.
pub type Tiers = [f64; TIERS];

/// A penalty at least this high forbids a break; one at most its negative forces a break.
pub const INFINITE_PENALTY: f64 = 10_000.0;

/// Cost of an [`Item::emergency`] break: high, so a paragraph takes no more than it must, but legal.
pub const EMERGENCY_PENALTY: f64 = 5_000.0;

/// What an [`Item`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// Fixed material (a glyph, a word). Never a break.
    Box,
    /// Elastic space. A break when it follows a box; discarded at the start of a line.
    Glue,
    /// A break opportunity with a cost; adds its width when a line ends there.
    Penalty,
}

/// Tier-0 stretch and shrink that a line gains at one of its ends (see [`Item::edge`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edge {
    pub stretch: f64,
    pub shrink: f64,
}

/// One item of a paragraph.
///
/// Build items from a constructor ([`Item::boxed`], [`Item::glue`], [`Item::penalty`], …) and
/// override the optional fields with struct update syntax, such as
/// `Item { hang: 2.0, ..Item::glue(3.0, 1.5, 0.6) }`. Fields added later get neutral values in the
/// constructors, so code written this way keeps compiling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    /// Natural width (boxes, glue), or the width a line broken here gains (penalties: a hyphen).
    pub width: f64,
    /// Stretch per tier. Boxes may carry letter-spacing and glyph-scaling stretch.
    pub stretch: Tiers,
    /// Shrink per tier.
    pub shrink: Tiers,
    /// Glue: infinite stretch. A line holding fill glue gives it all its leftover space.
    pub fill: bool,
    /// Glue: the unit in which stretch past every tier is rated. Lines whose glue has none rate it
    /// against their total stretch.
    pub overflow: f64,
    /// Penalty cost: [`INFINITE_PENALTY`] forbids the break, its negative forces it.
    pub cost: f64,
    /// Penalty: a flagged (hyphen) break. Consecutive flagged lines cost
    /// [`Params::flagged_demerits`] and count toward [`Params::hyphen_limit`].
    pub flagged: bool,
    /// Breaks: width a line ending here gives back (punctuation hanging into the margin).
    /// Boxes: width a line starting with this box gives back.
    pub hang: f64,
    /// Tier-0 stretch and shrink that a line ending at this break (glue, penalties) or starting
    /// with this box gains; negative values remove elasticity. The line's tier-0 totals are
    /// clamped at zero.
    pub edge: Edge,
    /// Penalty: an automatic or discretionary hyphenation point, subject to
    /// [`Rag::hyphenation_zone`].
    pub hyphenation: bool,
    /// Penalty: a break inside a word wider than the measure. A justified line ending here that
    /// must stretch is rated by its leftover space as a fraction of the measure, so the fullest
    /// piece wins.
    pub emergency: bool,
}

const BLANK: Item = Item {
    kind: Kind::Box,
    width: 0.0,
    stretch: [0.0; TIERS],
    shrink: [0.0; TIERS],
    fill: false,
    overflow: 0.0,
    cost: 0.0,
    flagged: false,
    hang: 0.0,
    edge: Edge { stretch: 0.0, shrink: 0.0 },
    hyphenation: false,
    emergency: false,
};

impl Item {
    /// A box of `width`.
    pub const fn boxed(width: f64) -> Item {
        Item { width, ..BLANK }
    }
    /// Glue of natural `width` with tier-0 `stretch` and `shrink`.
    pub const fn glue(width: f64, stretch: f64, shrink: f64) -> Item {
        Item { kind: Kind::Glue, width, stretch: [stretch, 0.0, 0.0], shrink: [shrink, 0.0, 0.0], ..BLANK }
    }
    /// Glue of zero width and infinite stretch: ends a paragraph (before [`Item::forced`]) whose
    /// last line is not justified.
    pub const fn fill() -> Item {
        Item { kind: Kind::Glue, fill: true, ..BLANK }
    }
    /// A break opportunity costing `cost`, adding `width` when taken.
    pub const fn penalty(width: f64, cost: f64, flagged: bool) -> Item {
        Item { kind: Kind::Penalty, width, cost, flagged, ..BLANK }
    }
    /// A forced break.
    pub const fn forced() -> Item {
        Item::penalty(0.0, -INFINITE_PENALTY, false)
    }
    /// A break that is never taken (keeps [`greedy`] from splitting two boxes).
    pub const fn no_break() -> Item {
        Item::penalty(0.0, INFINITE_PENALTY, false)
    }
    /// An emergency break inside an overlong word (see [`overlong`]).
    pub const fn emergency() -> Item {
        Item { emergency: true, ..Item::penalty(0.0, EMERGENCY_PENALTY, false) }
    }
    fn is_forced(&self) -> bool {
        self.kind == Kind::Penalty && self.cost <= -INFINITE_PENALTY
    }
    fn is_flagged(&self) -> bool {
        self.kind == Kind::Penalty && self.flagged
    }
}

/// Ragged (unjustified) setting. Spaces keep their width; a line is rated by its leftover space
/// against a rag stretch, as if every line ended with glue of that stretch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rag {
    /// Rag stretch of every line, in width units.
    pub stretch: f64,
    /// Rag stretch of every line, as a fraction of its measure (added to `stretch`).
    pub width_fraction: f64,
    /// A line ending at a forced break that is shorter than this fraction of its measure is rated
    /// by the shortfall instead of its leftover space (0 = off). Such lines otherwise cost nothing.
    pub short_last: f64,
    /// The unit of that shortfall, as a fraction of the measure.
    pub short_last_unit: f64,
    /// Hyphenation zone: a [`Item::hyphenation`] break is not taken when breaking at the space
    /// before its word would leave less than this much space at the end of the line (0 = off).
    pub hyphenation_zone: f64,
}

impl Default for Rag {
    fn default() -> Self {
        Rag { stretch: 0.0, width_fraction: 1.0 / 6.0, short_last: 0.0, short_last_unit: 1.0 / 3.0, hyphenation_zone: 0.0 }
    }
}

/// How lines are rated and chosen.
///
/// Breakers clamp `flagged_demerits` and `fitness_demerits` to at least 0, with NaN counting as 0
/// (negative demerits would let pruning drop the best candidate). Other fields are used as given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Largest adjustment ratio of a line not ending at a forced break ([`knuth_plass`]).
    pub tolerance: f64,
    /// When no line reaches a break, accept the overfull line from the last node that could
    /// ([`knuth_plass`]), so the pass always succeeds.
    pub overfull: bool,
    /// Added to every line's badness before squaring.
    pub line_penalty: f64,
    /// Added when two consecutive lines end at flagged breaks.
    pub flagged_demerits: f64,
    /// Added when neighbouring lines' fitness classes differ by more than one.
    pub fitness_demerits: f64,
    /// Upper ratio bounds of fitness classes 0 (tight, `r <` bound), 1 and 2 (`r <=` bound);
    /// class 3 (very loose) is above.
    pub fitness: [f64; 3],
    /// Fitness class assumed before the first line; `None`: the first line pays no fitness demerits.
    pub first_fitness: Option<u8>,
    /// Most consecutive lines ending at flagged breaks (0 = unlimited). A hard constraint: a
    /// Knuth–Plass pass that cannot meet it fails, and [`greedy`] breaks before the word instead.
    pub hyphen_limit: usize,
    /// Ratio charged per unit of each tier.
    pub tier_cost: Tiers,
    /// Badness cap (badness is `100·|r|³`, times the weight below).
    pub max_badness: f64,
    /// Badness multiplier of stretched lines.
    pub stretch_weight: f64,
    /// Badness multiplier of shrunk lines. [`greedy`] avoids shrinking when this exceeds
    /// `stretch_weight`.
    pub shrink_weight: f64,
    /// A line longer than its measure by at most this fits exactly (rounded widths).
    pub overrun: f64,
    /// First line index from which `width(k)` no longer changes. [`knuth_plass`] keeps the ways
    /// to reach a break apart by line number only for earlier lines; from here on they compete
    /// directly, which keeps the number of active breaks small.
    ///
    /// The default 0 suits a constant measure. A caller whose measures vary (a first-line indent,
    /// a drop cap, a shaped frame) must set it to the first index of the constant tail, such as 1
    /// for a first-line indent; with a smaller value the lines still get their own measures, but
    /// the breaks may not be the best ones. Large values make long paragraphs slow and can exceed
    /// the active-break limit, so that [`break_paragraph`] falls back to [`greedy`].
    pub uniform_from: usize,
    /// Ragged setting; `None` justifies.
    pub rag: Option<Rag>,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            tolerance: 2.5,
            overfull: false,
            line_penalty: 10.0,
            flagged_demerits: 3000.0,
            fitness_demerits: 3000.0,
            fitness: [-0.5, 0.5, 1.0],
            first_fitness: Some(1),
            hyphen_limit: 0,
            tier_cost: [1.0, 0.75, 1.0],
            max_badness: f64::INFINITY,
            stretch_weight: 1.0,
            shrink_weight: 1.0,
            overrun: 1e-6,
            uniform_from: 0,
            rag: None,
        }
    }
}

impl Params {
    /// These parameters with the clamps documented on [`Params`] applied.
    fn sanitized(&self) -> Params {
        let non_negative = |x: f64| if x.is_nan() { 0.0 } else { x.max(0.0) };
        Params { flagged_demerits: non_negative(self.flagged_demerits), fitness_demerits: non_negative(self.fitness_demerits), ..*self }
    }
}

/// A measure from a caller's `width`, clamped to `0.0..=f64::MAX` (NaN counts as 0).
fn clamp_measure(m: f64) -> f64 {
    if m.is_nan() { 0.0 } else { m.clamp(0.0, f64::MAX) }
}

/// One line chosen by a breaker.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Line {
    /// First item on the line.
    pub start: usize,
    /// The break: items `start..end` are on the line. `items.len()` for a paragraph that does not
    /// end with a forced break; a box's index where [`greedy`] split a word with no break in it.
    pub end: usize,
    /// First item of the next line (glue and penalties after the break are discarded).
    pub next: usize,
    /// Natural width: boxes and glue on the line, plus the break's penalty width, minus hangs.
    pub width: f64,
    /// The line's measure.
    pub measure: f64,
    /// Adjustment ratio as rated: positive stretches, negative shrinks. Each tier adds
    /// [`Params::tier_cost`] per unit used, so a line that fits may shrink below −1; an overfull
    /// line has a negative [`excess`](Self::excess) instead.
    pub ratio: f64,
    /// Fraction of each tier's stretch (positive) or shrink (negative) the line uses.
    pub tiers: Tiers,
    /// Width the tiers leave: taken by fill glue or the rag, stretch past every tier when
    /// justified (spread over the glue's `overflow`, else its stretch), or the overfull amount
    /// when negative.
    pub excess: f64,
    /// Ended at a flagged penalty.
    pub flagged: bool,
    /// Ended at a forced break or the paragraph end.
    pub forced: bool,
}

/// Runs of boxes, as item ranges, whose width fully shrunk exceeds `measure`: words no line can
/// hold. A composer adds [`Item::emergency`] breaks inside them where its script allows.
pub fn overlong(items: &[Item], measure: f64) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    // (first box, last box, width)
    let mut run: Option<(usize, usize, f64)> = None;
    let mut close = |run: &mut Option<(usize, usize, f64)>| {
        if let Some((first, last, w)) = run.take()
            && w > measure
        {
            out.push(first..last + 1);
        }
    };
    for (k, it) in items.iter().enumerate() {
        match it.kind {
            Kind::Box => {
                let w = it.width - it.shrink.iter().sum::<f64>();
                run = Some(run.map_or((k, k, w), |(first, _, acc)| (first, k, acc + w)));
            }
            Kind::Penalty if it.cost >= INFINITE_PENALTY => {}
            Kind::Glue | Kind::Penalty => close(&mut run),
        }
    }
    close(&mut run);
    out
}

/// Which breaker produced a paragraph's lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Method {
    /// The [`knuth_plass`] pass at `tolerances[pass]`.
    KnuthPlass { pass: usize },
    /// [`greedy`], because every Knuth–Plass pass failed or there were none.
    Greedy,
}

/// Lines from [`break_paragraph`] or [`balanced`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Breaks {
    pub lines: Vec<Line>,
    /// The breaker that produced `lines`.
    pub method: Method,
    /// [`balanced`] evened the lines. Always false from [`break_paragraph`].
    pub balanced: bool,
}

/// [`knuth_plass`] at each of `tolerances` in turn, the last pass also accepting overfull lines;
/// [`greedy`] if every pass fails or `tolerances` is empty. The last pass fails only past the
/// active-break limit (see [`Params::uniform_from`]).
pub fn break_paragraph(items: &[Item], width: &dyn Fn(usize) -> f64, params: &Params, tolerances: &[f64]) -> Breaks {
    for (pass, &tolerance) in tolerances.iter().enumerate() {
        let overfull = params.overfull || pass + 1 == tolerances.len();
        if let Some(lines) = knuth_plass(items, width, &Params { tolerance, overfull, ..*params }) {
            return Breaks { lines, method: Method::KnuthPlass { pass }, balanced: false };
        }
    }
    Breaks { lines: greedy(items, width, params), method: Method::Greedy, balanced: false }
}

/// Paragraphs with more items than this are not balanced (each balancing step breaks the whole
/// paragraph again).
pub const MAX_BALANCED_ITEMS: usize = 6000;

/// Balance lines: keep [`break_paragraph`]'s line count but make the lines as even as possible,
/// by breaking against the narrowest scaled measure that still gives that many lines. The lines
/// are measured against the real `width`. Meant for ragged text; justified text is balanced the
/// same way, so its lines stretch more.
///
/// Returns [`break_paragraph`]'s lines with [`Breaks::balanced`] false when the paragraph has
/// fewer than two lines, a forced break before its last line, more than [`MAX_BALANCED_ITEMS`]
/// items, or when no narrower measure keeps the line count.
pub fn balanced(items: &[Item], width: &dyn Fn(usize) -> f64, params: &Params, tolerances: &[f64]) -> Breaks {
    let base = break_paragraph(items, width, params, tolerances);
    let n = base.lines.len();
    if n < 2 || items.len() > MAX_BALANCED_ITEMS || base.lines[..n - 1].iter().any(|l| l.forced) {
        return base;
    }
    let attempt = |f: f64| -> Option<Breaks> {
        let w = |j: usize| clamp_measure(width(j)) * f;
        let b = break_paragraph(items, &w, params, tolerances);
        (b.lines.len() == n && b.lines.iter().enumerate().all(|(j, l)| l.width <= w(j) + params.overrun)).then_some(b)
    };
    let (mut lo, mut hi) = (0.3, 1.0);
    let mut best = None;
    for _ in 0..14 {
        let mid = (lo + hi) / 2.0;
        match attempt(mid) {
            Some(lines) => {
                best = Some(lines);
                hi = mid;
            }
            None => lo = mid,
        }
    }
    let Some(best) = best else { return base };
    let sums = Sums::new(items);
    let p = params.sanitized();
    let lines = best.lines.iter().enumerate().map(|(j, l)| sums.line(l.start, l.end, l.next, clamp_measure(width(j)), &p, 0.0)).collect();
    Breaks { lines, method: best.method, balanced: true }
}

/// How a line fits its measure.
#[derive(Clone, Copy, Debug)]
struct Fit {
    width: f64,
    ratio: f64,
    tiers: Tiers,
    excess: f64,
    /// Fits with the shrink available.
    feasible: bool,
}

/// Prefix sums of a paragraph's items, for measuring any line in constant time.
struct Sums<'a> {
    items: &'a [Item],
    /// Before item `k`: width of boxes and glue, their stretch and shrink, glue overflow and the
    /// number of fill glues.
    w: Vec<f64>,
    y: Vec<Tiers>,
    z: Vec<Tiers>,
    over: Vec<f64>,
    fill: Vec<u32>,
}

impl<'a> Sums<'a> {
    fn new(items: &'a [Item]) -> Self {
        let m = items.len();
        let (mut w, mut y, mut z, mut over, mut fill) =
            (Vec::with_capacity(m + 1), Vec::with_capacity(m + 1), Vec::with_capacity(m + 1), Vec::with_capacity(m + 1), Vec::with_capacity(m + 1));
        let (mut sw, mut sy, mut sz, mut so, mut sf) = (0.0, [0.0; TIERS], [0.0; TIERS], 0.0, 0);
        for it in items {
            w.push(sw);
            y.push(sy);
            z.push(sz);
            over.push(so);
            fill.push(sf);
            if it.kind != Kind::Penalty {
                sw += it.width;
                for t in 0..TIERS {
                    sy[t] += it.stretch[t];
                    sz[t] += it.shrink[t];
                }
            }
            if it.kind == Kind::Glue {
                so += it.overflow;
                sf += u32::from(it.fill);
            }
        }
        w.push(sw);
        y.push(sy);
        z.push(sz);
        over.push(so);
        fill.push(sf);
        Sums { items, w, y, z, over, fill }
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    /// The paragraph ends with a forced penalty (else its end is an implicit forced break).
    fn ends_forced(&self) -> bool {
        self.items.last().is_some_and(Item::is_forced)
    }

    /// Is `b` a legal break? `len()` is one unless the items end with a forced penalty.
    fn is_break(&self, b: usize) -> bool {
        match self.items.get(b) {
            None => b == self.len() && !self.ends_forced(),
            Some(it) => match it.kind {
                Kind::Box => false,
                Kind::Penalty => it.cost < INFINITE_PENALTY,
                Kind::Glue => b > 0 && self.items.get(b - 1).is_some_and(|p| p.kind == Kind::Box),
            },
        }
    }

    fn forced(&self, b: usize) -> bool {
        self.items.get(b).map_or(b >= self.len(), Item::is_forced)
    }

    /// First item of the line after a break at `b`: glue and penalties are discarded up to the next
    /// box or forced break.
    fn next_start(&self, b: usize) -> usize {
        let mut i = b + 1;
        while let Some(it) = self.items.get(i) {
            if it.kind == Kind::Box || it.is_forced() {
                break;
            }
            i += 1;
        }
        i.min(self.len())
    }

    fn at(&self, k: usize) -> (f64, Tiers, Tiers, f64, u32) {
        let k = k.min(self.len());
        (
            self.w.get(k).copied().unwrap_or(0.0),
            self.y.get(k).copied().unwrap_or_default(),
            self.z.get(k).copied().unwrap_or_default(),
            self.over.get(k).copied().unwrap_or(0.0),
            self.fill.get(k).copied().unwrap_or(0),
        )
    }

    /// How items `start..end` fit `measure`, ending at `brk` (a glue or penalty whose hang, edge
    /// elasticity and penalty width apply; `None` at the paragraph end or a split between boxes).
    /// `extra` is added to the natural width (boxes whose width depends on position).
    #[allow(clippy::too_many_arguments)]
    fn fit(&self, start: usize, end: usize, brk: Option<&Item>, forced: bool, measure: f64, p: &Params, extra: f64) -> Fit {
        let (w0, y0, z0, o0, f0) = self.at(start);
        let (w1, y1, z1, o1, f1) = self.at(end);
        let (start_hang, start_edge) =
            self.items.get(start).filter(|i| i.kind == Kind::Box && end > start).map_or((0.0, Edge::default()), |b| (b.hang, b.edge));
        let (pw, end_hang, end_edge, emergency) = brk
            .filter(|i| i.kind != Kind::Box)
            .map_or((0.0, 0.0, Edge::default(), false), |i| (if i.kind == Kind::Penalty { i.width } else { 0.0 }, i.hang, i.edge, i.emergency));
        let width = w1 - w0 + pw - start_hang - end_hang + extra;
        let mut y = [0.0; TIERS];
        let mut z = [0.0; TIERS];
        for t in 0..TIERS {
            y[t] = y1[t] - y0[t];
            z[t] = z1[t] - z0[t];
        }
        y[0] = (y[0] + start_edge.stretch + end_edge.stretch).max(0.0);
        z[0] = (z[0] + start_edge.shrink + end_edge.shrink).max(0.0);
        let mut d = measure - width;
        if d < 0.0 && d > -p.overrun {
            d = 0.0;
        }
        let free = |ratio: f64| Fit { width, ratio, tiers: [0.0; TIERS], excess: d, feasible: true };
        if d < 0.0 {
            let (ratio, tiers, excess, feasible) = tiered(d, &z, 0.0, &p.tier_cost);
            return Fit { width, ratio, tiers, excess, feasible };
        }
        if let Some(rag) = p.rag {
            if forced && rag.short_last > 0.0 {
                let short = measure * rag.short_last - width;
                let unit = measure * rag.short_last_unit;
                return free(if short <= 0.0 {
                    0.0
                } else if unit > 0.0 {
                    short / unit
                } else {
                    f64::INFINITY
                });
            }
            if f1 > f0 {
                return free(0.0);
            }
            let s = rag.stretch + rag.width_fraction * measure;
            return free(if d <= 0.0 {
                0.0
            } else if s > 0.0 {
                d / s
            } else {
                f64::INFINITY
            });
        }
        if f1 > f0 {
            return free(0.0);
        }
        let (mut ratio, tiers, excess, feasible) = tiered(d, &y, o1 - o0, &p.tier_cost);
        if emergency && ratio > 0.0 && measure > 0.0 {
            ratio = d / measure;
        }
        Fit { width, ratio, tiers, excess, feasible }
    }

    /// Measure the break at `b` for a line starting at `start` (`b == len()`: the paragraph end).
    fn measure(&self, start: usize, b: usize, measure: f64, p: &Params) -> Fit {
        self.fit(start, b, self.items.get(b), self.forced(b), measure, p, 0.0)
    }

    fn line(&self, start: usize, end: usize, next: usize, measure: f64, p: &Params, extra: f64) -> Line {
        let brk = self.items.get(end).filter(|i| i.kind != Kind::Box);
        let forced = brk.map_or(end >= self.len(), Item::is_forced);
        let f = self.fit(start, end, brk, forced, measure, p, extra);
        Line {
            start,
            end,
            next,
            width: f.width,
            measure,
            ratio: f.ratio,
            tiers: f.tiers,
            excess: f.excess,
            flagged: brk.is_some_and(Item::is_flagged),
            forced,
        }
    }
}

/// Adjustment ratio for `d` units of stretch (`d > 0`) or shrink (`d < 0`) from tier capacities
/// `cap`, used in tier order and charged `cost` per unit of each tier's ratio. Returns the ratio,
/// the fraction of each tier used (negative when shrinking), the width left over and whether the
/// line is feasible: shrinking past every tier is not. Stretch past every tier is rated in units
/// of `over`, or of the total stretch when `over` is zero.
fn tiered(d: f64, cap: &Tiers, over: f64, cost: &Tiers) -> (f64, Tiers, f64, bool) {
    const EPS: f64 = 1e-9;
    let mut used = [0.0; TIERS];
    if d.abs() < EPS {
        return (0.0, used, d, true);
    }
    let sign = d.signum();
    let mut rem = d.abs();
    let mut r = 0.0;
    for t in 0..TIERS {
        if cap[t] > EPS && rem > EPS {
            let take = rem.min(cap[t]);
            r += cost[t] * take / cap[t];
            used[t] = sign * take / cap[t];
            rem -= take;
        }
    }
    let excess = sign * rem;
    if d < 0.0 {
        return (-r, used, excess, rem <= 1e-6);
    }
    if rem > 1e-6 {
        let unit = if over > EPS { over } else { cap.iter().sum::<f64>() };
        r = if unit > EPS { r + rem / unit } else { f64::INFINITY };
    }
    (r, used, excess, true)
}

/// Fitness class 0 (tight) … 3 (very loose) of ratio `r`.
fn fitness_class(r: f64, bounds: &[f64; 3]) -> u8 {
    if r < bounds[0] {
        0
    } else if r <= bounds[1] {
        1
    } else if r <= bounds[2] {
        2
    } else {
        3
    }
}
