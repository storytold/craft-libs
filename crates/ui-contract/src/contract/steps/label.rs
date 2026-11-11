//! Step 9's side-label part: activation by the label and the label scrub.

use crate::contract::{Component, Cx};
use crate::runtime::access::scope;
use crate::runtime::memory::sub;
use crate::runtime::windows::WindowHost;
use craft_behavior::{CancelReason, Cursor, CursorPriority, DragWarp, GesturePhase, PaintedCursor, ValueDrag};
use egui::{Pos2, Vec2};

/// The press that carries a scrub: the field's own, or for a scrub by the label, the label's.
pub(super) fn scrub_press<C: Component>(comp: &C, cx: &Cx<'_, C>) -> Option<craft_behavior::PressState> {
    let scrub = comp.scrub()?;
    if scrub.on_field { Some(cx.core.press) } else { scope::field_label(&cx.core.ctx).map(|label| label.press) }
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

/// Step 9, before the component: a control that `Component::LABELLED_AFTER` marks registers with its label scope, so a
/// click on the label activates it.
pub(crate) fn activate_by_label<C: Component>(cx: &Cx<'_, C>) {
    if C::LABELLED_AFTER && scope::field_label(&cx.core.ctx).is_some() {
        scope::register_activates(&cx.core.ctx);
    }
}

/// Step 9, before the component: a drag on the side label (or on the component itself, for a scrub that
/// `Scrub::on_field` declares) scrubs the value of a component that opts in with `Component::scrub`. A label scrub
/// registers the control with its label scope so the label senses the drag from the next pass on.
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
    if !scrub.on_field && scope::field_label(&cx.core.ctx).is_some() {
        scope::register_scrub(&cx.core.ctx);
    }
    let Some(press) = scrub_press(comp, cx) else { return };
    let start_id = sub(cx.core.id, "scrub_start");
    let rtl = cx.core.env.direction().is_rtl();
    match press.phase {
        PressPhase::DragStart { .. } => {
            cx.core.memory.shown::<ScrubStart>(start_id).0 = cx.value().map_or(0.0, |value| (scrub.read)(value));
            *cx.core.memory.shown::<ScrubWarp>(sub(cx.core.id, "scrub_warp")) = ScrubWarp::default();
            hide_scrub_cursor(cx, &scrub.drag);
            cx.begin();
        }
        PressPhase::Drag { pos, .. } => {
            hide_scrub_cursor(cx, &scrub.drag);
            let start = cx.core.memory.shown::<ScrubStart>(start_id).0;
            let shift = scrub_warp(cx, &scrub.drag, pos);
            if let Some(next) = scrub.drag.value_shifted(start, &press, rtl, shift) {
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
