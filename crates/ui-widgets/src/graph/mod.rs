//! `NodeGraph`, `Node` and `NodeKind`.
//!
//! `NodeGraph` is a collection over a free layout: its items are `Node`s at positions the app stores, and it owns
//! the wires, which it draws under the nodes and registers as parts, with a selection of their own. Backdrops paint
//! under the nodes and notes and markers over them, apart from the node selection. Apps add kinds of node by
//! implementing `NodeKind`.

mod annotation;
#[cfg(feature = "suite")]
pub(crate) mod annotation_stories;
mod kind;
pub(crate) mod node;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use annotation::{AnnotationEvent, AnnotationKey, Backdrop, Marker, Note};
pub use kind::{NodeEvent, NodeFooter, NodeHeader, NodeId, NodeKind, NodeShape, PortDirection, PortLabels, PortRef, PortSide, Wire, WireDrag};
pub use node::{Node, NodeOutput, NodeStyle};

use crate::contract::{
    Bind, BindMut, Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, DropKind, NoValue, Outcome, PaintUi,
    PartsCx, Pointer as _, Role, RowState, SelectMode, Sizing, Styled as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::style::{ComponentStyle, Flags, Stateful, States, StyleColor, StyleCx, SurfaceKind, TypeRole, UiTokens};
use annotation::Layers;
use craft_behavior::{
    CanvasTransform, EditEvent, FreeLayout, HitLevel, HitPriority, HitShape, ItemSource, Metrics, Mixed, PartKey, PartRef, PointerButton, PressPhase,
    Selection, SelectionChange, SelectionModel, Sense, Value,
};
use craft_keymap::Mods;
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};
use node::port_center;

const WIRE: PartKey = PartKey::new("wire");

/// How many straight segments a wire's curve is cut into: its hit shape and its paint are the same polyline.
const CURVE_SEGMENTS: usize = 16;

/// How far a wire's control points reach along its side's normal, as a share of the span between its ends on that
/// axis.
const TENSION: f32 = 0.5;

/// The least reach of a wire's control points, in points, so a short or folded-back wire still leaves its port
/// along the side's normal.
const MIN_LEAD: f32 = 30.0;

/// The outward normal of `side`.
fn normal(side: PortSide) -> Vec2 {
    match side {
        PortSide::Left => -Vec2::X,
        PortSide::Right => Vec2::X,
        PortSide::Top => -Vec2::Y,
        PortSide::Bottom => Vec2::Y,
    }
}

/// The side facing `side`: the side a loose wire's free end enters by.
fn opposite(side: PortSide) -> PortSide {
    match side {
        PortSide::Left => PortSide::Right,
        PortSide::Right => PortSide::Left,
        PortSide::Top => PortSide::Bottom,
        PortSide::Bottom => PortSide::Top,
    }
}

/// The cubic of `controls` cut into `CURVE_SEGMENTS` straight segments: the wire's hit shape and its paint.
fn sampled([from, c1, c2, to]: [Pos2; 4]) -> Vec<Pos2> {
    (0..=CURVE_SEGMENTS)
        .map(|at| {
            let t = at as f32 / CURVE_SEGMENTS as f32;
            let u = 1.0 - t;
            (from.to_vec2() * (u * u * u) + c1.to_vec2() * (3.0 * u * u * t) + c2.to_vec2() * (3.0 * u * t * t) + to.to_vec2() * (t * t * t))
                .to_pos2()
        })
        .collect()
}

/// Writes `edit` of the value `bind` holds as one set event, when it changes the value. A given binding is not
/// written (`BindMut::write`).
fn edit_selection<T: Value + Clone + Default + PartialEq, C: Component>(mut bind: BindMut<'_, T>, cx: &mut Cx<'_, C>, edit: impl FnOnce(&mut T)) {
    let before = match bind.read(cx) {
        Some(Mixed::Same(held)) => held,
        _ => T::default(),
    };
    let mut after = before.clone();
    edit(&mut after);
    if after != before {
        let label = cx.label_arc();
        let _ = bind.write(cx, &EditEvent::set(Mixed::Same(before), after, label, None));
    }
}

/// The style of a node graph: the wires' colour at rest, under the pointer and selected, the colour of a wire that
/// touches a selected node, and the annotations' looks.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct NodeGraphStyle {
    #[style(metrics)]
    metrics: Metrics,
    wire: Stateful<StyleColor>,
    /// The wire a drag is making.
    loose: StyleColor,
    /// The edge of a backdrop.
    backdrop_edge: Stateful<StyleColor>,
    /// The type role of a node's title.
    title_role: TypeRole,
    /// The type role of an annotation's text.
    text_role: TypeRole,
    /// The ink of a note's text.
    note_text: Stateful<StyleColor>,
    /// The ink of a marker.
    marker: StyleColor,
    /// The corners of a selected marker's outline.
    marker_radius: CornerRadius,
    /// The ink of a backdrop's resize grip.
    grip: StyleColor,
    /// The least size a backdrop resizes to.
    backdrop_min: Vec2,
    /// The side of a backdrop's resize grip.
    grip_side: f32,
    /// The ink of a wire that touches a selected node and is not itself selected.
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
    /// toggles; a secondary press on an unselected wire selects it alone; a plain press on a node or on empty space
    /// clears the wires. Without it no wire is selected.
    #[prop]
    wire_selection: Option<Bind<'a, Selection<u64>>>,
    /// When the ports' names show; it reaches every node.
    #[prop]
    port_labels: PortLabels,
    zoom: CanvasTransform,
    backdrops: &'a [Backdrop<'a>],
    notes: &'a [Note<'a>],
    markers: &'a [Marker<'a>],
    annotation_selection: Option<Bind<'a, Option<AnnotationKey>>>,
    /// What the wires reported this pass.
    wire_events: Vec<WireEvent>,
    /// What the annotations reported this pass.
    annotation_events: Vec<AnnotationEvent>,
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

/// The control points of the cubic from `from` to `to`: each end leads along its side's outward normal by the span
/// between the ends on that normal's axis times the tension, and never less than `MIN_LEAD` points.
pub fn wire_controls(from: Pos2, from_side: PortSide, to: Pos2, to_side: PortSide) -> [Pos2; 4] {
    let lead = |side: PortSide| {
        let normal = normal(side);
        normal * ((to - from).dot(normal).abs() * TENSION).max(MIN_LEAD)
    };
    [from, from + lead(from_side), to + lead(to_side), to]
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
            wire_events: Vec::new(),
            annotation_events: Vec::new(),
        }
    }

    /// Takes the drops of `kind` from drags that did not start in the graph; kinds given earlier come first. A kind
    /// at `DropPlace::Spot` reports where in the graph it was dropped.
    #[must_use]
    pub fn takes_drop(mut self, kind: DropKind<'a, NodeId>) -> Self {
        self.drops.push(kind);
        self
    }

    /// The backdrops, which paint first under the items.
    #[must_use]
    pub fn backdrops(mut self, backdrops: &'a [Backdrop<'a>]) -> Self {
        self.backdrops = backdrops;
        self
    }

    /// The notes, over the parts.
    #[must_use]
    pub fn notes(mut self, notes: &'a [Note<'a>]) -> Self {
        self.notes = notes;
        self
    }

    /// The markers, one sub-level above the notes.
    #[must_use]
    pub fn markers(mut self, markers: &'a [Marker<'a>]) -> Self {
        self.markers = markers;
        self
    }

    /// The selected annotation, apart from the node selection: one at a time.
    #[must_use]
    pub fn annotation_selection(mut self, selection: Bind<'a, Option<AnnotationKey>>) -> Self {
        self.annotation_selection = Some(selection);
        self
    }

    /// Shows the nodes zoomed and moved by `zoom`, so their inline controls keep working at any scale. Inside a
    /// `Canvas`, pass the canvas space's `view.layer_transform()`, so the nodes sit where the canvas draws.
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

    /// The side of its node a port sits on, as the node's kind places it.
    fn port_side(&self, port: PortRef) -> Option<PortSide> {
        Some(self.kinds.get(port.node.0 as usize)?.port_side(port.port))
    }

    fn wire_curve(&self, origin: Pos2, wire: &Wire) -> Option<Vec<Pos2>> {
        let (from, from_side) = (self.port_point(origin, wire.from)?, self.port_side(wire.from)?);
        let (to, to_side) = (self.port_point(origin, wire.to)?, self.port_side(wire.to)?);
        Some(sampled(wire_controls(from, from_side, to, to_side)))
    }

    /// A point of the graph's layer, as press positions come, in document units: `port_point` undone.
    fn to_document(&self, origin: Pos2, at: Pos2) -> Pos2 {
        ((at - origin - self.zoom.offset) / self.zoom.scale).to_pos2()
    }

    /// The annotation layers, handed the graph's own slices.
    fn layers(&self) -> Layers<'a> {
        Layers::of(self)
    }

    /// A press on wire `id`, with `mods`: a plain primary press selects it alone, Shift adds it, Cmd toggles it; a
    /// secondary press selects an unselected wire alone and keeps a selected one's selection. The nodes are cleared
    /// unless Shift or Cmd is held or the secondary press keeps the selection.
    fn press_wire(&mut self, cx: &mut Cx<'_, Self>, id: u64, mods: Mods, secondary: bool) {
        let held = self.wire_selection.as_ref().and_then(Bind::value).is_some_and(|wires| wires.contains(&id));
        let (adds, toggles) = (!secondary && mods.shift, !secondary && mods.cmd);
        if let Some(bind) = self.wire_selection.as_mut() {
            edit_selection(bind.reborrow(), cx, |wires: &mut Selection<u64>| {
                if toggles || (adds && !held) {
                    wires.apply(SelectionChange::Toggle(id));
                } else if !(adds || (secondary && held)) {
                    wires.apply(SelectionChange::Set(vec![id]));
                }
            });
        }
        if !(adds || toggles || (secondary && held))
            && let Some(nodes) = self.selection()
        {
            edit_selection(nodes, cx, |nodes: &mut Selection<NodeId>| *nodes = Selection::default());
        }
    }
}

impl<E: 'static> Component for NodeGraph<'_, E> {
    const ROLE: Role = Role::GROUP;
    type Style = NodeGraphStyle;
    type Output = NodeGraphResponse<E>;

    fn style(cx: &StyleCx<'_>, _: NodeGraphStyleKey<E>) -> NodeGraphStyle {
        let color = &cx.tokens.color;
        let (rest, hover, accent) = (cx.color(color.line.mark), cx.color(color.text.normal), cx.color(color.accent.solid));
        let edge = cx.color(color.line.border);
        NodeGraphStyle {
            metrics: Metrics::new(),
            wire: Stateful::new(rest).off(States::new(rest, hover, hover, rest)).when(Flags::SELECTED, accent),
            loose: accent,
            backdrop_edge: Stateful::new(edge).when(Flags::SELECTED, accent),
            title_role: TypeRole::Heading,
            text_role: TypeRole::Body,
            note_text: Stateful::new(cx.color(color.text.normal)).when(Flags::SELECTED, accent),
            marker: accent,
            marker_radius: UiTokens::corner(cx.tokens.shape.radius.sm),
            grip: cx.color(color.text.muted),
            backdrop_min: Vec2::splat(2.0 * cx.tokens.size.hit.target),
            grip_side: cx.tokens.size.handle.md,
            hot: accent,
        }
    }

    /// The annotations' parts, then each wire as a polyline part at `Edge` priority: a press on a port beats the
    /// wire, which sits under the nodes. A wire in `wire_selection` carries `Flags::SELECTED`, so it resolves to its
    /// on look.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.layers().parts(self.zoom, rect.min, cx);
        let selected = self.wire_selection.as_ref().and_then(Bind::value);
        for wire in self.wires {
            let Some(points) = self.wire_curve(rect.min, wire) else { continue };
            let flags = if selected.is_some_and(|wires| wires.contains(&wire.id)) { Flags::SELECTED } else { Flags::empty() };

            cx.part(WIRE.at(wire.id), HitShape::polyline(points, false))
                .sense(Sense::CLICK | Sense::DRAG | Sense::CONTEXT_CLICK)
                .priority(HitPriority::new(HitLevel::Edge, 0))
                .flags(flags)
                .pointer_only("the graph's wire commands act on the selected wire: Insert reroute dot, Swap, Delete links");
        }
    }

    /// Reads each wire's press: it selects wires, and a double click, a secondary press and a primary press past the
    /// slop report `WireEvent`s. A plain primary press on a node or on empty space (the collection's own press)
    /// clears the wires. `Down` and `DragStart` come once per press, in the pass that applies that input, so a
    /// second pass in the frame selects and reports nothing again. Then the annotation layers read the pointer.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let origin = cx.rect.min;
        for wire in self.wires {
            let Some(press) = cx.part_if_declared(WIRE.at(wire.id)).map(|result| result.press) else { continue };
            match press.phase {
                PressPhase::Down { start, .. } => {
                    if press.double_pressed() {
                        let at = self.to_document(origin, start.origin);
                        self.wire_events.push(WireEvent::DoubleClicked { wire: wire.id, at });
                    }
                    if start.button == PointerButton::Secondary {
                        self.wire_events.push(WireEvent::ContextMenu { wire: wire.id, at: start.origin });
                    }
                    if matches!(start.button, PointerButton::Primary | PointerButton::Secondary) {
                        self.press_wire(cx, wire.id, press.mods, start.button == PointerButton::Secondary);
                    }
                }
                PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                    let at = self.to_document(origin, pos);
                    self.wire_events.push(WireEvent::Pulled { wire: wire.id, at });
                }
                _ => {}
            }
            if let Some(at) = press.context {
                self.wire_events.push(WireEvent::ContextMenu { wire: wire.id, at });
            }
        }
        let own = *cx.press();
        if let PressPhase::Down { start, .. } = own.phase
            && start.button == PointerButton::Primary
            && !(own.mods.shift || own.mods.cmd)
            && let Some(bind) = self.wire_selection.as_mut()
        {
            edit_selection(bind.reborrow(), cx, |wires: &mut Selection<u64>| *wires = Selection::default());
        }
        let layers = self.layers();
        self.annotation_events = layers.update(self.zoom, origin, cx, self.annotation_selection.as_mut(), self.selection.as_mut());
    }

    /// The backdrops, then the wires, under the nodes. A selected wire is its on look and a hovered one its hover
    /// look, both thick; a wire that only touches a selected node is `hot`; the wire a drag detaches is not drawn.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.layers().paint_under(self.zoom, cx.rect.min, ui, cx);
        let detached = cx.dragging::<WireDrag>().and_then(|(drag, _)| drag.detached);
        let selected = self.wire_selection.as_ref().and_then(Bind::value);
        let nodes = self.selection.as_ref().and_then(Bind::value);
        let draw = ui.draw();
        for wire in self.wires.iter().filter(|wire| Some(wire.id) != detached) {
            let Some(points) = self.wire_curve(cx.rect.min, wire) else { continue };
            let picked = selected.is_some_and(|wires| wires.contains(&wire.id));
            let hovered = cx.part(WIRE.at(wire.id)).hovered;
            let hot = !picked && !hovered && nodes.is_some_and(|nodes| nodes.contains(&wire.from.node) || nodes.contains(&wire.to.node));
            let width = if picked || hovered { StrokeWidth::Thick } else { StrokeWidth::Medium };
            let color = if hot { cx.style.hot } else { cx.resolve_part(WIRE.at(wire.id), &cx.style.wire) };
            draw.stroke(&PaintShape::Path(points), width, color);
        }
    }

    /// The notes and markers over the nodes, then the loose wire from the port a drag started on to the pointer. Its
    /// free end enters by the side facing the start port's.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.layers().paint_over(self.zoom, cx.rect.min, ui, cx);
        let Some((wire, at)) = cx.dragging::<WireDrag>() else { return };
        if let (Some(from), Some(side)) = (self.port_point(cx.rect.min, wire.from), self.port_side(wire.from)) {
            let points = sampled(wire_controls(from, side, at, opposite(side)));
            ui.draw().stroke(&PaintShape::Path(points), StrokeWidth::Medium, cx.style.loose);
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> NodeGraphResponse<E> {
        NodeGraphResponse { collection: cx.collection_response(), wires: self.wire_events, annotations: self.annotation_events }
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

    /// Hands each node the wires that feed its inputs, by id and source port, and when its ports' names show. The
    /// node owns what it is given, since the graph's `wires` cannot be lent for the item's `'r`.
    fn dress<'r>(&self, node: Self::Item<'r>, _: &RowState) -> Self::Item<'r> {
        let id = node.id();
        let place = self.positions.get(id.0 as usize).map_or(Pos2::ZERO, |rect| rect.min);
        let mut fed = vec![None; self.kinds.get(id.0 as usize).map_or(0, |kind| kind.port_count())];
        for wire in self.wires.iter().filter(|wire| wire.to.node == id) {
            if let Some(slot) = fed.get_mut(wire.to.port) {
                *slot = Some((wire.id, wire.from));
            }
        }
        node.in_graph(fed, self.port_labels).placed(place)
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
