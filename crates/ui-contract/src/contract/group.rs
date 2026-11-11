//! Group forms (20.3): a container and arena root whose members, placed anywhere in its body, join its arena and
//! register with its `GroupHandle`.

use crate::runtime::env::EnvValue;
use egui::Id;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, PoisonError};

/// The kind of a group form (radio, chip): it names the type of the members' values.
pub trait GroupKind: 'static {
    /// The value a member stands for.
    type Member: Clone + PartialEq + Send + Sync + 'static;
}

/// A group's registrations: the members shown in the last pass and the member the group picked.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GroupState<M> {
    /// The group's pick this frame (step 9).
    #[doc(hidden)]
    pub selected: Option<M>,
    /// Each member's value, id and enabled state, in the order they registered.
    #[doc(hidden)]
    pub members: Vec<(M, Id, bool)>,
}

impl<M> Default for GroupState<M> {
    fn default() -> Self {
        GroupState { selected: None, members: Vec::new() }
    }
}

/// A group form's handle, put into its body's env with `content_env`. Members and body code read it with
/// `cx.env.get::<GroupHandle<..>>()`; a member outside any group gets the inert root handle.
pub struct GroupHandle<G: GroupKind> {
    state: Option<Arc<Mutex<GroupState<G::Member>>>>,
    kind: PhantomData<fn() -> G>,
}

impl<G: GroupKind> Clone for GroupHandle<G> {
    fn clone(&self) -> Self {
        GroupHandle { state: self.state.clone(), kind: PhantomData }
    }
}

impl<G: GroupKind> PartialEq for GroupHandle<G> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.state, &other.state) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<G: GroupKind> EnvValue for GroupHandle<G> {
    fn root() -> Self {
        GroupHandle { state: None, kind: PhantomData }
    }
}

impl<G: GroupKind> GroupHandle<G> {
    /// The handle of the group whose registrations are `state`.
    ///
    /// For the group form's own code, not for members or body code.
    #[doc(hidden)]
    pub fn new(state: Arc<Mutex<GroupState<G::Member>>>) -> Self {
        GroupHandle { state: Some(state), kind: PhantomData }
    }

    /// Registers a member's value, id and enabled state for the group's pick next pass.
    pub fn register(&self, member: G::Member, id: Id, enabled: bool) {
        match &self.state {
            Some(_) => self.with(|state| state.members.push((member, id, enabled))),
            None => log::debug!("a {} member is shown outside any group", std::any::type_name::<G>()),
        }
    }

    /// The group's pick this frame.
    pub fn selected(&self) -> Option<G::Member> {
        self.with(|state| state.selected.clone())
    }

    /// Whether `value` is the group's pick.
    pub fn is(&self, value: &G::Member) -> bool {
        self.with(|state| state.selected.as_ref() == Some(value))
    }

    /// The enabled member registered with `id` last pass.
    ///
    /// For the group form's own code, not for members or body code.
    #[doc(hidden)]
    pub fn member(&self, id: Id) -> Option<G::Member> {
        self.with(|state| state.members.iter().find(|(_, member, enabled)| *member == id && *enabled).map(|(value, ..)| value.clone()))
    }

    /// The id of the enabled member registered with `value` last pass.
    ///
    /// For the group form's own code, not for members or body code.
    #[doc(hidden)]
    pub fn member_id(&self, value: &G::Member) -> Option<Id> {
        self.with(|state| state.members.iter().find(|(member, _, enabled)| member == value && *enabled).map(|(_, id, _)| *id))
    }

    /// Sets the pick.
    ///
    /// For the group form's own code, not for members or body code.
    #[doc(hidden)]
    pub fn select(&self, value: Option<G::Member>) {
        self.with(|state| state.selected = value);
    }

    /// Forgets the registrations: members register again as the body shows them.
    ///
    /// For the group form's own code, not for members or body code.
    #[doc(hidden)]
    pub fn begin_registration(&self) {
        self.with(|state| state.members.clear());
    }

    fn with<R>(&self, f: impl FnOnce(&mut GroupState<G::Member>) -> R) -> R
    where
        R: Default,
    {
        match &self.state {
            Some(state) => f(&mut state.lock().unwrap_or_else(PoisonError::into_inner)),
            None => R::default(),
        }
    }
}
