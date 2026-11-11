//! [`GradientPalette`]: the grid or list of gradient presets, in groups, that picks one. A pattern over the presets
//! family's [`PresetBrowser`]: the browser owns the grid, the list, the roving focus, the keys and the accessible tree;
//! the palette supplies the rows and paints each preset's thumbnail as a gradient ramp.

use craft_ui::contract::{Bind, NoValue, Outcome, Pattern};
use craft_ui::patterns::presets::{PresetBrowser, PresetEvent, PresetRow, PresetState};
use craft_ui::runtime::env::Label;
use egui::{Response, Ui};

#[cfg(feature = "screen-testing")]
mod stories;

use super::ramp::GradientRamp;
use super::stops::{Sampler, default_sampler};
use super::value::{GradientStops, StopRows};

/// A named group of gradient presets.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GradientGroup {
    /// The group's heading.
    pub name: String,
    /// The presets in the group.
    pub items: Vec<GradientItem>,
}

impl GradientGroup {
    /// A group called `name` holding `items`.
    pub fn new(name: impl Into<String>, items: Vec<GradientItem>) -> Self {
        Self { name: name.into(), items }
    }
}

/// A named gradient preset.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GradientItem {
    /// The preset's name.
    pub name: String,
    /// The preset's stops.
    pub stops: GradientStops,
}

impl GradientItem {
    /// A preset called `name` with `stops`.
    pub fn new(name: impl Into<String>, stops: GradientStops) -> Self {
        Self { name: name.into(), stops }
    }
}

/// The preset a pick names: a recent gradient by its place in the list the host passed, or an
/// item of a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GradientRef {
    /// Place in the Recent list.
    Recent(usize),
    /// An item of a group.
    #[non_exhaustive]
    Item {
        /// Index of the group.
        group: usize,
        /// Index of the item within the group.
        item: usize,
    },
}

impl GradientRef {
    /// Item `item` of group `group`.
    pub fn item(group: usize, item: usize) -> Self {
        GradientRef::Item { group, item }
    }
}

/// A palette of gradient presets in groups, with an optional Recent group first:
///
/// ```
/// use craft_theme::Srgba;
/// use craft_draw::ui::{ColorStop, GradientGroup, GradientItem, GradientPalette, GradientStops};
/// # egui::__run_test_ui(|ui| {
/// let stops = GradientStops::new(vec![ColorStop::new(0.0, Srgba::rgb(0.0, 0.0, 0.0)), ColorStop::new(1.0, Srgba::rgb(1.0, 1.0, 1.0))]);
/// let groups = [GradientGroup::new("Basics", vec![GradientItem::new("Black, White", stops)])];
/// let picked = GradientPalette::new("presets", "Gradient presets", &groups).show(ui);
/// if let Some(pick) = picked.picked {
///     // copy the preset's stops into the host's gradient
///     let _ = pick;
/// }
/// # });
/// ```
///
/// - A click, Space or Enter on a cell or row is one [`Set`](craft_ui::edit::EditEvent::Set) of its
///   [`GradientRef`], also when it is the selected one. The palette keeps no pick state; [`selected`](Self::selected) only draws a
///   ring (Grid) or the chosen fill (List).
/// - The name is the tooltip and the accessible name. Recent entries are named "{label} N" and
///   have no tooltip.
/// - One Tab stop: the selected cell, else the first. In Grid the arrows move the focus without
///   picking: Left and Right along a row and on to the next or previous row (mirrored right to
///   left; the gradients are not), Up and Down to the nearest cell in the row above or below
///   across group labels, Home and End to the first and last. In List, Up, Down, Home and End
///   move and Left and Right do nothing. A focus move scrolls the cell into view.
/// - The area scrolls past [`max_height`](Self::max_height).
/// - AccessKit: Grid is a `Group` named `label` with `Button` cells; List is a `ListBox` of
///   `ListBoxOption`s with `selected` on the chosen one. Cells carry their place and the set's
///   size. A screen reader's Focus action on a cell moves the palette's stop to it.
///
/// What the palette reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the palette's pick"]
pub struct GradientPaletteOutput {
    /// The browser's response.
    pub response: Response,
    /// The preset a click, Space or Enter picked this pass, also when it is the selected one.
    pub picked: Option<GradientRef>,
    /// What the browser reported beyond the pick: open groups and the like.
    pub events: Vec<PresetEvent<GradientRef>>,
}

impl Outcome for GradientPaletteOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The gradient presets, in groups and an optional Recent group first, shown as a grid or a list of ramps.
///
/// - The view (grid or list) comes from the [`PresetState`] the host binds.
/// - A click, Space or Enter reports the preset in [`GradientPaletteOutput::picked`], also on the selected one.
///   `selected` draws the browser's selection.
/// - Rename, delete and reorder stay off.
#[must_use = "show the palette with `show`"]
#[derive(Pattern, craft_ui::contract::Props)]
pub struct GradientPalette<'a> {
    label: Label,
    groups: &'a [GradientGroup],
    state: Bind<'a, PresetState<GradientRef>>,
    /// The recently used gradients, shown as the first group; each is named "{recent_label} N".
    #[prop]
    recent: &'a [GradientStops],
    /// The Recent group's name.
    #[prop(into)]
    recent_label: String,
    /// The row layout the ramps are sampled with.
    #[prop]
    rows: StopRows,
    /// Samples a ramp; the default sampler of the rows by default.
    #[prop]
    sampler: Option<&'a Sampler>,
    /// The selected preset.
    #[prop]
    selected: Option<GradientRef>,
}

impl<'a> GradientPalette<'a> {
    /// A palette named `label` of `groups`, keeping its view in `state`.
    pub fn new(label: impl Into<Label>, groups: &'a [GradientGroup], state: Bind<'a, PresetState<GradientRef>>) -> Self {
        GradientPalette {
            label: label.into(),
            groups,
            state,
            recent: &[],
            recent_label: "Recent".to_owned(),
            rows: StopRows::Split,
            sampler: None,
            selected: None,
        }
    }

    /// The rows of the groups: the header of each group, then its presets. A header's key names no preset
    /// (`item == usize::MAX`).
    fn preset_rows(&self) -> Vec<PresetRow<GradientRef>> {
        let mut rows = Vec::new();
        if !self.recent.is_empty() {
            rows.push(PresetRow::group(GradientRef::Recent(usize::MAX), self.recent_label.clone(), 0));
            for i in 0..self.recent.len() {
                rows.push(PresetRow::item(GradientRef::Recent(i), format!("{} {}", self.recent_label, i + 1), 1));
            }
        }
        for (group, g) in self.groups.iter().enumerate() {
            rows.push(PresetRow::group(GradientRef::Item { group, item: usize::MAX }, g.name.clone(), 0));
            for (item, preset) in g.items.iter().enumerate() {
                rows.push(PresetRow::item(GradientRef::Item { group, item }, preset.name.clone(), 1));
            }
        }
        rows
    }

    /// Shows the palette across the available width.
    pub fn show(self, ui: &mut Ui) -> GradientPaletteOutput {
        let rows = self.preset_rows();
        let default = default_sampler(self.rows);
        let sampler: &Sampler = self.sampler.unwrap_or(&default);
        let (groups, recent) = (self.groups, self.recent);
        let thumb =
            move |cx: &craft_ui::primitives::ContentPainterCx, rect: egui::Rect, info: craft_ui::patterns::presets::ThumbInfo<'_, GradientRef>| {
                if let Some(stops) = stops_of(groups, recent, *info.key) {
                    GradientRamp::paint(cx, rect, &GradientRamp::colors(stops, sampler));
                }
            };
        let mut browser = PresetBrowser::new(self.label, self.state, &rows).thumb(thumb);
        if let Some(selected) = self.selected {
            browser = browser.selected(selected);
        }
        let out = browser.show(ui);
        let mut picked = None;
        for event in &out.events {
            if let PresetEvent::Select { key, .. } | PresetEvent::Activate { key, .. } = event
                && stops_of(groups, recent, *key).is_some()
            {
                picked = Some(*key);
            }
        }
        GradientPaletteOutput { response: out.response, picked, events: out.events }
    }
}

/// The stops of the preset `key` names; none for a group header.
pub(super) fn stops_of<'a>(groups: &'a [GradientGroup], recent: &'a [GradientStops], key: GradientRef) -> Option<&'a GradientStops> {
    match key {
        GradientRef::Recent(i) => recent.get(i),
        GradientRef::Item { group, item } => groups.get(group)?.items.get(item).map(|preset| &preset.stops),
    }
}
