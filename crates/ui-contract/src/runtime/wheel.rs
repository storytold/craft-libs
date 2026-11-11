//! The wheel service (8.6): takers register their view rects, the begin hook splits the pass's wheel input
//! among them from last pass's registrations, and each taker reads its share once. The device reports lines and
//! pages; the service converts them to points with the notch each taker registered.

use crate::runtime::egui::{input, pass};
use crate::runtime::focus;
use crate::runtime::frame::PerViewport;
use crate::runtime::pointer;
use crate::runtime::sampler;
use craft_behavior::wheel::split;
use craft_behavior::{PointerButton, PointerEvent, Wheel, WheelPhase, WheelTaker, WheelUnit};
use egui::{Context, Id, Pos2, Rect, Vec2};

#[derive(Default)]
struct State {
    this: Vec<WheelTaker<Id>>,
    last: Vec<WheelTaker<Id>>,
    /// The last-pressed gated control while the pointer has stayed inside it since.
    armed: Option<Id>,
    shares: Vec<(Id, Wheel)>,
}

/// Registers a taker for next pass's split.
pub(crate) fn register(ctx: &Context, taker: WheelTaker<Id>) {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| state.this.push(taker));
}

/// The innermost non-gated taker of last pass whose view meets `rect`: where a reveal request goes.
pub(crate) fn nearest_scroll_owner(ctx: &Context, rect: Rect) -> Option<Id> {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| {
        state
            .last
            .iter()
            // Takers register outer first: of equal views the later one is the inner.
            .rev()
            .filter(|taker| !taker.gated && taker.rect.intersects(rect))
            .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
            .map(|taker| taker.owner)
    })
}

/// The begin hook (H7): rotates the registrations and splits this pass's wheel input.
pub(crate) fn begin(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    // Takes the pass's snapshot of the canvas a modal keeps open, which the input hook and the layer test read.
    sampler::hole_of_pass(ctx);
    let pass_pointer = pointer::pass_pointer(ctx);
    let frame = pass_pointer.frame.clone();
    let wheel_in = input::wheel_read(ctx, 1.0, 1.0);
    let focused = focus::focused_stop(ctx).map(|(owner, _)| owner);
    let hover = pass_pointer.hover.or(frame.as_ref().and_then(|frame| frame.pos));
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
    PerViewport::<State>::with(ctx, viewport, |state| {
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
        let (Some(frame), Some(at)) = (frame, hover) else { return };
        let (scroll, unit, phase, mods) = wheel_in.unwrap_or((Vec2::ZERO, WheelUnit::Point, WheelPhase::Move, frame.mods));
        if scroll == Vec2::ZERO && frame.zoom == 1.0 {
            return;
        }
        if blocked {
            return;
        }
        let share = Wheel::new(scroll, mods, unit, phase).zoom(frame.zoom).at(at);
        // The device's lines and pages become points of each taker: the notch is the scroll owner's to choose.
        let shares = split(&state.last, at, share, focused, state.armed);
        state.shares = shares
            .into_iter()
            .map(|(owner, mut share)| {
                if let Some(taker) = state.last.iter().find(|taker| taker.owner == owner) {
                    share.scroll = taker.points(share.scroll, unit);
                }
                (owner, share)
            })
            .collect();
    });
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
