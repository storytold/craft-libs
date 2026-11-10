//! [`FillPlacement`]: the projection settings of a `DataActionFill` besides its mode, filter and first UV
//! transform (see [`super::FillEffect`]): repetition, the 3D volume, falloffs, the warp grid and the size-mode
//! fields of the UV transform. The field comments give each meaning and how sure it is.

/// Every field is `None` when the file does not store it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FillPlacement {
    /// `uvSamplingWrap` enum: 3 ×7656, 0 ×33, 1 ×4 (Inferred: 0 none, 1 u, 2 v, 3 both).
    pub uv_wrap: Option<u64>,
    /// `dddWrap` enum: 1 continues beyond the volume (default), 0 clipped (Inferred).
    pub volume_wrap: Option<u64>,
    /// `sideHardness`: tri-planar blend hardness (0.5 default).
    pub side_hardness: Option<f32>,
    pub depth_culling: Option<bool>,
    pub depth_culling_hardness: Option<f32>,
    pub backface_culling: Option<bool>,
    /// Fraction of 180° (Inferred, weak).
    pub backface_culling_angle: Option<f32>,
    pub backface_culling_hardness: Option<f32>,
    /// Fraction of the circumference (1 = all).
    pub cylinder_culling_angle: Option<f32>,
    pub cylinder_hide_caps: Option<bool>,
    /// `dddPosition`, `dddEulerAngles` (radians), `dddSize`.
    pub position: Option<[f32; 3]>,
    pub euler_angles: Option<[f32; 3]>,
    pub size: Option<[f32; 3]>,
    /// `dddGrid` (`DataGridDeformation`).
    pub grid: Option<GridDeformation>,
    /// The size-mode fields of `uvTransformation0`, kept as read; their meaning is not established yet.
    pub uv_sizing: Option<UvSizing>,
    /// `uvTransformation1` is present and not the identity (never seen in the corpus).
    pub second_uv_transform: bool,
    /// `uvGrid` is present (never seen in the corpus).
    pub uv_grid: bool,
}

/// `DataGridDeformation`: a warp grid.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GridDeformation {
    /// `gridSize` (3 × 3 by default).
    pub size: [i32; 2],
    pub depth: f32,
    pub vertices: Vec<GridVertex>,
}

/// `DataGridVertex`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GridVertex {
    /// `position3D`, in the volume's local frame (−1…1).
    pub position: [f32; 3],
    /// `positionUV`.
    pub uv: [f32; 2],
    /// `tangentColumnPrev`, `tangentColumnNext` (along u), `tangentRowPrev`, `tangentRowNext` (along v).
    pub tangents: [[f32; 3]; 4],
}

/// The size-mode fields of a `Data2DTransformation` (newer files) and its UI state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UvSizing {
    pub scale_mode: Option<u64>,
    /// `scalePhysicalSize`, centimetres (Inferred).
    pub physical_size: Option<[f32; 2]>,
    pub scale_factors: Option<[f32; 2]>,
    pub physical_bounded: Option<bool>,
    pub pivot: Option<[f32; 2]>,
    pub ratio_locked: Option<bool>,
}
