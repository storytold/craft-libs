//! `MemoProps` and `MemoKey`: every input of a memoized view, as one key the framework compares.
//!
//! A memoized view's content is a `fn` pointer, which captures nothing, and its props are its only argument besides the
//! `Ui`. `MemoProps` is implemented only for types that cannot change behind a shared reference and that the key can
//! feed completely: plain values, strings, containers of props, references to props, `Tracked<T>` (by its revision),
//! callbacks as [`Stateless`] and the framework's value types. The types the plain-data derives reject (`Cell`,
//! `RefCell`, `OnceCell`, `OnceLock`, `Mutex`, `RwLock`, atomics, raw pointers, `dyn` trait objects, closures; the list
//! is on [`PlainData`](craft_tracked::PlainData)) have no implementation, so props holding one do not compile, and the
//! error points at the field:
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     count: std::cell::Cell<u32>,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props<'a> {
//!     count: &'a std::cell::RefCell<u32>,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     name: std::cell::OnceCell<String>,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     name: std::sync::OnceLock<String>,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     count: std::sync::Mutex<u32>,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     count: std::sync::RwLock<u32>,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     count: std::sync::atomic::AtomicU32,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     at: *const u8,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props<'a> {
//!     label: &'a dyn Fn() -> String,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     label: Box<dyn Fn() -> String>,
//! }
//! ```
//!
//! A closure, a field of a generic type and a field inside an enum variant are checked the same way:
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props<F> {
//!     label: F,
//! }
//! let n = 3;
//! craft_ui::contract::MemoKey::of(&Props { label: move || n });
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props<T> {
//!     value: T,
//! }
//! craft_ui::contract::MemoKey::of(&Props { value: std::cell::Cell::new(3) });
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! enum Props {
//!     Empty,
//!     Count(std::cell::Cell<u32>),
//! }
//! ```
//!
//! A field of a foreign plain type that has no implementation feeds the key through its `Hash`, with the reason it is
//! plain: `#[memo_props(hash = "why")]`. The type must be `Hash + Sync`, and its `Hash` must cover everything the view
//! shows and be prefix-free (two different values never feed bytes of which one starts the other). `Sync` keeps out
//! raw pointers, `NonNull`, `Cell` and `RefCell`, whose `Hash` (where they have one) says nothing about what they point
//! at or hold. A type without `Hash`, a raw pointer and a `NonNull` still fail:
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     #[memo_props(hash = "an address")]
//!     at: *const u8,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     #[memo_props(hash = "an address")]
//!     at: std::ptr::NonNull<u8>,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props {
//!     #[memo_props(hash = "a plain number")]
//!     count: std::cell::Cell<u32>,
//! }
//! ```
//!
//! A callback's type can't be named, so props holding a [`Stateless`] are generic over it. Its parameter is marked
//! `#[memo_props(stateless)]`, which bounds it by what `Stateless` needs instead of `MemoProps`:
//!
//! ```
//! # extern crate craft_ui_contract as craft_ui;
//! use craft_ui::contract::{MemoKey, MemoProps, Stateless};
//! #[derive(MemoProps)]
//! struct Props<#[memo_props(stateless)] E> {
//!     title: String,
//!     english: Stateless<E>,
//! }
//! fn english() -> &'static str {
//!     "English"
//! }
//! let props = |title: &str| Props { title: title.to_owned(), english: Stateless::new(english) };
//! assert_eq!(MemoKey::of(&props("a")), MemoKey::of(&props("a")));
//! assert_ne!(MemoKey::of(&props("a")), MemoKey::of(&props("b")));
//! ```
//!
//! The mark only lifts the bound: a marked parameter held as a bare field is still checked, and an unmarked parameter
//! holding a fn item is not props:
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props<#[memo_props(stateless)] E> {
//!     english: E,
//! }
//! ```
//!
//! ```compile_fail,E0277
//! # extern crate craft_ui_contract as craft_ui;
//! use craft_ui::contract::{MemoKey, MemoProps, Stateless};
//! #[derive(MemoProps)]
//! struct Props<E: 'static> {
//!     english: Stateless<E>,
//! }
//! fn english() -> &'static str {
//!     "English"
//! }
//! MemoKey::of(&Props { english: Stateless::new(english) });
//! ```
//!
//! The mark is for type parameters only:
//!
//! ```compile_fail
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Props<#[memo_props(stateless)] 'a> {
//!     title: &'a str,
//! }
//! ```
//!
//! A field another field stands for feeds nothing: `#[memo_props(covered_by = "<field>", reason = "why")]`. The named
//! field must be one of the same struct or variant that feeds the key itself, and the reason is required. A closure the
//! caller's revision covers:
//!
//! ```
//! # extern crate craft_ui_contract as craft_ui;
//! use craft_ui::contract::{MemoKey, MemoProps};
//! use std::sync::Arc;
//! #[derive(MemoProps)]
//! struct Rules {
//!     #[memo_props(covered_by = "revision", reason = "the caller moves the revision with every rule change")]
//!     allows: Arc<dyn Fn(&str) -> bool + Send + Sync>,
//!     revision: u64,
//! }
//! let rules = |revision, allows: fn(&str) -> bool| Rules { allows: Arc::new(allows), revision };
//! assert_eq!(MemoKey::of(&rules(1, |_| true)), MemoKey::of(&rules(1, |_| false)));
//! assert_ne!(MemoKey::of(&rules(1, |_| true)), MemoKey::of(&rules(2, |_| true)));
//! ```
//!
//! ```compile_fail
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Rules {
//!     #[memo_props(covered_by = "generation", reason = "no such field")]
//!     allows: std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync>,
//!     revision: u64,
//! }
//! ```
//!
//! ```compile_fail
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Rules {
//!     #[memo_props(covered_by = "revision")]
//!     allows: std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync>,
//!     revision: u64,
//! }
//! ```
//!
//! A `&'static` reference to a static item feeds where it points: `#[memo_props(address = "why")]`. Any other field
//! type is an error:
//!
//! ```
//! # extern crate craft_ui_contract as craft_ui;
//! use craft_ui::contract::{MemoKey, MemoProps};
//! struct Registration(&'static str);
//! // Equal registrations at two places: two items.
//! static FIRST: Registration = Registration("Button");
//! static SECOND: Registration = Registration("Button");
//! #[derive(MemoProps)]
//! struct Card {
//!     #[memo_props(address = "registrations are static items")]
//!     story: &'static Registration,
//! }
//! assert_eq!(MemoKey::of(&Card { story: &FIRST }), MemoKey::of(&Card { story: &FIRST }));
//! assert_ne!(MemoKey::of(&Card { story: &FIRST }), MemoKey::of(&Card { story: &SECOND }));
//! ```
//!
//! ```compile_fail
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! struct Card<'a> {
//!     #[memo_props(address = "a borrowed value")]
//!     story: &'a u32,
//! }
//! ```
//!
//! A handle to state written during the pass, or a context rebuilt each pass, is keyed in every pass:
//! `#[memo_props(every_pass = "why")]` on the type. It feeds no field (a field attribute there is an error), and a view
//! whose key holds it runs in every pass; outside a view's key it feeds nothing:
//!
//! ```
//! # extern crate craft_ui_contract as craft_ui;
//! use craft_ui::contract::{MemoKey, MemoProps};
//! use std::sync::{Arc, Mutex};
//! #[derive(MemoProps)]
//! #[memo_props(every_pass = "its members write the shared state during the pass")]
//! struct Handle(Arc<Mutex<Vec<u32>>>);
//! let handle = Handle(Arc::default());
//! assert_eq!(MemoKey::of(&handle), MemoKey::of(&handle));
//! ```
//!
//! ```compile_fail
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! #[memo_props(every_pass = "")]
//! struct Handle(std::sync::Arc<std::sync::Mutex<Vec<u32>>>);
//! ```
//!
//! ```compile_fail
//! # extern crate craft_ui_contract as craft_ui;
//! #[derive(craft_ui::contract::MemoProps)]
//! #[memo_props(every_pass = "rebuilt each pass")]
//! struct Handle(#[memo_props(hash = "a number")] u32);
//! ```
//!
//! Plain props derive, recursive types included:
//!
//! ```
//! # extern crate craft_ui_contract as craft_ui;
//! use craft_ui::contract::{MemoKey, MemoProps};
//! #[derive(MemoProps)]
//! enum Node {
//!     Leaf(String),
//!     Branch(Vec<Node>),
//! }
//! let tree = |name: &str| Node::Branch(vec![Node::Leaf(name.to_owned())]);
//! assert_eq!(MemoKey::of(&tree("a")), MemoKey::of(&tree("a")));
//! assert_ne!(MemoKey::of(&tree("a")), MemoKey::of(&tree("b")));
//! ```
//!
//! What the types cannot see: process-level state the `fn` or its callees read (`static`, `thread_local!`, `OnceLock`,
//! clocks, files, environment variables) and `unsafe` code. A design-lint rule covers UI crates and the replay audit
//! covers the rest.

use std::any::TypeId;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128, NonZeroIsize, NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128,
    NonZeroUsize,
};
use std::ops::{Range, RangeInclusive};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use craft_tracked::{PlainData, Revision, Tracked};
use siphasher::sip128::{Hasher128, SipHasher13};

pub use craft_ui_macros::MemoProps;

/// Every input a memoized view reads, fed into a [`MemoKey`]. Two equal keys mean equal inputs.
///
/// Derive it (`#[derive(MemoProps)]`): the derive feeds every field in order and requires `MemoProps` of each, so a
/// field the view could read without the key seeing it does not compile. Implementations by hand belong to this crate
/// only.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be memo props",
    label = "not memo props",
    note = "memo props hold plain values, strings, containers and references of props, `Tracked<T>` and framework value types; interior mutability (`Cell`, `RefCell`, `Mutex`, atomics), closures, `dyn` trait objects and raw pointers are rejected, because a memoized view could read them without its key seeing the change",
    note = "large app data goes in `Tracked<T>`, which keys by revision; a callback is a `Stateless` fn item or capture-free closure; a foreign plain type takes `#[memo_props(hash = \"why\")]`"
)]
pub trait MemoProps {
    /// Feeds every input the view reads into `key`.
    fn key(&self, key: &mut MemoKey);

    /// The props type with its lifetimes erased (`Props<'a>` and `Props<'b>` are one type): a memoized view seeds its
    /// key with it, so two views at one id whose props feed the same bytes, such as two empty props types, key apart.
    fn identity() -> TypeId
    where
        Self: Sized,
    {
        typeid::of::<Self>()
    }
}

/// A 128-bit hash of a memoized view's props (SipHash-1-3 with a 128-bit output, a fixed key). A collision has a chance
/// of about 2⁻¹²⁸ per compare. A [`Stateless`] callback feeds the 64 bits its `TypeId` hashes, so two callbacks key
/// apart unless those collide.
#[derive(Clone, Default)]
pub struct MemoKey {
    hasher: SipHasher13,
    bytes: usize,
    /// The context of the view that takes the key: framework values whose writes the context counts (kept and saved
    /// bindings) feed that count.
    context: Option<egui::Context>,
}

impl std::fmt::Debug for MemoKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoKey").field("bytes", &self.bytes).finish_non_exhaustive()
    }
}

impl MemoKey {
    /// An empty key.
    pub fn new() -> MemoKey {
        MemoKey::default()
    }

    /// An empty key of a view shown in `ctx`.
    pub(crate) fn of_view(ctx: &egui::Context) -> MemoKey {
        MemoKey { context: Some(ctx.clone()), ..MemoKey::default() }
    }

    /// An empty key of the same view as this one.
    pub(crate) fn child(&self) -> MemoKey {
        MemoKey { context: self.context.clone(), ..MemoKey::default() }
    }

    /// The context of the view that takes the key; `None` for a key taken outside a view (`MemoKey::of`).
    pub(crate) fn context(&self) -> Option<&egui::Context> {
        self.context.as_ref()
    }

    /// The key of `props`.
    pub fn of<P: MemoProps + ?Sized>(props: &P) -> u128 {
        let mut key = MemoKey::new();
        props.key(&mut key);
        key.finish()
    }

    /// Feeds an input whose value a key cannot follow (a handle to state shared and written elsewhere, a context a
    /// container rebuilds each pass): a view whose key holds it runs in every pass. Outside a view's key it feeds
    /// nothing.
    pub fn changes_every_pass(&mut self) {
        if let Some(ctx) = &self.context {
            let pass = ctx.cumulative_pass_nr();
            self.write(&pass.to_le_bytes());
        }
    }

    /// Feeds `bytes`.
    pub fn write(&mut self, bytes: &[u8]) {
        self.hasher.write(bytes);
        self.bytes += bytes.len();
    }

    /// Feeds a length or an enum variant's index.
    pub fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }

    /// How many bytes fed the key: the view that takes the key adds them to the pass's key-byte count.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The key of everything fed so far.
    pub fn finish(&self) -> u128 {
        self.hasher.finish128().as_u128()
    }

    /// Feeds `value` through its `Hash`: for a foreign plain type whose `Hash` is derived over every field (this
    /// crate's implementations, and `#[memo_props(hash = "..")]` fields). `Sync` keeps out raw pointers, `NonNull`,
    /// `Cell` and `RefCell`.
    #[doc(hidden)]
    pub fn feed_hash<T: Hash + Sync + ?Sized>(&mut self, value: &T) {
        value.hash(&mut Feed(self));
    }

    fn write_len(&mut self, len: usize) {
        self.write(&(len as u64).to_le_bytes());
    }
}

/// `Hasher` over a key, for [`MemoKey::feed_hash`].
struct Feed<'k>(&'k mut MemoKey);

impl Hasher for Feed<'_> {
    fn finish(&self) -> u64 {
        // Never read: the key finishes itself.
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }
}

macro_rules! bytes {
    ($($t:ty),* $(,)?) => {
        $(impl MemoProps for $t {
            fn key(&self, key: &mut MemoKey) {
                key.write(&self.to_le_bytes());
            }
        })*
    };
}

bytes!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

impl MemoProps for bool {
    fn key(&self, key: &mut MemoKey) {
        key.write(&[u8::from(*self)]);
    }
}

impl MemoProps for char {
    fn key(&self, key: &mut MemoKey) {
        key.write_u32(u32::from(*self));
    }
}

impl MemoProps for f32 {
    fn key(&self, key: &mut MemoKey) {
        key.write(&self.to_bits().to_le_bytes());
    }
}

impl MemoProps for f64 {
    fn key(&self, key: &mut MemoKey) {
        key.write(&self.to_bits().to_le_bytes());
    }
}

impl MemoProps for () {
    fn key(&self, _: &mut MemoKey) {}
}

impl<T: ?Sized> MemoProps for PhantomData<T> {
    fn key(&self, _: &mut MemoKey) {}
}

impl MemoProps for str {
    fn key(&self, key: &mut MemoKey) {
        // The length first, so ("ab", "") and ("a", "b") differ.
        key.write_len(self.len());
        key.write(self.as_bytes());
    }
}

impl MemoProps for String {
    fn key(&self, key: &mut MemoKey) {
        self.as_str().key(key);
    }
}

impl<B: MemoProps + ToOwned + ?Sized> MemoProps for Cow<'_, B> {
    fn key(&self, key: &mut MemoKey) {
        (**self).key(key);
    }
}

impl<T: MemoProps + ?Sized> MemoProps for &T {
    fn key(&self, key: &mut MemoKey) {
        (**self).key(key);
    }
}

impl<T: MemoProps + ?Sized> MemoProps for Box<T> {
    fn key(&self, key: &mut MemoKey) {
        (**self).key(key);
    }
}

/// By walking the value, as `Arc`: a large shared model goes in `Tracked`, which keys by revision.
impl<T: MemoProps + ?Sized> MemoProps for Rc<T> {
    fn key(&self, key: &mut MemoKey) {
        (**self).key(key);
    }
}

/// By walking the value: a large shared model goes in `Tracked`, which keys by revision.
impl<T: MemoProps + ?Sized> MemoProps for Arc<T> {
    fn key(&self, key: &mut MemoKey) {
        (**self).key(key);
    }
}

impl<T: MemoProps> MemoProps for Option<T> {
    fn key(&self, key: &mut MemoKey) {
        match self {
            None => key.write(&[0]),
            Some(value) => {
                key.write(&[1]);
                value.key(key);
            }
        }
    }
}

impl<T: MemoProps, E: MemoProps> MemoProps for Result<T, E> {
    fn key(&self, key: &mut MemoKey) {
        match self {
            Ok(value) => {
                key.write(&[0]);
                value.key(key);
            }
            Err(error) => {
                key.write(&[1]);
                error.key(key);
            }
        }
    }
}

impl<T: MemoProps> MemoProps for Range<T> {
    fn key(&self, key: &mut MemoKey) {
        let Range { start, end } = self;
        start.key(key);
        end.key(key);
    }
}

impl<T: MemoProps + PartialOrd> MemoProps for RangeInclusive<T> {
    fn key(&self, key: &mut MemoKey) {
        self.start().key(key);
        self.end().key(key);
        // An iterated-out range is empty whatever its ends.
        self.is_empty().key(key);
    }
}

impl MemoProps for Duration {
    fn key(&self, key: &mut MemoKey) {
        (self.as_secs(), self.subsec_nanos()).key(key);
    }
}

impl MemoProps for Ordering {
    fn key(&self, key: &mut MemoKey) {
        (*self as i8).key(key);
    }
}

macro_rules! non_zero {
    ($($t:ty),* $(,)?) => {
        $(impl MemoProps for $t {
            fn key(&self, key: &mut MemoKey) {
                self.get().key(key);
            }
        })*
    };
}

non_zero!(
    NonZeroU8,
    NonZeroU16,
    NonZeroU32,
    NonZeroU64,
    NonZeroU128,
    NonZeroUsize,
    NonZeroI8,
    NonZeroI16,
    NonZeroI32,
    NonZeroI64,
    NonZeroI128,
    NonZeroIsize,
);

/// Its bytes as the platform encodes them, length first.
impl MemoProps for OsStr {
    fn key(&self, key: &mut MemoKey) {
        let bytes = self.as_encoded_bytes();
        key.write_len(bytes.len());
        key.write(bytes);
    }
}

impl MemoProps for OsString {
    fn key(&self, key: &mut MemoKey) {
        self.as_os_str().key(key);
    }
}

impl MemoProps for Path {
    fn key(&self, key: &mut MemoKey) {
        self.as_os_str().key(key);
    }
}

impl MemoProps for PathBuf {
    fn key(&self, key: &mut MemoKey) {
        self.as_os_str().key(key);
    }
}

impl<T: MemoProps> MemoProps for [T] {
    fn key(&self, key: &mut MemoKey) {
        key.write_len(self.len());
        for value in self {
            value.key(key);
        }
    }
}

impl<T: MemoProps, const N: usize> MemoProps for [T; N] {
    fn key(&self, key: &mut MemoKey) {
        self.as_slice().key(key);
    }
}

impl<T: MemoProps> MemoProps for Vec<T> {
    fn key(&self, key: &mut MemoKey) {
        self.as_slice().key(key);
    }
}

impl<K: MemoProps, V: MemoProps> MemoProps for BTreeMap<K, V> {
    fn key(&self, key: &mut MemoKey) {
        key.write_len(self.len());
        for (k, v) in self {
            k.key(key);
            v.key(key);
        }
    }
}

impl<T: MemoProps> MemoProps for BTreeSet<T> {
    fn key(&self, key: &mut MemoKey) {
        key.write_len(self.len());
        for value in self {
            value.key(key);
        }
    }
}

macro_rules! tuples {
    ($(($($n:ident $i:tt),+)),* $(,)?) => {
        $(impl<$($n: MemoProps),+> MemoProps for ($($n,)+) {
            fn key(&self, key: &mut MemoKey) {
                $(self.$i.key(key);)+
            }
        })*
    };
}

tuples!(
    (A 0),
    (A 0, B 1),
    (A 0, B 1, C 2),
    (A 0, B 1, C 2, D 3),
    (A 0, B 1, C 2, D 3, E 4),
    (A 0, B 1, C 2, D 3, E 4, F 5),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7),
);

impl MemoProps for Revision {
    fn key(&self, key: &mut MemoKey) {
        let Revision { source, count } = *self;
        (source, count).key(key);
    }
}

/// By revision: equal revisions mean equal data.
impl<T: PlainData> MemoProps for Tracked<T> {
    fn key(&self, key: &mut MemoKey) {
        self.revision().key(key);
    }
}

/// A view's write access to a model: by revision, as `Tracked<T>`. A write the view makes moves the key, so the view
/// runs once more, from the written value, and replays from then on. `&mut` of any other type is not props: a write to
/// it would not move a key.
impl<T: PlainData> MemoProps for &mut Tracked<T> {
    fn key(&self, key: &mut MemoKey) {
        self.revision().key(key);
    }
}

/// A callback in props: a fn item or a capture-free closure, which is zero-sized, so its type alone says what it
/// does, and its `TypeId` keys it. A capturing closure is not zero-sized and fails to build where `Stateless::new` is
/// called (not at `cargo check`: the assertion is evaluated when the call is compiled):
///
/// ```compile_fail,E0080
/// # extern crate craft_ui_contract as craft_ui;
/// let n = 3;
/// let _ = craft_ui::contract::Stateless::new(move || n);
/// ```
///
/// ```
/// # extern crate craft_ui_contract as craft_ui;
/// use craft_ui::contract::{MemoKey, Stateless};
/// let three = Stateless::new(|| 3);
/// assert_eq!(three.get()(), 3);
/// assert_ne!(MemoKey::of(&three), MemoKey::of(&Stateless::new(|| 3)));
/// ```
#[derive(Clone, Copy)]
pub struct Stateless<F>(F);

impl<F: Copy + 'static> Stateless<F> {
    /// `f` as a prop; `f` must capture nothing.
    pub const fn new(f: F) -> Stateless<F> {
        const { assert!(size_of::<F>() == 0, "a Stateless callback captures nothing: pass a fn item or a capture-free closure") };
        Stateless(f)
    }

    /// The callback.
    pub fn get(&self) -> F {
        self.0
    }
}

impl<F> std::fmt::Debug for Stateless<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(std::any::type_name::<F>())
    }
}

impl<F: 'static> MemoProps for Stateless<F> {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(&TypeId::of::<F>());
    }
}

// The framework's value types and the foreign plain types they hold. Structs are destructured without `..`, so a new
// field fails to build here until it feeds the key. Types this crate cannot destructure (`#[non_exhaustive]` types of
// other crates, private fields) feed their `Hash`, which each derives over every field.

impl MemoProps for crate::runtime::env::Label {
    fn key(&self, key: &mut MemoKey) {
        use crate::runtime::env::Label;
        // `Static` and `Shared` of the same text show the same.
        match self {
            Label::Empty => key.write_u32(0),
            Label::Static(text) => {
                key.write_u32(1);
                text.key(key);
            }
            Label::Shared(text) => {
                key.write_u32(1);
                text.key(key);
            }
            Label::Word(word) => {
                key.write_u32(2);
                MemoProps::key(word, key);
            }
        }
    }
}

/// Derived `Hash` (`#[non_exhaustive]`).
impl MemoProps for craft_i18n::Word {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

impl<T: MemoProps> MemoProps for craft_behavior::Mixed<T> {
    fn key(&self, key: &mut MemoKey) {
        match self {
            craft_behavior::Mixed::Same(value) => {
                key.write_u32(0);
                value.key(key);
            }
            craft_behavior::Mixed::Mixed => key.write_u32(1),
        }
    }
}

/// The translator (`Words::generation`): words with the same generation read every word the same.
/// Every field (`#[non_exhaustive]`, so a field added later is listed here by hand): the selected keys, the lead and
/// the anchor.
impl<K: MemoProps> MemoProps for craft_behavior::Selection<K> {
    fn key(&self, key: &mut MemoKey) {
        self.items.key(key);
        self.lead.key(key);
        self.anchor.key(key);
    }
}

impl MemoProps for craft_i18n::Words {
    fn key(&self, key: &mut MemoKey) {
        self.generation().key(key);
    }
}

/// The format (`Format::generation`): a format writes every number the same for its whole life.
impl MemoProps for craft_i18n::Format {
    fn key(&self, key: &mut MemoKey) {
        self.generation().key(key);
    }
}

/// The format (`DateFormat::generation`): a format and its clones write every date the same.
impl MemoProps for craft_i18n::DateFormat {
    fn key(&self, key: &mut MemoKey) {
        self.generation().key(key);
    }
}

/// The host (`Host::generation`): a host and its clones hold the same services and language.
impl MemoProps for craft_host::Host {
    fn key(&self, key: &mut MemoKey) {
        self.generation().key(key);
    }
}

impl MemoProps for craft_keymap::CommandRef {
    fn key(&self, key: &mut MemoKey) {
        let craft_keymap::CommandRef { id, params } = self;
        id.key(key);
        params.key(key);
    }
}

/// Every value in order; an object's entries in the map's order.
impl MemoProps for serde_json::Value {
    fn key(&self, key: &mut MemoKey) {
        use serde_json::Value;
        match self {
            Value::Null => key.write_u32(0),
            Value::Bool(value) => {
                key.write_u32(1);
                value.key(key);
            }
            Value::Number(number) => {
                key.write_u32(2);
                // A number is exactly one of these; its text keys it whole.
                if let Some(value) = number.as_u64() {
                    key.write_u32(0);
                    value.key(key);
                } else if let Some(value) = number.as_i64() {
                    key.write_u32(1);
                    value.key(key);
                } else {
                    key.write_u32(2);
                    number.to_string().key(key);
                }
            }
            Value::String(text) => {
                key.write_u32(3);
                text.key(key);
            }
            Value::Array(values) => {
                key.write_u32(4);
                values.key(key);
            }
            Value::Object(map) => {
                key.write_u32(5);
                key.write_len(map.len());
                for (name, value) in map {
                    name.key(key);
                    value.key(key);
                }
            }
        }
    }
}

impl MemoProps for craft_keymap::Mods {
    fn key(&self, key: &mut MemoKey) {
        let craft_keymap::Mods { cmd, ctrl, alt, shift } = *self;
        (cmd, ctrl, alt, shift).key(key);
    }
}

/// Derived `Hash` (`#[non_exhaustive]`).
impl MemoProps for craft_keymap::Key {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

impl MemoProps for craft_keymap::Chord {
    fn key(&self, key: &mut MemoKey) {
        let craft_keymap::Chord { mods, key: pressed } = *self;
        mods.key(key);
        pressed.key(key);
    }
}

impl MemoProps for craft_theme::Srgba {
    fn key(&self, key: &mut MemoKey) {
        let craft_theme::Srgba { r, g, b, a } = *self;
        (r, g, b, a).key(key);
    }
}

impl MemoProps for craft_theme::Rgba {
    fn key(&self, key: &mut MemoKey) {
        let craft_theme::Rgba(channels) = *self;
        channels.key(key);
    }
}

/// Derived `Hash` over a unit enum.
impl MemoProps for craft_theme::TypeRole {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

/// Derived `Hash` (`#[non_exhaustive]`).
impl MemoProps for craft_theme::variant::Tone {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

/// Derived `Hash` over a unit enum.
impl MemoProps for craft_theme::LabelColor {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

/// Derived `Hash`.
impl MemoProps for craft_theme::ComponentSet {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

impl MemoProps for craft_behavior::IconId {
    fn key(&self, key: &mut MemoKey) {
        // The name is the whole id.
        self.name().key(key);
    }
}

/// Derived `Hash` (`#[non_exhaustive]`).
/// Every field (`#[non_exhaustive]`, so a field added later is listed here by hand): the screen rect, the document
/// point at its centre, the zoom, the turn, the mirror and the pixel aspect.
impl MemoProps for craft_behavior::canvas::CanvasView {
    fn key(&self, key: &mut MemoKey) {
        self.rect.key(key);
        for value in [self.center.x, self.center.y, self.zoom, self.rotation, self.aspect] {
            value.key(key);
        }
        self.flip_x.key(key);
    }
}

/// Every field (`#[non_exhaustive]`, so a field added later is listed here by hand): the rungs, the greatest zoom, the
/// least zoom and the device zoom of 100 %.
impl MemoProps for craft_behavior::ZoomScheme {
    fn key(&self, key: &mut MemoKey) {
        self.rungs.key(key);
        self.max.key(key);
        match self.floor {
            craft_behavior::ZoomFloor::ShorterSide(side) => {
                key.write_u32(0);
                side.key(key);
            }
            craft_behavior::ZoomFloor::Fixed(zoom) => {
                key.write_u32(1);
                zoom.key(key);
            }
            other => format!("{other:?}").key(key),
        }
        self.hundred.key(key);
    }
}

/// The width and the height.
impl MemoProps for craft_behavior::kurbo::Size {
    fn key(&self, key: &mut MemoKey) {
        self.width.key(key);
        self.height.key(key);
    }
}

/// Derived `Hash` (`#[non_exhaustive]`).
impl MemoProps for craft_behavior::Axis {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

/// Derived `Hash` (`#[non_exhaustive]`).
impl MemoProps for craft_behavior::overlay::Side {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

/// Derived `Hash` (`#[non_exhaustive]`).
impl MemoProps for craft_behavior::PixelSize {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

impl MemoProps for craft_behavior::fit::FitKeep {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

/// Derived `Hash` (`#[non_exhaustive]`).
impl MemoProps for craft_host::target::Os {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

/// Derived `Hash` (`#[non_exhaustive]`).
impl MemoProps for craft_host::target::WindowKind {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

/// Derived `Hash` (the layer's order and id).
impl MemoProps for crate::runtime::egui::areas::LayerKey {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

/// Derived `Hash` (an opaque hash value).
impl MemoProps for egui::Id {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(self);
    }
}

impl MemoProps for egui::Vec2 {
    fn key(&self, key: &mut MemoKey) {
        let egui::Vec2 { x, y } = *self;
        (x, y).key(key);
    }
}

impl MemoProps for egui::Pos2 {
    fn key(&self, key: &mut MemoKey) {
        let egui::Pos2 { x, y } = *self;
        (x, y).key(key);
    }
}

impl MemoProps for egui::Rect {
    fn key(&self, key: &mut MemoKey) {
        let egui::Rect { min, max } = *self;
        (min, max).key(key);
    }
}

impl MemoProps for egui::Color32 {
    fn key(&self, key: &mut MemoKey) {
        key.write(&self.to_array());
    }
}
