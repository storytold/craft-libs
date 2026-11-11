//! Tests. Every sample is generated here; the crate ships no asset files.

use super::*;

#[cfg(feature = "fbx")]
mod fbx;
#[cfg(feature = "gltf")]
mod gltf;
#[cfg(feature = "obj")]
mod obj;
#[cfg(feature = "ply")]
mod ply;
#[cfg(feature = "stl")]
mod stl;
#[cfg(feature = "threemf")]
mod threemf;

/// Every truncation of `good` and two mutations of every byte go through `read`: it may refuse
/// them, it must not panic, and what it accepts must be a valid scene.
pub(crate) fn hammer(good: &[u8], read: &dyn Fn(&[u8]) -> Result<Scene>) {
    let check = |bytes: &[u8]| {
        if let Ok(scene) = read(bytes) {
            scene.validate().unwrap();
        }
    };
    for len in 0..good.len() {
        check(&good[..len]);
    }
    for i in 0..good.len() {
        let mut b = good.to_vec();
        b[i] ^= 0xA5;
        check(&b);
        b[i] = 0xFF;
        check(&b);
    }
    check(b"\x00\x01garbage\xff\xfe");
}

pub(crate) fn tri_mesh() -> Mesh {
    Mesh {
        name: "Tri".into(),
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        parts: vec![Part { corners: [0, 1, 2, 0, 3, 1].map(Corner::at).to_vec(), ..Part::default() }],
        ..Mesh::default()
    }
}

#[test]
fn validate_catches_what_writers_cannot_take() {
    let mut m = tri_mesh();
    m.validate().unwrap();
    m.parts[0].corners[0].position = 4;
    assert!(m.validate().is_err());
    let mut m = tri_mesh();
    m.parts[0].corners[0].normal = 0;
    assert!(m.validate().is_err(), "a normal index without normals");
    let mut m = tri_mesh();
    m.parts[0].face_sizes = vec![4, 3];
    assert!(m.validate().is_err());
    let mut m = tri_mesh();
    m.colors = vec![[1.0; 4]];
    assert!(m.validate().is_err());
    let mut m = tri_mesh();
    m.parts[0].has_tangents = true;
    assert!(m.validate().is_err(), "a part names tangents without a per-position array");
    let mut s = Scene::from_meshes(vec![tri_mesh()], vec![]);
    s.validate().unwrap();
    s.meshes[0].parts[0].material = Some(0);
    assert!(s.validate().is_err());
    let mut s = Scene::from_meshes(vec![tri_mesh()], vec![]);
    s.nodes[0].children.push(7);
    assert!(s.validate().is_err());
}

#[test]
fn unified_gives_one_vertex_per_distinct_corner() {
    // Two triangles sharing an edge; the shared positions carry different normals.
    let mut m = tri_mesh();
    m.normals = vec![[0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];
    m.colors = vec![[1.0, 0.0, 0.0, 1.0]; 4];
    for (i, c) in m.parts[0].corners.iter_mut().enumerate() {
        c.normal = (i / 3) as u32;
    }
    let u = m.unified();
    assert_eq!(u.positions.len(), 6);
    assert_eq!(u.normals.len(), 6);
    assert_eq!(u.colors.len(), 6);
    assert!(u.uvs.is_empty());
    assert_eq!(u.corners.iter().map(|c| c.position).collect::<Vec<_>>(), [0, 1, 2, 0, 3, 1]);
    assert_eq!(u.parts, [vec![[0, 1, 2], [3, 4, 5]]]);
    // Without normals the shared positions are shared vertices.
    let u = tri_mesh().unified();
    assert_eq!((u.positions.len(), u.normals.len()), (4, 0));
    assert_eq!(u.parts, [vec![[0, 1, 2], [0, 3, 1]]]);
    // A corner out of range drops its triangle, not the process.
    let mut bad = tri_mesh();
    bad.parts[0].corners[4].position = 99;
    assert_eq!(bad.unified().parts, [vec![[0, 1, 2]]]);
}

#[test]
fn transforms() {
    let mut t = IDENTITY;
    t[12] = 5.0;
    let mut s = IDENTITY;
    s[0] = 2.0;
    // Scale first, then move.
    assert_eq!(transform_point(&mat_mul(&t, &s), [1.0, 1.0, 1.0]), [7.0, 1.0, 1.0]);
    // A non-uniform scale tilts a normal the other way.
    let n = transform_normal(&s, [1.0, 1.0, 0.0]);
    assert!((n[0] * 2.0 - n[1]).abs() < 1e-12 && n[1] > 0.0, "{n:?}");
    // A mirror keeps the normal pointing out of the mirrored surface.
    let mut mirror = IDENTITY;
    mirror[0] = -1.0;
    assert_eq!(transform_normal(&mirror, [1.0, 0.0, 0.0]).map(|x| x.round()), [-1.0, 0.0, 0.0]);
    assert_eq!(transform_normal(&[0.0; 16], [0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]);
}
