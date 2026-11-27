//! Derived values: data a component computes from its inputs and keeps across passes. [`derived`] keeps one value per
//! component id and value type, and builds it again only when its key changes. The key implements
//! [`MemoProps`], so it can only be made of checked inputs (`Tracked` revisions, framework
//! revisions, plain values), and a kept value is never stale while the key compiles.
//!
//! A value no pass asks for is dropped by the rule memoized views follow for absent entries (`lazy::absent_past_room`).

use crate::contract::{MemoKey, MemoProps};
use crate::runtime::egui::pass;
use crate::runtime::frame::PerViewport;
use egui::{Context, Id, IdMap};
use std::any::{Any, TypeId};
use std::sync::Arc;

/// A kept value: the key it was built from and the frame a pass last asked for it.
struct Entry {
    key: u128,
    value: Arc<dyn Any + Send + Sync>,
    touched: u64,
}

/// The kept values of one viewport.
#[derive(Default)]
pub(crate) struct Store {
    entries: IdMap<Entry>,
    /// The frame the entries were last pruned in.
    pruned: u64,
}

impl Store {
    /// Drops the absent entries past the memo rule's room, once per frame. An entry asked for in this frame or the one
    /// before is shown.
    fn prune(&mut self, frame: u64) {
        if self.pruned == frame {
            return;
        }
        self.pruned = frame;
        let shown = self.entries.values().filter(|entry| entry.touched + 1 >= frame).count();
        let absent = self.entries.iter().filter(|(_, entry)| entry.touched + 1 < frame).map(|(id, entry)| (entry.touched, *id));
        for id in super::lazy::absent_past_room(shown, absent) {
            self.entries.remove(&id);
        }
    }
}

/// The value of the component `id` derived from `key`: the kept one while `key` is the one it was built from, else
/// `build`'s, which is kept in its place. One value is kept per id and value type `V`; a component that derives two
/// values of one type gives each its own id.
///
/// While the key is unchanged, every call returns the same `Arc` (`Arc::ptr_eq` holds), so a caller that keeps the
/// last result can tell "unchanged" from a pointer compare.
pub fn derived<K: MemoProps, V: Send + Sync + 'static>(ctx: &Context, id: Id, key: &K, build: impl FnOnce() -> V) -> Arc<V> {
    let mut memo = MemoKey::of_view(ctx);
    memo.feed_hash(&K::identity());
    key.key(&mut memo);
    #[cfg(feature = "testing")]
    crate::runtime::measure::count_memo_key_bytes(ctx, memo.bytes());
    let key = memo.finish();
    let slot = id.with(TypeId::of::<V>());
    crate::runtime::replay::note_derived_keep(ctx, slot);
    let viewport = pass::current_viewport(ctx);
    let frame = pass::frame_nr(ctx, viewport);
    let kept = PerViewport::<Store>::with(ctx, viewport, |store| {
        store.prune(frame);
        let entry = store.entries.get_mut(&slot).filter(|entry| entry.key == key)?;
        entry.touched = frame;
        Arc::clone(&entry.value).downcast::<V>().ok()
    });
    if let Some(value) = kept {
        return value;
    }
    #[cfg(feature = "testing")]
    crate::runtime::measure::count_derived(ctx, id);
    // Built outside the store, so a build may derive values of its own.
    let value = Arc::new(build());
    let entry = Entry { key, value: Arc::clone(&value) as Arc<dyn Any + Send + Sync>, touched: frame };
    PerViewport::<Store>::with(ctx, viewport, |store| store.entries.insert(slot, entry));
    value
}

/// Keeps the values under `slots` through this frame, for a replayed run that asked for them: the replay asks for
/// nothing, and an entry no pass asked for in a frame is dropped by the memo rule.
pub(crate) fn keep(ctx: &Context, slots: &[Id]) {
    if slots.is_empty() {
        return;
    }
    let viewport = pass::current_viewport(ctx);
    let frame = pass::frame_nr(ctx, viewport);
    PerViewport::<Store>::with(ctx, viewport, |store| {
        for slot in slots {
            if let Some(entry) = store.entries.get_mut(slot) {
                entry.touched = frame;
            }
        }
    });
}
