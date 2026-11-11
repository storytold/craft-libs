//! [`PinWarp`]: Puppet Warp's pins on the canvas, placed, moved, turned and removed by pointer and
//! keyboard. Each pin is a part in the canvas's arena; the mesh is one part below them.

use craft_behavior::geometry::crop::normalized_angle;
use craft_behavior::kurbo::{Point, Vec2 as DocVec};
use craft_behavior::{
    Axis, CancelReason, CanvasView, CursorIcon, EditCommand, Granularity, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape, PartKey,
    PointerButton, PressPhase, Roving, Sense,
};
use craft_ui::canvas::{CanvasSpace, NUDGE, NUDGE_FAR, canvas_space};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Numeric, Outcome, PaintUi, PartsCx,
    Pointer as _, Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::{Label, Units, WordTemplate};
use craft_ui::runtime::focus::{EditSet, KeyUse};
use craft_ui::style::{ComponentStyle, DataColor, Flags, StyleColor, StyleCx};
use egui::{Color32, Pos2, Rect, Response, Vec2};

use super::{Pin, PinEdit, PinMeshView, PinSolver, PinWarpSession};
use crate::ui::warp::mesh::paint_mesh_lines;

craft_ui::words! {
    /// The accessible words of [`PinWarp`].
    pub struct PinWarpWords {
        pin: template "pinWarp.pin" = "Pin {n} of {count}";
        turn: template "pinWarp.turn" = "turn {degrees}°";
        position: template "pinWarp.position" = "{x}, {y}";
    }
}

/// The part of one pin: `PIN.at(i)` with the pin's index.
pub(super) const PIN: PartKey = PartKey::new("pin");
/// The part of the deformed mesh, below the pins: a press on it places a pin.
const MESH: PartKey = PartKey::new("mesh");

/// Photoshop's pin yellow.
const PIN_FILL: DataColor = DataColor::new(Color32::from_rgb(255, 214, 10));

/// The commands of Puppet Warp (11). They are app commands: the app runs them with [`PinWarpCommands::apply`] while
/// the session is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "pinWarp")]
#[non_exhaustive]
pub enum PinWarpCommands {
    /// Place a pin at the pointer, or at the middle of the mesh.
    #[command(label = "Add Pin", default = "Insert")]
    AddPin,
}

impl PinWarpCommands {
    /// Runs the command on `session`. `pointer` is the document point under the pointer
    /// ([`PinWarpOutput::pointer`]). `Err` carries a refused remesh's text.
    ///
    /// `AddPin` places a pin at the rest position under `pointer`, else at the mesh's
    /// [centre](PinMeshView::centre), and nothing where the mesh has no rest position.
    pub fn apply(
        self,
        session: &mut PinWarpSession,
        solver: &mut dyn PinSolver,
        mesh: PinMeshView<'_>,
        pointer: Option<Point>,
    ) -> Result<(), String> {
        match self {
            PinWarpCommands::AddPin => {
                let under = pointer.and_then(|at| mesh.rest_of(at).map(|rest| Pin::new(rest, at)));
                let centred = || {
                    let at = mesh.centre()?;
                    Some(Pin::new(mesh.rest_of(at)?, at))
                };
                let Some(pin) = under.or_else(centred) else { return Ok(()) };
                let mut grown = session.pins().to_vec();
                grown.push(pin);
                let edit = PinEdit::replace(session.pins(), grown, "Add Pin");
                session.apply(edit, solver)
            }
        }
    }
}

/// The look of the pins.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PinWarpStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// A pin's disc diameter.
    pin: f32,
    /// A selected pin's core diameter.
    core: f32,
    /// A pin's ring and a selected pin's core.
    ring: StyleColor,
    /// The fixed-rotation ring and its line.
    turn: StyleColor,
    /// The radius of the fixed-rotation ring.
    turn_ring: f32,
    /// The mesh lines while Show Mesh is on.
    mesh: StyleColor,
    /// How far from a selected pin an Alt press turns it: the turn ring is the target.
    turn_reach: f32,
    /// How far from a pin a press takes the mesh's pin hits.
    reach: f32,
}

/// What the pins report beyond the edit of the pin list.
#[derive(Clone, Debug, Default)]
struct Report {
    select: Option<Vec<usize>>,
    apply: bool,
    discard: bool,
    hovered: Option<usize>,
    pointer: Option<Point>,
}

/// Puppet Warp's pins over the warped picture (drawn by the transform preview beneath), and the mesh lines when
/// Show Mesh is on.
///
/// Pointer (Photoshop):
/// - A click on the mesh away from pins places a pin at the rest position under it
///   ([`PinMeshView::rest_of`]), selects it, and the drag that follows moves it: one step. A click off the mesh does
///   nothing.
/// - A press on a pin selects it and drags it; Shift-click adds it to the selection or takes it out; dragging a
///   selected pin moves every selected pin.
/// - Alt-click on a pin deletes it (one step).
/// - Alt-drag within the turn reach of a selected pin, not on it, turns it: the pin's turn at the press (its fixed
///   rotation, or an Auto pin's turn in the deformed mesh, [`PinMeshView::turn_at`]) changes by the angle the pointer
///   sweeps about the pin, and the result is its fixed rotation, which it keeps.
/// - Escape during a drag puts the pins back.
///
/// Keys (one Tab stop): Page Down and Page Up pick the next and previous pin, Home and End the first and last;
/// arrows move the picked pin (Shift by the large step); Delete or Backspace removes the selected pins; Cmd+A selects
/// every pin; Enter reports [`apply`](PinWarpOutput::apply) and Escape with no drag reports
/// [`discard`](PinWarpOutput::discard). [`PinWarpCommands::AddPin`] places a pin.
///
/// Each pin is a graphics object "Pin 2 of 5" valued "x, y" in the canvas's units, with "turn N°" when its rotation is
/// fixed, and its selected state. Nothing mirrors in a right-to-left interface.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct PinWarp<'a> {
    #[common]
    common: Common,
    pins: Bind<'a, Vec<Pin>>,
    mesh: PinMeshView<'a>,
    /// The selected pins' indices.
    #[prop]
    selected_pins: Option<&'a [usize]>,
    /// Whether the mesh lines draw.
    #[prop]
    show_mesh: bool,
    report: Report,
}

impl<'a> PinWarp<'a> {
    /// The pins named `label` over `pins`, on the solver's `mesh` of this pass.
    pub fn new(label: impl Into<Label>, pins: Bind<'a, Vec<Pin>>, mesh: PinMeshView<'a>) -> Self {
        PinWarp { common: Common::new(), pins, mesh, selected_pins: None, show_mesh: false, report: Report::default() }.label(label)
    }

    fn selection(&self) -> &'a [usize] {
        self.selected_pins.unwrap_or(&[])
    }
}

/// What a press does.
#[derive(Clone, Debug)]
enum Kind {
    /// Moves these pins by the pointer's movement.
    Move(Vec<usize>),
    /// Moves the placed pin, the last one; Escape restores the selection the press found.
    Place { before: Vec<usize> },
    /// Turns this pin about its position: its turn when pressed and the pointer's angle about it then, in degrees.
    Turn { pin: usize, base: f64, from: f64 },
}

/// A press the pins own, kept for its length.
#[derive(Clone, Debug)]
struct Drag {
    kind: Kind,
    /// The press, in document units.
    origin: Point,
    /// The pins the movement is added to, with the placed pin for a placement.
    base: Vec<Pin>,
    began: bool,
}

impl Drag {
    /// The pins with the pointer at document point `pointer`.
    fn value_at(&self, pointer: Point) -> Vec<Pin> {
        let mut pins = self.base.clone();
        let by = pointer - self.origin;
        match &self.kind {
            Kind::Move(moved) => {
                for &i in moved {
                    if let Some(pin) = pins.get_mut(i) {
                        pin.at += by;
                    }
                }
            }
            Kind::Place { .. } => {
                if let Some(pin) = pins.last_mut() {
                    pin.at += by;
                }
            }
            Kind::Turn { pin, base, from } => {
                if let Some(pin) = pins.get_mut(*pin)
                    && let Some(now) = angle_of(pointer - pin.at)
                {
                    pin.angle = Some(normalized_angle(base + (now - from)));
                }
            }
        }
        pins
    }
}

impl Default for Drag {
    fn default() -> Self {
        Drag { kind: Kind::Place { before: Vec::new() }, origin: Point::ZERO, base: Vec::new(), began: false }
    }
}

/// The angle of `v` in degrees, `None` for the zero vector.
fn angle_of(v: DocVec) -> Option<f64> {
    (v.hypot() > 0.0).then(|| v.y.atan2(v.x).to_degrees())
}

/// The selected pin nearest screen point `p` and within `reach` of it.
fn nearest_selected(view: &CanvasView, pins: &[Pin], selected: &[usize], p: Pos2, reach: f32) -> Option<usize> {
    selected
        .iter()
        .filter_map(|&i| pins.get(i).map(|pin| (i, view.to_screen(pin.at).distance(p))))
        .filter(|&(_, d)| d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// A document point written in the canvas's units with `template` ("{x}, {y}").
pub(in crate::ui::warp) fn point_text(space: &CanvasSpace, units: &Units, template: &WordTemplate, at: Point) -> String {
    let (x, zero_x) = space.rulers().across(Axis::Y);
    let (y, zero_y) = space.rulers().across(Axis::X);
    template.fill(&[("x", &x.format_value(units, at.x, zero_x)), ("y", &y.format_value(units, at.y, zero_y))])
}

impl PinWarp<'_> {
    /// A press on pin `i`: Alt deletes it; Shift toggles it in the selection; otherwise it is selected unless it
    /// already is. The drag that follows moves the selected pins.
    fn press_pin(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, pins: &[Pin], i: usize, (alt, shift): (bool, bool), origin: Pos2) {
        if alt {
            let mut rest = pins.to_vec();
            rest.remove(i);
            cx.set(rest);
            return;
        }
        let mut now = self.selection().to_vec();
        let moved = if shift {
            match now.iter().position(|&k| k == i) {
                Some(at) => {
                    now.remove(at);
                    None
                }
                None => {
                    now.push(i);
                    Some(now.clone())
                }
            }
        } else if now.contains(&i) {
            Some(now.clone())
        } else {
            now = vec![i];
            Some(vec![i])
        };
        if now != self.selection() {
            self.report.select = Some(now);
        }
        *cx.press_slot::<Drag>() =
            moved.map(|moved| Drag { kind: Kind::Move(moved), origin: space.view.to_doc(origin), base: pins.to_vec(), began: false });
    }

    /// A press on the mesh: Alt near a selected pin turns it; otherwise a pin is placed under the press.
    fn press_mesh(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, pins: &[Pin], alt: bool, origin: Pos2) {
        let at = space.view.to_doc(origin);
        if alt {
            let turn = nearest_selected(&space.view, pins, self.selection(), origin, cx.style.turn_reach).and_then(|i| {
                let pin = pins[i];
                let base = pin.angle.or_else(|| self.mesh.turn_at(pin.rest))?;
                Some(Drag { kind: Kind::Turn { pin: i, base, from: angle_of(at - pin.at)? }, origin: at, base: pins.to_vec(), began: false })
            });
            *cx.press_slot::<Drag>() = turn;
        } else if let Some(rest) = self.mesh.rest_of(at) {
            let mut placed = pins.to_vec();
            placed.push(Pin::new(rest, at));
            let before = self.selection().to_vec();
            self.report.select = Some(vec![placed.len() - 1]);
            cx.begin();
            cx.propose(placed.clone());
            *cx.press_slot::<Drag>() = Some(Drag { kind: Kind::Place { before }, origin: at, base: placed, began: true });
        }
    }

    /// Follows or ends the drag of a press.
    fn track(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, phase: PressPhase) {
        match phase {
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                let Some(mut drag) = cx.press_slot::<Drag>().clone() else { return };
                if !drag.began {
                    cx.begin();
                    drag.began = true;
                }
                cx.propose(drag.value_at(space.view.to_doc(pos)));
                *cx.press_slot::<Drag>() = Some(drag);
            }
            PressPhase::Release { pos, .. } => {
                let Some(drag) = cx.press_slot::<Drag>().take() else { return };
                if drag.began {
                    cx.propose(drag.value_at(space.view.to_doc(pos)));
                    cx.commit();
                }
            }
            PressPhase::Cancel { .. } => {
                if let Some(drag) = cx.press_slot::<Drag>().take()
                    && drag.began
                {
                    if let Kind::Place { before } = drag.kind {
                        self.report.select = Some(before);
                    }
                    cx.cancel(CancelReason::Escape);
                }
            }
            _ => {}
        }
    }

    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, pins: &[Pin]) {
        for i in 0..pins.len() {
            let Some(result) = cx.part_if_declared(PIN.at(i as u64)) else { continue };
            let (press, hovered, hover_pos) = (result.press, result.hovered, result.hover_pos);
            if hovered {
                self.report.hovered = Some(i);
                self.report.pointer = hover_pos.map(|at| space.view.to_doc(at));
            }
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    self.press_pin(cx, space, pins, i, (press.mods.alt, press.mods.shift), start.origin);
                }
                phase => self.track(cx, space, phase),
            }
        }
        let Some(result) = cx.part_if_declared(MESH) else { return };
        let (press, hovered, hover_pos) = (result.press, result.hovered, result.hover_pos);
        if hovered && self.report.pointer.is_none() {
            self.report.pointer = hover_pos.map(|at| space.view.to_doc(at));
        }
        match press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                self.press_mesh(cx, space, pins, press.mods.alt, start.origin);
            }
            phase => self.track(cx, space, phase),
        }
    }

    /// The keys of the focused pin: selection follows the focus, the arrows move it, Delete removes the selected
    /// pins, Cmd+A selects them all, Enter applies and Escape discards.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, pins: &[Pin]) {
        if cx.intents().take_exact(&Intent::Submit) {
            self.report.apply = true;
        }
        if cx.intents().take_exact(&Intent::Cancel) {
            self.report.discard = true;
        }
        // Focus arriving on a pin by a key or a request selects it. A press selects by its own rule (a Shift-click
        // grows the selection and focuses the pin it adds), so an arrival with the pin pressed selects nothing more.
        let arrived = cx.focus_arrived_by_key().filter(|part| part.key == PIN).and_then(|part| part.index).map(|index| index as usize);
        if let Some(arrived) = arrived.filter(|&i| i < pins.len())
            && self.report.select.is_none()
        {
            self.report.select = Some(vec![arrived]);
        }
        let focused = cx.active_part().filter(|part| part.key == PIN).and_then(|part| part.index).map(|index| index as usize);
        let Some(picked) = focused.filter(|&i| i < pins.len()) else {
            return;
        };
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let toward = space.view.unmap_vec(direction);
            if toward.hypot() > 0.0 {
                let mut moved = pins.to_vec();
                moved[picked].at += toward / toward.hypot() * step;
                cx.set(moved);
            }
        }
        if cx.intents().take_edit() == Some(EditCommand::SelectAll) {
            self.report.select = Some((0..pins.len()).collect());
        }
        let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
        let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        if forward || backward {
            let chosen: Vec<usize> = if self.selection().is_empty() { vec![picked] } else { self.selection().to_vec() };
            let rest: Vec<Pin> = pins.iter().enumerate().filter(|(i, _)| !chosen.contains(i)).map(|(_, pin)| *pin).collect();
            cx.set(rest);
        }
    }
}

/// What the pins report: the edit of the pin list and what they ask the app to do.
#[derive(Debug)]
#[non_exhaustive]
pub struct PinWarpOutput {
    /// The edit of the pin list: a drag is one `Begin` to `Commit`; a nudge, a delete or an Alt-click is a `Set`.
    pub pins: EditResponse<Vec<Pin>>,
    /// The selection the pins ask for, as indices into the pins.
    pub select: Option<Vec<usize>>,
    /// Enter: the app commits ([`PinWarpSession::result`]).
    pub apply: bool,
    /// Escape with no drag running: the app discards the session.
    pub discard: bool,
    /// The pin under the pointer.
    pub hovered: Option<usize>,
    /// The document point under the pointer, for [`PinWarpCommands::apply`].
    pub pointer: Option<Point>,
}

impl PinWarpOutput {
    /// The edits for [`PinWarpSession::apply`], in order: the selection, then the pins.
    pub fn edits(&self) -> Vec<PinEdit> {
        let selection = self.select.iter().map(|select| PinEdit::Select(select.clone()));
        let pins = self.pins.events.iter().map(|event| PinEdit::Pins(event.clone()));
        selection.chain(pins).collect()
    }
}

impl Outcome for PinWarpOutput {
    type Value = Vec<Pin>;

    fn response(&self) -> &Response {
        &self.pins.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.pins.story_facts()
    }
}

impl Component for PinWarp<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves().with_deletes().with_submits().with_edits(EditSet::NONE.with(EditCommand::SelectAll));
    type Style = PinWarpStyle;
    type Output = PinWarpOutput;

    fn style(cx: &StyleCx<'_>, _: PinWarpStyleKey) -> PinWarpStyle {
        let tokens = cx.tokens;
        let overlay = &tokens.color.overlay;
        PinWarpStyle {
            metrics: Metrics::new(),
            pin: tokens.size.handle.md,
            core: tokens.size.handle.sm,
            ring: cx.color(overlay.handle_stroke),
            turn: cx.color(overlay.stroke),
            turn_ring: tokens.size.handle.ring,
            mesh: cx.color(overlay.line),
            turn_reach: tokens.size.handle.ring,
            reach: tokens.size.hit.target,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, Vec<Pin>>> {
        Some(self.pins.reborrow())
    }

    /// The pins as handle-level drag parts in one roving group, and the mesh below them.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(pins)) = (canvas_space(&cx.env), cx.value()) else { return };
        let radius = cx.style.pin / 2.0;
        let selected = self.selection();
        for (i, pin) in pins.iter().enumerate() {
            let part = cx
                .part(PIN.at(i as u64), HitShape::circle(space.view.to_screen(pin.at), radius))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .cursor(CursorIcon::Move)
                .focusable();
            if selected.contains(&i) {
                part.selected().flags(Flags::SELECTED);
            }
        }
        if !pins.is_empty() {
            cx.roving(Roving::new(NavShape::Flat).wrap(true).page(Some(1)));
        }
        let points: Vec<Pos2> = self.mesh.deformed.iter().map(|&p| space.view.to_screen(p)).collect();
        if !points.is_empty() {
            let area = Rect::from_points(&points).expand(cx.style.reach);
            cx.part(MESH, HitShape::from(area))
                .sense(Sense::DRAG | Sense::CLICK)
                .priority(HitPriority::new(HitLevel::Body, 0))
                .pointer_only("PinWarpCommands::AddPin puts a pin under the pointer or at the middle of the mesh");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let pins = cx.value().cloned().unwrap_or_default();
        self.run_pointer(cx, &space, &pins);
        self.run_keys(cx, &space, &pins);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(pins)) = (canvas_space(&cx.env), cx.value()) else { return };
        let draw = ui.draw();
        let style = &cx.style;
        if self.show_mesh {
            let at = |i: u32| self.mesh.deformed.get(i as usize).copied();
            let edges: Vec<[Point; 2]> = self.mesh.edges().into_iter().filter_map(|[a, b]| Some([at(a)?, at(b)?])).collect();
            paint_mesh_lines(&draw, &space.view, &edges, style.mesh);
        }
        let selected = self.selection();
        let mut order: Vec<usize> = (0..pins.len()).collect();
        order.sort_by_key(|&i| pins[i].depth);
        for i in order {
            let pin = &pins[i];
            let center = space.view.to_screen(pin.at);
            let disc = PaintShape::Circle(center, style.pin / 2.0);
            draw.data_fill(&disc, PIN_FILL);
            draw.stroke(&disc, StrokeWidth::Thin, style.ring);
            if selected.contains(&i) {
                draw.fill(&PaintShape::Circle(center, style.core / 2.0), style.ring);
                if let Some(angle) = pin.angle {
                    let direction = space.view.map_vec(DocVec::from_angle(angle.to_radians()));
                    let direction = if direction.length() > 0.0 { direction.normalized() } else { Vec2::X };
                    draw.stroke(&PaintShape::Circle(center, style.turn_ring), StrokeWidth::Thin, style.turn);
                    draw.stroke(&PaintShape::Path(vec![center, center + direction * style.turn_ring]), StrokeWidth::Thin, style.turn);
                }
            }
        }
    }

    /// One graphics object per pin, named by its place among the pins and valued by its position and fixed turn.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(pins)) = (canvas_space(&cx.env), cx.value()) else { return };
        let words = cx.words::<PinWarpWords>();
        let units = cx.env.units();
        for (i, pin) in pins.iter().enumerate() {
            let name = words.pin.fill(&[("n", &(i + 1).to_string()), ("count", &pins.len().to_string())]);
            let mut value = point_text(&space, &units, &words.position, pin.at);
            if let Some(angle) = pin.angle {
                let degrees = units.number(angle, 0..=1);
                value = format!("{value}, {}", words.turn.fill(&[("degrees", &degrees)]));
            }
            let node = access.node(PIN.at(i as u64), Role::GRAPHICS_OBJECT);
            node.name = Name::Text(name);
            node.numeric = Some(Numeric::empty(0.0..=0.0).text(value));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PinWarpOutput {
        let Report { select, apply, discard, hovered, pointer } = self.report;
        PinWarpOutput { pins: cx.edit_response(), select, apply, discard, hovered, pointer }
    }
}
