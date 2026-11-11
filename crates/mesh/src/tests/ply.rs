use super::hammer;
use crate::ply::{Encoding, Kind, MAX_ELEMENTS, MAX_PROPERTIES, Rows, Scalar, is_ply, parse_header, read};
use crate::{Corner, Error, Limits, Scene};

fn parse(bytes: &[u8]) -> crate::Result<Scene> {
    read(bytes, &Limits::default())
}

/// A binary file with CRLF header lines, an element before the vertices, a property after the
/// colours and one triangle.
fn binary(big: bool) -> Vec<u8> {
    let order = if big { "big" } else { "little" };
    let mut b = format!(
        "ply\r\nformat binary_{order}_endian 1.0\r\ncomment made by hand\r\nelement camera 1\r\nproperty double k\r\nelement vertex 3\r\nproperty float32 x\r\nproperty float32 y\r\nproperty float32 z\r\nproperty uchar red\r\nproperty uchar green\r\nproperty uchar blue\r\nproperty float intensity\r\nelement face 1\r\nproperty list uchar int vertex_indices\r\nend_header\r\n"
    )
    .into_bytes();
    let f = |v: f32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    b.extend(if big { 9.5f64.to_be_bytes() } else { 9.5f64.to_le_bytes() });
    for (i, p) in [[0.0f32, 0.0, 0.0], [1.5, 0.0, 0.0], [0.0, 2.5, -3.0]].iter().enumerate() {
        for v in p {
            b.extend(f(*v));
        }
        b.extend([10 * i as u8, 20, 255]);
        b.extend(f(0.5));
    }
    b.push(3);
    for i in [0i32, 1, 2] {
        b.extend(if big { i.to_be_bytes() } else { i.to_le_bytes() });
    }
    b
}

/// A quad with normals, texture coordinates and 16-bit colours with alpha.
const FULL: &str = "ply\nformat ascii 1.0\ncomment the end_header is not here\nelement vertex 4\nproperty double x\nproperty double y\nproperty double z\n\
property float nx\nproperty float ny\nproperty float nz\nproperty float s\nproperty float t\n\
property ushort red\nproperty ushort green\nproperty ushort blue\nproperty ushort alpha\n\
element face 1\nproperty list uchar uint vertex_index\nend_header\n\
0 0 0 0 0 1 0 0 65535 0 0 65535\n1 0 0 0 0 1 1 0 0 65535 0 65535\n1 1 0 0 0 1 1 1 0 0 65535 0\n0.1 1 0 0 0 1 0 0.25 0 0 0 65535\n4 0 1 2 3\n";

#[test]
fn binary_little_and_big_endian_with_colours_and_other_elements() {
    for big in [false, true] {
        let bytes = binary(big);
        let h = parse_header(&bytes).unwrap();
        assert_eq!(h.encoding, if big { Encoding::Big } else { Encoding::Little });
        assert_eq!((h.count("vertex"), h.count("face"), h.count("edge")), (3, 1, 0));
        assert_eq!(h.comments, ["made by hand"]);
        let s = parse(&bytes).unwrap();
        s.validate().unwrap();
        let m = &s.meshes[0];
        assert_eq!(m.positions, [[0.0, 0.0, 0.0], [1.5, 0.0, 0.0], [0.0, 2.5, -3.0]]);
        assert_eq!(m.colors[1], [10.0 / 255.0, 20.0 / 255.0, 1.0, 1.0]);
        assert_eq!(m.parts[0].corners.iter().map(|c| c.position).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(m.parts[0].face_sizes.is_empty() && m.normals.is_empty() && m.uv_sets.is_empty());
    }
}

#[test]
fn ascii_with_normals_uvs_colours_and_a_quad() {
    let s = parse(FULL.as_bytes()).unwrap();
    s.validate().unwrap();
    let m = &s.meshes[0];
    // Doubles keep their precision.
    assert_eq!(m.positions[3], [0.1, 1.0, 0.0]);
    assert_eq!(m.normals, vec![[0.0, 0.0, 1.0]; 4]);
    // V is flipped to the image's top-down rows.
    assert_eq!(m.uv_sets[0], [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.75]]);
    // Integer channels are scaled by their type's range.
    assert_eq!(m.colors, [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);
    // The polygon stays whole, and each corner names its vertex's normal and UV.
    assert_eq!(m.parts[0].face_sizes, [4]);
    assert!(m.parts[0].corners.iter().all(|c| c.normal == c.position && c.uv == c.position));
    // Float colours are taken as they are; `u`/`v` name texture coordinates too.
    let floats = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float red\nproperty float green\nproperty float blue\nproperty float u\nproperty float v\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 1 0 0 0 0\n1 0 0 0 0.5 0 1 0\n0 1 0 0 0 1 0 1\n3 0 1 2\n";
    let s = parse(floats.as_bytes()).unwrap();
    assert_eq!(s.meshes[0].colors[1], [0.0, 0.5, 0.0, 1.0]);
    assert_eq!(s.meshes[0].uv_sets[0][2], [0.0, 0.0]);
}

#[test]
fn point_clouds_and_faces_before_vertices() {
    let points = "ply\nformat ascii 1.0\nelement vertex 2\nproperty float32 x\nproperty float32 y\nproperty float32 z\nend_header\n1 2 3\n4 5 6\n";
    let s = parse(points.as_bytes()).unwrap();
    assert_eq!((s.meshes[0].positions.as_slice(), s.meshes[0].parts.len()), (&[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]][..], 0));
    assert!(s.meshes[0].colors.is_empty());
    let face_first = "ply\nformat ascii 1.0\nelement face 1\nproperty list uchar int vertex_indices\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nend_header\n3 0 1 2\n0 0 0\n1 0 0\n0 1 0\n";
    let s = parse(face_first.as_bytes()).unwrap();
    assert_eq!((s.meshes[0].triangle_count(), s.meshes[0].positions.len()), (1, 3));
}

#[test]
fn faces_that_cannot_be_read_are_left_out() {
    let head = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 2\nproperty list uchar int vertex_indices\nend_header\n";
    // A face naming a vertex that is not there.
    let s = parse(format!("{head}0 0 0 1 0 0 0 1 0\n3 0 1 2\n3 0 1 99\n").as_bytes()).unwrap();
    assert_eq!((s.meshes[0].triangle_count(), s.warnings.len()), (1, 1));
    // A face naming a vertex whose position is not a number; the position itself is kept.
    let s = parse(format!("{head}0 0 0 1 0 0 nan 1 0\n3 0 1 2\n2 0 1\n").as_bytes()).unwrap();
    assert_eq!((s.meshes[0].triangle_count(), s.meshes[0].positions.len()), (0, 3));
    assert_eq!(s.warnings, ["2 face(s) could not be read and were left out"]);
    s.validate().unwrap();
}

#[test]
fn rows_serve_callers_that_read_more_than_a_mesh() {
    // Every scalar property of the vertices, as a point-cloud reader would take them.
    let bytes = binary(true);
    let h = parse_header(&bytes).unwrap();
    let mut rows = Rows::new(&h, &bytes);
    rows.skip_element(&h.elements[0], 16).unwrap();
    let vertex = &h.elements[1];
    rows.check(vertex).unwrap();
    assert_eq!(vertex.row_size(), Some(19));
    assert_eq!(vertex.find(&["Intensity"]), Some((6, Scalar::F32)));
    let mut intensity = Vec::new();
    for _ in 0..vertex.count {
        for (i, p) in vertex.properties.iter().enumerate() {
            let Kind::Scalar(t) = p.kind else { panic!("the vertices have no lists") };
            let v = rows.value(t).unwrap();
            if i == 6 {
                intensity.push(v);
            }
        }
    }
    assert_eq!(intensity, [0.5; 3]);
    assert_eq!(rows.count(Scalar::U8, 16).unwrap(), 3);
    assert!(Scalar::I16.is_integer() && !Scalar::F64.is_integer());
    assert_eq!(Scalar::U16.unit(65_535.0), 1.0);
}

#[test]
fn hostile_ply_is_refused_without_a_panic() {
    assert!(is_ply(b"ply\n") && !is_ply(b"plyx") && !is_ply(b"pl"));
    let cases: [&[u8]; 14] = [
        b"",
        b"ply",
        b"ply\nformat ascii 1.0\n",
        b"ply\nformat ascii 1.0\nelement vertex 18446744073709551615\nproperty float x\nproperty float y\nproperty float z\nend_header\n1 2 3",
        b"ply\nformat binary_little_endian 1.0\nelement vertex 18446744073709551615\nproperty float x\nproperty float y\nproperty float z\nend_header\n1234",
        b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty list uchar float x\nproperty float y\nproperty float z\nend_header\n\xff",
        b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nend_header\n1",
        b"ply\nformat ascii 1.0\nproperty float x\nend_header\n",
        b"ply\nformat binary_big_endian 1.0\nelement face 2\nproperty list float int vertex_indices\nend_header\n",
        b"ply\nformat utf8 1.0\nelement vertex 0\nend_header\n",
        b"ply\nelement vertex 0\nproperty float x\nproperty float y\nproperty float z\nend_header\n",
        b"ply\nformat ascii 1.0\nelement vertex 1\nproperty quad x\nend_header\n",
        b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header trailing\n1 2 3\n",
        b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n1 two 3\n",
    ];
    for c in cases {
        assert!(parse(c).is_err(), "{:?}", String::from_utf8_lossy(c));
    }
    // A cut-off file is refused, wherever it is cut.
    let good = binary(false);
    for n in 0..good.len() {
        assert!(parse(&good[..n]).is_err(), "cut at {n}");
    }
    // A header that never ends is not searched past its first megabyte.
    let mut endless = b"ply\nformat ascii 1.0\n".to_vec();
    endless.resize((1 << 20) + 64, b'c');
    endless.extend(b"\nend_header\n");
    assert!(parse(&endless).is_err());
    for good in [binary(false), binary(true), FULL.as_bytes().to_vec()] {
        hammer(&good, &parse);
    }
}

#[test]
fn limits() {
    let too = |r: crate::Result<Scene>| matches!(r, Err(Error::TooLarge(_)));
    let full = FULL.as_bytes();
    assert!(read(full, &Limits { max_vertices: 4, ..Limits::default() }).is_ok());
    assert!(too(read(full, &Limits { max_vertices: 3, ..Limits::default() })));
    assert!(read(full, &Limits { max_triangles: 2, ..Limits::default() }).is_ok());
    assert!(too(read(full, &Limits { max_triangles: 1, ..Limits::default() })));
    assert!(read(full, &Limits { max_face_corners: 4, ..Limits::default() }).is_ok());
    assert!(too(read(full, &Limits { max_face_corners: 3, ..Limits::default() })));
    // The header's own bounds.
    let elements = |n: usize| {
        format!(
            "ply\nformat ascii 1.0\n{}element vertex 0\nproperty float x\nproperty float y\nproperty float z\nend_header\n",
            "element e 0\n".repeat(n)
        )
    };
    assert!(parse(elements(MAX_ELEMENTS - 1).as_bytes()).is_ok());
    assert!(too(parse(elements(MAX_ELEMENTS).as_bytes())));
    let properties = |n: usize| {
        format!(
            "ply\nformat ascii 1.0\nelement vertex 0\n{}end_header\n",
            "property float x\nproperty float y\nproperty float z\n".repeat(n).lines().take(n).map(|l| format!("{l}\n")).collect::<String>()
        )
    };
    assert!(parse(properties(MAX_PROPERTIES).as_bytes()).is_ok());
    assert!(too(parse(properties(MAX_PROPERTIES + 1).as_bytes())));
}

/// As MeshLab writes a textured mesh: the texture named in a comment, a `u v` per face corner.
const WEDGE_UVS: &str = "ply\nformat ascii 1.0\ncomment TextureFile wood grain.png\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\n\
property float s\nproperty float t\nelement face 3\nproperty list uchar int vertex_indices\nproperty list uchar float texcoord\nend_header\n\
0 0 0 9 9\n1 0 0 9 9\n1 1 0 9 9\n0 1 0 9 9\n3 0 1 2 6 0 0 1 0 1 1\n3 0 2 3 6 0 0.25 1 1 0 1\n3 0 1 3 4 0 0 1 1\n";

#[test]
fn per_corner_texcoords_and_the_texture_file_comment() {
    let s = parse(WEDGE_UVS.as_bytes()).unwrap();
    s.validate().unwrap();
    let mesh = &s.meshes[0];
    // Vertex 0 has two coordinates, one per face: the faces' lists are the UV set (v flipped),
    // not the vertices' `s t`. A list that does not fit its face is passed over.
    assert_eq!(mesh.uv_sets, [vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.75], [1.0, 0.0], [0.0, 0.0]]]);
    let corners: Vec<Corner> =
        [(0, 0), (1, 1), (2, 2), (0, 3), (2, 4), (3, 5)].map(|(position, uv)| Corner { position, normal: crate::NONE, uv }).to_vec();
    assert_eq!(mesh.parts[0].corners, [corners, [0, 1, 3].map(Corner::at).to_vec()].concat());
    // The comment names the image of a material; nothing is opened.
    let material = &s.materials[mesh.parts[0].material.unwrap()];
    assert_eq!(s.images[material.base_color_texture.unwrap().image].uri.as_deref(), Some("wood grain.png"));
    assert_eq!(s.external, ["wood grain.png"]);
    // Without either, the vertices' own coordinates and no material, as before.
    let plain = WEDGE_UVS.replace("comment TextureFile wood grain.png\n", "").replace("texcoord", "other");
    let s = parse(plain.as_bytes()).unwrap();
    assert_eq!(
        (s.materials.len(), s.meshes[0].uv_sets[0].len(), s.meshes[0].parts[0].corners[4]),
        (0, 4, Corner { position: 2, normal: crate::NONE, uv: 2 })
    );
    // The coordinates count against the vertex limit, and a coordinate list may be twice a face.
    let tight = |max_vertices, max_face_corners| read(WEDGE_UVS.as_bytes(), &Limits { max_vertices, max_face_corners, ..Limits::default() });
    assert!(tight(6, 3).is_ok());
    assert!(matches!(tight(5, 3), Err(Error::TooLarge(_))));
    hammer(WEDGE_UVS.as_bytes(), &parse);
}
