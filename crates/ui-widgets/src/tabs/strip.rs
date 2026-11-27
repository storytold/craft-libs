//! `TabStrip`: a collection of `Tab`s side by side in one row. The item step owns the lead, the selection, the reorder
//! gesture and the drag session; the strip adds the item layout that fits the tabs to its width, the overflow button
//! with its menu of tabs, and the tear-off of a tab dragged away from every strip. A strip may carry end items
//! (`StripEnd`) beside its tabs; a ribbon strip slides one underline to its shown tab and fades out a contextual tab
//! its source drops.
//!
//! The tabs are the caller's items (`ShowCollection::show_items` with one `Tab` per key). The source's
//! `ItemSource::text` is each tab's name in the overflow menu. The drag payload of a tab is `ListDrag<K>`; a dock that
//! carries panels reads the same payload with the panel key as `K`.

mod end;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use end::StripEnd;

use super::rules::{FitMode, OverflowMenu, OverflowPlace, TabKind};
use super::words::StripWords;
use super::{Tab, TabPartEvent, TabRole};
use crate::buttons::IconButton;
use crate::contract::{
    Access, Animate as _, Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, Common, Component, ContainerProps,
    ContextMenuRequest, Cx, DropKind, DropMark, Entry, ListDrag, Meta, Name, NoValue, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx,
    Pointer as _, PopupKind, Props as _, Remember as _, Role, RowState, SelectMode, ShowContainer as _, ShowShape, Sizing, WindowDropTarget,
    WordSetCx as _, spec,
};
use crate::list::{DropTest, RowTest};
use crate::menu::Menu;
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::commands::FireSource;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, FocusLook, StyleColor, StyleCx};
use craft_behavior::fit::{FitItem, fit_items};
use craft_behavior::{
    Flow, HitLevel, HitPriority, IconId, Insets, ItemLayout, ItemSource, Metrics, NavShape, PartKey, PressPhase, ScrollPos, Selection,
    SelectionChange, Sense,
};
use egui::{Id, Pos2, Rect, Response, Vec2};
use std::cell::RefCell;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

const OVERFLOW: PartKey = PartKey::new("overflow");

/// One end item: `END.at(position in the slice given to `TabStrip::trailing`)`, which is fixed for a strip's life.
const END: PartKey = PartKey::new("end");

/// The popups of the end items that open a menu, one repeat per item position.
const ENDS: &str = "ends";

/// One contextual tab fading out: `GHOST.at(position among the tabs fading out)`, a part with no sense that its face
/// is painted on.
const GHOST: PartKey = PartKey::new("ghost");

/// A tab dragged away from every strip and released: the tabs the drag carried and where the pointer was. A tool rail
/// reports a group torn off a flyout as one too, its slot as both `keys` and `grabbed`.
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

impl<K> TornOff<K> {
    /// A tear-off of `keys`, the tab `grabbed` among them, released with the pointer at `pointer`. The struct is
    /// `#[non_exhaustive]`, so this is how another crate (a tool rail) reports one.
    pub fn new(keys: Vec<K>, grabbed: K, pointer: Pos2) -> Self {
        TornOff { keys, grabbed, pointer }
    }
}

/// What a `TabStrip` reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the strip's events"]
pub struct TabStripOutput<K> {
    /// The collection's response and events: a selection change (a click, or Space or Enter on the lead), Enter or a
    /// double click as `Activate`, a reorder or a drop from another strip. Close and context-menu events are in the
    /// fields below.
    pub response: CollectionResponse<K, TabPartEvent>,
    /// The tabs whose close button was pressed, or that a middle click closed where the kind closes on it
    /// (`TabStrip::can_close`).
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
    /// A drag started this pass on the strip's empty room, the part of its row no tab and no overflow button takes.
    /// A host makes the room a handle of its own, such as the room a dock group is dragged by.
    pub empty_drag: bool,
    /// The second press of a double click went down on the strip's empty room this pass.
    pub empty_double: bool,
    /// The overflow button, when some tabs do not fit: its rect in screen space and the id of its accessibility node.
    pub overflow: Option<(Rect, Id)>,
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

/// The room before the first tab of a strip: a regular pad for a ribbon strip, whose first label stands where the old
/// ribbon put it; none for the others.
fn strip_lead(kind: TabKind, tokens: &crate::style::UiTokens) -> f32 {
    if kind == TabKind::Ribbon { tokens.space.pad.regular } else { 0.0 }
}

/// The look of a strip: the focus ring of its end items and of the tab the keyboard is on; its row is as high as its
/// tabs.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TabStripStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The text line a wheel notch given in lines counts.
    line: f32,
    /// The colour of the one underline a ribbon strip slides under its shown tab.
    underline: StyleColor,
    /// The underline's distance from the row's bottom edge: half its stroke, so the stroke lies inside the row.
    underline_inset: f32,
    /// How far in from each side of the shown tab the underline starts: the tab's padding less half the stroke, so
    /// it runs just past the label at both ends, as the old ribbon drew it.
    underline_side: f32,
    /// The underline's height where a ribbon look sets one (`RibbonLook::tabs`), a bar at the row's bottom; the thick
    /// stroke otherwise.
    underline_height: Option<f32>,
}

/// Where the fit put one end item: its position in `TabStrip::trailing`, its distance from the start edge, its width
/// and whether it shows its icon only.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct EndSlot {
    item: usize,
    start: f32,
    width: f32,
    short: bool,
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
    /// The end items, in the order given.
    ends: Vec<EndSlot>,
    /// The room at the end edge the end items take with the gap before them; 0 without end items.
    ends_room: f32,
    /// The slot (distance from the start edge and width) of each contextual tab fading out, in the order
    /// `StyleCx::exiting` lists them; `None` for one that has no room in the row.
    ghosts: Vec<Option<(f32, f32)>>,
}

/// The room a contextual tab fading out keeps in the fit until its exit ends: the slot's place among the tabs
/// fading out, the position of the source tab it stood before, and its width.
#[derive(Clone, Copy, Debug)]
struct Reserved {
    slot: usize,
    before: usize,
    width: f32,
}

/// One place in the fitted row: a tab of the source, or the reserved slot of a tab fading out (an index into
/// `Fitting::ghosts`).
#[derive(Clone, Copy)]
enum Lane {
    Tab(usize),
    Ghost(usize),
}

/// The wheel scroll a strip has taken but not yet turned into a step, and the tab the strip left shown.
#[derive(Clone)]
struct WheelCarry<K> {
    owed: f32,
    left: Option<K>,
}

impl<K> Default for WheelCarry<K> {
    fn default() -> Self {
        Self { owed: 0.0, left: None }
    }
}

/// What the fit reads besides the tabs' widths: the shown tab, the kind's rule, the tabs kept ahead of the others and
/// the end items.
#[derive(Clone, Debug)]
struct Fitting {
    selected: usize,
    mode: FitMode,
    /// The contextual tabs, by position; empty when the strip has no `roles`.
    kept: Vec<bool>,
    /// The end items as the fit sees them, in the order given.
    ends: Vec<FitItem>,
    /// The gap between end items, and between the tabs and the first end item.
    gap: f32,
    /// The slots kept for the contextual tabs fading out.
    ghosts: Vec<Reserved>,
    /// How many contextual tabs are fading out, those without a slot included.
    departed: usize,
}

/// The item layout of a tab strip: the tabs in one row, each as wide as it measures itself, fitted to the width by
/// the kind's rule. The tabs that do not fit are not shown.
#[derive(Clone, Debug)]
pub struct StripLayout {
    ideal: Vec<f32>,
    min: Vec<f32>,
    width: f32,
    height: f32,
    /// Space between document tabs; included in the fit budget but not in their hit rectangles.
    tab_gap: f32,
    measured_height: bool,
    fitting: Fitting,
    shown: Vec<bool>,
    left: Vec<f32>,
    wide: Vec<f32>,
    report: Arc<Mutex<Report>>,
    /// A measuring run computes the fit and leaves the report to the real pass (`measure::effect`).
    ctx: egui::Context,
}

impl StripLayout {
    fn new(count: usize, size: Vec2, tab_gap: f32, fitting: Fitting, report: Arc<Mutex<Report>>, ctx: egui::Context) -> Self {
        let (width, height) = (size.x, size.y);
        let each = if count == 0 { 0.0 } else { width / count as f32 };
        let mut layout = StripLayout {
            ideal: vec![each; count],
            min: vec![0.0; count],
            width,
            height,
            tab_gap,
            measured_height: false,
            fitting,
            shown: Vec::new(),
            left: Vec::new(),
            wide: Vec::new(),
            report,
            ctx,
        };
        layout.refit();
        layout
    }

    /// The end items' places and the room they take at the end edge. They get the room the tabs' natural widths
    /// leave and drop their labels from the last one first (`fit_items`, rb.R4); they are `Pinned`, so once every one
    /// is short they keep that room and the tabs overflow instead.
    fn fit_ends(&self) -> (f32, Vec<EndSlot>) {
        let Fitting { ends, gap, ghosts, .. } = &self.fitting;
        if ends.is_empty() {
            return (0.0, Vec::new());
        }
        let places = self.ideal.len() + ghosts.len();
        let natural =
            self.ideal.iter().sum::<f32>() + ghosts.iter().map(|ghost| ghost.width).sum::<f32>() + self.tab_gap * places.saturating_sub(1) as f32;
        let fit = fit_items(ends, self.width - natural - gap, *gap, *gap, 0.0);
        let sized: Vec<(usize, f32, bool)> = fit
            .shown
            .iter()
            .filter_map(|&at| {
                let item = ends.get(at)?;
                let short = fit.short.contains(&at);
                Some((at, if short { item.short.unwrap_or(item.extent) } else { item.extent }, short))
            })
            .collect();
        let run = sized.iter().map(|&(_, width, _)| width).sum::<f32>() + gap * sized.len().saturating_sub(1) as f32;
        let mut start = self.width - run;
        let slots = sized
            .into_iter()
            .map(|(item, width, short)| {
                let slot = EndSlot { item, start, width, short };
                start += width + gap;
                slot
            })
            .collect();
        (run + gap, slots)
    }

    /// Places the tabs the fit keeps from the start edge; the others are not listed as visible, so they are not built.
    /// The end items take their room at the end edge first, and the tabs fit in what is left. The overflow button is
    /// as wide as the row is high. A contextual tab fading out takes part in the fit at the place it left, as a tab
    /// of its width, so the row holds still until its exit ends.
    fn refit(&mut self) {
        let count = self.ideal.len();
        let (ends_room, ends) = self.fit_ends();
        let StripLayout { ideal, min, width, height, tab_gap, fitting, shown, left, wide, report, ctx, .. } = self;
        let tab_gap = *tab_gap;
        let Fitting { selected, mode, kept, ghosts, departed, .. } = &*fitting;
        let mut lane = Vec::with_capacity(count + ghosts.len());
        for at in 0..=count {
            lane.extend(ghosts.iter().enumerate().filter(|(_, ghost)| ghost.before == at).map(|(k, _)| Lane::Ghost(k)));
            if at < count {
                lane.push(Lane::Tab(at));
            }
        }
        // Every place in the lane takes a trailing gap, and the last gap goes back to the available width: the spaces
        // are in the shrink and overflow budget without one after the last tab.
        let padded: Vec<f32> = lane
            .iter()
            .map(|place| match *place {
                Lane::Tab(at) => ideal.get(at).copied().unwrap_or(0.0),
                Lane::Ghost(k) => ghosts.get(k).map_or(0.0, |ghost| ghost.width),
            })
            .map(|width| width + tab_gap)
            .collect();
        let held: Vec<bool> = lane
            .iter()
            .map(|place| match *place {
                Lane::Tab(at) => kept.get(at).copied().unwrap_or(false),
                Lane::Ghost(_) => true,
            })
            .collect();
        let selected = lane.iter().position(|place| matches!(*place, Lane::Tab(at) if at == *selected)).unwrap_or(usize::MAX);
        let minimum = min.iter().copied().fold(0.0, f32::max);
        let fit = mode.fit(&padded, selected, &held, (*width - ends_room).max(0.0) + tab_gap, minimum + tab_gap, *height);
        *shown = vec![false; count];
        *left = vec![*width; count];
        *wide = vec![0.0; count];
        let mut at = 0.0;
        let mut placed = Vec::with_capacity(fit.shown.len());
        let mut slots: Vec<Option<(f32, f32)>> = vec![None; *departed];
        for &(position, padded_extent) in &fit.shown {
            let extent = (padded_extent - tab_gap).max(0.0);
            match lane.get(position) {
                Some(&Lane::Tab(index)) => {
                    shown[index] = true;
                    left[index] = at;
                    wide[index] = extent;
                    placed.push((index, at, extent));
                }
                Some(&Lane::Ghost(k)) => {
                    if let Some(slot) = ghosts.get(k).and_then(|ghost| slots.get_mut(ghost.slot)) {
                        *slot = Some((at, extent));
                    }
                }
                None => {}
            }
            at += extent + tab_gap;
        }
        if !fit.shown.is_empty() {
            at -= tab_gap;
        }
        let hidden: Vec<usize> = fit
            .overflow
            .iter()
            .filter_map(|&position| match lane.get(position) {
                Some(&Lane::Tab(index)) => Some(index),
                _ => None,
            })
            .collect();
        if !crate::runtime::measure::active(ctx) {
            *report.lock().unwrap_or_else(PoisonError::into_inner) =
                Report { hidden, tabs_end: at, overflow: fit.overflow_width, placed, ends, ends_room, ghosts: slots };
        }
    }
}

impl ItemLayout for StripLayout {
    fn set_view(&mut self, viewport: Vec2, _: ScrollPos) {
        if self.width != viewport.x {
            self.width = viewport.x;
            self.refit();
        }
    }

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

    fn drop_hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        self.hit(point, offset).or_else(|| {
            (0..self.ideal.len()).find(|&at| {
                if !self.shown[at] || self.left[at] <= 0.0 {
                    return false;
                }
                let rect = self.rect(at, offset);
                point.y >= rect.top() && point.y <= rect.bottom() && point.x >= rect.left() - self.tab_gap && point.x < rect.left()
            })
        })
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

/// The keyed presence of a ribbon strip's contextual tabs, one key per tab key (rb.R2a).
const LEAVE: &str = "contextual";

/// What a contextual tab's presence keeps of it while it shows and fades out after leaving the source: its name, the
/// position it had in the source and its width; no width while it sat in the overflow menu (rb.R2, S-L72).
#[derive(Clone, Debug)]
struct Ghost {
    label: String,
    before: usize,
    width: Option<f32>,
}

/// A departed contextual tab drawn this pass: its name and opacity.
#[derive(Clone, Debug)]
struct Fading {
    label: String,
    alpha: f32,
}

/// The role of the tab of a key: `TabStrip::roles`.
pub struct TabRoles<'a, K>(Box<dyn Fn(&K) -> TabRole + 'a>);

impl<'a, K> TabRoles<'a, K> {
    /// The roles `roles` gives.
    pub fn new(roles: impl Fn(&K) -> TabRole + 'a) -> Self {
        TabRoles(Box::new(roles))
    }
}

impl<K> TabRoles<'_, K> {
    #[doc(hidden)]
    pub fn role(&self, key: &K) -> TabRole {
        (self.0)(key)
    }
}

/// A strip of tabs: one Tab stop, arrows move the lead without showing its tab, a click, Space or Enter shows the tab,
/// Enter and a double click activate, Cmd/Ctrl+] and [ (or a drag) move the tab later and earlier, Shift to the ends, the menu key
/// asks for the tab's context menu.
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
    #[style_key]
    kind: TabKind,
    /// The strip's taller form with the body type.
    #[prop(flag)]
    large: bool,
    /// The look of a panel strip's tabs ([`TabKind::Panel`]).
    #[prop]
    panel_look: super::PanelTabLook,
    /// The tab the host shows. A click, Space, Enter or a drop reports the new one in the output.
    #[prop]
    shown: Option<K>,
    /// A tab dragged out and released where no target takes it is reported as `torn_off`.
    #[prop(flag)]
    tear_off: bool,
    /// The wheel over the strip shows the next or previous tab, one per wheel notch, stopping at the ends; not with
    /// Ctrl or Cmd held.
    #[prop(flag)]
    wheel_selects: bool,
    /// Which tabs may be dragged.
    #[prop(func = "Fn(&K) -> bool")]
    can_drag: Option<RowTest<'a, K>>,
    /// Which tabs a middle click closes, where the kind closes on one (`TabKind::middle_click_closes`). Unset, the
    /// kind's default (`TabKind::closes_by_default`): every document tab, no panel tab. A host whose tabs differ from
    /// that default (`Tab::closable`) names the ones that close.
    #[prop(func = "Fn(&K) -> bool")]
    can_close: Option<RowTest<'a, K>>,
    /// The role of each tab, an input of the fit: contextual tabs are kept ahead of plain tabs.
    #[prop(func = "Fn(&K) -> TabRole")]
    roles: Option<TabRoles<'a, K>>,
    /// What the strip takes from drags that did not start in it, in order.
    drops: Vec<DropKind<'a, K>>,
    /// Where the strip's own drag may land: the test gets the drag, the tab under the pointer and the position before which the
    /// dragged tabs would go.
    #[prop(func = "Fn(&ListDrag<K>, Option<&K>, usize) -> bool")]
    accepts: Option<DropTest<'a, K>>,
    /// Sets the items at the reading end of the strip, beside the tabs: each runs its command or opens its menu. When
    /// the room runs short they drop their labels, the last item first, before any tab overflows; they keep their
    /// icons whatever the room.
    #[prop]
    trailing: &'a [StripEnd<'a>],
    /// The contextual tabs fading out this pass, each as opaque as its presence.
    ghosts: Vec<Fading>,
    held: Bind<'a, Selection<K>>,
    report: Arc<Mutex<Report>>,
    /// Each tab's key and name, as the source listed them this pass.
    texts: RefCell<Vec<(K, String)>>,
    more: Option<Rect>,
    picked: Option<K>,
    torn: Option<TornOff<K>>,
    /// The tab the wheel stepped to this pass.
    wheeled: Option<K>,
    empty_drag: bool,
    empty_double: bool,
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> TabStrip<'a, K> {
    /// Sets the kind of strip: its fit rule and overflow menu, and the kind its tabs take. A strip of another kind than
    /// `Document` does not select the tab a drag rests over.
    #[must_use]
    pub fn kind(mut self, kind: TabKind) -> Self {
        self.kind = kind;
        self.items = self.items.select_under_drag(kind.selects_under_drag()).primary_picks(kind.primary_shows());
        self
    }

    /// A strip named `label` of document tabs.
    pub fn new(label: impl Into<Label>) -> Self {
        TabStrip {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::FillWidth),
            items: CollectionProps::new()
                .select(SelectMode::Single)
                .pages(true)
                .arrows_select(false)
                .overflow(craft_behavior::Axis::X)
                .select_under_drag(TabKind::Document.selects_under_drag())
                .primary_picks(TabKind::Document.primary_shows()),
            kind: TabKind::Document,
            large: false,
            panel_look: super::PanelTabLook::Plain,
            shown: None,
            tear_off: false,
            wheel_selects: false,
            can_drag: None,
            can_close: None,
            roles: None,
            drops: Vec::new(),
            accepts: None,
            trailing: &[],
            ghosts: Vec::new(),
            held: Bind::kept(),
            report: Arc::new(Mutex::new(Report::default())),
            texts: RefCell::new(Vec::new()),
            more: None,
            picked: None,
            torn: None,
            wheeled: None,
            empty_drag: false,
            empty_double: false,
        }
        .label(label)
    }

    /// A pointer press on a tab (primary or secondary, a drag's press too) leaves key focus where it is; the keyboard
    /// still reaches the strip by Tab. A dock's panel tabs: clicking one leaves the document's keys working.
    #[must_use]
    pub fn keeps_focus(mut self) -> Self {
        self.items = self.items.focus_on_press(false);
        self
    }

    /// A tab drag carries `map` of it, for the drop targets of type `P`; a tab the map gives `None` for does not drag
    /// unless a reorder rule is set. The drag stays one session with one ghost.
    #[must_use]
    pub fn drag_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<K>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.items = self.items.drag_payload(map);
        self
    }

    /// Takes the drops of `kind` from drags that did not start in the strip; kinds given earlier come first.
    #[must_use]
    pub fn takes_drop(mut self, kind: DropKind<'a, K>) -> Self {
        self.drops.push(kind);
        self
    }

    /// With `wheel_selects`, the wheel's whole notches step the shown tab through the tabs in the source's order:
    /// down to later tabs, up to earlier ones. The scroll short of a notch is kept for the next pass, so a trackpad's
    /// small deltas add up; it is dropped when the scroll reverses, when the strip stops at an end, and when the
    /// shown tab is not the one the last step left.
    fn wheel_step(&mut self, cx: &mut Cx<'_, Self>) {
        if !self.wheel_selects {
            return;
        }
        let Some(wheel) = cx.take_wheel().filter(|wheel| !wheel.mods.cmd && !wheel.mods.ctrl) else { return };
        let notch = self.items.wheel_notch(cx.style.line).points(cx.rect.height());
        if !notch.is_finite() || notch <= 0.0 || !wheel.scroll.y.is_finite() {
            return;
        }
        let mut carry = cx.memory::<WheelCarry<K>>().clone();
        if carry.left != self.shown || carry.owed * wheel.scroll.y < 0.0 {
            carry.owed = 0.0;
        }
        carry.owed += wheel.scroll.y;
        let steps = (carry.owed / notch).trunc();
        carry.owed -= steps * notch;
        let texts = self.texts.borrow();
        let at = self.shown.as_ref().and_then(|shown| texts.iter().position(|(key, _)| key == shown));
        if let Some(at) = at
            && steps != 0.0
        {
            let last = texts.len().saturating_sub(1) as i64;
            let to = (at as i64 - steps as i64).clamp(0, last) as usize;
            if to == at {
                carry.owed = 0.0;
            } else {
                self.wheeled = Some(texts[to].0.clone());
            }
        }
        carry.left = self.wheeled.clone().or_else(|| self.shown.clone());
        *cx.memory::<WheelCarry<K>>() = carry;
    }

    fn report(&self) -> Report {
        self.report.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The part of the row no tab, no overflow button and no end item takes, in screen space.
    fn empty_room(&self, content: Rect, rtl: bool) -> Rect {
        let report = self.report();
        let rest = Flow::new(content, rtl).take_end(report.ends_room).1.take_start(report.tabs_end).1;
        let rest = match (report.overflow > 0.0, self.kind.overflow_place()) {
            (false, _) => rest,
            (true, OverflowPlace::StripEnd) => rest.take_end(report.overflow).1,
            (true, OverflowPlace::AfterTabs) => rest.take_start(report.overflow).1,
        };
        rest.rect()
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

    /// The leave motion of a ribbon strip with `roles`: every contextual tab the source lists drives its own keyed
    /// presence open, carrying its name and slot; one the source no longer lists comes back from `exiting` and fades
    /// out at its last slot. It is drawn only, so it has no hit area, no access node and no overflow entry, and the
    /// presence drops its name and slot once the fade ends. A tab that comes back stops fading; one that enters is an
    /// item of the source like any other, hit and listed from its first pass.
    ///
    /// The strip keeps nothing of its own. A rerun pass of one frame drives the same keys at the same clock, so it
    /// draws what the first pass drew.
    fn track_leaving(&mut self, cx: &mut Cx<'_, Self>) {
        self.ghosts.clear();
        let Some(roles) = self.roles.as_ref().filter(|_| self.kind.slides_underline()) else { return };
        let report = self.report();
        let texts = self.texts.borrow();
        for (index, (key, label)) in texts.iter().enumerate() {
            if roles.role(key) != TabRole::Contextual {
                continue;
            }
            let width = report.placed.iter().find(|(at, ..)| *at == index).map(|&(_, _, width)| width);
            let _ = cx.presence_keyed(LEAVE, key, true, Ghost { label: label.clone(), before: index, width }, spec::OPEN_FADE);
        }
        drop(texts);
        // The same keys in the same order as the layout reserved their slots from (`reserved`).
        self.ghosts = cx
            .exiting::<Ghost>(LEAVE, spec::OPEN_FADE)
            .into_iter()
            .map(|(presence, ghost)| Fading { label: ghost.label, alpha: presence.t })
            .collect();
    }

    /// The menus of the end items that open one, each below its item.
    fn end_menus(&self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        for slot in self.report().ends {
            let Some(end) = self.trailing.get(slot.item).filter(|end| end.opens_menu()) else { continue };
            let popup = cx.popup(ENDS).at(slot.item);
            let layer = crate::menu::layer(false).anchor(ui.anchor_part(END.at(slot.item as u64)));
            let name = end.label().clone();
            let ran = ui.show(popup, layer, |ui, _| Menu::inline(name.clone()).show(ui, |ui, _| end.menu_rows(ui)).inner.flatten()).flatten();
            if let Some(command) = ran {
                cx.fire(command, FireSource::Menu);
            }
        }
    }

    /// The one underline of a ribbon strip under its shown tab. Its start and width run toward the shown tab's slot
    /// from where they are painted, so a tab shown mid-motion turns the underline where it is; under reduced motion
    /// they jump (`Animate`).
    fn paint_underline(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, report: &Report) {
        let texts = self.texts.borrow();
        let Some(&(_, start, width)) =
            report.placed.iter().find(|(index, ..)| texts.get(*index).is_some_and(|(key, _)| self.shown.as_ref() == Some(key)))
        else {
            return;
        };
        let start = cx.anim("underline_start", start, spec::MOVE);
        let width = cx.anim("underline_width", width, spec::MOVE);
        let side = cx.style.underline_side.min(width / 2.0).max(0.0);
        let rect = Flow::new(cx.core.content_rect, cx.flow.is_rtl()).span(start + side, width - 2.0 * side);
        if let Some(height) = cx.style.underline_height {
            let bar = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - height), rect.right_bottom());
            ui.draw().fill(&PaintShape::Rect(bar, egui::CornerRadius::ZERO), cx.style.underline);
            return;
        }
        let y = rect.bottom() - cx.style.underline_inset;
        ui.draw().stroke(&PaintShape::Path(vec![Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)]), StrokeWidth::Thick, cx.style.underline);
    }

    /// The slots the departing contextual tabs keep in the fit, and how many tabs are departing. `listed` are the
    /// keys the source lists now; a tab that sat in the overflow menu keeps no slot.
    fn reserved(&self, cx: &StyleCx<'_>, listed: &[K]) -> (Vec<Reserved>, usize) {
        if self.roles.is_none() || !self.kind.slides_underline() {
            return (Vec::new(), 0);
        }
        let departing = cx.exiting::<Ghost>(LEAVE, spec::OPEN_FADE, listed);
        let reserved = departing
            .iter()
            .enumerate()
            .filter_map(|(slot, (_, ghost))| Some(Reserved { slot, before: ghost.before.min(listed.len()), width: ghost.width? }))
            .collect();
        (reserved, departing.len())
    }

    /// The departed contextual tabs at the slots the fit kept for them, each painted as the `Tab` it was, as opaque
    /// as its presence has left it.
    fn paint_ghosts(&self, ui: &mut PaintUi<'_>, report: &Report) {
        for (at, fading) in self.ghosts.iter().enumerate() {
            if report.ghosts.get(at).copied().flatten().is_some() {
                let tab = Tab::new(fading.label.clone()).kind(TabKind::Ribbon).role(TabRole::Contextual);
                ui.face_faded(GHOST.at(at as u64), tab, fading.alpha);
            }
        }
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> Component for TabStrip<'_, K> {
    const ROLE: Role = Role::TAB_LIST;
    type Style = TabStripStyle;
    type Output = TabStripOutput<K>;

    fn style(cx: &StyleCx<'_>, key: TabStripStyleKey<K>) -> TabStripStyle {
        let tokens = cx.tokens;
        let ribbon = key.kind == TabKind::Ribbon;
        // A ribbon strip starts a regular pad in from its start edge, so its first label stands where the old
        // ribbon's did; its underline runs under the label, not the tab's padding.
        let (lead, side) = (strip_lead(key.kind, tokens), if ribbon { super::tab::ribbon_pad(tokens) - tokens.stroke.thick / 2.0 } else { 0.0 });
        TabStripStyle {
            metrics: Metrics::new().padding(Insets::new(lead, 0.0, 0.0, 0.0)),
            focus: cx.focus_look(),
            line: tokens.typography.body.line_height,
            underline: cx.color(tokens.color.accent.solid),
            underline_inset: tokens.stroke.thick / 2.0,
            underline_side: side,
            underline_height: {
                let look = cx.env::<crate::buttons::RibbonLook>();
                (ribbon && look.underline > 0).then_some(f32::from(look.underline))
            },
        }
    }

    fn sense(&self) -> Sense {
        let mut sense = <<Self as Meta>::Shape as ShowShape>::DEFAULT_SENSE;
        if self.wheel_selects {
            sense |= Sense::WHEEL_Y;
        }
        // A middle click on a tab closes it where the kind closes on one: the collection reports it on the tab.
        if self.kind.middle_click_closes() {
            sense |= Sense::MIDDLE_CLICK;
        }
        sense
    }

    /// The end items at the end of the row, a focus stop each, and the overflow button, at the end of the room the
    /// end items leave or right after the last shown tab.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        self.more = None;
        let report = self.report();
        let flow = Flow::new(cx.content_rect, cx.flow.is_rtl());
        // Alt+Down from the strip itself opens the first declared popup. Focused end items open their own menus.
        if report.overflow > 0.0 {
            let tabs = flow.take_end(report.ends_room).1;
            let slot = match self.kind.overflow_place() {
                OverflowPlace::StripEnd => tabs.take_end(report.overflow).0,
                OverflowPlace::AfterTabs => tabs.take_start(report.tabs_end).1.take_start(report.overflow).0,
            };
            self.more = Some(slot);
            cx.part(OVERFLOW, slot)
                .sense(Sense::CLICK)
                .priority(HitPriority::new(HitLevel::Edge, 0))
                .over()
                .toggles("menu")
                .pointer_only("Alt+Down opens the overflow menu");
        }
        for slot in &report.ends {
            let Some(end) = self.trailing.get(slot.item) else { continue };
            let enabled = end.enabled::<Self>(&**cx);
            let part = cx
                .part(END.at(slot.item as u64), flow.span(slot.start, slot.width))
                .sense(Sense::CLICK | Sense::FOCUS)
                .priority(HitPriority::new(HitLevel::Edge, 0))
                .over()
                .focusable()
                .inactive(!enabled);
            // An icon-only item keeps its name as its tooltip.
            let part = if slot.short { part.tooltip(end.label().clone()) } else { part };
            if end.opens_menu() {
                part.toggles_at(ENDS, slot.item);
            }
        }
        // A tab fading out has a part to paint its face on and no sense, so it is neither hit nor focused.
        for (at, slot) in report.ghosts.iter().enumerate() {
            if let Some((start, width)) = *slot {
                let _ = cx.part(GHOST.at(at as u64), flow.span(start, width));
            }
        }
    }

    /// A drag or a double click that starts on the empty room is reported; a drag of this strip's tabs that no target
    /// takes is a tear-off.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let press = *cx.press();
        let origin = match press.phase {
            PressPhase::Down { start, .. } | PressPhase::DragStart { start, .. } => Some(start.origin),
            _ => None,
        };
        if let Some(origin) = origin
            && self.empty_room(cx.core.content_rect, cx.flow.is_rtl()).contains(origin)
        {
            self.empty_drag = matches!(press.phase, PressPhase::DragStart { .. });
            self.empty_double = press.double_pressed();
        }
        for slot in self.report().ends {
            let Some(end) = self.trailing.get(slot.item) else { continue };
            if let Some(command) = end.command_ref()
                && let Some(source) = cx.part_activated(END.at(slot.item as u64))
            {
                cx.fire(command.clone(), source);
            }
        }
        self.track_leaving(cx);
        self.wheel_step(cx);
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

    /// The end items' menus, and the overflow menu: every tab with the shown one checked, or only the hidden tabs, by
    /// the kind.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        self.end_menus(ui, cx);
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

    /// The overflow button and each end item are buttons named by their labels and announce the menus they open.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if self.more.is_some() {
            let node = access.node(OVERFLOW, Role::BUTTON);
            node.name = Name::Text(cx.words::<StripWords>().more.as_str().to_owned());
            node.has_popup(PopupKind::Menu);
        }
        for slot in self.report().ends {
            let Some(end) = self.trailing.get(slot.item) else { continue };
            let node = access.node(END.at(slot.item as u64), Role::BUTTON);
            node.name = Name::Text(end.label().as_str().to_owned());
            if end.opens_menu() {
                node.has_popup(PopupKind::Menu);
            }
        }
    }

    /// The end items' faces, the sliding underline of a ribbon strip, the tabs fading out, and the overflow button.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let report = self.report();
        for slot in &report.ends {
            if let Some(end) = self.trailing.get(slot.item) {
                end.paint(ui, END.at(slot.item as u64), slot.short);
            }
        }
        if self.kind.slides_underline() {
            self.paint_underline(ui, cx, &report);
        }
        self.paint_ghosts(ui, &report);
        if self.more.is_some() {
            ui.face(OVERFLOW, IconButton::new(IconId::new("more")).label(cx.words::<StripWords>().more.clone()));
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> TabStripOutput<K> {
        let tab_rects = self.tab_rects(&cx);
        let overflow = self.more.map(|rect| (rect, crate::runtime::access::sub_node_id(cx.core.id, OVERFLOW.into())));
        let TabStrip { picked, torn, wheeled, empty_drag, empty_double, can_close, kind, .. } = self;
        let mut response = cx.collection_response();
        let (mut closed, mut menu) = (Vec::new(), None);
        let mut kept = Vec::with_capacity(response.events.len());
        for event in std::mem::take(&mut response.events) {
            match event {
                CollectionEvent::Item(key, TabPartEvent::Close) => closed.push(key),
                CollectionEvent::MiddleClick(key) => {
                    if can_close.as_ref().map_or(kind.closes_by_default(), |test| test.passes(&key)) {
                        closed.push(key);
                    }
                }
                CollectionEvent::ContextMenu(request) => menu = Some(request),
                other => kept.push(other),
            }
        }
        kept.extend(wheeled.map(|key| CollectionEvent::Selection {
            change: SelectionChange::Set(vec![key]),
            by: crate::contract::LeadSource::Pointer,
            mods: craft_keymap::Mods::NONE,
        }));
        response.events = kept;
        TabStripOutput { response, closed, menu, picked, torn_off: torn, tab_rects, empty_drag, empty_double, overflow }
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> Collection for TabStrip<'_, K> {
    type Key = K;
    type ItemLayout = StripLayout;
    type Selection = Selection<K>;
    type Item<'r> = Tab<'r>;
    type ItemEvent = TabPartEvent;
    const MEMBERS: bool = true;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    /// The strip asks for the largest tab's size, which `ideal_width` reads.
    fn measures_items(&self) -> bool {
        true
    }

    /// Where the strip takes its width from its content (a dialog sized by what it holds), every tab fits in a row:
    /// as many as the source holds, each as wide as the widest, so the strip does not send tabs to its overflow menu.
    fn ideal_width(&self, item: Option<Vec2>, rows: usize, _: &StyleCx<'_>) -> f32 {
        item.map_or(0.0, |largest| largest.x * rows as f32)
    }

    /// The shown tab is the single selection; the strip reports a change and the host applies it.
    fn selection(&mut self) -> Option<BindMut<'_, Selection<K>>> {
        self.held = Bind::given(self.shown.clone().map_or_else(Selection::default, Selection::single));
        Some(self.held.reborrow())
    }

    fn item_layout(&self, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> StripLayout {
        // The tabs and end items share the room the strip lead leaves (`strip_lead`), so the strip is no wider than
        // the room it was given.
        let width = (width - strip_lead(self.kind, cx.tokens)).max(0.0);
        let count = source.len();
        *self.texts.borrow_mut() = (0..count).map(|at| (source.key(at), source.text(at).map(|text| text.into_owned()).unwrap_or_default())).collect();
        let selected = self.shown.as_ref().and_then(|key| source.index_of(key)).unwrap_or(usize::MAX);
        let keys: Vec<K> = (0..count).map(|at| source.key(at)).collect();
        // The roles are inputs of the fit: it runs here, before any tab is built.
        let kept = match &self.roles {
            Some(roles) => keys.iter().map(|key| roles.role(key) == TabRole::Contextual).collect(),
            None => Vec::new(),
        };
        let (ghosts, departed) = self.reserved(cx, &keys);
        // Each end item is measured in this pass, with its label and icon only.
        let ends = self.trailing.iter().map(|end| end.fit_item(cx)).collect();
        let look = cx.env::<super::DocumentTabLook>();
        let fitting = Fitting { selected, mode: self.kind.fit_mode_in(look), kept, ends, gap: cx.tokens.space.gap.inline, ghosts, departed };
        // Shrinking document tabs stand apart by the inline gap; the other kinds and looks touch.
        let tab_gap = if self.kind.shrinks_in(look) { cx.tokens.space.gap.inline } else { 0.0 };
        StripLayout::new(count, Vec2::new(width, cx.tokens.size.bar.tab), tab_gap, fitting, self.report.clone(), cx.frame_ctx().clone())
    }

    /// The tab the keyboard is on shows the focus ring, not the whole strip.
    fn rings_lead(&self) -> bool {
        true
    }

    /// Every tab takes the strip's kind, panel look and size.
    fn dress<'r>(&self, tab: Self::Item<'r>, _: &RowState) -> Self::Item<'r> {
        let tab = tab.kind(self.kind).panel_look(self.panel_look);
        if self.large { tab.large() } else { tab }
    }

    fn can_drag(&self, key: &K) -> bool {
        self.can_drag.as_ref().is_none_or(|test| test.passes(key))
    }

    fn drops(&self) -> &[DropKind<'_, K>] {
        &self.drops
    }

    fn accepts(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        self.accepts.as_ref().is_none_or(|test| test.passes(drag, over, insert))
    }
}
