//! Long-block analysis transform, ETSI TS 102 366 §7.2.3.
use std::sync::OnceLock;

/// Analysis matrix includes the window and −2/N normalization (N=512).
/// Cached rather than evaluating trigonometric functions for each audio block.
fn matrix() -> &'static Vec<[f32; 512]> {
    static MATRIX: OnceLock<Vec<[f32; 512]>> = OnceLock::new();
    MATRIX.get_or_init(|| {
        let window = super::super::window();
        (0..256)
            .map(|k| {
                std::array::from_fn(|n| {
                    let w = if n < 256 { window[n] } else { window[511 - n] };
                    let phase = std::f64::consts::PI / 1024.0 * ((2 * n + 1) * (2 * k + 1)) as f64 + std::f64::consts::PI / 4.0 * (2 * k + 1) as f64;
                    (-phase.cos() * f64::from(w) / 256.0) as f32
                })
            })
            .collect()
    })
}

pub(super) fn analysis_long(input: &[f32; 512]) -> [f32; 256] {
    std::array::from_fn(|k| matrix()[k].iter().zip(input).map(|(&a, &b)| f64::from(a) * f64::from(b)).sum::<f64>() as f32)
}

/// Six-pole Butterworth low-pass at 120 Hz, before LFE analysis/downsampling.
/// Cascaded second-order sections retain stability and streaming continuity.
pub(super) struct LowPass {
    sections: [Biquad; 3],
}
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}
impl LowPass {
    pub(super) fn new(rate: u32) -> Self {
        let omega = std::f64::consts::TAU * 120.0 / f64::from(rate);
        Self {
            sections: [0.5176380902050415, std::f64::consts::FRAC_1_SQRT_2, 1.9318516525781366].map(|q| {
                let alpha = omega.sin() / (2.0 * q);
                let a0 = 1.0 + alpha;
                let b0 = (1.0 - omega.cos()) * 0.5 / a0;
                Biquad { b: [b0, 2.0 * b0, b0], a: [-2.0 * omega.cos() / a0, (1.0 - alpha) / a0], z: [0.0; 2] }
            }),
        }
    }
    pub(super) fn process(&mut self, input: f32) -> f32 {
        let mut value = f64::from(input);
        for section in &mut self.sections {
            let output = section.b[0] * value + section.z[0];
            section.z[0] = section.b[1] * value - section.a[0] * output + section.z[1];
            section.z[1] = section.b[2] * value - section.a[1] * output;
            value = output;
        }
        value as f32
    }
}

/// Short transforms use the same 512-sample window but two 256-point
/// cosine transforms with alpha = -1, +1, interleaved in the bitstream.
fn short_matrix() -> &'static Vec<[f32; 256]> {
    static MATRIX: OnceLock<Vec<[f32; 256]>> = OnceLock::new();
    MATRIX.get_or_init(|| {
        let window = super::super::window();
        (0..256)
            .map(|index| {
                let half = index % 2;
                let k = index / 2;
                std::array::from_fn(|n| {
                    let w = if half == 0 { window[n] } else { window[255 - n] };
                    let phase = std::f64::consts::PI / 512.0 * ((2 * n + 1) * (2 * k + 1)) as f64
                        + std::f64::consts::PI / 2.0 * (2 * k + 1) as f64 * half as f64;
                    (-phase.cos() * f64::from(w) / 128.0) as f32
                })
            })
            .collect()
    })
}

pub(super) fn analysis(input: &[f32; 512], short: bool) -> [f32; 256] {
    if !short {
        return analysis_long(input);
    }
    std::array::from_fn(|k| {
        let offset = (k % 2) * 256;
        short_matrix()[k].iter().zip(&input[offset..offset + 256]).map(|(&a, &b)| f64::from(a) * f64::from(b)).sum::<f64>() as f32
    })
}

/// Detect a substantial onset within the analysis window. Decisions are shared
/// by full-bandwidth channels so a compatible surround downmix can be formed in
/// the transform domain without mixing coefficients from different transforms.
pub(super) fn has_transient(input: &[f32; 512]) -> bool {
    let energy: Vec<f64> = input.as_chunks::<32>().0.iter().map(|chunk| chunk.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>()).collect();
    energy.windows(2).any(|e| e[1] > 0.001 && e[1] > (e[0] + 1e-8) * 8.0)
}
