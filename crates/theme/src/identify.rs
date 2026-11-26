//! Contrast helpers for state indicators. Selected and on states must be identifiable (WCAG 1.4.11)
//! in high contrast: a fill, an edge or a mark reaches the non-text minimum on what the state sits
//! on. Where a fill carries the state alone, its derived colour moves in OKLCH lightness to the
//! nearest step that does, with its text still readable on it. Standard contrast keeps the theme's
//! own fills.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::color::{Oklch, Rgba};
use crate::contrast::{contrast_ratio, ensure_contrast};
use crate::derive::over;
use crate::values;

/// Resolution of the lightness scan, then bisected to the exact step.
const SCAN: f64 = 0.01;
const BISECT_STEPS: usize = 24;

/// The inputs of a high-contrast [`Identify::fill`] search, which is a pure function of them.
/// Resolving the same colours again (another size, text size or settling round) finds the answers
/// here instead of repeating the lightness scan.
#[derive(PartialEq, Eq, Hash)]
struct MemoKey {
    on: Box<[Rgba]>,
    apart: Box<[Rgba]>,
    edge: Option<Rgba>,
    with: Box<[Rgba]>,
    fill: Rgba,
    text: Rgba,
}

/// Entries kept per thread before the memo starts over.
const MEMO_LIMIT: usize = 4096;

thread_local! {
    static MEMO: RefCell<HashMap<MemoKey, (Rgba, Rgba)>> = RefCell::default();
}

/// What a selected fill has to show against.
pub struct Identify<'a> {
    /// The fills it sits on; it reaches the non-text minimum on each.
    pub on: &'a [Rgba],
    /// Fills it touches that it also keeps a `values::LIST_HOVER_MIN` step from (a selected row
    /// beside its hovered state).
    pub apart: &'a [Rgba],
    /// An edge that identifies the state as well: when it reaches the minimum on every fill in
    /// `on`, the fill stays.
    pub edge: Option<Rgba>,
    /// Other fills the same text has to read on.
    pub with: &'a [Rgba],
    /// High contrast: the high minimums apply.
    pub high: bool,
}

impl<'a> Identify<'a> {
    /// A fill that has to show on the fills `on`.
    pub fn on(on: &'a [Rgba], high: bool) -> Self {
        Identify { on, apart: &[], edge: None, with: &[], high }
    }

    /// Also keeps a step from the fills `apart`.
    pub fn apart(self, apart: &'a [Rgba]) -> Self {
        Identify { apart, ..self }
    }

    /// Also shows by `edge`.
    pub fn edge(self, edge: Rgba) -> Self {
        Identify { edge: Some(edge), ..self }
    }

    /// Also has its text read on the fills `with`.
    pub fn with(self, with: &'a [Rgba]) -> Self {
        Identify { with, ..self }
    }

    fn pick(&self, (standard, high): (f64, f64)) -> f64 {
        if self.high { high } else { standard }
    }

    fn shows(&self, c: Rgba) -> bool {
        let min = self.pick(values::NON_TEXT_MIN);
        self.on.iter().all(|&bg| contrast_ratio(over(c, bg), bg) >= min)
    }

    /// The fill the neighbours are compared on: the first surface it sits on.
    fn base(&self) -> Rgba {
        self.on.first().copied().unwrap_or(Rgba::TRANSPARENT)
    }

    /// Whether `c` keeps its step from the fills beside it.
    fn separate(&self, c: Rgba) -> bool {
        let (base, min) = (self.base(), self.pick(values::LIST_HOVER_MIN));
        self.apart.iter().all(|&other| contrast_ratio(over(c, base), over(other, base)) >= min)
    }

    /// How close `c` is to passing: its lowest ratio over the minimum it has to reach, 1 or more
    /// when it passes.
    fn margin(&self, c: Rgba) -> f64 {
        let (min, apart, base) = (self.pick(values::NON_TEXT_MIN), self.pick(values::LIST_HOVER_MIN), self.base());
        let on = self.on.iter().map(|&bg| contrast_ratio(over(c, bg), bg) / min);
        let beside = self.apart.iter().map(|&other| contrast_ratio(over(c, base), over(other, base)) / apart);
        on.chain(beside).fold(f64::INFINITY, f64::min)
    }

    /// `text` moved until it reads on `fill` and on the fills in `with`.
    fn readable(&self, text: Rgba, fill: Rgba, min: f64) -> Rgba {
        // Moving away from one fill can move toward another only when they sit either side of the
        // text; a second round settles that case.
        (0..2).fold(text.with_alpha(255), |t, _| {
            self.with.iter().map(|&f| over(f, self.base())).chain([fill]).fold(t, |t, f| ensure_contrast(t, f, min))
        })
    }

    /// `fill` and `text` as they stay or move: both stay in standard contrast. In high contrast,
    /// `fill` when it, or its edge, already reaches the
    /// minimum on every fill it sits on and keeps its step from the fills beside it; otherwise
    /// the nearest lightness along its hue and chroma where it does and `text`, moved if it has
    /// to, reads on it. When no lightness satisfies both, the one that comes closest.
    pub fn fill(&self, fill: Rgba, text: Rgba) -> (Rgba, Rgba) {
        if !self.high {
            return (fill, text);
        }
        let key = MemoKey { on: self.on.into(), apart: self.apart.into(), edge: self.edge, with: self.with.into(), fill, text };
        if let Some(found) = MEMO.with(|m| m.borrow().get(&key).copied()) {
            return found;
        }
        let found = self.search(fill, text);
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            if m.len() >= MEMO_LIMIT {
                m.clear();
            }
            m.insert(key, found);
        });
        found
    }

    fn search(&self, fill: Rgba, text: Rgba) -> (Rgba, Rgba) {
        let text_min = self.pick(values::TEXT_MIN);
        if (self.shows(fill) || self.edge.is_some_and(|e| self.shows(e))) && self.separate(fill) {
            return (fill, text);
        }
        let start = fill.to_oklch();
        let at = |l: f64| Oklch { l, ..start }.to_rgba();
        let text_on = |c: Rgba| self.readable(text, c, text_min);
        // Passes with room to spare when at least 1.
        let score = |l: f64| {
            let c = at(l);
            let t = text_on(c);
            let reads = self.with.iter().map(|&f| contrast_ratio(t, over(f, self.base())));
            self.margin(c).min(reads.chain([contrast_ratio(t, c)]).fold(f64::INFINITY, f64::min) / text_min)
        };
        let mut best = (start.l, f64::NEG_INFINITY);
        let mut found: Option<f64> = None;
        for target in [0.0, 1.0] {
            let mut previous = start.l;
            let mut l = start.l;
            while l != target {
                l = if target > l { (l + SCAN).min(target) } else { (l - SCAN).max(target) };
                let s = score(l);
                if s > best.1 {
                    best = (l, s);
                }
                if s >= 1.0 {
                    let (mut lo, mut hi) = (previous, l);
                    for _ in 0..BISECT_STEPS {
                        let mid = 0.5 * (lo + hi);
                        if score(mid) >= 1.0 { hi = mid } else { lo = mid }
                    }
                    if found.is_none_or(|f| (hi - start.l).abs() < (f - start.l).abs()) {
                        found = Some(hi);
                    }
                    break;
                }
                previous = l;
            }
        }
        let moved = at(found.unwrap_or(best.0));
        (moved, text_on(moved))
    }
}

/// An outline of a box that is identifiable by its fill or its outline: `outline` unchanged where
/// `fill` or `outline` reaches the non-text minimum on every fill in `on`; otherwise `outline`
/// moved in lightness, keeping hue and chroma, just far enough to reach it.
pub fn identifiable_outline(outline: Rgba, fill: Rgba, on: &[Rgba], high: bool) -> Rgba {
    let min = if high { values::NON_TEXT_MIN.1 } else { values::NON_TEXT_MIN.0 };
    let shows = |c: Rgba| on.iter().all(|&bg| contrast_ratio(over(c, bg), bg) >= min);
    if shows(fill) || shows(outline) {
        return outline;
    }
    // Moving away from one surface can move toward the other only when they sit either side of
    // the outline; a second round settles that case.
    (0..2).fold(outline, |c, _| on.iter().fold(c, |c, &bg| ensure_contrast(c, bg, min)))
}

/// The hover fill of a list row: `hover`, moved along OKLCH lightness to the nearest step that is
/// at least `values::LIST_HOVER_MIN` (WCAG ratio) away from `rest` and from every fill in `beside`
/// (translucent fills over `rest`), and that `text` still reads on. When no step satisfies all of
/// them, the nearest step that keeps its distance from the others wins.
pub fn visible_hover(hover: Rgba, rest: Rgba, beside: &[Rgba], text: Rgba, high: bool) -> Rgba {
    let min = if high { values::LIST_HOVER_MIN.1 } else { values::LIST_HOVER_MIN.0 };
    let text_min = if high { values::TEXT_MIN.1 } else { values::TEXT_MIN.0 };
    let fills: Vec<Rgba> = std::iter::once(rest).chain(beside.iter().map(|&f| over(f, rest))).collect();
    let distinct = |c: Rgba| fills.iter().all(|&f| contrast_ratio(over(c, rest), f) >= min);
    if distinct(hover) {
        return hover;
    }
    let start = hover.to_oklch();
    let alpha = hover.a();
    let at = |l: f64| Oklch { l, ..start }.to_rgba().with_alpha(alpha);
    // For each fill and direction, the step nearest that fill that clears `min` against it.
    let mut candidates: Vec<Rgba> = Vec::new();
    for &fill in &fills {
        let l0 = over(fill, rest).to_oklch().l;
        for target in [0.0, 1.0] {
            let (mut lo, mut hi) = (l0, target);
            if contrast_ratio(over(at(hi), rest), fill) < min {
                continue;
            }
            for _ in 0..BISECT_STEPS {
                let mid = 0.5 * (lo + hi);
                if contrast_ratio(over(at(mid), rest), fill) >= min {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            candidates.push(at(hi));
        }
    }
    candidates.retain(|&c| distinct(c));
    let nearest =
        |pool: &mut dyn Iterator<Item = Rgba>| pool.min_by(|a, b| (a.to_oklch().l - start.l).abs().total_cmp(&(b.to_oklch().l - start.l).abs()));
    let readable = |c: &Rgba| contrast_ratio(text, over(*c, rest)) >= text_min;
    nearest(&mut candidates.iter().copied().filter(readable)).or_else(|| nearest(&mut candidates.iter().copied())).unwrap_or(hover)
}
