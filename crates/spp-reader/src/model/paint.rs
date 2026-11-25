//! [`PaintEffect`]: brush work (`DataActionPaint` and its projection, eraser and smudge variants) with its
//! strokes ([`ProjectStroke`]) and its 3D path strokes ([`StrokePath3d`]).

use super::brush::ProjectBrush;
use super::effect::EffectHeader;
use super::path::StrokePath3d;
use super::source::ProjectSource;
use super::stroke::ProjectStroke;

/// Which tool made the strokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintTool {
    /// `DataActionPaint`
    Brush,
    /// `DataActionPaintProj`
    Projection,
    /// `DataActionEraser`
    Eraser,
    /// `DataActionSmudge`
    Smudge,
}

impl PaintTool {
    pub fn from_class(class: &str) -> Option<PaintTool> {
        Some(match class {
            "DataActionPaint" => PaintTool::Brush,
            "DataActionPaintProj" => PaintTool::Projection,
            "DataActionEraser" => PaintTool::Eraser,
            "DataActionSmudge" => PaintTool::Smudge,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            PaintTool::Brush => "brush",
            PaintTool::Projection => "projection",
            PaintTool::Eraser => "eraser",
            PaintTool::Smudge => "smudge",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaintEffect {
    pub header: EffectHeader,
    pub tool: PaintTool,
    /// Tool opacity (`opacity`); 1 if missing.
    pub opacity: f32,
    /// `strokes`: the `DataStroke`s, in painting order.
    pub strokes: Vec<ProjectStroke>,
    /// `strokes3D` (`DataStroke3D`): path strokes on the mesh, in painting order (after `strokes`, Inferred).
    pub paths: Vec<StrokePath3d>,
    /// `brush`: uid of the `DataBrush` (size, flow, spacing, jitter, emitter) the strokes were painted with.
    pub brush_uid: Option<u64>,
    /// `brush`: its settings, if the effect has one.
    pub brush: Option<ProjectBrush>,
    /// `stampBlendingMode` (0 ×12126, 1 ×688; absent in old files): how stamps of one stroke combine.
    pub stamp_blending: u64,
    /// `sourceColor`: what the brush paints, per channel (uniform colours are linear).
    pub colors: Vec<ProjectSource>,
    /// `sourceTransparent`: the brush alpha (usually the brush shape procedural or a bitmap).
    pub alpha: Vec<ProjectSource>,
    /// `sourceStencil`, if a stencil was used.
    pub stencil: Option<ProjectSource>,
    /// `projMatrix`: 3×3, column by column, from the view's normalised coordinates to the stencil's.
    pub stencil_matrix: Option<[f32; 9]>,
}

impl PaintEffect {
    /// Number of path strokes.
    pub fn path_strokes(&self) -> usize {
        self.paths.len()
    }

    /// Strokes of both kinds.
    pub fn stroke_count(&self) -> usize {
        self.strokes.len() + self.paths.len()
    }
}
