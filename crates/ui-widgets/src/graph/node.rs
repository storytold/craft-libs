//! `Node`: the one item type of a `NodeGraph`.

use super::kind::{NodeEvent, NodeId, NodeKind, PortRef, WireDrag};
use crate::contract::{
    Access, Common, Component, Cx, DragPreview, DropMark, DropOperation, DropTarget, ItemOutput, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx,
    Pointer as _, Props as _, Role, Styled as _,
};
use crate::primitives::{PaintShape, StrokeWidth, TextRun};
use crate::style::{ComponentStyle, Flags, HandleLook, Stateful, StyleColor, StyleCx, SurfaceKind, SurfaceLook, TypeRole};
use craft_behavior::{CursorIcon, HitLevel, HitPriority, HitShape, Insets, Metrics, PartKey, PressPhase, Sense};
use egui::{Pos2, Rect, Response, Vec2};

pub(super) const PORT: PartKey = PartKey::new("port");

/// The style of a node: a raised card that takes the accent edge when selected.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct NodeStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(surface)]
    surface: SurfaceLook,
    role: TypeRole,
    handle: HandleLook,
    /// How far from a port's centre a press or drop still reaches it.
    reach: f32,
    pad: f32,
    title_height: f32,
}

/// The output of a node: its response and what its kind reported.
#[derive(Debug)]
#[non_exhaustive]
pub struct NodeOutput<E = ()> {
    /// The node's response.
    pub response: Response,
    /// The event of this pass.
    pub event: Option<NodeEvent<E>>,
}

impl<E> Outcome for NodeOutput<E> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<E> ItemOutput<NodeEvent<E>> for NodeOutput<E> {
    fn into_event(self) -> Option<NodeEvent<E>> {
        self.event
    }
}

/// A node of a kind: a title, ports, the kind's inline controls and its body paint.
#[derive(crate::contract::Component)]
#[component(probe(Node<'static, ()>))]
pub struct Node<'r, E: 'static = ()> {
    #[common]
    common: Common,
    id: NodeId,
    kind: &'r dyn NodeKind<E>,
    event: Option<NodeEvent<E>>,
}

impl<'r, E: 'static> Node<'r, E> {
    /// The node `id` of `kind`; its title names it.
    pub fn new(id: NodeId, kind: &'r dyn NodeKind<E>) -> Self {
        Node { common: Common::new(), id, kind, event: None }.label(kind.title().to_owned())
    }
}

/// Where port `port` of a node at `rect` sits.
pub(super) fn port_center<E: 'static>(kind: &dyn NodeKind<E>, rect: Rect, port: usize) -> Pos2 {
    rect.min + kind.port_offset(port, rect.size())
}

/// What a wire dropped on a node connects.
struct WireDrop {
    from: PortRef,
    to: PortRef,
    op: DropOperation,
}

/// A node accepts a wire only where the pointer is over one of its ports.
struct PortsTarget {
    node: NodeId,
    ports: Vec<(usize, Pos2)>,
    reach: f32,
}

impl DropTarget<WireDrag> for PortsTarget {
    type Drop = WireDrop;

    fn accept(&self, wire: &WireDrag, at: Pos2, op: DropOperation) -> Option<WireDrop> {
        let (port, _) = self.ports.iter().find(|(_, center)| center.distance(at) <= self.reach)?;
        (wire.from.node != self.node).then_some(WireDrop { from: wire.from, to: PortRef::new(self.node, *port), op })
    }

    fn mark(&self, drop: &WireDrop) -> DropMark {
        let center = self.ports.iter().find(|(port, _)| *port == drop.to.port).map_or(Pos2::ZERO, |(_, center)| *center);
        DropMark::Into(Rect::from_center_size(center, Vec2::splat(2.0 * self.reach)))
    }
}

impl<E: 'static> Component for Node<'_, E> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    type Style = NodeStyle;
    type Output = NodeOutput<E>;

    /// A port is a circle centred on the node's edge: half its side and half its edge stroke stick out.
    fn paint_outset(tokens: &crate::style::UiTokens) -> f32 {
        (tokens.size.handle.md + tokens.stroke.thin) / 2.0
    }

    fn style(cx: &StyleCx<'_>, _: NodeStyleKey<E>) -> NodeStyle {
        let control = cx.control();
        let color = &cx.tokens.color;
        let raised = cx.surface(SurfaceKind::Raised);
        let edge = Stateful::new(cx.color(color.line.border)).when(Flags::SELECTED, cx.color(color.accent.solid));
        let surface = SurfaceLook::new(Stateful::new(StyleColor(raised.fill)))
            .kind(SurfaceKind::Raised)
            .radius(raised.radius)
            .border(edge, cx.tokens.stroke.medium);
        let handle = cx.handle();
        NodeStyle {
            metrics: Metrics::new().padding(Insets::uniform(control.pad_x)).min_size(Vec2::splat(control.height)).gap(control.gap),
            surface,
            role: TypeRole::Label,
            reach: handle.side.max(cx.tokens.size.hit.target) / 2.0,
            handle,
            pad: control.pad_x,
            title_height: control.height,
        }
    }

    /// The title's width or the controls', and the title row's height above the controls.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let title = cx.layout(&TextRun::new(self.kind.title()).role(cx.style.role)).galley.size();
        let controls = self.kind.measure_controls(cx);
        Vec2::new(title.x.max(controls.x), cx.style.title_height + controls.y)
    }

    /// In a free layout an item's root shape senses `DRAG`: a press selects, and a drag of a selected item moves
    /// the selection.
    fn sense(&self) -> Sense {
        Sense::PRESS | Sense::FOCUS
    }

    /// The ports: drag parts at handle priority, above the node's body.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let reach = cx.style.reach;
        for port in 0..self.kind.port_count() {
            cx.part(PORT.at(port as u64), HitShape::circle(port_center(self.kind, rect, port), reach))
                .sense(Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Handle, 2))
                .cursor(CursorIcon::Crosshair)
                .pointer_only("the Connect... command");
        }
    }

    /// A press on a port that moves past the slop starts a wire drag; a wire dropped on a port of this node is
    /// reported as `Wired`, and the end of a wire that started here as `WireEnded`.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let ports = self.kind.port_count();
        for port in 0..ports {
            if matches!(cx.part(PORT.at(port as u64)).press.phase, PressPhase::DragStart { .. }) {
                cx.start_drag(WireDrag { from: PortRef::new(self.id, port) }, DragPreview::None);
            }
        }
        let centers = (0..ports).map(|port| (port, port_center(self.kind, cx.rect, port))).collect();
        let target = PortsTarget { node: self.id, ports: centers, reach: cx.style.reach };
        if let Some(WireDrop { from, to, op }) = cx.drop_target::<WireDrag, _>(&target) {
            self.event = Some(NodeEvent::Wired { from, to, op });
        }
        if let Some(op) = cx.drag_ended::<WireDrag>() {
            self.event = Some(NodeEvent::WireEnded { op });
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (rect, style) = (cx.rect, cx.style);
        let inner = rect.shrink(style.pad);
        let title = Rect::from_min_size(inner.min, Vec2::new(inner.width(), style.title_height));
        {
            let draw = ui.draw();
            draw.text(title, &TextRun::new(self.kind.title()).role(style.role));
            ui.with_style(cx.resolved.variant, |scx| self.kind.paint_body(&draw, rect, scx));
            for port in 0..self.kind.port_count() {
                let circle = PaintShape::Circle(port_center(self.kind, rect, port), style.handle.side / 2.0);
                draw.fill(&circle, cx.resolve_part(PORT.at(port as u64), &style.handle.fill));
                draw.stroke(&circle, StrokeWidth::Thin, style.handle.stroke);
            }
        }
        let below = Rect::from_min_max(Pos2::new(inner.left(), title.bottom()), inner.max);
        if let Some(edit) = self.kind.controls(ui, below) {
            self.event = Some(NodeEvent::Edit(edit));
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        for port in 0..self.kind.port_count() {
            access.node(PORT.at(port as u64), Role::BUTTON).name = Name::Text(self.kind.port_name(port));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> NodeOutput<E> {
        NodeOutput { response: cx.response.clone(), event: self.event }
    }
}
