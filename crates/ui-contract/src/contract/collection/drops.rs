//! What a collection takes from drags that did not start in it: one drop kind per payload type and place (17.4).

use std::any::{Any, TypeId};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

/// Where a drop kind takes its drops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropPlace {
    /// Between items, at the gap under the pointer: the item itself in a spatial layout, the whole collection when it
    /// is empty.
    Between,
    /// Onto the item under the pointer, which shows the `Into` mark and `RowState::drop_target`.
    Onto,
}

/// The place of one drop.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropAt<K> {
    /// Between items: before the position `insert` of the display order, the source's length after the last item.
    #[non_exhaustive]
    Gap {
        /// The item under the pointer; `None` in an empty collection.
        key: Option<K>,
        /// The position the payload lands before.
        insert: usize,
    },
    /// Onto the item.
    Onto(K),
}

impl<K> DropAt<K> {
    /// Between items, over `key`, before `insert`.
    pub const fn gap(key: Option<K>, insert: usize) -> Self {
        DropAt::Gap { key, insert }
    }

    /// The item under the pointer.
    pub fn key(&self) -> Option<&K> {
        match self {
            DropAt::Gap { key, .. } => key.as_ref(),
            DropAt::Onto(key) => Some(key),
        }
    }
}

type Test<'a, K> = Rc<dyn Fn(&dyn Any, &DropAt<K>) -> bool + 'a>;

/// A payload type a collection takes from a drag that did not start in it, at one place, where `accepts` passes. A
/// drag carries its payload and, for a row drag of a collection with `drag_payload`, a second one: a kind takes either.
/// The collection's own row drag is its reorder, never a drop kind. Files dragged in from the OS are a drag of `OsFiles`.
#[derive(Clone)]
pub struct DropKind<'a, K> {
    payload: TypeId,
    place: DropPlace,
    accepts: Test<'a, K>,
}

impl<'a, K> DropKind<'a, K> {
    /// Drops of payload type `P` at `place` where `accepts` passes for the payload and the place under the pointer.
    pub fn new<P: Send + Sync + 'static>(place: DropPlace, accepts: impl Fn(&P, &DropAt<K>) -> bool + 'a) -> Self {
        DropKind {
            payload: TypeId::of::<P>(),
            place,
            accepts: Rc::new(move |payload, at| payload.downcast_ref::<P>().is_some_and(|payload| accepts(payload, at))),
        }
    }

    pub(crate) fn payload(&self) -> TypeId {
        self.payload
    }

    pub(crate) fn place(&self) -> DropPlace {
        self.place
    }

    pub(crate) fn accepts(&self, payload: &dyn Any, at: &DropAt<K>) -> bool {
        (self.accepts)(payload, at)
    }
}

impl<K> fmt::Debug for DropKind<'_, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropKind").field("place", &self.place).finish_non_exhaustive()
    }
}

/// The payload of a drop a collection took, read by its type.
#[derive(Clone)]
pub struct DropPayload(Arc<dyn Any + Send + Sync>);

impl DropPayload {
    pub(crate) fn new(payload: Arc<dyn Any + Send + Sync>) -> Self {
        DropPayload(payload)
    }

    /// The payload, when it is a `P`.
    pub fn get<P: 'static>(&self) -> Option<&P> {
        self.0.downcast_ref::<P>()
    }
}

/// Two payloads are equal when they are the payload of the same drag.
impl PartialEq for DropPayload {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for DropPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DropPayload")
    }
}
