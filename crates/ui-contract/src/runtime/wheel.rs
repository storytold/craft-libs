//! The wheel service (8.6): takers register their view rects, the begin hook splits the pass's wheel input
//! among them from last pass's registrations, and each taker reads its share once. The device reports lines and
//! pages; the service converts them to points with the notch each taker registered.

use crate::runtime::egui::{input, pass, repaint};
use crate::runtime::focus;
use crate::runtime::frame::PerViewport;
use crate::runtime::pointer;
use crate::runtime::sampler;
use craft_behavior::wheel::split;
use craft_behavior::{
    GESTURE_GAP, PointerButton, PointerEvent, Wheel, WheelEase, WheelFocus, WheelGesture, WheelPhase, WheelSource, WheelTaker, WheelUnit,
};
use craft_keymap::Mods;
use egui::{Context, Id, Pos2, Rect, Vec2};

#[derive(Default)]
pub(crate) struct State {
    this: Vec<WheelTaker<Id>>,
    last: Vec<WheelTaker<Id>>,
    /// The last-pressed gated control while the pointer has stayed inside it since.
    armed: Option<Id>,
    shares: Vec<(Id, Wheel)>,
    /// The wheel gesture the split tracks across passes.
    gesture: WheelGesture<Id>,
    /// The running gesture's gathered scroll in its owner's points, reset when the gesture closes.
    gathered: Vec2,
    /// Whether the owner marked the running gesture moved, reset when the gesture closes.
    moved: bool,
    /// The last gesture the device ended, which a momentum run of the same owner within `GESTURE_GAP` resumes.
    device_ended: Option<Ended>,
    /// The wheel ease each owner holds.
    eases: egui::IdMap<WheelEase>,
}

/// A gesture the device ended (fingers lifted), as a momentum run that follows it continues it.
#[derive(Clone, Copy)]
struct Ended {
    owner: Id,
    at: f64,
    gathered: Vec2,
    moved: bool,
}

/// Marks the wheel gesture `id` owns, while it runs, as moved.
pub(crate) fn mark_moved(ctx: &Context, id: Id) {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| {
        if state.gesture.owner == Some(id) {
            state.moved = true;
        }
        // The swipe the device ended in this pass goes on in its momentum run, which must see the mark too.
        if let Some(ended) = state.device_ended.as_mut().filter(|ended| ended.owner == id) {
            ended.moved = true;
        }
    });
}

/// Feeds `pan` to the wheel ease of `id` and returns this pass's eased step. The step is paid by the clock, so a
/// later pass of the same frame pays zero.
pub(crate) fn ease(ctx: &Context, id: Id, pan: Vec2) -> Vec2 {
    let viewport = pass::current_viewport(ctx);
    let now = crate::runtime::frame::clock(ctx);
    let (step, owes) = PerViewport::<State>::with(ctx, viewport, |state| {
        if pan == Vec2::ZERO && !state.eases.contains_key(&id) {
            return (Vec2::ZERO, false);
        }
        let ease = state.eases.entry(id).or_default();
        ease.feed(pan, now);
        (ease.step(now), ease.owes())
    });
    // The pass that fed the first notch asks for the next one itself: `begin` of this pass saw nothing owed.
    if owes {
        repaint::request_after(ctx, viewport, 0.0);
    }
    step
}

/// Drops the wheel ease of `id`.
pub(crate) fn stop_ease(ctx: &Context, id: Id) {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| {
        state.eases.remove(&id);
    });
}

impl State {
    /// Closes the gesture a press (`pressed`) or the gap at `now` ended, and returns the owner `WheelGesture::end`
    /// gave. A closed gesture's gathered scroll and mark go; a press also drops every ease and the ended swipe a
    /// momentum run would resume, and an ease whose owner registered no taker last pass goes.
    fn close_gesture(&mut self, pressed: bool, now: f64) -> Option<Id> {
        if pressed {
            self.eases.clear();
            self.device_ended = None;
        }
        let last = &self.last;
        self.eases.retain(|id, _| last.iter().any(|taker| taker.owner == *id));
        let open = self.gesture.last.is_finite();
        let ended = self.gesture.end(if pressed { f64::INFINITY } else { now });
        if open && !self.gesture.last.is_finite() {
            self.gathered = Vec2::ZERO;
            self.moved = false;
        }
        ended
    }

    /// Before a split at `now` of a wheel from `source`: when the split opens a new gesture, its gathered scroll and
    /// mark start at zero, or at the swipe's a momentum run within `GESTURE_GAP` of the swipe's device end goes on
    /// from. Returns the owner of that swipe, which the run resumes only if it goes to the same owner.
    fn open_gesture(&mut self, source: WheelSource, now: f64) -> Option<Id> {
        if self.gesture.runs(now) {
            return None;
        }
        let swipe = self.device_ended.filter(|ended| source == WheelSource::Momentum && now - ended.at < GESTURE_GAP);
        (self.gathered, self.moved) = swipe.map_or((Vec2::ZERO, false), |ended| (ended.gathered, ended.moved));
        swipe.map(|ended| ended.owner)
    }

    /// Adds `share`, in its owner's points, to the gesture it belongs to and gives it the gesture's gathered scroll
    /// and mark; a share of another owner than the gesture's gathers only itself. A point share cancels its owner's
    /// owed ease on each axis it moves: direct input wins over an older line tail, and only on its own axis.
    fn gather(&mut self, owner: Id, share: &mut Wheel) {
        if share.unit == WheelUnit::Point
            && let Some(ease) = self.eases.get_mut(&owner)
        {
            ease.cancel_axis(share.scroll.x != 0.0, share.scroll.y != 0.0);
        }
        if self.gesture.owner == Some(owner) {
            self.gathered += share.scroll;
            share.gathered = self.gathered;
            share.moved = self.moved;
        } else {
            share.gathered = share.scroll;
        }
    }

    /// The seconds until the pass the service needs next: a value control's gesture end at the gap, or the next
    /// frame while an ease owes motion.
    fn wake(&self, now: f64) -> Option<f64> {
        let value_owner = self.gesture.owner.is_some_and(|owner| self.last.iter().any(|taker| taker.owner == owner && taker.value));
        let gap = (value_owner && self.gesture.runs(now)).then_some(self.gesture.last + GESTURE_GAP - now);
        let ease = self.eases.values().any(WheelEase::owes).then_some(0.0);
        match (gap, ease) {
            (Some(gap), Some(ease)) => Some(gap.min(ease)),
            (gap, ease) => gap.or(ease),
        }
    }
}

/// Registers a taker for next pass's split.
pub(crate) fn register(ctx: &Context, taker: WheelTaker<Id>) {
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| register(ctx, taker));
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| state.this.push(taker));
}

/// The innermost scroll region of last pass whose view meets `rect`: where a reveal request or a page move goes from
/// outside every declared scroll owner. A taker that is no scroll region (a canvas) owns only what it declares.
pub(crate) fn nearest_scroll_owner(ctx: &Context, rect: Rect) -> Option<Id> {
    PerViewport::<State>::with(ctx, pass::current_viewport(ctx), |state| {
        state
            .last
            .iter()
            // Takers register outer first: of equal views the later one is the inner.
            .rev()
            .filter(|taker| taker.scrolls && !taker.gated && taker.rect.intersects(rect))
            .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
            .map(|taker| taker.owner)
    })
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
        let ended = state.close_gesture(pressed_on.is_some(), now);
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
            // egui's zoom delta is the fingers' factor while they are down, and otherwise the trackpad pinch with the
            // zoom-modifier wheel: the share's zoom is what is left once the fingers' factor is divided out.
            let zoom = if touch_zoom.is_finite() && touch_zoom > 0.0 { frame.zoom / touch_zoom } else { frame.zoom };
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
                let swipe = state.open_gesture(source, now);
                let shares = split(&state.last, at, share, focus, state.armed, &mut state.gesture, now);
                // A momentum run resumes its swipe only on the swipe's owner; any split drops the ended swipe.
                if swipe.is_some() && state.gesture.owner != swipe {
                    (state.gathered, state.moved) = (Vec2::ZERO, false);
                }
                state.device_ended = None;
                // The device's lines and pages become points of each taker: the notch is the scroll owner's to choose.
                for (owner, mut share) in shares {
                    if unit == WheelUnit::Line {
                        share.lines = share.scroll;
                    }
                    if let Some(taker) = state.last.iter().find(|taker| taker.owner == owner) {
                        share.scroll = taker.points(share.scroll, unit);
                    }
                    state.gather(owner, &mut share);
                    state.shares.push((owner, share));
                }
                if device_end {
                    let (gathered, moved) = (state.gathered, state.moved);
                    state.device_ended = state.gesture.owner.map(|owner| Ended { owner, at: now, gathered, moved });
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
        // An owed ease pays in the passes that follow.
        state.wake(now)
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
