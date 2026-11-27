//! The fit rules: which tabs, bar items or titles stay in a strip and which go to an overflow menu. [`fit`] and
//! [`fit_whole`] assign tab widths (tabs shrink before they overflow, or keep their width); [`fit_items`] shortens
//! items that have a short form, in the order their host ranks, and drops command items and titles from the end.
//! All of them read no egui types.

/// Tabs visible on a strip and tabs that need an overflow menu, all in original order.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TabFit {
    /// `(input index, width)` for visible tabs. A nonempty input always retains the selected tab
    /// (or the final tab if `selected` was out of bounds), even when its width must become zero.
    pub shown: Vec<(usize, f32)>,
    /// The indices of the hidden tabs, in input order.
    pub overflow: Vec<usize>,
    /// Reserved width of the overflow button, zero when no tabs overflow. This can be smaller
    /// than the requested width in a tiny strip; use it when laying out the button.
    pub overflow_width: f32,
}

/// Largest meaningful extent for one tab, in logical points.
const MAX_EXTENT: f32 = 1_000_000.0;

fn tab_extent(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, MAX_EXTENT) } else { 0.0 }
}

/// Fit tab widths into `available` points, shrinking the widest tabs first to a common cap.
/// Each tab normally retains at least `min(natural_width, minimum_width)`. If that cannot fit,
/// retain the selected tab first, then as many others as fit, and reserve an overflow button.
/// A selected tab in a very narrow strip takes the space left after that button.
///
/// Negative and non-finite dimensions become zero; finite dimensions are capped at one million
/// points. Empty input and an out-of-range selection are valid. The widths of all shown tabs plus
/// `overflow_width` never exceed the sanitized available width. Work and storage are linear in
/// the input count (with at most 32 fitting passes); callers keep their own tab-count policy.
///
/// With `keep_selected` the selected tab keeps its natural width (only a tab wider than the strip minus the
/// button shrinks); the other tabs share the rest down to `minimum_width`, and the ones that still do not fit go to
/// the overflow, the last ones first.
pub fn fit(natural: &[f32], selected: usize, available: f32, minimum_width: f32, overflow_width: f32, keep_selected: bool) -> TabFit {
    let available = tab_extent(available);
    let minimum_width = tab_extent(minimum_width);
    let natural: Vec<f32> = natural.iter().map(|&w| tab_extent(w)).collect();
    let all: Vec<usize> = (0..natural.len()).collect();
    if natural.iter().map(|&w| f64::from(w)).sum::<f64>() <= f64::from(available) {
        return TabFit { shown: all.into_iter().zip(natural).collect(), ..Default::default() };
    }
    // One tab has nowhere to overflow; all available space belongs to it.
    if natural.len() == 1 {
        return TabFit { shown: vec![(0, available)], ..Default::default() };
    }
    let floor = |w: f32| w.min(minimum_width);
    if keep_selected {
        return fit_keeping_selected(&natural, selected, available, minimum_width, overflow_width);
    }
    if natural.iter().map(|&w| f64::from(floor(w))).sum::<f64>() <= f64::from(available) {
        return TabFit { shown: squeeze(&natural, &all, available, minimum_width), ..Default::default() };
    }
    let overflow_width = tab_extent(overflow_width).min(available);
    let budget = available - overflow_width;
    let selected = selected.min(natural.len().saturating_sub(1));
    let mut keep = vec![false; natural.len()];
    let mut used = 0.0f64;
    for i in std::iter::once(selected).chain(all.iter().copied().filter(|&i| i != selected)) {
        let Some(&width) = natural.get(i) else { continue };
        let width = f64::from(floor(width));
        if i == selected || used + width <= f64::from(budget) {
            if let Some(keep) = keep.get_mut(i) {
                *keep = true;
            }
            used += width;
        }
    }
    let (shown, overflow): (Vec<usize>, Vec<usize>) = all.into_iter().partition(|&i| keep.get(i).copied().unwrap_or(false));
    let shown = squeeze(&natural, &shown, budget, minimum_width);
    TabFit { shown, overflow, overflow_width }
}

/// [`fit`] with the selected tab at its natural width: the others squeeze beside it, then overflow from the end.
fn fit_keeping_selected(natural: &[f32], selected: usize, available: f32, minimum_width: f32, overflow_width: f32) -> TabFit {
    let selected = selected.min(natural.len().saturating_sub(1));
    let floor = |i: usize| natural.get(i).copied().unwrap_or(0.0).min(minimum_width);
    let others: Vec<usize> = (0..natural.len()).filter(|&i| i != selected).collect();
    let pinned = natural.get(selected).copied().unwrap_or(0.0);
    let floors: f64 = others.iter().map(|&i| f64::from(floor(i))).sum();
    let (kept, room, overflow_width) = if floors + f64::from(pinned) <= f64::from(available) {
        (others, available - pinned, 0.0)
    } else {
        let overflow_width = tab_extent(overflow_width).min(available);
        let room = (available - overflow_width - pinned).max(0.0);
        let mut used = 0.0f64;
        let kept: Vec<usize> = others
            .into_iter()
            .take_while(|&i| {
                used += f64::from(floor(i));
                used <= f64::from(room)
            })
            .collect();
        (kept, room, overflow_width)
    };
    let squeezed = squeeze(natural, &kept, room, minimum_width);
    let pinned = pinned.min(available - overflow_width);
    let mut shown = Vec::with_capacity(squeezed.len() + 1);
    let mut overflow = Vec::new();
    for i in 0..natural.len() {
        if i == selected {
            shown.push((i, pinned));
        } else if let Some(&(_, width)) = squeezed.iter().find(|&&(at, _)| at == i) {
            shown.push((i, width));
        } else {
            overflow.push(i);
        }
    }
    TabFit { shown, overflow, overflow_width }
}

/// Fit tabs at their natural widths; the tabs that do not fit go to an overflow menu.
///
/// Tabs are shown in input order up to the first one that does not fit beside the selected tab
/// and an overflow button of `overflow_width`. The selected tab (or the final tab if `selected`
/// is out of bounds) is always shown, in its logical place: when it lies past that run, it takes
/// the place of the last tabs that fit. Only a selected tab wider than the strip minus the button
/// gets less than its natural width; its painter elides the label. A lone tab never reserves a
/// button.
///
/// The tabs marked in `kept` take the room after the selected tab before the others, in order while each fits; a
/// kept tab that does not fit overflows. The others then follow the rule above. A `kept` slice that is empty or
/// shorter than `natural` marks no tab past its end, so an empty slice gives the plain rule.
///
/// Dimensions are sanitized as in [`fit`], and the shown widths plus `overflow_width` never
/// exceed the sanitized available width. Work and storage are linear in the input count.
pub fn fit_whole(natural: &[f32], selected: usize, kept: &[bool], available: f32, overflow_width: f32) -> TabFit {
    let available = tab_extent(available);
    let natural: Vec<f32> = natural.iter().map(|&w| tab_extent(w)).collect();
    if natural.iter().map(|&w| f64::from(w)).sum::<f64>() <= f64::from(available) {
        return TabFit { shown: natural.into_iter().enumerate().collect(), ..Default::default() };
    }
    if let [width] = natural[..] {
        return TabFit { shown: vec![(0, width.min(available))], ..Default::default() };
    }
    let overflow_width = tab_extent(overflow_width).min(available);
    let budget = available - overflow_width;
    let selected = selected.min(natural.len().saturating_sub(1));
    let active = natural.get(selected).copied().unwrap_or(0.0).min(budget);
    let mut room = f64::from(budget - active);
    let mut keep = vec![false; natural.len()];
    if let Some(keep) = keep.get_mut(selected) {
        *keep = true;
    }
    // The kept tabs take room first, then the others; each run stops at its first tab that does not fit.
    for kept_run in [true, false] {
        for (i, &width) in natural.iter().enumerate() {
            if i == selected || kept.get(i).copied().unwrap_or(false) != kept_run {
                continue;
            }
            if f64::from(width) > room {
                break;
            }
            room -= f64::from(width);
            if let Some(keep) = keep.get_mut(i) {
                *keep = true;
            }
        }
    }
    let mut shown = Vec::new();
    let mut overflow = Vec::new();
    for (i, &width) in natural.iter().enumerate() {
        if i == selected {
            shown.push((i, active));
        } else if keep.get(i).copied().unwrap_or(false) {
            shown.push((i, width));
        } else {
            overflow.push(i);
        }
    }
    TabFit { shown, overflow, overflow_width }
}

fn squeeze(natural: &[f32], indices: &[usize], total: f32, minimum: f32) -> Vec<(usize, f32)> {
    let width = |i: usize| natural.get(i).copied().unwrap_or(0.0);
    let sum_at = |cap: f32| indices.iter().map(|&i| f64::from(width(i).min(cap.max(width(i).min(minimum))))).sum::<f64>();
    let (mut lo, mut hi) = (minimum, indices.iter().map(|&i| width(i)).fold(minimum, f32::max));
    if sum_at(hi) > f64::from(total) {
        for _ in 0..32 {
            let mid = lo + (hi - lo) * 0.5;
            if sum_at(mid) <= f64::from(total) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
    } else {
        lo = hi;
    }
    let mut remaining = total;
    indices
        .iter()
        .map(|&i| {
            let natural = width(i);
            // Also handles a selected tab whose minimum alone exceeds the budget, and prevents
            // accumulated floating-point rounding from placing the overflow button past the end.
            let width = natural.min(lo.max(natural.min(minimum))).min(remaining);
            remaining = (remaining - width).max(0.0);
            (i, width)
        })
        .collect()
}

/// Whether an item may move into the overflow menu, or lives there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FitKeep {
    /// The item leaves the strip when the room runs out, last item first.
    #[default]
    Free,
    /// The item stays in the strip however narrow it gets.
    Pinned,
    /// The item is only in the menu, whatever the room: the strip always shows its "More" part.
    Menu,
}

/// One item as the fit rule sees it: its extent along the strip, its section and whether it may overflow.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct FitItem {
    /// The item's extent along the strip, in points.
    pub extent: f32,
    /// The item's section.
    pub section: u16,
    /// Whether the item may overflow.
    pub keep: FitKeep,
    /// The extent of the item's shorter form, if it has one.
    pub short: Option<f32>,
    /// The rank of the item's short form: lower ranks shorten first, and equal ranks shorten the last item first.
    pub short_order: u16,
    /// Whether the item keeps its full form until no more free items can leave.
    pub short_late: bool,
}

impl FitItem {
    /// An item `extent` long in `section`.
    pub const fn new(extent: f32, section: u16, keep: FitKeep) -> Self {
        FitItem { extent, section, keep, short: None, short_order: 0, short_late: false }
    }

    /// The item with a shorter form `extent` long, which it takes before any free item leaves unless it is late.
    /// An item marked [`FitItem::short_late`] takes that form only after no free item is left to leave.
    pub const fn short(mut self, extent: f32) -> Self {
        self.short = Some(extent);
        self
    }

    /// The item with its short form ranked `order`: lower ranks shorten first, and the last item of a rank first.
    /// Every item ranks 0 by default.
    pub const fn short_order(mut self, order: u16) -> Self {
        self.short_order = order;
        self
    }

    /// The item takes its short form only after every free item that can leave has left.
    /// The other items go to the overflow menu before this one shortens. A free item with a late short form leaves
    /// in its turn instead.
    pub const fn short_late(mut self) -> Self {
        self.short_late = true;
        self
    }
}

/// Which items the strip shows and which go to the overflow menu, as indexes into the input, in input order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fit {
    /// The items the strip shows.
    pub shown: Vec<usize>,
    /// The items in the overflow menu; empty when everything fits.
    pub overflow: Vec<usize>,
    /// The shown items that take their shorter form, as indexes into the input.
    pub short: Vec<usize>,
}

fn item_extent(value: f32) -> f64 {
    if value.is_finite() { f64::from(value.max(0.0)) } else { 0.0 }
}

/// The length of the items marked in `shown`, with `gap` between neighbours of a section and `section_gap` between
/// neighbours of different sections.
pub fn run_extent(items: &[FitItem], shown: &[bool], gap: f64, section_gap: f64) -> f64 {
    let mut total = 0.0;
    let mut previous: Option<u16> = None;
    for (item, _) in items.iter().zip(shown).filter(|(_, shown)| **shown) {
        if let Some(section) = previous {
            total += if section == item.section { gap } else { section_gap };
        }
        total += item_extent(item.extent);
        previous = Some(item.section);
    }
    total
}

/// Decides which items stay in a strip `available` long. Everything fits and no item is `Menu`: all items show.
/// Otherwise the `Menu` items are in the menu, and the items with a [`FitItem::short`] form take it first, lowest
/// [`FitItem::short_order`] first and the last item of a rank first, until the run fits (listed in [`Fit::short`]); a
/// `Pinned` item shortens as a `Free` one does. Only then does the last `Free` item leave, one at a time, until the
/// rest and an overflow control `overflow_extent` long, set off by `gap`, fit. When no `Free` item is left to go, the
/// items marked [`FitItem::short_late`] take their short forms in the same order; items that left do not come back.
/// `Pinned` items stay even when they alone are longer than `available`. Items that leave are never listed as short.
/// Negative and non-finite lengths count as zero.
pub fn fit_items(items: &[FitItem], available: f32, gap: f32, section_gap: f32, overflow_extent: f32) -> Fit {
    let (available, gap, section_gap, overflow_extent) =
        (item_extent(available), item_extent(gap), item_extent(section_gap), item_extent(overflow_extent));
    let mut shown: Vec<bool> = items.iter().map(|item| item.keep != FitKeep::Menu).collect();
    if shown.iter().all(|shown| *shown) && run_extent(items, &shown, gap, section_gap) <= available {
        return Fit { shown: (0..items.len()).collect(), overflow: Vec::new(), short: Vec::new() };
    }
    // The overflow control takes room only while some item is in the menu; a run that every item is in, shortened,
    // needs no control.
    let fits = |run: &[FitItem], shown: &[bool]| {
        let extent = run_extent(run, shown, gap, section_gap);
        if shown.iter().all(|shown| *shown) {
            return extent <= available;
        }
        let before_control = if shown.iter().any(|shown| *shown) { gap } else { 0.0 };
        extent + before_control + overflow_extent <= available
    };
    // The shadow run holds each item at the extent it takes: shortened items at their `short` extent.
    let mut run: Vec<FitItem> = items.to_vec();
    let mut short = vec![false; items.len()];
    // Takes the short forms of the shown items with this `late` mark, lowest rank first and the last item of a rank
    // first (`min_by_key` keeps the first minimum, which is the last index in a reversed scan), until the run fits.
    let shorten = |late: bool, run: &mut [FitItem], short: &mut [bool], shown: &[bool]| {
        while !fits(run, shown) {
            let next = (0..items.len())
                .rev()
                .filter(|&at| shown[at] && !short[at] && items[at].short.is_some() && items[at].short_late == late)
                .min_by_key(|&at| items[at].short_order);
            let Some(next) = next else { break };
            short[next] = true;
            if let (Some(item), Some(extent)) = (run.get_mut(next), items[next].short) {
                item.extent = extent;
            }
        }
    };
    shorten(false, &mut run, &mut short, &shown);
    while !fits(&run, &shown) {
        let Some(last) = (0..items.len()).rev().find(|&at| shown[at] && items[at].keep == FitKeep::Free) else { break };
        shown[last] = false;
    }
    shorten(true, &mut run, &mut short, &shown);
    let (kept, left): (Vec<usize>, Vec<usize>) = (0..items.len()).partition(|&at| shown[at]);
    let short = kept.iter().copied().filter(|&at| short[at]).collect();
    Fit { shown: kept, overflow: left, short }
}
