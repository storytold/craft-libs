//! [`RulerOverlay`]: the end handles of the Ruler line, a canvas control. Its three points and the line are parts in
//! the canvas's arena, so a press on one stays with the overlay and a press elsewhere reaches the Ruler tool.

use super::model::{RulerLine, RulerPart};
use super::words::RulerWords;
use craft_behavior::kurbo::{Point, Vec2 as DocVec};
use craft_behavior::{
    Cursor, CursorIcon, CursorPriority, Granularity, HitLevel, HitPriority, HitShape, IconId, Intent, Metrics, NavShape, PaintedCursor, PartKey,
    PartRef, PointerButton, PointerGlyph, PressPhase, Roving, Sense,
};
use craft_theme::{Stateful, States};
use craft_ui::canvas::{CanvasSpace, NUDGE, NUDGE_FAR, canvas_space};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, PaintUi, PartsCx, Pointer as _, Props as _,
    Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::{Label, Units};
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Rect, Vec2};

/// The part of one point of the line: `POINT.at(RulerPart::index)`.
pub(super) const POINT: PartKey = PartKey::new("point");
/// The line between the ends, of a ruler without a protractor: `LINE.at(0)`.
const LINE: PartKey = PartKey::new("line");

/// The style of the Ruler overlay: a cross at each point in the look of the line (a thin line over a thick halo).
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RulerOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The length of each arm of a cross, from its point.
    arm: f32,
    /// The cross's line per state: the line's colour at rest, the accent while hovered or dragged.
    line: Stateful<StyleColor>,
    /// The halo under the cross, so it reads on any image.
    halo: StyleColor,
    /// A press within this many screen points of a point on either axis grabs it; within as many of the line, the
    /// line.
    reach: f32,
}

/// What a drag in progress moves, kept until the button comes up.
#[derive(Clone, Copy, Default)]
struct Grab(Option<Grabbed>);

#[derive(Clone, Copy)]
enum Grabbed {
    /// One point.
    Point(RulerPart),
    /// The whole ruler: the press in document units and the line then.
    Whole { from: Point, at_press: RulerLine },
}

/// A part of the overlay.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Point(RulerPart),
    Line,
}

impl Target {
    fn reference(self) -> PartRef {
        match self {
            Target::Point(part) => POINT.at(part.key()),
            Target::Line => LINE.at(0),
        }
    }

    /// What a primary press on the target grabs: the line, and a protractor's vertex, the whole ruler.
    fn grabs(self, line: RulerLine, from: Point) -> Grabbed {
        match self {
            Target::Line => Grabbed::Whole { from, at_press: line },
            Target::Point(RulerPart::Start) if line.protractor.is_some() => Grabbed::Whole { from, at_press: line },
            Target::Point(part) => Grabbed::Point(part),
        }
    }

    /// Whether an Alt press on the target starts the protractor arm: an end of a plain ruler, or a protractor's far
    /// end (the vertex moves the whole protractor).
    fn starts_arm(self, line: RulerLine) -> bool {
        matches!(self.grabs(line, Point::ZERO), Grabbed::Point(RulerPart::Start | RulerPart::End))
    }
}

/// The Ruler's pointer: the arrow with a ruler, or with a protractor where an Alt press starts the protractor arm;
/// the crosshair while the canvas shows precise cursors.
pub(super) fn ruler_cursor(precise: bool, protractor: bool) -> Cursor {
    if precise {
        return CursorIcon::Crosshair.into();
    }
    PaintedCursor::pointer_tool(PointerGlyph::Arrow, IconId::new(if protractor { "protractor" } else { "ruler" })).into()
}

/// The Ruler line's points as handles: Start, End and, with a protractor, its free end.
///
/// - **Pointer:** a press in the square of the reach around a point grabs it (the protractor's end first, then the
///   end, then the start); a drag moves it and Shift puts it on the nearest 45° ray about the fixed end (about the
///   vertex for the protractor's arm), at the pointer's distance from it. Alt-press on an end starts the protractor
///   arm at that vertex (the ends swap first when the far end was pressed). A press within the reach of a plain
///   ruler's line, or on a protractor's vertex with or without Alt, moves the whole ruler by the drag; a protractor's
///   arm lines take no press. Escape puts the line back.
/// - **Cursor:** the Ruler's arrow with a ruler; with a protractor where an Alt press starts the protractor arm; the
///   crosshair while the canvas shows precise cursors.
/// - **Keyboard:** the points are one Tab stop; Page Up and Page Down pick a point, the arrows move it 1 unit and
///   Shift 10, Delete and Backspace clear the line.
/// - It paints a cross at each point, in the line's look; the app shows the line and the arm with `StraightLinePreview`
///   ([`RulerPass::show_line`](super::RulerPass::show_line)).
/// - [Read only](craft_ui::contract::Props::read_only), it paints the crosses at rest and takes no press, key or focus: the ruler drawn
///   under the other tools ([`show_ruler`](super::show_ruler)).
/// - **AccessKit:** the group's value reads "Length L, angle A°" in the ruler's units; each point is named and valued
///   "x, y" in the ruler's units.
#[derive(Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct RulerOverlay<'a> {
    #[common]
    common: Common,
    line: Bind<'a, Option<RulerLine>>,
}

impl<'a> RulerOverlay<'a> {
    /// An overlay named `label` over the Ruler `line` (document units).
    pub fn new(label: impl Into<Label>, line: Bind<'a, Option<RulerLine>>) -> Self {
        RulerOverlay { common: Common::new(), line }.label(label)
    }
}

/// `line` moved by `by` as a whole.
pub(super) fn translated(line: RulerLine, by: DocVec) -> RulerLine {
    RulerLine { start: line.start + by, end: line.end + by, protractor: line.protractor.map(|arm| arm + by) }
}

/// `to` snapped to the nearest 45 degree ray about `from`, keeping the pointer's legs instead of its distance: on a
/// diagonal both legs take the shorter of |dx| and |dy| (signs kept), on a horizontal or vertical ray the dominant leg
/// stays and the other is 0. The ruler never gets longer than the pointer's distance, which `turn_to_45` (used by the
/// pen, the connector and the gradient) does not promise.
fn turn_short(from: Point, to: Point) -> Point {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let step = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() as i64;
    let (x, y) = match step.rem_euclid(4) {
        0 => (dx, 0.0),
        2 => (0.0, dy),
        _ => {
            let leg = dx.abs().min(dy.abs());
            (leg.copysign(dx), leg.copysign(dy))
        }
    };
    Point::new(from.x + x, from.y + y)
}

/// The line with the grabbed `part` moved to `to`, Shift on 45° rays about the fixed point (`turn_short`): the end for
/// the start, the start for the end and the protractor.
pub(super) fn moved(line: RulerLine, part: RulerPart, to: Point, shift: bool) -> RulerLine {
    let constrain = |fixed: Point| if shift { turn_short(fixed, to) } else { to };
    let mut line = line;
    match part {
        RulerPart::Start => line.start = constrain(line.end),
        RulerPart::End => line.end = constrain(line.start),
        RulerPart::Protractor => line.protractor = Some(constrain(line.start)),
    }
    line
}

impl Component for RulerOverlay<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves().with_deletes().with_chrome_key(false);
    type Style = RulerOverlayStyle;
    type Output = EditResponse<Option<RulerLine>>;

    fn style(cx: &StyleCx<'_>, _: RulerOverlayStyleKey) -> RulerOverlayStyle {
        let (overlay, accent) = (&cx.tokens.color.overlay, &cx.tokens.color.accent);
        let rest = cx.color(overlay.stroke);
        let line =
            Stateful::new(rest).off(States::new(rest, cx.color(accent.hover), cx.color(accent.pressed), cx.color(cx.tokens.color.text.disabled)));
        let arm = cx.tokens.size.handle.sm;
        RulerOverlayStyle {
            metrics: Metrics::new().min_size(Vec2::splat(2.0 * arm)),
            arm,
            line,
            halo: cx.color(overlay.halo),
            reach: cx.tokens.size.hit.target,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<RulerLine>>> {
        Some(self.line.reborrow())
    }

    /// The points: drag parts at handle priority in one roving group, in keyboard order; a plain ruler's line: a drag
    /// part under them.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        if cx.resolved.read_only {
            return;
        }
        let (Some(space), Some(line)) = (canvas_space(&cx.env), cx.value().copied().flatten()) else { return };
        let reach = cx.style.reach;
        let mut any = false;
        for (sub, part) in [(0u8, RulerPart::Start), (1, RulerPart::End), (2, RulerPart::Protractor)] {
            let Some(at) = line.point(part) else { continue };
            let square = Rect::from_center_size(space.view.to_screen(at), Vec2::splat(2.0 * reach));
            cx.part(POINT.at(part.key()), HitShape::Rect(square))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, sub))
                .focusable();
            any = true;
        }
        if any {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
        let (a, b) = (space.view.to_screen(line.start), space.view.to_screen(line.end));
        let along = b - a;
        if line.protractor.is_none() && along.length() > 0.0 {
            let across = along.normalized().rot90() * reach;
            cx.part(LINE.at(0), HitShape::Polygon(vec![a + across, b + across, b - across, a - across]))
                .sense(Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Edge, 0))
                .pointer_only("the arrow keys move the whole ruler while no end has focus");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if cx.resolved.read_only {
            return;
        }
        let (Some(space), Some(line)) = (canvas_space(&cx.env), cx.value().copied().flatten()) else { return };
        let targets = [Target::Point(RulerPart::Protractor), Target::Point(RulerPart::End), Target::Point(RulerPart::Start), Target::Line];
        for target in targets {
            let Some(result) = cx.part_if_declared(target.reference()) else { continue };
            let (press, hovered) = (result.press, result.hovered);
            let running = cx.press_slot::<Grab>().is_some_and(|grab| grab.0.is_some());
            if hovered && !press.phase.is_press() && !running {
                let arm = press.mods.alt && target.starts_arm(line);
                cx.cursor(ruler_cursor(space.precise(), arm), CursorPriority::Part);
            }
            // The line the grab gives with the pointer at `pos`.
            let at = |grabbed: Grabbed, base: RulerLine, pos: egui::Pos2| match grabbed {
                Grabbed::Point(part) => moved(base, part, space.view.to_doc(pos), press.mods.shift),
                Grabbed::Whole { from, at_press } => translated(at_press, space.view.to_doc(pos) - from),
            };
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    cx.press_slot::<Grab>().replace(Grab(Some(target.grabs(line, space.view.to_doc(start.origin)))));
                }
                PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                    let pressed = target.grabs(line, space.view.to_doc(start.origin));
                    let grabbed = cx.press_slot::<Grab>().get_or_insert(Grab(Some(pressed))).0.unwrap_or(pressed);
                    let (base, grabbed) = match grabbed {
                        // Alt at an end starts the protractor arm at that vertex; the far end swaps in first.
                        Grabbed::Point(part @ (RulerPart::Start | RulerPart::End)) if press.mods.alt => {
                            let mut base = line;
                            if part == RulerPart::End {
                                std::mem::swap(&mut base.start, &mut base.end);
                            }
                            (base.with_protractor(space.view.to_doc(pos)), Grabbed::Point(RulerPart::Protractor))
                        }
                        grabbed => (line, grabbed),
                    };
                    cx.press_slot::<Grab>().replace(Grab(Some(grabbed)));
                    cx.begin();
                    cx.propose(Some(at(grabbed, base, pos)));
                }
                PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                    if let Some(grabbed) = cx.press_slot::<Grab>().and_then(|grab| grab.0) {
                        cx.propose(Some(at(grabbed, line, pos)));
                    }
                }
                PressPhase::Release { start, pos, click, .. } if start.button == PointerButton::Primary => {
                    let grab = cx.press_slot::<Grab>().take();
                    if let Some(grabbed) = grab.and_then(|grab| grab.0).filter(|_| !click) {
                        cx.propose(Some(at(grabbed, line, pos)));
                        cx.commit();
                    }
                }
                _ => {}
            }
        }

        let focused = cx.active_part().filter(|part| part.key == POINT).and_then(|part| part.index).and_then(RulerPart::of_key);
        if let (Some(part), Some((direction, mods))) = (focused, cx.intents().take_nudge()) {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let by = nudge_vector(&space.view, direction) * step;
            if let Some(at) = line.point(part) {
                cx.set(Some(moved(line, part, at + by, false)));
            }
        }
        let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
        let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        if forward || backward {
            cx.set(None);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(line)) = (canvas_space(&cx.env), cx.value().copied().flatten()) else { return };
        let draw = ui.draw();
        let style = &cx.style;
        for part in [RulerPart::Start, RulerPart::End, RulerPart::Protractor] {
            let Some(at) = line.point(part) else { continue };
            let centre = space.view.to_screen(at);
            let arms = [Vec2::new(style.arm, 0.0), Vec2::new(0.0, style.arm)].map(|arm| PaintShape::Path(vec![centre - arm, centre + arm]));
            // A point the pass's edit just added has no part yet; it shows as the control at rest.
            let at_rest = cx.part_if_declared(POINT.at(part.key())).is_none();
            let ink = if at_rest { cx.resolve(&style.line) } else { cx.resolve_part(POINT.at(part.key()), &style.line) };
            for arm in &arms {
                draw.stroke(arm, StrokeWidth::Thick, style.halo);
            }
            for arm in &arms {
                draw.stroke(arm, StrokeWidth::Thin, ink);
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(line)) = (canvas_space(&cx.env), cx.value().copied().flatten()) else { return };
        let words = cx.words::<RulerWords>();
        let units = cx.env.units();
        let rulers = space.rulers();
        access.value(reading_text(&line, &space, &units, &words));
        if cx.resolved.read_only {
            return;
        }
        for (part, name) in [(RulerPart::Start, &words.start), (RulerPart::End, &words.end), (RulerPart::Protractor, &words.protractor)] {
            let Some(at) = line.point(part) else { continue };
            let value = format!("{}, {}", rulers.x.format_value(&units, at.x, 0.0), rulers.y.format_value(&units, at.y, 0.0));
            let node = access.node(POINT.at(part.key()), Role::BUTTON);
            node.name = Name::Text(name.as_str().to_owned());
            node.value(value);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<Option<RulerLine>> {
        cx.edit_response()
    }
}

/// The line's length and angle as assistive technology reads them, in the canvas's ruler units.
pub(super) fn reading_text(line: &RulerLine, space: &CanvasSpace, units: &Units, words: &RulerWords) -> String {
    let length = space.rulers().x.format_value(units, line.reading().l1, 0.0);
    let angle = units.number(line.reading().angle, 0..=1);
    words.value.fill(&[("length", length.as_str()), ("angle", angle.as_str())])
}

/// The unit vector in document space of an arrow key's direction on screen.
fn nudge_vector(view: &craft_behavior::CanvasView, direction: Vec2) -> DocVec {
    let origin = view.rect.center();
    let moved = view.to_doc(origin + direction) - view.to_doc(origin);
    let length = moved.hypot();
    if length == 0.0 { DocVec::ZERO } else { moved / length }
}
