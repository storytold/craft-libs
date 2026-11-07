use craft_gpu::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, GpuProvider, chunk_buffer, f16_rgba_to_f32_bytes};

use crate::params::{params_contain, parse_u32};

/// CPU stand-in for accumulator/output readback (`read_accumulator` / `read_output` in FilmCraft gpu) until wgpu maps buffers.
pub struct ReadbackProvider;

impl GpuProvider for ReadbackProvider {
    fn id(&self) -> &'static str {
        "craft.readback"
    }

    fn run(&self, ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<GpuJobOutcome, GpuFault> {
        let width = parse_u32(&job.params, b"w:").unwrap_or(0);
        let height = parse_u32(&job.params, b"h:").unwrap_or(0);
        let input = job.inputs.first().copied().ok_or_else(|| GpuFault::InvalidJob("craft.readback expects one input buffer".into()))?;
        let raw = ctx.read_buffer(input)?;
        let caps = ctx.caps();
        let _chunks = chunk_buffer(&caps, &raw)?;

        let out_bytes = if params_contain(&job.params, b"fmt:u8") {
            let need = usize::try_from(width.saturating_mul(height).saturating_mul(4)).map_err(|_| GpuFault::InvalidJob("w*h overflow".into()))?;
            if need == 0 || raw.len() < need {
                return Err(GpuFault::InvalidJob("rgba8 readback shorter than w*h*4".into()));
            }
            raw[..need].to_vec()
        } else {
            let need = usize::try_from(width.saturating_mul(height).saturating_mul(8)).map_err(|_| GpuFault::InvalidJob("w*h overflow".into()))?;
            if need == 0 || raw.len() < need {
                return Err(GpuFault::InvalidJob("f16 rgba readback shorter than w*h*8".into()));
            }
            f16_rgba_to_f32_bytes(&raw[..need]).map_err(|len| GpuFault::InvalidJob(format!("f16 rgba length {len} not multiple of 8")))?
        };

        let handle = ctx.upload_buffer("craft.readback.out", &out_bytes)?;
        Ok(GpuJobOutcome { outputs: vec![handle], data: None })
    }
}
