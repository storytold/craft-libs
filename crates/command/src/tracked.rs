//! Dirty flag around a document value. `&mut` access goes through [`Tracked::write`].

use std::ops::Deref;

/// `Deref` reads. [`Tracked::write`] is the only `&mut` path, and it marks the value dirty.
///
/// There is no `Clone` impl. Cloning the guard would copy the flag and hide a missed `mark_bumped`.
#[derive(Debug)]
pub struct Tracked<T> {
    value: T,
    dirty: bool,
}

impl<T> Tracked<T> {
    pub fn new(value: T) -> Self {
        Self { value, dirty: false }
    }

    /// Borrow the value for mutation and mark it dirty.
    pub fn write(&mut self) -> &mut T {
        self.dirty = true;
        &mut self.value
    }

    /// Clear the dirty flag. The app calls this from `bump()` after it publishes a change.
    pub fn mark_bumped(&mut self) {
        self.dirty = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn clone_value(&self) -> T
    where
        T: Clone,
    {
        self.value.clone()
    }
}

impl<T> Deref for Tracked<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
