//! Step 9's scrub: a drag on the side label, the body or a part changes the value.

use crate::contract::{Component, Cx};
use crate::runtime::access::scope;
use crate::runtime::memory::sub;
use crate::runtime::windows::WindowHost;
use craft_behavior::{CancelReason, Cursor, CursorPriority, DragWarp, GesturePhase, PaintedCursor, ValueDrag};
use egui::{Pos2, Vec2};

/// The press that carries a scrub and the drag it scrubs by: a scrub part's, the side label's, or the field's own
/// while its text part leaves it the pointer.
pub(super) fn scrub_press<C: Component>(comp: &C, cx: &Cx<'_, C>) -> Option<(craft_behavior::PressState, ValueDrag)> {
    let scrub = comp.scrub()?;
    let pressed = |press: &craft_behavior::PressState| press.phase.is_press();
    let part = scrub.parts.iter().find_map(|(part, drag)| {
        let press = cx.core.results.get(&(part.key, part.index)).map(|result| result.press)?;
        pressed(&press).then(|| (press, drag.clone()))
    });
    let label = || scrub.by_label.then(|| side_label(cx)).flatten().map(|(press, _)| press).filter(pressed);
    let field = || (scrub.on_field && !cx.core.text_takes_pointer).then_some(cx.core.press).filter(pressed);
    part.or_else(|| label().or_else(field).map(|press| (press, scrub.drag.clone())))
}

/// The press and hover of the side label a `by_label` scrub reads: the inside label's part when the control declared
/// one (`PartsCx::label_part`), else the label scope's label.
fn side_label<C: Component>(cx: &Cx<'_, C>) -> Option<(craft_behavior::PressState, bool)> {
    match cx.core.results.get(&(crate::contract::InsideLabel::PART, None)) {
        Some(part) => Some((part.press, part.hovered)),
        None => scope::field_label(&cx.core.ctx).map(|label| (label.press, label.hovered)),
    }
}

/// The value a label scrub started from.
#[derive(Clone, Default)]
struct ScrubStart(f64);

/// The pointer warps of the running label scrub.
#[derive(Clone, Default)]
struct ScrubWarp(DragWarp);

/// The total warp of the running scrub. A scrub that warps moves the pointer to the other side of the window when
/// it reaches an edge, once the pointer from the last warp has shown up at its new place; a platform that cannot
/// warp leaves the drag as it is.
fn scrub_warp<C: Component>(cx: &mut Cx<'_, C>, drag: &ValueDrag, pos: Pos2) -> Vec2 {
    if !drag.warps() {
        return Vec2::ZERO;
    }
    let id = sub(cx.core.id, "scrub_warp");
    let ctx = cx.core.ctx.clone();
    let warp = &mut cx.core.memory.shown::<ScrubWarp>(id).0;
    let shift = warp.shift(pos);
    if warp.pending() {
        return shift;
    }
    let Some(screen) = crate::runtime::pointer::frame(&ctx).pos else { return shift };
    let hosts = WindowHost::of(&ctx);
    if let (Some(by), Some(desktop)) = (drag.warp_by(screen, ctx.content_rect()), hosts.desktop_pointer())
        && hosts.warp_pointer(desktop + by)
    {
        warp.sent(pos, by);
    }
    shift
}

/// Step 9, before the component: a drag on a source of the component's `Component::scrub` (its side label, its body,
/// a part) scrubs its value. A label scrub registers the control with its label scope so the label senses the drag
/// from the next pass on.
pub(crate) fn scrub_by_label<C: Component>(comp: &C, cx: &mut Cx<'_, C>) {
    use crate::contract::cx::Edit as _;
    use craft_behavior::{PressCancel, PressPhase};
    let Some(scrub) = comp.scrub() else { return };
    let running = cx.gesture().is_some_and(|phase| matches!(phase, GesturePhase::Begin | GesturePhase::Update));
    if cx.core.resolved.read_only || !cx.core.resolved.enabled {
        if running {
            cx.cancel(CancelReason::Dismissed);
        }
        return;
    }
    if scrub.by_label && scope::field_label(&cx.core.ctx).is_some() {
        scope::register_scrub(&cx.core.ctx);
    }
    let label = scrub.by_label.then(|| side_label(cx)).flatten();
    // A scrub's source shows the horizontal resize cursor under the pointer.
    let part_hovered = scrub.parts.iter().any(|(part, _)| cx.core.results.get(&(part.key, part.index)).is_some_and(|result| result.hovered));
    let field_hovered = scrub.on_field && !cx.core.text_takes_pointer && cx.core.response.hovered();
    if part_hovered || field_hovered || label.is_some_and(|(_, hovered)| hovered) {
        crate::runtime::cursor::request(&cx.core.ctx, Cursor::System(craft_behavior::CursorIcon::ResizeHorizontal), CursorPriority::Part);
    }
    let Some((press, drag)) = scrub_press(comp, cx) else { return };
    let start_id = sub(cx.core.id, "scrub_start");
    let rtl = cx.core.env.direction().is_rtl();
    match press.phase {
        PressPhase::DragStart { .. } => {
            cx.core.memory.shown::<ScrubStart>(start_id).0 = cx.value().map_or(0.0, |value| (scrub.read)(value));
            *cx.core.memory.shown::<ScrubWarp>(sub(cx.core.id, "scrub_warp")) = ScrubWarp::default();
            hide_scrub_cursor(cx, &drag);
            cx.begin();
        }
        PressPhase::Drag { pos, .. } => {
            hide_scrub_cursor(cx, &drag);
            let start = cx.core.memory.shown::<ScrubStart>(start_id).0;
            let shift = scrub_warp(cx, &drag, pos);
            if let Some(next) = drag.value_shifted(start, &press, rtl, shift) {
                cx.propose((scrub.write)(next));
            }
        }
        PressPhase::Release { .. } if running => cx.commit(),
        PressPhase::Cancel { reason, .. } if running => cx.cancel(match reason {
            PressCancel::Escape => CancelReason::Escape,
            PressCancel::Removed => CancelReason::Removed,
            _ => CancelReason::Blur,
        }),
        _ => {}
    }
}

/// A scrub that hides the pointer asks for no cursor while it runs.
fn hide_scrub_cursor<C: Component>(cx: &Cx<'_, C>, drag: &ValueDrag) {
    if drag.hides_cursor() {
        crate::runtime::cursor::request(&cx.core.ctx, Cursor::Painted(PaintedCursor::Hidden), CursorPriority::Drag);
    }
}
