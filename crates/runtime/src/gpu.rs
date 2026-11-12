use craft_gpu::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, ProviderRegistry};

/// Generic entry point for worker hosts (SharedWorker RPC handler).
///
/// Per job, in input order: a later job's failure never discards an earlier job's outcome.
/// Forwards [`ProviderRegistry::dispatch_batch`]'s own per-job contract unchanged; the caller
/// (`Runtime::handle_host_message`) maps this to the wire's [`craft_worker_host::GpuJobResult`].
pub fn dispatch_gpu_jobs(registry: &ProviderRegistry, ctx: &mut dyn GpuContext, jobs: &[GpuJob]) -> Vec<Result<GpuJobOutcome, GpuFault>> {
    registry.dispatch_batch(ctx, jobs)
}
