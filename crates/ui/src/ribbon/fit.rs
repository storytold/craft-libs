//! A ribbon card's three bands, and how its groups take the command band's width.
//!
//! An Office-style ribbon is three horizontal bands: the tab strip, the command band under it and
//! the group-label band under the commands. ECMA-376 Part 1 §14.1 (`customUI`) states the shape
//! declaratively — a tab holds groups and a group holds its controls, with the layout left to the
//! application. This module fixes that layout once for every Craft app: the label band's height,
//! the room a dialog launcher takes beside a caption, and the order in which groups give up their
//! natural rows before the tab has to scroll.
//!
//! The model is data, not widgets. A tab is its groups in drawing order; each [`Group`] carries the
//! widths the caller measured for it at each [`Level`]. The caller measures because it owns the
//! labels, icons, fonts and commands; nothing here draws or measures, and nothing here knows a
//! command catalogue, a theme or an app policy. A group whose widths are not known yet takes no
//! room, so the plan settles once the first frame has measured it.
//!
//! The two collapse orders the Office applications use are both here. [`Collapse::FirstMisfit`]
//! keeps each group in its natural rows until one does not fit, then folds that group and every one
//! after it (Word and Excel). [`Collapse::FromEnd`] folds from the last group backwards while the
//! row is too wide (PowerPoint). In both, groups fold before the row overflows: [`Plan::overflow`]
//! is set only when even the folded row is too wide, which is the caller's cue to scroll.
//!
//! Dimensions are logical points. Non-finite, negative or oversized measurements count as
//! unmeasured, so hostile input cannot panic, and every width returned is finite.

use egui::{Rect, pos2, vec2};

/// The Office group-label band's height, in points.
pub const LABEL_BAND: f32 = 18.0;

/// The square a dialog launcher is drawn in, inside the room reserved for it.
pub const LAUNCHER_BOX: f32 = 14.0;

/// The most groups one plan covers; a longer tab is planned to this many and reported as
/// overflowing.
pub const MAX_GROUPS: usize = 512;

/// Largest meaningful extent for one band or group, in logical points.
const MAX_EXTENT: f32 = 1_000_000.0;

/// A finite, non-negative extent, or zero.
fn extent(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, MAX_EXTENT) } else { 0.0 }
}

/// A measured width: a finite, positive extent, or `None` when the caller has not measured it.
fn measured(width: f32) -> Option<f32> {
    (width.is_finite() && width > 0.0 && width <= MAX_EXTENT).then_some(width)
}

/// How a group draws: its natural rows, its measured stacked form, or folded to one row.
///
/// The order is widest first, so `Natural < Stacked < Compact`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// The group as designed.
    #[default]
    Natural,
    /// The group's narrower multi-row form, where the caller measured one.
    Stacked,
    /// One row holding the group's name, whose drop-down opens the whole group.
    Compact,
}

impl Level {
    /// `0` for [`Level::Natural`], `1` for [`Level::Stacked`], `2` for [`Level::Compact`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Every level, widest first.
    pub const ALL: [Self; 3] = [Self::Natural, Self::Stacked, Self::Compact];
}

/// Where a tab's groups give up their natural rows first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Collapse {
    /// The first group that does not fit and every group after it fold to one row; the groups
    /// before it keep their rows (Word, Excel).
    #[default]
    FirstMisfit,
    /// The last groups fold first, while the row is too wide (PowerPoint).
    FromEnd,
}

/// One group of a tab, with the widths the caller measured at each [`Level`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Group {
    widths: [Option<f32>; 3],
}

impl Group {
    /// A group whose widths are not known yet: it takes no room until it is measured.
    pub const fn unmeasured() -> Self {
        Self { widths: [None; 3] }
    }

    /// A group measured only in its natural form.
    pub const fn natural(width: f32) -> Self {
        Self { widths: [Some(width), None, None] }
    }

    /// A group measured at every level it can draw.
    pub const fn measured(natural: f32, stacked: Option<f32>, compact: f32) -> Self {
        Self { widths: [Some(natural), stacked, Some(compact)] }
    }

    /// The width measured at `level`, or `None` when that level was not measured.
    pub fn width(&self, level: Level) -> Option<f32> {
        self.widths.get(level.index()).copied().flatten().and_then(measured)
    }

    /// The widest level this group was measured at; [`Level::Natural`] while it has no measurement.
    pub fn widest(&self) -> Level {
        Level::ALL.into_iter().find(|level| self.width(*level).is_some()).unwrap_or_default()
    }

    /// The width the group takes at `level`: its width there, or its widest measured width while
    /// that level is unmeasured.
    pub fn width_at(&self, level: Level) -> f32 {
        self.width(level).or_else(|| self.width(self.widest())).unwrap_or(0.0)
    }
}

/// A tab's groups at the levels that fit `avail` points, plus what the row costs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// One level per planned group, in drawing order.
    pub levels: Vec<Level>,
    /// Width the chosen levels take together.
    pub used: f32,
    /// The row is wider than the budget even with every group folded: the caller must scroll.
    pub overflow: bool,
}

impl Plan {
    /// The level planned for the group at `index`; [`Level::Natural`] outside the plan.
    pub fn level(&self, index: usize) -> Level {
        self.levels.get(index).copied().unwrap_or_default()
    }
}

/// Plan a tab's `groups` into `avail` points, keeping `edge` points clear at the row's end.
///
/// `edge` is the room the caller keeps past its last group, so the last group drawn in its rows
/// does not sit flush against the card's edge. The returned level list has one entry per planned
/// group: at most [`MAX_GROUPS`], and a longer tab is reported as overflowing.
pub fn plan(groups: &[Group], avail: f32, edge: f32, collapse: Collapse) -> Plan {
    let budget = f64::from(extent(avail) - extent(edge).min(extent(avail)));
    let preferred: Vec<Level> = groups.iter().map(Group::widest).collect();
    match collapse {
        Collapse::FirstMisfit => plan_first_misfit(groups, budget),
        Collapse::FromEnd => plan_from_end(groups, budget, &preferred),
    }
}

/// Plan a tab whose caller chose each group's form at this width, folding from the last group
/// backwards while the row is too wide.
///
/// `preferred` is what the caller measured for this width (a group's stacked form, for example);
/// an index past its end, or a level the group was not measured at, keeps the group's widest
/// measured form. A group whose compact form is not narrower than its current form is left alone:
/// folding it could never make the row fit, and the groups to its left still can.
pub fn plan_preferred(groups: &[Group], avail: f32, edge: f32, preferred: &[Level]) -> Plan {
    let budget = f64::from(extent(avail) - extent(edge).min(extent(avail)));
    let wanted: Vec<Level> = groups
        .iter()
        .enumerate()
        .map(|(i, group)| preferred.get(i).copied().filter(|level| group.width(*level).is_some()).unwrap_or_else(|| group.widest()))
        .collect();
    plan_from_end(groups, budget, &wanted)
}

/// The planned slice and whether the caller offered more groups than one plan covers.
fn bounded(groups: &[Group]) -> (&[Group], bool) {
    let Some((planned, rest)) = groups.split_at_checked(groups.len().min(MAX_GROUPS)) else {
        return (groups, false);
    };
    (planned, !rest.is_empty())
}

/// Width taken by `groups` drawn at `levels`, for the levels that were planned.
fn used_width(groups: &[Group], levels: &[Level]) -> f64 {
    groups.iter().zip(levels).map(|(group, level)| f64::from(group.width_at(*level))).sum()
}

fn plan_first_misfit(groups: &[Group], budget: f64) -> Plan {
    let (groups, longer) = bounded(groups);
    let mut used = 0.0f64;
    let mut break_at = groups.len();
    for (i, group) in groups.iter().enumerate() {
        let level = group.widest();
        let width = f64::from(group.width_at(level));
        if used + width > budget {
            break_at = i;
            break;
        }
        used += width;
    }
    let levels: Vec<Level> = groups.iter().enumerate().map(|(i, group)| if i < break_at { group.widest() } else { Level::Compact }).collect();
    let used = used_width(groups, &levels);
    Plan { levels, used: used as f32, overflow: longer || used > budget }
}

fn plan_from_end(groups: &[Group], budget: f64, preferred: &[Level]) -> Plan {
    let (groups, longer) = bounded(groups);
    let mut levels: Vec<Level> = groups
        .iter()
        .enumerate()
        .map(|(i, group)| preferred.get(i).copied().filter(|level| group.width(*level).is_some()).unwrap_or_else(|| group.widest()))
        .collect();
    let mut used = used_width(groups, &levels);
    for i in (0..groups.len()).rev() {
        if used <= budget {
            break;
        }
        let Some(group) = groups.get(i) else { continue };
        let Some(level) = levels.get_mut(i) else { continue };
        if *level == Level::Compact {
            continue;
        }
        let current = f64::from(group.width_at(*level));
        let Some(compact) = group.width(Level::Compact) else { continue };
        let compact = f64::from(compact);
        if compact >= current {
            continue;
        }
        used += compact - current;
        *level = Level::Compact;
    }
    Plan { levels, used: used as f32, overflow: longer || used > budget }
}

/// The three bands of a ribbon card, in points.
///
/// The tab strip is drawn by the application above the card; the card itself is the command band
/// over the group-label band. [`caption_slots`] lays a caption and its dialog launcher out inside
/// the label band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bands {
    /// The tab strip above the card.
    pub tabs: f32,
    /// The command band: one to three rows of controls.
    pub commands: f32,
    /// The group-label band under the commands.
    pub labels: f32,
}

impl Bands {
    /// An Office card: the command band over the standard label band.
    pub const fn office(tabs: f32, commands: f32) -> Self {
        Self { tabs, commands, labels: LABEL_BAND }
    }

    /// The card's own height, without the tab strip above it.
    pub fn card(&self) -> f32 {
        extent(self.commands) + extent(self.labels)
    }

    /// The card plus the tab strip above it.
    pub fn total(&self) -> f32 {
        extent(self.tabs) + self.card()
    }

    /// The command band inside `card`, top-aligned and no taller than the card.
    pub fn command_band(&self, card: Rect) -> Rect {
        Rect::from_min_max(card.min, pos2(card.max.x, (card.top() + extent(self.commands)).min(card.bottom())))
    }

    /// The group-label band inside `card`, bottom-aligned.
    pub fn label_band(&self, card: Rect) -> Rect {
        Rect::from_min_max(pos2(card.left(), (card.bottom() - extent(self.labels)).max(card.top())), card.max)
    }
}

/// Split a label band into the caption's text area and, with `launcher` points of room, the
/// dialog launcher's square at the band's trailing edge.
///
/// The text area never reaches into the launcher's room, so a group no wider than its caption
/// cannot draw the two over each other. `launcher` of zero (or less, or not finite) leaves the
/// whole band to the caption.
pub fn caption_slots(band: Rect, launcher: f32) -> (Rect, Option<Rect>) {
    let room = extent(launcher).min(band.width());
    if room <= 0.0 {
        return (band, None);
    }
    let text = Rect::from_min_max(band.min, pos2(band.right() - room, band.max.y));
    let side = room.min(LAUNCHER_BOX);
    let square = Rect::from_center_size(pos2(band.right() - room * 0.5, band.center().y), vec2(side, side));
    (text, Some(square))
}

/// The width a group takes: its measured content or, when the caption and its launcher are wider,
/// the caption, with `padding` either side.
///
/// Negative, non-finite or oversized arguments count as zero, so the result is always a finite,
/// non-negative width.
pub fn group_width(content: f32, caption: f32, launcher: f32, padding: f32) -> f32 {
    let content = extent(content);
    let caption = extent(caption) + extent(launcher);
    let padding = extent(padding);
    extent(content.max(caption) + padding * 2.0)
}

/// The width of a folded group's one-row button: its name, whatever stands before it (an icon,
/// 0 when it has none) and its drop-down chevron, with `padding`, and never under `min`.
pub fn compact_width(label: f32, lead: f32, chevron: f32, padding: f32, min: f32) -> f32 {
    extent(extent(label) + extent(lead) + extent(chevron) + extent(padding)).max(extent(min))
}
