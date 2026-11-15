//! The SBR filter banks (ISO/IEC 14496-3 §4.6.18.4): a 32-band complex QMF analysis bank at the
//! core rate and a 64-band complex QMF synthesis bank at the output rate, sharing the window `c`.
//!
//! Each bank is a polyphase window followed by a complex modulation: analysis
//! `2·exp(iπ(k + ½)(2n − ½)/64)`, synthesis `exp(iπ(k + ½)(2n − 251)/128)/64`. With the window's
//! indexing, the two phase offsets put the banks' combined response on its centre tap, so the low
//! band comes back unchanged (to about -60 dB, the window's own limit), 576 output samples late.
//! The synthesis bank holds back its last two output samples, which makes it 578: with the SBR
//! frame's lag, the 962 samples by which HE-AAC decoders (ffmpeg's among them) delay the core, and
//! which the priming encoders signal counts on.

use std::f64::consts::PI;
use std::sync::OnceLock;

use super::Cx;
use super::tables::QMF_WINDOW;

/// Output samples by which analysis followed by synthesis delays the low band.
pub(crate) const DELAY: usize = 578;

/// Modulation matrices, computed once.
struct Kernels {
    /// Analysis, at `[k][n]`: 32 × 64.
    analysis: Vec<[Cx; 64]>,
    /// Synthesis, at `[n][k]`: 128 × 64.
    synthesis: Vec<[Cx; 64]>,
}

fn kernels() -> &'static Kernels {
    static K: OnceLock<Kernels> = OnceLock::new();
    K.get_or_init(|| {
        let cx = |phase: f64, scale: f64| Cx { re: (phase.cos() * scale) as f32, im: (phase.sin() * scale) as f32 };
        let analysis = (0..32).map(|k| std::array::from_fn(|n| cx(PI * (k as f64 + 0.5) * (2.0 * n as f64 - 0.5) / 64.0, 2.0))).collect();
        let synthesis = (0..128).map(|n| std::array::from_fn(|k| cx(PI * (k as f64 + 0.5) * (2.0 * n as f64 - 251.0) / 128.0, 1.0 / 64.0))).collect();
        Kernels { analysis, synthesis }
    })
}

/// The 32-band analysis bank of one channel.
#[derive(Clone)]
pub(crate) struct Analysis {
    /// The last 320 input samples, newest first.
    x: Box<[f32; 320]>,
}

impl Analysis {
    pub(crate) fn new() -> Analysis {
        Analysis { x: Box::new([0.0; 320]) }
    }

    /// Split `32 · out.len()` input samples into `out.len()` slots of 32 subbands.
    pub(crate) fn process(&mut self, input: &[f32], out: &mut [[Cx; 32]]) {
        let k = kernels();
        for (block, slot) in input.as_chunks::<32>().0.iter().zip(out.iter_mut()) {
            self.x.copy_within(0..288, 32);
            for (n, &s) in block.iter().enumerate() {
                self.x[31 - n] = s;
            }
            let mut u = [0f32; 64];
            for (n, u) in u.iter_mut().enumerate() {
                *u = (0..5).map(|j| self.x[n + 64 * j] * QMF_WINDOW[2 * (n + 64 * j)]).sum();
            }
            for (sb, row) in slot.iter_mut().zip(&k.analysis) {
                let (mut re, mut im) = (0f32, 0f32);
                for (u, m) in u.iter().zip(row) {
                    re += u * m.re;
                    im += u * m.im;
                }
                *sb = Cx { re, im };
            }
        }
    }
}

/// The 64-band synthesis bank of one channel.
#[derive(Clone)]
pub(crate) struct Synthesis {
    v: Box<[f32; 1280]>,
    /// The last two output samples, which the next slot puts out first.
    held: [f32; 2],
}

impl Synthesis {
    pub(crate) fn new() -> Synthesis {
        Synthesis { v: Box::new([0.0; 1280]), held: [0.0; 2] }
    }

    /// Merge one slot of 64 subbands into 64 output samples (`out[..64]`, of which it needs 64).
    /// Bands from `bands` up are taken as zero.
    pub(crate) fn process(&mut self, x: &[Cx; 64], bands: usize, out: &mut [f32]) {
        let k = kernels();
        let bands = bands.min(64);
        self.v.copy_within(0..1152, 128);
        for (n, row) in k.synthesis.iter().enumerate() {
            self.v[n] = x[..bands].iter().zip(row).map(|(x, m)| x.re * m.re - x.im * m.im).sum();
        }
        let mut slot = [0f32; 66];
        slot[..2].copy_from_slice(&self.held);
        for (n, o) in slot[2..].iter_mut().enumerate() {
            let mut s = 0f32;
            for i in 0..5 {
                s += self.v[256 * i + n] * QMF_WINDOW[128 * i + n];
                s += self.v[256 * i + 192 + n] * QMF_WINDOW[128 * i + 64 + n];
            }
            *o = s;
        }
        self.held = [slot[64], slot[65]];
        for (o, s) in out.iter_mut().zip(&slot[..64]) {
            *o = *s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::SLOTS;
    use super::*;

    fn tones(t: f64) -> f64 {
        [0.0371, 0.0913, 0.1527, 0.2211, 0.2903, 0.3519, 0.4127].iter().enumerate().map(|(j, f)| (2.0 * PI * f * t + j as f64).sin() * 0.1).sum()
    }

    /// Tones across the core band through analysis and synthesis (low 32 bands only) come out at
    /// twice the rate, [`DELAY`] samples late, with the error 60 dB down.
    #[test]
    fn analysis_then_synthesis_reconstructs_the_input() {
        let input: Vec<f32> = (0..8 * 1024).map(|i| tones(i as f64) as f32).collect();
        let (mut a, mut s) = (Analysis::new(), Synthesis::new());
        let mut out = Vec::new();
        let mut slots = [[Cx::default(); 32]; SLOTS];
        for frame in input.as_chunks::<1024>().0 {
            a.process(frame, &mut slots);
            for slot in &slots {
                let mut x = [Cx::default(); 64];
                x[..32].copy_from_slice(slot);
                let mut o = [0f32; 64];
                s.process(&x, 32, &mut o);
                out.extend_from_slice(&o);
            }
        }
        assert_eq!(out.len(), 2 * input.len());
        let snr_at = |delay: usize| {
            let (mut err, mut sig) = (0f64, 0f64);
            for (i, &o) in out.iter().enumerate().skip(3000) {
                let want = tones((i as f64 - delay as f64) / 2.0);
                err += (o as f64 - want).powi(2);
                sig += want * want;
            }
            10.0 * (sig / err).log10()
        };
        let snr = snr_at(DELAY);
        assert!(snr > 60.0, "reconstruction SNR {snr:.1} dB");
        assert!(snr_at(DELAY - 1) < 40.0 && snr_at(DELAY + 1) < 40.0);
    }
}
