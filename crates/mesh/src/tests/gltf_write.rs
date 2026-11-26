//! The GLB writer's skins, morph targets and extension data, read back with the glTF reader and
//! checked against the rules of the Khronos glTF 2.0 specification.

use std::cell::Cell;

use serde_json::{Map, Value, json};

use crate::gltf::{Writer, extension_names, glb_container, read, split_glb, write_glb};
use crate::{Corner, Error, IDENTITY, Image, Limits, Mat4, Material, Mesh, MorphTarget, Node, Part, Scene, Skin, TextureRef, Wrap, no_external};

fn parse(bytes: &[u8]) -> crate::Result<Scene> {
    read(bytes, &Limits::default(), &mut no_external)
}

/// The JSON chunk of a GLB.
fn json_of(glb: &[u8]) -> Value {
    serde_json::from_slice(split_glb(glb).unwrap().0).unwrap()
}

fn object(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(o) => o,
        _ => panic!("not an object"),
    }
}

fn moved(x: f64, y: f64, z: f64) -> Mat4 {
    let mut m = IDENTITY;
    m[12..15].copy_from_slice(&[x, y, z]);
    m
}

/// A quad skinned to two joints, with three morph targets: `smile` moves every vertex and the
/// normal, `blink` one vertex, and `rest` nothing.
fn rig() -> Scene {
    let mesh = Mesh {
        name: "Face".into(),
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]],
        joints: vec![[0, 1, 0, 0], [0, 1, 0, 0], [1, 0, 0, 0], [1, 0, 0, 0]],
        weights: vec![[0.75, 0.25, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
        parts: vec![Part {
            material: Some(0),
            face_sizes: vec![4],
            corners: (0..4).map(|i| Corner { position: i, normal: 0, uv: crate::NONE }).collect(),
            ..Part::default()
        }],
        morph_targets: vec![
            MorphTarget { name: "smile".into(), positions: vec![[0.0, 0.0, 1.0]; 4], normals: vec![[0.5, 0.0, 0.0]], tangents: vec![] },
            MorphTarget { name: "blink".into(), positions: vec![[0.0; 3], [0.0; 3], [0.0; 3], [0.0, -0.25, 0.0]], ..MorphTarget::default() },
            MorphTarget { name: "rest".into(), ..MorphTarget::default() },
        ],
        morph_weights: vec![0.25, 0.75, 0.0],
        ..Mesh::default()
    };
    let mut scene = Scene::from_meshes(vec![mesh, Mesh { name: "empty".into(), ..Mesh::default() }], vec![Material::plain("Skin", [1.0; 4])]);
    // 0: the face, skinned, with weights for its first target only. 1: a node whose mesh has no
    // triangles, so it is written without a mesh, and so without its skin.
    scene.nodes[0].skin = Some(0);
    scene.nodes[0].morph_weights = vec![0.5];
    scene.nodes[1].skin = Some(0);
    // 2 → 3: the joints.
    scene.nodes.push(Node { name: "Hips".into(), transform: moved(0.0, 1.0, 0.0), children: vec![3], ..Node::default() });
    scene.nodes.push(Node { name: "Head".into(), transform: moved(0.0, 0.5, 0.0), ..Node::default() });
    scene.roots = vec![0, 1, 2];
    scene.skins = vec![Skin { name: "Body".into(), joints: vec![2, 3], inverse_bind: vec![moved(0.0, -1.0, 0.0), moved(0.0, -1.5, 0.0)] }];
    scene
}

/// Every vertex of the mesh's [`Mesh::unified`] triangles, shaped by `weights` as glTF shapes
/// it: the base position and normal plus each target's offset times its weight.
fn shaped(mesh: &Mesh, weights: &[f32]) -> Vec<([f64; 3], [f64; 3])> {
    let u = mesh.unified();
    u.corners
        .iter()
        .map(|c| {
            let (mut p, mut n) = (mesh.positions[c.position as usize], mesh.normals[c.normal as usize]);
            for (t, w) in mesh.morph_targets.iter().zip(weights) {
                let w = f64::from(*w);
                let dp = t.positions.get(c.position as usize).copied().unwrap_or_default();
                let dn = t.normals.get(c.normal as usize).copied().unwrap_or_default();
                for ((x, d), (y, e)) in p.iter_mut().zip(dp).zip(n.iter_mut().zip(dn)) {
                    *x += w * d;
                    *y += w * e;
                }
            }
            (p, n)
        })
        .collect()
}

/// The joints and weights of every vertex of the mesh's [`Mesh::unified`] triangles.
fn skinning(mesh: &Mesh) -> Vec<([u16; 4], [f32; 4])> {
    mesh.unified().corners.iter().map(|c| (mesh.joints[c.position as usize], mesh.weights[c.position as usize])).collect()
}

#[test]
fn skins_and_morph_targets_read_back() {
    let scene = rig();
    let back = parse(&write_glb(&scene, &Limits::default()).unwrap()).unwrap();
    back.validate().unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(back.skins, scene.skins);
    assert_eq!(back.nodes.iter().map(|n| (n.mesh, n.skin)).collect::<Vec<_>>(), [(Some(0), Some(0)), (None, None), (None, None), (None, None)]);
    let (face, read) = (&scene.meshes[0], &back.meshes[0]);
    assert_eq!(skinning(read), skinning(face));
    let names: Vec<&str> = read.morph_targets.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["smile", "blink", "rest"]);
    assert_eq!(read.morph_weights, face.morph_weights);
    // A node that sets fewer weights than its mesh has targets takes the mesh's for the rest.
    assert_eq!(back.nodes[0].morph_weights, [0.5, 0.75, 0.0]);
    assert!(back.nodes[1].morph_weights.is_empty());
    for weights in [&[1.0, 0.0, 0.0][..], &[0.0, 1.0, 0.0], &[0.5, 0.75, 1.0]] {
        assert_eq!(shaped(read, weights), shaped(face, weights), "weights {weights:?}");
    }
    // Worked out by hand from the offsets in `rig`, with the node's weights.
    let p: Vec<[f64; 3]> = shaped(read, &back.nodes[0].morph_weights).into_iter().map(|(p, _)| p).collect();
    assert_eq!(p, [[0.0, 0.0, 0.5], [1.0, 0.0, 0.5], [1.0, 1.0, 0.5], [0.0, 0.8125, 0.5]]);
}

#[test]
fn written_rigging_follows_the_specification() {
    let j = json_of(&write_glb(&rig(), &Limits::default()).unwrap());
    let accessor = |a: &Value| j["accessors"][a.as_u64().unwrap() as usize].clone();
    let mesh = &j["meshes"][0];
    let targets = &mesh["primitives"][0]["targets"];
    assert_eq!(targets.as_array().unwrap().len(), 3);
    // Every primitive of a mesh has as many targets as the mesh has weights.
    assert!(mesh["primitives"].as_array().unwrap().iter().all(|p| p["targets"] == *targets));
    assert_eq!(mesh["weights"], json!([0.25, 0.75, 0.0]));
    assert_eq!(mesh["extras"], json!({ "targetNames": ["smile", "blink", "rest"] }));
    // A node's weights are one per target of its mesh; `skin` and `weights` need a `mesh`.
    assert_eq!(j["nodes"][0]["weights"], json!([0.5, 0.75, 0.0]));
    assert_eq!(j["nodes"][0]["skin"], json!(0));
    for n in j["nodes"].as_array().unwrap() {
        assert!(n.get("mesh").is_some() || (n.get("skin").is_none() && n.get("weights").is_none()), "{n}");
    }
    // Every target's POSITION accessor carries its bounds.
    for t in targets.as_array().unwrap() {
        let a = accessor(&t["POSITION"]);
        assert!(a.get("min").is_some() && a.get("max").is_some(), "{a}");
        assert_eq!((a["type"].as_str(), a["count"].as_u64()), (Some("VEC3"), Some(4)));
    }
    let (smile, blink, rest) = (accessor(&targets[0]["POSITION"]), accessor(&targets[1]["POSITION"]), accessor(&targets[2]["POSITION"]));
    // A target that moves every vertex is written whole.
    assert!(smile.get("bufferView").is_some() && smile.get("sparse").is_none());
    assert!(targets[0].get("NORMAL").is_some() && targets[1].get("NORMAL").is_none());
    // One that moves one vertex names it over zeros; one that moves nothing is only zeros.
    assert!(blink.get("bufferView").is_none());
    assert_eq!(blink["sparse"]["count"], json!(1));
    assert_eq!(blink["sparse"]["indices"]["componentType"], json!(5125));
    assert_eq!((blink["min"].clone(), blink["max"].clone()), (json!([0.0, -0.25, 0.0]), json!([0.0, 0.0, 0.0])));
    assert!(rest.get("bufferView").is_none() && rest.get("sparse").is_none());
    assert_eq!((rest["min"].clone(), rest["max"].clone()), (json!([0.0, 0.0, 0.0]), json!([0.0, 0.0, 0.0])));
    // Sparse indices and values have no buffer view target.
    for key in ["indices", "values"] {
        let view = &j["bufferViews"][blink["sparse"][key]["bufferView"].as_u64().unwrap() as usize];
        assert!(view.get("target").is_none(), "{view}");
    }
    // Skinning attributes and the inverse bind matrices.
    let attrs = &mesh["primitives"][0]["attributes"];
    let (joints, weights) = (accessor(&attrs["JOINTS_0"]), accessor(&attrs["WEIGHTS_0"]));
    assert_eq!((joints["type"].as_str(), joints["componentType"].as_u64()), (Some("VEC4"), Some(5123)));
    assert_eq!((weights["type"].as_str(), weights["componentType"].as_u64()), (Some("VEC4"), Some(5126)));
    let skin = &j["skins"][0];
    assert_eq!((skin["joints"].clone(), skin["name"].clone()), (json!([2, 3]), json!("Body")));
    let ibm = accessor(&skin["inverseBindMatrices"]);
    assert_eq!((ibm["type"].as_str(), ibm["count"].as_u64()), (Some("MAT4"), Some(2)));
    let view = &j["bufferViews"][ibm["bufferView"].as_u64().unwrap() as usize];
    assert!(view.get("target").is_none(), "{view}");
}

/// [`rig`] with extension data on the file, nodes, meshes and materials, as a VRM file holds it.
fn rig_with_extensions() -> Scene {
    let mut s = rig();
    s.extensions = object(json!({
        "VRMC_vrm": { "specVersion": "1.0", "humanoid": { "humanBones": { "hips": { "node": 2 }, "head": { "node": 3 } } } },
        // Lights are read into `Scene::lights`, which the writer does not write.
        "KHR_lights_punctual": { "lights": [{ "type": "point" }] },
    }));
    s.extras = Some(json!({ "author": "test" }));
    s.nodes[3].extensions = object(json!({ "VRMC_node_constraint": { "specVersion": "1.0", "constraint": { "roll": { "source": 2 } } } }));
    s.nodes[2].extensions = object(json!({ "KHR_lights_punctual": { "light": 0 } }));
    s.nodes[2].extras = Some(json!(["kept"]));
    s.meshes[0].extras = Some(json!({ "kind": "face", "targetNames": ["old"] }));
    s.meshes[0].extensions = object(json!({ "EXAMPLE_mesh": { "level": 2 } }));
    s.images = vec![
        Image { name: "skin".into(), data: Some(b"\x89PNG skin".to_vec()), ..Image::default() },
        Image { name: "shade".into(), data: Some(b"\x89PNG shade".to_vec()), ..Image::default() },
    ];
    // The file's textures, in its order: the extension data names them by index.
    s.texture_variants =
        vec![TextureRef { image: 0, wrap: [Wrap::Clamp; 2], ..TextureRef::default() }, TextureRef { image: 1, ..TextureRef::default() }];
    let m = &mut s.materials[0];
    m.base_color_texture = Some(TextureRef { image: 1, ..TextureRef::default() });
    // The unlit extension is read into `Material::unlit`, which says it is not unlit.
    m.extensions = object(json!({
        "VRMC_materials_mtoon": {
            "specVersion": "1.0",
            "shadeMultiplyTexture": { "index": 0, "extensions": { "KHR_texture_transform": { "scale": [2.0, 2.0] } } },
        },
        "KHR_materials_unlit": {},
    }));
    m.extras = Some(json!({ "note": 1 }));
    s
}

#[test]
fn extension_data_is_written_and_listed() {
    let scene = rig_with_extensions();
    let glb = write_glb(&scene, &Limits::default()).unwrap();
    let j = json_of(&glb);
    let used: Vec<&str> = j["extensionsUsed"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    for name in ["VRMC_vrm", "VRMC_node_constraint", "VRMC_materials_mtoon", "KHR_texture_transform", "EXAMPLE_mesh"] {
        assert!(used.contains(&name), "{name} in {used:?}");
    }
    for name in ["KHR_lights_punctual", "KHR_materials_unlit"] {
        assert!(!used.contains(&name), "{name} in {used:?}");
    }
    assert!(j["extensions"].get("KHR_lights_punctual").is_none() && j["nodes"][2].get("extensions").is_none());
    // The file's textures keep their indices, so the extension data still names the right
    // ones, and a material slot that matches one of them uses it.
    assert_eq!(j["textures"].as_array().unwrap().len(), 2);
    assert_eq!((j["textures"][0]["source"].clone(), j["textures"][1]["source"].clone()), (json!(0), json!(1)));
    assert_eq!(j["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"], json!(1));
    let back = parse(&glb).unwrap();
    back.validate().unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(Value::Object(back.extensions.clone()), json!({ "VRMC_vrm": scene.extensions["VRMC_vrm"] }));
    assert_eq!(back.extras, scene.extras);
    assert_eq!(back.nodes[3].extensions, scene.nodes[3].extensions);
    assert_eq!((back.nodes[2].extensions.is_empty(), &back.nodes[2].extras), (true, &scene.nodes[2].extras));
    // The names of the targets replace the ones the mesh's extras held.
    assert_eq!(back.meshes[0].extras, Some(json!({ "kind": "face", "targetNames": ["smile", "blink", "rest"] })));
    assert_eq!(back.meshes[0].extensions, scene.meshes[0].extensions);
    let m = &back.materials[0];
    assert_eq!(Value::Object(m.extensions.clone()), json!({ "VRMC_materials_mtoon": scene.materials[0].extensions["VRMC_materials_mtoon"] }));
    assert_eq!((m.unlit, &m.extras), (false, &scene.materials[0].extras));
    assert_eq!(back.texture_variants.iter().map(|t| (t.image, t.wrap)).collect::<Vec<_>>(), [(0, [Wrap::Clamp; 2]), (1, [Wrap::Repeat; 2])]);
    // Targets without names leave the kept `targetNames` as they are.
    let mut s = rig_with_extensions();
    s.meshes[0].morph_targets.iter_mut().for_each(|t| t.name.clear());
    let j = json_of(&write_glb(&s, &Limits::default()).unwrap());
    assert_eq!(j["meshes"][0]["extras"], json!({ "kind": "face", "targetNames": ["old"] }));
}

/// A name that counts how often it is compared.
struct Counted<'a> {
    name: &'a str,
    count: &'a Cell<usize>,
}

impl PartialEq for Counted<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.count.set(self.count.get() + 1);
        self.name == other.name
    }
}

impl Eq for Counted<'_> {}

impl PartialOrd for Counted<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Counted<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.count.set(self.count.get() + 1);
        self.name.cmp(other.name)
    }
}

/// `n` extensions at the top of the file, each holding an object in an array that uses itself
/// again and one extension every one of them shares, so `3 * n` names are found and `n + 1` differ.
fn many_extensions(n: usize) -> Map<String, Value> {
    (0..n)
        .map(|i| {
            let name = format!("EXT_{i:05}");
            let inner = json!({ "nested": [{ "extensions": { name.clone(): {}, "SHARED": {} } }] });
            (name, inner)
        })
        .collect()
}

// A `Counted` orders by its name only; its counter is not part of the key.
#[allow(clippy::mutable_key_type)]
#[test]
fn many_extension_names_are_listed_once_in_n_log_n_comparisons() {
    const N: usize = 5000;
    let mut scene = rig();
    scene.extensions = many_extensions(N);
    // Extension keys inside the application's extras are not listed.
    scene.extras = Some(json!({ "extensions": { "IN_EXTRAS": {} } }));
    let j = json_of(&write_glb(&scene, &Limits::default()).unwrap());
    let mut expected: Vec<String> = (0..N).map(|i| format!("EXT_{i:05}")).collect();
    expected.push("SHARED".into());
    assert_eq!(j["extensionsUsed"], json!(expected));

    let root = Map::from_iter([("extensions".to_string(), Value::Object(many_extensions(N)))]);
    let count = Cell::new(0);
    let names = extension_names(&root, |name| Counted { name, count: &count });
    assert_eq!(names.len(), N + 1);
    // 3 * N insertions into a set of at most N + 1 names: about 13 comparisons each. A search of
    // every name found so far takes on the order of N * N.
    let limit = 3 * N * 32;
    assert!(count.get() <= limit, "{} comparisons for {} names (limit {limit})", count.get(), 3 * N);
}

#[test]
fn a_texture_left_out_for_want_of_an_image_is_named() {
    // The first texture has no image the reader reads, so the second one becomes the first.
    let doc = json!({
        "asset": { "version": "2.0" },
        "images": [{ "uri": "a.png" }],
        "textures": [{ "extensions": { "EXAMPLE_texture": { "source": 0 } } }, { "source": 0 }],
        "extensions": { "EXAMPLE_root": { "texture": 1 } },
    });
    let s = parse(&glb_container(&serde_json::to_vec(&doc).unwrap(), &[]).unwrap()).unwrap();
    assert_eq!(s.texture_variants.len(), 1);
    assert_eq!(s.warnings, ["1 texture(s) have no image that is read and were left out; the textures after them are numbered from one less"]);
}

#[test]
fn a_texture_of_a_webp_image_names_it_through_the_extension() {
    let webp = b"RIFF\x04\x00\x00\x00WEBPVP8 ".to_vec();
    let mut s = rig();
    s.images = vec![Image { data: Some(webp.clone()), ..Image::default() }, Image { uri: Some("b.WEBP?v=1#top".into()), ..Image::default() }];
    // The first texture is the file's and no material uses it; the second a material's.
    s.texture_variants = vec![TextureRef::default()];
    s.materials[0].base_color_texture = Some(TextureRef { image: 1, ..TextureRef::default() });
    let glb = write_glb(&s, &Limits::default()).unwrap();
    let j = json_of(&glb);
    assert_eq!(j["images"][0]["mimeType"], json!("image/webp"));
    for (k, image) in [(0, 0), (1, 1)] {
        let t = &j["textures"][k];
        assert!(t.get("source").is_none(), "{t}");
        assert_eq!(t["extensions"]["EXT_texture_webp"]["source"], json!(image));
    }
    assert_eq!(j["extensionsRequired"], json!(["EXT_texture_webp"]));
    assert!(j["extensionsUsed"].as_array().unwrap().contains(&json!("EXT_texture_webp")));
    let back = parse(&glb).unwrap();
    assert_eq!(back.texture_variants.iter().map(|t| t.image).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(back.materials[0].base_color_texture.map(|t| t.image), Some(1));
    assert_eq!(back.images[0].data.as_deref(), Some(&webp[..]));
    // Without a WebP image nothing is required.
    assert!(json_of(&write_glb(&rig(), &Limits::default()).unwrap()).get("extensionsRequired").is_none());
}

#[test]
fn a_named_image_keeps_its_media_type() {
    // The URI has no extension, so only the media type says the image is WebP.
    let mut s = rig();
    s.images = vec![Image { uri: Some("asset?id=42".into()), mime_type: Some("image/webp".into()), ..Image::default() }];
    s.materials[0].base_color_texture = Some(TextureRef::default());
    let first = write_glb(&s, &Limits::default()).unwrap();
    let j = json_of(&first);
    assert_eq!((j["images"][0]["uri"].clone(), j["images"][0]["mimeType"].clone()), (json!("asset?id=42"), json!("image/webp")));
    let back = parse(&first).unwrap();
    assert_eq!(back.images[0].mime_type.as_deref(), Some("image/webp"));
    let second = json_of(&write_glb(&back, &Limits::default()).unwrap());
    assert_eq!(second["textures"][0]["extensions"]["EXT_texture_webp"]["source"], json!(0));
    assert_eq!(second, j);
    // A named image of no known type is written without one.
    s.images[0].mime_type = None;
    assert!(json_of(&write_glb(&s, &Limits::default()).unwrap())["images"][0].get("mimeType").is_none());
}

#[test]
fn texture_slots_use_the_first_of_the_files_textures_that_match() {
    let mut s = rig_with_extensions();
    // The file holds the same texture three times; a material slot that matches uses the first.
    s.texture_variants = vec![TextureRef { image: 0, ..TextureRef::default() }, TextureRef { image: 1, ..TextureRef::default() }];
    s.texture_variants.extend([TextureRef { image: 1, ..TextureRef::default() }; 2]);
    let j = json_of(&write_glb(&s, &Limits::default()).unwrap());
    assert_eq!(j["textures"].as_array().unwrap().len(), 4);
    assert_eq!(j["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"], json!(1));
    // A slot that matches none of them adds one texture, which the next such slot uses again.
    s.materials[0].normal_texture = Some(TextureRef { image: 1, wrap: [Wrap::Mirror; 2], ..TextureRef::default() });
    s.materials[0].emissive_texture = s.materials[0].normal_texture;
    let j = json_of(&write_glb(&s, &Limits::default()).unwrap());
    assert_eq!(j["textures"].as_array().unwrap().len(), 5);
    assert_eq!((j["materials"][0]["normalTexture"]["index"].clone(), j["materials"][0]["emissiveTexture"]["index"].clone()), (json!(4), json!(4)));
}

#[test]
fn parts_with_and_without_normals_are_written_over_their_own_vertices() {
    // A triangle without normals next to the quad, which has normals and a target that moves them.
    let mut s = rig();
    let corners = (0..3).map(|i| Corner { position: i, normal: crate::NONE, uv: crate::NONE }).collect();
    s.meshes[0].parts.push(Part { material: Some(0), face_sizes: vec![3], corners, ..Part::default() });
    let glb = write_glb(&s, &Limits::default()).unwrap();
    let j = json_of(&glb);
    let primitives = j["meshes"][0]["primitives"].as_array().unwrap();
    assert_eq!(primitives.len(), 2);
    assert!(primitives[0]["attributes"].get("NORMAL").is_some() && primitives[0]["targets"][0].get("NORMAL").is_some());
    assert!(primitives[1]["attributes"].get("NORMAL").is_none() && primitives[1]["targets"][0].get("NORMAL").is_none());
    let back = parse(&glb).unwrap();
    back.validate().unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    let read = &back.meshes[0];
    assert!(read.parts[0].corners.iter().all(|c| c.normal != crate::NONE));
    assert!(read.parts[1].corners.iter().all(|c| c.normal == crate::NONE));
    // The quad moves as it did without the triangle.
    let quad = |m: &Mesh| Mesh { parts: vec![m.parts[0].clone()], ..m.clone() };
    assert_eq!(skinning(&quad(read)), skinning(&quad(&s.meshes[0])));
    for weights in [&[1.0, 0.0, 0.0][..], &[0.5, 0.75, 1.0]] {
        assert_eq!(shaped(&quad(read), weights), shaped(&quad(&s.meshes[0]), weights), "weights {weights:?}");
    }
}

#[test]
fn writing_a_written_file_again_gives_the_same_bytes() {
    let first = write_glb(&rig_with_extensions(), &Limits::default()).unwrap();
    let second = write_glb(&parse(&first).unwrap(), &Limits::default()).unwrap();
    assert_eq!(json_of(&second), json_of(&first));
    assert_eq!(second, first);
}

#[test]
fn writer_refuses_rigging_that_is_not_gltf() {
    let limits = Limits::default();
    let refused = |s: &Scene| matches!(write_glb(s, &limits), Err(Error::Malformed(_)));
    let mut s = rig();
    s.meshes[0].morph_targets[1].positions[3][1] = f64::NAN;
    assert!(refused(&s), "an offset that is not finite");
    let mut s = rig();
    s.meshes[0].morph_targets[0].normals[0][0] = f64::INFINITY;
    assert!(refused(&s), "a normal offset that is not finite");
    let mut s = rig();
    s.meshes[0].morph_weights[0] = f32::NAN;
    assert!(refused(&s), "a mesh weight that is not finite");
    let mut s = rig();
    s.nodes[0].morph_weights[0] = f32::INFINITY;
    assert!(refused(&s), "a node weight that is not finite");
    let mut s = rig();
    s.nodes[0].morph_weights = vec![0.5; 4];
    assert!(refused(&s), "more node weights than targets");
    let mut s = rig();
    s.nodes[1].morph_weights = vec![1.0];
    assert!(refused(&s), "node weights for a mesh without targets");
    let mut s = rig();
    s.meshes[0].weights[2][0] = f32::NAN;
    assert!(refused(&s), "a skin weight that is not finite");
    let mut s = rig();
    s.skins[0].inverse_bind[1][13] = f64::NAN;
    assert!(refused(&s), "an inverse bind matrix that is not finite");
    let mut s = rig();
    s.skins[0].inverse_bind.pop();
    assert!(refused(&s), "fewer inverse bind matrices than joints");
    let mut s = rig();
    s.skins[0].joints.clear();
    s.skins[0].inverse_bind.clear();
    assert!(refused(&s), "a skin without joints");
    let mut s = rig();
    s.meshes[0].weights.clear();
    assert!(refused(&s), "joints without weights");
    let mut s = rig();
    s.meshes[0].joints.clear();
    assert!(refused(&s), "weights without joints");
    let mut s = rig();
    s.meshes[0].weights[0] = [-0.25, 1.25, 0.0, 0.0];
    assert!(refused(&s), "a negative skinning weight");
    let mut s = rig();
    s.meshes[0].joints[0] = [0; 4];
    assert!(refused(&s), "a joint named twice with weights other than 0");
    let mut s = rig();
    s.meshes[0].joints[1] = [0, 1, 0, 1];
    s.meshes[0].weights[1] = [0.5, 0.25, 0.0, 0.25];
    assert!(refused(&s), "a joint named twice in slots that are not next to each other");
    let mut s = rig();
    s.skins[0].joints = vec![2, 2];
    assert!(refused(&s), "a joint node named twice in a skin");
    let mut s = rig();
    s.skins[0].inverse_bind[0][3] = 1.0;
    assert!(refused(&s), "an inverse bind matrix whose last row is not 0, 0, 0, 1");
    let mut s = rig();
    s.meshes[0].joints.clear();
    s.meshes[0].weights.clear();
    assert!(refused(&s), "a skin on a written mesh without joints and weights");
    // `rig` names joint 0 again in the slots whose weight is 0, which glTF allows.
    assert!(rig().meshes[0].joints[0][2..] == [0, 0] && write_glb(&rig(), &limits).is_ok());
    let mut s = rig();
    s.meshes[0].joints[3][1] = 2;
    assert!(refused(&s), "a joint the node's skin does not have");
    // A position no triangle uses is not written, so its joints are not checked.
    let mut u = rig();
    let face = &mut u.meshes[0];
    face.positions.push([5.0; 3]);
    face.joints.push([7, 0, 0, 0]);
    face.weights.push([1.0, 0.0, 0.0, 0.0]);
    face.morph_targets[0].positions.push([0.0; 3]);
    face.morph_targets[1].positions.push([0.0; 3]);
    assert!(write_glb(&u, &limits).is_ok());
    // The same mesh under a node without a skin is not checked against one.
    s.nodes[0].skin = None;
    assert!(write_glb(&s, &limits).is_ok());
    // A skin without inverse bind matrices has identity ones, and none are written.
    let mut s = rig();
    s.skins[0].inverse_bind.clear();
    let glb = write_glb(&s, &limits).unwrap();
    assert!(json_of(&glb)["skins"][0].get("inverseBindMatrices").is_none());
    assert_eq!(parse(&glb).unwrap().skins, s.skins);
    // The skinning and morph target data count against the data limit like everything else in
    // the binary chunk (which is padded to 4 bytes).
    let glb = write_glb(&rig(), &limits).unwrap();
    let bin = split_glb(&glb).unwrap().1.unwrap().len();
    assert!(write_glb(&rig(), &Limits { max_data_bytes: bin, ..Limits::default() }).is_ok());
    assert!(matches!(write_glb(&rig(), &Limits { max_data_bytes: bin - 4, ..Limits::default() }), Err(Error::TooLarge(_))));
}

#[test]
fn the_data_limit_stops_the_writer_before_the_binary_chunk_passes_it() {
    // 1,024 vertices and 64 targets that move each of them: 12 KiB of positions and 768 KiB of
    // target offsets.
    let mut mesh = rig().meshes.swap_remove(0);
    let n = 1024u32;
    mesh.positions = (0..n).map(|i| [f64::from(i), 0.0, 0.0]).collect();
    mesh.normals.clear();
    mesh.joints.clear();
    mesh.weights.clear();
    mesh.parts[0].face_sizes = vec![3; n as usize / 3];
    mesh.parts[0].corners = (0..n / 3 * 3).map(|i| Corner { position: i, normal: crate::NONE, uv: crate::NONE }).collect();
    mesh.morph_targets = (0..64).map(|_| MorphTarget { positions: vec![[0.0, 0.0, 1.0]; n as usize], ..MorphTarget::default() }).collect();
    mesh.morph_weights.clear();
    let limits = Limits { max_data_bytes: 64 * 1024, ..Limits::default() };
    let mut w = Writer::new(&limits);
    assert!(matches!(w.mesh(&mesh), Err(Error::TooLarge(_))));
    assert!(w.bin.len() <= limits.max_data_bytes, "{} bytes written past a limit of {}", w.bin.len(), limits.max_data_bytes);
    // The limit holds for sparse accessors too: each target now moves one vertex in four, 4 KiB
    // of sparse indices and values a target.
    for t in &mut mesh.morph_targets {
        t.positions.iter_mut().enumerate().filter(|(i, _)| i % 4 != 0).for_each(|(_, p)| *p = [0.0; 3]);
    }
    let mut w = Writer::new(&limits);
    assert!(matches!(w.mesh(&mesh), Err(Error::TooLarge(_))));
    assert!(w.bin.len() <= limits.max_data_bytes, "{} bytes written past a limit of {}", w.bin.len(), limits.max_data_bytes);
    assert!(w.accessors.iter().any(|a| a.get("sparse").is_some()));
    // A mesh with more morph targets than the reader reads is refused before any target is
    // written: only the POSITION accessor is.
    let limits = Limits { max_morph_targets: 63, ..Limits::default() };
    let mut w = Writer::new(&limits);
    assert!(matches!(w.mesh(&mesh), Err(Error::TooLarge(_))));
    assert_eq!(w.accessors.len(), 1);
    let mut scene = rig();
    scene.meshes[0] = mesh;
    scene.nodes[0].morph_weights.clear();
    scene.nodes[0].skin = None;
    scene.nodes[1].skin = None;
    assert!(write_glb(&scene, &Limits { max_morph_targets: 64, ..Limits::default() }).is_ok());
    assert!(matches!(write_glb(&scene, &limits), Err(Error::TooLarge(_))));
}
