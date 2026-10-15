use craft_gpu::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, GpuProvider};

/// Placeholder until readback/composite WGSL moves here from app gpu crates.
pub struct NoopProvider;

impl GpuProvider for NoopProvider {
    fn id(&self) -> &'static str {
        "craft.noop"
    }

    fn run(&self, _ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<GpuJobOutcome, GpuFault> {
        Ok(GpuJobOutcome { outputs: job.outputs.clone(), data: None })
    }
}
