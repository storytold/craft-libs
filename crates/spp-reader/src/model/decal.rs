//! [`DecalHit`]: one pick of the reference app's polygon fill tool (`DataDecalHitRect`, `DataDecalHitPunctual` in
//! `DataActionDecal.hits`): a click or a dragged rectangle, in the 2D (UV) view or in the 3D view with the
//! camera it was made with, the value it writes and how far the pick grows. Field meanings are in the comments.

use super::stroke::{StoredMat4, StrokeView};

/// Where a hit was made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitShape {
    /// `DataDecalHitPunctual.position`: one click.
    Point([f32; 2]),
    /// `DataDecalHitRect.position0` / `position1`: two opposite corners of a dragged rectangle.
    Rect([f32; 2], [f32; 2]),
}

/// `granularity`: what one picked triangle grows to, the tool's fill modes: 0 polygon, 1 triangle, 2 mesh, 3 UV chunk
/// (measured: each mode saved by version 12 and its picks compared).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitGranularity {
    Triangle,
    Polygon,
    Mesh,
    UvIsland,
    Other(u64),
}

impl HitGranularity {
    pub fn from_value(v: u64) -> HitGranularity {
        match v {
            0 => HitGranularity::Polygon,
            1 => HitGranularity::Triangle,
            2 => HitGranularity::Mesh,
            3 => HitGranularity::UvIsland,
            other => HitGranularity::Other(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecalHit {
    pub uid: Option<u64>,
    pub shape: HitShape,
    /// `color`: the value written into the picked triangles (a grey in masks: white adds, black removes).
    pub color: [f32; 3],
    pub granularity: HitGranularity,
    /// `viewType`.
    pub view: StrokeView,
    /// `projViewInvMatrix`: the inverse of the 3D view's projection × view matrix (zero in the 2D view).
    pub proj_view_inv: StoredMat4,
    /// `duplicateCount` / `duplicateMatrix`: the symmetry copy (count 0: none).
    pub duplicate_count: u64,
    pub duplicate_matrix: StoredMat4,
    /// `paintThrough` (punctual hits of newer files): the pick also reaches hidden faces.
    pub paint_through: bool,
    /// `collisionMode` (rectangles of newer files): 0 a polygon's centre must lie inside ("Contained center"),
    /// 1 any part of it ("Intersection", the tool's default). Absent in older files.
    pub collision: Option<u64>,
    /// `occlusionCulling` (rectangles of newer files, "Visible polygons only"): only what the camera sees inside
    /// the rectangle is picked. Absent in older files.
    pub occlusion_culling: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::HitGranularity;

    #[test]
    fn granularity_follows_the_fill_modes() {
        let modes: Vec<HitGranularity> = (0..5).map(HitGranularity::from_value).collect();
        use HitGranularity::*;
        assert_eq!(modes, [Polygon, Triangle, Mesh, UvIsland, Other(4)]);
    }
}
