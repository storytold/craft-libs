use craft_gpu::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, ProviderRegistry};

/// Generic entry point for worker hosts (SharedWorker RPC handler).
pub fn dispatch_gpu_jobs(registry: &ProviderRegistry, ctx: &mut dyn GpuContext, jobs: &[GpuJob]) -> Result<Vec<GpuJobOutcome>, GpuFault> {
    registry.dispatch_batch(ctx, jobs)
}
