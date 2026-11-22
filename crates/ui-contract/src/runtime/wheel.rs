//! The wheel service (8.6): takers register their view rects, the begin hook splits the pass's wheel input
//! among them from last pass's registrations, and each taker reads its share once. The device reports lines and
//! pages; the service converts them to points with the notch each taker registered.

use crate::runtime::egui::{input, pass, repaint};
use crate::runtime::focus;
use crate::runtime::frame::PerViewport;
use crate::runtime::pointer;
use crate::runtime::sampler;
use craft_behavior::wheel::split;
use craft_behavior::{GESTURE_GAP, PointerButton, PointerEvent, Wheel, WheelFocus, WheelGesture, WheelPhase, WheelSource, WheelTaker, WheelUnit};
use craft_keymap::Mods;
use egui::{Context, Id, Pos2, Vec2};

#[derive(Default)]
pub(crate) struct State {
    this: Vec<WheelTaker<Id>>,
    last: Vec<WheelTaker<Id>>,
    /// The last-pressed gated control while the pointer has stayed inside it since.
    armed: Option<Id>,
    shares: Vec<(Id, Wheel)>,
    /// The wheel gesture the split tracks across passes.
    gesture: WheelGesture<Id>,
}

/// Registers a taker for next pass's split.
pub(crate) fn register(ctx: &Context, taker: WheelTaker<Id>) {
    crate::runtime::replay::journal(ctx, || taker, |ctx, &taker| register(ctx, taker));
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| state.this.push(taker));
}

/// The innermost scroll owner inside the field `owner` under `at` that can still move by the vertical scroll `dy`: the
/// split gives a focused field the whole wheel, and a multi-line text part that scrolls its own text inside the field
/// (`text_adapter`) answers for it. `None` when `owner` is no field, the share is not vertical, or nothing inside it
/// scrolls that way, so the field keeps its share and a text at its end lets the wheel go as the field would.
fn scroll_owner_inside(takers: &[WheelTaker<Id>], owner: Id, at: Pos2, dy: f32) -> Option<Id> {
    if dy == 0.0 {
        return None;
    }
    let field = takers.iter().find(|taker| taker.owner == owner && taker.field)?;
    takers
        .iter()
        // Takers register outer first: of equal views the later one is the inner.
        .rev()
        .filter(|taker| taker.owner != owner && taker.scrolls && taker.vertical && taker.rect.contains(at) && field.rect.contains_rect(taker.rect))
        .filter(|taker| taker.room.is_none_or(|room| if dy > 0.0 { room.positive.y > 0.0 } else { room.negative.y > 0.0 }))
        .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
        .map(|taker| taker.owner)
}

/// The begin hook (H7): rotates the registrations, ends the wheel gesture a press or the gap closed, and splits this
/// pass's wheel input.
pub(crate) fn begin(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    // Takes the pass's snapshot of the canvas a modal keeps open, which the input hook and the layer test read.
    let hole = sampler::hole_of_pass(ctx);
    let pass_pointer = pointer::pass_pointer(ctx);
    let frame = pass_pointer.frame.clone();
    let wheel_in = input::wheel_read(ctx, 1.0, 1.0);
    let now = crate::runtime::frame::clock(ctx);
    let focused = focus::focused_stop(ctx).map(|(owner, _)| owner);
    // A text field the app drew itself holds key focus: it reads its own wheel.
    let app_field = wheel_in.is_some() && focus::record(ctx, viewport).host_text;
    let hover = pass_pointer.hover.or(frame.as_ref().and_then(|frame| frame.pos));
    // A popup the pointer is over, opened above the focused field's layer, and the canvas a modal keeps open below it
    // take the wheel by position.
    let by_position =
        wheel_in.is_some() && hover.is_some_and(|at| focus::popup_above_focus(ctx, at) || hole.as_ref().is_some_and(|hole| hole.answers(at)));
    let pressed_on = pass_pointer
        .events
        .iter()
        .rev()
        .find_map(|event| match event {
            PointerEvent::Pressed { button: PointerButton::Primary, pos, .. } => Some(*pos),
            _ => None,
        })
        .map(|at| pointer::press_winner(ctx, at, pass_pointer.press_targets).map(|(winner, _)| winner.owner));
    // A modal layer holds the wheel, except over the canvas it keeps open and inside its own family.
    let blocked = hover.is_some_and(|at| !sampler::wheel_allowed(ctx, at));
    let wake = PerViewport::<State>::with(ctx, viewport, |state| {
        state.last = std::mem::take(&mut state.this);
        state.shares.clear();
        if let Some(winner) = pressed_on {
            state.armed = winner.filter(|id| state.last.iter().any(|taker| taker.gated && taker.owner == *id));
        }
        if let Some(armed) = state.armed {
            let inside = state
                .last
                .iter()
                .find(|taker| taker.owner == armed)
                .zip(hover)
                .is_some_and(|(taker, at): (&WheelTaker<Id>, Pos2)| taker.rect.contains(at));
            if !inside {
                state.armed = None;
            }
        }
        // A press ends the gesture at once, the gap after its last event otherwise. A value control that owned it
        // hears the end as a zero-scroll `End` share; a scroll region and a canvas hear only the device's phases.
        let ended = state.gesture.end(if pressed_on.is_some() { f64::INFINITY } else { now });
        let ended = ended.filter(|owner| state.last.iter().any(|taker| taker.owner == *owner && taker.value));
        if let Some(owner) = ended {
            let mods = frame.as_ref().map_or(Mods::NONE, |frame| frame.mods);
            state.shares.push((owner, Wheel::new(Vec2::ZERO, mods, WheelUnit::Point, WheelPhase::End)));
        }
        if let (Some(frame), Some(at)) = (&frame, hover)
            && !blocked
        {
            let (scroll, unit, phase, mods, source) =
                wheel_in.unwrap_or((Vec2::ZERO, WheelUnit::Point, WheelPhase::Move, frame.mods, WheelSource::Unknown));
            let (touch_zoom, touch_rotation) = frame.multi_touch.map_or((1.0, 0.0), |touch| (touch.zoom, touch.rotation));
            // The pass's own pinch steps and zoom-modifier wheel deltas; the fingers' factor is `touch_zoom` alone.
            let zoom = frame.zoom;
            // A device's end reaches the gesture's owner even when it moves nothing.
            let device_end = wheel_in.is_some() && matches!(phase, WheelPhase::End | WheelPhase::Cancel);
            let moves = scroll != Vec2::ZERO || zoom != 1.0 || touch_zoom != 1.0 || frame.rotation != 0.0 || touch_rotation != 0.0;
            if moves || device_end {
                let share = Wheel::new(scroll, mods, unit, phase)
                    .source(source)
                    .zoom(zoom)
                    .touch_zoom(touch_zoom)
                    .rotation(frame.rotation)
                    .touch_rotation(touch_rotation)
                    .at(at);
                let focus = WheelFocus::new(focused).app_field(app_field).by_position(by_position);
                // The device's lines and pages become points of each taker: the notch is the scroll owner's to choose.
                for (owner, mut share) in split(&state.last, at, share, focus, state.armed, &mut state.gesture, now) {
                    let owner = scroll_owner_inside(&state.last, owner, at, share.scroll.y).unwrap_or(owner);
                    if unit == WheelUnit::Line {
                        share.lines = share.scroll;
                    }
                    if let Some(taker) = state.last.iter().find(|taker| taker.owner == owner) {
                        share.scroll = taker.points(share.scroll, unit);
                    }
                    state.shares.push((owner, share));
                }
            }
        }
        // One share per owner per pass: when the new input lands on the owner whose gesture this pass ended, the
        // two are one `Start` share, and the owner ends its last run as the new one begins.
        if let Some(owner) = ended
            && let Some(at) = state.shares.iter().skip(1).position(|(id, _)| *id == owner)
        {
            state.shares.remove(0);
            state.shares[at].1.phase = WheelPhase::Start;
        }
        // A value control that owns the running gesture hears its end without a timer of its own: a pass at the gap.
        let value_owner = state.gesture.owner.is_some_and(|owner| state.last.iter().any(|taker| taker.owner == owner && taker.value));
        (value_owner && state.gesture.runs(now)).then_some(state.gesture.last + GESTURE_GAP - now)
    });
    if let Some(seconds) = wake {
        repaint::request_after(ctx, viewport, seconds.max(0.0));
    }
}

/// The component's share of this pass's wheel, left in place.
pub(crate) fn peek(ctx: &Context, id: Id) -> Option<Wheel> {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| state.shares.iter().find(|(owner, _)| *owner == id).map(|share| share.1))
}

/// The component's share of this pass's wheel, once.
pub(crate) fn take(ctx: &Context, id: Id) -> Option<Wheel> {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| {
        let at = state.shares.iter().position(|(owner, _)| *owner == id)?;
        Some(state.shares.remove(at).1)
    })
}
