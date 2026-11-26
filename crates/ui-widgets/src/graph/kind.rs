//! The data types of a node graph and `NodeKind`, the trait apps implement to add kinds of node.

use super::Node;
use crate::contract::{DropOperation, MeasureCx, PaintUi};
use crate::primitives::{Draw, IconSpec};
use crate::style::{DataColor, StyleCx};
use craft_keymap::Mods;
use egui::{Pos2, Rect, Vec2};

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

/// Which way a wire may run through a port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortDirection {
    /// Either way: a wire may start or end here.
    Any,
    /// A wire ends here.
    Input,
    /// A wire starts here.
    Output,
}

/// The side of a node a port sits on; a wire leaves and enters along its side's outward normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortSide {
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
}

/// How a node is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodeShape {
    /// A card with a title.
    #[default]
    Card,
    /// A circle filling the node's rect, with no title and its ports on its edge.
    Dot,
}

/// When the ports' names show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortLabels {
    /// Never.
    #[default]
    Hidden,
    /// Beside each port while the pointer is over the node.
    OnHover,
}

/// A node's header: a data-tone band with a rule under it, the elided title in it and the icon at its end.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NodeHeader {
    /// The band's tone.
    pub tone: DataColor,
    /// The band's height, in document units.
    pub height: f32,
    /// The icon at the band's end.
    pub icon: Option<IconSpec<'static>>,
}

impl NodeHeader {
    /// A header band of `tone`, `height` document units high.
    pub fn new(tone: DataColor, height: f32) -> Self {
        NodeHeader { tone, height, icon: None }
    }

    /// The header shows `icon` at its end.
    #[must_use]
    pub fn icon(self, icon: IconSpec<'static>) -> Self {
        NodeHeader { icon: Some(icon), ..self }
    }
}

/// A node's footer: one centred caption line at the card's bottom.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NodeFooter<'a> {
    /// The caption.
    pub text: &'a str,
    /// The footer's height, in document units.
    pub height: f32,
}

impl<'a> NodeFooter<'a> {
    /// A footer of `text`, `height` document units high.
    pub fn new(text: &'a str, height: f32) -> Self {
        NodeFooter { text, height }
    }
}

/// What a drag that starts on a port carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WireDrag {
    /// The port the drag started on: the source port of the wire being detached, else the port itself.
    pub from: PortRef,
    /// The id of the wire the drag took off a fed input, if it did.
    pub detached: Option<u64>,
    /// The direction of the port the drag started on.
    pub from_direction: PortDirection,
}

/// What a node reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NodeEvent<E = ()> {
    /// A wire was dropped on a port of this node.
    #[non_exhaustive]
    Wired {
        /// The output end: the port the wire started on, or the port a wire from an input was dropped on.
        from: PortRef,
        /// The input end: the port the wire was dropped on, or the input a reverse wire started on.
        to: PortRef,
        /// The drop operation: Alt copies.
        op: DropOperation,
        /// The id of the wire the drag took off an input, if it did.
        detached: Option<u64>,
    },
    /// A wire that started on a port of this node was released, in the pass after the drop.
    #[non_exhaustive]
    WireEnded {
        /// What the drop did, as the target answered it; `None` when no port took it.
        op: Option<DropOperation>,
        /// What the drag carried as its start: the source port of a detached wire, else the port pressed.
        from: PortRef,
        /// The id of the wire the drag took off an input, if it did.
        detached: Option<u64>,
        /// Where the wire was released, in document units, when no port took it.
        loose: Option<Pos2>,
        /// The port under a release no port took, when the topmost node there refused the wire on it: a port of the
        /// wire's own node, or one whose direction does not pair with the wire's start.
        refused: Option<PortRef>,
    },
    /// A primary press and release with no drag, on the node's body or a port.
    #[non_exhaustive]
    Clicked {
        /// The port clicked, or `None` for the body.
        port: Option<usize>,
        /// The modifiers held.
        mods: Mods,
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

    /// Which way a wire may run through port `port`; any way by default.
    fn port_direction(&self, _port: usize) -> PortDirection {
        PortDirection::Any
    }

    /// The side of the node port `port` sits on: the right for an output, the left otherwise.
    fn port_side(&self, port: usize) -> PortSide {
        if self.port_direction(port) == PortDirection::Output { PortSide::Right } else { PortSide::Left }
    }

    /// Whether input port `port` takes more than one wire; no by default.
    fn port_takes_many(&self, _port: usize) -> bool {
        false
    }

    /// The tone of port `port`'s handle; `None` keeps the handle look.
    fn port_tone(&self, _port: usize, _cx: &StyleCx<'_>) -> Option<DataColor> {
        None
    }

    /// The node's header band; none by default.
    fn header(&self, _cx: &StyleCx<'_>) -> Option<NodeHeader> {
        None
    }

    /// The node's footer caption; none by default.
    fn footer(&self) -> Option<NodeFooter<'_>> {
        None
    }

    /// How the node is drawn: a card by default.
    fn shape(&self) -> NodeShape {
        NodeShape::Card
    }

    /// The node's tooltip; none by default.
    fn tooltip(&self) -> Option<String> {
        None
    }
}
