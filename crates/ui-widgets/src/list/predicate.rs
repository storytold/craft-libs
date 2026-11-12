//! The tests a `List` asks its owner: which rows rename or drag, where a drag may come from and where it may land.
//! Each is a named type over a boxed closure; the setter's `func` attribute spells the closure's signature, so the
//! caller's closure parameters are inferred.

use crate::contract::ListDrag;

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

/// A text for the row of a key, or none: `List::rename_draft`.
pub struct RowText<'a, K>(Box<dyn Fn(&K) -> Option<String> + 'a>);

impl<'a, K> RowText<'a, K> {
    /// The text `text` gives.
    pub fn new(text: impl Fn(&K) -> Option<String> + 'a) -> Self {
        RowText(Box::new(text))
    }
}

impl<K> RowText<'_, K> {
    #[doc(hidden)]
    pub fn text(&self, key: &K) -> Option<String> {
        (self.0)(key)
    }
}

/// Whether a collection's own drag may land at a spot, given the drag, the row under the pointer and the insert position:
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
