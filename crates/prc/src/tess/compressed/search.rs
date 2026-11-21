//! The planar-face check, and the search for the one mesh no retried walk fits.

use std::collections::HashMap;

use super::normals::{NormalArrays, NormalReader};
use super::walk::{Mark, Walk};
use super::{Arrays, MAX_BEND, Rebuilt, V, add, cross, dot, rebuilt, scale, sub, unit};

/// The searches [`searched_reading`] runs in turn: whether turn candidates are offered, and
/// how many inverted choices are combined.
pub(super) const STAGES: [(bool, usize); 4] = [(false, 2), (true, 2), (true, 4), (true, 6)];
/// How many unsignalled folds and turns before a failure are tried.
const PRIMARIES: usize = 40;
/// How many signalled folds among them are tried: fewer, so a run of weak signalled triangles
/// cannot fill the window and hide the one unsignalled sliver whose fold is wrong.
const SECONDARIES: usize = 20;
/// Triangle steps one search may spend before giving up undecided.
const STEP_BUDGET: usize = 2_000_000;
/// Triangles between the snapshots a resumed walk rewinds to.
const SNAP_EVERY: usize = 64;
/// Budget elements a snapshot takes beside its stack.
const SNAP_ELEMENTS: usize = 24;

/// Whether every planar face among `tris[from..]` lies within [`MAX_BEND`] tolerances of the
/// plane through its centroid, oriented by the sum of its triangles' normals. True for a mesh
/// without stored normals, which names no planar faces.
pub(super) fn flat(a: &Arrays<'_>, pos: &[V], tris: &[[u32; 3]], from: usize) -> bool {
    let Some(n) = a.normals.as_ref() else {
        return true;
    };
    let mut faces: HashMap<u32, Vec<[u32; 3]>> = HashMap::new();
    for (i, t) in tris.iter().enumerate().skip(from) {
        let Some(&f) = n.face_of.get(i) else { break };
        if n.planar.get(f as usize).copied().unwrap_or(false) {
            faces.entry(f).or_default().push(*t);
        }
    }
    faces.values().all(|ts| face_is_flat(pos, ts, a.tolerance))
}

fn face_is_flat(pos: &[V], ts: &[[u32; 3]], tolerance: f64) -> bool {
    let at = |v: u32| pos.get(v as usize).copied().unwrap_or_default();
    let (mut normal, mut centre) = ([0.0; 3], [0.0; 3]);
    for t in ts {
        let (p, q, r) = (at(t[0]), at(t[1]), at(t[2]));
        let w = cross(sub(q, p), sub(r, p));
        normal = add(normal, if dot(w, normal) < 0.0 { scale(w, -1.0) } else { w });
        centre = add(centre, add(p, add(q, r)));
    }
    if dot(normal, normal) == 0.0 {
        return true;
    }
    #[allow(clippy::cast_precision_loss)] // A face's triangle count.
    let centre = scale(centre, 1.0 / ts.len().saturating_mul(3) as f64);
    let normal = unit(normal);
    ts.iter().flatten().all(|&v| dot(sub(at(v), centre), normal).abs() <= MAX_BEND * tolerance)
}

/// One inverted decision at a triangle: its fold, or the half-turn of its degenerate apex
/// frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Fold(usize),
    Turn(usize),
}

impl Choice {
    fn at(self) -> usize {
        match self {
            Choice::Fold(i) | Choice::Turn(i) => i,
        }
    }
}

/// What one walk with a fixed set of inverted choices came to.
enum Outcome {
    Fit(Rebuilt),
    /// Failed at a triangle; the candidates before it, in its component and after the last
    /// inversion, nearest last.
    Failed(Vec<Choice>),
    /// Failed in a way no further inversion can repair.
    Dead,
}

/// [`searched_reading`] on `a`, then on `turned`, the same arrays read with
/// [`Arrays::ortho_turned`] flipped. The second search runs only when the first searched every
/// combination, found no fit, and met an apex the flip moves: otherwise it would repeat the
/// first step for step, or a refused or exhausted first reading would be answered by a guess.
pub(super) fn unique_fit_either(a: &Arrays<'_>, turned: &Arrays<'_>) -> Option<Rebuilt> {
    then_turned(searched_reading(a), || searched_reading(turned).0)
}

/// The mesh the first reading's search `first` and, after a true no-fit, `turned` settle on.
fn then_turned(first: (Search, bool), turned: impl FnOnce() -> Search) -> Option<Rebuilt> {
    match first {
        (Search::Unique(m), _) => Some(*m),
        (Search::NoFit, true) => turned().unique(),
        (Search::NoFit, false) | (Search::Refused | Search::Exhausted, _) => None,
    }
}

/// [`searched_reading`]'s fit, if unique, and whether any of its walks read
/// [`Arrays::ortho_turned`].
#[cfg(test)]
pub(super) fn unique_fit_reading(a: &Arrays<'_>) -> (Option<Rebuilt>, bool) {
    let (found, reads) = searched_reading(a);
    (found.unique(), reads)
}

/// Searches combinations of inverted choices for one that rebuilds the mesh, and says whether
/// any of its walks read [`Arrays::ortho_turned`]; a refusal is kept apart from no fit. A
/// fold candidate is a triangle no stored decision orients, or a sliver at most a tolerance
/// tall whose decoded decision is noise; a turn candidate is an apex whose frame is
/// degenerate, whose turn rests on a rounding residue. A fit consumes every array and stored
/// normal exactly and keeps every planar face flat. It is unique only if it is the single
/// such geometry among the combinations searched within [`STEP_BUDGET`] and the stream's
/// search steps: two, or a search cut short after a fit, would make it a guess, and are
/// refused. Combinations grow only from failed walks, so a fit's own extensions are not
/// tried, and a geometry reached only through them is not ruled out. A search cut short
/// before any fit is exhausted, not a no-fit, and ends the reading. Each of [`STAGES`] runs
/// only when the one before searched all its combinations and found no fit.
fn searched_reading(a: &Arrays<'_>) -> (Search, bool) {
    let Some(n) = a.normals.as_ref() else {
        return (Search::NoFit, false);
    };
    let mut reads_turned = false;
    for (turns, depth) in STAGES {
        let (found, reads) = search_reading(a, n, Plan { turns, depth, budget: STEP_BUDGET });
        reads_turned |= reads;
        match found {
            Search::NoFit if a.steps.get() > 0 => {}
            found => return (found, reads_turned),
        }
    }
    (Search::NoFit, reads_turned)
}

/// Whether a search of at most `depth` choices, turn candidates offered or not, rebuilds `a`.
#[cfg(test)]
pub(super) fn fits_within(a: &Arrays<'_>, turns: bool, depth: usize) -> bool {
    a.normals.as_ref().is_some_and(|n| matches!(search_reading(a, n, Plan { turns, depth, budget: STEP_BUDGET }).0, Search::Unique(_)))
}

/// How one search runs.
#[derive(Clone, Copy)]
struct Plan {
    /// Whether turn candidates are offered.
    turns: bool,
    /// How many inverted choices are combined.
    depth: usize,
    budget: usize,
}

/// What one search came to.
enum Search {
    /// The one geometry among the combinations searched.
    Unique(Box<Rebuilt>),
    /// Two geometries fit, or the steps ran out after a fit.
    Refused,
    /// The steps ran out before any fit, with combinations left to try.
    Exhausted,
    /// Every combination was tried and none fit.
    NoFit,
}

impl Search {
    fn unique(self) -> Option<Rebuilt> {
        match self {
            Search::Unique(m) => Some(*m),
            Search::Refused | Search::Exhausted | Search::NoFit => None,
        }
    }
}

/// One depth-first search as `plan` says, and whether any of its walks read
/// [`Arrays::ortho_turned`]. Each attempt resumes the last one's walk where their choices
/// first differ, so the budget counts triangle steps actually taken.
fn search_reading(a: &Arrays<'_>, n: &NormalArrays<'_>, plan: Plan) -> (Search, bool) {
    let mut walker = Resume::new(a, n, SNAP_EVERY);
    let mut stack = vec![Vec::new()];
    let mut found: Option<Rebuilt> = None;
    while walker.spent < plan.budget && a.steps.get() > 0 {
        let Some(flips) = stack.pop() else { break };
        match walker.attempt(a, &flips) {
            Outcome::Fit(m) => match &found {
                Some(f) if f.triangles != m.triangles || f.positions != m.positions => {
                    return (Search::Refused, walker.w.reads_turned);
                }
                _ => found = Some(m),
            },
            Outcome::Failed(c) if flips.len() < plan.depth => {
                for j in c.into_iter().filter(|j| plan.turns || matches!(j, Choice::Fold(_))) {
                    let mut g = flips.clone();
                    g.push(j);
                    stack.push(g);
                }
            }
            Outcome::Failed(_) | Outcome::Dead => {}
        }
    }
    let complete = stack.is_empty() && a.steps.get() > 0;
    let search = match found {
        Some(m) if complete => Search::Unique(Box::new(m)),
        Some(_) => Search::Refused,
        None if complete => Search::NoFit,
        None => Search::Exhausted,
    };
    (search, walker.w.reads_turned)
}

/// What one completed step of a walk offers the search.
#[derive(Clone, Copy)]
struct Step {
    /// Whether the step began a component.
    start: bool,
    /// Whether its fold is a candidate.
    fold: bool,
    /// Whether its stored normal signalled the fold, so the candidate is a secondary one.
    signalled: bool,
    /// Whether its turn is a candidate.
    turn: bool,
}

impl Step {
    /// The step `w` just took.
    fn of(w: &Walk<'_>, start: bool) -> Self {
        Step { start, fold: !w.signalled || w.weak, signalled: w.signalled, turn: w.degenerate }
    }
}

/// The candidates a search tries after a failure, from `back`: the steps before it, nearest
/// first, within its component and after the last inversion. The last [`PRIMARIES`]
/// unsignalled folds and turns, and at most [`SECONDARIES`] signalled folds among them,
/// nearest last.
fn window<'s>(back: impl Iterator<Item = (usize, &'s Step)>) -> Vec<Choice> {
    let (mut primaries, mut secondaries) = (0usize, 0usize);
    let mut c = Vec::new();
    for (i, s) in back {
        if s.turn {
            c.push(Choice::Turn(i));
            primaries = primaries.saturating_add(1);
        }
        if s.fold && !s.signalled {
            c.push(Choice::Fold(i));
            primaries = primaries.saturating_add(1);
        }
        if s.fold && s.signalled && secondaries < SECONDARIES {
            c.push(Choice::Fold(i));
            secondaries = secondaries.saturating_add(1);
        }
        if primaries >= PRIMARIES || s.start {
            break;
        }
    }
    c.reverse();
    c
}

/// The walk state before a triangle, beyond what a [`Mark`] holds, and the budget elements
/// it takes.
struct Snap {
    mark: Mark,
    stack: Vec<(u32, u32, u32)>,
    next: Option<(u32, u32, u32)>,
    sliver: Option<usize>,
    held: usize,
}

/// One walk kept across a search's attempts. Its state before triangle `i` depends only on
/// the choices at triangles before `i`, so an attempt rewinds to the last snapshot at or
/// before the first choice it does not share with the previous attempt and walks on from
/// there. Snapshots stop once they and the walk would hold more than [`Arrays::limit`].
struct Resume<'a> {
    w: Walk<'a>,
    flips: Vec<Choice>,
    steps: Vec<Step>,
    snaps: Vec<Snap>,
    snapped: usize,
    /// Triangles between snapshots.
    every: usize,
    /// Triangle steps taken so far.
    spent: usize,
}

impl<'a> Resume<'a> {
    fn new(a: &Arrays<'_>, n: &'a NormalArrays<'a>, every: usize) -> Self {
        let w = Walk::new(a.triangles, a.limit, Some(NormalReader::new(n)));
        let snap = Snap { mark: w.mark(0), stack: Vec::new(), next: None, sliver: None, held: SNAP_ELEMENTS };
        Resume { w, flips: Vec::new(), steps: Vec::new(), snaps: vec![snap], snapped: SNAP_ELEMENTS, every: every.max(1), spent: 0 }
    }

    /// Walks the whole mesh oriented, inverting each of `flips`, with no retries; the same
    /// outcome as a walk from the first triangle.
    fn attempt(&mut self, a: &Arrays<'_>, flips: &[Choice]) -> Outcome {
        let differ = self.flips.iter().filter(|c| !flips.contains(c)).chain(flips.iter().filter(|c| !self.flips.contains(c))).map(|c| c.at()).min();
        self.rewind(differ.unwrap_or(usize::MAX).min(self.steps.len()));
        self.flips = flips.to_vec();
        let after = flips.iter().map(|c| c.at().saturating_add(1)).max().unwrap_or(0);
        for i in self.steps.len()..a.triangles {
            if i % self.every == 0 {
                self.snapshot(a, i);
            }
            let start = self.w.next.is_none();
            let status = a.edge_status.get(i).copied().unwrap_or(0);
            self.w.turn = flips.contains(&Choice::Turn(i));
            self.spent = self.spent.saturating_add(1);
            if self.w.step(a, status, flips.contains(&Choice::Fold(i))).is_err() {
                return Outcome::Failed(if start { Vec::new() } else { self.candidates(i, after) });
            }
            self.steps.push(Step::of(&self.w, start));
        }
        self.finish(a)
    }

    /// The fit a completed walk makes, if its arrays and normals are used up and its planar
    /// faces are flat.
    fn finish(&mut self, a: &Arrays<'_>) -> Outcome {
        let w = &self.w;
        if !w.used_up(a) {
            return Outcome::Dead;
        }
        self.spent = self.spent.saturating_add(a.triangles);
        match w.normals.as_ref().and_then(|r| r.finished(&w.pos)) {
            Some(stored) if flat(a, &w.pos, &w.tris, 0) => Outcome::Fit(rebuilt(w.pos.clone(), w.tris.clone(), stored)),
            _ => Outcome::Dead,
        }
    }

    /// Snapshots the walk before triangle `i`, unless one is there or memory is short.
    fn snapshot(&mut self, a: &Arrays<'_>, i: usize) {
        if !self.snaps.last().is_some_and(|s| s.mark.tri < i) {
            return;
        }
        let held = self.w.stack.len().saturating_mul(2).saturating_add(SNAP_ELEMENTS);
        let snapped = self.snapped.saturating_add(held);
        if self.w.held().saturating_add(snapped) > a.limit {
            return;
        }
        self.snapped = snapped;
        self.snaps.push(Snap { mark: self.w.mark(i), stack: self.w.stack.clone(), next: self.w.next, sliver: self.w.sliver, held });
    }

    /// Restores the walk to the last snapshot at or before triangle `at`.
    fn rewind(&mut self, at: usize) {
        while self.snaps.len() > 1 && self.snaps.last().is_some_and(|s| s.mark.tri > at) {
            if let Some(s) = self.snaps.pop() {
                self.snapped = self.snapped.saturating_sub(s.held);
            }
        }
        let Some(s) = self.snaps.last() else { return };
        self.w.rewind(s.mark);
        self.w.stack.clone_from(&s.stack);
        (self.w.next, self.w.sliver) = (s.next, s.sliver);
        self.steps.truncate(s.mark.tri);
    }

    /// The [`window`] of candidates before a failure at `failed`, in its component and from
    /// `after` on.
    fn candidates(&self, failed: usize, after: usize) -> Vec<Choice> {
        window((after..failed).rev().map_while(|i| self.steps.get(i).map(|s| (i, s))))
    }
}

#[cfg(test)]
mod tests;
