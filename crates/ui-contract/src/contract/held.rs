//! `Held`: a caller's binding read and written from inside a container body.

use super::Bind;
use crate::runtime::memory::StageAccess;
use craft_behavior::{EditEvent, Mixed, Value};
use std::sync::Arc;

/// The caller's binding of a value a pattern edits: the body reads the value at the start of its pass and returns
/// the edited one in its result, and the caller writes it as a single set edit from `Inner::settle`, once per pass
/// after the real run. A body may run more than once per pass, so it never writes.
pub struct Held<'a, T>(Bind<'a, T>);

impl<'a, T: Value> Held<'a, T> {
    /// Holds `bind`.
    pub fn new(bind: Bind<'a, T>) -> Self {
        Held(bind)
    }

    /// The bound value; `None` while a mixed or kept binding holds none.
    pub fn read(&mut self, cx: &mut impl StageAccess) -> Option<T> {
        match self.0.reborrow().read(cx) {
            Some(Mixed::Same(value)) => Some(value),
            _ => None,
        }
    }

    /// Replaces `before` with `after`, the edit named `name`.
    pub fn write(&mut self, cx: &mut impl StageAccess, before: &T, after: T, name: &str) {
        let event = EditEvent::set(Mixed::Same(before.clone()), after, Arc::from(name), None);
        let _ = self.0.reborrow().write(cx, &event);
    }
}
