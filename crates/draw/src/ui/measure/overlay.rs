//! [`RulerOverlay`]: the end handles of the Ruler line, a canvas control. Its three points are parts in the canvas's
//! arena, so a press on one stays with the overlay and a press elsewhere reaches the Ruler tool.

use super::model::{RulerLine, RulerPart};
use super::words::RulerWords;
use craft_behavior::geometry::{Directions, round_to_direction};
use craft_behavior::kurbo::{Point, Vec2 as DocVec};
use craft_behavior::{Granularity, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape, PartKey, PointerButton, PressPhase, Roving, Sense};
use craft_ui::canvas::{NUDGE, NUDGE_FAR, canvas_space};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, PaintUi, PartsCx, Pointer as _, Props as _,
    Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, HandleLook, StyleCx};
use egui::{CornerRadius, Rect, Vec2};

/// The part of one point of the line: `POINT.at(RulerPart::index)`.
pub(super) const POINT: PartKey = PartKey::new("point");

/// The style of the Ruler overlay.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RulerOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    handle: HandleLook,
    /// A press within this many screen points of a point grabs it.
    reach: f32,
}

/// The point a drag in progress moves, kept until the button comes up.
#[derive(Clone, Copy, Default)]
struct Grab(Option<RulerPart>);

/// The Ruler line's points as handles: Start, End and, with a protractor, its free end.
///
/// - **Pointer:** a press within the reach of a point grabs it (the protractor's end first, then the end, then the
///   start); a drag moves it and Shift puts it on the nearest 45° ray about the fixed end (about the vertex for the
///   protractor's arm), at the pointer's distance from it. Alt-press on an end starts the protractor arm at that
///   vertex (the ends swap first when the far end was pressed). Escape puts the line back.
/// - **Keyboard:** the points are one Tab stop; Page Up and Page Down pick a point, the arrows move it 1 unit and
///   Shift 10, Delete and Backspace clear the line.
/// - It paints the handles only; the app shows the line and the arm with `StraightLinePreview`
///   ([`RulerPass::show_line`](super::RulerPass::show_line)).
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

/// `to` on the nearest 45° ray from `fixed`, at its distance from `fixed` (`to` itself when it is on `fixed` or not
/// finite). The ruler keeps the pointer's distance; a projection would shorten it.
pub(super) fn on_ray(fixed: Point, to: Point) -> Point {
    let offset = to - fixed;
    let length = offset.hypot();
    if !length.is_finite() || length == 0.0 {
        return to;
    }
    let angle = round_to_direction(offset.atan2(), Directions::Eight);
    fixed + DocVec::new(angle.cos(), angle.sin()) * length
}

/// The line with the grabbed `part` moved to `to`, Shift on 45° rays about the fixed point: the end for the start, the
/// start for the end and the protractor.
pub(super) fn moved(line: RulerLine, part: RulerPart, to: Point, shift: bool) -> RulerLine {
    let constrain = |fixed: Point| if shift { on_ray(fixed, to) } else { to };
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
        let handle = cx.handle();
        RulerOverlayStyle { metrics: Metrics::new().min_size(Vec2::splat(handle.side)), handle, reach: cx.tokens.size.hit.target }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<RulerLine>>> {
        Some(self.line.reborrow())
    }

    /// The points: drag parts at handle priority in one roving group, in keyboard order.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(line)) = (canvas_space(&cx.env), cx.value().copied().flatten()) else { return };
        let reach = cx.style.reach;
        let mut any = false;
        for (sub, part) in [(0u8, RulerPart::Start), (1, RulerPart::End), (2, RulerPart::Protractor)] {
            let Some(at) = line.point(part) else { continue };
            cx.part(POINT.at(part.key()), HitShape::circle(space.view.to_screen(at), reach))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, sub))
                .cursor(craft_behavior::CursorIcon::Crosshair)
                .focusable();
            any = true;
        }
        if any {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(space), Some(line)) = (canvas_space(&cx.env), cx.value().copied().flatten()) else { return };
        for part in [RulerPart::Protractor, RulerPart::End, RulerPart::Start] {
            let Some(result) = cx.part_if_declared(POINT.at(part.key())) else { continue };
            let press = result.press;
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    cx.press_slot::<Grab>().replace(Grab(Some(part)));
                }
                PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                    let grabbed = cx.press_slot::<Grab>().get_or_insert(Grab(Some(part))).0.unwrap_or(part);
                    let (base, grabbed) = if press.mods.alt && grabbed != RulerPart::Protractor {
                        // Alt at an end starts the protractor arm at that vertex; the far end swaps in first.
                        let mut base = line;
                        if grabbed == RulerPart::End {
                            std::mem::swap(&mut base.start, &mut base.end);
                        }
                        (base.with_protractor(space.view.to_doc(pos)), RulerPart::Protractor)
                    } else {
                        (line, grabbed)
                    };
                    cx.press_slot::<Grab>().replace(Grab(Some(grabbed)));
                    cx.begin();
                    cx.propose(Some(moved(base, grabbed, space.view.to_doc(pos), press.mods.shift)));
                }
                PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                    if let Some(grabbed) = cx.press_slot::<Grab>().map(|grab| grab.0).flatten() {
                        cx.propose(Some(moved(line, grabbed, space.view.to_doc(pos), press.mods.shift)));
                    }
                }
                PressPhase::Release { start, pos, click, .. } if start.button == PointerButton::Primary => {
                    let grab = cx.press_slot::<Grab>().take();
                    if let Some(grabbed) = grab.and_then(|grab| grab.0).filter(|_| !click) {
                        cx.propose(Some(moved(line, grabbed, space.view.to_doc(pos), press.mods.shift)));
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
        let look = &cx.style.handle;
        for part in [RulerPart::Start, RulerPart::End, RulerPart::Protractor] {
            let Some(at) = line.point(part) else { continue };
            let square = PaintShape::Rect(Rect::from_center_size(space.view.to_screen(at), Vec2::splat(look.side)), CornerRadius::ZERO);
            // A point the pass's edit just added has no part yet; it shows as the control at rest.
            let at_rest = cx.part_if_declared(POINT.at(part.key())).is_none();
            draw.fill(&square, if at_rest { cx.resolve(&look.fill) } else { cx.resolve_part(POINT.at(part.key()), &look.fill) });
            draw.stroke(&square, StrokeWidth::Thin, look.stroke);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(line)) = (canvas_space(&cx.env), cx.value().copied().flatten()) else { return };
        let words = cx.words::<RulerWords>();
        let units = cx.env.units();
        let rulers = space.rulers();
        let length = rulers.x.format_value(&units, line.reading().l1, 0.0);
        let angle = units.number(line.reading().angle, 0..=1);
        access.value(words.value.fill(&[("length", length.as_str()), ("angle", angle.as_str())]));
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

/// The unit vector in document space of an arrow key's direction on screen.
fn nudge_vector(view: &craft_behavior::CanvasView, direction: Vec2) -> DocVec {
    let origin = view.rect.center();
    let moved = view.to_doc(origin + direction) - view.to_doc(origin);
    let length = moved.hypot();
    if length == 0.0 { DocVec::ZERO } else { moved / length }
}
