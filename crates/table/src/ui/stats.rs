//! Selection statistics in the status bar.

use std::sync::Arc;

use craft_behavior::{Axis, CursorIcon, Flow, Metrics, PartKey, Sense};
use craft_theme::{Stateful, TypeRole};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Name, Outcome, Overlay, OverlayUi, Overlays as _, PaintUi,
    PartsCx, PopupKind, Props as _, Role, Show as _, ShowContainer as _, Styled as _, Trigger, WordSetCx as _,
};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::TextRun;
use craft_ui::runtime::commands::{ClipItem, Commands as _};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, FocusLook, StyleCx, TextLook};
use egui::{Rect, Response, Vec2};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The statistics' names, English by default (Excel's).
    #[non_exhaustive]
    pub struct StatsWords {
        average: template "stats.average" = "Average: {v}";
        count: template "stats.count" = "Count: {v}";
        numerical_count: template "stats.numericalCount" = "Numerical Count: {v}";
        minimum: template "stats.minimum" = "Minimum: {v}";
        maximum: template "stats.maximum" = "Maximum: {v}";
        sum: template "stats.sum" = "Sum: {v}";
        average_item: "stats.averageItem" = "Average";
        count_item: "stats.countItem" = "Count";
        numerical_count_item: "stats.numericalCountItem" = "Numerical Count";
        minimum_item: "stats.minimumItem" = "Minimum";
        maximum_item: "stats.maximumItem" = "Maximum";
        sum_item: "stats.sumItem" = "Sum";
        click_to_copy: "stats.clickToCopy" = "Click to copy";
        menu: "stats.menu" = "Statistics";
    }
}

/// The part of the statistic at an index.
const STAT: PartKey = PartKey::new("stat");

/// Why a statistic has no key path of its own.
const COPY_REASON: &str = "Copy copies the selection; the statistics are values of it";

/// A selection's statistics, worked out by the app over its cells.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SelectionStats {
    /// Non-empty cells.
    pub count: u64,
    /// Cells holding numbers.
    pub numeric: u64,
    /// Sum of the numbers.
    pub sum: f64,
    /// Smallest of the numbers.
    pub min: f64,
    /// Largest of the numbers.
    pub max: f64,
}

impl SelectionStats {
    /// The statistics of `count` non-empty cells, `numeric` of them numbers with the given sum, smallest and largest.
    pub fn new(count: u64, numeric: u64, sum: f64, min: f64, max: f64) -> Self {
        SelectionStats { count, numeric, sum, min, max }
    }

    /// `None` without numbers.
    pub fn average(&self) -> Option<f64> {
        (self.numeric > 0).then(|| self.sum / self.numeric as f64)
    }
}

/// Which statistics show; one settings section (Excel's status bar menu is app-wide).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct StatsShown {
    /// Show the average.
    pub average: bool,
    /// Show the count of non-empty cells.
    pub count: bool,
    /// Show the count of cells holding numbers.
    pub numerical_count: bool,
    /// Show the smallest number.
    pub minimum: bool,
    /// Show the largest number.
    pub maximum: bool,
    /// Show the sum.
    pub sum: bool,
}

impl StatsShown {
    /// Every statistic shown.
    pub fn all() -> Self {
        StatsShown { average: true, count: true, numerical_count: true, minimum: true, maximum: true, sum: true }
    }
}

impl StatPart {
    /// A statistic of `text` ("Average: 3.5") whose value alone is `value` ("3.5").
    pub fn new(text: impl Into<String>, value: impl Into<String>) -> Self {
        StatPart { text: text.into(), value: value.into() }
    }
}

impl Default for StatsShown {
    /// Excel's: Average, Count and Sum.
    fn default() -> Self {
        Self { average: true, count: true, numerical_count: false, minimum: false, maximum: false, sum: true }
    }
}

/// One statistic of the status bar: the label with its value, and the value alone, which a click copies.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatPart {
    /// "Average: 3.5".
    pub text: String,
    /// "3.5".
    pub value: String,
}

/// The status bar's statistics, left to right: Average, Count, Numerical Count, Minimum, Maximum, Sum, each when
/// shown and the selection has a value for it (Count and Numerical Count with any cell, the others with numbers).
/// `format` formats a number in the active cell's number format.
pub fn stats_parts(stats: &SelectionStats, shown: &StatsShown, words: &StatsWords, format: impl Fn(f64) -> String) -> Vec<StatPart> {
    let numbers = stats.numeric > 0;
    let any = stats.count > 0;
    let mut parts = Vec::new();
    let mut push = |word: &craft_ui::runtime::env::WordTemplate, value: String| parts.push(StatPart { text: word.fill(&[("v", &value)]), value });
    if let Some(average) = stats.average().filter(|_| shown.average) {
        push(&words.average, format(average));
    }
    if shown.count && any {
        push(&words.count, stats.count.to_string());
    }
    if shown.numerical_count && any {
        push(&words.numerical_count, stats.numeric.to_string());
    }
    if shown.minimum && numbers {
        push(&words.minimum, format(stats.min));
    }
    if shown.maximum && numbers {
        push(&words.maximum, format(stats.max));
    }
    if shown.sum && numbers {
        push(&words.sum, format(stats.sum));
    }
    parts
}

/// One statistic of the menu.
#[derive(Clone, Copy)]
enum Statistic {
    Average,
    Count,
    NumericalCount,
    Minimum,
    Maximum,
    Sum,
}

impl Statistic {
    const ALL: [Statistic; 6] =
        [Statistic::Average, Statistic::Count, Statistic::NumericalCount, Statistic::Minimum, Statistic::Maximum, Statistic::Sum];

    fn name(self, words: &StatsWords) -> Label {
        match self {
            Statistic::Average => words.average_item.clone(),
            Statistic::Count => words.count_item.clone(),
            Statistic::NumericalCount => words.numerical_count_item.clone(),
            Statistic::Minimum => words.minimum_item.clone(),
            Statistic::Maximum => words.maximum_item.clone(),
            Statistic::Sum => words.sum_item.clone(),
        }
    }

    fn flag(self, shown: &mut StatsShown) -> &mut bool {
        match self {
            Statistic::Average => &mut shown.average,
            Statistic::Count => &mut shown.count,
            Statistic::NumericalCount => &mut shown.numerical_count,
            Statistic::Minimum => &mut shown.minimum,
            Statistic::Maximum => &mut shown.maximum,
            Statistic::Sum => &mut shown.sum,
        }
    }
}

/// The style of the statistics readout.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SelectionStatsReadoutStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The statistics' text.
    #[style(text)]
    text: TextLook,
    /// The gap between the statistics.
    gap: f32,
}

/// What the readout did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct SelectionStatsOutput {
    /// The readout's response and the edits of the menu's check rows.
    pub edit: EditResponse<StatsShown>,
    /// The value a click on a statistic copied this pass, without its label.
    pub copied: Option<String>,
}

impl Outcome for SelectionStatsOutput {
    type Value = StatsShown;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The statistics in the status bar for a selection of more than one cell, in a row. A click on a statistic copies
/// its value without the label; a secondary click, the Menu key or Shift+F10 opens the menu of check items (one per
/// statistic), which changes the bound [`StatsShown`]. The readout takes the room it is given, so the menu opens
/// with no statistic shown. The readout is one Tab stop.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(overlay)]
pub struct SelectionStatsReadout<'a> {
    #[common]
    common: Common,
    parts: &'a [StatPart],
    shown: Bind<'a, StatsShown>,
    rects: Vec<Rect>,
    copied: Option<String>,
}

impl<'a> SelectionStatsReadout<'a> {
    /// A readout named `label` of `parts`, with `shown` choosing which of them appear; its menu edits `shown`.
    pub fn new(label: impl Into<Label>, parts: &'a [StatPart], shown: Bind<'a, StatsShown>) -> Self {
        SelectionStatsReadout { common: Common::new(), parts, shown, rects: Vec::new(), copied: None }.label(label)
    }

    fn run<'t>(text: &'t str, style: &SelectionStatsReadoutStyle) -> TextRun<'t> {
        TextRun::new(text).role(style.text.role)
    }
}

impl Component for SelectionStatsReadout<'_> {
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::GROUP;
    type Style = SelectionStatsReadoutStyle;
    type Output = SelectionStatsOutput;

    fn style(cx: &StyleCx<'_>, _: SelectionStatsReadoutStyleKey) -> SelectionStatsReadoutStyle {
        SelectionStatsReadoutStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            text: TextLook::new(TypeRole::Label, Stateful::new(cx.color(cx.tokens.color.text.muted))),
            gap: cx.tokens.space.gap.stack,
        }
    }

    /// The statistics side by side, at least one text line tall.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let line = cx.layout(&Self::run("0", cx.style)).size().y;
        let mut size = Vec2::new(0.0, line);
        for (at, part) in self.parts.iter().enumerate() {
            let text = cx.layout(&Self::run(&part.text, cx.style)).size();
            size.x += text.x + if at > 0 { cx.style.gap } else { 0.0 };
            size.y = size.y.max(text.y);
        }
        size
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn bind(&mut self) -> Option<BindMut<'_, StatsShown>> {
        Some(self.shown.reborrow())
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let words = cx.words::<StatsWords>();
        self.rects.clear();
        let mut flow = Flow::new(rect, cx.flow.is_rtl());
        // The statistic that reaches the end edge takes the room left and elides; the ones after it take none, so
        // they are not painted, though each keeps its part and access node.
        for (at, part) in self.parts.iter().enumerate() {
            let width = cx.layout(&Self::run(&part.text, cx.style)).size().x.min(flow.rect().width());
            let (taken, rest) = flow.take_start(width);
            flow = rest.take_start(cx.style.gap.min(rest.rect().width())).1;
            cx.part(STAT.at(at as u64), taken)
                .sense(Sense::CLICK)
                .cursor(CursorIcon::PointingHand)
                .tooltip(words.click_to_copy.clone())
                .pointer_only(COPY_REASON);
            self.rects.push(taken);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        for (at, part) in self.parts.iter().enumerate() {
            if cx.part_activated(STAT.at(at as u64)).is_some() {
                cx.clipboard().put(&[ClipItem::Text(Arc::from(part.value.as_str()))]);
                self.copied = Some(part.value.clone());
            }
        }
        cx.popup("menu").trigger(Trigger::Context, cx);
    }

    /// The menu: a check row per statistic that leaves the menu open.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("menu");
        let words = cx.words::<StatsWords>();
        let shown = cx.value().copied().unwrap_or_default();
        let picked = ui.show(popup, Overlay::menu(), |ui, _| {
            Menu::inline(words.menu.clone())
                .show(ui, |ui, _| {
                    let mut flags = shown;
                    let mut picked = None;
                    for statistic in Statistic::ALL {
                        let on = *statistic.flag(&mut flags);
                        if MenuItem::new(statistic.name(&words)).check(on).keep_open(true).show(ui).ran {
                            picked = Some(statistic);
                        }
                    }
                    picked
                })
                .inner
                .flatten()
        });
        if let Some(statistic) = picked.flatten() {
            let mut next = shown;
            let flag = statistic.flag(&mut next);
            *flag = !*flag;
            cx.set(next);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.has_popup(PopupKind::Menu);
        for (at, part) in self.parts.iter().enumerate() {
            access.node(STAT.at(at as u64), Role::LABEL).name = Name::Text(part.text.clone());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let color = cx.resolve(&cx.style.text.color);
        for (part, rect) in self.parts.iter().zip(&self.rects).filter(|(_, rect)| rect.width() > 0.0) {
            draw.text(*rect, &Self::run(&part.text, cx.style).elide().color(color));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> SelectionStatsOutput {
        SelectionStatsOutput { edit: cx.edit_response(), copied: self.copied }
    }
}
