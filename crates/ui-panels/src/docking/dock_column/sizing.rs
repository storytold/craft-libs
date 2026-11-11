//! The widths of dock columns and the heights of the groups stacked in one.

use crate::contract::SizeLimits;
use crate::docking::{DockHeights, DockSizes, PanelSpec};
use craft_behavior::dock::Column;
use egui::{Pos2, Rect};

/// The width bounds of an expanded column: no narrower than `least` or the widest minimum of its panels, no wider
/// than the narrowest maximum.
pub fn width_bounds(least: f32, limits: impl IntoIterator<Item = SizeLimits>) -> (f32, f32) {
    let (mut lo, mut hi) = (least, f32::INFINITY);
    for limit in limits {
        lo = lo.max(limit.min.x);
        hi = hi.min(limit.max.map_or(f32::INFINITY, |max| max.x));
    }
    (lo, hi.max(lo))
}

/// Column widths that fit `room`. Each entry is `(width, floor)`: a column with a floor shrinks in proportion, never
/// below its floor; one without keeps its width. A dock saved on a larger screen still fits as far as the floors
/// allow.
pub fn fit_widths(columns: &[(f32, Option<f32>)], room: f32) -> Vec<f32> {
    let mut widths: Vec<f32> = columns.iter().map(|(width, _)| *width).collect();
    for _ in 0..columns.len() {
        let total: f32 = widths.iter().sum();
        if total <= room {
            break;
        }
        let flex: f32 =
            widths.iter().zip(columns).filter(|(width, (_, floor))| floor.is_some_and(|floor| **width > floor)).map(|(width, _)| *width).sum();
        if flex <= 0.0 {
            break;
        }
        let scale = ((room - (total - flex)) / flex).max(0.0);
        for (width, (_, floor)) in widths.iter_mut().zip(columns) {
            if let Some(floor) = *floor
                && *width > floor
            {
                *width = (*width * scale).max(floor);
            }
        }
    }
    widths
}

/// A group of a column as [`group_heights`] sizes it. Heights include the tab strip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GroupSize {
    /// Only the strip shows.
    pub minimized: bool,
    /// The height the user set; `None` or not finite when the group was never resized.
    pub saved: Option<f32>,
    /// The least height.
    pub min: f32,
    /// The heights the panels ask for, with the strip added to `fill` only when it is above 0, so 0 still means the
    /// group does not ask to fill.
    pub content: Option<DockHeights>,
}

impl GroupSize {
    /// An open group no shorter than `min` that was never resized and asks for no heights.
    pub fn new(min: f32) -> Self {
        GroupSize { minimized: false, saved: None, min, content: None }
    }
}

/// The heights of stacked groups in a column body `room` high. Minimized groups take `strip`, groups with a saved
/// height keep it, and the groups the user never resized whose content asks to fill share what is left. Without
/// those, the groups without content heights share it, and when every open group has a height the last one takes
/// the rest. Other groups take their default height, or their minimum. Default heights shrink towards their compact
/// heights until the filling groups get their fill height, nearest the filling groups first; then every height
/// shrinks in proportion towards its minimum.
pub fn group_heights(groups: &[GroupSize], strip: f32, room: f32) -> Vec<f32> {
    let saved = |group: &GroupSize| group.saved.filter(|height| height.is_finite());
    let open: Vec<usize> = (0..groups.len()).filter(|at| groups.get(*at).is_some_and(|group| !group.minimized)).collect();
    let unsaved =
        |at: &usize, test: fn(Option<DockHeights>) -> bool| groups.get(*at).is_some_and(|group| saved(group).is_none() && test(group.content));
    let mut shared: Vec<usize> = open.iter().copied().filter(|at| unsaved(at, |content| content.is_some_and(|content| content.fill > 0.0))).collect();
    if shared.is_empty() {
        shared = open.iter().copied().filter(|at| unsaved(at, |content| content.is_none())).collect();
    }
    if shared.is_empty() {
        shared.extend(open.last());
    }
    let sized: Vec<usize> = open.iter().copied().filter(|at| !shared.contains(at)).collect();
    let mut defaulted: Vec<usize> = sized.iter().copied().filter(|at| groups.get(*at).is_some_and(|group| saved(group).is_none())).collect();
    let mut heights: Vec<f32> = groups
        .iter()
        .enumerate()
        .map(|(at, group)| {
            if group.minimized {
                strip
            } else if shared.contains(&at) {
                0.0
            } else {
                saved(group).or(group.content.map(|content| content.default)).unwrap_or(group.min).max(group.min)
            }
        })
        .collect();
    let mins: Vec<f32> = groups.iter().map(|group| if group.minimized { strip } else { group.min }).collect();
    let compacts: Vec<f32> = groups.iter().zip(&mins).map(|(group, min)| group.content.map_or(*min, |content| content.compact).max(*min)).collect();
    let fill: f32 =
        shared.iter().filter_map(|at| groups.get(*at)).map(|group| group.content.map_or(group.min, |content| content.fill).max(group.min)).sum();
    defaulted.sort_by_key(|at| shared.iter().map(|shared| shared.abs_diff(*at)).min());
    let mut deficit = heights.iter().sum::<f32>() + fill - room;
    for at in defaulted {
        if deficit <= 0.0 {
            break;
        }
        if let (Some(height), Some(floor)) = (heights.get_mut(at), compacts.get(at)) {
            let give = (*height - floor).clamp(0.0, deficit);
            *height -= give;
            deficit -= give;
        }
    }
    let shared_min: f32 = shared.iter().filter_map(|at| mins.get(*at)).sum();
    give_way(&mut heights, &sized, &mins, shared_min - room);
    let free = (room - heights.iter().sum::<f32>()).max(0.0);
    // The shared groups split what is left evenly, each at least its minimum: groups whose minimum is above the even
    // share take their minimum and the rest split the remainder.
    let mut floors: Vec<f32> = shared.iter().filter_map(|at| mins.get(*at).copied()).collect();
    floors.sort_by(|a, b| b.total_cmp(a));
    let (mut rest, mut count) = (free, floors.len());
    for floor in floors {
        if count > 1 && floor > rest / count as f32 {
            rest -= floor;
            count -= 1;
        } else {
            break;
        }
    }
    let share = rest / count.max(1) as f32;
    for at in &shared {
        if let (Some(height), Some(min)) = (heights.get_mut(*at), mins.get(*at)) {
            *height = share.max(*min);
        }
    }
    heights
}

/// Shrinks the `which` heights in proportion towards their `floors` until the column's heights plus `extra` fit, or
/// every one is at its floor.
fn give_way(heights: &mut [f32], which: &[usize], floors: &[f32], extra: f32) {
    let deficit = heights.iter().sum::<f32>() + extra;
    let slack: f32 = which.iter().filter_map(|at| Some((heights.get(*at)? - floors.get(*at)?).max(0.0))).sum();
    if deficit > 0.0 && slack > 0.0 {
        let take = deficit.min(slack) / slack;
        for at in which {
            if let (Some(height), Some(floor)) = (heights.get_mut(*at), floors.get(*at)) {
                *height -= (*height - floor).max(0.0) * take;
            }
        }
    }
}

/// The rects of groups `heights` high, stacked from the top of `area`.
pub fn stacked(area: Rect, heights: &[f32]) -> Vec<Rect> {
    let mut top = area.top();
    heights
        .iter()
        .map(|height| {
            let rect = Rect::from_min_max(Pos2::new(area.left(), top), Pos2::new(area.right(), top + height));
            top += height;
            rect
        })
        .collect()
}

/// The sizes of the groups of `column` for [`group_heights`]: the strip added to the tallest minimum of a group's
/// panels, and the largest of each of their [`DockHeights`] with the strip added.
pub(super) fn group_sizes<P>(column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec, sizes: &DockSizes) -> Vec<GroupSize> {
    column
        .groups
        .iter()
        .map(|group| {
            let specs: Vec<PanelSpec> = group.panels.iter().map(spec).collect();
            let body = specs.iter().map(|spec| spec.limits.min.y).fold(sizes.least, f32::max);
            let content = specs
                .iter()
                .filter_map(|spec| spec.heights.filter(|heights| heights.valid()))
                .reduce(|a, b| DockHeights::new(a.default.max(b.default)).compact(a.compact.max(b.compact)).fill(a.fill.max(b.fill)))
                .map(|heights| {
                    DockHeights::new(heights.default + sizes.strip)
                        .compact(heights.compact.min(heights.default) + sizes.strip)
                        .fill(if heights.fill > 0.0 { heights.fill + sizes.strip } else { 0.0 })
                });
            let mut size = GroupSize::new(sizes.strip + body);
            size.minimized = group.minimized;
            size.saved = group.height;
            size.content = content;
            size
        })
        .collect()
}
