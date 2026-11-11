//! The value core of chapter 10: the bound every value type meets, `Mixed`, edit events and the list a pass's events
//! fold into.

use crate::doc_key::DocKey;
use smallvec::SmallVec;
use std::sync::Arc;

/// What a bound value must be: cloneable, comparable and shareable across threads.
pub trait Value: Clone + PartialEq + Send + Sync + 'static {}

impl<T: Clone + PartialEq + Send + Sync + 'static> Value for T {}

/// A value that may differ across a multi-selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mixed<T> {
    /// Every selected item holds this value.
    Same(T),
    /// The selected items differ.
    Mixed,
}

impl<T> Mixed<T> {
    /// The shared value, `None` while the items differ.
    pub const fn same(&self) -> Option<&T> {
        match self {
            Mixed::Same(value) => Some(value),
            Mixed::Mixed => None,
        }
    }

    /// The shared value, or `start` while the items differ.
    pub const fn or<'a>(&'a self, start: &'a T) -> &'a T {
        match self {
            Mixed::Same(value) => value,
            Mixed::Mixed => start,
        }
    }

    /// Every part of `values` as a shared value: the per-part form of a composite value no part of which differs.
    pub fn each<const N: usize>(values: [T; N]) -> [Mixed<T>; N] {
        values.map(Mixed::Same)
    }
}

impl<T, const N: usize> Mixed<[Mixed<T>; N]> {
    /// The per-part form of a composite value: a whole `Mixed` binding means every part differs.
    pub fn parts(self) -> [Mixed<T>; N] {
        match self {
            Mixed::Same(parts) => parts,
            Mixed::Mixed => std::array::from_fn(|_| Mixed::Mixed),
        }
    }
}

impl<T> From<T> for Mixed<T> {
    fn from(value: T) -> Self {
        Mixed::Same(value)
    }
}

/// Names a gesture; ids grow in the order the gestures begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub struct GestureId(pub u64);

impl GestureId {
    /// The id numbered `n`.
    pub const fn new(n: u64) -> GestureId {
        GestureId(n)
    }

    /// The id of the gesture after this one.
    pub const fn next(self) -> GestureId {
        GestureId(self.0 + 1)
    }
}

/// Why a gesture ended without a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CancelReason {
    /// The check refused the commit.
    Refused,
    /// The user dismissed the overlay that carried the gesture.
    Dismissed,
    /// Escape.
    Escape,
    /// Focus left the component.
    Blur,
    /// The component was removed while the gesture ran.
    Removed,
    /// The app changed the value mid-gesture.
    HostChanged,
    /// An undo or redo command ran while the gesture was open.
    UndoDuringGesture,
}

impl CancelReason {
    /// The document changed under the gesture, so the gesture's own edits stay and a revert would also take back the
    /// change.
    pub const fn document_moved(self) -> bool {
        matches!(self, CancelReason::HostChanged | CancelReason::UndoDuringGesture)
    }
}

/// What applying an event did to a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Applied {
    /// The value changed.
    Changed,
    /// The value was already the event's.
    Unchanged,
    /// A cancel put the value from before the gesture back.
    Restored,
}

/// When a write-through source is written: on every proposal of a gesture or only on its commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum EditPolicy {
    /// Every proposal.
    #[default]
    WriteThrough,
    /// The commit only.
    OnCommit,
}

/// What a check says about a value. `M` is the message type: craft-ui exports the alias with its own `Message`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Validity<T, M> {
    /// Accepted as it is.
    Ok,
    /// Accepted as this value instead.
    Fix(T),
    /// Accepted with a warning.
    Warn(M),
    /// Not accepted; a commit is refused.
    Refuse(M),
}

/// Where a gesture stands in one pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GesturePhase {
    /// The gesture began in this pass and has no value yet.
    Begin,
    /// The gesture runs.
    Update,
    /// The gesture committed in this pass.
    Commit,
    /// The gesture was cancelled in this pass.
    Cancel,
}

/// One step of an edit. A pass raises several; [`EditEvents`] holds them in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EditEvent<T> {
    /// A gesture began. It names the undo step.
    #[non_exhaustive]
    Begin {
        /// The gesture.
        id: GestureId,
        /// The value when it began.
        before: Mixed<T>,
        /// The resolved label of the component.
        label: Arc<str>,
        /// The component's document key.
        doc_key: Option<DocKey>,
    },
    /// A proposed value of the running gesture.
    #[non_exhaustive]
    Update {
        /// The gesture.
        id: GestureId,
        /// The proposed value.
        value: T,
    },
    /// The gesture ended with this value.
    #[non_exhaustive]
    Commit {
        /// The gesture.
        id: GestureId,
        /// The value when it began.
        before: Mixed<T>,
        /// The committed value.
        after: T,
    },
    /// The gesture ended without a commit.
    #[non_exhaustive]
    Cancel {
        /// The gesture.
        id: GestureId,
        /// Why it ended.
        reason: CancelReason,
        /// The value when it began.
        before: Option<Mixed<T>>,
    },
    /// A value that arrived as one step, or a gesture that began and committed in one pass. It names itself the way
    /// a `Begin` does. It belongs to no gesture: each `Set` is its own undo step.
    #[non_exhaustive]
    Set {
        /// The value before.
        before: Mixed<T>,
        /// The new value.
        after: T,
        /// The resolved label of the component.
        label: Arc<str>,
        /// The component's document key.
        doc_key: Option<DocKey>,
    },
}

impl<T> EditEvent<T> {
    /// A gesture began.
    pub fn begin(id: GestureId, before: Mixed<T>, label: Arc<str>, doc_key: Option<DocKey>) -> Self {
        EditEvent::Begin { id, before, label, doc_key }
    }

    /// A proposed value of the running gesture.
    pub fn update(id: GestureId, value: T) -> Self {
        EditEvent::Update { id, value }
    }

    /// The gesture committed `after`.
    pub fn commit(id: GestureId, before: Mixed<T>, after: T) -> Self {
        EditEvent::Commit { id, before, after }
    }

    /// The gesture ended without a commit.
    pub fn cancel(id: GestureId, reason: CancelReason, before: Option<Mixed<T>>) -> Self {
        EditEvent::Cancel { id, reason, before }
    }

    /// A one-step change, named the way a `Begin` names its gesture.
    pub fn set(before: Mixed<T>, after: T, label: Arc<str>, doc_key: Option<DocKey>) -> Self {
        EditEvent::Set { before, after, label, doc_key }
    }

    /// The gesture the event belongs to; a `Set` belongs to none.
    pub fn id(&self) -> Option<GestureId> {
        match self {
            EditEvent::Begin { id, .. } | EditEvent::Update { id, .. } | EditEvent::Commit { id, .. } | EditEvent::Cancel { id, .. } => Some(*id),
            EditEvent::Set { .. } => None,
        }
    }

    /// Maps the carried values and keeps the gesture.
    pub fn map<U>(self, f: impl Fn(T) -> U) -> EditEvent<U> {
        let mixed = |m: Mixed<T>| match m {
            Mixed::Same(value) => Mixed::Same(f(value)),
            Mixed::Mixed => Mixed::Mixed,
        };
        match self {
            EditEvent::Begin { id, before, label, doc_key } => EditEvent::Begin { id, before: mixed(before), label, doc_key },
            EditEvent::Update { id, value } => EditEvent::Update { id, value: f(value) },
            EditEvent::Commit { id, before, after } => EditEvent::Commit { id, before: mixed(before), after: f(after) },
            EditEvent::Cancel { id, reason, before } => EditEvent::Cancel { id, reason, before: before.map(mixed) },
            EditEvent::Set { before, after, label, doc_key } => EditEvent::Set { before: mixed(before), after: f(after), label, doc_key },
        }
    }

    /// Maps the carried values and keeps the gesture; `None` when `f` has no value for one of them.
    pub fn filter_map<U>(self, f: impl Fn(T) -> Option<U>) -> Option<EditEvent<U>> {
        let mixed = |m: Mixed<T>| match m {
            Mixed::Same(value) => f(value).map(Mixed::Same),
            Mixed::Mixed => Some(Mixed::Mixed),
        };
        Some(match self {
            EditEvent::Begin { id, before, label, doc_key } => EditEvent::Begin { id, before: mixed(before)?, label, doc_key },
            EditEvent::Update { id, value } => EditEvent::Update { id, value: f(value)? },
            EditEvent::Commit { id, before, after } => EditEvent::Commit { id, before: mixed(before)?, after: f(after)? },
            EditEvent::Cancel { id, reason, before } => {
                EditEvent::Cancel { id, reason, before: before.map(mixed).map_or(Some(None), |b| b.map(Some))? }
            }
            EditEvent::Set { before, after, label, doc_key } => EditEvent::Set { before: mixed(before)?, after: f(after)?, label, doc_key },
        })
    }

    /// The value the event carries: a proposal, a commit or a set.
    pub fn value(&self) -> Option<&T> {
        match self {
            EditEvent::Update { value, .. } => Some(value),
            EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(after),
            EditEvent::Begin { .. } | EditEvent::Cancel { .. } => None,
        }
    }

    /// The value the binding holds after the event: the carried value, or the value a cancel restores.
    pub fn landed(&self) -> Option<&T> {
        match self {
            EditEvent::Cancel { before: Some(Mixed::Same(before)), .. } => Some(before),
            event => event.value(),
        }
    }

    /// Where the gesture stands after this event; a `Set` is not a gesture.
    pub fn phase(&self) -> Option<GesturePhase> {
        match self {
            EditEvent::Begin { .. } => Some(GesturePhase::Begin),
            EditEvent::Update { .. } => Some(GesturePhase::Update),
            EditEvent::Commit { .. } => Some(GesturePhase::Commit),
            EditEvent::Cancel { .. } => Some(GesturePhase::Cancel),
            EditEvent::Set { .. } => None,
        }
    }
}

impl<T: Value> EditEvent<T> {
    /// Applies the event to `v`: a proposal, commit or set writes its value, a cancel restores the value from before
    /// the gesture.
    pub fn apply(&self, v: &mut T) -> Applied {
        match self {
            EditEvent::Update { value: new, .. } | EditEvent::Commit { after: new, .. } | EditEvent::Set { after: new, .. } => {
                if *v == *new {
                    Applied::Unchanged
                } else {
                    *v = new.clone();
                    Applied::Changed
                }
            }
            EditEvent::Cancel { before: Some(Mixed::Same(before)), .. } => {
                *v = before.clone();
                Applied::Restored
            }
            EditEvent::Begin { .. } | EditEvent::Cancel { .. } => Applied::Unchanged,
        }
    }

    /// Applies the event to a value that may be mixed: the written value is a concrete one.
    pub fn apply_mixed(&self, v: &mut Mixed<T>) -> Applied {
        match (self, self.value()) {
            (EditEvent::Cancel { before: Some(before), .. }, _) => {
                *v = before.clone();
                Applied::Restored
            }
            (_, Some(new)) if !matches!(v, Mixed::Same(held) if held == new) => {
                *v = Mixed::Same(new.clone());
                Applied::Changed
            }
            _ => Applied::Unchanged,
        }
    }
}

/// The edit events of one pass, in order. `push` folds what loses nothing, within one gesture: a later `Update`,
/// `Commit` or `Cancel` replaces the `Update` before it (it carries the value on), `Begin` then `Commit` is one `Set`,
/// and `Begin` then `Cancel` is nothing. Every other event follows the one before, so a pass reports at most
/// `[Begin, Update]` of one gesture.
#[derive(Clone, Debug, PartialEq)]
pub struct EditEvents<T>(SmallVec<[EditEvent<T>; 2]>);

impl<T> Default for EditEvents<T> {
    fn default() -> Self {
        EditEvents(SmallVec::new())
    }
}

impl<T> EditEvents<T> {
    /// No events.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the next event of the pass, folding it with the one before when that loses nothing.
    pub fn push(&mut self, event: EditEvent<T>) {
        let gesture = event.id();
        let same = |last: Option<&EditEvent<T>>| gesture.is_some() && last.and_then(EditEvent::id) == gesture;
        if matches!(event, EditEvent::Update { .. } | EditEvent::Commit { .. } | EditEvent::Cancel { .. })
            && matches!(self.0.last(), Some(EditEvent::Update { .. }))
            && same(self.0.last())
        {
            self.0.pop();
        }
        if matches!(event, EditEvent::Commit { .. } | EditEvent::Cancel { .. })
            && matches!(self.0.last(), Some(EditEvent::Begin { .. }))
            && same(self.0.last())
        {
            let Some(EditEvent::Begin { before, label, doc_key, .. }) = self.0.pop() else { return };
            if let EditEvent::Commit { after, .. } = event {
                self.0.push(EditEvent::Set { before, after, label, doc_key });
            }
            return;
        }
        self.0.push(event);
    }

    /// No event happened.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The number of events.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The events in order.
    pub fn iter(&self) -> std::slice::Iter<'_, EditEvent<T>> {
        self.0.iter()
    }

    /// The latest event.
    pub fn last(&self) -> Option<&EditEvent<T>> {
        self.0.last()
    }

    /// The latest value the events carry: a proposal, a commit or a set.
    pub fn value(&self) -> Option<&T> {
        self.0.iter().rev().find_map(EditEvent::value)
    }

    /// The value the binding holds after the events: the latest carried value, or the value a cancel restores.
    pub fn landed(&self) -> Option<&T> {
        self.0.iter().rev().find_map(EditEvent::landed)
    }

    /// A gesture began.
    pub fn began(&self) -> bool {
        self.0.iter().any(|event| matches!(event, EditEvent::Begin { .. }))
    }

    /// The `Commit`, `Cancel` or `Set` that ended an edit in this pass; the latest one.
    pub fn ended(&self) -> Option<&EditEvent<T>> {
        self.0.iter().rev().find(|event| matches!(event, EditEvent::Commit { .. } | EditEvent::Cancel { .. } | EditEvent::Set { .. }))
    }

    /// Why a gesture was cancelled in this pass; the latest cancel.
    pub fn cancel_reason(&self) -> Option<CancelReason> {
        self.0.iter().rev().find_map(|event| match event {
            EditEvent::Cancel { reason, .. } => Some(*reason),
            _ => None,
        })
    }

    /// Maps the carried values of every event.
    pub fn map<U>(self, f: impl Fn(T) -> U) -> EditEvents<U> {
        EditEvents(self.0.into_iter().map(|event| event.map(&f)).collect())
    }

    /// Maps the carried values of every event, dropping the events `f` has no value for.
    pub fn filter_map<U>(self, f: impl Fn(T) -> Option<U>) -> EditEvents<U> {
        self.0.into_iter().filter_map(|event| event.filter_map(&f)).collect()
    }
}

impl<T> From<EditEvent<T>> for EditEvents<T> {
    fn from(event: EditEvent<T>) -> Self {
        EditEvents(smallvec::smallvec![event])
    }
}

impl<T> From<Option<EditEvent<T>>> for EditEvents<T> {
    fn from(event: Option<EditEvent<T>>) -> Self {
        event.into_iter().collect()
    }
}

impl<T> Extend<EditEvent<T>> for EditEvents<T> {
    fn extend<I: IntoIterator<Item = EditEvent<T>>>(&mut self, events: I) {
        for event in events {
            self.push(event);
        }
    }
}

impl<T> FromIterator<EditEvent<T>> for EditEvents<T> {
    fn from_iter<I: IntoIterator<Item = EditEvent<T>>>(events: I) -> Self {
        let mut list = EditEvents::new();
        list.extend(events);
        list
    }
}

impl<T> IntoIterator for EditEvents<T> {
    type Item = EditEvent<T>;
    type IntoIter = smallvec::IntoIter<[EditEvent<T>; 2]>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a EditEvents<T> {
    type Item = &'a EditEvent<T>;
    type IntoIter = std::slice::Iter<'a, EditEvent<T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// The running gesture of a value component, kept in shown memory under the component. Plain data; the pipeline
/// reads and writes it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Gesture<V> {
    /// The running or last gesture.
    pub id: GestureId,
    /// The value when it began; `Some` while a gesture runs.
    pub before: Option<Mixed<V>>,
    /// The latest proposed value.
    pub proposal: Option<V>,
    /// Last pass's given value, read while the gesture runs for the host-change comparison.
    #[doc(hidden)]
    pub last_given: Option<V>,
    /// The last value the pipeline applied or emitted: the value step 11 wrote to a write-through source, or for a
    /// given source `Begin`'s start value and then each `Update`.
    #[doc(hidden)]
    pub emitted: Option<V>,
    /// The value of the last `Update` the gesture reported; a proposal equal to it reports nothing.
    #[doc(hidden)]
    pub reported: Option<V>,
    /// A gesture was cancelled under the press that carried it: nothing is proposed until the release.
    #[doc(hidden)]
    pub dead: bool,
    /// The last `cancel_all` serial this gesture has seen.
    #[doc(hidden)]
    pub cancel_seen: u64,
    /// The runtime's removal watch holds this gesture's id.
    #[doc(hidden)]
    pub watched: bool,
}

impl<V> Default for Gesture<V> {
    fn default() -> Self {
        Gesture {
            id: GestureId(0),
            before: None,
            proposal: None,
            last_given: None,
            emitted: None,
            reported: None,
            dead: false,
            cancel_seen: 0,
            watched: false,
        }
    }
}

impl<V> Gesture<V> {
    /// The gesture after it ended: no value, the id kept, and `dead` while the press that carried it is held.
    pub fn ended(&self, dead: bool) -> Gesture<V> {
        Gesture { id: self.id, cancel_seen: self.cancel_seen, watched: self.watched, dead, ..Gesture::default() }
    }

    /// A gesture is running.
    pub fn is_running(&self) -> bool {
        self.before.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: GestureId = GestureId(1);

    fn begin() -> EditEvent<i32> {
        EditEvent::Begin { id: ID, before: Mixed::Same(0), label: "Gain".into(), doc_key: None }
    }

    fn fold(events: Vec<EditEvent<i32>>) -> Vec<EditEvent<i32>> {
        events.into_iter().collect::<EditEvents<_>>().into_iter().collect()
    }

    #[test]
    fn a_pass_keeps_every_event_and_folds_what_loses_nothing() {
        let update = |value| EditEvent::Update { id: ID, value };
        let commit = EditEvent::Commit { id: ID, before: Mixed::Same(0), after: 5 };
        let cancel = EditEvent::Cancel { id: ID, reason: CancelReason::Escape, before: Some(Mixed::Same(0)) };
        let set = fold(vec![begin(), update(3), commit.clone()]);
        assert!(matches!(&set[..], [EditEvent::Set { after: 5, label, .. }] if &**label == "Gain"), "Begin..Commit is one Set: {set:?}");
        assert_eq!(fold(vec![begin(), update(3), cancel.clone()]), vec![], "Begin..Cancel is nothing");
        assert_eq!(fold(vec![begin(), update(3), update(4)]), vec![begin(), update(4)], "Begin then the last Update");
        assert_eq!(fold(vec![update(3), commit.clone()]), vec![commit.clone()], "Update..Commit is Commit");
        assert_eq!(fold(vec![update(3), cancel.clone()]), vec![cancel], "Update..Cancel is Cancel");
        let next = EditEvent::Begin { id: GestureId(2), before: Mixed::Same(5), label: "Gain".into(), doc_key: None };
        assert_eq!(fold(vec![update(3), commit.clone(), next.clone()]), vec![commit, next], "a new gesture follows the ended one");
        let other = EditEvent::Update { id: GestureId(2), value: 7 };
        assert_eq!(fold(vec![update(3), other.clone()]), vec![update(3), other], "updates of two gestures stay apart");
    }
}
