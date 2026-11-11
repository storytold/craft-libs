//! `NodeGraph`, `Node` and `NodeKind`.
//!
//! `NodeGraph` is a collection over a free layout: its items are `Node`s at positions the app stores, and it owns
//! the wires, which it draws under the nodes and registers as parts. Apps add kinds of node by implementing
//! `NodeKind`.

mod kind;
mod node;
#[cfg(feature = "testing")]
mod stories;

pub use kind::{NodeEvent, NodeId, NodeKind, PortRef, Wire, WireDrag};
pub use node::{Node, NodeOutput, NodeStyle};

use crate::contract::{
    Bind, BindMut, Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, PaintUi, PartsCx, Pointer as _, Role,
    SelectMode, Sizing, Styled as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::style::{ComponentStyle, Stateful, States, StyleColor, StyleCx, SurfaceKind};
use craft_behavior::{CanvasTransform, FreeLayout, HitLevel, HitPriority, HitShape, ItemSource, Metrics, PartKey, Selection};
use egui::{Pos2, Rect, Vec2};
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
}

/// A graph of nodes at stored positions, joined by wires. `positions` and `kinds` are indexed by `NodeId`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection, probe(NodeGraph<'static, ()>))]
pub struct NodeGraph<'a, E: 'static = ()> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    wires: &'a [Wire],
    kinds: &'a [&'a dyn NodeKind<E>],
    positions: &'a [Rect],
    /// The selection, owned by the app.
    #[prop]
    selection: Option<Bind<'a, Selection<NodeId>>>,
    zoom: CanvasTransform,
}

impl<'a, E: 'static> NodeGraph<'a, E> {
    /// A graph of the nodes at `positions` (their rects from the graph's top left, in document units) of the
    /// `kinds`, joined by `wires`.
    pub fn new(positions: &'a [Rect], kinds: &'a [&'a dyn NodeKind<E>], wires: &'a [Wire]) -> Self {
        let zoom = CanvasTransform::new(1.0, Vec2::ZERO);
        let props = ContainerProps::new().surface(SurfaceKind::Sunken).arena(true).sizing(Sizing::Fill);
        let items = CollectionProps::new().select(SelectMode::Multi);
        NodeGraph { common: Common::new(), props, items, wires, kinds, positions, selection: None, zoom }
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
    type Output = CollectionResponse<NodeId, NodeEvent<E>>;

    fn style(cx: &StyleCx<'_>, _: NodeGraphStyleKey<E>) -> NodeGraphStyle {
        let color = &cx.tokens.color;
        let rest = cx.color(color.line.mark);
        let states = States::new(rest, cx.color(color.accent.hover), cx.color(color.accent.pressed), rest);
        NodeGraphStyle { metrics: Metrics::new(), wire: Stateful::new(rest).off(states), loose: cx.color(color.accent.solid) }
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

    fn finish(self, mut cx: Cx<'_, Self>) -> CollectionResponse<NodeId, NodeEvent<E>> {
        cx.collection_response()
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

    fn item_layout(&self, source: &dyn ItemSource<NodeId>, _width: f32, _cx: &StyleCx<'_>) -> FreeLayout {
        let rects = (0..source.len()).map(|at| self.positions.get(source.key(at).0 as usize).copied().unwrap_or(Rect::NOTHING)).collect();
        FreeLayout::new(rects)
    }
}
