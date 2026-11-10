//! Repairs for non-finite normals and tangents. Assimp writes NaN normals for vertices that
//! only touch degenerate (zero-area) triangles; 3 of 156 corpus files have some.

use super::scene_mesh::ScenePart as MeshPart;
use super::transform::{Vec3, cross, dot, normalize};

/// Replaces non-finite normals by the area-weighted normal of the vertex's triangles (or +Z
/// when those have no area either), and non-finite tangents by a vector perpendicular to the
/// normal. Returns how many vertices were repaired.
pub fn repair_frames(part: &mut MeshPart) -> usize {
    let bad: Vec<bool> = part.normals.iter().map(|n| !finite(n)).collect();
    let mut repaired = 0;
    if bad.iter().any(|&b| b) {
        let face_normals = accumulated_face_normals(part, &bad);
        // A vertex with a normal but no position has no faces: it gets the zero sum, so +Z.
        let sums = face_normals.into_iter().chain(std::iter::repeat([0.0; 3]));
        let wanted = part.normals.iter_mut().zip(&bad).zip(sums);
        for ((n, _), sum) in wanted.filter(|((_, bad), _)| **bad) {
            *n = if dot(sum, sum) > 0.0 { normalize(sum) } else { [0.0, 0.0, 1.0] };
            repaired += 1;
        }
    }
    for (t, n) in part.tangents.iter_mut().zip(&part.normals) {
        if !t.iter().all(|x| x.is_finite()) {
            let p = perpendicular(*n);
            *t = [p[0], p[1], p[2], 1.0];
            repaired += 1;
        }
    }
    repaired
}

fn finite(v: &Vec3) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// Sum of area-weighted face normals around each vertex flagged in `wanted`.
fn accumulated_face_normals(part: &MeshPart, wanted: &[bool]) -> Vec<Vec3> {
    let mut sums = vec![[0.0f32; 3]; part.positions.len()];
    for tri in &part.triangles {
        if !tri.iter().any(|&i| wanted.get(i as usize).copied().unwrap_or(false)) {
            continue;
        }
        let [a, b, c] = tri.map(|i| part.positions.get(i as usize).copied().unwrap_or_default());
        let n = cross(sub(b, a), sub(c, a));
        if !finite(&n) {
            continue;
        }
        for &i in tri {
            if let Some(s) = sums.get_mut(i as usize) {
                *s = [s[0] + n[0], s[1] + n[1], s[2] + n[2]];
            }
        }
    }
    sums
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// A unit vector perpendicular to `n`.
fn perpendicular(n: Vec3) -> Vec3 {
    let axis = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    normalize(cross(cross(n, axis), n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nan_normals_become_face_normals_and_tangents_follow() {
        let mut part = MeshPart {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[f32::NAN; 3], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
            tangents: vec![[f32::NAN; 4], [1.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0]],
            triangles: vec![[0, 1, 2]],
            ..MeshPart::default()
        };
        assert_eq!(repair_frames(&mut part), 2);
        assert_eq!(part.normals[0], [0.0, 0.0, 1.0]);
        let t = part.tangents[0];
        assert!(dot([t[0], t[1], t[2]], part.normals[0]).abs() < 1e-6 && t[3] == 1.0);
    }
}
