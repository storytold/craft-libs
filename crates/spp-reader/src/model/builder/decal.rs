//! `DataDecalHitRect` / `DataDecalHitPunctual` → [`DecalHit`].

use super::ModelBuilder;
use crate::model::decal::{DecalHit, HitGranularity, HitShape};
use crate::model::raw::RawObject;
use crate::model::stroke::{StoredMat4, StrokeView};

impl ModelBuilder {
    /// The colour of a decal that stores it once for all its hits (a uniform source) instead of on each hit.
    /// A source of another class stays unrecognised, and the hits fall back to the default colour.
    pub(super) fn decal_color(&mut self, e: &RawObject) -> Option<[f32; 3]> {
        let source = e.objects("sources").next()?;
        if source.class != "DataSourceUniform" {
            self.raw(source);
            return None;
        }
        self.seen(source);
        source.vec3("color")
    }

    /// One hit; `None` for a class or shape that is not a known hit (it then stays unrecognised).
    pub(super) fn decal_hit(&mut self, h: &RawObject, decal_color: Option<[f32; 3]>) -> Option<DecalHit> {
        let vec2 = |name: &str| match h.floats(name) {
            Some([x, y, ..]) => Some([*x, *y]),
            _ => None,
        };
        let shape = match h.class.as_str() {
            "DataDecalHitPunctual" => HitShape::Point(vec2("position")?),
            "DataDecalHitRect" => HitShape::Rect(vec2("position0")?, vec2("position1")?),
            _ => return None,
        };
        self.seen(h);
        let mat4 = |name: &str| -> StoredMat4 {
            match h.floats(name) {
                Some(m) if m.len() == 16 => std::array::from_fn(|i| m.get(i).copied().unwrap_or(0.0)),
                _ => [0.0; 16],
            }
        };
        Some(DecalHit {
            uid: h.uid(),
            shape,
            color: h.vec3("color").or(decal_color).unwrap_or([1.0; 3]),
            granularity: HitGranularity::from_value(h.u64("granularity").unwrap_or(0)),
            view: StrokeView::from_value(h.u64("viewType").unwrap_or(1)),
            proj_view_inv: match h.floats("projViewInvMatrix") {
                Some(_) => mat4("projViewInvMatrix"),
                None => invert(&multiply(&mat4("projectionMatrix"), &mat4("viewMatrix"))),
            },
            duplicate_count: h.u64("duplicateCount").unwrap_or(0),
            duplicate_matrix: mat4("duplicateMatrix"),
            paint_through: h.u64("paintThrough").unwrap_or(0) != 0,
            collision: h.u64("collisionMode"),
            occlusion_culling: h.bool("occlusionCulling"),
        })
    }
}

/// `a b` for column-major matrices.
fn multiply(a: &StoredMat4, b: &StoredMat4) -> StoredMat4 {
    let at = |m: &StoredMat4, column: usize, row: usize| m.get(column * 4 + row).copied().unwrap_or(0.0);
    std::array::from_fn(|i| (0..4).map(|k| at(a, k, i % 4) * at(b, i / 4, k)).sum())
}

/// The inverse by cofactors; all zeros for a singular matrix, which is what the 2D view stores.
fn invert(m: &StoredMat4) -> StoredMat4 {
    let e = |i: usize| m.get(i).copied().unwrap_or(0.0);
    let s = [
        e(0) * e(5) - e(1) * e(4),
        e(0) * e(6) - e(2) * e(4),
        e(0) * e(7) - e(3) * e(4),
        e(1) * e(6) - e(2) * e(5),
        e(1) * e(7) - e(3) * e(5),
        e(2) * e(7) - e(3) * e(6),
    ];
    let c = [
        e(8) * e(13) - e(9) * e(12),
        e(8) * e(14) - e(10) * e(12),
        e(8) * e(15) - e(11) * e(12),
        e(9) * e(14) - e(10) * e(13),
        e(9) * e(15) - e(11) * e(13),
        e(10) * e(15) - e(11) * e(14),
    ];
    let [s0, s1, s2, s3, s4, s5] = s;
    let [c0, c1, c2, c3, c4, c5] = c;
    let det = s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0;
    if det == 0.0 || !det.is_finite() {
        return [0.0; 16];
    }
    let out = [
        e(5) * c5 - e(6) * c4 + e(7) * c3,
        -e(1) * c5 + e(2) * c4 - e(3) * c3,
        e(13) * s5 - e(14) * s4 + e(15) * s3,
        -e(9) * s5 + e(10) * s4 - e(11) * s3,
        -e(4) * c5 + e(6) * c2 - e(7) * c1,
        e(0) * c5 - e(2) * c2 + e(3) * c1,
        -e(12) * s5 + e(14) * s2 - e(15) * s1,
        e(8) * s5 - e(10) * s2 + e(11) * s1,
        e(4) * c4 - e(5) * c2 + e(7) * c0,
        -e(0) * c4 + e(1) * c2 - e(3) * c0,
        e(12) * s4 - e(13) * s2 + e(15) * s0,
        -e(8) * s4 + e(9) * s2 - e(11) * s0,
        -e(4) * c3 + e(5) * c1 - e(6) * c0,
        e(0) * c3 - e(1) * c1 + e(2) * c0,
        -e(12) * s3 + e(13) * s1 - e(14) * s0,
        e(8) * s3 - e(9) * s1 + e(10) * s0,
    ];
    out.map(|v| v / det)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_of_a_view_projection() {
        let projection = [3.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, -1.0, -1.0, 0.0, 0.0, -0.02, 0.0];
        let view = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.5, 0.25, -2.0, 1.0];
        let m = multiply(&projection, &view);
        let identity = multiply(&m, &invert(&m));
        for (i, v) in identity.iter().enumerate() {
            assert!((v - if i % 5 == 0 { 1.0 } else { 0.0 }).abs() < 1e-3, "{i}: {v}");
        }
        assert_eq!(invert(&[0.0; 16]), [0.0; 16]);
    }
}
