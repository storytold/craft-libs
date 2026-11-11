//! The style cache and variant acceptance.

use super::cx::StyleCx;
use super::key::{EnvKey, StyleKey};
use super::look::ComponentStyle;
use super::replacements::Frozen;
use crate::runtime::egui::memory::Service;
use craft_theme::{ControlSize, Variant};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// The variant a component takes for a requested one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Accepted {
    /// The variant the style is built for.
    pub variant: Variant,
    /// The requested variant was not accepted, so the default stands in.
    pub fell_back: bool,
}

/// A component accepts its `VARIANTS` plus every custom variant built for its style key. Any other variant gives
/// the default, the first of `VARIANTS`; a component with none takes every other request as the default look.
pub fn accepted_variant<K: StyleKey, S: ComponentStyle>(requested: Variant) -> Accepted {
    let custom = matches!(requested, Variant::Custom(custom) if custom.key_type() == TypeId::of::<K>());
    if custom || S::VARIANTS.contains(&requested) {
        return Accepted { variant: requested, fell_back: false };
    }
    match S::VARIANTS.first().copied() {
        Some(default) => Accepted { variant: default, fell_back: true },
        None => Accepted { variant: Variant::SECONDARY, fell_back: false },
    }
}

/// `StyleKey` erased: equality and hashing through `Any`, so one map serves every component.
trait ErasedKey: Send + Sync {
    fn eq_dyn(&self, other: &dyn Any) -> bool;
}

impl<K: StyleKey> ErasedKey for K {
    fn eq_dyn(&self, other: &dyn Any) -> bool {
        other.downcast_ref::<K>().is_some_and(|other| other == self)
    }
}

struct Entry {
    ty: TypeId,
    style_ty: TypeId,
    key: Box<dyn ErasedKey>,
    env: EnvKey,
    variant: Variant,
    size: ControlSize,
    style: Arc<dyn Any + Send + Sync>,
}

fn hash_of<K: StyleKey, S: ComponentStyle>(key: &K, env: EnvKey, variant: Variant, size: ControlSize) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    TypeId::of::<K>().hash(&mut hasher);
    TypeId::of::<S>().hash(&mut hasher);
    key.hash(&mut hasher);
    env.hash(&mut hasher);
    variant.hash(&mut hasher);
    size.hash(&mut hasher);
    hasher.finish()
}

/// The cache of one context: buckets by hash, each holding the few entries that share one.
#[derive(Default)]
struct StyleCache {
    buckets: HashMap<u64, Vec<Entry>>,
    fallbacks: HashSet<(TypeId, Variant)>,
}

impl StyleCache {
    fn get<K: StyleKey, S: ComponentStyle>(&self, hash: u64, key: &K, env: EnvKey, variant: Variant, size: ControlSize) -> Option<Arc<S>> {
        let entry = self.buckets.get(&hash)?.iter().find(|e| {
            e.ty == TypeId::of::<K>()
                && e.style_ty == TypeId::of::<S>()
                && e.env == env
                && e.variant == variant
                && e.size == size
                && e.key.eq_dyn(key)
        })?;
        entry.style.clone().downcast::<S>().ok()
    }
}

/// The style of a component: the cached one for `(TypeId::of::<K>(), TypeId::of::<S>(), key, EnvKey, variant, size)`, else the app's
/// replacement or `default` run on a miss. A hit costs one lock, one hash lookup and one `Arc` clone; the style
/// function runs outside the lock and its result is inserted after it returns.
#[doc(hidden)]
pub fn resolve_style<K: StyleKey, S: ComponentStyle>(
    cx: &StyleCx<'_>,
    key: K,
    ctx: &egui::Context,
    default: for<'a> fn(&StyleCx<'a>, K) -> S,
) -> Arc<S> {
    let accepted = accepted_variant::<K, S>(cx.variant);
    let (env, size) = (cx.key, cx.size);
    let hash = hash_of::<K, S>(&key, env, accepted.variant, size);
    let service = Service::<StyleCache>::of(ctx);
    if let Some(hit) = service.with(|cache| cache.get::<K, S>(hash, &key, env, accepted.variant, size)) {
        return hit;
    }
    let build = Frozen::of::<K, S>(ctx).unwrap_or(default);
    let style = Arc::new(build(&cx.with_variant(accepted.variant), key));
    let entry =
        Entry { ty: TypeId::of::<K>(), style_ty: TypeId::of::<S>(), key: Box::new(key), env, variant: accepted.variant, size, style: style.clone() };
    service.with(|cache| {
        if accepted.fell_back && cache.fallbacks.insert((TypeId::of::<K>(), cx.variant)) {
            log::warn!("{} does not accept the variant {:?}; the default look stands in", std::any::type_name::<K>(), cx.variant);
        }
        let bucket = cache.buckets.entry(hash).or_default();
        match bucket.iter_mut().find(|e| {
            e.ty == entry.ty && e.style_ty == entry.style_ty && e.env == env && e.variant == entry.variant && e.size == size && e.key.eq_dyn(&key)
        }) {
            Some(slot) => *slot = entry,
            None => bucket.push(entry),
        }
    });
    style
}
