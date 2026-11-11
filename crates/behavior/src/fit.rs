//! The fit rules: which tabs, bar items or titles stay in a strip and which go to an overflow menu. [`fit`] and
//! [`fit_whole`] assign tab widths (tabs shrink before they overflow, or keep their width); [`fit_items`] drops
//! command items and titles from the end. All of them read no egui types.

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
pub fn fit(natural: &[f32], selected: usize, available: f32, minimum_width: f32, overflow_width: f32) -> TabFit {
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

/// Fit tabs at their natural widths; the tabs that do not fit go to an overflow menu.
///
/// Tabs are shown in input order up to the first one that does not fit beside the selected tab
/// and an overflow button of `overflow_width`. The selected tab (or the final tab if `selected`
/// is out of bounds) is always shown, in its logical place: when it lies past that run, it takes
/// the place of the last tabs that fit. Only a selected tab wider than the strip minus the button
/// gets less than its natural width; its painter elides the label. A lone tab never reserves a
/// button.
///
/// Dimensions are sanitized as in [`fit`], and the shown widths plus `overflow_width` never
/// exceed the sanitized available width. Work and storage are linear in the input count.
pub fn fit_whole(natural: &[f32], selected: usize, available: f32, overflow_width: f32) -> TabFit {
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
    let mut shown = Vec::new();
    let mut overflow = Vec::new();
    let mut full = false;
    for (i, &width) in natural.iter().enumerate() {
        if i == selected {
            shown.push((i, active));
        } else if !full && f64::from(width) <= room {
            room -= f64::from(width);
            shown.push((i, width));
        } else {
            full = true;
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
}

impl FitItem {
    /// An item `extent` long in `section`.
    pub const fn new(extent: f32, section: u16, keep: FitKeep) -> Self {
        FitItem { extent, section, keep }
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
/// Otherwise the `Menu` items are in the menu, and the last `Free` item leaves first, one at a time, until the rest
/// and an overflow control `overflow_extent` long, set off by `gap`, fit; `Pinned` items stay even when they alone
/// are longer than `available`. Negative and non-finite lengths count as zero.
pub fn fit_items(items: &[FitItem], available: f32, gap: f32, section_gap: f32, overflow_extent: f32) -> Fit {
    let (available, gap, section_gap, overflow_extent) =
        (item_extent(available), item_extent(gap), item_extent(section_gap), item_extent(overflow_extent));
    let mut shown: Vec<bool> = items.iter().map(|item| item.keep != FitKeep::Menu).collect();
    if shown.iter().all(|shown| *shown) && run_extent(items, &shown, gap, section_gap) <= available {
        return Fit { shown: (0..items.len()).collect(), overflow: Vec::new() };
    }
    let fits = |shown: &[bool]| {
        let before_control = if shown.iter().any(|shown| *shown) { gap } else { 0.0 };
        run_extent(items, shown, gap, section_gap) + before_control + overflow_extent <= available
    };
    while !fits(&shown) {
        let Some(last) = (0..items.len()).rev().find(|&at| shown[at] && items[at].keep == FitKeep::Free) else { break };
        shown[last] = false;
    }
    let (kept, left): (Vec<usize>, Vec<usize>) = (0..items.len()).partition(|&at| shown[at]);
    Fit { shown: kept, overflow: left }
}
