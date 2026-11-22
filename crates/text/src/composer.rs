//! Every-line composer: a Knuth–Plass style total-fit line breaker.
//!
//! Boxes are the paragraph's glyphs, glue is its spaces, and penalties are break opportunities
//! after hyphens/dashes/CJK and at hyphenation points. The breaks minimise the sum of squared
//! (badness + penalty) demerits over the whole paragraph, plus a demerit when neighbouring lines
//! fall in fitness classes more than one apart (a tight line next to a loose one).
//!
//! - **Justified** lines: word spacing, letter spacing and glyph scaling may stretch and shrink
//!   within the paragraph's Justification options (each glyph's [`Flexibility`](crate::composer::Flexibility)); the badness comes from
//!   the stretch/shrink ratio, so loose lines are traded for evenly spaced ones.
//! - **Ragged** lines (left/centre/right aligned): spaces keep their width, so a line never
//!   shrinks (it must fit). The badness comes from the line's leftover space measured against a
//!   rag zone of a sixth of the line width, so the rag is even rather than full lines followed by
//!   a short one, and hyphenating costs less. The last line costs nothing unless it is shorter
//!   than a third of the width (a word or two left dangling).

/// Glyph measurements needed by the paragraph composer.
pub trait Glyph {
    /// The glyph's natural width.
    fn advance(&self) -> f64;
    /// Whether the glyph is a space (glue between words).
    fn is_space(&self) -> bool;
    /// Whether the glyph is a soft hyphen (a break opportunity that shows a hyphen when taken).
    fn is_soft_hyphen(&self) -> bool;
}

/// Stretch and shrink available for a glyph.
pub trait Flexibility {
    /// How much the glyph's width may grow.
    fn stretch(&self) -> f64;
    /// How much the glyph's width may shrink.
    fn shrink(&self) -> f64;
}

const HYPHEN_PENALTY: f64 = 50.0;
const DOUBLE_HYPHEN_DEMERITS: f64 = 3000.0;
const LINE_PENALTY: f64 = 10.0;
/// Demerits for adjacent lines whose fitness classes differ by more than one.
const ADJ_DEMERITS_JUSTIFIED: f64 = 10_000.0;
const ADJ_DEMERITS_RAGGED: f64 = 1_000.0;
/// Ragged lines only gain from hyphenation by filling the rag, so it costs less than in
/// justified text (where it also evens the word spacing).
const HYPHEN_PENALTY_RAGGED: f64 = 25.0;
/// Ragged text: leftover space is measured in units of this fraction of the line width.
const RAG_ZONE: f64 = 1.0 / 6.0;
/// Ragged text: a last line shorter than this fraction of the width is penalised, by how much
/// shorter it is in units of `SHORT_LAST_ZONE` of the width.
const SHORT_LAST: f64 = 1.0 / 3.0;
const SHORT_LAST_ZONE: f64 = 1.0 / 3.0;
/// Give up (fall back to the single-line composer) beyond this many active breakpoints.
const MAX_ACTIVE: usize = 4096;
const EPS: f64 = 1e-6;

/// A place a line may end: glyph index `end` (exclusive), plus the width of the hyphen that is
/// added when breaking there (0 = no hyphen).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Breakpoint {
    /// Index of the glyph the line ends before (exclusive).
    pub end: usize,
    /// Width of the hyphen added when breaking here; 0 when none is added.
    pub hyphen: f64,
}

/// How a paragraph is composed.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// The last line is justified too (Justify All).
    pub justify_last: bool,
    /// Left/centre/right aligned text: spaces keep their width and lines are scored by their rag.
    pub ragged: bool,
    /// Maximum stretch ratio (justified) or leftover space in rag zones (ragged) of a line.
    pub tolerance: f64,
    /// Lines from this index on all have the same width, so their line numbers are interchangeable
    /// (keeps the number of active breakpoints small).
    pub uniform_from: usize,
    /// Hyphen Limit: the most consecutive lines that may end with a hyphen before a hyphenation
    /// point is no longer a break; 0 sets no limit. A soft hyphen stays a break and counts.
    pub hyphen_limit: usize,
}

/// The search gave up: it ran past its budget ([`Measure::line`]).
#[derive(Debug)]
pub struct Exhausted;

/// Where each line of a paragraph goes and how wide it is: the measure a line gets may depend on
/// the lines set above it (their heights move it down a shaped frame, or past a wrap object).
pub trait Measure {
    /// What the measure of the next line depends on; breaks reached with equal states are
    /// interchangeable from there on.
    type State: Clone + PartialEq;
    /// The state of the paragraph's first line.
    fn first(&mut self) -> Self::State;
    /// The width of the line of glyphs `pos..end` set from `state`, and the state of the line
    /// after it; `Ok(None)` when the line can't be set (the frame is full), and [`Exhausted`] when
    /// the search is to stop.
    fn line(&mut self, state: &Self::State, pos: usize, end: usize) -> Result<Option<(f64, Self::State)>, Exhausted>;
    /// Whether a line from `state` longer than `pos..end`, which [`Measure::line`] just measured,
    /// may get more room than it. While it may, a line from `state` that is too long, or that
    /// can't be set, does not end the reach of `state`. Lines from a state are all as wide by
    /// default.
    fn may_widen(&mut self, _state: &Self::State, _pos: usize, _end: usize) -> bool {
        false
    }
}

/// Line widths by line number: `width(k)` for line `k`, lines from `uniform_from` on all as wide.
struct ByLine<'a> {
    width: &'a dyn Fn(usize) -> f64,
    uniform_from: usize,
}

impl Measure for ByLine<'_> {
    type State = usize;
    fn first(&mut self) -> usize {
        0
    }
    fn line(&mut self, &line: &usize, _: usize, _: usize) -> Result<Option<(f64, usize)>, Exhausted> {
        Ok(Some(((self.width)(line), (line + 1).min(self.uniform_from))))
    }
}

struct Node<S> {
    pos: usize,
    /// The measure of the next line (its line number, clamped to `uniform_from`, for [`compose`]).
    state: S,
    fitness: u8,
    demerits: f64,
    /// Lines in a row ending with a hyphen up to this break (at most `hyphen_limit`, or 1 with no
    /// limit); 0 when this break has no hyphen.
    hyphens: usize,
    prev: Option<usize>,
}

/// Fitness class 0 (tight) … 3 (very loose) of a line with adjustment ratio `r`.
fn fitness(r: f64, ragged: bool) -> u8 {
    let bounds = if ragged { [0.15, 0.4, 0.75] } else { [-0.5, 0.5, 1.0] };
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

/// Adjustment ratio for a candidate line; `None` means a ragged line is already too long.
fn adjustment_ratio(natural: f64, width: f64, stretch: f64, shrink: f64, last: bool, p: &Params) -> Option<f64> {
    if p.ragged {
        if natural > width + EPS {
            return None;
        }
        return Some(if last && !p.justify_last {
            ((width * SHORT_LAST - natural) / (width * SHORT_LAST_ZONE).max(1.0)).max(0.0)
        } else {
            (width - natural).max(0.0) / (width * RAG_ZONE).max(1.0)
        });
    }
    Some(if natural < width {
        if last && !p.justify_last {
            0.0
        } else if stretch > EPS {
            (width - natural) / stretch
        } else {
            f64::INFINITY
        }
    } else if natural > width {
        if shrink > EPS { (width - natural) / shrink } else { f64::NEG_INFINITY }
    } else {
        0.0
    })
}

/// Compose the paragraph `g` into lines. `width(k)` is the available width of line `k`;
/// `start_credit(i)` is the width a wrapped line starting at glyph `i` gives back (Mojikumi sets
/// an opening bracket there flush with the line's start); `end_credit(t)` is the width a line whose
/// last character is glyph `t - 1` gives back (Mojikumi sets a closing mark there half width), at
/// most that glyph's advance, and nothing at a hyphenation point; `cands` are the break opportunities
/// in increasing order (the paragraph end is added); `flex` holds how far each glyph may stretch
/// and shrink in a justified line.
/// Returns `(end, hyphen)` per line, or `None` if no set of breaks fits within the tolerance
/// (the caller then falls back to single-line breaking).
pub fn compose<G: Glyph, F: Flexibility>(
    g: &[G],
    width: &dyn Fn(usize) -> f64,
    start_credit: &dyn Fn(usize) -> f64,
    end_credit: &dyn Fn(usize) -> f64,
    cands: &[Breakpoint],
    flex: &[F],
    p: &Params,
) -> Option<Vec<(usize, bool)>> {
    compose_with(g, &mut ByLine { width, uniform_from: p.uniform_from }, start_credit, end_credit, cands, flex, p)
}

/// [`compose`] with each line's width from `measure`, which follows where the lines above it went.
pub fn compose_with<G: Glyph, F: Flexibility, M: Measure>(
    g: &[G],
    measure: &mut M,
    start_credit: &dyn Fn(usize) -> f64,
    end_credit: &dyn Fn(usize) -> f64,
    cands: &[Breakpoint],
    flex: &[F],
    p: &Params,
) -> Option<Vec<(usize, bool)>> {
    let n = g.len();
    // The search assumes increasing break positions when indexing its prefix sums.
    // Reject malformed candidates before they can make a later line run backwards.
    if cands.windows(2).any(|pair| pair[0].end > pair[1].end) {
        return None;
    }
    if n == 0 {
        return Some(vec![]);
    }
    // Prefix sums of advances and of how far the glyphs may stretch and shrink.
    let mut w = vec![0.0; n + 1];
    let mut st = vec![0.0; n + 1];
    let mut sh = vec![0.0; n + 1];
    for (i, gl) in g.iter().enumerate() {
        let f = flex.get(i);
        w[i + 1] = w[i] + gl.advance();
        st[i + 1] = st[i] + f.map_or(0.0, Flexibility::stretch);
        sh[i + 1] = sh[i] + f.map_or(0.0, Flexibility::shrink);
    }
    if !w[n].is_finite() {
        return None;
    }
    // Trailing spaces are dropped at a break.
    let trim = |mut b: usize, a: usize| {
        while b > a && g.get(b - 1).is_some_and(Glyph::is_space) {
            b -= 1;
        }
        b
    };
    let line_end_credit = |t: usize| {
        let credit = end_credit(t);
        let most = t.checked_sub(1).and_then(|k| g.get(k)).map_or(0.0, |gl| gl.advance().max(0.0));
        if credit.is_finite() { credit.clamp(0.0, most) } else { 0.0 }
    };
    let mut all: Vec<Breakpoint> = cands.iter().copied().filter(|c| c.end > 0 && c.end < n).collect();
    all.push(Breakpoint { end: n, hyphen: 0.0 });
    let (adj_demerits, hyphen_penalty) =
        if p.ragged { (ADJ_DEMERITS_RAGGED, HYPHEN_PENALTY_RAGGED) } else { (ADJ_DEMERITS_JUSTIFIED, HYPHEN_PENALTY) };
    let mut nodes = vec![Node { pos: 0, state: measure.first(), fitness: 1, demerits: 0.0, hyphens: 0, prev: None }];
    let mut active: Vec<usize> = vec![0];
    for bp in &all {
        let last = bp.end == n;
        let hyphen = bp.hyphen > 0.0;
        // `automatic` is true for a hyphenation point and false for a soft hyphen typed in the text.
        let automatic = hyphen && !bp.end.checked_sub(1).and_then(|k| g.get(k)).is_some_and(Glyph::is_soft_hyphen);
        let mut best: Vec<(M::State, u8, usize, f64, usize)> = vec![]; // (state, fitness, hyphens, demerits, from)
        let mut keep = Vec::with_capacity(active.len());
        for &ai in &active {
            let Some(a) = nodes.get(ai) else { continue };
            let t = trim(bp.end, a.pos);
            let credit = if a.pos > 0 { start_credit(a.pos) } else { 0.0 };
            // A line that ends with a hyphen ends with it, not with its last character. At most the
            // last glyph's advance, so a longer line from the same node is never narrower.
            let end = if hyphen || t <= a.pos { 0.0 } else { line_end_credit(t) };
            let natural = w[t] - w[a.pos] - credit - end + bp.hyphen;
            // A line that can't be set (no room left in the frame) ends this node's reach, unless a
            // longer line may be placed elsewhere (a taller line steps down the frame by its own
            // leading).
            let Some((lw, next)) = measure.line(&a.state, a.pos, bp.end).ok()? else {
                if measure.may_widen(&a.state, a.pos, bp.end) {
                    keep.push(ai);
                }
                continue;
            };
            // Adjustment ratio, badness and whether the line ends the active node's reach.
            let ratio = adjustment_ratio(natural, lw, st[t] - st[a.pos], sh[t] - sh[a.pos], last, p);
            let Some(ratio) = ratio.filter(|&r| r >= -1.0 || r.is_nan()) else {
                // Too long already; longer lines from this node only get worse, unless they may
                // get more room (a taller line set lower down a shaped frame).
                if measure.may_widen(&a.state, a.pos, bp.end) {
                    keep.push(ai);
                }
                continue;
            };
            keep.push(ai);
            if automatic && p.hyphen_limit > 0 && a.hyphens >= p.hyphen_limit {
                continue;
            }
            let free_last = last && (p.ragged || !p.justify_last);
            // A line that can't stretch at all (no word spaces, nor letter spacing or glyph scaling
            // to vary) never fills its measure, however loose the tolerance.
            if (ratio > p.tolerance || ratio == f64::INFINITY) && !(last && (p.ragged || (ratio.is_infinite() && p.justify_last))) {
                continue;
            }
            // Justified lines aren't capped: past a ratio of 4.6 (only reached in the unbounded pass)
            // a looser line must still score worse than a less loose one.
            let cap = if p.ragged { 10_000.0 } else { f64::INFINITY };
            let badness = if ratio.is_finite() { (100.0 * ratio.abs().powi(3)).min(cap) } else { 10_000.0 };
            let penalty = if hyphen { hyphen_penalty } else { 0.0 };
            let mut d = (LINE_PENALTY + badness).powi(2) + penalty * penalty;
            if hyphen && a.hyphens > 0 {
                d += DOUBLE_HYPHEN_DEMERITS;
            }
            // A ragged last line's length says nothing about the rag: it keeps its neighbour's class.
            let fit = if p.ragged && free_last { a.fitness } else { fitness(ratio, p.ragged) };
            if a.prev.is_some() && fit.abs_diff(a.fitness) > 1 {
                d += adj_demerits;
            }
            let total = a.demerits + d;
            // With a limit, breaks reached with different runs of hyphenated lines stay apart.
            let hyphens = match (hyphen, p.hyphen_limit) {
                (false, _) => 0,
                (true, 0) => 1,
                (true, limit) => (a.hyphens + 1).min(limit),
            };
            match best.iter_mut().find(|b| b.0 == next && b.1 == fit && b.2 == hyphens) {
                Some(b) if b.3 <= total => {}
                Some(b) => *b = (next, fit, hyphens, total, ai),
                None => best.push((next, fit, hyphens, total, ai)),
            }
        }
        active = keep;
        for (state, fitness, hyphens, demerits, from) in best {
            nodes.push(Node { pos: bp.end, state, fitness, demerits, hyphens, prev: Some(from) });
            active.push(nodes.len() - 1);
        }
        if active.is_empty() || active.len() > MAX_ACTIVE {
            return None;
        }
    }
    let end = nodes.iter().enumerate().filter(|(_, nd)| nd.pos == n && nd.prev.is_some()).min_by(|a, b| a.1.demerits.total_cmp(&b.1.demerits))?.0;
    let mut out = vec![];
    let mut cur = Some(end);
    while let Some(i) = cur {
        let nd = nodes.get(i)?;
        if nd.prev.is_some() {
            out.push((nd.pos, nd.hyphens > 0));
        }
        cur = nd.prev;
        if out.len() > n {
            return None;
        }
    }
    out.reverse();
    Some(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_ui::testing::trial;

    struct Unit(char);
    impl Glyph for Unit {
        fn advance(&self) -> f64 {
            1.0
        }
        fn is_space(&self) -> bool {
            self.0 == ' '
        }
        fn is_soft_hyphen(&self) -> bool {
            self.0 == '\u{ad}'
        }
    }
    struct Rigid;
    impl Flexibility for Rigid {
        fn stretch(&self) -> f64 {
            0.0
        }
        fn shrink(&self) -> f64 {
            0.0
        }
    }
    #[trial]
    fn rejects_breaks_that_move_backwards() {
        let glyphs: Vec<_> = "abcd".chars().map(Unit).collect();
        let candidates = [Breakpoint { end: 3, hyphen: 0.0 }, Breakpoint { end: 1, hyphen: 0.0 }];
        let params = Params { justify_last: false, ragged: true, tolerance: 100.0, uniform_from: 0, hyphen_limit: 0 };
        assert_eq!(compose(&glyphs, &|_| 3.0, &|_| 0.0, &|_| 0.0, &candidates, &[] as &[Rigid], &params), None);
    }

    #[trial]
    fn composes_ragged_paragraph_and_trims_spaces() {
        let glyphs: Vec<_> = "aa bb cc".chars().map(Unit).collect();
        let candidates = [Breakpoint { end: 3, hyphen: 0.0 }, Breakpoint { end: 6, hyphen: 0.0 }];
        let params = Params { justify_last: false, ragged: true, tolerance: 100.0, uniform_from: 0, hyphen_limit: 0 };
        let result = compose(&glyphs, &|_| 3.0, &|_| 0.0, &|_| 0.0, &candidates, &[] as &[Rigid], &params);
        assert_eq!(result, Some(vec![(3, false), (6, false), (8, false)]));
    }
    struct Flex(f64, f64);
    impl Flexibility for Flex {
        fn stretch(&self) -> f64 {
            self.0
        }
        fn shrink(&self) -> f64 {
            self.1
        }
    }
    struct Wide(f64);
    impl Glyph for Wide {
        fn advance(&self) -> f64 {
            self.0
        }
        fn is_space(&self) -> bool {
            false
        }
        fn is_soft_hyphen(&self) -> bool {
            false
        }
    }
    fn justified() -> Params {
        Params { justify_last: false, ragged: false, tolerance: 10.0, uniform_from: 0, hyphen_limit: 0 }
    }
    fn units(text: &str) -> Vec<Unit> {
        text.chars().map(Unit).collect()
    }
    fn brk(end: usize, hyphen: f64) -> Breakpoint {
        Breakpoint { end, hyphen }
    }

    #[trial]
    fn fitness_classes_and_adjustment_ratios() {
        assert_eq!([-0.6, 0.0, 0.7, 2.0].map(|r| fitness(r, false)), [0, 1, 2, 3]);
        assert_eq!([0.1, 0.3, 0.5, 0.9].map(|r| fitness(r, true)), [0, 1, 2, 3]);
        let j = justified();
        assert_eq!(adjustment_ratio(8.0, 10.0, 4.0, 0.0, false, &j), Some(0.5));
        assert_eq!(adjustment_ratio(8.0, 10.0, 0.0, 0.0, false, &j), Some(f64::INFINITY));
        assert_eq!(adjustment_ratio(8.0, 10.0, 4.0, 0.0, true, &j), Some(0.0));
        assert_eq!(adjustment_ratio(12.0, 10.0, 0.0, 4.0, false, &j), Some(-0.5));
        assert_eq!(adjustment_ratio(12.0, 10.0, 0.0, 0.0, false, &j), Some(f64::NEG_INFINITY));
        assert_eq!(adjustment_ratio(10.0, 10.0, 0.0, 0.0, false, &j), Some(0.0));
        let all = Params { justify_last: true, ..j };
        assert_eq!(adjustment_ratio(8.0, 10.0, 4.0, 0.0, true, &all), Some(0.5));
        let ragged = Params { ragged: true, ..j };
        assert_eq!(adjustment_ratio(11.0, 10.0, 0.0, 0.0, false, &ragged), None);
    }

    #[trial]
    fn empty_and_unmeasurable_paragraphs() {
        let none: [Unit; 0] = [];
        assert_eq!(compose(&none, &|_| 3.0, &|_| 0.0, &|_| 0.0, &[], &[] as &[Rigid], &justified()), Some(vec![]));
        let huge = [Wide(f64::INFINITY)];
        assert_eq!(compose(&huge, &|_| 3.0, &|_| 0.0, &|_| 0.0, &[], &[] as &[Rigid], &justified()), None);
    }

    #[trial]
    fn justified_lines_stretch_and_shrink_within_tolerance() {
        let glyphs = units("a a b b");
        let flex: Vec<_> = glyphs.iter().map(|g| if g.0 == ' ' { Flex(1.0, 0.5) } else { Flex(0.0, 0.0) }).collect();
        let cands = [brk(4, 0.0)];
        // "a a" (the trailing space is dropped) stretches its inner space to fill 4 units...
        let lines = compose(&glyphs, &|_| 4.0, &|_| 0.0, &|_| 0.0, &cands, &flex, &justified());
        assert_eq!(lines, Some(vec![(4, false), (7, false)]));
        // ...or shrinks it into 2.75.
        let lines = compose(&glyphs, &|_| 2.75, &|_| 0.0, &|_| 0.0, &cands, &flex, &justified());
        assert_eq!(lines, Some(vec![(4, false), (7, false)]));
        // Rigid glyphs can't fill a wider line except the last, so there is no fit.
        let rigid: Vec<_> = glyphs.iter().map(|_| Rigid).collect();
        assert_eq!(compose(&glyphs, &|_| 4.0, &|_| 0.0, &|_| 0.0, &cands, &rigid, &justified()), None);
        // Justify All lets an unstretchable last line through.
        let all = Params { justify_last: true, ..justified() };
        let word = units("ab");
        assert_eq!(compose(&word, &|_| 4.0, &|_| 0.0, &|_| 0.0, &[], &[Rigid, Rigid], &all), Some(vec![(2, false)]));
    }

    #[trial]
    fn hyphen_limit_and_soft_hyphens() {
        // "ab-cd-ef" with hyphenation points after each pair: every line wants a hyphen.
        let glyphs = units("abcdef");
        let cands = [brk(2, 1.0), brk(4, 1.0)];
        let ragged = Params { justify_last: false, ragged: true, tolerance: 100.0, uniform_from: 0, hyphen_limit: 0 };
        let free = compose(&glyphs, &|_| 3.0, &|_| 0.0, &|_| 0.0, &cands, &[] as &[Rigid], &ragged).unwrap();
        assert_eq!(free, vec![(2, true), (4, true), (6, false)]);
        // A limit of one hyphenated line in a row forbids the second automatic hyphen.
        let limited = Params { hyphen_limit: 1, ..ragged };
        assert_eq!(compose(&glyphs, &|_| 3.0, &|_| 0.0, &|_| 0.0, &cands, &[] as &[Rigid], &limited), None);
        // A typed soft hyphen stays a break regardless of the limit.
        let soft = units("ab\u{ad}cd\u{ad}ef");
        let cands = [brk(3, 1.0), brk(6, 1.0)];
        let out = compose(&soft, &|_| 4.0, &|_| 0.0, &|_| 0.0, &cands, &[] as &[Rigid], &limited).unwrap();
        assert_eq!(out, vec![(3, true), (6, true), (8, false)]);
    }

    #[trial]
    fn credits_are_clamped_to_the_glyph_and_ignored_when_not_finite() {
        let glyphs = units("aa bb");
        let cands = [brk(3, 0.0)];
        let ragged = Params { justify_last: false, ragged: true, tolerance: 100.0, uniform_from: 0, hyphen_limit: 0 };
        for credit in [f64::NAN, 5.0, -1.0] {
            let out = compose(&glyphs, &|_| 2.0, &|_| 0.0, &|_| credit, &cands, &[] as &[Rigid], &ragged);
            assert_eq!(out, Some(vec![(3, false), (5, false)]), "{credit}");
        }
        // A start credit gives a wrapped line room back.
        let out = compose(&units("aa bbb"), &|_| 2.0, &|_| 1.0, &|_| 0.0, &cands, &[] as &[Rigid], &ragged);
        assert_eq!(out, Some(vec![(3, false), (6, false)]));
    }

    /// Two lines of width 2, then the frame is full; a line may widen when the frame says so.
    struct Frame {
        widen: bool,
        calls: usize,
    }
    impl Measure for Frame {
        type State = usize;
        fn first(&mut self) -> usize {
            0
        }
        fn line(&mut self, &k: &usize, _: usize, _: usize) -> Result<Option<(f64, usize)>, Exhausted> {
            self.calls += 1;
            if self.calls > 1_000 {
                return Err(Exhausted);
            }
            Ok((k < 2).then_some((2.0, k + 1)))
        }
        fn may_widen(&mut self, _: &usize, _: usize, _: usize) -> bool {
            self.widen
        }
    }

    #[trial]
    fn measure_can_end_reach_widen_or_exhaust() {
        let ragged = Params { justify_last: false, ragged: true, tolerance: 100.0, uniform_from: 0, hyphen_limit: 0 };
        let two = units("aa bb");
        let cands = [brk(3, 0.0)];
        let mut frame = Frame { widen: false, calls: 0 };
        assert_eq!(compose_with(&two, &mut frame, &|_| 0.0, &|_| 0.0, &cands, &[] as &[Rigid], &ragged), Some(vec![(3, false), (5, false)]));
        // A third line can't be set: no fit, whether or not the frame may widen.
        let three = units("aa bb cc");
        let cands = [brk(3, 0.0), brk(6, 0.0)];
        for widen in [false, true] {
            let mut frame = Frame { widen, calls: 0 };
            assert_eq!(compose_with(&three, &mut frame, &|_| 0.0, &|_| 0.0, &cands, &[] as &[Rigid], &ragged), None);
        }
        // Too long for the line, but the frame may widen: the node stays active.
        let long = units("aaa b");
        let mut frame = Frame { widen: true, calls: 0 };
        assert_eq!(compose_with(&long, &mut frame, &|_| 0.0, &|_| 0.0, &[brk(4, 0.0)], &[] as &[Rigid], &ragged), None);
        // The measure may stop the search.
        let mut frame = Frame { widen: false, calls: 1_000 };
        assert_eq!(compose_with(&two, &mut frame, &|_| 0.0, &|_| 0.0, &[brk(3, 0.0)], &[] as &[Rigid], &ragged), None);
    }
}
