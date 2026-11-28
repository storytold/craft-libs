//! [`GrungeVariant`]: the four looks of the `grunge` node (concrete, scratches, spots, smudges) as a point
//! function of (u, v), tileable with period 1. Our own looks built from our noise primitives.

use crate::noise::{self, smoothstep as smooth};

/// Which look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Concrete,
    Scratches,
    Spots,
    Smudges,
}

impl Variant {
    /// 0 concrete, 1 scratches, 2 spots, anything else smudges.
    pub fn from_index(i: u32) -> Variant {
        match i {
            0 => Variant::Concrete,
            1 => Variant::Scratches,
            2 => Variant::Spots,
            _ => Variant::Smudges,
        }
    }
}

/// The settings of one grunge look. Ranges as the node clamps them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrungeVariant {
    pub variant: Variant,
    pub seed: u32,
    /// 0..1, 0.5 neutral: added to the value.
    pub balance: f32,
    // concrete
    pub large_variation: f32,
    pub base_noisiness: f32,
    pub dirt_specks: f32,
    pub invert_dirt: bool,
    pub scratches: f32,
    pub sharpen: f32,
    // scratches
    /// Scratch columns (1..=32).
    pub quantity: i32,
    pub scratch_width: f32,
    pub scratch_length: f32,
    // spots
    /// Spot cells per side (1..=128).
    pub details: i32,
    pub coverage: f32,
    pub coverage_contrast: f32,
}

impl GrungeVariant {
    /// The node's defaults for `variant`.
    pub fn new(variant: Variant, seed: u32) -> GrungeVariant {
        GrungeVariant {
            variant,
            seed,
            balance: 0.5,
            large_variation: 0.55,
            base_noisiness: 0.45,
            dirt_specks: 0.0,
            invert_dirt: false,
            scratches: 0.0,
            sharpen: 0.0,
            quantity: 3,
            scratch_width: 0.4,
            scratch_length: 0.45,
            details: 22,
            coverage: 0.35,
            coverage_contrast: 0.25,
        }
    }

    /// The value at (a, b) (already warped by the caller, if at all), 0..1.
    pub fn value(&self, a: f32, b: f32) -> f32 {
        let seed = self.seed;
        let val = match self.variant {
            Variant::Concrete => {
                let n = noise::fbm(a, b, 8, 8, 6, 0.55, seed, 0) * self.large_variation
                    + noise::fbm(a, b, 40, 40, 3, 0.5, seed.wrapping_add(1), 0) * self.base_noisiness;
                let mut c = ((n - 0.35) * 2.2).clamp(0.0, 1.0);
                if self.dirt_specks > 0.0 {
                    let sp = noise::spots(a, b, 64, 64, seed.wrapping_add(11), |id| (0.1, 0.18 + id * 0.2), 0.0);
                    c = if self.invert_dirt { c + (1.0 - c) * sp * self.dirt_specks } else { c * (1.0 - sp * self.dirt_specks) };
                }
                if self.scratches > 0.0 {
                    let line = noise::perlin(a * 3.0 % 1.0, b, 3, 160, seed.wrapping_add(13));
                    c = (c + smooth(0.62, 0.7, line) * self.scratches).clamp(0.0, 1.0);
                }
                if self.sharpen > 0.0 {
                    c += (smooth(0.0, 1.0, c) - c) * self.sharpen;
                }
                c
            }
            Variant::Scratches => {
                let (quant, width, length) = (self.quantity, self.scratch_width, self.scratch_length);
                let q = quant as f32;
                let line = noise::perlin(a * q % 1.0, b, quant, 90, seed);
                let m = noise::fbm(a, b, 5, 5, 4, 0.5, seed.wrapping_add(3), 0);
                smooth(1.0 - width, 1.1 - width, line) * smooth(length, length + 0.15, m)
            }
            Variant::Spots => {
                let (details, cover, cover_c) = (self.details, self.coverage, self.coverage_contrast);
                let m = noise::fbm(a, b, 6, 6, 4, 0.5, seed.wrapping_add(4), 0);
                noise::spots(a, b, details, details, seed, |id| (0.12, 0.2 + id * 0.3), 0.0) * smooth(cover, cover + cover_c, m)
            }
            Variant::Smudges => {
                let n = noise::fbm(a, b, 3, 3, 5, 0.6, seed, 2);
                smooth(0.2, 0.8, n)
            }
        };
        (val + (self.balance - 0.5)).clamp(0.0, 1.0)
    }
}
