//! Knuth–Plass total fit: the breaks that minimise the paragraph's summed demerits.

use super::{Item, Kind, Line, Params, Sums, clamp_measure, fitness_class};

/// Give up (return `None`) beyond this many active breaks.
const MAX_ACTIVE: usize = 4096;
/// Ratios beyond this are rated as this (an unstretchable line is infinitely loose).
const MAX_RATIO: f64 = 10_000.0;
/// Demerits of an overfull line accepted because nothing else reaches a break.
const OVERFULL_DEMERITS: f64 = 1e8;

#[derive(Clone, Copy, Debug)]
struct Node {
    /// The break this node ends a line at.
    pos: usize,
    /// First item of the next line.
    start: usize,
    /// Number of the next line.
    line: usize,
    fitness: Option<u8>,
    demerits: f64,
    /// Consecutive lines ending at flagged breaks, up to here.
    hyphens: usize,
    flagged: bool,
    prev: Option<usize>,
}

/// The best way found to reach the current break for one (line class, fitness, hyphen state).
/// Line classes are line numbers below [`Params::uniform_from`], then one class for the rest.
struct Candidate {
    line: usize,
    fitness: u8,
    hyphens: usize,
    demerits: f64,
    from: usize,
}

/// One total-fit pass over `items`. `width(k)` is the measure of line `k`; set
/// [`Params::uniform_from`] when it varies. Returns `None` when no set of breaks keeps every line
/// within [`Params::tolerance`] (unless [`Params::overfull`]), or past the active-break limit.
pub fn knuth_plass(items: &[Item], width: &dyn Fn(usize) -> f64, p: &Params) -> Option<Vec<Line>> {
    let p = &p.sanitized();
    let width = |k: usize| clamp_measure(width(k));
    let s = Sums::new(items);
    let m = items.len();
    let limit = p.hyphen_limit;
    // Hyphen states 0..=limit; unlimited still tells a flagged line (1) from others (0).
    let max_hyphens = if limit == 0 { 1 } else { limit };
    let zone = p.rag.map_or(0.0, |r| r.hyphenation_zone);
    let mut nodes = vec![Node { pos: 0, start: 0, line: 0, fitness: p.first_fitness, demerits: 0.0, hyphens: 0, flagged: false, prev: None }];
    let mut active: Vec<usize> = vec![0];
    let mut keep: Vec<usize> = Vec::new();
    let mut best: Vec<Candidate> = Vec::new();
    // The last break at a space (glue), for the hyphenation zone.
    let mut last_space: Option<usize> = None;
    for b in 0..=m {
        if !s.is_break(b) {
            continue;
        }
        let item = items.get(b);
        let forced = s.forced(b);
        let flagged = item.is_some_and(Item::is_flagged);
        let cost = item.filter(|i| i.kind == Kind::Penalty).map_or(0.0, |i| i.cost);
        best.clear();
        keep.clear();
        let mut last_removed = None;
        for &a in &active {
            let Some(n) = nodes.get(a).copied() else { continue };
            // The node's next line starts after `b` (the penalties between were discarded): no
            // line ends here, but one may end at a later break.
            if b < n.start {
                keep.push(a);
                continue;
            }
            let measure = width(n.line);
            let f = s.measure(n.start, b, measure, p);
            // An overfull node is dropped even at a penalty with width (a hyphen), as in TeX,
            // although a later break without that width might still fit.
            if f.feasible && !forced {
                keep.push(a);
            } else {
                last_removed = Some(a);
            }
            if !f.feasible || (f.ratio > p.tolerance && !forced) {
                continue;
            }
            if flagged {
                if limit > 0 && n.hyphens >= limit {
                    continue;
                }
                // Hyphenation zone: break at the space before the word instead when that leaves
                // less than the zone at the line end.
                if zone > 0.0
                    && item.is_some_and(|i| i.hyphenation)
                    && let Some(sp) = last_space.filter(|&sp| sp >= n.start)
                    && measure - s.measure(n.start, sp, measure, p).width < zone
                {
                    continue;
                }
            }
            let (d, fitness) = line_demerits(&n, f.ratio, forced, flagged, cost, p);
            let hyphens = if flagged { (n.hyphens + 1).min(max_hyphens) } else { 0 };
            let line = (n.line + 1).min(p.uniform_from);
            let demerits = n.demerits + d;
            record(&mut best, Candidate { line, fitness, hyphens, demerits, from: a });
        }
        if p.overfull
            && keep.is_empty()
            && best.is_empty()
            && let Some(a) = last_removed
            && let Some(n) = nodes.get(a)
        {
            // An overfull line ending here would leave the next line empty up to a forced break:
            // end it at that forced break instead. Nor may it pass the hyphen limit: it ends at a
            // later break.
            let empty_next = !forced && s.items.get(s.next_start(b)).is_some_and(Item::is_forced);
            let too_many_hyphens = flagged && limit > 0 && n.hyphens >= limit;
            if empty_next || too_many_hyphens {
                keep.push(a);
            } else {
                best.push(Candidate {
                    line: (n.line + 1).min(p.uniform_from),
                    fitness: 0,
                    hyphens: 0,
                    demerits: n.demerits + OVERFULL_DEMERITS,
                    from: a,
                });
            }
        }
        std::mem::swap(&mut active, &mut keep);
        if item.is_some_and(|i| i.kind == Kind::Glue) {
            last_space = Some(b);
        } else if forced {
            last_space = None;
        }
        if best.is_empty() {
            if active.is_empty() {
                return None;
            }
            continue;
        }
        let start = s.next_start(b);
        // Knuth's pruning: within a line class and hyphen state, a node more than the fitness
        // demerits behind the best can never win.
        for c in &best {
            let min = best.iter().filter(|o| o.line == c.line && o.hyphens == c.hyphens).map(|o| o.demerits).fold(f64::INFINITY, f64::min);
            if c.demerits > min + p.fitness_demerits {
                continue;
            }
            let Some(prev) = nodes.get(c.from).copied() else { continue };
            nodes.push(Node {
                pos: b,
                start,
                line: prev.line + 1,
                fitness: Some(c.fitness),
                demerits: c.demerits,
                hyphens: if flagged { prev.hyphens + 1 } else { 0 },
                flagged,
                prev: Some(c.from),
            });
            active.push(nodes.len() - 1);
        }
        if active.len() > MAX_ACTIVE {
            return None;
        }
    }
    let end = if s.ends_forced() { m - 1 } else { m };
    let chain = best_chain(&nodes, &active, end)?;
    let mut start = 0;
    let mut lines = Vec::with_capacity(chain.len());
    for (k, &b) in chain.iter().enumerate() {
        let next = s.next_start(b);
        lines.push(s.line(start, b, next, width(k), p, 0.0));
        start = next;
    }
    Some(lines)
}

/// Keep the cheaper of the candidates sharing a (line class, fitness, hyphen state).
fn record(best: &mut Vec<Candidate>, new: Candidate) {
    match best.iter_mut().find(|c| c.line == new.line && c.fitness == new.fitness && c.hyphens == new.hyphens) {
        Some(c) if c.demerits <= new.demerits => {}
        Some(c) => {
            c.demerits = new.demerits;
            c.from = new.from;
        }
        None => best.push(new),
    }
}

/// Demerits of ending a line of adjustment `ratio` after node `n` at a break, and its fitness class.
fn line_demerits(n: &Node, ratio: f64, forced: bool, flagged: bool, cost: f64, p: &Params) -> (f64, u8) {
    let weight = if ratio > 0.0 {
        p.stretch_weight
    } else if ratio < 0.0 {
        p.shrink_weight
    } else {
        1.0
    };
    let badness = (weight * 100.0 * ratio.abs().min(MAX_RATIO).powi(3)).min(p.max_badness);
    let lp = p.line_penalty + badness;
    let mut d = if forced {
        lp * lp
    } else if cost >= 0.0 {
        lp * lp + cost * cost
    } else {
        lp * lp - cost * cost
    };
    if flagged && n.flagged {
        d += p.flagged_demerits;
    }
    // A ragged line ending at a forced break says nothing about the rag: it keeps its
    // neighbour's class.
    let class = fitness_class(ratio, &p.fitness);
    let fitness = if forced && p.rag.is_some() { n.fitness.unwrap_or(class) } else { class };
    if n.fitness.is_some_and(|nf| fitness.abs_diff(nf) > 1) {
        d += p.fitness_demerits;
    }
    (d, fitness)
}

/// Break positions of the cheapest path that ends at `end`, in order.
fn best_chain(nodes: &[Node], active: &[usize], end: usize) -> Option<Vec<usize>> {
    let last = active
        .iter()
        .copied()
        .filter(|&a| nodes.get(a).is_some_and(|n| n.pos == end && n.prev.is_some()))
        .min_by(|&a, &b| nodes[a].demerits.total_cmp(&nodes[b].demerits))?;
    let mut chain = Vec::new();
    let mut cur = Some(last);
    while let Some(i) = cur {
        let n = nodes.get(i)?;
        if n.prev.is_some() {
            chain.push(n.pos);
        }
        cur = n.prev;
    }
    chain.reverse();
    Some(chain)
}
