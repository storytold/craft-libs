//! Context data: `IdTypeMap` temp entries, the per-service state behind one lock each, and id minting.

use egui::util::IdTypeMap;
use egui::util::id_type_map::RawKey;
use egui::{Context, Id};
use std::any::Any;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Context data: the map the runtime's slots live in.
pub(crate) type Map = IdTypeMap;

/// A boxed entry moved out of context data. The caller drops it later, outside every lock.
pub(crate) type Removed = Box<dyn Any + Send + Sync>;

/// Moves one typed slot out of the map as the box it is stored in. `IdTypeMap::remove_temp::<T>` would run
/// `T::default()` and drop the placeholder under the data lock; the raw removal runs no app code.
pub(crate) fn take_raw<T: Any + Send + Sync>(data: &mut Map, id: Id) -> Option<Removed> {
    data.remove_temp_raw(RawKey::new::<T>(id))
}

/// Moves the slot of type `T` under `id` out of context data as the box it is stored in.
pub(crate) fn take_boxed<T: Any + Send + Sync>(ctx: &Context, id: Id) -> Option<Removed> {
    ctx.data_mut(|data| take_raw::<T>(data, id))
}

/// Moves the slot of type `T` under `id` out of context data.
pub(crate) fn take<T: Any + Send + Sync>(ctx: &Context, id: Id) -> Option<T> {
    let boxed = take_boxed::<T>(ctx, id)?;
    boxed.downcast::<T>().ok().map(|value| *value)
}

/// Stores `value` under `id`. The entry it replaces is dropped after the data lock is released.
pub(crate) fn put<T: Any + Clone + Send + Sync>(ctx: &Context, id: Id, value: T) {
    let replaced = ctx.data_mut(|data| {
        let old = take_raw::<T>(data, id);
        data.insert_temp(id, value);
        old
    });
    drop(replaced);
}

/// A copy of the plain value under `id`, read in place: `Copy` means no slot code runs under the data lock.
pub(crate) fn peek_copy<T: Any + Copy + Send + Sync>(ctx: &Context, id: Id) -> Option<T> {
    ctx.data(|data| data.get_temp::<T>(id))
}

/// Stores `mark` under `id` unless a value of its type is there already; true when it stored it.
pub(crate) fn mark_seen<T: Any + Clone + Send + Sync>(data: &mut Map, id: Id, mark: T) -> bool {
    let absent = data.get_temp::<T>(id).is_none();
    if absent {
        data.insert_temp(id, mark);
    }
    absent
}

/// Puts a taken-out box back under `id`, in the allocation it left in. The value it replaced, if any, is returned
/// to drop outside the lock.
pub(crate) fn put_boxed<T: Any + Clone + Send + Sync>(data: &mut Map, id: Id, value: Box<T>) -> Option<Removed> {
    data.insert_temp_boxed(id, value)
}

/// A copy of the slot under `id` without running slot code under the data lock: the entry moves out, is
/// cloned and moves back.
pub(crate) fn peek_clone<T: Any + Clone + Send + Sync>(ctx: &Context, id: Id) -> Option<T> {
    let moved = take::<T>(ctx, id)?;
    let copy = moved.clone();
    ctx.data_mut(|data| data.insert_temp(id, moved));
    Some(copy)
}

/// Whether a slot of type `T` is stored under `id`.
pub(crate) fn exists<T: Any + Send + Sync>(ctx: &Context, id: Id) -> bool {
    ctx.data(|data| data.get_temp_raw(RawKey::new::<T>(id)).is_some())
}

/// Runs `f` on the context's data under its lock; `f` runs no app code and makes no call that reaches the context.
pub(crate) fn locked<R>(ctx: &Context, f: impl FnOnce(&mut Map) -> R) -> R {
    ctx.data_mut(f)
}

/// The id derived from `source` alone, with no parent.
pub(crate) fn unique(source: impl Hash + std::fmt::Debug) -> Id {
    Id::unique(source)
}

/// A service's state in context data. Cloning shares the state.
///
/// The context's data lock is held only to fetch the handle. The service's own lock is held while `with`
/// runs its closure, so the closure runs no app code and makes no call that reaches the context (7.2, the
/// lock rule).
#[derive(Default)]
pub(crate) struct Service<S>(Arc<Mutex<S>>);

impl<S> Clone for Service<S> {
    fn clone(&self) -> Self {
        Service(Arc::clone(&self.0))
    }
}

/// A multiply-and-rotate hasher for keys that are already hashes or ids: the maps of the runtime hold no
/// attacker-chosen keys.
#[derive(Default)]
pub(crate) struct FastHasher(u64);

impl FastHasher {
    #[inline]
    fn mix(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.mix(u64::from_le_bytes(word));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.mix(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.mix(i);
    }
}

/// A `HashMap` over [`FastHasher`].
pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[cfg(feature = "testing")]
thread_local! {
    static LOOKUPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many service lookups in context data this thread has made: each one takes the data lock. Pass-local
/// state (`pass_state`) makes none inside a pass.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn service_lookups() -> u64 {
    LOOKUPS.with(std::cell::Cell::get)
}

impl<S: Default + Send + 'static> Service<S> {
    /// The context's service of type `S`, created on first use.
    pub(crate) fn of(ctx: &Context) -> Service<S> {
        #[cfg(feature = "testing")]
        LOOKUPS.with(|lookups| lookups.set(lookups.get() + 1));
        static SLOT: OnceLock<Id> = OnceLock::new();
        let slot = *SLOT.get_or_init(|| unique("craft_ui.service"));
        locked(ctx, |data| data.get_temp_mut_or_default::<Service<S>>(slot).clone())
    }

    /// Runs `f` on the context's service of type `S`, created on first use.
    pub(crate) fn run<R>(ctx: &Context, f: impl FnOnce(&mut S) -> R) -> R {
        Self::of(ctx).with(f)
    }

    /// Runs `f` on the state under the service's lock.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut S) -> R) -> R {
        #[cfg(feature = "testing")]
        crate::runtime::layout_memory::guard::touch::<S>();
        f(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Runs `f` on the context's service of type `S`.
#[doc(hidden)]
pub fn with<S: Default + Send + 'static, R>(ctx: &Context, f: impl FnOnce(&mut S) -> R) -> R {
    Service::<S>::run(ctx, f)
}

/// Runs `f` on a child `Ui` of `ui` whose auto ids start from `salt`: equal widgets under different salts never meet.
#[cfg(feature = "testing")]
pub(crate) fn scoped_by<R>(ui: &mut egui::Ui, salt: impl std::hash::Hash + std::fmt::Debug, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.push_id(salt, f).inner
}

/// A hidden sizing-pass child of `ui` whose auto ids start from `salt`.
#[cfg(feature = "testing")]
pub(crate) fn sizing_child(ui: &mut egui::Ui, salt: &'static str) -> egui::Ui {
    ui.new_child(egui::UiBuilder::new().id_salt(salt).sizing_pass().invisible())
}
