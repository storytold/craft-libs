//! What the derives write: registrations and probes, style keys per instantiation, setters and style accessors.

use craft_behavior::{CancelReason, ItemSource, Selection, SpanLayout};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::contract::{
    Collection, CollectionProps, Common, Component, Container, ContainerProps, Cx, EditResponse, Pattern, Role, ShapeKind, register_contract,
};
use craft_ui::primitives::Draw;
use craft_ui::style::StyleCx;
use craft_ui::testing::{Probe, trial};
use craft_ui_test_kit::contract::{PlainStyle, RowProbe};
use egui::Response;
use std::marker::PhantomData;

/// Two codecs, as type arguments of a generic component.
struct Plain;
struct Number;

/// A generic value leaf whose style depends on a kind field.
#[derive(Component)]
#[component(probe(Tagged<Plain>, Tagged<Number>))]
struct Tagged<C: 'static> {
    #[common]
    common: Common,
    #[style_key]
    kind: u8,
    codec: PhantomData<fn() -> C>,
}

impl<C: 'static> Component for Tagged<C> {
    const ROLE: Role = Role::SLIDER;
    type Style = PlainStyle;
    type Output = EditResponse<u8>;

    fn style(_: &StyleCx<'_>, _: TaggedStyleKey<C>) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<u8> {
        craft_ui::testing::contract::edit_response(cx.response.clone())
    }
}

/// A canvas control that is exempt from a shared case.
#[derive(Component)]
#[component(canvas_control, exempt(case = "sizes", reason = "fills the canvas"))]
struct Handle {
    #[common]
    common: Common,
}

impl Component for Handle {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: HandleStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A container that declares an open state.
#[derive(Component)]
#[component(container, overlay)]
struct Frame {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Frame {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: FrameStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Frame {}

/// A collection: one row per key.
#[derive(Component)]
#[component(collection)]
struct Rows {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
}

impl Component for Rows {
    const ROLE: Role = Role::LIST;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: RowsStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Collection for Rows {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = Selection<usize>;
    type Item<'r> = RowProbe;
    type ItemEvent = ();

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, _: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::new(source, width, 10.0)
    }
}

/// A row of tools: a pattern assembles components and has no contract shape of its own.
#[derive(Pattern)]
struct ToolRow;

/// A pattern whose impl is written by hand.
struct Handwritten;

impl Pattern for Handwritten {}

register_contract!(Handwritten; pattern);

/// A tool that does nothing, registered as a story host of the tool kind.
#[derive(craft_ui::contract::Pattern)]
#[pattern(tool)]
struct IdleTool;

impl CanvasTool for IdleTool {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("none; the tool does nothing")
    }
}

/// Every probe linked into this binary, the integration binary's own fixtures included.
fn probes() -> Vec<Probe> {
    craft_ui::contract::registrations().into_iter().filter_map(|registration| registration.probe().copied()).collect()
}

fn named(name: &str) -> Vec<Probe> {
    probes().into_iter().filter(|probe| probe.type_name().contains(name)).collect()
}

#[trial]
fn a_registration_records_the_facts_the_contract_types_state() {
    let tagged = named("::Tagged<");
    assert_eq!(tagged.len(), 2, "one probe per listed instantiation");
    assert!(tagged.iter().any(|probe| probe.type_name().ends_with("Plain>")) && tagged.iter().any(|probe| probe.type_name().ends_with("Number>")));
    for probe in &tagged {
        assert_eq!((probe.shape, probe.role, probe.crate_name), (ShapeKind::Leaf, Role::SLIDER, env!("CARGO_PKG_NAME")));
        assert!(probe.carries_value() && !probe.overlay && !probe.canvas_control && probe.exempt.is_empty());
    }

    let handle = named("derives::Handle");
    assert_eq!(handle.len(), 1);
    assert!(handle[0].canvas_control && !handle[0].carries_value());
    assert_eq!((handle[0].exempt[0].case, handle[0].exempt[0].reason), ("sizes", "fills the canvas"));

    let frame = named("derives::Frame");
    assert_eq!(frame[0].shape, ShapeKind::Body);
    assert!(frame[0].overlay, "`#[component(container, overlay)]` declares the open state");
    assert_eq!(named("derives::Rows")[0].shape, ShapeKind::Items);
}
