use serde_json::{Value, json};

use super::hammer;
use crate::gltf::{MAX_UV_SETS, MAX_ZERO_COMPONENTS, base64_decode, glb_container, read, split_glb, uri_decode, write_glb};
use crate::{
    AlphaMode, ChannelPath, Corner, Error, IDENTITY, Image, Interpolation, LightKind, Limits, Material, Mesh, NONE, Node, Part, Projection, Scene,
    TextureRef, Wrap, face_normal, no_external, transform_point,
};

/// Builds the binary chunk with its buffer views and accessors.
#[derive(Default)]
struct B {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl B {
    fn view(&mut self, bytes: &[u8]) -> usize {
        self.bin.resize(self.bin.len().next_multiple_of(4), 0);
        self.views.push(json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() }));
        self.bin.extend_from_slice(bytes);
        self.views.len() - 1
    }

    fn acc(&mut self, bytes: &[u8], component: u32, count: usize, kind: &str) -> usize {
        let view = self.view(bytes);
        self.accessors.push(json!({ "bufferView": view, "componentType": component, "count": count, "type": kind }));
        self.accessors.len() - 1
    }

    fn f32s(&mut self, v: &[f32], kind: &str, per: usize) -> usize {
        self.acc(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>(), 5126, v.len() / per, kind)
    }

    fn u16s(&mut self, v: &[u16]) -> usize {
        self.acc(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>(), 5123, v.len(), "SCALAR")
    }

    /// The document: `rest` with the accessors, views and buffer added.
    fn json(&self, mut rest: Value) -> Value {
        let o = rest.as_object_mut().unwrap();
        o.entry("asset").or_insert(json!({ "version": "2.0" }));
        o.insert("accessors".into(), json!(self.accessors));
        o.insert("bufferViews".into(), json!(self.views));
        o.entry("buffers").or_insert(json!([{ "byteLength": self.bin.len() }]));
        rest
    }

    fn glb(&self, rest: Value) -> Vec<u8> {
        glb_container(&serde_json::to_vec(&self.json(rest)).unwrap(), &self.bin).unwrap()
    }
}

const QUAD: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];

fn parse(bytes: &[u8]) -> crate::Result<Scene> {
    read(bytes, &Limits::default(), &mut no_external)
}

/// A quad with normals and UVs under a node tree: a root moved by TRS, its child placed by a
/// matrix, with a material.
fn quad() -> Vec<u8> {
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    let n = b.f32s(&[0.0, 0.0, 1.0].repeat(4), "VEC3", 3);
    let t = b.f32s(&[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0], "VEC2", 2);
    let i = b.u16s(&[0, 1, 2, 0, 2, 3]);
    let mut m = IDENTITY;
    m[13] = 5.0;
    let h = std::f64::consts::FRAC_1_SQRT_2;
    b.glb(json!({
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [
            { "name": "root", "translation": [1.0, 0.0, 0.0], "rotation": [0.0, 0.0, h, h], "scale": [2.0, 2.0, 2.0], "children": [1] },
            { "name": "leaf", "matrix": m, "mesh": 0 },
            { "name": "not in the scene", "mesh": 0 }
        ],
        "meshes": [{ "name": "Quad", "primitives": [{ "attributes": { "POSITION": p, "NORMAL": n, "TEXCOORD_0": t }, "indices": i, "material": 0 }] }],
        "materials": [{ "name": "Paint", "pbrMetallicRoughness": { "baseColorFactor": [1.0, 0.5, 0.25, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.4 }, "doubleSided": true, "alphaMode": "MASK" }]
    }))
}

#[test]
fn glb_quad_with_a_node_tree_and_a_material() {
    let s = parse(&quad()).unwrap();
    s.validate().unwrap();
    assert!(s.warnings.is_empty() && s.external.is_empty());
    let m = &s.meshes[0];
    assert_eq!((m.name.as_str(), m.positions.len(), m.normals.len(), m.uv_sets[0].len(), m.triangle_count()), ("Quad", 4, 4, 4, 2));
    assert_eq!(m.parts[0].corners[4], Corner::all(2));
    assert_eq!(m.parts[0].material, Some(0));
    // UVs are glTF's own convention: kept as stored.
    assert_eq!(m.uv_sets[0][2], [1.0, 1.0]);
    let paint = &s.materials[0];
    assert_eq!((paint.name.as_str(), paint.base_color, paint.metallic, paint.roughness), ("Paint", [1.0, 0.5, 0.25, 1.0], 0.0, 0.4));
    assert_eq!((paint.double_sided, paint.alpha_mode), (true, AlphaMode::Mask(0.5)));
    // The default scene's roots; the tree places the mesh: up 5, scaled by 2, turned a quarter
    // about Z, moved 1 along X.
    assert_eq!(s.roots, [0]);
    assert!(s.nodes[0].trs.is_some() && s.nodes[1].trs.is_none());
    let placed = s.instances(&Limits::default()).unwrap();
    assert_eq!(placed.len(), 1);
    let p = transform_point(&placed[0].1, [1.0, 0.0, 0.0]);
    assert!((p[0] - -9.0).abs() < 1e-9 && (p[1] - 2.0).abs() < 1e-9 && p[2].abs() < 1e-9, "{p:?}");
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = c.iter().enumerate().fold(0u32, |n, (k, b)| n | u32::from(*b) << (16 - 8 * k));
        for k in 0..4 {
            out.push(if k <= c.len() { T[(n >> (18 - 6 * k)) as usize & 63] as char } else { '=' });
        }
    }
    out
}

/// One triangle as a `.gltf` document whose buffer and image are named by `buffer` and `image`.
fn gltf_text(buffer: &str, image: &str) -> (Vec<u8>, Vec<u8>) {
    let mut b = B::default();
    let p = b.f32s(&QUAD[..9], "VEC3", 3);
    let doc = b.json(json!({
        "buffers": [{ "byteLength": b.bin.len(), "uri": buffer }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": p }, "material": 0 }] }],
        "materials": [{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0, "texCoord": 1 } } }],
        "textures": [{ "source": 0, "sampler": 0 }],
        "samplers": [{ "wrapS": 33071, "wrapT": 33648 }],
        "images": [{ "uri": image }]
    }));
    (serde_json::to_vec(&doc).unwrap(), b.bin)
}

#[test]
fn embedded_and_external_resources() {
    // Embedded: data URIs for the buffer and the image.
    let bin = gltf_text("", "").1;
    let png = b"\x89PNG not really";
    let (doc, _) = gltf_text(&format!("data:application/octet-stream;base64,{}", base64(&bin)), &format!("data:image/png;base64,{}", base64(png)));
    let mut asked = 0;
    let s = read(&doc, &Limits::default(), &mut |_| {
        asked += 1;
        None
    })
    .unwrap();
    assert_eq!((asked, s.external.len(), s.meshes[0].triangle_count()), (0, 0, 1));
    assert_eq!(
        (s.images[0].data.as_deref(), s.images[0].mime_type.as_deref(), s.images[0].uri.as_deref()),
        (Some(&png[..]), Some("image/png"), None)
    );
    // The texture slot keeps its UV set and wrap modes.
    assert_eq!(s.materials[0].base_color_texture, Some(TextureRef { image: 0, uv_set: 1, wrap: [Wrap::Clamp, Wrap::Mirror] }));
    // No scene in the file: every node that is nobody's child is a root.
    assert_eq!(s.roots, [0]);

    // External: the buffer is asked for by its decoded name; the image is only named.
    let (doc, bin) = gltf_text("my%20data.bin", "../textures/wood.png");
    let mut asked = Vec::new();
    let s = read(&doc, &Limits::default(), &mut |name| {
        asked.push(name.to_string());
        Some(bin.clone())
    })
    .unwrap();
    assert_eq!(asked, ["my data.bin"]);
    assert_eq!(s.external, ["my data.bin", "../textures/wood.png"]);
    assert_eq!((s.images[0].uri.as_deref(), s.images[0].data.as_ref()), (Some("../textures/wood.png"), None));
    s.validate().unwrap();
    // A buffer that is not supplied is named in the error.
    assert_eq!(parse(&doc), Err(Error::Missing("my data.bin".into())));
    // An image that cannot be decoded is left without bytes.
    let (doc, _) = gltf_text(&format!("data:;base64,{}", base64(&bin)), "data:image/png;base64,!!!");
    assert_eq!(parse(&doc).unwrap().images[0].data, None);
    assert!(parse(&gltf_text("data:;base64,!!!", "x.png").0).is_err());
}

#[test]
fn base64_and_uri_decoding() {
    assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
    assert_eq!(base64_decode("aGVs\nbG8").unwrap(), b"hello");
    assert_eq!(base64_decode("-_8").unwrap(), [0xfb, 0xff]);
    assert!(base64_decode("a$b").is_none());
    assert_eq!(uri_decode("a%20b%2Fc"), "a b/c");
    assert_eq!(uri_decode("a%+1b"), "a%+1b");
    assert_eq!(uri_decode("a%4"), "a%4");
    assert_eq!(uri_decode("%"), "%");
}

#[test]
fn glb_container_checks() {
    let glb = quad();
    let (json, bin) = split_glb(&glb).unwrap();
    assert!(json.starts_with(b"{") && bin.unwrap().len() >= 12 * 4);
    assert!(split_glb(b"glTF").is_err());
    assert!(split_glb(b"nope").is_err());
    let mut v1 = glb.clone();
    v1[4] = 1;
    assert!(matches!(split_glb(&v1), Err(Error::Unsupported(_))));
    // The header's length is checked against the file.
    assert!(split_glb(&glb[..glb.len() - 1]).is_err());
    // A chunk that runs past the end.
    let mut long = glb.clone();
    long[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(split_glb(&long).is_err());
    // A binary chunk before the JSON one.
    let mut swapped = glb.clone();
    swapped[16..20].copy_from_slice(b"BIN\0");
    assert!(split_glb(&swapped).is_err());
    // An unknown chunk is skipped.
    let mut extra = glb.clone();
    extra.extend(4u32.to_le_bytes());
    extra.extend(b"XTRAdata");
    let len = extra.len() as u32;
    extra[8..12].copy_from_slice(&len.to_le_bytes());
    assert_eq!(parse(&extra).unwrap().meshes, parse(&glb).unwrap().meshes);
}

fn one_mesh(b: &B, primitive: Value) -> Vec<u8> {
    b.glb(json!({ "nodes": [{ "mesh": 0 }], "meshes": [{ "primitives": [primitive] }] }))
}

fn positions_of(s: &Scene) -> Vec<[u32; 3]> {
    s.meshes[0].parts.iter().flat_map(Part::triangles).map(|t| t.map(|c| c.position)).collect()
}

#[test]
fn strips_fans_and_unindexed_primitives() {
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    let i = b.u16s(&[0, 1, 3, 2]);
    let s = parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p }, "indices": i, "mode": 5 }))).unwrap();
    // Every second triangle of a strip is turned so that all face the same way.
    assert_eq!(positions_of(&s), [[0, 1, 3], [3, 1, 2]]);
    let s = parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p }, "mode": 6 }))).unwrap();
    assert_eq!(positions_of(&s), [[0, 1, 2], [0, 2, 3]]);
    // Without indices the vertices are taken in order; a vertex left over is dropped.
    let s = parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p } }))).unwrap();
    assert_eq!((positions_of(&s), s.warnings.len()), (vec![[0, 1, 2]], 1));
    // Points and lines are left out, with a warning; the mesh stays so indices keep their meaning.
    let s = parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p }, "mode": 1 }))).unwrap();
    assert_eq!((s.meshes.len(), s.meshes[0].parts.len(), s.warnings.len()), (1, 0, 1));
}

#[test]
fn strides_quantized_attributes_and_accessors_without_a_buffer() {
    let mut b = B::default();
    // Positions and normals interleaved: 24 bytes per vertex.
    let interleaved: Vec<u8> =
        QUAD.chunks(3).flat_map(|p| p.iter().chain(&[0.0, 0.0, 1.0]).flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>()).collect();
    let view = b.view(&interleaved);
    b.views[view]["byteStride"] = json!(24);
    b.accessors.push(json!({ "bufferView": view, "componentType": 5126, "count": 4, "type": "VEC3" }));
    b.accessors.push(json!({ "bufferView": view, "byteOffset": 12, "componentType": 5126, "count": 4, "type": "VEC3" }));
    // Normalised bytes for UVs and colours, as KHR_mesh_quantization allows.
    let uv = b.acc(&[0, 0, 255, 0, 255, 255, 0, 255], 5121, 4, "VEC2");
    b.accessors[uv]["normalized"] = json!(true);
    let color = b.acc(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255], 5121, 4, "VEC3");
    b.accessors[color]["normalized"] = json!(true);
    // A second UV set with no buffer view: all zeros.
    b.accessors.push(json!({ "componentType": 5126, "count": 4, "type": "VEC2" }));
    let zero = b.accessors.len() - 1;
    let i = b.acc(&[0, 1, 2, 0, 2, 3], 5121, 6, "SCALAR");
    let glb =
        one_mesh(&b, json!({ "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": uv, "TEXCOORD_1": zero, "COLOR_0": color }, "indices": i }));
    let s = parse(&glb).unwrap();
    s.validate().unwrap();
    let m = &s.meshes[0];
    assert_eq!(m.positions[2], [1.0, 1.0, 0.0]);
    assert_eq!(m.normals, vec![[0.0, 0.0, 1.0]; 4]);
    assert_eq!(m.uv_sets, [vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], vec![[0.0; 2]; 4]]);
    // A colour without alpha is opaque.
    assert_eq!(m.colors[1], [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(m.triangle_count(), 2);
    // A stride smaller than the element is refused.
    b.views[view]["byteStride"] = json!(8);
    assert!(parse(&one_mesh(&b, json!({ "attributes": { "POSITION": 0 } }))).is_err());
    // Accessors without a buffer share one budget of zeros, however often they are named.
    let mut b = B::default();
    let keys = b.f32s(&[0.0; 3], "VEC3", 3);
    b.accessors.push(json!({ "componentType": 5126, "count": MAX_ZERO_COMPONENTS, "type": "SCALAR" }));
    let zeros = b.accessors.len() - 1;
    let animated = |output: usize| {
        b.glb(json!({ "nodes": [{}], "animations": [{ "samplers": [{ "input": zeros, "output": output }], "channels": [{ "sampler": 0, "target": { "node": 0, "path": "translation" } }] }] }))
    };
    assert_eq!(parse(&animated(keys)).unwrap().animations[0].channels[0].times.len(), MAX_ZERO_COMPONENTS);
    assert!(matches!(parse(&animated(zeros)), Err(Error::TooLarge(_))));
}

#[test]
fn primitives_of_a_mesh_share_its_pools() {
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    let n = b.f32s(&[0.0, 0.0, 1.0].repeat(4), "VEC3", 3);
    let c = b.f32s(&[1.0, 0.0, 0.0, 0.5].repeat(4), "VEC4", 4);
    let t = b.f32s(&[0.25; 8], "VEC2", 2);
    let i = b.u16s(&[0, 1, 2]);
    let glb = b.glb(json!({ "meshes": [{ "primitives": [
        { "attributes": { "POSITION": p }, "indices": i },
        { "attributes": { "POSITION": p, "NORMAL": n, "COLOR_0": c, "TEXCOORD_0": t }, "indices": i },
        { "attributes": { "POSITION": p }, "indices": i }
    ] }] }));
    let s = parse(&glb).unwrap();
    s.validate().unwrap();
    let m = &s.meshes[0];
    // The first and third name the same accessors and share their vertices.
    assert_eq!((m.positions.len(), m.normals.len(), m.uv_sets[0].len(), m.colors.len(), m.parts.len()), (8, 4, 4, 8, 3));
    // The second primitive's corners count from where its vertices start in each pool.
    assert_eq!(m.parts[1].corners[1], Corner { position: 5, normal: 1, uv: 1 });
    assert_eq!(m.parts[2].corners[1], Corner { position: 1, normal: NONE, uv: NONE });
    // Vertices of primitives without colours are white.
    assert_eq!((m.colors[0], m.colors[5]), ([1.0; 4], [1.0, 0.0, 0.0, 0.5]));
    // An attribute that does not fit the positions is passed over.
    let short = b.f32s(&[0.0, 0.0, 1.0], "VEC3", 3);
    let s = parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p, "NORMAL": short } }))).unwrap();
    assert!(s.meshes[0].normals.is_empty());
}

#[test]
fn uv_sets_past_the_bound_are_not_read() {
    let mut b = B::default();
    let p = b.f32s(&QUAD[..9], "VEC3", 3);
    let t = b.f32s(&[0.5; 6], "VEC2", 2);
    let mut attrs = json!({ "POSITION": p });
    for k in 0..MAX_UV_SETS + 2 {
        attrs[format!("TEXCOORD_{k}")] = json!(t);
    }
    let s = parse(&one_mesh(&b, json!({ "attributes": attrs }))).unwrap();
    assert_eq!(s.meshes[0].uv_sets.len(), MAX_UV_SETS);
    s.validate().unwrap();
}

#[test]
fn bad_triangles_are_left_out_and_later_ones_keep_their_vertices() {
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    // Triangles: (0,1,2), (0,1,9) out of range, (0,2,3), and two indices left over.
    let i = b.u16s(&[0, 1, 2, 0, 1, 9, 0, 2, 3, 1, 2]);
    let s = parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p }, "indices": i }))).unwrap();
    assert_eq!(positions_of(&s), [[0, 1, 2], [0, 2, 3]]);
    assert_eq!(s.warnings, ["2 triangle(s) could not be read and were left out"]);
    // A position that is not finite takes its triangles with it.
    let mut nan = QUAD;
    nan[3] = f32::NAN;
    let p = b.f32s(&nan, "VEC3", 3);
    let i = b.u16s(&[0, 1, 2, 0, 2, 3]);
    let s = parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p }, "indices": i }))).unwrap();
    assert_eq!(positions_of(&s), [[0, 2, 3]]);
    s.validate().unwrap();
}

#[test]
fn counts_are_checked_against_the_buffer_before_anything_is_allocated() {
    // A one-triangle buffer whose accessors claim far more.
    for (kind, count) in [("VEC3", 1u64 << 40), ("MAT4", u64::MAX / 16), ("VEC3", u64::MAX), ("VEC3", 4)] {
        let mut b = B::default();
        b.f32s(&QUAD[..9], "VEC3", 3);
        b.accessors[0]["count"] = json!(count);
        b.accessors[0]["type"] = json!(kind);
        let limits = Limits { max_vertices: usize::MAX, max_triangles: usize::MAX, ..Limits::default() };
        assert!(read(&one_mesh(&b, json!({ "attributes": { "POSITION": 0 } })), &limits, &mut no_external).is_err(), "{kind} {count}");
    }
    // Integer accessors too.
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    let i = b.u16s(&[0, 1, 2]);
    b.accessors[i]["count"] = json!(1u64 << 33);
    let limits = Limits { max_triangles: usize::MAX, ..Limits::default() };
    assert!(read(&one_mesh(&b, json!({ "attributes": { "POSITION": p }, "indices": i })), &limits, &mut no_external).is_err());
    // A view that runs past its buffer, an accessor offset past its view, things that are not there.
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    b.views[0]["byteLength"] = json!(1u64 << 40);
    assert!(parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p } }))).is_err());
    b.views[0]["byteLength"] = json!(48);
    b.accessors[0]["byteOffset"] = json!(40);
    assert!(parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p } }))).is_err());
    assert!(parse(&one_mesh(&b, json!({ "attributes": { "POSITION": 7 } }))).is_err());
    b.accessors[0]["bufferView"] = json!(3);
    assert!(parse(&one_mesh(&b, json!({ "attributes": { "POSITION": p } }))).is_err());
}

#[test]
fn what_is_not_read_is_refused_or_named() {
    let mut b = B::default();
    let p = b.f32s(&QUAD[..9], "VEC3", 3);
    let doc = |extra: Value| {
        let mut v = b.json(json!({ "meshes": [{ "primitives": [{ "attributes": { "POSITION": p } }] }] }));
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        glb_container(&serde_json::to_vec(&v).unwrap(), &b.bin).unwrap()
    };
    assert!(parse(&doc(json!({}))).is_ok());
    assert!(matches!(parse(&doc(json!({ "asset": { "version": "1.0" } }))), Err(Error::Unsupported(_))));
    assert!(matches!(parse(&doc(json!({ "asset": {} }))), Err(Error::Unsupported(_))));
    assert!(matches!(parse(&doc(json!({ "extensionsRequired": ["KHR_draco_mesh_compression"] }))), Err(Error::Unsupported(_))));
    assert!(matches!(parse(&doc(json!({ "extensionsRequired": ["VENDOR_something_new"] }))), Err(Error::Unsupported(_))));
    // Extensions that only change looks are named in a warning; the ones the reader handles are not.
    let s = parse(&doc(
        json!({ "extensionsRequired": ["KHR_texture_transform", "KHR_materials_clearcoat", "KHR_mesh_quantization", "KHR_materials_unlit"] }),
    ))
    .unwrap();
    assert_eq!(s.warnings.len(), 2);
    assert!(parse(b"{ not json").is_err());
    assert!(parse(b"").is_err());
}

/// A skinned, animated triangle with a camera and lights.
fn rigged() -> Vec<u8> {
    let mut b = B::default();
    let p = b.f32s(&QUAD[..9], "VEC3", 3);
    let joints = b.acc(&[0, 1, 0, 0].repeat(3), 5121, 3, "VEC4");
    let weights = b.f32s(&[0.75, 0.25, 0.0, 0.0].repeat(3), "VEC4", 4);
    let mut bind = IDENTITY.map(|x| x as f32).to_vec();
    bind.extend(IDENTITY.map(|x| x as f32));
    bind[12] = -1.0;
    let ibm = b.f32s(&bind, "MAT4", 16);
    let times = b.f32s(&[0.0, 1.0], "SCALAR", 1);
    let moves = b.f32s(&[0.0, 0.0, 0.0, 0.0, 2.0, 0.0], "VEC3", 3);
    b.glb(json!({
        "extensions": { "KHR_lights_punctual": { "lights": [
            { "type": "spot", "color": [1.0, 0.5, 0.0], "intensity": 40.0, "range": 9.0, "spot": { "innerConeAngle": 0.1, "outerConeAngle": 0.5 } },
            { "name": "Sun", "type": "directional" }
        ] } },
        "scenes": [{ "nodes": [0, 3] }],
        "nodes": [
            { "name": "body", "mesh": 0, "skin": 0, "children": [1] },
            { "name": "hip", "children": [2] },
            { "name": "knee" },
            { "name": "eye", "camera": 0, "extensions": { "KHR_lights_punctual": { "light": 1 } } }
        ],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": p, "JOINTS_0": joints, "WEIGHTS_0": weights } }] }],
        "skins": [{ "name": "rig", "joints": [1, 2, 99], "inverseBindMatrices": ibm }],
        "animations": [{ "name": "hop", "samplers": [{ "input": times, "output": moves, "interpolation": "STEP" }],
            "channels": [{ "sampler": 0, "target": { "node": 1, "path": "translation" } }, { "sampler": 0, "target": { "node": 1, "path": "weights" } }, { "sampler": 0, "target": { "node": 77, "path": "scale" } }] }],
        "cameras": [{ "name": "Main", "type": "perspective", "perspective": { "yfov": 0.6, "aspectRatio": 1.5, "znear": 0.1 } }, { "type": "orthographic", "orthographic": { "xmag": 2.0, "ymag": 1.0, "znear": 0.1, "zfar": 50.0 } }]
    }))
}

#[test]
fn skins_animations_cameras_and_lights() {
    let s = parse(&rigged()).unwrap();
    s.validate().unwrap();
    let m = &s.meshes[0];
    assert_eq!((m.joints[0], m.weights[0]), ([0, 1, 0, 0], [0.75, 0.25, 0.0, 0.0]));
    // A joint that names no node is dropped.
    let skin = &s.skins[0];
    assert_eq!((skin.name.as_str(), skin.joints.as_slice(), skin.inverse_bind.len()), ("rig", &[1, 2][..], 2));
    assert_eq!(skin.inverse_bind[0][12], -1.0);
    assert_eq!(s.nodes[0].skin, Some(0));
    // Only the channels this model holds, on nodes that exist.
    let hop = &s.animations[0];
    assert_eq!((hop.name.as_str(), hop.channels.len()), ("hop", 1));
    let c = &hop.channels[0];
    assert_eq!((c.node, c.path, c.interpolation), (1, ChannelPath::Translation, Interpolation::Step));
    assert_eq!((c.times.as_slice(), c.values.as_slice()), (&[0.0, 1.0][..], &[0.0, 0.0, 0.0, 0.0, 2.0, 0.0][..]));
    assert_eq!(s.cameras[0].projection, Projection::Perspective { yfov: 0.6, aspect: Some(1.5), znear: 0.1, zfar: None });
    assert_eq!(s.cameras[1].projection, Projection::Orthographic { xmag: 2.0, ymag: 1.0, znear: 0.1, zfar: 50.0 });
    assert_eq!((s.nodes[3].camera, s.nodes[3].light), (Some(0), Some(1)));
    assert_eq!((s.lights[0].kind, s.lights[0].color, s.lights[0].range), (LightKind::Spot { inner: 0.1, outer: 0.5 }, [1.0, 0.5, 0.0], Some(9.0)));
    assert_eq!((s.lights[1].name.as_str(), s.lights[1].kind, s.lights[1].intensity), ("Sun", LightKind::Directional, 1.0));
}

#[test]
fn materials_and_embedded_images() {
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    let image = b.view(b"\xff\xd8 jpeg bytes");
    let glb = b.glb(json!({
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": p }, "material": 5 }] }],
        "materials": [{
            "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 }, "metallicRoughnessTexture": { "index": 9 } },
            "normalTexture": { "index": 0, "scale": 0.5 }, "occlusionTexture": { "index": 0, "strength": 0.25 }, "emissiveTexture": { "index": 1 },
            "emissiveFactor": [1.0, 0.5, 0.0], "alphaMode": "BLEND",
            "extensions": { "KHR_materials_emissive_strength": { "emissiveStrength": 4.0 }, "KHR_materials_unlit": {} }
        }, {}],
        "textures": [{ "source": 0 }, { "source": 3 }],
        "images": [{ "name": "wood", "bufferView": image, "mimeType": "image/jpeg" }]
    }));
    let s = parse(&glb).unwrap();
    s.validate().unwrap();
    let m = &s.materials[0];
    assert_eq!(
        (s.images[0].name.as_str(), s.images[0].data.as_deref(), s.images[0].mime_type.as_deref()),
        ("wood", Some(&b"\xff\xd8 jpeg bytes"[..]), Some("image/jpeg"))
    );
    assert_eq!(m.base_color_texture, Some(TextureRef::default()));
    // Slots that name a texture or an image that is not there stay empty.
    assert_eq!((m.metallic_roughness_texture, m.emissive_texture), (None, None));
    assert_eq!((m.normal_scale, m.occlusion_strength, m.emissive, m.alpha_mode, m.unlit), (0.5, 0.25, [4.0, 2.0, 0.0], AlphaMode::Blend, true));
    // glTF's defaults; a material index that does not exist leaves the part without one.
    assert_eq!(s.materials[1], Material::default());
    assert_eq!(s.meshes[0].parts[0].material, None);
}

fn too(r: crate::Result<Scene>) -> bool {
    matches!(r, Err(Error::TooLarge(_)))
}

fn with(glb: &[u8], f: &dyn Fn(&mut Limits)) -> crate::Result<Scene> {
    let mut l = Limits::default();
    f(&mut l);
    read(glb, &l, &mut no_external)
}

#[test]
fn limits() {
    let q = quad();
    assert!(with(&q, &|l| l.max_vertices = 4).is_ok());
    assert!(too(with(&q, &|l| l.max_vertices = 3)));
    assert!(with(&q, &|l| l.max_triangles = 2).is_ok());
    assert!(too(with(&q, &|l| l.max_triangles = 1)));
    assert!(with(&q, &|l| l.max_parts = 1).is_ok());
    assert!(too(with(&q, &|l| l.max_parts = 0)));
    assert!(with(&q, &|l| l.max_meshes = 1).is_ok());
    assert!(too(with(&q, &|l| l.max_meshes = 0)));
    assert!(with(&q, &|l| l.max_materials = 1).is_ok());
    assert!(too(with(&q, &|l| l.max_materials = 0)));
    assert!(with(&q, &|l| l.max_nodes = 3).is_ok());
    assert!(too(with(&q, &|l| l.max_nodes = 2)));
    assert!(with(&q, &|l| l.max_depth = 1).is_ok());
    assert!(too(with(&q, &|l| l.max_depth = 0)));
    let bin = split_glb(&q).unwrap().1.unwrap().len();
    assert!(with(&q, &|l| l.max_data_bytes = bin).is_ok());
    assert!(too(with(&q, &|l| l.max_data_bytes = bin - 1)));
}

#[test]
fn limits_count_shared_accessors_expansions_and_extras() {
    // One accessor named by the primitives of one mesh is one set of vertices; named by
    // several meshes, each keeps its own, so the count that matters is the sum.
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    let i = b.u16s(&[0, 1, 2]);
    let prims: Vec<Value> = (0..5).map(|_| json!({ "attributes": { "POSITION": p }, "indices": i })).collect();
    let one = b.glb(json!({ "meshes": [{ "primitives": prims }] }));
    assert!(with(&one, &|l| l.max_vertices = 4).is_ok());
    assert_eq!(parse(&one).unwrap().meshes[0].positions.len(), 4);
    let meshes: Vec<Value> = (0..5).map(|_| json!({ "primitives": [prims[0]] })).collect();
    let shared = b.glb(json!({ "meshes": meshes }));
    assert!(with(&shared, &|l| l.max_vertices = 20).is_ok());
    assert!(too(with(&shared, &|l| l.max_vertices = 19)));
    assert!(with(&shared, &|l| l.max_triangles = 5).is_ok());
    assert!(too(with(&shared, &|l| l.max_triangles = 4)));
    assert!(with(&one, &|l| l.max_parts = 5).is_ok());
    assert!(too(with(&one, &|l| l.max_parts = 4)));
    // Strips and unindexed primitives count the triangles they expand to.
    let strip = one_mesh(&b, json!({ "attributes": { "POSITION": p }, "mode": 5 }));
    assert!(with(&strip, &|l| l.max_triangles = 2).is_ok());
    assert!(too(with(&strip, &|l| l.max_triangles = 1)));

    // Skins, animations, cameras, lights and the values they read.
    let r = rigged();
    assert!(with(&r, &|l| l.max_extras = 3).is_ok());
    for (key, extra) in
        [("skins", json!({})), ("animations", json!({})), ("cameras", json!({ "type": "perspective" })), ("buffers", json!({ "byteLength": 0 }))]
    {
        let (json, bin) = split_glb(&r).unwrap();
        let mut doc: Value = serde_json::from_slice(json).unwrap();
        let list = doc[key].as_array_mut().unwrap();
        while list.len() < 4 {
            list.push(extra.clone());
        }
        let glb = glb_container(&serde_json::to_vec(&doc).unwrap(), bin.unwrap()).unwrap();
        assert!(with(&glb, &|l| l.max_extras = 4).is_ok(), "{key}");
        assert!(too(with(&glb, &|l| l.max_extras = 3)), "{key}");
    }
    let image_and_keys = with(&r, &|l| l.max_data_bytes = split_glb(&r).unwrap().1.unwrap().len());
    assert!(image_and_keys.is_ok());
}

#[test]
fn hierarchies_with_cycles_or_missing_nodes() {
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    let tree = |nodes: Value| {
        b.glb(json!({ "scenes": [{ "nodes": [0] }], "nodes": nodes, "meshes": [{ "primitives": [{ "attributes": { "POSITION": p } }] }] }))
    };
    // A node that is its own descendant is refused instead of walked for ever.
    assert!(matches!(parse(&tree(json!([{ "children": [1] }, { "children": [0], "mesh": 0 }]))), Err(Error::TooLarge(_))));
    assert!(matches!(parse(&tree(json!([{ "children": [0] }]))), Err(Error::TooLarge(_))));
    // Children, meshes and skins that do not exist are dropped.
    let s = parse(&tree(json!([{ "children": [1, 40], "mesh": 3, "skin": 2, "camera": 1 }, { "mesh": 0 }]))).unwrap();
    s.validate().unwrap();
    assert_eq!((s.nodes[0].children.as_slice(), s.nodes[0].mesh, s.nodes[0].skin, s.nodes[0].camera), (&[1][..], None, None, None));
    // A shared subtree expanded too often runs into the visit limit, not out of memory.
    let fan: Vec<Value> = (0..20).map(|i| if i < 19 { json!({ "children": [i + 1, i + 1] }) } else { json!({ "mesh": 0 }) }).collect();
    assert!(matches!(parse(&tree(json!(fan))), Err(Error::TooLarge(_))));
}

#[test]
fn hostile_files_never_panic() {
    hammer(&quad(), &parse);
    hammer(&rigged(), &parse);
    let (doc, _) = gltf_text(&format!("data:application/octet-stream;base64,{}", base64(&gltf_text("", "").1)), "wood.png");
    hammer(&doc, &parse);
}

fn painted() -> Scene {
    let mesh = Mesh {
        name: "Card".into(),
        positions: QUAD.chunks(3).map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]).collect(),
        normals: vec![[0.0, 0.0, 1.0]],
        uv_sets: vec![vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]], vec![[0.5, 0.5]; 4]],
        colors: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4]],
        parts: vec![
            // A quad: written as two triangles.
            Part {
                name: "front".into(),
                material: Some(0),
                face_sizes: vec![4],
                corners: (0..4).map(|i| Corner { position: i, normal: 0, uv: i }).collect(),
                ..Part::default()
            },
            Part { material: Some(1), corners: [0, 2, 1].map(|i| Corner { position: i, normal: 0, uv: i }).to_vec(), ..Part::default() },
            Part::default(),
        ],
        ..Mesh::default()
    };
    let wood = TextureRef { image: 0, uv_set: 1, wrap: [Wrap::Clamp, Wrap::Repeat] };
    let materials = vec![
        Material {
            name: "Wood".into(),
            base_color: [0.5, 0.25, 0.125, 1.0],
            metallic: 0.0,
            roughness: 0.75,
            emissive: [2.0, 1.0, 0.0],
            alpha_mode: AlphaMode::Mask(0.25),
            double_sided: true,
            unlit: true,
            base_color_texture: Some(wood),
            normal_texture: Some(TextureRef { image: 1, ..TextureRef::default() }),
            normal_scale: 0.5,
            occlusion_texture: Some(TextureRef { image: 2, ..TextureRef::default() }),
            ..Material::default()
        },
        Material { alpha_mode: AlphaMode::Blend, ..Material::plain("Glass", [1.0, 1.0, 1.0, 0.5]) },
    ];
    let mut scene = Scene::from_meshes(vec![mesh, Mesh { name: "empty".into(), ..Mesh::default() }], materials);
    scene.images = vec![
        Image { name: "wood".into(), data: Some(b"\x89PNG wood".to_vec()), ..Image::default() },
        Image { uri: Some("bump.png".into()), ..Image::default() },
        Image::default(),
    ];
    // root → the card, moved; the empty mesh's node stays, without a mesh.
    scene.nodes[0].transform[12] = 3.0;
    scene.nodes.push(Node { name: "root".into(), children: vec![0, 1], ..Node::default() });
    scene.roots = vec![2];
    scene
}

#[test]
fn written_files_read_back() {
    let scene = painted();
    let glb = write_glb(&scene, &Limits::default()).unwrap();
    assert_eq!(glb.len() % 4, 0);
    let back = parse(&glb).unwrap();
    back.validate().unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    // The hierarchy: names, placement, and the mesh without triangles gone from its node.
    assert_eq!(back.roots, [2]);
    assert_eq!(
        back.nodes.iter().map(|n| (n.name.as_str(), n.mesh, n.children.len())).collect::<Vec<_>>(),
        [("Card", Some(0), 0), ("empty", None, 0), ("root", None, 2)]
    );
    assert_eq!(back.nodes[0].transform, scene.nodes[0].transform);
    // The geometry, as one vertex per distinct corner.
    let (a, b) = (scene.meshes[0].unified(), back.meshes[0].unified());
    assert_eq!((back.meshes.len(), back.meshes[0].name.as_str()), (1, "Card"));
    assert_eq!((&a.positions, &a.normals, &a.uvs, &a.colors), (&b.positions, &b.normals, &b.uvs, &b.colors));
    assert_eq!(b.parts, [vec![[0, 1, 2], [0, 2, 3]], vec![[0, 2, 1]]]);
    assert_eq!(back.meshes[0].uv_sets[1], vec![[0.5, 0.5]; 4]);
    assert_eq!(back.meshes[0].parts.iter().map(|p| p.material).collect::<Vec<_>>(), [Some(0), Some(1)]);
    // The materials; the image without bytes or a name takes its texture slot with it.
    let mut wood = scene.materials[0].clone();
    wood.occlusion_texture = None;
    assert_eq!(back.materials, [wood, scene.materials[1].clone()]);
    assert_eq!(back.images.len(), 2);
    assert_eq!((back.images[0].data.as_deref(), back.images[0].mime_type.as_deref()), (Some(&b"\x89PNG wood"[..]), Some("image/png")));
    assert_eq!(back.images[1].uri.as_deref(), Some("bump.png"));
    assert_eq!(back.external, ["bump.png"]);
}

#[test]
fn writer_refuses_what_is_not_gltf() {
    let limits = Limits::default();
    let mut s = painted();
    s.nodes[0].transform[12] = f64::NAN;
    assert!(write_glb(&s, &limits).is_err());
    let mut s = painted();
    s.meshes[0].positions[0][0] = f64::INFINITY;
    assert!(write_glb(&s, &limits).is_err());
    let mut s = painted();
    s.materials[0].roughness = f32::NAN;
    assert!(write_glb(&s, &limits).is_err());
    let mut s = painted();
    s.meshes[0].parts[0].corners[0].position = 50;
    assert!(write_glb(&s, &limits).is_err());
    let mut s = painted();
    s.nodes[0].children = vec![2];
    assert!(write_glb(&s, &limits).is_err(), "a cycle");
    assert!(matches!(write_glb(&painted(), &Limits { max_data_bytes: 64, ..Limits::default() }), Err(Error::TooLarge(_))));
    // A scene with nothing in it is still a file.
    let empty = parse(&write_glb(&Scene::default(), &limits).unwrap()).unwrap();
    assert!(empty.meshes.is_empty() && empty.nodes.is_empty());
    // A mirrored node keeps its matrix: glTF viewers turn the winding themselves.
    let mut s = Scene::from_meshes(vec![super::tri_mesh()], vec![]);
    s.nodes[0].transform[0] = -1.0;
    let back = parse(&write_glb(&s, &limits).unwrap()).unwrap();
    assert_eq!(back.nodes[0].transform[0], -1.0);
    let t = back.meshes[0].unified().parts[0][0].map(|i| back.meshes[0].positions[i as usize]);
    assert_eq!(face_normal(t), Some([0.0, 0.0, 1.0]));
}

#[test]
fn sparse_accessors_replace_the_elements_they_name() {
    let mut b = B::default();
    let p = b.f32s(&QUAD, "VEC3", 3);
    let indices = b.view(&[3u8, 1]);
    let far = b.view(&[4u8, 1]);
    let values = b.view(&[0.0f32, 5.0, 0.0, 2.0, 0.0, 0.0].iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>());
    b.accessors[p]["sparse"] = json!({ "count": 2, "indices": { "bufferView": indices, "componentType": 5121 }, "values": { "bufferView": values } });
    // The same over an accessor without a buffer view: zeros, then the sparse values.
    b.accessors.push(json!({ "componentType": 5126, "count": 4, "type": "VEC3", "sparse": b.accessors[p]["sparse"] }));
    let prims = json!([{ "attributes": { "POSITION": p } }, { "attributes": { "POSITION": p + 1 } }]);
    let s = parse(&b.glb(json!({ "nodes": [{ "mesh": 0 }], "meshes": [{ "primitives": prims }] }))).unwrap();
    s.validate().unwrap();
    let pos = &s.meshes[0].positions;
    assert_eq!(pos[..4], [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 5.0, 0.0]]);
    assert_eq!(pos[4..], [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 5.0, 0.0]]);

    // What a sparse accessor may not do: name an element past the accessor, hold more entries
    // than it, read past a view, or leave its indices or values out.
    let broken = |change: &dyn Fn(&mut Value)| {
        let mut b2 = B { bin: b.bin.clone(), views: b.views.clone(), accessors: b.accessors.clone() };
        change(&mut b2.accessors[p]["sparse"]);
        parse(&one_mesh(&b2, json!({ "attributes": { "POSITION": p } })))
    };
    assert!(broken(&|_| {}).is_ok());
    for change in [
        (&|s: &mut Value| s["indices"]["bufferView"] = json!(far)) as &dyn Fn(&mut Value),
        &|s| s["count"] = json!(5),
        &|s| s["count"] = json!(3),
        &|s| s["values"]["byteOffset"] = json!(4),
        &|s| s["indices"]["componentType"] = json!(5126),
        &|s| *s = json!({ "count": 1 }),
    ] {
        assert!(matches!(broken(change), Err(Error::Malformed(_))));
    }
    hammer(&one_mesh(&b, json!({ "attributes": { "POSITION": p } })), &parse);
}

#[test]
fn specular_glossiness_and_webp_textures() {
    let mut b = B::default();
    let p = b.f32s(&QUAD[..9], "VEC3", 3);
    let doc = b.glb(json!({
        "extensionsRequired": ["KHR_materials_pbrSpecularGlossiness", "EXT_texture_webp"],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": p }, "material": 0 }] }],
        "images": [{ "uri": "fallback.png" }, { "uri": "only.webp" }, { "uri": "better.webp" }],
        "textures": [
            { "extensions": { "EXT_texture_webp": { "source": 1 } } },
            { "source": 0, "extensions": { "EXT_texture_webp": { "source": 2 } } },
        ],
        "materials": [
            {
                "pbrMetallicRoughness": { "baseColorFactor": [0.0, 0.0, 0.0, 1.0], "metallicRoughnessTexture": { "index": 1 } },
                "normalTexture": { "index": 1 },
                "extensions": { "KHR_materials_pbrSpecularGlossiness": {
                    "diffuseFactor": [0.5, 0.25, 1.0, 0.75], "diffuseTexture": { "index": 0, "texCoord": 1 }, "glossinessFactor": 0.25,
                } },
            },
            { "extensions": { "KHR_materials_pbrSpecularGlossiness": {} } },
        ],
    }));
    let s = parse(&doc).unwrap();
    s.validate().unwrap();
    // Both extensions are read, so neither is named in a warning.
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    let m = &s.materials[0];
    assert_eq!((m.base_color, m.metallic, m.roughness), ([0.5, 0.25, 1.0, 0.75], 0.0, 0.75));
    // A texture with only a WebP image uses it; one with a fallback keeps the fallback.
    assert_eq!(m.base_color_texture, Some(TextureRef { image: 1, uv_set: 1, ..TextureRef::default() }));
    assert_eq!(m.normal_texture.map(|t| t.image), Some(0));
    assert_eq!(m.metallic_roughness_texture, None);
    let plain = &s.materials[1];
    assert_eq!((plain.base_color, plain.metallic, plain.roughness, plain.base_color_texture), ([1.0; 4], 0.0, 0.0, None));
}
