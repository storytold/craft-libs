//! [`ProjectBrush`]: the brush a paint effect's strokes were painted with (`DataBrush`) and how it lays its stamps
//! along a stroke (`stampsEmitter`: `DataEmitterStroke`, `DataEmitterPhysics` for particle brushes,
//! `DataEmitterStroke3D` for path strokes). The meanings in the comments are Inferred unless the import's replay
//! calibration says otherwise (`translate::layers::replay`).

use super::procedural::Tweak;

/// `DataBrush`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectBrush {
    pub uid: Option<u64>,
    /// `brushSize`: stamp size (0.001 … 1.5; 0.1 most common).
    pub size: f32,
    /// `brushSizeSpace` (1 in every corpus brush).
    pub size_space: u64,
    /// `brushSizeJitter` 0 … 1.
    pub size_jitter: f32,
    /// `flow` 0 … 1.
    pub flow: f32,
    /// `flowJitter` 0 … 1.
    pub flow_jitter: f32,
    /// `alignment` (3 ×12254, 1 ×844, 2 ×496): how a stamp lies on the surface.
    pub alignment: u64,
    /// `cullingEnabled` and `cullingAngle` (0.5 everywhere): surfaces turned away are not painted.
    pub culling: Option<f32>,
    /// `paintThrough` (0, 1; absent in old files).
    pub paint_through: u64,
    /// `filtering` (0, 1).
    pub filtering: u64,
    pub emitter: StampEmitter,
}

impl Default for ProjectBrush {
    fn default() -> ProjectBrush {
        ProjectBrush {
            uid: None,
            size: 0.1,
            size_space: 1,
            size_jitter: 0.0,
            flow: 1.0,
            flow_jitter: 0.0,
            alignment: 3,
            culling: Some(0.5),
            paint_through: 0,
            filtering: 0,
            emitter: StampEmitter::Stroke(StrokeEmitter::default()),
        }
    }
}

/// `stampsEmitter`.
#[derive(Debug, Clone, PartialEq)]
pub enum StampEmitter {
    /// `DataEmitterStroke`: stamps along the pen's path.
    Stroke(StrokeEmitter),
    /// `DataEmitterPhysics`: a particle brush; stamps come from a simulation.
    Physics(PhysicsEmitter),
    /// `DataEmitterStroke3D`: stamps along a 3D path stroke.
    Stroke3D(StrokeEmitter),
    /// The brush is a `DataBrushRibbon`: no stamps, the source is laid along a path as a band.
    Ribbon(RibbonBrush),
    /// No emitter, or a class the model does not know.
    Other(String),
}

/// `DataBrushRibbon`: a path tool brush that lays its source along the path as a band of `width`, instead of stamps.
/// Meanings measured on flat test paths unless marked Inferred.
#[derive(Debug, Clone, PartialEq)]
pub struct RibbonBrush {
    /// `width`: the band's full width in mesh units (as `brushSize`).
    pub width: f32,
    /// `tilingMode`: 0 the source repeats along the path, a whole number of times stretched to fit, each copy
    /// `width × aspectRatioFactor` long before fitting; 2 one copy over the whole path, its ends kept at that length
    /// and the part between `stretchGuides` stretched (caps); 1 (with `count` 1: panels) one copy stretched over the
    /// whole path (Inferred).
    pub tiling_mode: u64,
    /// `stretchMode` (0, 1, 2, 4) and `stretchGuides` (0.25/0.75, 0.5/0.5): where the source is cut for stretching.
    pub stretch_mode: u64,
    pub stretch_guides: [f32; 2],
    /// `aspectRatioFactor` (1, 2) and `count` (4, 1).
    pub aspect_ratio: f32,
    pub count: u64,
    /// `sourceDirection` (0, 2), `alignment` (0, 1, 2: text presets), `cornerShape` (0, 1, 3), `flipAlongWidth`,
    /// `repeatMirror`, `omitEndsWhenClosed`, `alphaBlendingMode` (2, 7), `depthHardness`, `projectionDepthRatio`:
    /// kept, not used yet.
    pub source_direction: u64,
    pub alignment: u64,
    pub corner_shape: u64,
    pub flip_along_width: bool,
    pub repeat_mirror: bool,
    pub omit_ends_when_closed: bool,
    pub alpha_blending: u64,
    pub depth_hardness: f32,
    pub projection_depth_ratio: f32,
}

/// `DataEmitterPhysics`. Meanings inferred from the names and the reference app's tool panel; none is used yet.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhysicsEmitter {
    /// `urlEmitter`, `urlReceiver`: the resources that run the particles (names only here).
    pub emitter: String,
    pub receiver: String,
    /// `deltaTime`: the simulation's step in seconds (the panel's `dt`).
    pub delta_time: Option<f32>,
    /// `time`: seconds at the end of the last stroke.
    pub time: Option<f32>,
    /// `source` (1 where seen).
    pub source: Option<u64>,
    /// `tweaksEmitter`, `tweaksReceiver`: parameters by the scripts' own names, a different set per script.
    pub emitter_tweaks: Vec<Tweak>,
    pub receiver_tweaks: Vec<Tweak>,
}

/// One pressure response (`pressureSizeEnabled` / `pressureSizeMinimum`, the same for flow).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressureResponse {
    pub enabled: bool,
    /// The value at zero pressure, as a fraction of the full value.
    pub minimum: f32,
}

/// `DataEmitterStroke` (and the shared fields of `DataEmitterStroke3D`).
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeEmitter {
    /// `spacing`: distance between stamps, a fraction of the stamp size (0.2 most common).
    pub spacing: f32,
    /// `positionJitter` (0 … 1.1), `positionJitterBothAxes`, `positionJitterDistribution` (0, 1).
    pub position_jitter: f32,
    pub position_jitter_both_axes: bool,
    pub position_jitter_distribution: u64,
    /// `rotation` in radians and `rotationJitter` in radians (0 … π).
    pub rotation: f32,
    pub rotation_jitter: f32,
    /// `followPath`: stamps turn along the stroke.
    pub follow_path: bool,
    pub pressure_size: PressureResponse,
    pub pressure_flow: PressureResponse,
    /// `aspectRatio` (1 in the corpus; old files only).
    pub aspect_ratio: f32,
    /// `DataEmitterStroke3D` only: `upVector` (0, 1, 0 by default; Inferred: where an unturned tip's top points on a
    /// path) and `tolerance` (0.05; meaning unknown, perhaps the curve's sampling).
    pub up_vector: Option<[f32; 3]>,
    pub tolerance: Option<f32>,
}

impl Default for StrokeEmitter {
    fn default() -> StrokeEmitter {
        StrokeEmitter {
            spacing: 0.2,
            position_jitter: 0.0,
            position_jitter_both_axes: true,
            position_jitter_distribution: 1,
            rotation: 0.0,
            rotation_jitter: 0.0,
            follow_path: false,
            pressure_size: PressureResponse { enabled: true, minimum: 0.0 },
            pressure_flow: PressureResponse { enabled: false, minimum: 0.0 },
            aspect_ratio: 1.0,
            up_vector: None,
            tolerance: None,
        }
    }
}
