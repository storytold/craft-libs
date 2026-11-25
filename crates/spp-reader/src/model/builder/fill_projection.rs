//! The projection fields of a `DataActionFill` ([`FillPlacement`]).

use super::ModelBuilder;
use crate::model::fill_projection::{FillPlacement, GridDeformation, GridVertex, UvSizing};
use crate::model::raw::RawObject;

fn vec2(o: &RawObject, name: &str) -> Option<[f32; 2]> {
    match o.floats(name)? {
        [x, y, ..] => Some([*x, *y]),
        _ => None,
    }
}

impl ModelBuilder {
    pub(super) fn fill_placement(&mut self, e: &RawObject) -> FillPlacement {
        let uv = e.object("uvTransformation0").or_else(|| e.object("uvTransformation"));
        let second = e.object("uvTransformation1");
        if let Some(t) = second {
            self.seen(t);
        }
        FillPlacement {
            uv_wrap: e.u64("uvSamplingWrap"),
            volume_wrap: e.u64("dddWrap"),
            side_hardness: e.f32("sideHardness"),
            depth_culling: e.bool("depthCulling"),
            depth_culling_hardness: e.f32("depthCullingHardness"),
            backface_culling: e.bool("backfaceCulling"),
            backface_culling_angle: e.f32("backfaceCullingAngle"),
            backface_culling_hardness: e.f32("backfaceCullingHardness"),
            cylinder_culling_angle: e.f32("cylinderCullingAngle"),
            cylinder_hide_caps: e.bool("cylinderHideCaps"),
            position: e.vec3("dddPosition"),
            euler_angles: e.vec3("dddEulerAngles"),
            size: e.vec3("dddSize"),
            grid: e.object("dddGrid").map(|g| self.grid(g)),
            uv_sizing: uv.map(uv_sizing),
            second_uv_transform: second.is_some_and(|t| !is_identity(t)),
            uv_grid: e.object("uvGrid").is_some(),
        }
    }

    fn grid(&mut self, g: &RawObject) -> GridDeformation {
        self.seen(g);
        let mut vertices = Vec::new();
        for v in g.objects("vertices") {
            self.seen(v);
            let t = |name: &str| v.vec3(name).unwrap_or([0.0; 3]);
            vertices.push(GridVertex {
                position: v.vec3("position3D").unwrap_or([0.0; 3]),
                uv: vec2(v, "positionUV").unwrap_or([0.0; 2]),
                tangents: [t("tangentColumnPrev"), t("tangentColumnNext"), t("tangentRowPrev"), t("tangentRowNext")],
            });
        }
        GridDeformation { size: g.ivec2("gridSize").unwrap_or([3, 3]), depth: g.f32("depth").unwrap_or(0.1), vertices }
    }
}

fn uv_sizing(t: &RawObject) -> UvSizing {
    UvSizing {
        scale_mode: t.u64("scaleMode"),
        physical_size: vec2(t, "scalePhysicalSize"),
        scale_factors: vec2(t, "scaleFactors"),
        physical_bounded: t.bool("physicalSizeBoundedByScaleFactors"),
        pivot: vec2(t, "GUIPivot"),
        ratio_locked: t.bool("GUIScaleRatioLocked"),
    }
}

fn is_identity(t: &RawObject) -> bool {
    vec2(t, "scale").is_none_or(|s| s == [1.0, 1.0])
        && vec2(t, "scaleFactors").is_none_or(|s| s == [1.0, 1.0])
        && vec2(t, "translation").is_none_or(|s| s == [0.0, 0.0])
        && t.f32("rotation").is_none_or(|r| r == 0.0)
}
