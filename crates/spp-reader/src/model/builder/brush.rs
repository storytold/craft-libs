//! `DataBrush` and its `stampsEmitter` → [`ProjectBrush`].

use super::ModelBuilder;
use crate::model::brush::{PhysicsEmitter, PressureResponse, ProjectBrush, RibbonBrush, StampEmitter, StrokeEmitter};
use crate::model::raw::RawObject;

impl ModelBuilder {
    pub(super) fn brush(&mut self, b: &RawObject) -> ProjectBrush {
        self.seen(b);
        let d = ProjectBrush::default();
        let culling = match b.bool("cullingEnabled").unwrap_or(true) {
            true => Some(b.f32("cullingAngle").unwrap_or(0.5)),
            false => None,
        };
        ProjectBrush {
            uid: b.uid(),
            size: b.f32("brushSize").unwrap_or(d.size),
            size_space: b.u64("brushSizeSpace").unwrap_or(d.size_space),
            size_jitter: b.f32("brushSizeJitter").unwrap_or(0.0),
            flow: b.f32("flow").unwrap_or(1.0),
            flow_jitter: b.f32("flowJitter").unwrap_or(0.0),
            alignment: b.u64("alignment").unwrap_or(d.alignment),
            culling,
            paint_through: b.u64("paintThrough").unwrap_or(0),
            filtering: b.u64("filtering").unwrap_or(0),
            emitter: match (b.class.as_str(), b.object("stampsEmitter")) {
                ("DataBrushRibbon", _) => StampEmitter::Ribbon(ribbon(b)),
                (_, Some(e)) => self.emitter(e),
                (_, None) => StampEmitter::Other(String::new()),
            },
        }
    }

    fn emitter(&mut self, e: &RawObject) -> StampEmitter {
        match e.class.as_str() {
            "DataEmitterStroke" => {
                self.seen(e);
                StampEmitter::Stroke(stroke_emitter(e))
            }
            "DataEmitterStroke3D" => {
                self.seen(e);
                StampEmitter::Stroke3D(stroke_emitter(e))
            }
            "DataEmitterPhysics" => {
                self.seen(e);
                StampEmitter::Physics(PhysicsEmitter {
                    emitter: e.string("urlEmitter").unwrap_or_default().to_string(),
                    receiver: e.string("urlReceiver").unwrap_or_default().to_string(),
                    delta_time: e.f32("deltaTime"),
                    time: e.f32("time"),
                    source: e.u64("source"),
                    emitter_tweaks: e.objects("tweaksEmitter").map(|t| self.tweak(t)).collect(),
                    receiver_tweaks: e.objects("tweaksReceiver").map(|t| self.tweak(t)).collect(),
                })
            }
            other => StampEmitter::Other(other.to_string()),
        }
    }
}

fn ribbon(b: &RawObject) -> RibbonBrush {
    let guides = match b.floats("stretchGuides") {
        Some([a, c, ..]) => [*a, *c],
        _ => [0.25, 0.75],
    };
    RibbonBrush {
        width: b.f32("width").unwrap_or(0.1),
        tiling_mode: b.u64("tilingMode").unwrap_or(0),
        stretch_mode: b.u64("stretchMode").unwrap_or(0),
        stretch_guides: guides,
        aspect_ratio: b.f32("aspectRatioFactor").unwrap_or(1.0),
        count: b.u64("count").unwrap_or(4),
        source_direction: b.u64("sourceDirection").unwrap_or(0),
        alignment: b.u64("alignment").unwrap_or(0),
        corner_shape: b.u64("cornerShape").unwrap_or(0),
        flip_along_width: b.bool("flipAlongWidth").unwrap_or(false),
        repeat_mirror: b.bool("repeatMirror").unwrap_or(false),
        omit_ends_when_closed: b.bool("omitEndsWhenClosed").unwrap_or(false),
        alpha_blending: b.u64("alphaBlendingMode").unwrap_or(0),
        depth_hardness: b.f32("depthHardness").unwrap_or(1.0),
        projection_depth_ratio: b.f32("projectionDepthRatio").unwrap_or(1.5),
    }
}

fn stroke_emitter(e: &RawObject) -> StrokeEmitter {
    let d = StrokeEmitter::default();
    // old files (format 51) store `pressure2Size` / `pressure2Flow` (0 or 1) instead of the switches
    let response = |enabled: &str, old: &str, minimum: &str, default: PressureResponse| PressureResponse {
        enabled: e.bool(enabled).or_else(|| e.f32(old).map(|v| v > 0.0)).unwrap_or(default.enabled),
        minimum: e.f32(minimum).unwrap_or(default.minimum),
    };
    StrokeEmitter {
        spacing: e.f32("spacing").unwrap_or(d.spacing),
        position_jitter: e.f32("positionJitter").unwrap_or(0.0),
        position_jitter_both_axes: e.bool("positionJitterBothAxes").unwrap_or(true),
        position_jitter_distribution: e.u64("positionJitterDistribution").unwrap_or(1),
        rotation: e.f32("rotation").unwrap_or(0.0),
        rotation_jitter: e.f32("rotationJitter").unwrap_or(0.0),
        follow_path: e.bool("followPath").unwrap_or(false),
        pressure_size: response("pressureSizeEnabled", "pressure2Size", "pressureSizeMinimum", d.pressure_size),
        pressure_flow: response("pressureFlowEnabled", "pressure2Flow", "pressureFlowMinimum", d.pressure_flow),
        aspect_ratio: e.f32("aspectRatio").unwrap_or(1.0),
        up_vector: e.floats("upVector").and_then(|v| <[f32; 3]>::try_from(v.get(..3)?).ok()),
        tolerance: e.f32("tolerance"),
    }
}
