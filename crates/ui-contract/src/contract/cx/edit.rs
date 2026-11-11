//! The `Edit` trait: edit gestures and their events, with the press facts a gesture reads.

use super::context::{Cx, CxCore};
use crate::contract::{Component, EditResponse, Validity, ValueOf};
use crate::primitives::Adopted;
use craft_behavior::{CancelReason, EditEvent, Gesture, GesturePhase, Mixed, PressCancel, PressPhase, Value};
use egui::Id;
use std::sync::Arc;

impl CxCore<'_> {
    /// The resolved label as the name an edit event carries.
    #[doc(hidden)]
    pub fn label_arc(&self) -> Arc<str> {
        Arc::from(self.resolved.label)
    }

    /// A press on the component or one of its parts is down: the press that carries a gesture.
    pub(crate) fn press_held(&self) -> bool {
        self.press.phase.button_down() || self.results.values().any(|result| result.press.phase.button_down())
    }

    /// The reason a press of the component or one of its parts was cancelled in this pass.
    pub(crate) fn press_cancelled(&self) -> Option<PressCancel> {
        let reason = |phase: &PressPhase| match phase {
            PressPhase::Cancel { reason, .. } => Some(*reason),
            _ => None,
        };
        reason(&self.press.phase).or_else(|| self.results.values().find_map(|result| reason(&result.press.phase)))
    }
}

/// Values and edits (chapter 10). `value`, `validity`, `gesture`, `adopt` and `edit_response` take `&self`, so `paint`
/// can call them.
pub trait Edit {
    /// The component's value type.
    type Value: Value;

    /// The shown value: the running gesture's proposal, else the binding's. `None` for a mixed binding, a kept
    /// binding with nothing in it, and a component with no value.
    fn value(&self) -> Option<&Self::Value>;

    /// What the binding's check says about the shown value.
    fn validity(&self) -> Validity<Self::Value>;

    /// Where this pass's gesture stands.
    fn gesture(&self) -> Option<GesturePhase>;

    /// Maps each of the child's events onto a clone of this component's shown value and queues the resulting events, in
    /// the child's order, for step 15. A composite whose own value is mixed or not yet shown maps the edits onto
    /// `start` instead; its events then begin from a mixed value.
    fn adopt<V: Value>(&self, child: &EditResponse<V>, start: &Self::Value, map: impl Fn(&V, &mut Self::Value));

    /// The `EditResponse` a value component returns from `finish`.
    fn edit_response(&self) -> EditResponse<Self::Value>;

    /// Sets the running gesture's value, beginning a gesture if none runs and the press that carries it has had no
    /// gesture cancelled. A value equal to the gesture's last reported `Update` reports nothing again.
    fn propose(&mut self, v: Self::Value);

    /// A one-shot change outside a gesture.
    fn set(&mut self, v: Self::Value);

    /// Opens a gesture.
    fn begin(&mut self);

    /// Commits the proposed value.
    fn commit(&mut self);

    /// Cancels the running gesture.
    fn cancel(&mut self, reason: CancelReason);
}

/// The key of the gesture slot under a component's base id.
pub(crate) fn gesture_key(id: Id) -> Id {
    crate::runtime::memory::sub(id, "edit")
}

impl<C: Component> Cx<'_, C> {
    fn gesture_slot(&mut self) -> &mut Gesture<ValueOf<C>> {
        let id = gesture_key(self.core.id);
        self.core.memory.shown(id)
    }

    /// The value before an edit: the shown value, or mixed.
    fn before(&self) -> Mixed<ValueOf<C>> {
        match (&self.shown, self.mixed) {
            (Some(value), false) => Mixed::Same((**value).clone()),
            _ => Mixed::Mixed,
        }
    }

    /// An edit of a read-only or disabled component reaches nothing.
    fn refuses_edits(&self) -> bool {
        self.core.resolved.read_only || !self.core.resolved.enabled
    }

    /// Turns an adopted child event into the composite's own and records its gesture, as `begin`, `propose`, `commit`
    /// and `cancel` do for the component's own. The composite's gesture takes a fresh id; its `Commit` and `Cancel`
    /// carry the value the gesture began with, not the latest proposal. An event with no gesture of the composite's
    /// to end is dropped.
    pub(crate) fn absorb(&mut self, event: EditEvent<ValueOf<C>>) -> Option<EditEvent<ValueOf<C>>> {
        let dead = self.core.press_held();
        let fresh = crate::runtime::edit::next_gesture_id(&self.core.ctx);
        let slot = self.gesture_slot();
        match event {
            EditEvent::Begin { before, label, doc_key, .. } => {
                *slot = slot.ended(false);
                slot.id = fresh;
                slot.before = Some(before.clone());
                Some(EditEvent::begin(fresh, before, label, doc_key))
            }
            EditEvent::Update { value, .. } => {
                slot.proposal = Some(value.clone());
                Some(EditEvent::update(slot.id, value))
            }
            EditEvent::Commit { before, after, .. } => {
                let running = slot.is_running();
                let began = slot.before.clone().filter(|_| running);
                let id = if running { slot.id } else { fresh };
                *slot = slot.ended(false);
                Some(EditEvent::commit(id, began.unwrap_or(before), after))
            }
            EditEvent::Cancel { reason, .. } => {
                if !slot.is_running() {
                    return None;
                }
                let id = slot.id;
                let began = slot.before.clone().filter(|_| !reason.document_moved());
                *slot = slot.ended(dead);
                if let Some(before) = began.clone() {
                    self.shown = match before {
                        Mixed::Same(value) => Some(Arc::new(value)),
                        Mixed::Mixed => None,
                    };
                    self.mixed = self.shown.is_none();
                }
                Some(EditEvent::cancel(id, reason, began))
            }
            EditEvent::Set { before, after, label, doc_key, .. } => {
                *slot = slot.ended(false);
                Some(EditEvent::set(before, after, label, doc_key))
            }
            _ => None,
        }
    }
}

impl<C: Component> Edit for Cx<'_, C> {
    type Value = ValueOf<C>;

    fn value(&self) -> Option<&ValueOf<C>> {
        self.shown.as_deref()
    }

    fn validity(&self) -> Validity<ValueOf<C>> {
        self.validity.clone()
    }

    fn gesture(&self) -> Option<GesturePhase> {
        if let Some(phase @ (GesturePhase::Commit | GesturePhase::Cancel)) = self.events.last().and_then(EditEvent::phase) {
            return Some(phase);
        }
        if !self.core.memory.recall::<Gesture<ValueOf<C>>>(gesture_key(self.core.id)).is_running() {
            return None;
        }
        let began = self.events.iter().any(|event| matches!(event, EditEvent::Begin { .. }));
        let updated = self.events.iter().any(|event| matches!(event, EditEvent::Update { .. }));
        Some(if began && !updated { GesturePhase::Begin } else { GesturePhase::Update })
    }

    fn adopt<V: Value>(&self, child: &EditResponse<V>, start: &ValueOf<C>, map: impl Fn(&V, &mut ValueOf<C>)) {
        let before = self.before();
        for event in &child.events {
            let mut mine = match &before {
                Mixed::Same(value) => value.clone(),
                Mixed::Mixed => start.clone(),
            };
            if let Some(value) = event.value() {
                map(value, &mut mine);
            }
            let before = before.clone();
            let adopted = match event {
                EditEvent::Begin { id, label, doc_key, .. } => EditEvent::begin(*id, before, label.clone(), *doc_key),
                EditEvent::Update { id, .. } => EditEvent::update(*id, mine),
                EditEvent::Commit { id, .. } => EditEvent::commit(*id, before, mine),
                EditEvent::Cancel { id, reason, .. } => EditEvent::cancel(*id, *reason, Some(before)),
                EditEvent::Set { label, doc_key, .. } => EditEvent::set(before, mine, label.clone(), *doc_key),
                _ => continue,
            };
            self.core.paint.borrow_mut().adopted.push(Adopted::new(adopted));
        }
    }

    fn edit_response(&self) -> EditResponse<ValueOf<C>> {
        let mut response = EditResponse::new(self.core.response.clone(), self.events.iter().cloned().collect(), self.validity.clone());
        response.escaped = self.core.escaped || matches!(self.events.last(), Some(EditEvent::Cancel { reason: CancelReason::Escape, .. }));
        response.submitted = self.core.submitted;
        response.submit_mods = self.core.texts.as_ref().map_or(craft_keymap::Mods::NONE, |(_, text)| text.submit_mods());
        response.left_by_tab = self.core.texts.as_ref().and_then(|(_, text)| text.left_by_tab());
        response.unparsed = self.core.text_invalid;
        response.value_text = self.core.value_text.borrow().clone();
        response.shown = match (&self.shown, self.mixed) {
            (_, true) => Some(Mixed::Mixed),
            (Some(value), false) => Some(Mixed::Same(Arc::clone(value))),
            (None, false) => None,
        };
        response
    }

    fn propose(&mut self, v: ValueOf<C>) {
        if self.refuses_edits() {
            return;
        }
        let before = self.before();
        let (label, doc_key) = (self.core.label_arc(), self.core.resolved.doc_key);
        let fresh = crate::runtime::edit::next_gesture_id(&self.core.ctx);
        let slot = self.gesture_slot();
        // A gesture cancelled under its press stays cancelled until the release.
        if slot.dead {
            return;
        }
        let started = !slot.is_running();
        if started {
            slot.id = fresh;
            slot.before = Some(before.clone());
        }
        slot.proposal = Some(v.clone());
        let id = slot.id;
        let report = slot.reported.as_ref() != Some(&v);
        if report {
            slot.reported = Some(v.clone());
        }
        if started {
            self.events.push(EditEvent::begin(id, before, label, doc_key));
        }
        if report {
            self.events.push(EditEvent::update(id, v.clone()));
        }
        self.shown = Some(Arc::new(v));
        self.mixed = false;
    }

    fn set(&mut self, v: ValueOf<C>) {
        if self.refuses_edits() {
            return;
        }
        let before = self.before();
        let (label, doc_key) = (self.core.label_arc(), self.core.resolved.doc_key);
        self.events.push(EditEvent::set(before, v.clone(), label, doc_key));
        self.shown = Some(Arc::new(v));
        self.mixed = false;
    }

    fn begin(&mut self) {
        if self.refuses_edits() {
            return;
        }
        let before = self.before();
        let (label, doc_key) = (self.core.label_arc(), self.core.resolved.doc_key);
        let fresh = crate::runtime::edit::next_gesture_id(&self.core.ctx);
        let slot = self.gesture_slot();
        if !slot.is_running() && !slot.dead {
            slot.id = fresh;
            slot.before = Some(before.clone());
            let id = slot.id;
            self.events.push(EditEvent::begin(id, before, label, doc_key));
        }
    }

    fn commit(&mut self) {
        let slot = self.gesture_slot();
        if !slot.is_running() {
            return;
        }
        let ended = slot.ended(false);
        let running = std::mem::replace(slot, ended);
        let (id, Some(before)) = (running.id, running.before) else { return };
        match (running.proposal, before) {
            (Some(after), before) => self.events.push(EditEvent::commit(id, before, after)),
            (None, Mixed::Same(held)) => self.events.push(EditEvent::commit(id, Mixed::Same(held.clone()), held)),
            (None, Mixed::Mixed) => self.events.push(EditEvent::cancel(id, CancelReason::Dismissed, Some(Mixed::Mixed))),
        }
    }

    fn cancel(&mut self, reason: CancelReason) {
        let dead = self.core.press_held();
        let slot = self.gesture_slot();
        if !slot.is_running() {
            return;
        }
        let ended = slot.ended(dead);
        let running = std::mem::replace(slot, ended);
        let restored = running.before.clone().filter(|_| !reason.document_moved());
        self.events.push(EditEvent::cancel(running.id, reason, restored.clone()));
        if let Some(before) = restored {
            self.shown = match before {
                Mixed::Same(value) => Some(Arc::new(value)),
                Mixed::Mixed => None,
            };
            self.mixed = self.shown.is_none();
        }
    }
}
