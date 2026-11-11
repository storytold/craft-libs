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
fn part_faces_and_fans() {
    let quad = Part { face_sizes: vec![4, 3], corners: (0..7).map(Corner::at).collect(), ..Part::default() };
    assert_eq!(quad.faces().map(<[Corner]>::len).collect::<Vec<_>>(), [4, 3]);
    assert_eq!(quad.triangle_count(), 3);
    let tris: Vec<[u32; 3]> = quad.triangles().map(|t| t.map(|c| c.position)).collect();
    assert_eq!(tris, [[0, 1, 2], [0, 2, 3], [4, 5, 6]]);
    // Sizes that overrun the corners stop the iteration instead of panicking.
    let short = Part { face_sizes: vec![3, 9], corners: (0..5).map(Corner::at).collect(), ..Part::default() };
    assert_eq!(short.faces().count(), 1);
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

fn chain(len: usize) -> Scene {
    let mut s = Scene::from_meshes(vec![tri_mesh()], vec![]);
    s.nodes = (0..len)
        .map(|i| Node { children: if i + 1 < len { vec![i + 1] } else { vec![] }, mesh: (i + 1 == len).then_some(0), ..Node::default() })
        .collect();
    s.roots = vec![0];
    s
}

#[test]
fn instances_walk_the_hierarchy_within_limits() {
    let mut s = chain(3);
    s.nodes[0].transform[12] = 1.0;
    s.nodes[2].transform[13] = 2.0;
    let placed = s.instances(&Limits::default()).unwrap();
    assert_eq!(placed.len(), 1);
    assert_eq!(transform_point(&placed[0].1, [0.0; 3]), [1.0, 2.0, 0.0]);
    // Limit: depth.
    let limits = Limits { max_depth: 4, ..Limits::default() };
    assert!(chain(5).instances(&limits).is_ok());
    assert!(matches!(chain(6).instances(&limits), Err(Error::TooLarge(_))));
    // Limit: visits. A cycle ends there too.
    let limits = Limits { max_nodes: 5, max_depth: usize::MAX, ..Limits::default() };
    assert!(chain(5).instances(&limits).is_ok());
    assert!(matches!(chain(6).instances(&limits), Err(Error::TooLarge(_))));
    let mut cycle = chain(2);
    cycle.nodes[1].children = vec![0];
    assert!(matches!(cycle.instances(&Limits::default()), Err(Error::TooLarge(_))));
    // A child that does not exist is an error, not a panic.
    let mut missing = chain(1);
    missing.nodes[0].children = vec![9];
    assert!(matches!(missing.instances(&Limits::default()), Err(Error::Malformed(_))));
}

#[test]
fn srgb_round_trips() {
    for c in [0.0f32, 0.002, 0.2, 0.5, 1.0] {
        assert!((linear_to_srgb(srgb_to_linear(c)) - c).abs() < 1e-6);
    }
}
