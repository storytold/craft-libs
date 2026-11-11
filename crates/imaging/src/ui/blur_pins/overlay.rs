//! [`BlurPinsOverlay`]: the Blur Gallery's pins over a canvas, a canvas control (20.6), and the commands of the pins.

use std::f64::consts::TAU;

use craft_behavior::kurbo::Point;
use craft_behavior::{
    CanvasView, Granularity, HitLevel, HitPriority, HitShape, Intent, NavShape, PartKey, PartRef, PointerButton, PressPhase, Roving, Sense,
};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Numeric, Outcome, PaintUi, PartsCx,
    Pointer as _, Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{ContentPainterCx, Draw, FocusKind, FocusRing, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::StyleCx;
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

use super::grab::{AXIS_HANDLES, BlurGrab, PinReach, drag, feather_handles, iris_point, round_handle, tilt_point, turn_handle};
use super::model::{BlurKind, BlurPin, BlurPins};
use super::style::BlurPinsStyle;
use super::words::BlurPinsWords;

/// The part of one pin's dot: `PIN.at(index)`.
pub(super) const PIN: PartKey = PartKey::new("pin");
/// The ring of the selected pin.
const RING: PartKey = PartKey::new("ring");
/// An outline handle of the selected pin: `AXIS.at(0..4)`.
const AXIS: PartKey = PartKey::new("axis");
/// Iris's rounding square.
const ROUND: PartKey = PartKey::new("round");
/// One of Iris's feather dots: `FEATHER.at(0..4)`.
const FEATHER: PartKey = PartKey::new("feather");
/// A Tilt-Shift turn dot: `TURN.at(0)` is the +1 side, `TURN.at(1)` the −1 side.
const TURN: PartKey = PartKey::new("turn");
/// A Tilt-Shift solid line: `FOCUS.at(0)` above, `FOCUS.at(1)` below the centre line.
const FOCUS: PartKey = PartKey::new("focus");
/// A Tilt-Shift dashed line, as [`FOCUS`].
const TRANSITION: PartKey = PartKey::new("transition");
/// The canvas behind the pins: a press there adds a pin.
const ADD: PartKey = PartKey::new("add");

/// The distance an arrow key moves the selected pin, in document pixels.
const NUDGE: f64 = 1.0;
/// The distance an arrow key moves it with Shift held.
const NUDGE_FAR: f64 = 10.0;
/// Segments of the ring's hit shape and of an Iris outline.
const RING_SEGMENTS: usize = 48;
const OUTLINE_SEGMENTS: usize = 128;
/// Segments of a full amount arc.
const ARC_SEGMENTS: f32 = 64.0;

/// The commands of the pins (11). They are app commands: the app owns the pins and runs them with
/// [`BlurPinsCommand::apply`]; the panel and the menu offer Add Pin, the keyboard path to creating a pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "blurPins")]
#[non_exhaustive]
pub enum BlurPinsCommand {
    /// Add a pin at the middle of the bounds.
    #[command(label = "Add Pin")]
    AddPin,
    /// Remove the selected pin.
    #[command(label = "Delete Pin")]
    DeletePin,
}

impl BlurPinsCommand {
    /// Runs the command on `pins`. Returns whether the pins changed.
    pub fn apply(self, pins: &mut BlurPins) -> bool {
        match self {
            BlurPinsCommand::AddPin => {
                let b = pins.bounds;
                pins.add(Point::new((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0))
            }
            BlurPinsCommand::DeletePin => pins.remove_selected(),
        }
    }
}

fn usable(pins: &BlurPins) -> bool {
    let b = pins.bounds;
    b[2] > b[0] && b[3] > b[1]
}

/// The press the overlay holds: what it grabbed and the pins it works from.
#[derive(Clone)]
struct Held {
    grab: BlurGrab,
    /// The pins at the press, with the pin a press on nothing added or a press selected.
    start: BlurPins,
    /// What a release without a drag sets: `start` when the press added or selected.
    pending: Option<BlurPins>,
    /// The press and the previous pass's pointer, in document pixels.
    from: Point,
    last: Point,
    /// The pins the drag proposed last: the ring turns from them pass by pass.
    frame: BlurPins,
    began: bool,
}

#[derive(Clone, Default)]
struct HeldSlot(Option<Held>);

/// One mark of the pins that takes the pointer.
struct Mark {
    part: PartRef,
    grab: BlurGrab,
    shape: HitShape,
    priority: HitPriority,
}

fn circle_shape(centre: Pos2, radius: f32) -> HitShape {
    HitShape::circle(centre, radius)
}

/// The marks in the legacy grab order: the selected pin's handles, its ring and lines, every pin's dot, the canvas.
fn marks(pins: &BlurPins, view: &CanvasView, reach: &PinReach, canvas: Rect) -> Vec<Mark> {
    let screen = |p: Point| view.to_screen(p);
    let handle = HitPriority::new(HitLevel::Handle, 0);
    let mut out = Vec::new();
    if let Some(index) = pins.selected
        && let Some(pin) = pins.pins.get(index)
    {
        if let Some(shape) = pin.iris {
            for (axis, &(u, v)) in AXIS_HANDLES.iter().enumerate() {
                out.push(Mark {
                    part: AXIS.at(axis as u64),
                    grab: BlurGrab::Axis { pin: index, axis },
                    shape: circle_shape(screen(iris_point(pins, pin, shape, u, v)), reach.grab),
                    priority: handle,
                });
            }
            if pins.kind == BlurKind::Iris {
                let (u, v) = round_handle(shape);
                out.push(Mark {
                    part: ROUND.into(),
                    grab: BlurGrab::Round { pin: index },
                    shape: circle_shape(screen(iris_point(pins, pin, shape, u, v)), reach.grab),
                    priority: handle,
                });
                for (dot, &(u, v)) in feather_handles(shape).iter().enumerate() {
                    out.push(Mark {
                        part: FEATHER.at(dot as u64),
                        grab: BlurGrab::Feather { pin: index, dot },
                        shape: circle_shape(screen(iris_point(pins, pin, shape, u, v)), reach.grab),
                        priority: handle,
                    });
                }
            }
        }
        if let Some(band) = pin.tilt {
            for (slot, side) in [1i8, -1].into_iter().enumerate() {
                out.push(Mark {
                    part: TURN.at(slot as u64),
                    grab: BlurGrab::Turn { pin: index, side },
                    shape: circle_shape(screen(turn_handle(pins, pin, band, view, reach, f64::from(side))), reach.grab),
                    priority: handle,
                });
            }
            let side = pins.short_side();
            let line = |offset: f64| {
                HitShape::segment(screen(tilt_point(pins, pin, band, offset, -side)), screen(tilt_point(pins, pin, band, offset, side)))
            };
            let edge = HitPriority::new(HitLevel::Edge, 0);
            for (slot, sign) in [1.0, -1.0].into_iter().enumerate() {
                out.push(Mark {
                    part: FOCUS.at(slot as u64),
                    grab: BlurGrab::Focus { pin: index },
                    shape: line(sign * band.focus * side),
                    priority: edge,
                });
                out.push(Mark {
                    part: TRANSITION.at(slot as u64),
                    grab: BlurGrab::Transition { pin: index },
                    shape: line(sign * (band.focus + band.transition) * side),
                    priority: edge,
                });
            }
        }
        let centre = screen(pins.to_document(pin));
        let ring: Vec<Pos2> = (0..RING_SEGMENTS).map(|n| centre + Vec2::angled(TAU as f32 * n as f32 / RING_SEGMENTS as f32) * reach.ring).collect();
        out.push(Mark {
            part: RING.into(),
            grab: BlurGrab::Ring { pin: index },
            shape: HitShape::polyline(ring, true),
            priority: HitPriority::new(HitLevel::Edge, 1),
        });
    }
    for (index, pin) in pins.pins.iter().enumerate() {
        out.push(Mark {
            part: PIN.at(index as u64),
            grab: BlurGrab::Pin { pin: index },
            shape: circle_shape(screen(pins.to_document(pin)), reach.grab),
            priority: HitPriority::new(HitLevel::Body, 0),
        });
    }
    out.push(Mark { part: ADD.into(), grab: BlurGrab::New, shape: HitShape::Rect(canvas), priority: HitPriority::new(HitLevel::Background, 0) });
    out
}

/// The Blur Gallery's pins over a canvas: a dot per pin and, round the selected one, its ring, outline and handles
/// (Tilt-Shift: its lines and turn dots).
///
/// - A press on a pin selects it and a drag moves it; a press on the selected pin's ring turns its amount, and on a
///   handle or a Tilt-Shift line shapes it. A press on the canvas outside the pins adds a pin and drags it
///   (Tilt-Shift has one band and adds none). A press that moves is one gesture; a click that only adds or selects
///   is a set. Escape cancels the drag.
/// - The pins are one Tab stop. Page Up and Page Down select the previous or next pin, the arrow keys move it one
///   document pixel (Shift ten) and Delete or Backspace remove it. [`BlurPinsCommand::AddPin`] adds a pin at the
///   middle of the bounds.
/// - Show it at the canvas's rect in the canvas body, after the document.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct BlurPinsOverlay<'a> {
    #[common]
    common: Common,
    pins: Bind<'a, BlurPins>,
}

impl<'a> BlurPinsOverlay<'a> {
    /// The pins named `label` over `pins`.
    pub fn new(label: impl Into<Label>, pins: Bind<'a, BlurPins>) -> Self {
        BlurPinsOverlay { common: Common::new(), pins }.label(label)
    }

    /// The pointer on the pins: a press grabs, a drag shapes, the release commits.
    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, view: &CanvasView, pins: &BlurPins) {
        let reach = cx.style.reach;
        let mut held = cx.memory::<HeldSlot>().0.take();
        for mark in marks(pins, view, &reach, cx.rect) {
            let Some(result) = cx.part_if_declared(mark.part) else { continue };
            let press = result.press;
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    let from = view.to_doc(start.origin);
                    let mut at = pins.clone();
                    let mut grab = mark.grab;
                    let mut pending = None;
                    match grab {
                        BlurGrab::New => {
                            if at.add(from) {
                                grab = BlurGrab::Pin { pin: at.pins.len() - 1 };
                                pending = Some(at.clone());
                            }
                        }
                        BlurGrab::Pin { pin } if at.selected != Some(pin) => {
                            at.selected = Some(pin);
                            pending = Some(at.clone());
                        }
                        _ => {}
                    }
                    held = Some(Held { grab, start: at.clone(), pending, from, last: from, frame: at, began: false });
                }
                PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                    let Some(h) = held.as_mut().filter(|h| h.grab != BlurGrab::New) else { continue };
                    if !h.began {
                        cx.begin();
                        h.began = true;
                    }
                    let to = view.to_doc(pos);
                    // The ring turns by the angle since the last pass, so several turns add up.
                    let ring = matches!(h.grab, BlurGrab::Ring { .. });
                    let base = if ring { h.frame.clone() } else { h.start.clone() };
                    let mut next = base.clone();
                    drag(&mut next, h.grab, &base, if ring { h.last } else { h.from }, to);
                    cx.propose(next.clone());
                    h.frame = next;
                    h.last = to;
                }
                PressPhase::Release { start, .. } if start.button == PointerButton::Primary => {
                    if let Some(mut h) = held.take() {
                        if h.began {
                            cx.commit();
                        } else if let Some(pending) = h.pending.take() {
                            cx.set(pending);
                        }
                    }
                }
                PressPhase::Cancel { .. } => {
                    held = None;
                }
                _ => {}
            }
        }
        cx.memory::<HeldSlot>().0 = held;
    }

    /// The keys of the focused pin: selection follows the focus, the arrows move it, Delete removes it.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, view: &CanvasView, pins: &BlurPins) {
        let focused = cx.active_part().filter(|part| part.key == PIN).and_then(|part| part.index);
        // Focus arriving on a pin by a key or a request selects it; a press selects by its own rule.
        let arrived = cx.focus_arrived_by_key().is_some_and(|part| part.key == PIN && part.index.is_some());
        let Some(index) = focused.and_then(|index| usize::try_from(index).ok()).filter(|index| *index < pins.pins.len()) else { return };
        let mut next = pins.clone();
        if arrived {
            next.selected = Some(index);
        }
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            next.selected = Some(index);
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let by = view.unmap_vec(direction);
            let length = by.x.hypot(by.y);
            if length > 0.0 {
                let at = next.to_document(&next.pins[index]);
                let (x, y) = next.from_document(Point::new(at.x + by.x / length * step, at.y + by.y / length * step));
                next.pins[index].x = x;
                next.pins[index].y = y;
            }
        }
        let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
        let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        if forward || backward {
            next.selected = Some(index);
            if next.remove_selected()
                && let Some(now) = next.selected
            {
                cx.focus_part(PIN.at(now as u64));
            }
        }
        if next != *pins {
            cx.set(next);
        }
    }

    /// A pin's accessible name: "Pin 3, Blur 15 px" (Spin Blur: "Pin 3, Blur Angle 90°").
    fn name_of(cx: &Cx<'_, Self>, pins: &BlurPins, index: usize, pin: &BlurPin) -> String {
        let words = cx.words::<BlurPinsWords>();
        let units = cx.env.units();
        let (word, amount) = if pins.kind == BlurKind::Spin {
            (&words.pin_part_angle, units.degrees(pin.amount.round()))
        } else {
            (&words.pin_part, units.pixels(pin.amount.round()))
        };
        let n = units.number(index as f64 + 1.0, 0..=0);
        word.fill(&[("n", n.as_str()), ("amount", amount.as_str())])
    }
}

/// What the pins report: the edit of the pins.
#[derive(Debug)]
#[non_exhaustive]
pub struct BlurPinsOutput {
    /// The edit of the pins: a press that moves is one `Begin` to `Commit`; a click that adds or selects, a nudge and
    /// a delete are a `Set`.
    pub edit: EditResponse<BlurPins>,
}

impl Outcome for BlurPinsOutput {
    type Value = BlurPins;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl Component for BlurPinsOverlay<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_deletes();
    type Style = BlurPinsStyle;
    type Output = BlurPinsOutput;

    fn style(cx: &StyleCx<'_>, _: BlurPinsOverlayStyleKey) -> BlurPinsStyle {
        BlurPinsStyle::of(cx)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, BlurPins>> {
        Some(self.pins.reborrow())
    }

    /// The marks in grab order; the pin dots are one roving group, Page Up and Page Down move its arrival.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(pins)) = (canvas_space(&cx.env), cx.value()) else { return };
        if !usable(pins) {
            return;
        }
        let tilt_shift = pins.kind == BlurKind::TiltShift;
        let selected = pins.selected;
        let mut any = false;
        for mark in marks(pins, &space.view, &cx.style.reach, rect) {
            let cursor = mark.grab.cursor(tilt_shift);
            let sense = Sense::DRAG | Sense::CLICK;
            match mark.grab {
                BlurGrab::Pin { pin } => {
                    let part = cx
                        .part(mark.part, mark.shape)
                        .sense(sense | Sense::TAKE_FOCUS)
                        .priority(mark.priority)
                        .cursor(cursor)
                        .drag_cursor(cursor)
                        .focusable();
                    if selected == Some(pin) {
                        part.selected();
                    }
                    any = true;
                }
                BlurGrab::New => {
                    cx.part(mark.part, mark.shape)
                        .sense(sense)
                        .priority(mark.priority)
                        .cursor(cursor)
                        .pointer_only("BlurPinsCommand::AddPin adds a pin at the middle of the bounds");
                }
                _ => {
                    cx.part(mark.part, mark.shape)
                        .sense(sense)
                        .priority(mark.priority)
                        .cursor(cursor)
                        .drag_cursor(cursor)
                        .pointer_only("a handle shapes the selected pin by pointer; the Blur row of the panel sets its amount");
                }
            }
        }
        if any {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(space), Some(pins)) = (canvas_space(&cx.env), cx.value().cloned()) else { return };
        if !usable(&pins) {
            return;
        }
        self.run_pointer(cx, &space.view, &pins);
        let pins = cx.value().cloned().unwrap_or(pins);
        self.run_keys(cx, &space.view, &pins);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(pins)) = (canvas_space(&cx.env), cx.value()) else { return };
        if !usable(pins) {
            return;
        }
        let painter = ui.content_painter(cx);
        let draw = ui.draw();
        let (style, view, ppp) = (&cx.style, space.view, painter.pixels_per_point());
        let focused = cx.focus_visible().then(|| cx.active_part()).flatten().filter(|part| part.key == PIN).and_then(|part| part.index);
        let scene = Scene { painter: &painter, draw: &draw, view: &view, pins, style, dot: style.handle.side / 2.0 };
        for (index, pin) in pins.pins.iter().enumerate() {
            let centre = view.to_screen(pins.to_document(pin));
            if pins.selected == Some(index) {
                selected(&scene, pin, centre);
            } else {
                painter.outline(&PaintShape::Circle(centre, scene.dot));
            }
            if focused == Some(index as u64) {
                let footprint =
                    PaintShape::Rect(Rect::from_center_size(centre, Vec2::splat(2.0 * style.reach.grab)), CornerRadius::from(style.reach.grab));
                painter.painter.extend(FocusRing::shapes(&style.focus, FocusKind::Halo, &footprint, ppp));
            }
        }
    }

    /// One node per pin, named by its number and amount.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(pins) = cx.value() else { return };
        for (index, pin) in pins.pins.iter().enumerate() {
            let node = access.node(PIN.at(index as u64), Role::SLIDER);
            node.name = Name::Text(Self::name_of(cx, pins, index, pin));
            node.numeric = Some(Numeric::new(pin.amount, 0.0..=pins.kind.max()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> BlurPinsOutput {
        BlurPinsOutput { edit: cx.edit_response() }
    }
}

/// The selected pin: its outline and handles (or Tilt-Shift's lines and dots), its ring with the amount as an arc
/// from the top, and its dot filled.
#[derive(Clone, Copy)]
struct Scene<'a> {
    painter: &'a ContentPainterCx<'a>,
    draw: &'a Draw<'a>,
    view: &'a CanvasView,
    pins: &'a BlurPins,
    style: &'a BlurPinsStyle,
    /// A dot's and a handle's radius.
    dot: f32,
}

impl Scene<'_> {
    /// A filled handle: the handle colour inside a halo-coloured edge.
    fn handle(&self, shape: &PaintShape) {
        self.draw.fill(shape, self.style.line);
        self.draw.stroke(shape, StrokeWidth::Thin, self.style.halo);
    }

    /// `from` to `to` as dashes along the kit's halo-and-line outline.
    fn dashed(&self, from: Pos2, to: Pos2) {
        let length = (to - from).length();
        let (dash, period) = (self.style.dash.max(f32::EPSILON), (self.style.dash + self.style.dash_gap).max(f32::EPSILON));
        let along = (to - from) / length.max(f32::EPSILON);
        let mut start = 0.0;
        while start < length {
            let end = (start + dash).min(length);
            self.painter.outline(&PaintShape::Path(vec![from + along * start, from + along * end]));
            start += period;
        }
    }
}

/// The part of the document segment from `a` to `b` inside `bounds` (Liang-Barsky), or `None` outside it.
fn clipped(a: Point, b: Point, bounds: [f64; 4]) -> Option<(Point, Point)> {
    let (delta, mut enter, mut leave) = (b - a, 0.0f64, 1.0f64);
    for (p, q) in [(-delta.x, a.x - bounds[0]), (delta.x, bounds[2] - a.x), (-delta.y, a.y - bounds[1]), (delta.y, bounds[3] - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 { enter = enter.max(t) } else { leave = leave.min(t) }
        }
    }
    (enter <= leave).then(|| (a + delta * enter, a + delta * leave))
}

/// The selected pin: its outline and handles (or Tilt-Shift's lines and dots), its ring with the amount as an arc
/// from the top, and its dot filled.
fn selected(scene: &Scene<'_>, pin: &BlurPin, centre: Pos2) {
    let Scene { painter, draw, view, pins, style, dot } = *scene;
    let screen = |p: Point| view.to_screen(p);
    if let Some(shape) = pin.iris {
        // Where the blur reaches its full amount (Iris) or where the spin ends.
        let m = shape.exponent();
        let outline: Vec<Pos2> = (0..=OUTLINE_SEGMENTS)
            .map(|n| {
                let a = TAU * n as f64 / OUTLINE_SEGMENTS as f64;
                let (sin, cos) = a.sin_cos();
                let r = (cos.abs().powf(m) + sin.abs().powf(m)).powf(-1.0 / m);
                screen(iris_point(pins, pin, shape, r * cos, r * sin))
            })
            .collect();
        painter.outline(&PaintShape::Path(outline));
        for (u, v) in AXIS_HANDLES {
            scene.handle(&PaintShape::Circle(screen(iris_point(pins, pin, shape, u, v)), dot));
        }
        if pins.kind == BlurKind::Iris {
            let (u, v) = round_handle(shape);
            let square = Rect::from_center_size(screen(iris_point(pins, pin, shape, u, v)), Vec2::splat(2.0 * dot));
            scene.handle(&PaintShape::Rect(square, CornerRadius::from(style.round_corner)));
            for (u, v) in feather_handles(shape) {
                scene.handle(&PaintShape::Circle(screen(iris_point(pins, pin, shape, u, v)), dot + style.feather_extra));
            }
        }
    }
    if let Some(band) = pin.tilt {
        // The sharp band's edges (solid) and where the blur is full (dashed), across the image.
        let side = pins.short_side();
        let b = pins.bounds;
        let reach = (b[2] - b[0]).hypot(b[3] - b[1]);
        for (offset, dash) in [(band.focus, false), (band.focus + band.transition, true)] {
            for sign in [1.0, -1.0] {
                let o = sign * offset * side;
                let ends = (tilt_point(pins, pin, band, o, -reach), tilt_point(pins, pin, band, o, reach));
                let Some((a, z)) = clipped(ends.0, ends.1, b) else { continue };
                if dash {
                    scene.dashed(screen(a), screen(z));
                } else {
                    painter.outline(&PaintShape::Path(vec![screen(a), screen(z)]));
                }
            }
        }
        for sign in [1.0, -1.0] {
            scene.handle(&PaintShape::Circle(screen(turn_handle(pins, pin, band, view, &style.reach, sign)), dot + style.turn_extra));
        }
    }
    let ring = style.reach.ring;
    painter.outline(&PaintShape::Circle(centre, ring));
    let fraction = (pin.amount / pins.kind.max()).clamp(0.0, 1.0) as f32;
    if fraction > 0.0 {
        let steps = (fraction * ARC_SEGMENTS).ceil().max(1.0) as usize;
        let arc: Vec<Pos2> = (0..=steps)
            .map(|n| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * fraction * n as f32 / steps as f32;
                centre + Vec2::angled(a) * ring
            })
            .collect();
        draw.stroke(&PaintShape::Path(arc), StrokeWidth::Thick, style.amount);
    }
    scene.handle(&PaintShape::Circle(centre, dot));
}
