#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_mesh::{AlphaMode, Corner, Image, Material, Mesh, Node, Part, Scene, TextureRef, UvTransform, Wrap};

use super::readback::assemble;
use super::{TextureWrap, UpAxis, mesh_scene_from, write_prc};

/// A PNG signature and a header claiming 1 x 2: enough for the writer.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x02";

/// One triangle in the XY plane, its corners sharing index i for position, normal and UV.
fn triangle(material: Option<usize>) -> Mesh {
    Mesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        uv_sets: vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 0.25]]],
        parts: vec![Part { material, corners: (0..3).map(Corner::all).collect(), ..Part::default() }],
        ..Mesh::default()
    }
}

fn textured(alpha_mode: AlphaMode) -> Material {
    let slot = TextureRef { wrap: [Wrap::Mirror, Wrap::Clamp], ..TextureRef::default() };
    Material { alpha_mode, base_color_texture: Some(slot), ..Material::plain("t", [1.0; 4]) }
}

fn with_image(mut scene: Scene, data: &[u8]) -> Scene {
    scene.images = vec![Image { data: Some(data.to_vec()), ..Image::default() }];
    scene
}

#[test]
fn a_y_up_scene_is_turned_to_z_up_at_its_roots_only() {
    let mut scene = Scene::from_meshes(vec![triangle(None)], Vec::new());
    // The root holds a child at +1 along y: a turned root carries it to +1 along z.
    let child = Node { mesh: Some(0), transform: translate_y(), ..Node::default() };
    scene.nodes.push(child);
    scene.nodes[0].children = vec![1];
    let out = mesh_scene_from(&scene, 1000.0, UpAxis::Y);
    assert!(out.scene.nodes[1].matrix.is_some_and(|m| m[13] == 1.0), "the child keeps its own transform");
    let model = assemble(&write_prc(&out.scene).unwrap().prc);
    let ys: Vec<f64> = model.meshes.iter().flat_map(|m| m.positions.iter().map(|p| p[1])).collect();
    let zs: Vec<f64> = model.meshes.iter().flat_map(|m| m.positions.iter().map(|p| p[2])).collect();
    assert!(zs.iter().any(|z| (*z - 1.0).abs() < 1e-9), "+y became +z: {zs:?}");
    assert!(zs.iter().any(|z| (*z - 2.0).abs() < 1e-9), "the child sits one more up: {zs:?}");
    assert!(ys.iter().all(|y| *y <= 1e-9), "+y became -z's partner, so y is never positive: {ys:?}");
    let z_up = mesh_scene_from(&scene, 1000.0, UpAxis::Z);
    assert_eq!(z_up.scene.nodes[0].matrix, None, "a Z-up root is left alone");
}

#[test]
fn colours_go_from_linear_to_display_values() {
    // Linear 0.2159 is display 0.502, 128 of 255; the emissive likewise.
    let mut material = Material::plain("m", [0.215_9, 1.0, 0.0, 1.0]);
    material.emissive = [0.215_9, 0.0, 0.0];
    let scene = Scene::from_meshes(vec![triangle(Some(0))], vec![material]);
    let out = mesh_scene_from(&scene, 1000.0, UpAxis::Z);
    let m = &out.scene.materials[0];
    assert!((m.rgba[0] - 0.502).abs() < 1e-3, "{:?}", m.rgba);
    assert!((m.emissive[0] - 0.502).abs() < 1e-3, "{:?}", m.emissive);
    let model = assemble(&write_prc(&out.scene).unwrap().prc);
    assert_eq!(model.colours, vec![Some([128, 255, 0, 255])]);
}

#[test]
fn alpha_follows_the_mode() {
    let modes = [(AlphaMode::Opaque, 1.0), (AlphaMode::Blend, 0.3), (AlphaMode::Mask(0.25), 1.0), (AlphaMode::Mask(0.5), 0.0)];
    for (mode, want) in modes {
        let material = Material { alpha_mode: mode, ..Material::plain("m", [1.0, 1.0, 1.0, 0.3]) };
        let out = mesh_scene_from(&Scene::from_meshes(vec![triangle(Some(0))], vec![material]), 1.0, UpAxis::Z);
        assert!((out.scene.materials[0].rgba[3] - want).abs() < 1e-6, "{mode:?}: {:?}", out.scene.materials[0].rgba);
    }
}

#[test]
fn a_texture_brings_its_wrap_and_bottom_left_uvs() {
    let mut material = textured(AlphaMode::Blend);
    let transform = UvTransform { offset: [0.5, 0.0], rotation: 0.0, scale: [1.0, 1.0] };
    material.base_color_texture = material.base_color_texture.map(|t| TextureRef { transform: Some(transform), ..t });
    let scene = with_image(Scene::from_meshes(vec![triangle(Some(0))], vec![material]), PNG);
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(out.scene.materials[0].texture, Some(0));
    let t = &out.scene.textures[0];
    assert_eq!((t.wrap, t.alpha, t.image.as_slice()), ([TextureWrap::MirroredRepeat, TextureWrap::Clamp], true, PNG));
    assert_eq!(out.scene.meshes[0].primitives[0].uvs, vec![[0.5, 1.0], [1.5, 1.0], [0.5, 0.75]]);
    assert_eq!(write_prc(&out.scene).unwrap().textured, 1);
}

#[test]
fn one_image_slot_is_one_texture_and_an_unusable_one_is_dropped() {
    let two = vec![textured(AlphaMode::Opaque), textured(AlphaMode::Opaque)];
    let scene = with_image(Scene::from_meshes(vec![triangle(Some(0)), triangle(Some(1))], two.clone()), PNG);
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    assert_eq!(out.scene.textures.len(), 1);
    assert_eq!(out.scene.materials.iter().map(|m| m.texture).collect::<Vec<_>>(), vec![Some(0), Some(0)]);

    let broken = with_image(Scene::from_meshes(vec![triangle(Some(0))], two), b"\x89PNG\r\n\x1a\n");
    let out = mesh_scene_from(&broken, 1.0, UpAxis::Z);
    assert!(out.scene.textures.is_empty());
    assert!(out.scene.meshes[0].primitives[0].uvs.is_empty(), "no UVs without a texture");
    assert!(out.warnings.iter().any(|w| w.starts_with("2 base-colour")), "{:?}", out.warnings);
    assert!(write_prc(&out.scene).is_ok());
}

#[test]
fn a_mask_is_noted_only_when_its_texture_is_kept() {
    let scene = with_image(Scene::from_meshes(vec![triangle(Some(0))], vec![textured(AlphaMode::Mask(0.5))]), PNG);
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    assert!(out.warnings.iter().any(|w| w.starts_with("1 alpha-mask")), "{:?}", out.warnings);
}

#[test]
fn corners_become_one_vertex_each_and_missing_normals_stay_missing() {
    let mut mesh = triangle(None);
    // A quad over four positions: two triangles sharing two corners.
    mesh.positions.push([1.0, 1.0, 0.0]);
    // Only two corners name a normal, so none is kept.
    mesh.parts[0].corners = vec![Corner::all(0), Corner::all(1), Corner::at(3), Corner::at(2)];
    mesh.parts[0].face_sizes = vec![4];
    let out = mesh_scene_from(&Scene::from_meshes(vec![mesh], Vec::new()), 1.0, UpAxis::Z);
    let p = &out.scene.meshes[0].primitives[0];
    assert_eq!((p.positions.len(), p.triangles.len()), (4, 2));
    assert!(p.normals.is_empty(), "the corners name no normal");
    assert_eq!(write_prc(&out.scene).unwrap().flat_normals, 1);
}

#[test]
fn hostile_indices_and_hierarchies_are_dropped_not_followed() {
    let mut mesh = triangle(Some(7));
    // A second triangle over two unused positions and one past the list, last.
    mesh.positions.extend([[2.0, 0.0, 0.0], [2.0, 1.0, 0.0]]);
    mesh.parts[0].corners.extend([Corner::at(3), Corner::at(4), Corner::at(99)]);
    mesh.colors = vec![[1.0; 4]; 3];
    let mut scene = Scene::from_meshes(vec![mesh], Vec::new());
    // Node 0 is its own child, a root names no node, and a node names no mesh.
    scene.nodes[0].children = vec![0, 1];
    scene.nodes.push(Node { mesh: Some(5), ..Node::default() });
    scene.roots.push(42);
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    let joined = out.warnings.join("\n");
    for want in [
        "1 part(s) name a material",
        "1 triangle(s) name a vertex",
        "1 mesh(es) have vertex colours",
        "node(s) name a mesh",
        "past the copy or depth limit",
        "1 root or child reference(s) name a node",
    ] {
        assert!(joined.contains(want), "{want}: {joined}");
    }
    let p = &out.scene.meshes[0].primitives[0];
    assert_eq!((p.triangles.len(), p.positions.len()), (1, 3), "the dropped triangle leaves no vertex");
    assert_eq!(out.scene.materials.len(), 0);
    write_prc(&out.scene).unwrap();
}

#[test]
fn a_shared_subtree_is_copied_under_each_parent() {
    let mut scene = Scene::from_meshes(vec![triangle(None)], Vec::new());
    scene.nodes[0].mesh = None;
    scene.nodes[0].children = vec![1, 1];
    scene.nodes.push(Node { name: "leaf".into(), mesh: Some(0), ..Node::default() });
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    assert_eq!(out.scene.nodes.len(), 3);
    assert_eq!(out.scene.nodes[0].children, vec![1, 2]);
    assert_eq!(assemble(&write_prc(&out.scene).unwrap().prc).triangles, 2, "drawn twice");
}

/// One unit along +y, column-major.
fn translate_y() -> craft_mesh::Mat4 {
    let mut m = craft_mesh::IDENTITY;
    m[13] = 1.0;
    m
}

#[test]
fn a_subtree_shared_too_often_stops_at_the_copy_limit() {
    // Twenty levels, each node listing the next one twice: 2^20 paths to the leaf.
    let mut scene = Scene::from_meshes(vec![triangle(None)], Vec::new());
    scene.nodes = (0..20).map(|i| Node { children: vec![i + 1, i + 1], ..Node::default() }).collect();
    scene.nodes.push(Node { mesh: Some(0), ..Node::default() });
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    assert_eq!(out.scene.nodes.len(), super::MAX_NODE_COPIES);
    assert!(out.warnings.iter().any(|w| w.contains("past the copy or depth limit")), "{:?}", out.warnings);
    write_prc(&out.scene).unwrap();
}

#[test]
fn a_node_listing_itself_many_times_is_cut_without_queueing_every_reference() {
    // Before the queue was bounded this took about ten billion discarded visits.
    let mut scene = Scene::from_meshes(vec![triangle(None)], Vec::new());
    scene.nodes[0].children = vec![0; 100_000];
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    assert!(out.scene.nodes.len() <= super::MAX_NODE_COPIES);
    // Over 2^32, so the count must not be a 32-bit `usize`.
    let cut = format!("{} subtree(s)", 100_000u64 * 100_000 - (out.scene.nodes.len() as u64 - 1));
    assert!(out.warnings.iter().any(|w| w.starts_with(&cut)), "{cut}: {:?}", out.warnings);
}

#[test]
fn more_roots_than_the_copy_limit_are_cut() {
    let mut scene = Scene::from_meshes(vec![triangle(None)], Vec::new());
    scene.roots = vec![0; super::MAX_NODE_COPIES + 5];
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    assert_eq!(out.scene.nodes.len(), super::MAX_NODE_COPIES);
    assert!(out.warnings.iter().any(|w| w.starts_with("5 subtree(s)")), "{:?}", out.warnings);
}

#[test]
fn invalid_children_are_counted_once_per_copy_not_queued() {
    // 50,000 references naming nothing, then 50,000 to the node itself: each copy queues only the valid
    // ones, so the invalid ones cannot keep refilling the queue (before, this took billions of visits).
    let mut scene = Scene::from_meshes(vec![triangle(None)], Vec::new());
    scene.nodes[0].children = (0..100_000).map(|i| if i < 50_000 { 9_999_999 } else { 0 }).collect();
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    let copies = out.scene.nodes.len() as u64;
    assert_eq!(copies, super::MAX_NODE_COPIES as u64);
    let bad = format!("{} ", 50_000 * copies);
    assert!(out.warnings.iter().any(|w| w.starts_with(&bad) && w.contains("node")), "{bad}: {:?}", out.warnings);
}

#[test]
fn a_non_finite_colour_reaches_the_writer_unrepaired() {
    let bad = [f32::INFINITY, f32::NEG_INFINITY, f32::NAN];
    for (i, v) in bad.into_iter().enumerate() {
        for field in 0..3 {
            let mut m = Material::plain("m", [0.5, 0.5, 0.5, 0.5]);
            match field {
                0 => m.base_color[i] = v,
                1 => m.emissive[i] = v,
                _ => (m.alpha_mode, m.base_color[3]) = (AlphaMode::Mask(0.5), v),
            }
            let scene = Scene::from_meshes(vec![triangle(Some(0))], vec![m]);
            let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
            let written = write_prc(&out.scene);
            assert!(matches!(written, Err(super::PrcWriteError::BadNumber(_))), "{v} in field {field}: {written:?}");
        }
    }
}

#[test]
fn an_image_shared_by_many_materials_is_probed_once() {
    // A JPEG whose frame header follows 4 MiB of fill bytes: probing it per material took
    // 20,000 walks of the fill (before, about 80 billion steps).
    let mut jpeg = vec![0xFF, 0xD8];
    jpeg.resize(4 << 20, 0xFF);
    jpeg.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 2, 0, 1, 1, 1, 0x11, 0]);
    let mut scene = with_image(Scene::from_meshes(vec![triangle(Some(0))], Vec::new()), &jpeg);
    scene.materials = vec![textured(AlphaMode::Opaque); 20_000];
    let out = mesh_scene_from(&scene, 1.0, UpAxis::Z);
    assert_eq!(out.scene.textures.len(), 1);
    assert_eq!(out.scene.materials.iter().filter(|m| m.texture == Some(0)).count(), 20_000);
}
