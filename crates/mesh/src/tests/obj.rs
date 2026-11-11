use super::hammer;
use crate::obj::{material_names, read, read_mtl, sibling_name, write, write_mtl};
use crate::{AlphaMode, Corner, Error, Limits, Material, Mesh, NONE, Part, Scene, no_external};

/// A unit cube with texture coordinates, normals, two materials and one face written with
/// negative references.
const CUBE: &str = "# a unit cube\nmtllib cube.mtl\no Cube\n\
v -0.5 -0.5  0.5\nv  0.5 -0.5  0.5\nv  0.5  0.5  0.5\nv -0.5  0.5  0.5\nv -0.5 -0.5 -0.5\nv  0.5 -0.5 -0.5\nv  0.5  0.5 -0.5\nv -0.5  0.5 -0.5\n\
vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn  0  0  1\nvn  0  0 -1\nvn  1  0  0\nvn -1  0  0\nvn  0  1  0\nvn  0 -1  0\n\
usemtl Red\nf 1/1/1 2/2/1 3/3/1 4/4/1\nf 6/1/2 5/2/2 8/3/2 7/4/2\nf 2/1/3 6/2/3 7/3/3 3/4/3\n\
usemtl Gold\nf 5/1/4 1/2/4 4/3/4 8/4/4\nf 4/1/5 3/2/5 7/3/5 8/4/5\nf -4/1/6 -3/2/6 -7/3/6 -8/4/6\n";
const CUBE_MTL: &str =
    "# materials for the cube\nnewmtl Red\nKd 1.0 0.0 0.0\nNs 98\nd 1.0\n\nnewmtl Gold\nKd 1.0 0.766 0.336\nKe 0.0 0.0 0.0\nPr 0.3\nPm 1.0\n";

fn parse(bytes: &[u8]) -> crate::Result<Scene> {
    read(bytes, &Limits::default(), &mut no_external)
}

fn cube() -> Scene {
    read(CUBE.as_bytes(), &Limits::default(), &mut |name| (name == "cube.mtl").then(|| CUBE_MTL.as_bytes().to_vec())).unwrap()
}

#[test]
fn cube_with_its_material_library() {
    let s = cube();
    s.validate().unwrap();
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    assert_eq!(s.external, ["cube.mtl"]);
    assert_eq!(s.materials.len(), 2);
    let red = &s.materials[0];
    assert_eq!((red.name.as_str(), red.base_color, red.metallic), ("Red", [1.0, 0.0, 0.0, 1.0], 0.0));
    assert!((red.roughness - (2.0f32 / 100.0).sqrt()).abs() < 1e-6);
    assert_eq!((s.materials[1].metallic, s.materials[1].roughness), (1.0, 0.3));
    let m = &s.meshes[0];
    assert_eq!((m.name.as_str(), m.parts.len(), m.triangle_count()), ("Cube", 2, 12));
    assert_eq!((m.parts[0].material, m.parts[1].material), (Some(0), Some(1)));
    // Polygons stay whole, and the pools are the file's: 8 positions, 4 UVs, 6 normals.
    assert_eq!(m.parts[0].face_sizes, [4, 4, 4]);
    assert_eq!((m.positions.len(), m.uv_sets[0].len(), m.normals.len()), (8, 4, 6));
    // Four distinct (v, vt, vn) corners per face.
    let u = m.unified();
    assert_eq!((u.positions.len(), u.normals.len(), u.uvs.len()), (24, 24, 24));
    // Every face is wound counter-clockwise from outside (its face normal agrees with `vn`).
    for t in u.parts.iter().flatten() {
        let p = t.map(|i| u.positions[i as usize]);
        let f = crate::face_normal(p).unwrap();
        let n = u.normals[t[0] as usize];
        assert!(f[0] * n[0] + f[1] * n[1] + f[2] * n[2] > 0.99);
    }
    // V is flipped to the image's top-down rows.
    assert_eq!(m.uv_sets[0][0], [0.0, 1.0]);
    // The negative references of the last face name the same vertices as positive ones would.
    let last: Vec<u32> = m.parts[1].faces().last().unwrap().iter().map(|c| c.position).collect();
    assert_eq!(last, [4, 5, 1, 0]);
}

#[test]
fn a_library_that_is_not_supplied_leaves_named_defaults() {
    let s = parse(CUBE.as_bytes()).unwrap();
    assert_eq!(s.external, ["cube.mtl"]);
    assert_eq!(s.warnings.len(), 1);
    assert_eq!(s.materials.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Red", "Gold"]);
    assert_eq!(s.materials[0].base_color, [0.8, 0.8, 0.8, 1.0]);
}

#[test]
fn library_names_with_spaces_several_names_and_foreign_paths() {
    let src = |lib: &str| format!("mtllib {lib}\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl Red\nf 1 2 3\n");
    for (lib, supplied) in [("my materials.mtl", "my materials.mtl"), ("a.mtl b.mtl", "b.mtl"), ("C:\\models\\cube.mtl", "cube.mtl")] {
        let mut asked = Vec::new();
        let s = read(src(lib).as_bytes(), &Limits::default(), &mut |name| {
            asked.push(name.to_string());
            (name == supplied).then(|| CUBE_MTL.as_bytes().to_vec())
        })
        .unwrap();
        assert_eq!(s.materials[0].base_color, [1.0, 0.0, 0.0, 1.0], "{lib}: asked {asked:?}");
        // The name as written is asked first and is what the scene lists.
        assert_eq!((asked[0].as_str(), s.external[0].as_str()), (lib, lib));
    }
    assert_eq!(sibling_name("C:\\textures\\checker.png"), Some("checker.png"));
    assert_eq!(sibling_name("checker.png"), None);
}

#[test]
fn mtl_images_are_named_not_opened() {
    let mtl = "newmtl A\nmap_Kd my checker.png\nmap_Bump -bm 1 bump.png\nmap_Ke glow.png\nd 0.5\nnewmtl B\nmap_Kd my checker.png\nTr 0.25\n";
    let (materials, images) = read_mtl(mtl.as_bytes(), &Limits::default()).unwrap();
    // A complete name wins over its final token; options are passed over; a name is one image.
    assert_eq!(images.iter().map(|i| i.uri.as_deref().unwrap()).collect::<Vec<_>>(), ["my checker.png", "bump.png", "glow.png"]);
    assert!(images.iter().all(|i| i.data.is_none()));
    assert_eq!(materials[0].base_color_texture.unwrap().image, 0);
    assert_eq!(materials[0].normal_texture.unwrap().image, 1);
    assert_eq!(materials[0].emissive, [1.0; 3], "an emissive map without a colour shows");
    assert_eq!(materials[1].base_color_texture.unwrap().image, 0);
    assert_eq!((materials[0].alpha_mode, materials[0].base_color[3]), (AlphaMode::Blend, 0.5));
    assert_eq!(materials[1].base_color[3], 0.75);
    // Through an OBJ the image names are listed after the library.
    let s =
        read(b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl A\nf 1 2 3\n", &Limits::default(), &mut |_| Some(mtl.as_bytes().to_vec())).unwrap();
    assert_eq!(s.external, ["m.mtl", "my checker.png", "bump.png", "glow.png"]);
    s.validate().unwrap();
}

#[test]
fn objects_groups_quads_and_negative_indices() {
    let src = "# cube\no Cube\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
               f 1 4 3 2\nf 5 6 7 8\ng side walls\nf 1 2 6 5\nf 2/1 3/2 7/3 6/4\nf 3//1 4//1 8//1 7//1\nf -4 -8 -5 -1\n\
               o Tri\nv 5 5 5\nv 6 5 5\nv 5 6 5\nf -3 -2 -1\n";
    let s = parse(src.as_bytes()).unwrap();
    s.validate().unwrap();
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    assert_eq!(
        s.meshes.iter().map(|m| (m.name.as_str(), m.triangle_count(), m.positions.len())).collect::<Vec<_>>(),
        [("Cube", 12, 8), ("Tri", 1, 3)]
    );
    // Each object holds only its own vertices, and has a node.
    assert_eq!(s.meshes[1].positions[0], [5.0, 5.0, 5.0]);
    assert_eq!(s.nodes.iter().map(|n| n.mesh).collect::<Vec<_>>(), [Some(0), Some(1)]);
    // A group is a part; its name keeps its spaces.
    assert_eq!(s.meshes[0].parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["", "side walls"]);
    // References to texture coordinates and normals that are not there leave the corner without.
    assert!(s.meshes[0].parts[1].corners.iter().all(|c| c.uv == NONE && c.normal == NONE));
    // Without `o`, the file is one unnamed mesh.
    let s = parse(b"v 0 0 0\nv 1 0 0\nv 0 1 0\ng a\nf 1 2 3\ng b\nf 3 2 1\n").unwrap();
    assert_eq!((s.meshes.len(), s.meshes[0].name.as_str(), s.meshes[0].parts.len()), (1, "", 2));
}

#[test]
fn corner_normals_and_smoothing_groups() {
    let vertices = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 -0.8660254037844386 0.5\n";
    let authored = format!("{vertices}vn 0 0 1\nvn 0 0.5 0.8660254037844386\nf 1//-2 2//-2 3//-2\nf 2//-1 1//-1 4//-1\n");
    let s = parse(authored.as_bytes()).unwrap();
    let m = &s.meshes[0];
    assert_eq!((m.positions.len(), m.normals.len()), (4, 2));
    // Normals keep the precision of the text.
    assert_eq!(m.normals[1], [0.0, 0.5, 0.8660254037844386]);
    assert_eq!(m.parts[0].corners.iter().map(|c| c.normal).collect::<Vec<_>>(), [0, 0, 0, 1, 1, 1]);
    // A smoothing group is a part; `s off` and `s 0` end it.
    let s = parse(format!("{vertices}s 5\nf 1 2 3\ns off\nf 2 1 4\ns on\nf 1 2 3\ns 0\nf 1 2 3\n").as_bytes()).unwrap();
    assert_eq!(s.meshes[0].parts.iter().map(|p| (p.smoothing, p.triangle_count())).collect::<Vec<_>>(), [(5, 1), (0, 2), (1, 1)]);
}

#[test]
fn vertex_colours_continuations_and_polygons() {
    let src = "v 0 0 0 1 0 0\nv 1 0 0\nv 1 1 0 0 0 1\nv 0 1 0\nv 0.5 2 0\nf 1 2 \\\n 3 4 5\n";
    let s = parse(src.as_bytes()).unwrap();
    let m = &s.meshes[0];
    // Vertices without a colour are white once any has one.
    assert_eq!(m.colors, [[1.0, 0.0, 0.0, 1.0], [1.0; 4], [0.0, 0.0, 1.0, 1.0], [1.0; 4], [1.0; 4]]);
    assert_eq!((m.parts[0].face_sizes.as_slice(), m.triangle_count()), (&[5][..], 3));
    // Triangles alone keep no sizes; a later polygon fills them in.
    let s = parse(b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3\nf 1 2 3 4\n").unwrap();
    assert_eq!(s.meshes[0].parts[0].face_sizes, [3, 4]);
    assert!(parse(b"v 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\n").unwrap().meshes[0].parts[0].face_sizes.is_empty());
    assert!(s.meshes[0].colors.is_empty());
}

#[test]
fn bad_lines_are_left_out_and_counted() {
    // A vertex that cannot be read keeps its number; faces that use it are left out.
    let s = parse(b"v nan 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 2 3\nf 2 3 4\nf 2 3 x\nf 2 3\n").unwrap();
    assert_eq!(s.meshes[0].triangle_count(), 1);
    assert_eq!(s.meshes[0].positions, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    assert_eq!(s.warnings, ["4 vertex or face line(s) could not be read and were left out"]);
    // Non-finite normals and texture coordinates leave their corners without.
    let s = parse(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvn nan 0 1\nvt inf 0\nf 1/1/1 2/1/1 3/1/1\n").unwrap();
    assert!(s.meshes[0].normals.is_empty() && s.meshes[0].uv_sets.is_empty());
    assert_eq!(s.warnings.len(), 1);
    for src in [
        "",
        "f 1 2 3",
        "v 1 2\nf 1 1 1",
        "v 0 0 0\n",
        "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 99999999999999999999",
        "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -1 -2 -9",
        "o\ng\nf",
    ] {
        assert!(parse(src.as_bytes()).is_err(), "{src:?}");
    }
    assert!(parse(b"\x00\xffv 1 1 1").is_err());
    hammer(CUBE.as_bytes(), &parse);
    hammer(CUBE_MTL.as_bytes(), &|b| read(CUBE.as_bytes(), &Limits::default(), &mut |_| Some(b.to_vec())));
}

#[test]
fn limits() {
    let too = |r: crate::Result<Scene>| matches!(r, Err(Error::TooLarge(_)));
    let lim = |f: &dyn Fn(&mut Limits)| {
        let mut l = Limits::default();
        f(&mut l);
        l
    };
    let read_with = |src: &str, l: &Limits| read(src.as_bytes(), l, &mut |_| Some(CUBE_MTL.as_bytes().to_vec()));
    // Each pool on its own, counting definitions no face uses.
    assert!(read_with(CUBE, &lim(&|l| l.max_vertices = 8)).is_ok());
    assert!(too(read_with(CUBE, &lim(&|l| l.max_vertices = 7))));
    let unused = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 0 0\nvt 0 0\nvt 0 0\nf 1 2 3\n";
    assert!(too(read_with(unused, &lim(&|l| l.max_vertices = 3))), "texture coordinates");
    assert!(too(read_with(&unused.replace("vt 0 0", "vn 0 0 1"), &lim(&|l| l.max_vertices = 3))), "normals");
    assert!(read_with(CUBE, &lim(&|l| l.max_triangles = 12)).is_ok());
    assert!(too(read_with(CUBE, &lim(&|l| l.max_triangles = 11))));
    assert!(read_with(CUBE, &lim(&|l| l.max_face_corners = 4)).is_ok());
    assert!(too(read_with(CUBE, &lim(&|l| l.max_face_corners = 3))));
    assert!(read_with(CUBE, &lim(&|l| l.max_parts = 2)).is_ok());
    assert!(too(read_with(CUBE, &lim(&|l| l.max_parts = 1))));
    assert!(read_with(CUBE, &lim(&|l| l.max_materials = 2)).is_ok());
    assert!(too(read_with(CUBE, &lim(&|l| l.max_materials = 1))));
    // Materials named by `usemtl` alone count too.
    assert!(too(read(CUBE.as_bytes(), &lim(&|l| l.max_materials = 1), &mut no_external)));
    let two = "o a\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\no b\nf 1 2 3\n";
    assert!(read_with(two, &lim(&|l| l.max_meshes = 2)).is_ok());
    assert!(too(read_with(two, &lim(&|l| l.max_meshes = 1))));
    assert!(read_with(CUBE, &lim(&|l| l.max_data_bytes = CUBE_MTL.len())).is_ok());
    assert!(too(read_with(CUBE, &lim(&|l| l.max_data_bytes = CUBE_MTL.len() - 1))));
}

#[test]
fn written_files_read_back() {
    let scene = cube();
    let limits = Limits::default();
    let text = write(&scene, Some("cube.mtl"), &limits).unwrap();
    let mtl = write_mtl(&scene);
    let back = read(text.as_bytes(), &limits, &mut |name| (name == "cube.mtl").then(|| mtl.as_bytes().to_vec())).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(back.meshes, scene.meshes);
    for (a, b) in back.materials.iter().zip(&scene.materials) {
        assert_eq!((a.name.as_str(), a.metallic), (b.name.as_str(), b.metallic));
        assert!(a.base_color.iter().zip(b.base_color).all(|(x, y)| (x - y).abs() < 1e-5), "{a:?} {b:?}");
        assert!((a.roughness - b.roughness).abs() < 1e-5);
    }
    // Without a library name no `mtllib` is written; `usemtl` still names the materials.
    let alone = write(&scene, None, &limits).unwrap();
    assert!(!alone.contains("mtllib") && alone.contains("usemtl Red\n"));
}

#[test]
fn writer_places_meshes_keeps_colours_and_names_materials_apart() {
    let mesh = Mesh {
        name: "A\nB".into(),
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]],
        colors: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]],
        parts: vec![
            Part {
                name: "g1".into(),
                material: Some(0),
                smoothing: 2,
                corners: [0, 1, 2].map(|p| Corner { position: p, normal: 0, uv: NONE }).to_vec(),
                ..Part::default()
            },
            Part { material: Some(1), corners: [0, 1, 2].map(Corner::at).to_vec(), ..Part::default() },
        ],
        ..Mesh::default()
    };
    let mut scene =
        Scene::from_meshes(vec![mesh], vec![Material::plain("Steel", [1.0; 4]), Material::plain("steel", [1.0; 4]), Material::plain("  ", [1.0; 4])]);
    assert_eq!(material_names(&scene), ["Steel", "steel_2", "material"]);
    // The mesh placed twice: moved, and mirrored.
    scene.nodes.push(scene.nodes[0].clone());
    scene.roots.push(1);
    scene.nodes[0].transform[14] = 3.0;
    scene.nodes[1].transform[0] = -1.0;
    let text = write(&scene, Some("m.mtl"), &Limits::default()).unwrap();
    assert!(text.contains("o A B\n"), "a control character cannot split the line: {text}");
    assert!(text.contains("v 0 0 3 1 0 0\n") && text.contains("vn 0 0 1\n"));
    assert!(text.contains("g g1\nusemtl Steel\ns 2\nf 1//1 2//1 3//1\nusemtl steel_2\ns off\nf 1 2 3\n"), "{text}");
    // The second object's indices run on, and its mirrored faces are turned back.
    assert!(text.contains("s 2\nf 6//2 5//2 4//2\n"), "{text}");
    let back = parse(text.as_bytes()).unwrap();
    assert_eq!(back.meshes.len(), 2);
    assert_eq!(back.meshes[0].colors, scene.meshes[0].colors);
    for m in &back.meshes {
        let u = m.unified();
        let t = u.parts[0][0].map(|i| u.positions[i as usize]);
        assert!(crate::face_normal(t).unwrap()[2] > 0.99, "both copies face +Z");
    }
    // Nothing placed, or a mesh that fails validation, is an error.
    let mut empty = scene.clone();
    empty.roots.clear();
    assert!(write(&empty, None, &Limits::default()).is_err());
    scene.meshes[0].parts[0].corners[0].position = 9;
    assert!(write(&scene, None, &Limits::default()).is_err());
}

#[test]
fn map_options_are_passed_over_by_their_argument_counts() {
    // `-o` takes one to three numbers, so the name after it may hold spaces; the final token alone
    // would be `checker.png`.
    let mtl = "newmtl A\nmap_Kd -o 0.5 0.5 my checker.png\nmap_Bump -bm 0.3 -clamp on rough bump.png\nmap_Ke -s 2 2 2 -mm 0 1 glow.png\n\
               newmtl B\nmap_Kd -o 1 wood.png\nmap_Bump -unknown 3 my bump.png\nmap_Ke -bm\n";
    let (materials, images) = read_mtl(mtl.as_bytes(), &Limits::default()).unwrap();
    let name = |t: Option<crate::TextureRef>| t.map(|t| images[t.image].uri.clone().unwrap());
    assert_eq!(name(materials[0].base_color_texture).as_deref(), Some("my checker.png"));
    assert_eq!(name(materials[0].normal_texture).as_deref(), Some("rough bump.png"));
    assert_eq!(name(materials[0].emissive_texture).as_deref(), Some("glow.png"));
    assert_eq!(name(materials[1].base_color_texture).as_deref(), Some("wood.png"));
    // an option this reader does not know: the final token
    assert_eq!(name(materials[1].normal_texture).as_deref(), Some("bump.png"));
    // a lone token is the name, whatever it looks like
    assert_eq!(name(materials[1].emissive_texture).as_deref(), Some("-bm"));
}

#[test]
fn a_library_named_again_is_read_once() {
    // Exporters repeat `mtllib` for every object. Each repeat used to read the library again:
    // its materials and images twice, and its bytes counted against the limit once per line.
    let mtl = "newmtl Red\nKd 1 0 0\nmap_Kd red.png\n";
    let src = "mtllib m.mtl\no a\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl Red\nf 1 2 3\nmtllib m.mtl\no b\nusemtl Red\nf 1 2 3\nmtllib m.mtl\n";
    let mut asked = 0;
    let limits = Limits { max_data_bytes: mtl.len() * 2, ..Limits::default() };
    let s = read(src.as_bytes(), &limits, &mut |_| {
        asked += 1;
        Some(mtl.as_bytes().to_vec())
    })
    .unwrap();
    assert_eq!(asked, 1);
    assert_eq!((s.materials.len(), s.images.len()), (1, 1));
    assert_eq!(s.external, ["m.mtl", "red.png"]);
}
