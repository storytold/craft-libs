use craft_gpu::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, GpuProvider};

/// Concatenates job params with bytes from the first input handle (shared test / debug provider).
pub struct EchoProvider;

impl GpuProvider for EchoProvider {
    fn id(&self) -> &'static str {
        "craft.test.echo"
    }

    fn run(&self, ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<GpuJobOutcome, GpuFault> {
        let mut data = job.params.0.clone();
        if let Some(&h) = job.inputs.first() {
            data.extend_from_slice(&ctx.read_buffer(h)?);
        }
        Ok(GpuJobOutcome {
            outputs: job.outputs.clone(),
            data: Some(data),
        })
    }
}
