//! Step 4, value: the shown value and its copies.

use crate::contract::cx::gesture_key;
use crate::contract::values::reuse_arc;
use crate::contract::{Component, ValueOf};
use crate::runtime::commands::{CommandKind, ResolvedCommand};
use crate::runtime::memory::MemoryStage;
use craft_behavior::{CancelReason, EditEvent, Gesture, Mixed, Value};
use egui::{Context, Id};
use std::sync::Arc;

/// The shown copy of a component's value, in shown memory.
pub(crate) struct ShownCopy<V>(pub Option<Arc<V>>);

impl<V> Clone for ShownCopy<V> {
    fn clone(&self) -> Self {
        ShownCopy(self.0.clone())
    }
}

impl<V> Default for ShownCopy<V> {
    fn default() -> Self {
        ShownCopy(None)
    }
}

/// What step 4 reads: the shown value, whether the binding is mixed, and the `Cancel` of a gesture step 4 ended.
pub(crate) struct ValueStep<V> {
    pub value: Option<Arc<V>>,
    pub mixed: bool,
    pub cancelled: Option<EditEvent<V>>,
}

/// The held copy when `v` equals it, else a new copy.
fn reuse_or_new<V: Value>(held: &Option<Arc<V>>, v: V) -> Arc<V> {
    match held {
        Some(held) if **held == v => held.clone(),
        _ => Arc::new(v),
    }
}

/// The value of a `bool` component that has no binding of its own and is bound to a toggle or radio command: the
/// command's `checked` state, as a given value (10.4).
fn command_given<V: Value>(command: Option<&ResolvedCommand>) -> Option<V> {
    let command = command.filter(|command| matches!(command.kind, CommandKind::Toggle | CommandKind::Radio { .. }))?;
    let checked = command.checked?;
    (&checked as &dyn std::any::Any).downcast_ref::<V>().cloned()
}

/// The pass a component's value step last ran in.
#[derive(Clone, Default)]
struct LastDrawn(u64);

/// Step 4, value: the shown value is the running gesture's proposal, else the binding's value. The one shown copy is
/// replaced only when the binding differs from it. A gesture ends here when `cancel_all` ran, or when the app changed the
/// value under it.
pub(crate) fn value<C: Component>(comp: &mut C, stage: &mut MemoryStage, id: Id, command: Option<&ResolvedCommand>) -> ValueStep<ValueOf<C>> {
    type V<C> = ValueOf<C>;
    let held = stage.shown::<ShownCopy<V<C>>>(id).0.clone();
    let slot = gesture_key(id);
    let (serial, all_reason) = crate::runtime::edit::latest(stage.ctx());
    let mut cancelled = None;
    // A component not drawn for a pass loses its press at the end hook; its running gesture ends here, the first pass it is
    // drawn again.
    let pass_nr = crate::runtime::pointer::pass_of(stage.ctx());
    let removed = std::mem::replace(&mut stage.shown::<LastDrawn>(slot).0, pass_nr).saturating_add(1) < pass_nr;
    if stage.shown::<Gesture<V<C>>>(slot).is_running() {
        // `(given, value)` of a single-valued binding the app owns.
        let host = comp.bind().and_then(|bind| match bind.get() {
            Some(Mixed::Same(value)) if !bind.is_kept() => Some((bind.is_given(), value)),
            _ => None,
        });
        let gesture = stage.shown::<Gesture<V<C>>>(slot);
        let reason = match all_reason.filter(|_| serial > gesture.cancel_seen) {
            _ if removed => Some(CancelReason::Removed),
            Some(reason) => Some(reason),
            None => {
                // A given source: the app changed it when it differs from last pass's given value and from the last value
                // emitted, so an app that applies only `Commit` keeps its drag. Any other source: it differs from the
                // last value step 11 wrote.
                let changed = match &host {
                    Some((true, value)) => {
                        gesture.last_given.as_ref().is_some_and(|last| last != value)
                            && gesture.emitted.as_ref().is_some_and(|emitted| !comp.same_value(emitted, value))
                    }
                    Some((false, value)) => gesture.emitted.as_ref().is_some_and(|emitted| !comp.same_value(emitted, value)),
                    None => false,
                };
                changed.then_some(CancelReason::HostChanged)
            }
        };
        if let (Some((true, value)), None) = (&host, reason) {
            gesture.last_given = Some(value.clone());
        }
        if let Some(reason) = reason {
            // The document moved under the gesture, so there is nothing to restore.
            let before = gesture.before.clone().filter(|_| !reason.document_moved());
            cancelled = Some(EditEvent::cancel(gesture.id, reason, before));
            *gesture = gesture.ended(true);
        }
    }
    stage.shown::<Gesture<V<C>>>(slot).cancel_seen = serial;
    let proposal = stage.shown::<Gesture<V<C>>>(slot).proposal.clone();
    // A binding equal to the held copy is not read: no clone, no new copy.
    let bound = comp.bind().map(|bind| {
        let unchanged = held.as_deref().is_some_and(|held| bind.shown_matches(held));
        let got = if unchanged { None } else { bind.get() };
        (unchanged, got, bind.is_kept())
    });
    let mut out = ValueStep { value: None, mixed: false, cancelled };
    match (proposal, bound) {
        (Some(proposal), _) => out.value = Some(reuse_or_new(&held, proposal)),
        (None, Some((true, ..))) => out.value = held.clone(),
        (None, Some((false, Some(Mixed::Same(value)), ..))) => out.value = Some(Arc::new(value)),
        (None, Some((false, Some(Mixed::Mixed), ..))) => out.mixed = true,
        // A kept binding with a `SaveKey` reads the saved tier, one without reads session memory under the id.
        (None, Some((false, None, _))) => {
            out.value = comp.bind().and_then(|bind| bind.kept(stage, id, &held));
        }
        (None, None) => out.value = command_given::<V<C>>(command).map(|value| reuse_arc(&held, &value)),
    }
    let shown = &mut stage.shown::<ShownCopy<V<C>>>(id).0;
    let same = match (&*shown, &out.value) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    if !same {
        *shown = out.value.clone();
    }
    out
}

/// The shown value for measuring and faces: reads the binding, applies nothing, opens no stage. A kept binding reads
/// shown memory.
/// Returns the value and whether the binding is mixed.
pub(crate) fn peek_value<C: Component>(comp: &mut C, ctx: &Context, id: Id, command: Option<&ResolvedCommand>) -> (Option<Arc<ValueOf<C>>>, bool) {
    let held = MemoryStage::peek::<ShownCopy<ValueOf<C>>>(ctx, id).0;
    let Some(bind) = comp.bind() else {
        let given = command_given::<ValueOf<C>>(command).map(|value| reuse_arc(&held, &value));
        return (given.or(held), false);
    };
    if held.as_deref().is_some_and(|held| bind.shown_matches(held)) {
        return (held, false);
    }
    match bind.get() {
        Some(Mixed::Same(value)) => (Some(Arc::new(value)), false),
        Some(Mixed::Mixed) => (None, true),
        None => (held, false),
    }
}
