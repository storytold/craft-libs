//! Steps 11 and 15, apply: the events written to the binding.

use super::measure::{MeasureIn, measure};
use crate::contract::cx::gesture_key;
use crate::contract::props::Resolved;
use crate::contract::values::Checked;
use crate::contract::{Component, Cx, ValueOf};
use crate::runtime::egui::sizing;
use crate::runtime::reads::{self, BindKey};
use craft_behavior::{Applied, CancelReason, EditEvent, Gesture, Mixed};
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
    let Some(mut bind) = comp.bind() else { return ValueEdit::default() };
    let key = bind.key(cx.core.id);
    let slot = gesture_key(cx.core.id);
    let (mut emitted, mut began, mut landing, mut last_update) = (None, false, None, None);
    let (mut given, mut written) = (false, false);
    let mut at = from;
    while at < cx.events.len() {
        match bind.checked(&mut cx.events[at]) {
            Checked::Keep => {}
            Checked::Cancel => {
                if let EditEvent::Commit { id, before, .. } = cx.events[at].clone() {
                    cx.events[at] = EditEvent::cancel(id, CancelReason::Refused, Some(before));
                    let dead = cx.core.press_held();
                    let gesture = cx.core.memory.shown::<Gesture<ValueOf<C>>>(slot);
                    *gesture = gesture.ended(dead);
                    if matches!(cx.events[at], EditEvent::Cancel { before: Some(Mixed::Mixed), .. }) {
                        cx.shown = None;
                        cx.mixed = true;
                    }
                }
            }
            Checked::Drop => {
                cx.events.remove(at);
                cx.applied = cx.events.len();
                continue;
            }
        }
        let event = &cx.events[at];
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
        let carries = matches!(event, EditEvent::Update { .. } | EditEvent::Commit { .. });
        if bind.is_given()
            && let EditEvent::Update { value, .. } | EditEvent::Commit { after: value, .. } = event
        {
            given |= bind.get() != Some(Mixed::Same(value.clone()));
        }
        written |= bind.apply(event) != Applied::Unchanged && carries;
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
    let gesture = cx.core.memory.shown::<Gesture<ValueOf<C>>>(slot);
    let (running, watched) = (gesture.is_running().then_some(gesture.id), gesture.watched);
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
        let ctx = cx.core.ctx.clone();
        let watch = cx.core.memory.shown::<crate::runtime::edit::RemovalWatch>(slot);
        match running {
            Some(id) => watch.arm(&ctx, id),
            None => watch.disarm(),
        }
    }
    if let Some(value) = landing {
        if bind.is_kept() {
            let id = cx.core.id;
            written |= cx.shown.as_deref() != Some(&value);
            bind.keep(&mut *cx.core.memory, id, value.clone());
        }
        cx.shown = Some(Arc::new(value));
        cx.mixed = false;
    }
    if let Some(value) = cx.shown.as_deref() {
        cx.validity = bind.validity(value);
    }
    let shown = cx.shown.as_deref() != shown_before.as_deref();
    ValueEdit { key, given, written, shown }
}

/// The rerun an edit asks for, so every value shown is current in the frame it changes: a given value that changed
/// (the framework cannot see who read it); a written value another component read earlier in the pass, in a real or a
/// measuring run; or a shown value whose size the earlier measures of this component took from the old value. A leaf
/// measures itself again at each room and proposal it was measured at and asks only when a size differs; a body,
/// whose measure runs its content, asks when any measuring run read the value.
pub(crate) fn rerun_after_edit<C: Component>(
    comp: &mut C,
    cx: &Cx<'_, C>,
    edit: ValueEdit,
    input: &MeasureIn<'_>,
    resolved: &Resolved<'_>,
    leaf: bool,
) {
    let ctx = &cx.core.ctx;
    if edit.given {
        sizing::request_discard(ctx, sizing::Rerun::Edit);
        return;
    }
    let Some(key) = edit.key.filter(|_| edit.written || edit.shown) else { return };
    let id = cx.core.id;
    let earlier = reads::earlier(ctx, key, id);
    if edit.written && earlier.other {
        return sizing::request_discard(ctx, sizing::Rerun::Edit);
    }
    let resized = if leaf {
        earlier.samples.iter().any(|sample| {
            let input = MeasureIn { ui: input.ui, env: input.env, proposal: sample.proposal, room: sample.room };
            measure(comp, &input, cx.style, resolved, cx.shown.as_deref(), Vec2::ZERO).0 != sample.took
        })
    } else {
        earlier.measured
    };
    if resized {
        sizing::request_discard(ctx, sizing::Rerun::Edit);
    }
}
