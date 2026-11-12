use crate::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, GpuProvider};

/// Lookup table filled at worker/app init. Generic runtime code only talks to this.
pub struct ProviderRegistry {
    providers: Vec<Box<dyn GpuProvider>>,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self { providers: Vec::new() }
    }

    pub fn register(&mut self, provider: Box<dyn GpuProvider>) {
        self.providers.push(provider);
    }

    pub fn dispatch(&self, ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<GpuJobOutcome, GpuFault> {
        for provider in &self.providers {
            if provider.id() == job.provider_id {
                return provider.run(ctx, job);
            }
        }
        Err(GpuFault::UnknownProvider(job.provider_id.clone()))
    }

    /// Run every job in order and answer per job, in input order. A later job's failure never
    /// discards outcomes earlier providers already produced: the caller replies per job and can
    /// retry exactly the failed ones. A job whose inputs came from a failed earlier job fails
    /// honestly on its own bad handles.
    pub fn dispatch_batch(&self, ctx: &mut dyn GpuContext, jobs: &[GpuJob]) -> Vec<Result<GpuJobOutcome, GpuFault>> {
        jobs.iter().map(|job| self.dispatch(ctx, job)).collect()
    }
}
