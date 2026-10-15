use craft_gpu::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, GpuJobParams, GpuProvider};

/// CPU stand-in for FilmCraft mask composite until WGSL runs on wgpu.
pub struct MaskProvider;

impl GpuProvider for MaskProvider {
    fn id(&self) -> &'static str {
        "craft.mask"
    }

    fn run(&self, ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<GpuJobOutcome, GpuFault> {
        let width = parse_u32(&job.params, b"w:").unwrap_or(1).max(1) as usize;
        let height = parse_u32(&job.params, b"h:").unwrap_or(1).max(1) as usize;
        let len = width.saturating_mul(height).saturating_mul(4);
        let (base, mask) = match job.inputs.as_slice() {
            [a, b] => (a, b),
            [a] => (a, a),
            _ => return Err(GpuFault::InvalidJob("craft.mask expects one or two RGBA inputs".into())),
        };
        let base_bytes = ctx.read_buffer(*base)?;
        let mask_bytes = ctx.read_buffer(*mask)?;
        if base_bytes.len() < len || mask_bytes.len() < len {
            return Err(GpuFault::InvalidJob("input buffer shorter than w*h*4".into()));
        }
        let mut out = vec![0u8; len];
        for i in 0..len {
            let m = mask_bytes[i];
            out[i] = ((u16::from(base_bytes[i]) * u16::from(m)) / 255) as u8;
        }
        let handle = ctx.upload_buffer("craft.mask.out", &out)?;
        Ok(GpuJobOutcome { outputs: vec![handle], data: None })
    }
}

fn parse_u32(params: &GpuJobParams, prefix: &[u8]) -> Option<u32> {
    let bytes = params.0.as_slice();
    let start = bytes.windows(prefix.len()).position(|w| w == prefix)? + prefix.len();
    let rest = &bytes[start..];
    let end = rest.iter().position(|&b| b == b',' || b == b';').unwrap_or(rest.len());
    std::str::from_utf8(&rest[..end]).ok()?.parse().ok()
}
