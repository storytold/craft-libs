//! The data types of a node graph and `NodeKind`, the trait apps implement to add kinds of node.

use super::Node;
use crate::contract::{DropOperation, MeasureCx, PaintUi};
use crate::primitives::Draw;
use crate::style::StyleCx;
use egui::{Rect, Vec2};

/// The key of a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct NodeId(pub u32);

impl NodeId {
    /// The node numbered `id`.
    pub const fn new(id: u32) -> Self {
        NodeId(id)
    }
}

/// A port: the node and the port's index on its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PortRef {
    /// The node.
    pub node: NodeId,
    /// The port's index.
    pub port: usize,
}

impl PortRef {
    /// Port `port` of `node`.
    pub const fn new(node: NodeId, port: usize) -> Self {
        PortRef { node, port }
    }
}

/// A wire between two ports, with the stable id its part is keyed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Wire {
    /// The wire's id: a wire keeps its press and hover when another is deleted.
    pub id: u64,
    /// Where it starts.
    pub from: PortRef,
    /// Where it ends.
    pub to: PortRef,
}

impl Wire {
    /// The wire `id` from `from` to `to`.
    pub const fn new(id: u64, from: PortRef, to: PortRef) -> Self {
        Wire { id, from, to }
    }
}

/// What a drag that starts on a port carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WireDrag {
    /// The port the drag started on.
    pub from: PortRef,
}

/// What a node reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NodeEvent<E = ()> {
    /// A wire was dropped on a port of this node.
    #[non_exhaustive]
    Wired {
        /// The port the wire started on.
        from: PortRef,
        /// The port it was dropped on.
        to: PortRef,
        /// The drop operation: Alt copies.
        op: DropOperation,
    },
    /// A wire that started on a port of this node was released, in the pass after the drop.
    #[non_exhaustive]
    WireEnded {
        /// What the drop did, as the target answered it.
        op: DropOperation,
    },
    /// An edit the node's kind reported from its inline controls.
    Edit(E),
}

/// What an app knows about one kind of node. Every method has a default, so a kind implements only what it
/// has; kinds are added without new item types. `E` is the app's edit type for the kinds' inline controls.
pub trait NodeKind<E: 'static = ()> {
    /// The node's title.
    fn title(&self) -> &str {
        ""
    }

    /// The number of ports.
    fn port_count(&self) -> usize {
        0
    }

    /// The accessible name of port `port`.
    fn port_name(&self, _port: usize) -> String {
        String::new()
    }

    /// Where port `port` sits, from the node's top left, a pure function of the kind and the node's size. `Node`
    /// and `NodeGraph` (its wires) call this one function, so wires read no item part. The default spaces the
    /// ports evenly down the left edge.
    fn port_offset(&self, port: usize, node_size: Vec2) -> Vec2 {
        Vec2::new(0.0, node_size.y * (port as f32 + 1.0) / (self.port_count() as f32 + 1.0))
    }

    /// The size of the kind's inline controls; `Vec2::ZERO` when it has none.
    fn measure_controls(&self, _cx: &MeasureCx<'_, Node<'_, E>>) -> Vec2 {
        Vec2::ZERO
    }

    /// Shows the kind's inline controls in `rect` with `ui.child` or `ui.labelled` over values the kind borrows
    /// (`Bind::given`), and returns the edit they made this pass.
    fn controls(&self, _ui: &mut PaintUi<'_>, _rect: Rect) -> Option<E> {
        None
    }

    /// Paints the kind's body in `rect`. Colours come from `cx`, through a token or an app token group.
    fn paint_body(&self, _draw: &Draw<'_>, _rect: Rect, _cx: &StyleCx<'_>) {}
}
