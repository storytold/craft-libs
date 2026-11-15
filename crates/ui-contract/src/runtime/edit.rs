//! The pass's edit events and edit-command routing: the context-free calls an app makes around edits.

use super::egui::repaint;
use crate::runtime::memory::MemoryStage;
use crate::runtime::pass_state;
use craft_behavior::{CancelReason, Gesture, GestureId, Value};
use egui::{Context, Id, ViewportId};

/// The latest `cancel_all`: a serial that grows with each call, and why.
#[derive(Clone, Copy, Default)]
pub(crate) struct CancelAll {
    serial: u64,
    reason: Option<CancelReason>,
}

/// Cancels every running gesture at its next step 4, which then stays cancelled under its press. The app's undo and
/// redo commands call it before they change the document, so a drag ends with `reason` instead of resuming over the
/// changed value.
pub fn cancel_all(ctx: &Context, reason: CancelReason) {
    pass_state::with::<CancelAll, _>(ctx, |all| {
        all.serial += 1;
        all.reason = Some(reason);
    });
    repaint::request(ctx);
}

/// The serial and reason of the latest `cancel_all`; serial 0 before the first.
pub(crate) fn latest(ctx: &Context) -> (u64, Option<CancelReason>) {
    pass_state::with::<CancelAll, _>(ctx, |all| (all.serial, all.reason))
}

/// The note the component `id` set on its running gesture with [`Cx::propose_with`](crate::contract::Cx::propose_with),
/// when it is an `N`; `None` while no gesture of the component runs. `V` is the component's value type, under which
/// its gesture is kept. A reader outside the component's pass, drawn before or after it, reads the same note.
pub fn gesture_note<V: Value, N: Clone + 'static>(ctx: &Context, id: Id) -> Option<N> {
    MemoryStage::peek_with::<Gesture<V>, _>(ctx, crate::contract::gesture_key(id), |gesture| {
        if !gesture.is_running() {
            return None;
        }
        gesture.note.as_ref()?.get::<N>().cloned()
    })
}

/// Whether a member of the arena root `arena` (a canvas control in a canvas, say) has a running gesture, in
/// `viewport`: begun and not yet committed, cancelled or removed. It reads each member's own gesture record, so a
/// reader before or after the arena's pass reads the same fact.
pub fn member_gesture_running(ctx: &Context, viewport: ViewportId, arena: Id) -> bool {
    crate::primitives::sensing::members(ctx, viewport, crate::runtime::env::ArenaId::new(arena))
        .into_iter()
        .any(|member| MemoryStage::peek_with::<RemovalWatch, _>(ctx, crate::contract::gesture_key(member), RemovalWatch::armed))
}

/// The newest gesture id handed out in this context, and how many times a component has reported its gesture running
/// at an apply step.
#[derive(Clone, Copy, Default)]
pub(crate) struct Gestures {
    newest: GestureId,
    running: u64,
}

/// A gesture id no gesture of this context has had: ids grow in the order gestures begin, across components and
/// across a component's eviction, as `GestureHost` assumes.
pub fn next_gesture_id(ctx: &Context) -> GestureId {
    pass_state::with::<Gestures, _>(ctx, |gestures| {
        gestures.newest = gestures.newest.next();
        gestures.newest
    })
}

/// Notes that a component's gesture runs after its apply step, for [`watch_gestures`].
pub(crate) fn note_running(ctx: &Context) {
    if !crate::runtime::measure::active(ctx) {
        pass_state::with::<Gestures, _>(ctx, |gestures| gestures.running += 1);
    }
}

/// Runs `body` and says whether a gesture of a component shown inside it was running after the component's apply
/// step: a drag or a typing run under way. A container that records its body's edits as undo steps waits for the
/// gesture to end, so a drag is one step. A measuring run sees none: gestures run in real runs.
pub fn watch_gestures<R>(ctx: &Context, body: impl FnOnce() -> R) -> (R, bool) {
    if crate::runtime::measure::active(ctx) {
        return (body(), false);
    }
    let before = pass_state::with::<Gestures, _>(ctx, |gestures| gestures.running);
    let out = body();
    let after = pass_state::with::<Gestures, _>(ctx, |gestures| gestures.running);
    (out, after != before)
}

/// Notes that the component running gesture `id` was evicted: the gesture ends with `CancelReason::Removed`, which the
/// edit hosts read from `FrameOutput::removed_gestures`.
pub(crate) fn note_removed(ctx: &Context, id: GestureId) {
    super::frame::note_removed_gesture(ctx, id);
    repaint::request(ctx);
}

/// The removal watch of a running gesture, a shown slot beside it under the same owner. Its drop is the dispose hook of
/// the owner's eviction: only the slot the runtime moves through the stage is armed, so a clone dropped by a read
/// notes nothing.
#[derive(Default)]
#[doc(hidden)]
pub struct RemovalWatch {
    armed: Option<(Context, GestureId)>,
}

impl RemovalWatch {
    #[doc(hidden)]
    pub fn arm(&mut self, ctx: &Context, id: GestureId) {
        self.armed = Some((ctx.clone(), id));
    }

    #[doc(hidden)]
    pub fn disarm(&mut self) {
        self.armed = None;
    }

    /// The gesture it watches runs.
    pub(crate) fn armed(&self) -> bool {
        self.armed.is_some()
    }
}

impl Clone for RemovalWatch {
    fn clone(&self) -> Self {
        RemovalWatch::default()
    }
}

impl Drop for RemovalWatch {
    fn drop(&mut self) {
        if let Some((ctx, id)) = self.armed.take() {
            note_removed(&ctx, id);
        }
    }
}
