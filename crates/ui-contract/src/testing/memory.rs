//! The memory services the trials check: a stage fixture that implements `Remember`, eviction, `forget` and
//! the id derivations.

use crate::runtime::egui::{child, memory as store};
use crate::runtime::memory::{self, Facts, MemoryStage, SaveKey, StageAccess, register_owner, sealed};
use egui::{Context, Id, Ui};
use std::fmt::Debug;
use std::hash::Hash;

/// A memory stage opened the way the pipeline opens one at step 3: the owner is registered, then the stage
/// opens for `base`. `finish` is the write-back of step 17. It implements `Remember` over `base`.
pub struct Stage {
    stage: MemoryStage,
    base: Id,
}

impl Stage {
    /// A stage whose entries belong to `base`, which is registered as its own owner.
    pub fn open(ctx: &Context, base: Id) -> Self {
        Self::open_owned(ctx, base, base)
    }

    /// A stage keyed by `base` whose entries belong to `owner`, registered here.
    pub fn open_owned(ctx: &Context, base: Id, owner: Id) -> Self {
        register_owner(ctx, owner);
        Stage { stage: MemoryStage::open_owned(ctx, base, owner), base }
    }

    /// The stage of a sizing pass: it reads kept values and writes nothing.
    pub fn open_peek(ctx: &Context, base: Id) -> Self {
        Stage { stage: MemoryStage::open_peek(ctx, base), base }
    }

    /// Writes every slot back to context data.
    pub fn finish(self) {
        self.stage.write_back();
    }
}

impl sealed::Sealed for Stage {}

impl StageAccess for Stage {
    fn stage(&self) -> &MemoryStage {
        &self.stage
    }

    fn stage_mut(&mut self) -> &mut MemoryStage {
        &mut self.stage
    }

    fn base(&self) -> Id {
        self.base
    }
}

/// Removes the session slots under `id`.
pub fn forget(ctx: &Context, id: Id) {
    memory::forget(ctx, id);
}

/// Removes the saved slots of `key`.
pub fn forget_saved(ctx: &Context, key: &SaveKey) {
    memory::forget_saved(ctx, key);
}

/// Drops the slots the end hook evicted.
pub fn dispose_evicted(ctx: &Context) {
    memory::dispose_evicted(ctx);
}

/// Stores `value` in context data under `id`, as a slot's `Drop` may.
pub fn put<T: std::any::Any + Clone + Send + Sync>(ctx: &Context, id: Id, value: T) {
    store::put(ctx, id, value);
}

/// The runtime sub-id of `base` for `tag`.
pub fn sub(base: Id, tag: impl Hash + Debug) -> Id {
    memory::sub(base, tag)
}

/// The id of a runtime singleton named `name` under `scope`.
pub fn global(name: &'static str, scope: impl Hash + Debug) -> Id {
    memory::global(name, scope)
}

/// Registers `fact` for the next pass of the running viewport.
pub fn register_fact<T: Clone + Send + Sync + 'static>(ctx: &Context, fact: T) {
    Facts::register(ctx, fact);
}

/// The facts the running viewport's previous pass registered.
pub fn facts_of_last_pass<T: Clone + Send + Sync + 'static>(ctx: &Context) -> Vec<T> {
    Facts::of_last_pass(ctx)
}

/// Runs `f` on a child `Ui` of `ui`.
pub fn scoped<R>(ui: &mut Ui, f: impl FnOnce(&mut Ui) -> R) -> R {
    child::scoped(ui, f)
}

/// Runs `f` on a child `Ui` of `ui` with its own id scope named by `salt`.
pub fn scoped_by<R>(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, f: impl FnOnce(&mut Ui) -> R) -> R {
    crate::runtime::egui::memory::scoped_by(ui, salt, f)
}
