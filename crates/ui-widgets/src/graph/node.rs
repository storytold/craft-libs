//! `Node`: the one item type of a `NodeGraph`.

use super::kind::{NodeEvent, NodeId, NodeKind, NodeShape, PortDirection, PortLabels, PortRef, PortSide, WireDrag};
use crate::contract::{
    Access, Common, Component, Cx, DragPreview, DropMark, DropOperation, DropTarget, ItemOutput, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx,
    Pointer as _, Props as _, Role, Styled as _,
};
use crate::primitives::{Draw, HAlign, PaintShape, StrokeWidth, TextRun};
use crate::style::{ComponentStyle, Corners, Flags, HandleLook, RuleKind, Stateful, StyleColor, StyleCx, SurfaceKind, SurfaceLook, TypeRole};
use craft_behavior::{CursorIcon, HitLevel, HitPriority, HitShape, Insets, Metrics, PartKey, PointerButton, PressPhase, PressState, Sense};
use craft_theme::Rgba;
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

pub(super) const PORT: PartKey = PartKey::new("port");

/// A Dot's circle: it takes the presses a Card's root takes, so the rect's corners outside the circle take none.
const BODY: PartKey = PartKey::new("body");

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
    /// The edge at rest and hovered, thin: the Card's surface border and the Dot's circle.
    edge: Stateful<StyleColor>,
    /// The selected edge, one weight wider than `edge`: clear at rest, so it is painted always.
    selected_edge: Stateful<StyleColor>,
    /// How far a port's name stands from the port.
    label_gap: f32,
    /// The plate's margin around a port's name.
    label_pad: f32,
    /// A Dot's fill: the raised surface's.
    dot_fill: StyleColor,
    /// A failed node's edge and a bypassed node's strike.
    failed: StyleColor,
    /// The width of a bypassed node's strike.
    strike: StrokeWidth,
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
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(probe(Node<'static, ()>))]
pub struct Node<'r, E: 'static = ()> {
    #[common]
    common: Common,
    id: NodeId,
    kind: &'r dyn NodeKind<E>,
    event: Option<NodeEvent<E>>,
    /// The card's edge takes the bad status colour, one weight wider than the selected edge, and wins over it.
    #[prop]
    failed: bool,
    /// A strike in the bad status colour, top-left to bottom-right; the node stays interactive.
    #[prop]
    bypassed: bool,
    /// The kind's shape is a Dot: its surface is the circle `paint` draws, not the card's.
    #[style_key]
    dot: bool,
    /// For each input port, the id and source port of the wire that ends there.
    fed: Vec<Option<(u64, PortRef)>>,
    /// When the ports' names show.
    port_labels: PortLabels,
    /// The node's top left in document units, which maps a release in the node's layer to the document.
    place: Pos2,
}

impl<'r, E: 'static> Node<'r, E> {
    /// The node `id` of `kind`; its title names it, and the kind's tooltip shows over it.
    pub fn new(id: NodeId, kind: &'r dyn NodeKind<E>) -> Self {
        let node = Node {
            common: Common::new(),
            id,
            kind,
            event: None,
            failed: false,
            bypassed: false,
            dot: kind.shape() == NodeShape::Dot,
            fed: Vec::new(),
            port_labels: PortLabels::default(),
            place: Pos2::ZERO,
        }
        .label(kind.title().to_owned());
        match kind.tooltip() {
            Some(tooltip) => node.tooltip(tooltip),
            None => node,
        }
    }

    /// The node as a member of a graph: `fed[port]` is the id and source port of the wire that ends at that input
    /// port, and `port_labels` says when the ports' names show.
    pub(super) fn in_graph(self, fed: Vec<Option<(u64, PortRef)>>, port_labels: PortLabels) -> Self {
        Node { fed, port_labels, ..self }
    }

    /// The node's top left in document units, from the graph's `positions`: `WireEnded::loose` is the release
    /// mapped through it.
    pub(super) fn placed(self, place: Pos2) -> Self {
        Node { place, ..self }
    }

    /// The node's key.
    pub(super) fn id(&self) -> NodeId {
        self.id
    }

    /// What a drag from `port` carries (the detach rule): a fed input that takes one wire gives that wire up, so the
    /// drag runs from the wire's source with it detached; any other input starts a reverse wire; an output or a
    /// port of either way starts a wire from itself.
    fn wire_from(&self, port: usize) -> WireDrag {
        let here = PortRef::new(self.id, port);
        match self.kind.port_direction(port) {
            PortDirection::Input => match self.fed.get(port).copied().flatten() {
                Some((wire, source)) if !self.kind.port_takes_many(port) => {
                    WireDrag { from: source, detached: Some(wire), from_direction: PortDirection::Output }
                }
                _ => WireDrag { from: here, detached: None, from_direction: PortDirection::Input },
            },
            direction => WireDrag { from: here, detached: None, from_direction: direction },
        }
    }
}

/// Where port `port` of a node at `rect` sits.
pub(super) fn port_center<E: 'static>(kind: &dyn NodeKind<E>, rect: Rect, port: usize) -> Pos2 {
    rect.min + kind.port_offset(port, rect.size())
}

/// A primary press released within slop: the pass's click, read once from the pass that receives the release.
fn primary_click(press: &PressState) -> bool {
    matches!(press.phase, PressPhase::Release { click: true, start, .. } if start.button == PointerButton::Primary)
}

/// What a wire dropped on a node connects.
struct WireDrop {
    from: PortRef,
    to: PortRef,
    /// The port of this node the wire was dropped on.
    port: usize,
    op: DropOperation,
    detached: Option<u64>,
}

/// A node accepts a wire only where the pointer is over one of its ports, from another node, on a port whose
/// direction pairs with the wire's start.
struct PortsTarget {
    node: NodeId,
    ports: Vec<(usize, Pos2, PortDirection)>,
    reach: f32,
}

impl DropTarget<WireDrag> for PortsTarget {
    type Drop = WireDrop;

    fn accept(&self, wire: &WireDrag, at: Pos2, op: DropOperation) -> Option<WireDrop> {
        if wire.from.node == self.node {
            return None;
        }
        let &(port, _, direction) = self.ports.iter().find(|(_, center, _)| center.distance(at) <= self.reach)?;
        if direction == wire.from_direction && direction != PortDirection::Any {
            return None;
        }
        let here = PortRef::new(self.node, port);
        // A reverse wire started on an input, so the port it lands on is the output end.
        let (from, to) = if wire.from_direction == PortDirection::Input { (here, wire.from) } else { (wire.from, here) };
        Some(WireDrop { from, to, port, op, detached: wire.detached })
    }

    fn mark(&self, drop: &WireDrop) -> DropMark {
        let center = self.ports.iter().find(|(port, _, _)| *port == drop.port).map_or(Pos2::ZERO, |(_, center, _)| *center);
        DropMark::Into(Rect::from_center_size(center, Vec2::splat(2.0 * self.reach)))
    }
}

impl<E: 'static> Node<'_, E> {
    /// Paints the name of each port beside it, inward, on a popup plate in the caption role.
    // PLAN: SG-5 puts the names outward from the ports, but an item's paint is clipped to its rect grown by
    // `paint_outset` (`contract/run/typed.rs:929`); outward names need `NodeGraph::paint_over` (graph-w4).
    fn paint_port_labels(&self, draw: &Draw<'_>, rect: Rect, style: &NodeStyle) {
        let gap = style.label_gap;
        for port in 0..self.kind.port_count() {
            let name = self.kind.port_name(port);
            let run = TextRun::new(name.as_str()).role(TypeRole::Caption).elide();
            let side = self.kind.port_side(port);
            let room = match side {
                PortSide::Left | PortSide::Right => rect.width() / 2.0 - gap,
                PortSide::Top | PortSide::Bottom => rect.width(),
            };
            let layout = draw.layout(&run, room.max(0.0));
            let size = layout.size();
            let center = port_center(self.kind, rect, port);
            let min = match side {
                PortSide::Left => Pos2::new(center.x + gap, center.y - size.y / 2.0),
                PortSide::Right => Pos2::new(center.x - gap - size.x, center.y - size.y / 2.0),
                PortSide::Top => Pos2::new(center.x - size.x / 2.0, center.y + gap),
                PortSide::Bottom => Pos2::new(center.x - size.x / 2.0, center.y - gap - size.y),
            };
            draw.surface(Rect::from_min_size(min, size).expand(style.label_pad), SurfaceKind::Popup, Corners::All);
            draw.galley(min, &layout);
        }
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

    fn style(cx: &StyleCx<'_>, key: NodeStyleKey<E>) -> NodeStyle {
        let control = cx.control();
        let color = &cx.tokens.color;
        let raised = cx.surface(SurfaceKind::Raised);
        let edge = Stateful::new(cx.color(color.line.border)).when(Flags::SELECTED, cx.color(color.accent.solid));
        let surface = if key.dot {
            SurfaceLook::new(Stateful::new(cx.color(Rgba::TRANSPARENT)))
        } else {
            SurfaceLook::new(Stateful::new(StyleColor(raised.fill)))
                .kind(SurfaceKind::Raised)
                .radius(raised.radius)
                .border(edge.clone(), cx.tokens.stroke.thin)
        };
        let handle = cx.handle();
        let selected_edge = Stateful::new(cx.color(Rgba::TRANSPARENT)).when(Flags::SELECTED, cx.color(color.accent.solid));
        NodeStyle {
            label_gap: handle.side / 2.0 + control.pad_x,
            label_pad: control.pad_x / 2.0,
            selected_edge,
            metrics: Metrics::new().padding(Insets::uniform(control.pad_x)).min_size(Vec2::splat(control.height)).gap(control.gap),
            surface,
            role: TypeRole::Label,
            reach: handle.side.max(cx.tokens.size.hit.target) / 2.0,
            handle,
            pad: control.pad_x,
            title_height: control.height,
            edge,
            dot_fill: StyleColor(raised.fill),
            failed: cx.color(color.status.bad.solid),
            strike: StrokeWidth::Medium,
        }
    }

    /// The title's width or the controls', and the title row's height above the controls.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let title = cx.layout(&TextRun::new(self.kind.title()).role(cx.style.role)).galley.size();
        let controls = self.kind.measure_controls(cx);
        Vec2::new(title.x.max(controls.x), cx.style.title_height + controls.y)
    }

    /// In a free layout an item's root shape senses `DRAG`: a press selects, and a drag of a selected item moves
    /// the selection. A Dot's root only takes focus: its circle is the `BODY` part.
    fn sense(&self) -> Sense {
        if self.dot { Sense::FOCUS } else { Sense::PRESS | Sense::FOCUS }
    }

    /// The ports: press parts at handle priority, above the node's body. A Dot's body is a circle part that passes
    /// its drag to the graph, so the graph's free move moves it; the hit resolve grows the circle to the pointer's
    /// target size. A Dot's ports reach around its circle but not into it, so a press inside the circle is the
    /// body's and one just outside it, in the grown grab area, is the port's.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let body = (rect.center(), rect.width() / 2.0);
        if self.dot {
            cx.part(BODY, HitShape::circle(body.0, body.1))
                .sense(Sense::CLICK | Sense::PASSES_DRAG)
                .drag_to_root()
                .priority(HitPriority::BODY)
                .pointer_only("the node's own focus stop");
        }
        let reach = cx.style.reach;
        for port in 0..self.kind.port_count() {
            let circle = HitShape::circle(port_center(self.kind, rect, port), reach);
            let shape = if self.dot { HitShape::excluding(circle, vec![body]) } else { circle };
            cx.part(PORT.at(port as u64), shape)
                .sense(Sense::PRESS)
                .priority(HitPriority::new(HitLevel::Handle, 2))
                .cursor(CursorIcon::Crosshair)
                .pointer_only("the Connect... command");
        }
    }

    /// A press on a port that moves past the slop starts a wire drag; a primary click on a port or the body is
    /// `Clicked`; a wire dropped on a port of this node is reported as `Wired`, and the end of a wire that started
    /// here as `WireEnded`. Press phases are the pass's own: a second pass of the frame reports no `DragStart` or
    /// `Release` the first took, so none of these repeats.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let ports = self.kind.port_count();
        for port in 0..ports {
            let press = cx.part(PORT.at(port as u64)).press;
            if matches!(press.phase, PressPhase::DragStart { .. }) {
                cx.start_drag(self.wire_from(port), DragPreview::None);
            } else if primary_click(&press) || matches!(cx.part_activated(PORT.at(port as u64)), Some(crate::runtime::commands::FireSource::Access)) {
                // An assistive technology's click is a primary click with no modifiers.
                self.event = Some(NodeEvent::Clicked { port: Some(port), mods: press.mods });
            }
        }
        let body = if self.dot { cx.part(BODY).press } else { *cx.press() };
        if primary_click(&body) {
            self.event = Some(NodeEvent::Clicked { port: None, mods: body.mods });
        }
        let centers = (0..ports).map(|port| (port, port_center(self.kind, cx.rect, port), self.kind.port_direction(port)));
        let centers = centers.collect();
        let target = PortsTarget { node: self.id, ports: centers, reach: cx.style.reach };
        if let Some(WireDrop { from, to, op, detached, .. }) = cx.drop_target::<WireDrag, _>(&target) {
            self.event = Some(NodeEvent::Wired { from, to, op, detached });
        }
        // `WireEnded` comes in the pass `drag_ended` reports; `from` and `detached` are the drag's own payload.
        if let Some(end) = cx.drag_ended::<WireDrag>() {
            // The node's layer is the graph's body layer, where the node's rect is the body origin plus its place.
            let loose = end.op.is_none().then(|| self.place + (end.at - cx.rect.min));
            let WireDrag { from, detached, .. } = end.payload;
            self.event = Some(NodeEvent::WireEnded { op: end.op, from, detached, loose });
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (rect, style) = (cx.rect, cx.style);
        let variant = cx.resolved.variant;
        let inner = rect.shrink(style.pad);
        let (header, tones) = ui.with_style(variant, |scx| {
            (self.kind.header(scx), (0..self.kind.port_count()).map(|port| self.kind.port_tone(port, scx)).collect::<Vec<_>>())
        });
        let footer = self.kind.footer();
        let over_part = (0..self.kind.port_count()).any(|port| cx.part(PORT.at(port as u64)).hovered) || (self.dot && cx.part(BODY).hovered);
        let hovered = cx.hit() || over_part;
        let mut below = Rect::from_min_max(Pos2::new(inner.left(), inner.top() + style.title_height), inner.max);
        {
            let draw = ui.draw();
            if self.dot {
                let circle = PaintShape::Circle(rect.center(), rect.width() / 2.0);
                draw.fill(&circle, style.dot_fill);
                ui.with_style(variant, |scx| self.kind.paint_body(&draw, rect, scx));
                // A failed edge is one weight wider than the selected edge and wins over it.
                if self.failed {
                    draw.stroke(&circle, StrokeWidth::Thick, style.failed);
                } else {
                    draw.stroke(&circle, StrokeWidth::Thin, cx.resolve(&style.edge));
                    draw.stroke(&circle, StrokeWidth::Medium, cx.resolve(&style.selected_edge));
                }
            } else {
                match &header {
                    Some(header) => {
                        let band = Rect::from_min_size(rect.min, Vec2::new(rect.width(), header.height));
                        let radius = CornerRadius { sw: 0, se: 0, ..style.surface.radius };
                        draw.data_fill(&PaintShape::Rect(band, radius), header.tone);
                        draw.rule(RuleKind::Divider, band.left_bottom(), band.right_bottom());
                        let mut title = band.shrink2(Vec2::new(style.pad, 0.0));
                        if let Some(icon) = &header.icon {
                            let mark = Rect::from_min_max(Pos2::new(title.right() - band.height(), band.top()), title.right_bottom());
                            draw.icon(mark, icon);
                            title.max.x = mark.left();
                        }
                        let ink = ui.with_style(variant, |scx| scx.ink_on(header.tone));
                        draw.text(title, &TextRun::new(self.kind.title()).role(style.role).color(ink).elide());
                        below.min.y = band.bottom() + style.pad;
                    }
                    None => {
                        let title = Rect::from_min_size(inner.min, Vec2::new(inner.width(), style.title_height));
                        draw.text(title, &TextRun::new(self.kind.title()).role(style.role));
                    }
                }
                ui.with_style(variant, |scx| self.kind.paint_body(&draw, rect, scx));
                if let Some(footer) = &footer {
                    let band = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - footer.height), rect.max);
                    let caption = TextRun::new(footer.text).role(TypeRole::Caption).color(draw.role(|c| c.text.muted)).align(HAlign::Center);
                    draw.text(band.shrink2(Vec2::new(style.pad, 0.0)), &caption.elide());
                    below.max.y = below.max.y.min(band.top());
                }
                let edge = PaintShape::Rect(rect, style.surface.radius);
                if self.failed {
                    draw.stroke(&edge, StrokeWidth::Thick, style.failed);
                } else {
                    draw.stroke(&edge, StrokeWidth::Medium, cx.resolve(&style.selected_edge));
                }
            }
            if self.bypassed {
                draw.stroke(&PaintShape::Path(vec![rect.left_top(), rect.right_bottom()]), style.strike, style.failed);
            }
            for (port, tone) in tones.into_iter().enumerate() {
                let circle = PaintShape::Circle(port_center(self.kind, rect, port), style.handle.side / 2.0);
                match tone {
                    Some(tone) => draw.data_fill(&circle, tone),
                    None => draw.fill(&circle, cx.resolve_part(PORT.at(port as u64), &style.handle.fill)),
                }
                draw.stroke(&circle, StrokeWidth::Thin, style.handle.stroke);
            }
            if self.port_labels == PortLabels::OnHover && hovered {
                self.paint_port_labels(&draw, rect, style);
            }
        }
        if self.dot {
            return;
        }
        if let Some(edit) = self.kind.controls(ui, below) {
            self.event = Some(NodeEvent::Edit(edit));
        }
    }

    /// The ports are buttons named by the kind; the footer's caption is the node's description.
    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        for port in 0..self.kind.port_count() {
            access.node(PORT.at(port as u64), Role::BUTTON).name = Name::Text(self.kind.port_name(port));
        }
        if let Some(footer) = self.kind.footer() {
            access.description(footer.text);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> NodeOutput<E> {
        NodeOutput { response: cx.response.clone(), event: self.event }
    }
}
