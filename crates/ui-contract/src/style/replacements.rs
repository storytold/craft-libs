//! App style replacements, type-erased so the runtime names no contract type.

use super::cx::StyleCx;
use super::key::StyleKey;
use super::look::ComponentStyle;
use crate::runtime::egui::memory::Service;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// A replacement style function: the same `StyleCx` and key, the same style type.
type Replacement<K, S> = for<'a> fn(&StyleCx<'a>, K) -> S;

/// The replacements an app asks for, collected before `runtime::install` freezes them.
#[derive(Default)]
pub struct StyleReplacements {
    entries: Vec<(TypeId, &'static str, Arc<dyn Any + Send + Sync>)>,
}

impl StyleReplacements {
    /// No replacements.
    pub const fn new() -> Self {
        StyleReplacements { entries: Vec::new() }
    }

    /// Replaces the style function of the component whose style key is `K`. Two replacements for one key are an
    /// install error.
    pub fn push<K: StyleKey, S: ComponentStyle>(&mut self, f: Replacement<K, S>) -> &mut Self {
        self.entries.push((TypeId::of::<K>(), std::any::type_name::<K>(), Arc::new(f)));
        self
    }

    /// The map `install` stores in context data.
    pub(crate) fn freeze(self) -> Result<Frozen, DuplicateStyle> {
        let mut map = HashMap::new();
        for (ty, name, f) in self.entries {
            if map.insert(ty, f).is_some() {
                return Err(DuplicateStyle(name));
            }
        }
        Ok(Frozen { map })
    }
}

/// Two replacements named one style key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DuplicateStyle(pub &'static str);

/// The frozen replacements of a context.
#[derive(Default)]
pub(crate) struct Frozen {
    map: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Frozen {
    /// The replacement for key type `K`, if the app made one.
    pub(crate) fn get<K: StyleKey, S: ComponentStyle>(&self) -> Option<Replacement<K, S>> {
        self.map.get(&TypeId::of::<K>()).and_then(|f| f.downcast_ref::<Replacement<K, S>>()).copied()
    }

    /// Stores `self` as the context's replacements.
    pub(crate) fn store(self, ctx: &egui::Context) {
        Service::<Frozen>::run(ctx, |slot| *slot = self);
    }

    /// The replacement for `K` in `ctx`.
    pub(crate) fn of<K: StyleKey, S: ComponentStyle>(ctx: &egui::Context) -> Option<Replacement<K, S>> {
        Service::<Frozen>::run(ctx, |frozen| frozen.get::<K, S>())
    }
}
