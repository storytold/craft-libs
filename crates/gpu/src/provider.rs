use crate::{GpuContext, GpuFault, GpuJob, GpuJobOutcome};

/// One GPU operation family. App and generic providers implement this trait.
pub trait GpuProvider {
    fn id(&self) -> &'static str;

    fn run(&self, ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<GpuJobOutcome, GpuFault>;
}
