//! `Inner::settle`: the body's memory, handed to the caller once after the real run.

use crate::runtime::memory::{MemoryStage, StageAccess, sealed};
use egui::{Context, Id};

/// Where a real run left the body's memory. Set only when a real run called the body.
#[derive(Debug)]
#[doc(hidden)]
pub struct SettleAt {
    pub(crate) ctx: Context,
    pub(crate) body: Id,
    pub(crate) owner: Id,
}

impl SettleAt {
    pub(crate) fn run<T>(&self, f: impl FnOnce(&mut SettleCx<'_>) -> T) -> T {
        let mut stage = MemoryStage::open_after(&self.ctx, self.body, self.owner);
        let value = f(&mut SettleCx { stage: &mut stage, id: self.body });
        stage.write_back();
        value
    }
}

/// The body's memory after the real run: the same `Remember` access the body's context has, and the `Held` and
/// `Bind` reads and writes that take it. It claims no stage and does not mark the body as seen.
pub struct SettleCx<'a> {
    stage: &'a mut MemoryStage,
    id: Id,
}

impl sealed::Sealed for SettleCx<'_> {}

impl StageAccess for SettleCx<'_> {
    fn stage(&self) -> &MemoryStage {
        self.stage
    }

    fn stage_mut(&mut self) -> &mut MemoryStage {
        self.stage
    }

    fn base(&self) -> Id {
        self.id
    }
}
