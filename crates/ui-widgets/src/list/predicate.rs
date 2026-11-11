//! The tests a `List` asks its owner: which rows rename or drag, where a drag may come from and where it may land.
//! Each is a named type over a boxed closure; the setter's `func` attribute spells the closure's signature, so the
//! caller's closure parameters are inferred.

use crate::contract::ListDrag;
use egui::Id;

/// Whether the row of a key passes: `List::can_rename` and `List::can_drag`.
pub struct RowTest<'a, K>(Box<dyn Fn(&K) -> bool + 'a>);

impl<'a, K> RowTest<'a, K> {
    /// The test that runs `test`.
    pub fn new(test: impl Fn(&K) -> bool + 'a) -> Self {
        RowTest(Box::new(test))
    }
}

impl<K> RowTest<'_, K> {
    #[doc(hidden)]
    pub fn passes(&self, key: &K) -> bool {
        (self.0)(key)
    }
}

/// Whether a drag that started in the list with an id may drop here: `List::accept_from`.
pub struct SourceTest<'a>(Box<dyn Fn(Id) -> bool + 'a>);

impl<'a> SourceTest<'a> {
    /// The test that runs `test`.
    pub fn new(test: impl Fn(Id) -> bool + 'a) -> Self {
        SourceTest(Box::new(test))
    }
}

impl SourceTest<'_> {
    pub(crate) fn passes(&self, from: Id) -> bool {
        (self.0)(from)
    }
}

/// Whether a drag may land at a spot, given the drag, the row under the pointer and the insert position:
/// `List::accepts`.
pub struct DropTest<'a, K>(Box<DropFn<'a, K>>);

type DropFn<'a, K> = dyn Fn(&ListDrag<K>, Option<&K>, usize) -> bool + 'a;

impl<'a, K> DropTest<'a, K> {
    /// The test that runs `test`.
    pub fn new(test: impl Fn(&ListDrag<K>, Option<&K>, usize) -> bool + 'a) -> Self {
        DropTest(Box::new(test))
    }
}

impl<K> DropTest<'_, K> {
    #[doc(hidden)]
    pub fn passes(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        (self.0)(drag, over, insert)
    }
}
