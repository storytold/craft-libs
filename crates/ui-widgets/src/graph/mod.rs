//! `NodeGraph`, `Node` and `NodeKind`.
//!
//! `NodeGraph` is a collection over a free layout: its items are `Node`s at positions the app stores, and it owns
//! the wires, which it draws under the nodes and registers as parts. Apps add kinds of node by implementing
//! `NodeKind`.

mod annotation;
mod kind;
pub(crate) mod node;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use annotation::{AnnotationEvent, AnnotationKey, Backdrop, Marker, Note};
pub use kind::{NodeEvent, NodeFooter, NodeHeader, NodeId, NodeKind, NodeShape, PortDirection, PortLabels, PortRef, PortSide, Wire, WireDrag};
pub use node::{Node, NodeOutput, NodeStyle};

use crate::contract::{
    Bind, BindMut, Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, DropKind, NoValue, Outcome, PaintUi,
    PartsCx, Pointer as _, Role, SelectMode, Sizing, Styled as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::style::{ComponentStyle, Stateful, States, StyleColor, StyleCx, SurfaceKind, TypeRole};
use craft_behavior::{CanvasTransform, FreeLayout, HitLevel, HitPriority, HitShape, ItemSource, Metrics, PartKey, PartRef, Selection};
use egui::{Pos2, Rect, Response, Vec2};
use node::port_center;

const WIRE: PartKey = PartKey::new("wire");

/// How many straight segments a wire's curve is cut into: its hit shape and its paint are the same polyline.
const CURVE_SEGMENTS: usize = 16;

/// How far a wire's control points reach along the line between its ends, as a share of its horizontal span.
const TENSION: f32 = 0.5;

/// The curve of a wire from `from` to `to`: a cubic that leaves and enters horizontally.
fn curve(from: Pos2, to: Pos2) -> Vec<Pos2> {
    let lead = (to.x - from.x).abs() * TENSION;
    let (c1, c2) = (from.to_vec2() + Vec2::new(lead, 0.0), to.to_vec2() - Vec2::new(lead, 0.0));
    (0..=CURVE_SEGMENTS)
        .map(|at| {
            let t = at as f32 / CURVE_SEGMENTS as f32;
            let u = 1.0 - t;
            (from.to_vec2() * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + to.to_vec2() * (t * t * t)).to_pos2()
        })
        .collect()
}

/// The style of a node graph: the wires' colour, which takes the accent under the pointer.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct NodeGraphStyle {
    #[style(metrics)]
    metrics: Metrics,
    wire: Stateful<StyleColor>,
    /// The wire a drag is making.
    loose: StyleColor,
    /// The edge of a backdrop.
    #[allow(dead_code)] // P3(graph-w4): ge.C23
    backdrop_edge: Stateful<StyleColor>,
    /// The type role of a node's title.
    #[allow(dead_code)] // P3(graph-w4): ge.C23
    title_role: TypeRole,
    /// The type role of an annotation's text.
    #[allow(dead_code)] // P3(graph-w4): ge.C23
    text_role: TypeRole,
    /// The ink of a note's text.
    #[allow(dead_code)] // P3(graph-w4): ge.C23
    note_text: Stateful<StyleColor>,
    /// The ink of a marker.
    #[allow(dead_code)] // P3(graph-w4): ge.C23
    marker: StyleColor,
    /// The ink of a backdrop's resize grip.
    #[allow(dead_code)] // P3(graph-w4): ge.C23
    grip: StyleColor,
    /// The least size a backdrop resizes to.
    #[allow(dead_code)] // P3(graph-w4): ge.C23
    backdrop_min: Vec2,
    /// The side of a backdrop's resize grip.
    #[allow(dead_code)] // P3(graph-w4): ge.C23
    grip_side: f32,
    /// The ink of a wire that touches a selected node and is not itself selected.
    #[allow(dead_code)] // P3(graph-w4): ge.C23
    hot: StyleColor,
}

/// A graph of nodes at stored positions, joined by wires. `positions` and `kinds` are indexed by `NodeId`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection, probe(NodeGraph<'static, ()>))]
pub struct NodeGraph<'a, E: 'static = ()> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    #[collection]
    items: CollectionProps,
    drops: Vec<DropKind<'a, NodeId>>,
    wires: &'a [Wire],
    kinds: &'a [&'a dyn NodeKind<E>],
    positions: &'a [Rect],
    /// The selection, owned by the app.
    #[prop]
    selection: Option<Bind<'a, Selection<NodeId>>>,
    /// The wires selected, by `Wire::id`: a plain click selects one alone and clears the nodes; Shift adds, Cmd
    /// toggles; a plain press on a node or on empty space clears the wires.
    #[prop]
    #[allow(dead_code)] // P3(graph-w4): ge.C20
    wire_selection: Option<Bind<'a, Selection<u64>>>,
    /// When the ports' names show; it reaches every node.
    #[prop]
    #[allow(dead_code)] // P3(graph-w4): ge.C20
    port_labels: PortLabels,
    zoom: CanvasTransform,
    #[allow(dead_code)] // P3(graph-w4): ge.C26
    backdrops: &'a [Backdrop<'a>],
    #[allow(dead_code)] // P3(graph-w4): ge.C26
    notes: &'a [Note<'a>],
    #[allow(dead_code)] // P3(graph-w4): ge.C26
    markers: &'a [Marker<'a>],
    #[allow(dead_code)] // P3(graph-w4): ge.C26
    annotation_selection: Option<Bind<'a, Option<AnnotationKey>>>,
}

/// What a `NodeGraph` reports: each family's events, in order.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the graph's events"]
pub struct NodeGraphResponse<E> {
    /// The collection's response and its events, with the nodes' own.
    pub collection: CollectionResponse<NodeId, NodeEvent<E>>,
    /// What the wires reported.
    pub wires: Vec<WireEvent>,
    /// What the annotations reported.
    pub annotations: Vec<AnnotationEvent>,
}

impl<E> Outcome for NodeGraphResponse<E> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.collection.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.collection.story_facts()
    }
}

/// What a wire of the graph reports.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum WireEvent {
    /// A double click on the wire, at a point in document units.
    DoubleClicked {
        /// The wire's id.
        wire: u64,
        /// Where, in document units.
        at: Pos2,
    },
    /// The context menu was asked for on the wire, at a screen point.
    ContextMenu {
        /// The wire's id.
        wire: u64,
        /// Where the menu opens.
        at: Pos2,
    },
    /// A primary press on the wire moved past the slop; reported once, at a point in document units.
    Pulled {
        /// The wire's id.
        wire: u64,
        /// Where, in document units.
        at: Pos2,
    },
}

/// The control points of the cubic from `from` to `to`: each end leads along its side's outward normal.
pub fn wire_controls(from: Pos2, from_side: PortSide, to: Pos2, to_side: PortSide) -> [Pos2; 4] {
    let _ = (from, from_side, to, to_side);
    todo!("ge.C24")
}

impl<'a, E: 'static> NodeGraph<'a, E> {
    /// A graph of the nodes at `positions` (their rects from the graph's top left, in document units) of the
    /// `kinds`, joined by `wires`.
    pub fn new(positions: &'a [Rect], kinds: &'a [&'a dyn NodeKind<E>], wires: &'a [Wire]) -> Self {
        let zoom = CanvasTransform::new(1.0, Vec2::ZERO);
        let props = ContainerProps::new().surface(SurfaceKind::Sunken).arena(true).sizing(Sizing::Fill);
        let items = CollectionProps::new().select(SelectMode::Multi);
        NodeGraph {
            common: Common::new(),
            props,
            items,
            drops: Vec::new(),
            wires,
            kinds,
            positions,
            selection: None,
            wire_selection: None,
            port_labels: PortLabels::default(),
            zoom,
            backdrops: &[],
            notes: &[],
            markers: &[],
            annotation_selection: None,
        }
    }

    /// Takes the drops of `kind` from drags that did not start in the graph; kinds given earlier come first. A kind
    /// at `DropPlace::Spot` reports where in the graph it was dropped.
    #[must_use]
    pub fn takes_drop(self, kind: DropKind<'a, NodeId>) -> Self {
        let _ = (self, kind);
        todo!("ge.C19")
    }

    /// The backdrops, which paint first under the items.
    #[must_use]
    pub fn backdrops(self, backdrops: &'a [Backdrop<'a>]) -> Self {
        let _ = (self, backdrops);
        todo!("ge.C26")
    }

    /// The notes, over the parts.
    #[must_use]
    pub fn notes(self, notes: &'a [Note<'a>]) -> Self {
        let _ = (self, notes);
        todo!("ge.C26")
    }

    /// The markers, one sub-level above the notes.
    #[must_use]
    pub fn markers(self, markers: &'a [Marker<'a>]) -> Self {
        let _ = (self, markers);
        todo!("ge.C26")
    }

    /// The selected annotation, apart from the node selection: one at a time.
    #[must_use]
    pub fn annotation_selection(self, selection: Bind<'a, Option<AnnotationKey>>) -> Self {
        let _ = (self, selection);
        todo!("ge.C26")
    }

    /// Shows the nodes zoomed and moved by `zoom`, so their inline controls keep working at any scale.
    #[must_use]
    pub fn zoom(mut self, zoom: CanvasTransform) -> Self {
        self.props = self.props.transform(zoom);
        self.zoom = zoom;
        self
    }

    /// A port's position on screen: the node's place from the graph plus the kind's `port_offset`, through the
    /// zoom. The one function behind the wires' parts, their paint and the loose wire.
    fn port_point(&self, origin: Pos2, port: PortRef) -> Option<Pos2> {
        let rect = self.positions.get(port.node.0 as usize)?;
        let kind = self.kinds.get(port.node.0 as usize)?;
        Some(origin + self.zoom.to_layer(port_center(*kind, *rect, port.port)).to_vec2())
    }

    fn wire_curve(&self, origin: Pos2, wire: &Wire) -> Option<Vec<Pos2>> {
        Some(curve(self.port_point(origin, wire.from)?, self.port_point(origin, wire.to)?))
    }
}

impl<E: 'static> Component for NodeGraph<'_, E> {
    const ROLE: Role = Role::GROUP;
    type Style = NodeGraphStyle;
    type Output = NodeGraphResponse<E>;

    fn style(cx: &StyleCx<'_>, _: NodeGraphStyleKey<E>) -> NodeGraphStyle {
        let color = &cx.tokens.color;
        let rest = cx.color(color.line.mark);
        let states = States::new(rest, cx.color(color.accent.hover), cx.color(color.accent.pressed), rest);
        // P3(graph-w4): ge.C23
        NodeGraphStyle {
            metrics: Metrics::new(),
            wire: Stateful::new(rest).off(states),
            loose: cx.color(color.accent.solid),
            backdrop_edge: Stateful::new(rest),
            title_role: TypeRole::Label,
            text_role: TypeRole::Body,
            note_text: Stateful::new(cx.color(color.text.normal)),
            marker: rest,
            grip: rest,
            backdrop_min: Vec2::ZERO,
            grip_side: 0.0,
            hot: cx.color(color.accent.solid),
        }
    }

    /// Each wire is a polyline part at `Edge` priority: a press on a port beats the wire, which sits under the nodes.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        for wire in self.wires {
            if let Some(points) = self.wire_curve(rect.min, wire) {
                cx.part(WIRE.at(wire.id), HitShape::polyline(points, false)).priority(HitPriority::new(HitLevel::Edge, 0));
            }
        }
    }

    /// The wires, under the nodes.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        for wire in self.wires {
            if let Some(points) = self.wire_curve(cx.rect.min, wire) {
                draw.stroke(&PaintShape::Path(points), StrokeWidth::Medium, cx.resolve_part(WIRE.at(wire.id), &cx.style.wire));
            }
        }
    }

    /// The loose wire, from the port a drag started on to the pointer, over the nodes.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some((wire, at)) = cx.dragging::<WireDrag>() else { return };
        if let Some(from) = self.port_point(cx.rect.min, wire.from) {
            ui.draw().stroke(&PaintShape::Path(curve(from, at)), StrokeWidth::Medium, cx.style.loose);
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> NodeGraphResponse<E> {
        // P3(graph-w4): ge.C21
        NodeGraphResponse { collection: cx.collection_response(), wires: Vec::new(), annotations: Vec::new() }
    }
}

impl<E: 'static> Collection for NodeGraph<'_, E> {
    type Key = NodeId;
    type ItemLayout = FreeLayout;
    type Selection = Selection<NodeId>;
    type Item<'r> = Node<'r, E>;
    type ItemEvent = NodeEvent<E>;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<NodeId>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn drops(&self) -> &[DropKind<'_, NodeId>] {
        &self.drops
    }

    fn moves_from_part(&self, part: PartRef) -> bool {
        part.key == WIRE
    }

    fn item_layout(&self, source: &dyn ItemSource<NodeId>, _width: f32, _cx: &StyleCx<'_>) -> FreeLayout {
        let rects = (0..source.len()).map(|at| self.positions.get(source.key(at).0 as usize).copied().unwrap_or(Rect::NOTHING)).collect();
        FreeLayout::new(rects)
    }
}
