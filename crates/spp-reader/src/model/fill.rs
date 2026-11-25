//! [`FillEffect`]: `DataActionFill`, the content of fill layers and the generators/fills in masks.

use super::channel::ProjectChannel;
use super::effect::EffectHeader;
use super::fill_projection::FillPlacement;
use super::source::ProjectSource;

#[derive(Debug, Clone, PartialEq)]
pub struct FillEffect {
    pub header: EffectHeader,
    /// `sources`: one uniform/bitmap source per channel, or one procedural source for a set of
    /// channels, or several of these mixed.
    pub sources: Vec<ProjectSource>,
    /// `projection` enum: 0 ×4644 (UV, Inferred), 6 ×1645, 2 ×1298, 1, 3, 7, 4 rarely.
    pub projection: u64,
    /// `filtering` enum (2 ×6422, 0 ×1271).
    pub filtering: u64,
    /// `uvTransformation0` (`uvTransformation` in a few newer files): placement of a UV-projected fill.
    pub uv_transform: Option<UvTransform>,
    /// The other projection settings (repetition, volume, falloffs, warp grid, size modes).
    pub placement: FillPlacement,
}

/// `Data2DTransformation`: scale (tiling), translation (offset) and rotation (radians) in UV space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvTransform {
    pub scale: [f32; 2],
    pub translation: [f32; 2],
    pub rotation: f32,
}

impl UvTransform {
    /// No tiling, offset or rotation: UV maps 1:1 onto the texture.
    pub fn is_identity(&self) -> bool {
        self.scale == [1.0, 1.0] && self.translation == [0.0, 0.0] && self.rotation == 0.0
    }
}

impl FillEffect {
    /// The uniform values per channel, as stored (linear colours).
    pub fn uniform_values(&self) -> impl Iterator<Item = (ProjectChannel, [f32; 3])> + '_ {
        self.sources.iter().filter_map(|s| match s {
            ProjectSource::Uniform(u) => u.channels.channels().next().map(|c| (c, u.color)),
            _ => None,
        })
    }

    /// True if every source is a uniform value (the fill is computable without any resource).
    pub fn is_uniform(&self) -> bool {
        !self.sources.is_empty() && self.sources.iter().all(|s| matches!(s, ProjectSource::Uniform(_)))
    }
}
