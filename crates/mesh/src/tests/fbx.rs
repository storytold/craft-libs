//! FBX: the same scene as ASCII and as every binary layout, textures, limits and hostile files.

use super::hammer;
use crate::fbx::{BINARY_MAGIC, Document, Prop, Record, is_binary, mesh_bytes, read, read_document, value_bytes};
use crate::{AlphaMode, Corner, Error, Limits, NONE, Scene, mat_mul, no_external, transform_point};

/// A Z-up scene: a Null parent with a mesh child (rotated, scaled, with a geometric offset)
/// holding a quad and a triangle with a material each.
const ASCII: &str = r#"; FBX 7.4.0 project file
; ----------------------------------------------------
FBXHeaderExtension:  {
	FBXHeaderVersion: 1003
	FBXVersion: 7400
	Creator: "craft-mesh test"
}
GlobalSettings:  {
	Version: 1000
	Properties70:  {
		P: "UpAxis", "int", "Integer", "",2
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",1
		P: "FrontAxisSign", "int", "Integer", "",-1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",2.54
	}
}
Objects:  {
	Geometry: 100, "Geometry::Shape", "Mesh" {
		Vertices: *15 {
			a: 0,0,0,1,0,0,1,1,0,0,1,0,0.5,0.5,1
		}
		PolygonVertexIndex: *7 {
			a: 0,1,2,-4,0,1,-5
		}
		LayerElementNormal: 0 {
			MappingInformationType: "ByPolygon"
			ReferenceInformationType: "Direct"
			Normals: *6 {
				a: 0,0,1,0,-1,0
			}
		}
		LayerElementUV: 0 {
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "IndexToDirect"
			UV: *8 {
				a: 0,0,1,0,1,1,0,1
			}
			UVIndex: *7 {
				a: 0,1,2,3,0,1,2
			}
		}
		LayerElementMaterial: 0 {
			MappingInformationType: "ByPolygon"
			ReferenceInformationType: "IndexToDirect"
			Materials: *2 {
				a: 0,1
			}
		}
	}
	Model: 200, "Model::Parent", "Null" {
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",10,0,0
		}
	}
	Model: 300, "Model::Shape", "Mesh" {
		Properties70:  {
			P: "Lcl Rotation", "Lcl Rotation", "", "A",0,0,90
			P: "Lcl Scaling", "Lcl Scaling", "", "A",2,2,2
			P: "GeometricTranslation", "Vector3D", "Vector", "",0,0,1
		}
		Shading: T
	}
	Material: 400, "Material::Red", "" {
		ShadingModel: "phong"
		Properties70:  {
			P: "DiffuseColor", "Color", "", "A",1,0,0
			P: "ShininessExponent", "Number", "", "A",98
		}
	}
	Material: 500, "Material::Glass", "" {
		ShadingModel: "lambert"
		Properties70:  {
			P: "DiffuseColor", "Color", "", "A",0,0,1
			P: "Opacity", "double", "Number", "",0.25
		}
	}
}
Connections:  {
	;Model::Parent, Model::RootNode
	C: "OO",200,0
	C: "OO",300,200
	C: "OO",100,300
	C: "OO",400,300
	C: "OO",500,300
}
"#;

fn parse(bytes: &[u8]) -> crate::Result<Scene> {
    read(bytes, &Limits::default(), &mut no_external)
}

fn within(bytes: &[u8], max_data_bytes: usize) -> crate::Result<Scene> {
    read(bytes, &Limits { max_data_bytes, ..Limits::default() }, &mut no_external)
}

fn document(bytes: &[u8]) -> Document {
    read_document(bytes, &Limits::default()).unwrap()
}

fn check_scene(doc: &Document, m: &Scene) {
    m.validate().unwrap();
    assert_eq!(m.meshes.len(), 1);
    let names: Vec<&str> = m.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["Parent", "Shape", "Shape geometry"]);
    assert_eq!(m.roots, vec![0]);
    assert_eq!(m.nodes[0].children, vec![1]);
    assert_eq!(m.nodes[1].children, vec![2]);
    assert_eq!(m.nodes[2].mesh, Some(0));

    let mesh = &m.meshes[0];
    assert_eq!((mesh.name.as_str(), mesh.positions.len(), mesh.parts.len()), ("Shape", 5, 2));
    let (quad, tri) = (&mesh.parts[0], &mesh.parts[1]);
    // The quad stays whole; a normal per polygon; UVs through the index.
    assert_eq!(quad.face_sizes, [4]);
    assert_eq!(quad.corners, [0, 1, 2, 3].map(|i| Corner { position: i, normal: 0, uv: i }));
    assert!(tri.face_sizes.is_empty());
    assert_eq!(tri.corners, [(0, 0), (1, 1), (4, 2)].map(|(p, uv)| Corner { position: p, normal: 1, uv }));
    assert_eq!(mesh.normals, [[0.0, 0.0, 1.0], [0.0, -1.0, 0.0]]);
    // v flipped to image rows.
    assert_eq!(mesh.uv_sets, [vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]]);

    assert_eq!(m.materials.len(), 2);
    let (red, glass) = (&m.materials[quad.material.unwrap()], &m.materials[tri.material.unwrap()]);
    assert_eq!(red.name, "Red");
    assert_eq!(red.base_color, [1.0, 0.0, 0.0, 1.0]);
    assert!((red.roughness - 0.02f32.sqrt()).abs() < 1e-6 && red.metallic == 0.0);
    assert_eq!(glass.name, "Glass");
    assert_eq!(glass.base_color, [0.0, 0.0, 1.0, 0.25]);
    assert_eq!((glass.alpha_mode, glass.roughness), (AlphaMode::Blend, 1.0));

    // Parent translation, Z rotation, scale and the geometric offset; then Z-up → Y-up, which
    // the scene leaves to the caller.
    let inst = m.instances(&Limits::default()).unwrap();
    assert_eq!(inst.len(), 1);
    let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
    let apex = transform_point(&inst[0].1, [0.5, 0.5, 1.0]);
    assert!(close(apex, [9.0, 1.0, 4.0]), "{apex:?}");
    let apex = transform_point(&mat_mul(&doc.axes_to_y_up(), &inst[0].1), [0.5, 0.5, 1.0]);
    assert!(close(apex, [9.0, 4.0, -1.0]), "{apex:?}");
    assert_eq!(doc.unit_scale(), Some(2.54));
}

/// A binary FBX of the same record tree (`a:` children become typed arrays).
fn write_binary(records: &[Record], version: u32, compress: bool) -> Vec<u8> {
    fn prop(out: &mut Vec<u8>, p: &Prop, compress: bool) {
        let array = |out: &mut Vec<u8>, t: u8, len: usize, raw: Vec<u8>| {
            out.push(t);
            out.extend((len as u32).to_le_bytes());
            let data = if compress { miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6) } else { raw };
            out.extend((compress as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
        };
        match p {
            Prop::Int(i) => match i32::try_from(*i) {
                Ok(v) => {
                    out.push(b'I');
                    out.extend(v.to_le_bytes());
                }
                Err(_) => {
                    out.push(b'L');
                    out.extend(i.to_le_bytes());
                }
            },
            Prop::Float(f) => {
                out.push(b'D');
                out.extend(f.to_le_bytes());
            }
            Prop::Str(s) => {
                let s = match s.split_once("::") {
                    Some((class, name)) => format!("{name}\0\x01{class}"),
                    None => s.clone(),
                };
                out.push(b'S');
                out.extend((s.len() as u32).to_le_bytes());
                out.extend(s.as_bytes());
            }
            Prop::Bytes(b) => {
                out.push(b'R');
                out.extend((b.len() as u32).to_le_bytes());
                out.extend(b);
            }
            Prop::Ints(v) => array(out, b'i', v.len(), v.iter().flat_map(|&i| (i as i32).to_le_bytes()).collect()),
            Prop::Floats(v) => array(out, b'd', v.len(), v.iter().flat_map(|f| f.to_le_bytes()).collect()),
        }
    }
    fn record(out: &mut Vec<u8>, n: &Record, wide: bool, compress: bool) {
        let w = if wide { 8 } else { 4 };
        let start = out.len();
        out.extend(vec![0u8; 3 * w]);
        out.push(n.name.len() as u8);
        out.extend(n.name.as_bytes());
        let props = match n.child("a") {
            Some(a) if a.props.iter().all(|p| matches!(p, Prop::Int(_))) => vec![Prop::Ints(a.ints())],
            Some(a) => vec![Prop::Floats(a.floats())],
            None => n.props.clone(),
        };
        let pstart = out.len();
        for p in &props {
            prop(out, p, compress);
        }
        let plen = out.len() - pstart;
        let kids: Vec<&Record> = n.children.iter().filter(|c| c.name != "a").collect();
        if !kids.is_empty() {
            for k in kids {
                record(out, k, wide, compress);
            }
            out.extend(vec![0u8; 3 * w + 1]);
        }
        let end = out.len();
        for (k, v) in [end, props.len(), plen].into_iter().enumerate() {
            let at = start + k * w;
            if wide {
                out[at..at + 8].copy_from_slice(&(v as u64).to_le_bytes());
            } else {
                out[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
            }
        }
    }
    let wide = version >= 7500;
    let mut out = BINARY_MAGIC.to_vec();
    out.extend(version.to_le_bytes());
    for n in records {
        record(&mut out, n, wide, compress);
    }
    out.extend(vec![0u8; if wide { 25 } else { 13 }]);
    // Footer bytes after the closing null record.
    out.extend([0xfa; 16]);
    out
}

#[test]
fn ascii_scene() {
    assert!(!is_binary(ASCII.as_bytes()));
    check_scene(&document(ASCII.as_bytes()), &parse(ASCII.as_bytes()).unwrap());
}

#[test]
fn binary_matches_ascii() {
    let doc = document(ASCII.as_bytes());
    let ascii = parse(ASCII.as_bytes()).unwrap();
    for (version, compress) in [(7400, false), (7400, true), (7500, true), (6100, false)] {
        let bin = write_binary(&doc.records, version, compress);
        assert!(is_binary(&bin));
        let d = document(&bin);
        assert_eq!(d.version, version);
        assert_eq!(d.records.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["FBXHeaderExtension", "GlobalSettings", "Objects", "Connections"]);
        let m = parse(&bin).unwrap();
        check_scene(&d, &m);
        assert_eq!(m, ascii, "version {version}, compressed {compress}");
    }
}

/// [`ASCII`] with more objects before the end of `Objects` and more connections.
fn with(objects: &str, connections: &str) -> String {
    ASCII
        .replace("}\nConnections:  {", &format!("{objects}}}\nConnections:  {{"))
        .replace("C: \"OO\",500,300\n", &format!("C: \"OO\",500,300\n{connections}"))
}

fn b64(b: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            s.push(if k <= c.len() { A[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

#[test]
fn embedded_named_and_supplied_textures() {
    let text = with(
        &format!(
            "\tTexture: 600, \"Texture::Base\", \"\" {{\n\t\tRelativeFilename: \"maps\\base.png\"\n\t}}\n\
             \tTexture: 700, \"Texture::Glow\", \"\" {{\n\t\tRelativeFilename: \"missing.png\"\n\t}}\n\
             \tTexture: 900, \"Texture::Bumps\", \"\" {{\n\t\tFileName: \"C:\\art\\bumps.png\"\n\t}}\n\
             \tVideo: 800, \"Video::Glow\", \"Clip\" {{\n\t\tContent: \"{}\"\n\t}}\n",
            b64(b"embedded image")
        ),
        "\tC: \"OP\",600,400, \"DiffuseColor\"\n\tC: \"OP\",700,500, \"EmissiveColor\"\n\tC: \"OO\",800,700\n\tC: \"OP\",900,400, \"NormalMap\"\n",
    );
    let mut asked = vec![];
    let m = read(text.as_bytes(), &Limits::default(), &mut |uri: &str| {
        asked.push(uri.to_string());
        (uri == "maps/base.png").then(|| b"file image".to_vec())
    })
    .unwrap();
    m.validate().unwrap();
    let image = |t: Option<crate::TextureRef>| &m.images[t.unwrap().image];
    let red = m.materials.iter().find(|x| x.name == "Red").unwrap();
    let base = image(red.base_color_texture);
    assert_eq!((base.name.as_str(), base.uri.as_deref(), base.data.as_deref()), ("Base", Some("maps/base.png"), Some(&b"file image"[..])));
    // Declined (as written, then by its last component): named only.
    let bumps = image(red.normal_texture);
    assert_eq!((bumps.uri.as_deref(), &bumps.data), (Some("C:/art/bumps.png"), &None));
    // Embedded content wins over the file, and an emissive texture lights the material.
    let glass = m.materials.iter().find(|x| x.name == "Glass").unwrap();
    let glow = image(glass.emissive_texture);
    assert_eq!((glow.uri.as_deref(), glow.data.as_deref()), (Some("missing.png"), Some(&b"embedded image"[..])));
    assert_eq!(glass.emissive, [1.0; 3]);
    assert_eq!(m.images.len(), 3);
    assert_eq!(asked, ["maps/base.png", "C:/art/bumps.png", "bumps.png"]);
    assert_eq!(m.external, ["maps/base.png", "C:/art/bumps.png", "missing.png"]);

    // The same embedded content in a binary file is raw bytes.
    let mut doc = document(text.as_bytes());
    let objects = doc.records.iter_mut().find(|r| r.name == "Objects").unwrap();
    let video = objects.children.iter_mut().find(|r| r.name == "Video").unwrap();
    video.children[0].props = vec![Prop::Bytes(b"embedded image".to_vec())];
    let bin = parse(&write_binary(&doc.records, 7400, true)).unwrap();
    assert!(bin.images.iter().any(|i| i.data.as_deref() == Some(&b"embedded image"[..])));
}

#[test]
fn textures_that_end_at_one_image_share_it() {
    // Two Texture objects naming one file; a third reaching it through its second name.
    let text = with(
        "\tTexture: 600, \"Texture::Base\", \"\" {\n\t\tRelativeFilename: \"maps\\base.png\"\n\t}\n\
         \tTexture: 700, \"Texture::Base again\", \"\" {\n\t\tRelativeFilename: \"maps\\base.png\"\n\t}\n\
         \tTexture: 710, \"Texture::Other spelling\", \"\" {\n\t\tRelativeFilename: \"missing\\base.png\"\n\t\tFileName: \"maps\\base.png\"\n\t}\n",
        "\tC: \"OP\",600,400, \"DiffuseColor\"\n\tC: \"OP\",700,500, \"DiffuseColor\"\n\tC: \"OP\",710,500, \"EmissiveColor\"\n",
    );
    let image = b"twelve bytes".to_vec();
    let supply = |limit: usize| {
        let limits = Limits { max_data_bytes: limit, ..Limits::default() };
        read(text.as_bytes(), &limits, &mut |uri: &str| (uri == "maps/base.png").then(|| image.clone()))
    };
    let m = supply(1 << 30).unwrap();
    assert_eq!(m.images.len(), 1);
    assert!(m.materials.iter().all(|x| x.base_color_texture.map(|t| t.image) == Some(0)));
    assert_eq!(m.materials[1].emissive_texture.map(|t| t.image), Some(0));
    // The image's bytes count against the memory limit, once.
    let values = value_bytes(&document(text.as_bytes()).records).unwrap();
    let mesh = mesh_bytes(&m.meshes[0]).unwrap();
    assert_eq!(supply(values + mesh + image.len()).unwrap().images.len(), 1);
    assert!(matches!(supply(values + mesh + image.len() - 1), Err(Error::TooLarge(e)) if e.contains("memory limit")));
}

#[test]
fn hostile_files_fail_cleanly() {
    let doc = document(ASCII.as_bytes());
    // Every truncation and two corruptions of every byte return (no panic, no runaway), and
    // what is accepted is a valid scene.
    hammer(&write_binary(&doc.records, 7500, true), &parse);
    hammer(&write_binary(&doc.records, 7400, false), &parse);
    hammer(ASCII.as_bytes(), &parse);
    // An array claiming 4 G elements.
    let mut huge = BINARY_MAGIC.to_vec();
    huge.extend(7400u32.to_le_bytes());
    let body: Vec<u8> =
        b"d".iter().copied().chain(u32::MAX.to_le_bytes()).chain(0u32.to_le_bytes()).chain(8u32.to_le_bytes()).chain([0u8; 8]).collect();
    let end = huge.len() + 13 + 1 + body.len();
    huge.extend((end as u32).to_le_bytes());
    huge.extend(1u32.to_le_bytes());
    huge.extend((body.len() as u32).to_le_bytes());
    huge.push(1);
    huge.push(b'V');
    huge.extend(body);
    assert!(matches!(read_document(&huge, &Limits::default()), Err(Error::TooLarge(_))));
    // Deep nesting, both encodings.
    assert!(matches!(read_document("A: {".repeat(100_000).as_bytes(), &Limits::default()), Err(Error::TooLarge(_))));
    let mut deep = Record { name: "Leaf".into(), ..Record::default() };
    for _ in 0..200 {
        deep = Record { name: "N".into(), props: vec![], children: vec![deep] };
    }
    assert!(matches!(read_document(&write_binary(&[deep], 7400, false), &Limits::default()), Err(Error::TooLarge(_))));
    // Not an FBX scene at all, and an FBX version that is not read.
    assert!(matches!(parse(b"\x00\x01garbage"), Err(Error::Malformed(_))));
    assert!(parse(b"Kaydara FBX Binary  \0\x1a\0").is_err());
    let v5 = "FBXHeaderExtension:  {\n FBXVersion: 5800\n}\nObjects: {\n}\n";
    hammer(FBX6.as_bytes(), &parse);
    hammer(&write_binary(&document(FBX6.as_bytes()).records, 6100, false), &parse);
    assert!(matches!(parse(v5.as_bytes()), Err(Error::Unsupported(_))));
    // A scene without meshes is a scene.
    let empty = parse(b"Objects: {\n Model: 1, \"Model::Empty\", \"Null\" {\n }\n}\n").unwrap();
    assert_eq!((empty.nodes.len(), empty.roots.as_slice(), empty.meshes.len()), (1, &[0][..], 0));
    // Polygons that name a vertex that is not there, or have two corners, are left out; layer
    // indices past their arrays give corners without a normal.
    let text = "Objects: {\n Geometry: 1, \"Geometry::G\", \"Mesh\" {\n  Vertices: 0,0,0,1,0,0,0,1,0\n  PolygonVertexIndex: 0,1,99,-3,0,-2,0,1,-3\n  \
                LayerElementNormal: 0 {\n   MappingInformationType: \"ByVertice\"\n   ReferenceInformationType: \"IndexToDirect\"\n   Normals: 0,0,1\n   NormalsIndex: 0,7,-1\n  }\n }\n \
                Model: 2, \"Model::M\", \"Mesh\" {\n }\n}\nConnections: {\n C: \"OO\",1,2\n C: \"OO\",2,2\n}\n";
    let m = parse(text.as_bytes()).unwrap();
    m.validate().unwrap();
    assert_eq!(m.meshes[0].parts[0].corners, [Corner { position: 0, normal: 0, uv: NONE }, Corner::at(1), Corner::at(2)]);
    assert_eq!(m.warnings.len(), 1);
    assert!(m.warnings[0].starts_with("2 polygons"), "{:?}", m.warnings);
}

/// A version-7400 binary FBX holding one top-level record `A` with uncompressed arrays of type
/// `t` (`b'b'`, `b'i'`, `b'f'`, `b'd'`, `b'l'`) and `len` elements each.
fn binary_arrays(arrays: &[(u8, usize)]) -> Vec<u8> {
    let mut props = vec![];
    for &(t, len) in arrays {
        let size = match t {
            b'd' | b'l' => 8,
            b'f' | b'i' => 4,
            _ => 1,
        };
        props.push(t);
        props.extend((len as u32).to_le_bytes());
        props.extend(0u32.to_le_bytes());
        props.extend(((len * size) as u32).to_le_bytes());
        props.extend(std::iter::repeat_n(1u8, len * size));
    }
    let mut out = BINARY_MAGIC.to_vec();
    out.extend(7400u32.to_le_bytes());
    let end = out.len() + 13 + 1 + props.len();
    out.extend((end as u32).to_le_bytes());
    out.extend((arrays.len() as u32).to_le_bytes());
    out.extend((props.len() as u32).to_le_bytes());
    out.push(1);
    out.push(b'A');
    out.extend(props);
    out
}

#[test]
fn arrays_are_charged_for_the_memory_they_keep() {
    let doc = |bin: &[u8], max_data_bytes: usize| read_document(bin, &Limits { max_data_bytes, ..Limits::default() });
    // A boolean array is one byte per element in the file but eight once kept as `Prop::Ints`:
    // the limit counts the eight.
    let n = 10_000;
    let bin = binary_arrays(&[(b'b', n)]);
    assert!(doc(&bin, n * 8).is_ok());
    assert!(matches!(doc(&bin, n * 8 - 1), Err(Error::TooLarge(e)) if e.contains("memory limit")));
    // 32-bit floats widen to f64 the same way.
    assert!(doc(&binary_arrays(&[(b'f', n)]), n * 4).is_err());
    // Several arrays each within the limit but not together.
    let bin = binary_arrays(&[(b'i', n), (b'd', n), (b'b', n)]);
    assert!(doc(&bin, n * 24).is_ok());
    assert!(doc(&bin, n * 20).is_err());
    // ASCII values count too.
    let values = value_bytes(&document(ASCII.as_bytes()).records).unwrap();
    assert!(values > 45 * std::mem::size_of::<Prop>(), "{values}");
    assert!(matches!(within(ASCII.as_bytes(), values - 1), Err(Error::TooLarge(e)) if e.contains("memory limit")));
}

/// [`ASCII`] with two more Models instancing its geometry: `Copy` with the same two materials,
/// `Red only` with just the first.
fn instanced_scene() -> String {
    with(
        "\tModel: 301, \"Model::Copy\", \"Mesh\" {\n\t}\n\tModel: 302, \"Model::Red only\", \"Mesh\" {\n\t}\n",
        "\tC: \"OO\",301,0\n\tC: \"OO\",100,301\n\tC: \"OO\",400,301\n\tC: \"OO\",500,301\n\tC: \"OO\",302,0\n\tC: \"OO\",100,302\n\tC: \"OO\",400,302\n",
    )
}

#[test]
fn models_instancing_one_geometry_share_its_mesh() {
    let text = instanced_scene();
    let m = parse(text.as_bytes()).unwrap();
    m.validate().unwrap();
    let mesh_of = |name: &str| {
        let n = m.nodes.iter().position(|n| n.name == name).unwrap();
        // The mesh sits on the node, or on its geometric-offset child.
        m.nodes[n].mesh.or_else(|| m.nodes[n].children.iter().find_map(|&c| m.nodes[c].mesh)).unwrap()
    };
    assert_eq!(mesh_of("Shape"), mesh_of("Copy"), "same geometry and materials: one mesh");
    assert_ne!(mesh_of("Shape"), mesh_of("Red only"), "different materials: its own mesh");
    assert_eq!(m.meshes.len(), 2);
    assert_eq!(m.roots.len(), 3);
    // A limit with room for the two meshes, but not a third, still reads.
    let meshes: usize = m.meshes.iter().map(|m| mesh_bytes(m).unwrap()).sum();
    let values = value_bytes(&document(text.as_bytes()).records).unwrap();
    assert!(within(text.as_bytes(), values + meshes).is_ok());
    assert!(within(text.as_bytes(), values + meshes - 1).is_err());
}

#[test]
fn binary_child_record_past_its_parent_is_an_error() {
    let parent = Record { name: "P".into(), props: vec![], children: vec![Record { name: "C".into(), ..Record::default() }] };
    let after = Record { name: "Q".into(), props: vec![Prop::Int(1)], children: vec![] };
    let mut bin = write_binary(&[parent, after], 7400, false);
    assert_eq!(document(&bin).records.len(), 2);
    // `P` starts after the magic and version; `C` after P's 12-byte header, name length and name.
    let p = BINARY_MAGIC.len() + 4;
    let p_end = u32::from_le_bytes(bin[p..p + 4].try_into().unwrap());
    let c = p + 14;
    bin[c..c + 4].copy_from_slice(&(p_end + 1).to_le_bytes());
    assert!(matches!(read_document(&bin, &Limits::default()), Err(Error::Malformed(e)) if e.contains("bad record end")));
}

#[test]
fn a_failed_array_inflate_reports_only_its_status() {
    let doc = document(ASCII.as_bytes());
    let mut bin = write_binary(&doc.records, 7400, true);
    // Break the zlib header of the first compressed array (`Vertices`: type, count, encoding 1,
    // length, then the stream).
    let at = bin.windows(8).position(|w| w == b"Vertices").unwrap() + 8 + 13;
    bin[at] = 0xff;
    let Err(Error::Malformed(e)) = read_document(&bin, &Limits::default()) else { panic!("read a broken array") };
    assert!(e.contains("array zlib") && e.len() < 80, "{e}");
}

#[test]
fn every_limit_is_kept() {
    let text = instanced_scene();
    let refused = |limits: Limits| matches!(read(text.as_bytes(), &limits, &mut no_external), Err(Error::TooLarge(_)));
    let d = Limits::default;
    // 5 vertices; 3 triangles in each of 2 meshes; a quad; 2 material slots; 2 materials;
    // 4 Models and 3 geometry nodes; 2 meshes.
    for (at, under) in [
        (Limits { max_vertices: 5, ..d() }, Limits { max_vertices: 4, ..d() }),
        (Limits { max_triangles: 6, ..d() }, Limits { max_triangles: 5, ..d() }),
        (Limits { max_face_corners: 4, ..d() }, Limits { max_face_corners: 3, ..d() }),
        (Limits { max_parts: 2, ..d() }, Limits { max_parts: 1, ..d() }),
        (Limits { max_materials: 2, ..d() }, Limits { max_materials: 1, ..d() }),
        (Limits { max_nodes: 5, ..d() }, Limits { max_nodes: 4, ..d() }),
        (Limits { max_meshes: 2, ..d() }, Limits { max_meshes: 1, ..d() }),
    ] {
        assert!(!refused(at.clone()), "{at:?}");
        assert!(refused(under.clone()), "{under:?}");
    }
    // Images share the materials' limit.
    let textured = with(
        "\tTexture: 600, \"Texture::A\", \"\" {\n\t\tFileName: \"a.png\"\n\t}\n\tTexture: 700, \"Texture::B\", \"\" {\n\t\tFileName: \"b.png\"\n\t}\n\tMaterial: 510, \"Material::Third\", \"\" {\n\t}\n",
        "\tC: \"OP\",600,400, \"DiffuseColor\"\n\tC: \"OP\",700,400, \"NormalMap\"\n\tC: \"OO\",510,300\n",
    );
    let images = |max_materials: usize| read(textured.as_bytes(), &Limits { max_materials, ..d() }, &mut no_external);
    assert_eq!(images(3).unwrap().images.len(), 2);
    assert!(matches!(images(1), Err(Error::TooLarge(_))));
}

#[test]
fn axes_that_are_not_a_turn_are_left_alone() {
    let both_on_z = ASCII.replace("\"FrontAxis\", \"int\", \"Integer\", \"\",1", "\"FrontAxis\", \"int\", \"Integer\", \"\",2");
    assert_eq!(document(both_on_z.as_bytes()).axes_to_y_up(), crate::IDENTITY);
    let no_settings = document(b"Objects: {\n}\n");
    assert_eq!((no_settings.axes_to_y_up(), no_settings.unit_scale()), (crate::IDENTITY, None));
}

/// FBX 6.1 as Blender's exporter wrote it: the Model holds the mesh, `Properties60`, objects
/// named in `Connect` lines, `GlobalSettings` among the objects.
const FBX6: &str = r#"; FBX 6.1.0 project file
FBXHeaderExtension:  {
	FBXHeaderVersion: 1003
	FBXVersion: 6100
}
Objects:  {
	Model: "Model::Root", "Null" {
		Properties60:  {
			Property: "Lcl Scaling", "Lcl Scaling", "A+",2,2,2
		}
	}
	Model: "Model::Tri", "Mesh" {
		Properties60:  {
			Property: "Lcl Translation", "Lcl Translation", "A+",0,5,0
		}
		Vertices: 0,0,0,1,0,0,0,1,0,1,1,0
		PolygonVertexIndex: 0,1,-3,1,3,-3
		LayerElementNormal: 0 {
			MappingInformationType: "ByVertice"
			ReferenceInformationType: "IndexToDirect"
			Normals: 0,0,1
			NormalIndex: 0,0,0,0
		}
		LayerElementMaterial: 0 {
			MappingInformationType: "ByPolygon"
			ReferenceInformationType: "IndexToDirect"
			Materials: 0,1
		}
	}
	Material: "Material::Blue", "" {
		ShadingModel: "lambert"
		Properties60:  {
			Property: "Diffuse", "ColorRGB", "",0,0,1
		}
	}
	Material: "Material::Green", "" {
		Properties60:  {
			Property: "DiffuseColor", "ColorRGB", "",0,1,0
		}
	}
	GlobalSettings:  {
		Version: 1000
		Properties60:  {
			Property: "UpAxis", "int", "",2
			Property: "FrontAxis", "int", "",1
			Property: "FrontAxisSign", "int", "",-1
			Property: "UnitScaleFactor", "double", "",100
		}
	}
}
Connections:  {
	Connect: "OO", "Model::Root", "Model::Scene"
	Connect: "OO", "Model::Tri", "Model::Root"
	Connect: "OO", "Material::Blue", "Model::Tri"
	Connect: "OO", "Material::Green", "Model::Tri"
}
"#;

#[test]
fn fbx6_model_holds_its_mesh() {
    let doc = document(FBX6.as_bytes());
    let text = parse(FBX6.as_bytes()).unwrap();
    let binary = write_binary(&doc.records, 6100, false);
    assert_eq!(document(&binary).version, 6100);
    assert_eq!(parse(&binary).unwrap(), text);
    for (m, d) in [(&text, &doc), (&parse(&binary).unwrap(), &document(&binary))] {
        m.validate().unwrap();
        assert_eq!(m.nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["Root", "Tri"]);
        assert_eq!((m.roots.as_slice(), m.nodes[0].children.as_slice(), m.nodes[1].mesh), (&[0][..], &[1][..], Some(0)));
        let mesh = &m.meshes[0];
        assert_eq!((mesh.positions.len(), mesh.normals.as_slice()), (4, &[[0.0, 0.0, 1.0]][..]));
        // A part per material slot, in the order the materials connect to the Model.
        let colors: Vec<[f32; 4]> = mesh.parts.iter().map(|p| m.materials[p.material.unwrap()].base_color).collect();
        assert_eq!(colors, [[0.0, 0.0, 1.0, 1.0], [0.0, 1.0, 0.0, 1.0]]);
        assert_eq!(mesh.parts[1].corners, [1, 3, 2].map(|i| Corner { position: i, normal: 0, uv: NONE }));
        // Translated by 5 inside a parent scaled by 2.
        let world = m.instances(&Limits::default()).unwrap()[0].1;
        assert_eq!(transform_point(&world, [1.0, 1.0, 0.0]), [2.0, 12.0, 0.0]);
        // Z-up, from the settings among the objects.
        assert_eq!(transform_point(&d.axes_to_y_up(), [1.0, 2.0, 3.0]), [1.0, 3.0, -2.0]);
        assert_eq!(d.unit_scale(), Some(100.0));
    }
}
