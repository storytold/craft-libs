//! Greedy first fit: each line takes as much as fits.

use super::{INFINITE_PENALTY, Item, Kind, Line, Params, Sums, clamp_measure};

/// Break `items` first fit: each line ends at the last break where it still fits its measure
/// `width(k)` with its shrink. A line with no such break ends at the last [`Item::emergency`]
/// break that fits, else before the first box that overflows (never after an
/// [`Item::no_break`]).
pub fn greedy(items: &[Item], width: &dyn Fn(usize) -> f64, p: &Params) -> Vec<Line> {
    greedy_with(items, width, p, &|_, _, k| items.get(k).map_or(0.0, |i| i.width))
}

/// [`greedy`] with box widths that depend on position: `advance(line, x, k)` is the width of box
/// `k` when it starts `x` from the start of line `line` (a tab reaching its stop). It returns
/// `items[k].width` for other boxes.
pub fn greedy_with(items: &[Item], width: &dyn Fn(usize) -> f64, p: &Params, advance: &dyn Fn(usize, f64, usize) -> f64) -> Vec<Line> {
    let p = &p.sanitized();
    let width = |k: usize| clamp_measure(width(k));
    let s = Sums::new(items);
    let m = items.len();
    // Japanese "push out first": break where the line fits without shrinking when there is one.
    let avoid_shrink = p.shrink_weight > p.stretch_weight;
    let zone = p.rag.map_or(0.0, |r| r.hyphenation_zone);
    let zoned = |b: usize| zone > 0.0 && items.get(b).is_some_and(|i| i.is_flagged() && i.hyphenation);
    let no_break_before = |k: usize| k > 0 && items.get(k - 1).is_some_and(|i| i.kind == Kind::Penalty && i.cost >= INFINITE_PENALTY);
    let mut out = Vec::new();
    let mut start = 0;
    let mut line = 0;
    let mut hyphens = 0;
    loop {
        let measure = width(line);
        // extra[k - start]: width the boxes before item `k` gained from `advance`.
        let mut extra = Vec::new();
        let mut ext = 0.0;
        let mut last_ok = None;
        let mut last_natural = None;
        let mut last_emergency = None;
        // (break, line width there)
        let mut last_space: Option<(usize, f64)> = None;
        let mut k = start;
        let (end, next) = loop {
            extra.push(ext);
            let Some(it) = items.get(k) else { break (m, m) };
            if s.is_break(k) {
                if it.is_forced() {
                    break (k, s.next_start(k));
                }
                let blocked = it.is_flagged() && p.hyphen_limit > 0 && hyphens >= p.hyphen_limit;
                let f = s.fit(start, k, Some(it), false, measure, p, ext);
                if !blocked && f.feasible {
                    if it.emergency {
                        last_emergency = Some(k);
                    } else {
                        last_ok = Some(k);
                        if f.width <= measure + p.overrun {
                            last_natural = Some(k);
                        }
                    }
                }
                if it.kind == Kind::Glue {
                    last_space = Some((k, f.width));
                }
            } else if it.kind == Kind::Box {
                let x = s.fit(start, k, None, false, measure, p, ext).width;
                ext += advance(line, x, k) - it.width;
                if k > start {
                    // Would a line ending right after this box overflow? (The following break's
                    // hang and edge elasticity apply; a hyphen is checked at its own break.)
                    let after = items.get(k + 1).filter(|i| i.kind != Kind::Box).map(|i| Item { width: 0.0, emergency: false, ..*i });
                    let f = s.fit(start, k + 1, after.as_ref(), false, measure, p, ext);
                    if avoid_shrink
                        && f.width > measure + p.overrun
                        && let Some(b) = last_natural
                    {
                        break (b, s.next_start(b));
                    }
                    if !f.feasible {
                        match (last_ok, last_emergency) {
                            (Some(b), _) => {
                                // Hyphenation zone: break before the word if that leaves little space.
                                if zoned(b)
                                    && let Some((sp, xs)) = last_space
                                    && measure - xs < zone
                                {
                                    break (sp, s.next_start(sp));
                                }
                                break (b, s.next_start(b));
                            }
                            (None, Some(b)) => break (b, s.next_start(b)),
                            (None, None) if !no_break_before(k) => break (k, k),
                            (None, None) => {}
                        }
                    }
                }
            }
            k += 1;
        };
        hyphens = if items.get(end).is_some_and(Item::is_flagged) { hyphens + 1 } else { 0 };
        let ext_end = extra.get(end.saturating_sub(start)).copied().unwrap_or(ext);
        out.push(s.line(start, end, next, measure, p, ext_end));
        if next >= m {
            break;
        }
        start = next;
        line += 1;
    }
    out
}
