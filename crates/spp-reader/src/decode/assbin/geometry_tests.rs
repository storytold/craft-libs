use super::scene_mesh::ScenePart as MeshPart;
use super::test_writer::{Buf, IDENTITY, header, node, scene_from};
use super::*;

/// A unit quad in the XY plane facing +Z, stored as one 4-index polygon, with tangent frames
/// and one UV set (OpenGL-style: v = 0 at the bottom).
fn quad_mesh() -> Buf {
    let mut b = Buf::default();
    b.string("Quad").u32(8).u32(0).u32(0x1 | 0x2 | 0x4 | 0x100);
    b.u32(4).f32s(&[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.]);
    b.u32(4).f32s(&[0., 0., 1.].repeat(4));
    b.u32(4).f32s(&[1., 0., 0.].repeat(4));
    b.u32(4).f32s(&[0., 1., 0.].repeat(4));
    b.u32(2).u32(4).f32s(&[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.]);
    b.u32(1).u32(1).u32(4).u32(0).u32(1).u32(2).u32(3);
    b.u32(0);
    b
}

fn translate_x(x: f32) -> [f32; 16] {
    let mut m = IDENTITY;
    m[3] = x;
    m
}

fn mirror_x() -> [f32; 16] {
    let mut m = IDENTITY;
    m[0] = -1.0;
    m
}

/// The quad placed twice: shifted by 10 along X under a parent that scales by 2, and mirrored.
fn two_placements() -> Vec<u8> {
    let mut scale2 = IDENTITY;
    [0, 5, 10].iter().for_each(|&i| scale2[i] = 2.0);
    let shifted = node("Shifted", translate_x(10.0), &[0], Vec::new());
    let parent = node("Parent", scale2, &[], vec![shifted]);
    let mirrored = node("Mirrored", mirror_x(), &[0], Vec::new());
    let root = node("RootNode", IDENTITY, &[], vec![parent, mirrored]);
    let mut file = header(7, false);
    file.0.extend_from_slice(&scene_from(root, vec![quad_mesh()], &["Ink"]).0);
    file.0
}

fn parts() -> Vec<MeshPart> {
    let mesh = AssbinGeometry::parse(&two_placements()).unwrap();
    assert_eq!(mesh.materials, vec!["Ink".to_string()]);
    assert_eq!(mesh.validate(), Ok(()));
    mesh.parts
}

#[test]
fn applies_world_transforms_and_triangulates() {
    let parts = parts();
    assert_eq!(parts.len(), 2);
    let shifted = &parts[0];
    assert_eq!(shifted.name, "Quad [Shifted]");
    assert_eq!(shifted.material, 0);
    // Parent scale 2 applies after the child's translation by 10.
    assert_eq!(shifted.positions[2], [22.0, 2.0, 0.0]);
    assert_eq!(shifted.triangles, vec![[0, 1, 2], [0, 2, 3]]);
    assert_eq!(shifted.normals[0], [0.0, 0.0, 1.0]);
    assert_eq!(shifted.tangents[0], [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn flips_v_to_the_gltf_convention() {
    let uvs = &parts()[0].uv_sets;
    assert_eq!(uvs.len(), 1);
    assert_eq!(uvs[0], vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
}

#[test]
fn mirrors_reverse_winding_and_handedness() {
    let mirrored = &parts()[1];
    assert_eq!(mirrored.name, "Quad [Mirrored]");
    assert_eq!(mirrored.positions[1], [-1.0, 0.0, 0.0]);
    assert_eq!(mirrored.triangles, vec![[0, 2, 1], [0, 3, 2]]);
    assert_eq!(mirrored.normals[0], [0.0, 0.0, 1.0]);
    assert_eq!(mirrored.tangents[0], [-1.0, 0.0, 0.0, -1.0]);
}

#[test]
fn unplaced_meshes_are_kept_once_and_counts_match_the_summary() {
    let mut file = header(7, false);
    let root = node("RootNode", IDENTITY, &[], Vec::new());
    file.0.extend_from_slice(&scene_from(root, vec![quad_mesh()], &["Ink"]).0);
    let mesh = AssbinGeometry::parse(&file.0).unwrap();
    assert_eq!(mesh.parts.len(), 1);
    assert_eq!(mesh.parts[0].name, "Quad");
    let summary = AssbinScene::parse(&file.0).unwrap();
    assert_eq!(summary.meshes[0].vertices as usize, mesh.parts[0].positions.len());
}

#[test]
fn bad_references_and_truncation_are_errors() {
    let data = two_placements();
    for len in 0..data.len() - 16 {
        assert!(AssbinGeometry::parse(&data[..len]).is_err(), "length {len}");
    }
    let root = node("RootNode", IDENTITY, &[3], Vec::new());
    let mut file = header(7, false);
    file.0.extend_from_slice(&scene_from(root, vec![quad_mesh()], &["Ink"]).0);
    assert!(matches!(AssbinGeometry::parse(&file.0), Err(DecodeError::Invalid { .. })));
    let mut no_material = header(7, false);
    let root = node("RootNode", IDENTITY, &[0], Vec::new());
    no_material.0.extend_from_slice(&scene_from(root, vec![quad_mesh()], &[]).0);
    assert!(matches!(AssbinGeometry::parse(&no_material.0), Err(DecodeError::Invalid { .. })));
}
