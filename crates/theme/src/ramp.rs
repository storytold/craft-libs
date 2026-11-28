//! 12-step colour ramps in the Radix step convention.

use super::color::{Oklch, Rgba};

/// Light or dark appearance; selects the lightness curve of a ramp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Light backgrounds, dark text.
    Light,
    /// Dark backgrounds, light text.
    Dark,
}

/// How a ramp takes chroma from its seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RampKind {
    /// A tinted grey: the seed's chroma is capped at [`NEUTRAL_MAX_CHROMA`].
    Neutral,
    /// A hue family: the seed's full chroma is available at step 9.
    Chromatic,
}

/// Highest seed chroma a [`RampKind::Neutral`] ramp keeps.
pub const NEUTRAL_MAX_CHROMA: f64 = 0.03;

/// OKLCH lightness per step (1 to 12) for light ramps: falls from near white to near black.
/// Entry 9 is the nominal solid; chromatic ramps replace it with the seed's own lightness.
pub const LIGHT_L: [f64; 12] = [0.992, 0.975, 0.950, 0.925, 0.895, 0.860, 0.810, 0.740, 0.620, 0.570, 0.480, 0.270];

/// OKLCH lightness per step (1 to 12) for dark ramps: rises from near black to near white.
pub const DARK_L: [f64; 12] = [0.160, 0.190, 0.240, 0.280, 0.320, 0.370, 0.440, 0.540, 0.620, 0.680, 0.800, 0.940];

/// Fraction of the seed's chroma kept at each step: low at the background and text ends, full at 9.
pub const CHROMA_FACTOR: [f64; 12] = [0.06, 0.15, 0.30, 0.42, 0.54, 0.66, 0.80, 0.92, 1.0, 0.98, 0.85, 0.45];

/// Smallest lightness gap kept between neighbouring steps when a chromatic seed's lightness
/// forces the curve to bend around step 9.
const MIN_STEP_GAP: f64 = 0.01;

/// Twelve colours from background to text. Steps 1 to 2 are app and subtle backgrounds, 3 to 5
/// component fills (rest, hover, pressed), 6 to 8 borders (subtle, normal, strong), 9 to 10 solid
/// fills (rest, hover), 11 to 12 text (muted, normal). Index 0 is step 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ramp(pub [Rgba; 12]);

impl Ramp {
    /// Step `n` of 1 to 12. Out-of-range steps clamp to the ends.
    pub fn step(&self, n: usize) -> Rgba {
        self.0[n.clamp(1, 12) - 1]
    }

    /// Builds the ramp for `mode` from a seed. Hue comes from the seed, chroma from the seed
    /// scaled by [`CHROMA_FACTOR`]. A neutral ramp takes every lightness, step 9 included, from
    /// the curve of the mode and caps the seed's chroma at [`NEUTRAL_MAX_CHROMA`]. A chromatic
    /// ramp has the seed itself at step 9 and bends the other steps by at least 0.01 lightness
    /// around it so lightness stays monotonic for any seed not near black or white. Every step
    /// is gamut-mapped.
    pub fn from_seed(seed: Rgba, mode: Mode, kind: RampKind) -> Ramp {
        let s = seed.to_oklch();
        let chroma = match kind {
            RampKind::Neutral => s.c.min(NEUTRAL_MAX_CHROMA),
            RampKind::Chromatic => s.c,
        };
        let curve = curve(mode);
        let mut out = [Rgba::BLACK; 12];
        for (i, slot) in out.iter_mut().enumerate() {
            let step = i + 1;
            if kind == RampKind::Neutral {
                *slot = neutral(s, i, curve[i]);
                continue;
            }
            if step == 9 {
                *slot = seed.with_alpha(255);
                continue;
            }
            let c = chroma * CHROMA_FACTOR[i];
            let gap = MIN_STEP_GAP * step.abs_diff(9) as f64;
            // Light ramps fall with the step number and dark ones rise, so `above` marks the
            // steps that must be lighter than step 9.
            let above = (step < 9) == (mode == Mode::Light);
            let l = if above { curve[i].max(s.l + gap) } else { curve[i].min(s.l - gap) };
            *slot = Oklch { l: l.clamp(0.0, 1.0), c, h: s.h }.to_rgba();
        }
        Ramp(out)
    }
}

/// The lightness curve of `mode`, step 1 first.
pub(crate) const fn curve(mode: Mode) -> &'static [f64; 12] {
    match mode {
        Mode::Light => &LIGHT_L,
        Mode::Dark => &DARK_L,
    }
}

/// Step `step` (1 to 12) of the neutral ramp from `seed` with lightness `l`, clamped to 0..=1;
/// hue and chroma as [`Ramp::from_seed`] gives the step. With the curve's lightness it equals
/// the ramp's step.
pub(crate) fn neutral_at(seed: Rgba, step: usize, l: f64) -> Rgba {
    neutral(seed.to_oklch(), step.clamp(1, 12) - 1, l.clamp(0.0, 1.0))
}

/// The neutral colour at index `i` with lightness `l`, from the seed's OKLCH.
fn neutral(seed: Oklch, i: usize, l: f64) -> Rgba {
    Oklch { l, c: seed.c.min(NEUTRAL_MAX_CHROMA) * CHROMA_FACTOR[i], h: seed.h }.to_rgba()
}
