use craft_gpu::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, GpuProvider};

use crate::params::parse_u32;

/// CPU stand-in for FilmCraft `composite.wgsl` Normal/Dissolve premultiplied "over" until WGSL runs on wgpu.
pub struct PremulCompositeProvider;

impl GpuProvider for PremulCompositeProvider {
    fn id(&self) -> &'static str {
        "craft.composite.premul"
    }

    fn run(&self, ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<GpuJobOutcome, GpuFault> {
        let width = parse_u32(&job.params, b"w:").unwrap_or(1).max(1) as usize;
        let height = parse_u32(&job.params, b"h:").unwrap_or(1).max(1) as usize;
        let len = width.saturating_mul(height).saturating_mul(4);
        let (dst_h, src_h) = match job.inputs.as_slice() {
            [a, b] => (a, b),
            [a] => (a, a),
            _ => {
                return Err(GpuFault::InvalidJob(
                    "craft.composite.premul expects one or two RGBA8 inputs (backdrop, layer)".into(),
                ));
            }
        };
        let dst_bytes = ctx.read_buffer(*dst_h)?;
        let src_bytes = ctx.read_buffer(*src_h)?;
        if dst_bytes.len() < len || src_bytes.len() < len {
            return Err(GpuFault::InvalidJob("input buffer shorter than w*h*4".into()));
        }
        let mut out = vec![0u8; len];
        for px in 0..width.saturating_mul(height) {
            let i = px * 4;
            let mut dst = straight_to_premul([
                dst_bytes[i],
                dst_bytes[i + 1],
                dst_bytes[i + 2],
                dst_bytes[i + 3],
            ]);
            let src = straight_to_premul([
                src_bytes[i],
                src_bytes[i + 1],
                src_bytes[i + 2],
                src_bytes[i + 3],
            ]);
            premul_over(&mut dst, src);
            let straight = premul_to_straight(dst);
            out[i..i + 4].copy_from_slice(&straight);
        }
        let handle = ctx.upload_buffer("craft.composite.premul.out", &out)?;
        Ok(GpuJobOutcome { outputs: vec![handle], data: None })
    }
}

/// Premultiplied `src` over premultiplied `dst` (`filmcraft_render` / `composite.wgsl` Normal path).
fn premul_over(dst: &mut [f32; 4], src: [f32; 4]) {
    let k = 1.0 - src[3];
    for c in 0..4 {
        dst[c] = src[c] + dst[c] * k;
    }
}

fn straight_to_premul(p: [u8; 4]) -> [f32; 4] {
    let a = f32::from(p[3]) / 255.0;
    let s = a / 255.0;
    [f32::from(p[0]) * s, f32::from(p[1]) * s, f32::from(p[2]) * s, a]
}

fn premul_to_straight(p: [f32; 4]) -> [u8; 4] {
    let a = p[3].clamp(0.0, 1.0);
    if a <= f32::EPSILON {
        return [0, 0, 0, 0];
    }
    let inv = 255.0 / a;
    [
        (p[0] * inv).round().clamp(0.0, 255.0) as u8,
        (p[1] * inv).round().clamp(0.0, 255.0) as u8,
        (p[2] * inv).round().clamp(0.0, 255.0) as u8,
        (a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}
