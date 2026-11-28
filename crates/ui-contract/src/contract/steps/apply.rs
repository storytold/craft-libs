//! Steps 11 and 15, apply: the events written to the binding.

use super::measure::{MeasureIn, measure};
use crate::contract::cx::{CxCore, gesture_key};
use crate::contract::props::Resolved;
use crate::contract::run::ErasedPass;
use crate::contract::values::Checked;
use crate::contract::{BindMut, Component, Cx, Validity};
use crate::runtime::egui::sizing;
use crate::runtime::reads::{self, BindKey};
use crate::style::StyleView;
use craft_behavior::{Applied, CancelReason, EditEvent, Gesture, Mixed, Value};
use egui::Vec2;
use std::sync::Arc;

/// What an apply step did to the component's value, for the rerun decision of [`rerun_after_edit`].
#[derive(Default)]
pub(crate) struct ValueEdit {
    /// The binding's key in the read record; none for a given binding.
    key: Option<BindKey>,
    /// A given binding's value changed: the app applies it, and the framework cannot see who read it.
    given: bool,
    /// The value in the binding's place or kept slot changed.
    written: bool,
    /// The component's shown value changed.
    shown: bool,
}

/// Steps 11 and 15, apply: the events not yet applied are checked and written to the binding. A read-only component refuses
/// every one.
pub(crate) fn apply_values<C: Component>(comp: &mut C, cx: &mut Cx<'_, C>) -> ValueEdit {
    let from = cx.applied;
    cx.applied = cx.events.len();
    if cx.core.resolved.read_only {
        // A gesture's end still reaches the app; only events that carry a value are refused.
        let mut at = 0;
        cx.events.retain(|event| {
            let keep = at < from || matches!(event, EditEvent::Cancel { .. });
            at += 1;
            keep
        });
        cx.applied = cx.events.len();
        return ValueEdit::default();
    }
    let shown_before = cx.shown.clone();
    let Some(bind) = comp.bind() else { return ValueEdit::default() };
    let state =
        BindingState { events: &mut cx.events, applied: &mut cx.applied, shown: &mut cx.shown, mixed: &mut cx.mixed, validity: &mut cx.validity };
    apply_binding(bind, cx.core, state, from, shown_before)
}

/// Mutable value state for the apply step, shared by components that bind the same value type.
struct BindingState<'a, V> {
    events: &'a mut Vec<EditEvent<V>>,
    applied: &'a mut usize,
    shown: &'a mut Option<Arc<V>>,
    mixed: &'a mut bool,
    validity: &'a mut Validity<V>,
}

fn apply_binding<V: Value>(
    mut bind: BindMut<'_, V>,
    core: &mut CxCore<'_>,
    state: BindingState<'_, V>,
    from: usize,
    shown_before: Option<Arc<V>>,
) -> ValueEdit {
    let key = bind.key(core.id);
    let slot = gesture_key(core.id);
    let (mut emitted, mut began, mut landing, mut last_update) = (None, false, None, None);
    let (mut given, mut written) = (false, false);
    let mut at = from;
    while at < state.events.len() {
        match bind.checked(&mut state.events[at]) {
            Checked::Keep => {}
            Checked::Cancel => {
                if let EditEvent::Commit { id, before, .. } = state.events[at].clone() {
                    state.events[at] = EditEvent::cancel(id, CancelReason::Refused, Some(before));
                    let dead = core.press_held();
                    let gesture = core.memory.shown::<Gesture<V>>(slot);
                    *gesture = gesture.ended(dead);
                    if matches!(state.events[at], EditEvent::Cancel { before: Some(Mixed::Mixed), .. }) {
                        *state.shown = None;
                        *state.mixed = true;
                    }
                }
            }
            Checked::Drop => {
                state.events.remove(at);
                *state.applied = state.events.len();
                continue;
            }
        }
        let event = &state.events[at];
        match event {
            EditEvent::Begin { before: Mixed::Same(value), .. } => {
                emitted = Some(value.clone());
                began = true;
            }
            EditEvent::Update { value, .. } => {
                last_update = Some(value.clone());
                if bind.writes(event) || bind.is_given() {
                    emitted = Some(value.clone());
                }
            }
            _ => {}
        }
        // Every event that changes the value counts: a proposal, commit or set, and a cancel that restores.
        if bind.is_given()
            && let Some(value) = event.landed()
        {
            given |= bind.get() != Some(Mixed::Same(value.clone()));
        }
        written |= bind.apply(event) != Applied::Unchanged;
        if let Some(value) = event.landed() {
            landing = Some(value.clone());
        }
        at += 1;
    }
    // The given value of the pass a gesture began in, for step 4's host-change comparison next pass.
    let given_now = if began && bind.is_given() {
        match bind.get() {
            Some(Mixed::Same(value)) => Some(value),
            _ => None,
        }
    } else {
        None
    };
    let gesture = core.memory.shown::<Gesture<V>>(slot);
    let (running, watched) = (gesture.is_running().then_some(gesture.id), gesture.watched);
    if running.is_some() {
        crate::runtime::edit::note_running(&core.ctx);
    }
    if gesture.is_running() {
        gesture.emitted = emitted.or(gesture.emitted.take());
        gesture.last_given = given_now.or(gesture.last_given.take());
        // A value the check replaced is the gesture's proposal, so the next pass shows it.
        if let Some(value) = last_update {
            gesture.proposal = Some(value);
        }
    }
    // The owner's eviction while the gesture runs ends it as removed (7.2).
    if running.is_some() || watched {
        gesture.watched = running.is_some();
        let ctx = core.ctx.clone();
        let watch = core.memory.shown::<crate::runtime::edit::RemovalWatch>(slot);
        match running {
            Some(id) => watch.arm(&ctx, id),
            None => watch.disarm(),
        }
    }
    if let Some(value) = landing {
        if bind.is_kept() {
            let id = core.id;
            written |= state.shown.as_deref() != Some(&value);
            bind.keep(&mut *core.memory, id, value.clone());
        }
        *state.shown = Some(Arc::new(value));
        *state.mixed = false;
    }
    if let Some(value) = state.shown.as_deref() {
        *state.validity = bind.validity(value);
    }
    let shown = state.shown.as_deref() != shown_before.as_deref();
    ValueEdit { key, given, written, shown }
}

/// The rerun an edit asks for, so every value shown is current in the frame it changes: a written value another
/// component read earlier in the pass, in a real or a measuring run; or a shown value whose size the earlier measures
/// of this component took from the old value. A leaf measures itself again at each room and proposal it was measured
/// at and asks only when a size differs; a body, whose measure runs its content, asks when any measuring run read the
/// value. A given value has no read record, so its edit asks for no rerun: the component shows the edit itself, and
/// the app's other views of the value show it in the next frame, which the edit requests: the app writes the edit into
/// its model, and the memoized views whose props hold that model run.
pub(crate) fn rerun_after_edit(
    p: &mut dyn ErasedPass<'_>,
    style: &dyn StyleView,
    edit: ValueEdit,
    input: &MeasureIn<'_>,
    resolved: &Resolved<'_>,
    leaf: bool,
) {
    if edit.given {
        p.core_ref().ctx.request_repaint();
        return;
    }
    let Some(key) = edit.key.filter(|_| edit.written || edit.shown) else { return };
    let (ctx, id) = (p.core_ref().ctx.clone(), p.core_ref().id);
    if edit.written {
        crate::runtime::lazy::note_edit(&ctx, key);
    }
    let earlier = reads::earlier(&ctx, key, id);
    if edit.written && earlier.other {
        log::debug!(target: sizing::RERUN_LOG, "{id:?} wrote {key:?}, which another component read earlier in the pass");
        return sizing::request_discard(&ctx, sizing::Rerun::Edit);
    }
    let resized = if leaf {
        earlier.samples.iter().any(|sample| {
            let input = MeasureIn { ui: input.ui, env: input.env, proposal: sample.proposal, room: sample.room };
            measure(style, &input, p.measure_content(&input, resolved).0, Vec2::ZERO, resolved) != sample.took
        })
    } else {
        earlier.measured
    };
    if resized {
        log::debug!(target: sizing::RERUN_LOG, "{id:?} changed {key:?}, which an earlier measure in the pass took its size from");
        sizing::request_discard(&ctx, sizing::Rerun::Edit);
    }
}
