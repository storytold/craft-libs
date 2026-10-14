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
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn GpuProvider>) {
        self.providers.push(provider);
    }

    pub fn dispatch(
        &self,
        ctx: &mut dyn GpuContext,
        job: &GpuJob,
    ) -> Result<GpuJobOutcome, GpuFault> {
        for provider in &self.providers {
            if provider.id() == job.provider_id {
                return provider.run(ctx, job);
            }
        }
        Err(GpuFault::UnknownProvider(job.provider_id.clone()))
    }

    pub fn dispatch_batch(
        &self,
        ctx: &mut dyn GpuContext,
        jobs: &[GpuJob],
    ) -> Result<Vec<GpuJobOutcome>, GpuFault> {
        let mut out = Vec::with_capacity(jobs.len());
        for job in jobs {
            out.push(self.dispatch(ctx, job)?);
        }
        Ok(out)
    }
}
