//! The library values the env carries: scale, motion policy and arena id.

use craft_behavior::MotionMode;
use craft_theme::MotionTokens;
use egui::Id;
use std::hash::Hash;

/// The interface scale: pixels per point.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Scale(pub f32);

impl Scale {
    /// `pixels_per_point` pixels per point.
    pub const fn new(pixels_per_point: f32) -> Self {
        Scale(pixels_per_point)
    }
}

impl Default for Scale {
    fn default() -> Self {
        Scale(1.0)
    }
}

/// The motion mode in effect, carried in `Env.motion`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MotionPolicy {
    /// The level animations run at.
    pub mode: MotionMode,
    /// The durations and curves of the theme's motions; reduced motion keeps them.
    pub tokens: MotionTokens,
}

impl MotionPolicy {
    /// The policy that runs animations at `mode` with `tokens`.
    pub const fn new(mode: MotionMode, tokens: MotionTokens) -> Self {
        Self { mode, tokens }
    }
}

/// The id of an arena, carried in `Env.arena`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ArenaId(pub Id);

impl ArenaId {
    /// The arena whose id is `id`.
    pub const fn new(id: Id) -> Self {
        ArenaId(id)
    }
}
