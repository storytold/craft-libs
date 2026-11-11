//! App-defined env values: EnvValue, StyleValue and AppValues.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// An app-defined value inherited down the tree like the library's own.
pub trait EnvValue: Clone + PartialEq + Send + Sync + 'static {
    /// A value that changes how components look is folded into the style key, so cached styles follow it.
    const AFFECTS_STYLE: bool = false;

    /// The value where nothing above sets one; computed at its first `get`, outside every hook.
    fn root() -> Self;

    /// The hash of the value, folded into `EnvKey` when `AFFECTS_STYLE`. Written by `#[env(style)]`; a value
    /// that does not affect style returns `None`.
    #[doc(hidden)]
    fn style_hash(&self) -> Option<u64> {
        None
    }
}

/// Marker for values `style` may read. `Hash`, because the value is folded into the style key.
pub trait StyleValue: EnvValue + Hash {}

/// The app values a root, a scope or a component's `content_env` carries: one entry per `EnvValue` type, the
/// innermost wins.
#[derive(Clone, Default)]
pub struct AppValues {
    entries: Arc<ValueEntries>,
}

#[derive(Clone, Default)]
struct ValueEntries {
    map: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// The hash of each style-affecting value, by type.
    style: HashMap<TypeId, u64>,
    /// The hash of every style-affecting value, independent of the order they were set in.
    style_hash: u64,
}

impl ValueEntries {
    fn rehash(&mut self) {
        self.style_hash = self
            .style
            .iter()
            .map(|(ty, hash)| {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                (ty, hash).hash(&mut hasher);
                hasher.finish()
            })
            .fold(0, u64::wrapping_add);
    }
}

impl AppValues {
    /// Sets the value of type `T`.
    pub fn set<T: EnvValue>(&mut self, value: T) -> &mut Self {
        debug_assert!(
            !T::AFFECTS_STYLE || value.style_hash().is_some(),
            "{} sets AFFECTS_STYLE but gives no style hash: use `#[env(style)]`",
            std::any::type_name::<T>()
        );
        let entries = Arc::make_mut(&mut self.entries);
        match value.style_hash().filter(|_| T::AFFECTS_STYLE) {
            Some(hash) => entries.style.insert(TypeId::of::<T>(), hash),
            None => entries.style.remove(&TypeId::of::<T>()),
        };
        entries.map.insert(TypeId::of::<T>(), Arc::new(value));
        entries.rehash();
        self
    }

    /// The hash of every style-affecting value, independent of the order they were set in.
    pub(crate) fn style_hash(&self) -> u64 {
        self.entries.style_hash
    }

    /// A clone of the value, else `T::root()`.
    pub(crate) fn get<T: EnvValue>(&self) -> T {
        self.entries.map.get(&TypeId::of::<T>()).and_then(|value| value.downcast_ref::<T>()).cloned().unwrap_or_else(T::root)
    }

    /// Takes every value `other` sets over this set's.
    pub(crate) fn extend(&mut self, other: &AppValues) {
        if other.entries.map.is_empty() {
            return;
        }
        let entries = Arc::make_mut(&mut self.entries);
        for (ty, value) in &other.entries.map {
            entries.map.insert(*ty, Arc::clone(value));
            match other.entries.style.get(ty) {
                Some(hash) => entries.style.insert(*ty, *hash),
                None => entries.style.remove(ty),
            };
        }
        entries.rehash();
    }
}
