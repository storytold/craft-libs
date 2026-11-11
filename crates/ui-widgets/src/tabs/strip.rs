//! `TabStrip`: a collection of `Tab`s side by side in one row. The item step owns the lead, the selection, the reorder
//! gesture and the drag session; the strip adds the item layout that fits the tabs to its width, the overflow button
//! with its menu of tabs, and the tear-off of a tab dragged away from every strip.
//!
//! The tabs are the caller's items (`ShowCollection::show_items` with one `Tab` per key). The source's
//! `ItemSource::text` is each tab's name in the overflow menu. The drag payload of a tab is `ListDrag<K>`; a dock that
//! carries panels reads the same payload with the panel key as `K`.

#[cfg(feature = "testing")]
mod stories;

use super::rules::{FitMode, OverflowMenu, OverflowPlace, TabKind};
use super::words::StripWords;
use super::{Tab, TabPartEvent};
use crate::buttons::IconButton;
use crate::contract::{
    Access, Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, Common, Component, ContainerProps, ContextMenuRequest,
    Cx, DropMark, Entry, ListDrag, Name, NoValue, Outcome, OverlayUi, Overlays as _, PartsCx, Pointer as _, Props as _, Role, RowState, SelectMode,
    Sizing, WindowDropTarget, WordSetCx as _,
};
use crate::list::{DropTest, RowTest, SourceTest};
use crate::menu::Menu;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{HitLevel, HitPriority, IconId, ItemLayout, ItemSource, Metrics, NavShape, PartKey, ScrollPos, Selection, Sense};
use egui::{Id, Pos2, Rect, Response, Vec2};
use std::cell::RefCell;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

const OVERFLOW: PartKey = PartKey::new("overflow");

/// A tab dragged away from every strip and released: the tabs the drag carried and where the pointer was.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TornOff<K> {
    /// The dragged tabs in display order.
    pub keys: Vec<K>,
    /// The tab the pointer grabbed.
    pub grabbed: K,
    /// The pointer when the drag was released, in screen points.
    pub pointer: Pos2,
}

/// What a `TabStrip` reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the strip's events"]
pub struct TabStripOutput<K> {
    /// The collection's response and events: a selection change (a click, or Space or an arrow on the lead), Enter or
    /// a double click as `Activate`, a reorder or a drop from another strip. Close and context-menu events are in the
    /// fields below.
    pub response: CollectionResponse<K, TabPartEvent>,
    /// The tabs whose close button was pressed, or that a middle click closed where the kind closes on it.
    pub closed: Vec<K>,
    /// The tab context menu is asked for: a secondary click, the context-menu key or Shift+F10.
    pub menu: Option<ContextMenuRequest<K>>,
    /// The tab the user picked in the overflow menu: the host shows it.
    pub picked: Option<K>,
    /// A tab drag was released outside every strip (`TabStrip::tear_off`).
    pub torn_off: Option<TornOff<K>>,
    /// The rect of each shown tab, in screen space, in the order the source lists them: where a drop between two
    /// tabs resolves its insertion index.
    pub tab_rects: Vec<(K, Rect)>,
    /// The part of the strip's row that no tab and no overflow button takes, in screen space; its width is 0 when the
    /// tabs fill the row. A host makes it a handle of its own, such as the grip a dock group is dragged by.
    pub empty: Rect,
}

impl<K> Outcome for TabStripOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.response.story_facts()
    }
}

/// The look of a strip: it has none of its own; its row is as high as its tabs.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TabStripStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What the layout tells the strip's parts and menu after fitting the tabs.
#[derive(Clone, Debug, Default, PartialEq)]
struct Report {
    /// The positions of the tabs that do not fit.
    hidden: Vec<usize>,
    /// Where the last shown tab ends, from the start edge.
    tabs_end: f32,
    /// The width of the overflow button; 0 when every tab fits.
    overflow: f32,
    /// Each shown tab's position in the source, distance from the start edge and width.
    placed: Vec<(usize, f32, f32)>,
}

/// The item layout of a tab strip: the tabs in one row, each as wide as it measures itself, fitted to the width by
/// the kind's rule. The tabs that do not fit are not shown.
#[derive(Clone, Debug)]
pub struct StripLayout {
    ideal: Vec<f32>,
    min: Vec<f32>,
    width: f32,
    height: f32,
    measured_height: bool,
    selected: usize,
    mode: FitMode,
    shown: Vec<bool>,
    left: Vec<f32>,
    wide: Vec<f32>,
    report: Arc<Mutex<Report>>,
    /// A measuring run computes the fit and leaves the report to the real pass (`measure::effect`).
    ctx: egui::Context,
}

impl StripLayout {
    fn new(count: usize, width: f32, height: f32, selected: usize, mode: FitMode, report: Arc<Mutex<Report>>, ctx: egui::Context) -> Self {
        let each = if count == 0 { 0.0 } else { width / count as f32 };
        let mut layout = StripLayout {
            ideal: vec![each; count],
            min: vec![0.0; count],
            width,
            height,
            measured_height: false,
            selected,
            mode,
            shown: Vec::new(),
            left: Vec::new(),
            wide: Vec::new(),
            report,
            ctx,
        };
        layout.refit();
        layout
    }

    /// Places the tabs the fit keeps from the start edge; the others are not listed as visible, so they are not built. The overflow button
    /// is as wide as the row is high.
    fn refit(&mut self) {
        let count = self.ideal.len();
        let minimum = self.min.iter().copied().fold(0.0, f32::max);
        let fit = self.mode.fit(&self.ideal, self.selected, self.width, minimum, self.height);
        self.shown = vec![false; count];
        self.left = vec![self.width; count];
        self.wide = vec![0.0; count];
        let mut at = 0.0;
        let mut placed = Vec::with_capacity(fit.shown.len());
        for &(index, width) in &fit.shown {
            self.shown[index] = true;
            self.left[index] = at;
            self.wide[index] = width;
            placed.push((index, at, width));
            at += width;
        }
        if !crate::runtime::measure::active(&self.ctx) {
            *self.report.lock().unwrap_or_else(PoisonError::into_inner) =
                Report { hidden: fit.overflow, tabs_end: at, overflow: fit.overflow_width, placed };
        }
    }
}

impl ItemLayout for StripLayout {
    fn content_size(&self) -> ScrollPos {
        ScrollPos { x: f64::from(self.width), y: f64::from(self.height) }
    }

    fn visible(&self, _: ScrollPos, _: Vec2, out: &mut Vec<usize>) {
        out.extend((0..self.ideal.len()).filter(|&at| self.shown[at]));
    }

    fn rect(&self, at: usize, offset: ScrollPos) -> Rect {
        Rect::from_min_size(Pos2::new(self.left[at] - offset.x as f32, 0.0), Vec2::new(self.wide[at], self.height))
    }

    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        (0..self.ideal.len()).find(|&at| self.shown[at] && self.rect(at, offset).contains(point))
    }

    fn nav(&self) -> NavShape {
        NavShape::RaggedGrid
    }

    fn row_len(&self, row: usize) -> Option<usize> {
        (row == 0).then_some(self.ideal.len())
    }

    fn shown(&self, at: usize) -> bool {
        self.shown.get(at).copied().unwrap_or(false)
    }

    fn is_auto(&self, _: usize) -> bool {
        true
    }

    fn measured(&mut self, heights: &dyn Fn(usize) -> Option<f32>) {
        let tallest =
            (0..self.ideal.len()).filter_map(heights).fold(None, |most: Option<f32>, height| Some(most.map_or(height, |held| held.max(height))));
        if let Some(height) = tallest {
            self.height = height;
            self.measured_height = true;
            self.refit();
        }
    }

    fn set_height(&mut self, _: usize, height: f32) {
        self.height = if self.measured_height { self.height.max(height) } else { height };
        self.measured_height = true;
        self.refit();
    }

    fn auto_widths(&self) -> bool {
        true
    }

    fn set_width(&mut self, at: usize, ideal: f32, min: f32) {
        if at < self.ideal.len() && ((self.ideal[at] - ideal).abs() > f32::EPSILON || (self.min[at] - min).abs() > f32::EPSILON) {
            self.ideal[at] = ideal;
            self.min[at] = min;
            self.refit();
        }
    }
}

/// Accepts a drag of this strip's tabs wherever no other target takes it: the tab is torn off.
struct TearOff {
    list: Id,
}

impl<K: Send + Sync + 'static> WindowDropTarget<ListDrag<K>> for TearOff {
    type Drop = ();

    fn accept(&self, payload: &ListDrag<K>, _: craft_behavior::DropOperation) -> Option<()> {
        (payload.list == self.list).then_some(())
    }

    fn mark(&self, _: &()) -> DropMark {
        DropMark::Into(Rect::ZERO)
    }
}

/// A strip of tabs: one Tab stop, arrows move the lead (and the selection with it), a click selects, Enter and a
/// double click activate, Alt+Left and Alt+Right (or a drag) reorder, the menu key asks for the tab's context menu.
///
/// The `kind` picks the rules: document and text tabs keep whole titles and send the rest to an overflow menu of
/// every tab; panel tabs shrink before they overflow and the menu lists the hidden ones. The shown tab is always
/// visible. A tab dragged out of the strip and released where no other strip, dock or drop target takes it is
/// reported as `torn_off`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection, overlay, probe(TabStrip<'static, usize>))]
pub struct TabStrip<'a, K: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    #[collection]
    items: CollectionProps,
    /// The kind of strip: its fit rule and overflow menu, and the kind its tabs take.
    #[prop]
    kind: TabKind,
    /// The strip's taller form with the body type.
    #[prop(flag)]
    large: bool,
    /// The tab the host shows. A click, an arrow key or a drop reports the new one in the output.
    #[prop]
    shown: Option<K>,
    /// A tab dragged out and released where no target takes it is reported as `torn_off`.
    #[prop(flag)]
    tear_off: bool,
    /// Which tabs may be dragged.
    #[prop(func = "Fn(&K) -> bool")]
    can_drag: Option<RowTest<'a, K>>,
    /// Which collections a drag may come from: the test gets the id of the collection the drag started in.
    #[prop(func = "Fn(Id) -> bool")]
    accept_from: Option<SourceTest<'a>>,
    /// Where a drag may land: the test gets the drag, the tab under the pointer and the position before which the
    /// dragged tabs would go.
    #[prop(func = "Fn(&ListDrag<K>, Option<&K>, usize) -> bool")]
    accepts: Option<DropTest<'a, K>>,
    held: Bind<'a, Selection<K>>,
    report: Arc<Mutex<Report>>,
    /// Each tab's key and name, as the source listed them this pass.
    texts: RefCell<Vec<(K, String)>>,
    more: Option<Rect>,
    picked: Option<K>,
    torn: Option<TornOff<K>>,
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> TabStrip<'a, K> {
    /// A strip named `label` of document tabs.
    pub fn new(label: impl Into<Label>) -> Self {
        TabStrip {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::FillWidth),
            items: CollectionProps::new().select(SelectMode::Single).pages(true).overflow(craft_behavior::Axis::X),
            kind: TabKind::Document,
            large: false,
            shown: None,
            tear_off: false,
            can_drag: None,
            accept_from: None,
            accepts: None,
            held: Bind::kept(),
            report: Arc::new(Mutex::new(Report::default())),
            texts: RefCell::new(Vec::new()),
            more: None,
            picked: None,
            torn: None,
        }
        .label(label)
    }

    /// A tab drag carries `map` of it, for the drop targets of type `P`; a tab the map gives `None` for does not drag
    /// unless a reorder rule is set. The drag stays one session with one ghost.
    #[must_use]
    pub fn drag_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<K>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.items = self.items.drag_payload(map);
        self
    }

    fn report(&self) -> Report {
        self.report.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The shown tabs' rects in screen space, mirrored under right-to-left.
    fn tab_rects(&self, cx: &Cx<'_, Self>) -> Vec<(K, Rect)> {
        let flow = craft_behavior::Flow::new(cx.core.content_rect, cx.flow.is_rtl());
        let band = cx.core.content_rect;
        let texts = self.texts.borrow();
        self.report()
            .placed
            .iter()
            .filter_map(|&(index, start, width)| {
                let logical = Rect::from_min_max(Pos2::new(band.left() + start, band.top()), Pos2::new(band.left() + start + width, band.bottom()));
                Some((texts.get(index)?.0.clone(), flow.to_physical(logical)))
            })
            .collect()
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> Component for TabStrip<'_, K> {
    const ROLE: Role = Role::TAB_LIST;
    type Style = TabStripStyle;
    type Output = TabStripOutput<K>;

    fn style(_: &StyleCx<'_>, _: TabStripStyleKey<K>) -> TabStripStyle {
        TabStripStyle { metrics: Metrics::new() }
    }

    /// The overflow button, over the end of the row or right after the last shown tab.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        self.more = None;
        let report = self.report();
        if report.overflow <= 0.0 {
            return;
        }
        let flow = craft_behavior::Flow::new(cx.content_rect, cx.flow.is_rtl());
        let slot = match self.kind.overflow_place() {
            OverflowPlace::StripEnd => flow.take_end(report.overflow).0,
            OverflowPlace::AfterTabs => flow.take_start(report.tabs_end).1.take_start(report.overflow).0,
        };
        self.more = Some(slot);
        cx.part(OVERFLOW, slot)
            .sense(Sense::CLICK)
            .priority(HitPriority::new(HitLevel::Edge, 0))
            .over()
            .toggles("menu")
            .on_press()
            .pointer_only("Alt+Down opens the overflow menu");
    }

    /// A drag of this strip's tabs that no target takes is a tear-off.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if !self.tear_off {
            return;
        }
        let held = cx.dragging::<ListDrag<K>>();
        let target = TearOff { list: cx.core.id };
        if cx.window_drop_target::<ListDrag<K>, _>(&target).is_some()
            && let Some((payload, pointer)) = held
        {
            self.torn = Some(TornOff { keys: payload.keys.clone(), grabbed: payload.grabbed.clone(), pointer });
        }
    }

    /// The overflow menu: every tab with the shown one checked, or only the hidden tabs, by the kind.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        if self.more.is_none() {
            return;
        }
        let popup = cx.popup("menu");
        let report = self.report();
        let texts = self.texts.borrow();
        let all = self.kind.overflow_menu() == OverflowMenu::AllTabs;
        let entries: Vec<Entry<K>> = texts
            .iter()
            .enumerate()
            .filter(|(at, _)| all || report.hidden.contains(at))
            .map(|(_, (key, text))| Entry::new(key.clone(), text.clone()).checked(all && self.shown.as_ref() == Some(key)))
            .collect();
        let name = cx.label();
        let layer = crate::menu::layer(false).anchor(ui.anchor_part(OVERFLOW));
        if let Some(pick) = ui.show(popup, layer, |ui, _| Menu::inline(name.clone()).pick(ui, &entries)) {
            self.picked = pick.picked.or(self.picked.take());
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if self.more.is_some() {
            let node = access.node(OVERFLOW, Role::BUTTON);
            node.name = Name::Text(cx.words::<StripWords>().more.as_str().to_owned());
        }
    }

    fn paint_over(&mut self, ui: &mut crate::contract::PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.more.is_some() {
            ui.face(OVERFLOW, IconButton::new(IconId::new("more")).label(cx.words::<StripWords>().more.clone()));
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> TabStripOutput<K> {
        let tab_rects = self.tab_rects(&cx);
        let report = self.report();
        let rest = craft_behavior::Flow::new(cx.core.content_rect, cx.flow.is_rtl()).take_start(report.tabs_end).1;
        let rest = match (report.overflow > 0.0, self.kind.overflow_place()) {
            (false, _) => rest,
            (true, OverflowPlace::StripEnd) => rest.take_end(report.overflow).1,
            (true, OverflowPlace::AfterTabs) => rest.take_start(report.overflow).1,
        };
        let empty = rest.rect();
        let TabStrip { picked, torn, .. } = self;
        let mut response = cx.collection_response();
        let (mut closed, mut menu) = (Vec::new(), None);
        let mut kept = Vec::with_capacity(response.events.len());
        for event in std::mem::take(&mut response.events) {
            match event {
                CollectionEvent::Item(key, TabPartEvent::Close) => closed.push(key),
                CollectionEvent::ContextMenu(request) => menu = Some(request),
                other => kept.push(other),
            }
        }
        response.events = kept;
        TabStripOutput { response, closed, menu, picked, torn_off: torn, tab_rects, empty }
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> Collection for TabStrip<'_, K> {
    type Key = K;
    type ItemLayout = StripLayout;
    type Selection = Selection<K>;
    type Item<'r> = Tab<'r>;
    type ItemEvent = TabPartEvent;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    /// The shown tab is the single selection; the strip reports a change and the host applies it.
    fn selection(&mut self) -> Option<BindMut<'_, Selection<K>>> {
        self.held = Bind::given(self.shown.clone().map_or_else(Selection::default, Selection::single));
        Some(self.held.reborrow())
    }

    fn item_layout(&self, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> StripLayout {
        let count = source.len();
        *self.texts.borrow_mut() = (0..count).map(|at| (source.key(at), source.text(at).map(|text| text.into_owned()).unwrap_or_default())).collect();
        let selected = self.shown.as_ref().and_then(|key| source.index_of(key)).unwrap_or(usize::MAX);
        StripLayout::new(count, width, cx.tokens.size.bar.tab, selected, self.kind.fit_mode(), self.report.clone(), cx.frame_ctx().clone())
    }

    /// Every tab takes the strip's kind and size.
    fn dress<'r>(&self, tab: Self::Item<'r>, _: &RowState) -> Self::Item<'r> {
        let tab = tab.kind(self.kind);
        if self.large { tab.large() } else { tab }
    }

    fn can_drag(&self, key: &K) -> bool {
        self.can_drag.as_ref().is_none_or(|test| test.passes(key))
    }

    fn accept_from(&self, from: Id) -> bool {
        self.accept_from.as_ref().is_none_or(|test| test.passes(from))
    }

    fn accepts(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        self.accepts.as_ref().is_none_or(|test| test.passes(drag, over, insert))
    }
}
