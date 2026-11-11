//! Components for the contract trials, written with the derives: a leaf, a container and a collection.

use craft_behavior::{Intent, ItemSource, Metrics, PartRef, PressPhase, Selection, Sense, SpanLayout};
use craft_ui::contract::{
    Collection, CollectionProps, Common, Component, Container, ContainerProps, Cx, Focus as _, MeasureCx, Pointer as _, Props as _, Role,
};
use craft_ui::runtime::commands::FireSource;
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Id, Rect, Response, Vec2};
use std::cell::RefCell;

/// A style with a minimum size.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PlainStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl PlainStyle {
    /// A style whose component is at least `min` large.
    pub fn min(min: Vec2) -> Self {
        PlainStyle { metrics: Metrics::new().min_size(min) }
    }
}

/// What a probe saw while it ran.
#[non_exhaustive]
pub struct Seen {
    /// The rect of the last `update`.
    pub rect: Rect,
    /// The id of the last `update`.
    pub id: Option<Id>,
    /// The press phases of the probe's own shapes, one per pass that had a press.
    pub press: Vec<PressPhase>,
    /// How many passes had an activation.
    pub activations: u32,
    /// How many cancel intents the component took.
    pub cancels: u32,
    /// The active part in the last `update`.
    pub active: Option<PartRef>,
    /// How many times `update` ran.
    pub updates: u32,
}

/// Records what the pipeline gave the component in `update`.
fn record<C: Component>(seen: &RefCell<Seen>, takes_cancel: bool, cx: &mut Cx<'_, C>) {
    let mut seen = seen.borrow_mut();
    seen.updates += 1;
    seen.rect = cx.rect;
    seen.id = Some(cx.id);
    let phase = cx.press().phase;
    if phase.is_press() {
        seen.press.push(phase);
    }
    seen.activations += u32::from(cx.activation().is_some());
    seen.cancels += u32::from(takes_cancel && cx.intents().take(Intent::Cancel));
    seen.active = cx.active_part();
}

impl Default for Seen {
    fn default() -> Self {
        Seen { rect: Rect::NOTHING, id: None, press: Vec::new(), activations: 0, cancels: 0, active: None, updates: 0 }
    }
}

/// A leaf of a given size that records what the pipeline gives it.
#[derive(Component)]
pub struct Probe<'a> {
    #[common]
    common: Common,
    size: Vec2,
    sense: Option<Sense>,
    takes_cancel: bool,
    injects: Option<FireSource>,
    seen: &'a RefCell<Seen>,
}

impl<'a> Probe<'a> {
    /// A 40 × 20 probe, labelled "Probe", that records into `seen`.
    pub fn new(seen: &'a RefCell<Seen>) -> Self {
        Probe { common: Common::new(), size: Vec2::new(40.0, 20.0), sense: None, takes_cancel: true, injects: None, seen }.label("Probe")
    }

    /// The probe's size.
    pub fn size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    /// What the probe senses.
    pub fn sense(mut self, sense: Sense) -> Self {
        self.sense = Some(sense);
        self
    }
}

impl Component for Probe<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ProbeStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        self.size
    }

    fn sense(&self) -> Sense {
        self.sense.unwrap_or(Sense::CLICK | Sense::FOCUS)
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        record(self.seen, self.takes_cancel, cx);
        if let Some(source) = self.injects
            && cx.activation().is_some()
        {
            cx.activate(source);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A container whose body is the caller's closure.
#[derive(Component)]
#[component(container)]
pub struct Panel<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    sense: Sense,
    seen: Option<&'a RefCell<Seen>>,
}

impl<'a> Panel<'a> {
    /// A panel with an empty body that senses nothing.
    pub fn new() -> Self {
        Panel { common: Common::new(), props: ContainerProps::new(), sense: Sense::empty(), seen: None }
    }

    /// Records what the panel gets in `update` into `seen`.
    pub fn seen(mut self, seen: &'a RefCell<Seen>) -> Self {
        self.seen = Some(seen);
        self
    }

    /// Whether the panel's body is an arena.
    pub fn arena(mut self, on: bool) -> Self {
        self.props = self.props.arena(on);
        self
    }

    /// What the panel senses.
    pub fn sense(mut self, sense: Sense) -> Self {
        self.sense = sense;
        self
    }
}

impl Component for Panel<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PanelStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        self.sense
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(seen) = self.seen {
            record(seen, true, cx);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Panel<'_> {}

/// A collection of rows of fixed height.
#[derive(Component)]
#[component(collection)]
pub struct Table {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    row_h: f32,
}

impl Component for Table {
    const ROLE: Role = Role::LIST;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TableStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Collection for Table {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = Selection<usize>;
    type Item<'r> = RowProbe;
    type ItemEvent = ();

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, _: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::new(source, width, self.row_h)
    }
}

/// One row of a `Table`: it records its key and the arena it joined.
#[non_exhaustive]
#[derive(Component)]
pub struct RowProbe {
    #[common]
    common: Common,
    /// Where each row records its key and rect.
    pub log: &'static std::sync::Mutex<Vec<(usize, Rect)>>,
    key: usize,
}

impl Component for RowProbe {
    const ROLE: Role = Role::LIST_ITEM;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: RowProbeStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(10.0, 10.0))
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Ok(mut log) = self.log.lock() {
            log.push((self.key, cx.rect));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
