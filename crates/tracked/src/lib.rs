#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! `Tracked<T>`: app data whose every write moves a revision.
//!
//! A reader that only needs to know whether the data changed compares the [`Revision`] (two integers) instead of the
//! data. The revision is complete because the compiler sees every write:
//!
//! - Every way to change the data goes through `&mut Tracked<T>`: [`DerefMut`] and [`Tracked::edit`] move the
//!   revision, and craft-ui's `bind_tracked!` binds fields that move it when an edit changes them.
//! - `T` is [`PlainData`], so a shared reference can't change it: interior mutability, raw pointers, `dyn` objects
//!   and closures have no implementation, and a `Tracked` holding one does not compile.
//!
//! ```
//! use craft_tracked::Tracked;
//! let mut names = Tracked::new(vec!["a".to_owned()]);
//! let before = names.revision();
//! assert_eq!(names.len(), 1);
//! assert_eq!(names.revision(), before, "reading leaves the revision");
//! names.push("b".to_owned());
//! assert_ne!(names.revision(), before, "a write moves it");
//! ```
//!
//! # App types
//!
//! App types derive the marker with `#[derive(PlainData)]` (craft-ui-macros, re-exported by craft-ui and the
//! `craft` facade), which requires it of every field and names the field whose type has none:
//!
//! ```
//! use craft_tracked::Tracked;
//! use craft_ui_macros::PlainData;
//!
//! #[derive(PlainData)]
//! enum Node {
//!     Leaf(String),
//!     Group { name: String, children: Vec<Node> },
//! }
//!
//! let tree = Tracked::new(Node::Group { name: "root".into(), children: vec![Node::Leaf("a".into())] });
//! # let _ = tree;
//! ```
//!
//! Generic and self-referring types derive it too; each type parameter must then be plain data, and a type that
//! holds a parameter only in `PhantomData` (a typed id) needs nothing more:
//!
//! ```
//! use craft_tracked::Tracked;
//! use craft_ui_macros::PlainData;
//! use std::marker::PhantomData;
//!
//! #[derive(PlainData)]
//! struct Tree<T> {
//!     value: T,
//!     children: Vec<Tree<T>>,
//! }
//!
//! #[derive(PlainData)]
//! struct List {
//!     name: String,
//!     next: Option<Box<Self>>,
//! }
//!
//! #[derive(PlainData)]
//! struct Id<T> {
//!     raw: u64,
//!     kind: PhantomData<T>,
//! }
//!
//! let mut tree = Tracked::new(Tree { value: 1_u32, children: vec![Tree { value: 2, children: Vec::new() }] });
//! tree.children[0].value = 3;
//! let list = Tracked::new(List { name: "a".into(), next: Some(Box::new(List { name: "b".into(), next: None })) });
//! let id = Tracked::new(Id::<List> { raw: 7, kind: PhantomData });
//! # let _ = (list, id);
//! ```
//!
//! Two kinds of type have no derived implementation and take one of two escapes, each a stated promise that a shared
//! reference can't change what readers see:
//!
//! - **A foreign plain type** used as a field (an egui `Rect`, a `serde_json::Value`): the field carries
//!   `#[plain_data(assume = "why the type is plain")]`, which skips the check for that field only. The reason is
//!   required; an empty one does not compile.
//! - **A type that encapsulates interior mutability readers can't observe** (a tile whose bytes move between memory
//!   and disk while its pixels stay the same): the crate that owns the type implements the marker by hand,
//!   `impl PlainData for Tile {}`, with a comment saying why. Hand implementations are allowed by design in the
//!   non-UI crate that owns such a type; UI crates have no such types, use the derive, and design-lint rejects a hand
//!   implementation there.
//!
//! ```
//! use craft_tracked::Tracked;
//! use craft_ui_macros::PlainData;
//!
//! # mod egui { pub struct Rect; }
//! #[derive(PlainData)]
//! struct Panel {
//!     title: String,
//!     #[plain_data(assume = "egui geometry: plain numbers, no interior mutability")]
//!     rect: egui::Rect,
//! }
//! # let _ = Tracked::new(Panel { title: String::new(), rect: egui::Rect });
//! ```
//!
//! # Threads
//!
//! The revision is a plain `u64`: every write needs `&mut Tracked<T>`, so the thread that owns the value is the one
//! that moves it. An engine that runs jobs on worker threads and applies their results on the UI thread keeps its
//! model in a `Tracked` there: a worker computes on a snapshot and sends its result back, and applying the result
//! through `&mut` moves the revision on the owning side.
//!
//! # Rejected types
//!
//! ```compile_fail,E0277
//! let _ = craft_tracked::Tracked::new(std::cell::Cell::new(1));
//! ```
//!
//! ```compile_fail,E0277
//! let _ = craft_tracked::Tracked::new(std::cell::RefCell::new(1));
//! ```
//!
//! ```compile_fail,E0277
//! let _ = craft_tracked::Tracked::new(std::sync::Mutex::new(1));
//! ```
//!
//! ```compile_fail,E0277
//! let _ = craft_tracked::Tracked::new(std::sync::RwLock::new(1));
//! ```
//!
//! ```compile_fail,E0277
//! let _ = craft_tracked::Tracked::new(std::sync::atomic::AtomicU32::new(1));
//! ```
//!
//! ```compile_fail,E0277
//! let _ = craft_tracked::Tracked::new(std::sync::OnceLock::<u32>::new());
//! ```
//!
//! ```compile_fail,E0277
//! let _ = craft_tracked::Tracked::new(std::ptr::null::<u8>());
//! ```
//!
//! ```compile_fail,E0277
//! let _ = craft_tracked::Tracked::new(Box::new(|| 1) as Box<dyn Fn() -> i32>);
//! ```
//!
//! ```compile_fail,E0277
//! let _ = craft_tracked::Tracked::new(std::sync::Arc::new(std::sync::Mutex::new(1)));
//! ```
//!
//! A derived type with a rejected field:
//!
//! ```compile_fail,E0277
//! #[derive(craft_ui_macros::PlainData)]
//! struct Counter {
//!     hits: std::cell::Cell<u32>,
//! }
//! ```
//!
//! A generic derived type whose field is rejected for every parameter fails at the derive:
//!
//! ```compile_fail,E0277
//! #[derive(craft_ui_macros::PlainData)]
//! struct Counter<T> {
//!     hits: std::cell::Cell<T>,
//! }
//! ```
//!
//! A generic derived type used with a rejected parameter:
//!
//! ```compile_fail,E0277
//! #[derive(craft_ui_macros::PlainData)]
//! struct Wrap<T>(T);
//! let _ = craft_tracked::Tracked::new(Wrap(std::cell::Cell::new(1)));
//! ```
//!
//! An `assume` without a reason (the derive reports it with `compile_error!`, which has no error code):
//!
//! ```compile_fail
//! #[derive(craft_ui_macros::PlainData)]
//! struct Panel {
//!     #[plain_data(assume = "")]
//!     rect: std::cell::Cell<u32>,
//! }
//! ```

// The derive reaches this crate as `::craft_tracked`, here too.
extern crate self as craft_tracked;

use std::cell::Cell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

/// Data that a shared reference can't change: no interior mutability, no raw pointers, no `dyn` objects, no
/// closures. Implemented for plain std types and their containers; app types derive it.
///
/// A hand implementation is a promise the compiler can't check. It belongs only in this crate and in the crate that
/// owns a type whose interior mutability readers can't observe; see the crate docs.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not plain data",
    label = "`{Self}` can change behind a shared reference, or has no `PlainData` implementation",
    note = "`Cell`, `RefCell`, `Mutex`, `RwLock`, `OnceLock`, atomics, raw pointers, `dyn` objects and closures are rejected, because a write through them would not move the revision",
    note = "app types use `#[derive(PlainData)]`; a field of a foreign plain type takes `#[plain_data(assume = \"why the type is plain\")]`"
)]
pub trait PlainData {}

macro_rules! plain {
    ($($t:ty),* $(,)?) => { $(impl PlainData for $t {})* };
}

plain!(bool, char, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64, str, String, ());
plain!(std::num::NonZeroU8, std::num::NonZeroU16, std::num::NonZeroU32, std::num::NonZeroU64, std::num::NonZeroUsize);
plain!(std::num::NonZeroI8, std::num::NonZeroI16, std::num::NonZeroI32, std::num::NonZeroI64, std::num::NonZeroIsize);
plain!(std::cmp::Ordering, std::time::Duration, std::time::Instant, std::time::SystemTime);
plain!(std::path::PathBuf, std::path::Path, std::ffi::OsString, std::ffi::OsStr);
plain!(std::collections::hash_map::RandomState);

impl<T: PlainData + ?Sized> PlainData for &T {}
impl<T: PlainData + ?Sized> PlainData for Box<T> {}
impl<T: PlainData + ?Sized> PlainData for std::sync::Arc<T> {}
impl<T: PlainData + ?Sized> PlainData for std::rc::Rc<T> {}
impl<B: PlainData + ToOwned + ?Sized> PlainData for std::borrow::Cow<'_, B> where B::Owned: PlainData {}
impl<T: PlainData> PlainData for Option<T> {}
impl<T: PlainData, E: PlainData> PlainData for Result<T, E> {}
impl<T: PlainData> PlainData for Vec<T> {}
impl<T: PlainData> PlainData for [T] {}
impl<T: PlainData, const N: usize> PlainData for [T; N] {}
impl<T: PlainData> PlainData for std::ops::Range<T> {}
impl<T: PlainData> PlainData for std::ops::RangeInclusive<T> {}
impl<T: PlainData> PlainData for std::collections::VecDeque<T> {}
impl<T: PlainData> PlainData for std::collections::BinaryHeap<T> {}
impl<K: PlainData, V: PlainData, S: PlainData> PlainData for std::collections::HashMap<K, V, S> {}
impl<K: PlainData, S: PlainData> PlainData for std::collections::HashSet<K, S> {}
impl<K: PlainData, V: PlainData> PlainData for std::collections::BTreeMap<K, V> {}
impl<K: PlainData> PlainData for std::collections::BTreeSet<K> {}
impl<H> PlainData for std::hash::BuildHasherDefault<H> {}
impl<T: ?Sized> PlainData for std::marker::PhantomData<T> {}
impl<T: PlainData> PlainData for Tracked<T> {}

macro_rules! tuples {
    ($(($($n:ident),+)),* $(,)?) => { $(impl<$($n: PlainData),+> PlainData for ($($n,)+) {})* };
}

tuples!(
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F),
    (A, B, C, D, E, F, G),
    (A, B, C, D, E, F, G, H),
    (A, B, C, D, E, F, G, H, I),
    (A, B, C, D, E, F, G, H, I, J),
    (A, B, C, D, E, F, G, H, I, J, K),
    (A, B, C, D, E, F, G, H, I, J, K, L),
);

/// The next source id. Ids start at 1 and are never reused in a process.
static NEXT_SOURCE: AtomicU64 = AtomicU64::new(1);

fn next_source() -> u64 {
    NEXT_SOURCE.fetch_add(1, Ordering::Relaxed)
}

/// Which tracked value, and which state of it: equal revisions mean equal data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Revision {
    /// The value's source: unique in the process, and new for every clone.
    pub source: u64,
    /// How many writes the value has had.
    pub count: u64,
}

/// `T` with a revision that every write moves.
///
/// `Deref` reads; `DerefMut` and [`edit`](Self::edit) move the revision whether or not the
/// write changes the data (a write with no change costs a reader one needless run, never a stale one). A clone is a
/// new source: two clones written apart never share a revision.
#[derive(Debug)]
pub struct Tracked<T: PlainData> {
    value: T,
    revision: Revision,
}

impl<T: PlainData + Default> Default for Tracked<T> {
    fn default() -> Tracked<T> {
        Tracked::new(T::default())
    }
}

impl<T: PlainData> Tracked<T> {
    /// `value`, as a new source at count 0.
    pub fn new(value: T) -> Tracked<T> {
        Tracked { value, revision: Revision { source: next_source(), count: 0 } }
    }

    /// The current revision.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The value's source id: unique in the process, new for every clone.
    pub fn source(&self) -> u64 {
        self.revision.source
    }

    /// Changes the value through `f` and moves the revision.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut **self)
    }

    /// Called by craft-ui's `bind_tracked!` only: the value without moving the revision, the write count and the
    /// source id. The macro borrows each bound field from the value through a field path and gives the binding the
    /// count, which it moves on every write that changes the field, and the source, which names the field's reads. A direct call can write any part of the value without moving the
    /// revision, so readers keyed by it would miss the write; design-lint rejects direct calls in UI crates.
    #[doc(hidden)]
    pub fn __split_for_bind(&mut self) -> (&mut T, &Cell<u64>, u64) {
        (&mut self.value, Cell::from_mut(&mut self.revision.count), self.revision.source)
    }

    /// The value, leaving the tracking.
    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T: PlainData + Clone> Clone for Tracked<T> {
    fn clone(&self) -> Tracked<T> {
        Tracked::new(self.value.clone())
    }
}

/// `value`, as a new source at count 0 ([`Tracked::new`]).
impl<T: PlainData> From<T> for Tracked<T> {
    fn from(value: T) -> Tracked<T> {
        Tracked::new(value)
    }
}

/// Equal values compare equal, whatever their revisions.
impl<T: PlainData + PartialEq> PartialEq for Tracked<T> {
    fn eq(&self, other: &Tracked<T>) -> bool {
        self.value == other.value
    }
}

impl<T: PlainData + Eq> Eq for Tracked<T> {}

/// Saved as its value.
#[cfg(feature = "serde")]
impl<T: PlainData + serde::Serialize> serde::Serialize for Tracked<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

/// Loaded as a new source.
#[cfg(feature = "serde")]
impl<'de, T: PlainData + serde::Deserialize<'de>> serde::Deserialize<'de> for Tracked<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Tracked<T>, D::Error> {
        T::deserialize(deserializer).map(Tracked::new)
    }
}

impl<T: PlainData> Deref for Tracked<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: PlainData> DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.revision.count = self.revision.count.wrapping_add(1);
        &mut self.value
    }
}

#[cfg(test)]
mod tests {
    use super::Tracked;

    struct Doc {
        name: String,
        size: (u32, u32),
    }

    impl super::PlainData for Doc {}

    #[test]
    fn the_revision_moves_on_writes_only_and_a_clone_is_a_new_source() {
        let mut doc = Tracked::new(Doc { name: "a".into(), size: (1, 2) });
        let start = doc.revision();
        assert_eq!(doc.name, "a");
        assert_eq!(doc.size.1, 2);
        assert_eq!(doc.revision(), start, "reads leave the revision");

        doc.name.push('b');
        let after_deref = doc.revision();
        assert_eq!(after_deref.count, start.count + 1, "a `&mut` access moves it");
        doc.edit(|doc| doc.size.0 = 3);
        assert_eq!(doc.revision().count, start.count + 2, "an edit moves it");
        assert_eq!((doc.size, doc.name.as_str()), ((3, 2), "ab"));
        assert_eq!(doc.source(), start.source);

        let mut a = Tracked::new(1_u32);
        let mut b = a.clone();
        assert_ne!(a.source(), b.source(), "a clone is a new source");
        *a += 1;
        *b += 2;
        assert_ne!(a.revision(), b.revision(), "two clones written apart never share a revision");
    }
}
