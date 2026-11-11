//! Stories of the graph family.

use super::{Node, NodeEvent, NodeGraph, NodeId, NodeKind, PortRef, Wire};
use crate::contract::{Bind, CollectionEvent, MeasureCx, PaintUi, Props as _, Role, Show as _, ShowCollection as _};
use crate::controls::disclosure::Disclosure;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{CanvasTransform, EditEvents, keyed};
use egui::{Pos2, Rect, Ui, Vec2};

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

static MIX: Mix = Mix;

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
    for event in &response.events {
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

#[story(Node<'static, ()>, covers = [Default])]
fn mix(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Node::<()>::new(NodeId(0), &MIX).show(ui).into()
}
