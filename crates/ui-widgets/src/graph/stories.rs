//! Stories of the graph family.

use super::{
    Node, NodeEvent, NodeFooter, NodeGraph, NodeGraphResponse, NodeHeader, NodeId, NodeKind, NodeShape, PortDirection, PortLabels, PortRef, Wire,
};
use crate::contract::{Bind, CollectionEvent, MeasureCx, PaintUi, Props as _, Role, Show as _, ShowCollection as _};
use crate::controls::disclosure::Disclosure;
use crate::primitives::IconSpec;
use crate::style::{DataColor, StyleCx};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{CanvasTransform, EditEvents, IconId, Selection, keyed};
use egui::{Color32, Pos2, Rect, Ui, Vec2};

/// How far the zoomed story magnifies the graph.
const ZOOM: f32 = 2.0;
/// How far the zoomed story pans the canvas up, in layer points, so the second row's titles lie inside the view with room to grow.
const PAN_UP: f32 = 48.0;

const NODE_WIDTH: f32 = 140.0;
const NODE_HEIGHT: f32 = 72.0;
const GAP: f32 = 24.0;
const PER_ROW: u32 = 2;

/// The edit type of the stories' graph: what the inline controls of the kinds report.
#[derive(Clone, Debug, PartialEq)]
enum CompEdit {
    Fold(EditEvents<bool>),
}

/// Two inputs on the left and an output on the right.
struct Mix;

const MIX: Mix = Mix;

impl<E: 'static> NodeKind<E> for Mix {
    fn title(&self) -> &str {
        "Mix"
    }

    fn port_count(&self) -> usize {
        3
    }

    fn port_name(&self, port: usize) -> String {
        ["Input A", "Input B", "Output"][port % 3].to_owned()
    }

    fn port_offset(&self, port: usize, size: Vec2) -> Vec2 {
        if port == 2 { Vec2::new(size.x, size.y / 2.0) } else { Vec2::new(0.0, size.y * (port as f32 + 1.0) / 3.0) }
    }

    fn port_direction(&self, port: usize) -> PortDirection {
        if port == 2 { PortDirection::Output } else { PortDirection::Any }
    }
}

/// An input, an output and a disclosure control inside the node.
struct Fold {
    open: bool,
}

impl NodeKind<CompEdit> for Fold {
    fn title(&self) -> &str {
        "Fold"
    }

    fn port_count(&self) -> usize {
        2
    }

    fn port_name(&self, port: usize) -> String {
        let name = if port == 0 { "Input" } else { "Output" };
        name.to_owned()
    }

    fn port_offset(&self, port: usize, size: Vec2) -> Vec2 {
        Vec2::new(if port == 0 { 0.0 } else { size.x }, size.y / 2.0)
    }

    fn port_direction(&self, port: usize) -> PortDirection {
        if port == 1 { PortDirection::Output } else { PortDirection::Any }
    }

    fn measure_controls(&self, cx: &MeasureCx<'_, Node<'_, CompEdit>>) -> Vec2 {
        Vec2::splat(cx.icon())
    }

    fn controls(&self, ui: &mut PaintUi<'_>, rect: Rect) -> Option<CompEdit> {
        let side = Rect::from_min_size(rect.min, Vec2::splat(rect.height()));
        let events = ui.child("fold", side, Disclosure::new(Bind::given(self.open)).label("Details")).events;
        (!events.is_empty()).then_some(CompEdit::Fold(events))
    }
}

fn place(at: u32) -> Rect {
    let (column, row) = ((at % PER_ROW) as f32, (at / PER_ROW) as f32);
    let (left, top) = (GAP + column * (NODE_WIDTH + GAP), GAP + row * (NODE_HEIGHT + GAP));
    Rect::from_min_max(Pos2::new(left, top), Pos2::new(left + NODE_WIDTH, top + NODE_HEIGHT))
}

/// A graph of `count` nodes, alternating the two kinds, joined by `wired`: each pair is the output of the first
/// node to the input of the second.
fn graph(ui: &mut Ui, data: &mut StoryData, count: u32, wired: &[(u32, u32)], zoom: Option<CanvasTransform>) -> StoryOutput {
    let fold = Fold { open: *data.value("open", true) };
    let kinds: Vec<&dyn NodeKind<CompEdit>> = (0..count).map(|at| if at % 2 == 0 { &MIX as &dyn NodeKind<CompEdit> } else { &fold }).collect();
    let positions: Vec<Rect> = (0..count).map(place).collect();
    let ids: Vec<NodeId> = (0..count).map(NodeId).collect();
    let wires: Vec<Wire> = wired
        .iter()
        .enumerate()
        .map(|(at, (from, to))| {
            Wire::new(at as u64, PortRef::new(NodeId(*from), 2.min(kinds[*from as usize].port_count() - 1)), PortRef::new(NodeId(*to), 0))
        })
        .collect();
    let source = keyed(&ids, |id| *id);
    let mut shown = NodeGraph::new(&positions, &kinds, &wires).label("Graph");
    if let Some(zoom) = zoom {
        shown = shown.zoom(zoom);
    }
    let response = shown.show_items(ui, &source, &mut |id, _| Node::new(*id, kinds[id.0 as usize]));
    for event in &response.collection.events {
        if let CollectionEvent::Item(_, NodeEvent::Edit(CompEdit::Fold(edits))) = event {
            for edit in edits {
                let _ = edit.apply(data.value("open", true));
            }
        }
    }
    response.into()
}

/// Selects the first node by a click on it: a click at the middle of a graph lands on a node, not on the graph.
fn click_first_node(fixture: &mut Fixture) {
    if let Some(node) = fixture.by_role(Role::GRAPHICS_OBJECT).first().copied() {
        fixture.click(&node);
    }
}

#[story(NodeGraph<'static, ()>, covers = [Empty], drive = Drive::Custom(click_first_node))]
fn no_nodes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    graph(ui, data, 0, &[], None)
}

#[story(NodeGraph<'static, ()>, covers = [One], drive = Drive::Custom(click_first_node))]
fn one_node(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    graph(ui, data, 1, &[], None)
}

#[story(NodeGraph<'static, ()>, covers = [Many, Default], drive = Drive::Custom(click_first_node))]
fn wired_nodes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    graph(ui, data, 4, &[(0, 1), (1, 2), (2, 3)], None)
}

/// Nodes that report no edits: the graph of the component's own edit type.
#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(click_first_node))]
fn plain_nodes(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let kinds: Vec<&dyn NodeKind<()>> = vec![&MIX; 3];
    let positions: Vec<Rect> = (0..3).map(place).collect();
    let ids: Vec<NodeId> = (0..3).map(NodeId).collect();
    let wires =
        [Wire::new(0, PortRef::new(NodeId(0), 2), PortRef::new(NodeId(1), 0)), Wire::new(1, PortRef::new(NodeId(1), 2), PortRef::new(NodeId(2), 0))];
    let source = keyed(&ids, |id| *id);
    let response =
        NodeGraph::new(&positions, &kinds, &wires).label("Graph").show_items(ui, &source, &mut |id, _| Node::new(*id, kinds[id.0 as usize]));
    response.into()
}

#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(click_first_node))]
fn zoomed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    graph(ui, data, 4, &[(0, 1), (1, 2), (2, 3)], Some(CanvasTransform::new(ZOOM, -PAN_UP * Vec2::Y)))
}

/// The tone of the stories' header bands and first port.
const TONE: DataColor = DataColor::new(Color32::from_rgb(0x2e, 0x7d, 0xd1));
/// The tone of the stories' second port.
const TONE_B: DataColor = DataColor::new(Color32::from_rgb(0xd0, 0x8a, 0x20));
/// The tone of the stories' output port.
const TONE_OUT: DataColor = DataColor::new(Color32::from_rgb(0x3a, 0xa0, 0x5a));
/// The side of the dot node's square, in document units.
const DOT_SIDE: f32 = 24.0;

/// A source with a header band, a footer caption and a tooltip, and one output on the right.
struct Banded;

impl NodeKind<()> for Banded {
    fn title(&self) -> &str {
        "Source"
    }

    fn port_count(&self) -> usize {
        1
    }

    fn port_name(&self, _: usize) -> String {
        "Output".to_owned()
    }

    fn port_offset(&self, _: usize, size: Vec2) -> Vec2 {
        Vec2::new(size.x, size.y / 2.0)
    }

    fn port_direction(&self, _: usize) -> PortDirection {
        PortDirection::Output
    }

    fn header(&self, _: &StyleCx<'_>) -> Option<NodeHeader> {
        Some(NodeHeader::new(TONE, 20.0).icon(IconSpec::svg(IconId::new("pin"))))
    }

    fn footer(&self) -> Option<NodeFooter<'_>> {
        Some(NodeFooter::new("48 kHz", 16.0))
    }

    fn tooltip(&self) -> Option<String> {
        Some("Reads the clip's samples".to_owned())
    }
}

/// `Mix` with a data tone on each port's handle.
struct Toned;

impl NodeKind<()> for Toned {
    fn title(&self) -> &str {
        "Mix"
    }

    fn port_count(&self) -> usize {
        NodeKind::<()>::port_count(&MIX)
    }

    fn port_name(&self, port: usize) -> String {
        NodeKind::<()>::port_name(&MIX, port)
    }

    fn port_offset(&self, port: usize, size: Vec2) -> Vec2 {
        NodeKind::<()>::port_offset(&MIX, port, size)
    }

    fn port_direction(&self, port: usize) -> PortDirection {
        NodeKind::<()>::port_direction(&MIX, port)
    }

    fn port_tone(&self, port: usize, _: &StyleCx<'_>) -> Option<DataColor> {
        Some([TONE, TONE_B, TONE_OUT][port % 3])
    }
}

/// A junction drawn as a dot: an input on its left edge and an output on its right.
struct Junction;

impl NodeKind<()> for Junction {
    fn title(&self) -> &str {
        "Junction"
    }

    fn port_count(&self) -> usize {
        2
    }

    fn port_name(&self, port: usize) -> String {
        let name = if port == 0 { "Input" } else { "Output" };
        name.to_owned()
    }

    fn port_offset(&self, port: usize, size: Vec2) -> Vec2 {
        Vec2::new(if port == 0 { 0.0 } else { size.x }, size.y / 2.0)
    }

    fn port_direction(&self, port: usize) -> PortDirection {
        if port == 1 { PortDirection::Output } else { PortDirection::Any }
    }

    fn shape(&self) -> NodeShape {
        NodeShape::Dot
    }
}

/// What a showcase story shows: its kinds at `place`, its wires, and its starting selections, which the story's
/// data keeps.
struct Scene<'k> {
    kinds: &'k [&'k dyn NodeKind<()>],
    wires: &'k [Wire],
    place: fn(u32) -> Rect,
    port_labels: PortLabels,
    node: for<'r> fn(NodeId, Node<'r, ()>) -> Node<'r, ()>,
    nodes: &'static [u32],
    picked: &'static [u64],
}

impl<'k> Scene<'k> {
    /// `kinds` in the stories' grid, joined by `wires`, nothing selected.
    fn new(kinds: &'k [&'k dyn NodeKind<()>], wires: &'k [Wire]) -> Self {
        Scene { kinds, wires, place, port_labels: PortLabels::Hidden, node: |_, node| node, nodes: &[], picked: &[] }
    }

    /// Shows the graph with both selections bound to the story's data.
    fn show(self, ui: &mut Ui, data: &mut StoryData) -> NodeGraphResponse<()> {
        let count = self.kinds.len() as u32;
        let positions: Vec<Rect> = (0..count).map(self.place).collect();
        let ids: Vec<NodeId> = (0..count).map(NodeId).collect();
        let start_nodes: Vec<NodeId> = self.nodes.iter().map(|at| NodeId(*at)).collect();
        let mut nodes = data.value("nodes", Selection::new(start_nodes.clone(), start_nodes.last().copied(), start_nodes.first().copied())).clone();
        let start_wires = self.picked.to_vec();
        let mut picked = data.value("wires", Selection::new(start_wires.clone(), start_wires.last().copied(), start_wires.first().copied())).clone();
        let source = keyed(&ids, |id| *id);
        let (kinds, node) = (self.kinds, self.node);
        let response = NodeGraph::new(&positions, kinds, self.wires)
            .label("Graph")
            .port_labels(self.port_labels)
            .selection(Bind::bound(&mut nodes))
            .wire_selection(Bind::bound(&mut picked))
            .show_items(ui, &source, &mut |id, _| node(*id, Node::new(*id, kinds[id.0 as usize])));
        *data.value("nodes", Selection::default()) = nodes;
        *data.value("wires", Selection::default()) = picked;
        response
    }
}

/// Output port 2 of `from` to input port 0 of `to`, as wire `id`.
fn mix_wire(id: u64, from: u32, to: u32) -> Wire {
    Wire::new(id, PortRef::new(NodeId(from), 2), PortRef::new(NodeId(to), 0))
}

/// The stories' grid, with node 1 a small square for a dot.
fn dot_place(at: u32) -> Rect {
    if at == 1 { Rect::from_center_size(place(at).center(), Vec2::splat(DOT_SIDE)) } else { place(at) }
}

#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(click_first_node))]
fn header_and_footer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 2] = [&Banded, &MIX];
    let wires = [Wire::new(0, PortRef::new(NodeId(0), 0), PortRef::new(NodeId(1), 0))];
    Scene::new(&kinds, &wires).show(ui, data).into()
}

#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(click_first_node))]
fn port_tones(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 2] = [&Toned, &Toned];
    let wires = [mix_wire(0, 0, 1)];
    Scene::new(&kinds, &wires).show(ui, data).into()
}

/// The click on the first node leaves the pointer over it, so its ports' names show.
#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(click_first_node))]
fn port_labels_on_hover(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 2] = [&MIX, &MIX];
    let wires = [mix_wire(0, 0, 1)];
    Scene { port_labels: PortLabels::OnHover, ..Scene::new(&kinds, &wires) }.show(ui, data).into()
}

#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(click_first_node))]
fn failed_node(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 3] = [&MIX, &MIX, &MIX];
    let wires = [mix_wire(0, 0, 1), mix_wire(1, 1, 2)];
    Scene { node: |id, node| node.failed(id == NodeId(1)), ..Scene::new(&kinds, &wires) }.show(ui, data).into()
}

#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(click_first_node))]
fn bypassed_node(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 3] = [&MIX, &MIX, &MIX];
    let wires = [mix_wire(0, 0, 1), mix_wire(1, 1, 2)];
    Scene { node: |id, node| node.bypassed(id == NodeId(1)), ..Scene::new(&kinds, &wires) }.show(ui, data).into()
}

#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(click_first_node))]
fn dot_node(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 3] = [&MIX, &Junction, &MIX];
    let wires = [mix_wire(0, 0, 1), Wire::new(1, PortRef::new(NodeId(1), 1), PortRef::new(NodeId(2), 0))];
    Scene { place: dot_place, ..Scene::new(&kinds, &wires) }.show(ui, data).into()
}

/// Wire 1 is selected and node 3 is, so wire 2, which touches it, is hot; the click on the first node clears the
/// wires.
#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(click_first_node))]
fn selected_wires(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 4] = [&MIX, &MIX, &MIX, &MIX];
    let wires = [mix_wire(0, 0, 1), mix_wire(1, 1, 2), mix_wire(2, 2, 3)];
    Scene { nodes: &[3], picked: &[1], ..Scene::new(&kinds, &wires) }.show(ui, data).into()
}

/// How far a wire drag moves to end on empty canvas: down out of the first node into the gap between the rows.
const TO_EMPTY: Vec2 = Vec2::new(-16.0, 60.0);

/// Drags a wire backwards from the first node's unfed input out to empty canvas.
fn drag_from_input(fixture: &mut Fixture) {
    if let Some(input) = fixture.by_label("Input A") {
        fixture.drag_at(input.center(), TO_EMPTY);
    }
}

#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(drag_from_input))]
fn reverse_wire_drag(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 2] = [&MIX, &MIX];
    let wires = [mix_wire(0, 0, 1)];
    Scene::new(&kinds, &wires).show(ui, data).into()
}

/// Releases a wire from the first node's output on empty canvas, then on the same node's own input, which refuses
/// it: both report where the loose wire was let go.
fn release_loose(fixture: &mut Fixture) {
    let (Some(output), Some(input)) = (fixture.by_label("Output"), fixture.by_label("Input A")) else { return };
    let before = fixture.data().note("loose", Vec::<Pos2>::new()).len();
    fixture.drag_at(output.center(), Vec2::new(0.0, 40.0));
    fixture.drag_at(output.center(), input.center() - output.center());
    let after = fixture.data().note("loose", Vec::<Pos2>::new()).len();
    assert!(after >= before + 2, "a release on empty canvas and one on a refused port each report a loose end");
}

#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(release_loose))]
fn loose_wire_drag(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 2] = [&MIX, &MIX];
    let response = Scene::new(&kinds, &[]).show(ui, data);
    let loose = data.note("loose", Vec::<Pos2>::new());
    for event in &response.collection.events {
        if let CollectionEvent::Item(_, NodeEvent::WireEnded { loose: Some(at), .. }) = event {
            loose.push(*at);
        }
    }
    let readout = match loose.last() {
        Some(at) => format!("Loose ends: {}, the last at {:.0}, {:.0}", loose.len(), at.x, at.y),
        None => "Loose ends: none".to_owned(),
    };
    StoryOutput::from(response).readout(readout)
}

#[story(Node<'static, ()>, covers = [Default])]
fn mix(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Node::<()>::new(NodeId(0), &MIX).show(ui).into()
}
