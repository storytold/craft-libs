use super::hammer;
use crate::stl::{read, write_ascii, write_binary};
use crate::{Error, IDENTITY, Limits, Scene};

/// A facet: normal, vertices, attribute word.
type Facet = ([f32; 3], [[f32; 3]; 3], u16);

/// A binary STL: `header` (padded to 80 bytes) and its facets.
fn binary_stl(header: &[u8], facets: &[Facet]) -> Vec<u8> {
    let mut b = header.to_vec();
    b.resize(80, 0);
    b.extend((facets.len() as u32).to_le_bytes());
    for (n, v, attr) in facets {
        for c in n.iter().chain(v.iter().flatten()) {
            b.extend(c.to_le_bytes());
        }
        b.extend(attr.to_le_bytes());
    }
    b
}

/// The unit square in the XY plane, facing +Z, as two facets.
const SQUARE: [[[f32; 3]; 3]; 2] = [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]], [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]];

fn parse(bytes: &[u8]) -> crate::Result<Scene> {
    read(bytes, &Limits::default())
}

#[test]
fn binary_is_faceted_and_keeps_the_files_axes() {
    // A header that begins with `solid`, as many binary exporters write it.
    let bytes = binary_stl(b"solid part", &[([0.0, 0.0, 1.0], SQUARE[0], 0), ([0.0, 0.0, 1.0], SQUARE[1], 0)]);
    let s = parse(&bytes).unwrap();
    s.validate().unwrap();
    let m = &s.meshes[0];
    assert_eq!(m.name, "part");
    assert!(s.materials.is_empty() && s.warnings.is_empty());
    assert_eq!((m.triangle_count(), m.positions.len(), m.parts.len()), (2, 6, 1));
    assert!(m.uv_sets.is_empty());
    assert_eq!(m.normals, vec![[0.0, 0.0, 1.0]; 2]);
    assert_eq!(m.positions[2], [1.0, 1.0, 0.0]);
    // One root node places the mesh.
    assert_eq!((s.roots.as_slice(), s.nodes[0].mesh, s.nodes[0].transform), (&[0][..], Some(0), IDENTITY));
    // Trailing bytes after the last facet are ignored.
    let mut longer = bytes.clone();
    longer.extend([0u8; 7]);
    assert_eq!(parse(&longer).unwrap().meshes, s.meshes);
}

#[test]
fn ascii_with_polygon_loops_and_several_solids() {
    let src = "solid Bracket\n  facet normal 0 0 1\n    outer loop\n      vertex 0 0 0\n      vertex 1 0 0\n      vertex 1 1 0\n      vertex 0 1 0\n    endloop\n  endfacet\nendsolid Bracket\n\
               solid second\nfacet normal 0 0 0\nouter loop\nvertex 0 0 1\nvertex 1 0 1\nvertex 1E0 1 1\nendloop\nendfacet\nendsolid\n";
    let s = parse(src.as_bytes()).unwrap();
    assert_eq!(s.meshes[0].name, "Bracket");
    // The quad fans into two triangles; the second solid adds one.
    assert_eq!(s.meshes[0].triangle_count(), 3);
    assert!(parse(b"solid x\nfacet normal 0 0 1\nouter loop\nvertex 0 zero 0\n").is_err());
    // A name that is not UTF-8, and a facet with a non-finite vertex, which is left out.
    let mut ascii = b"solid caf\xe9\n".to_vec();
    ascii.extend(b"facet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\n");
    ascii.extend(b"facet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex nan 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid\n");
    let s = parse(&ascii).unwrap();
    assert_eq!((s.meshes[0].triangle_count(), s.warnings.len()), (1, 1));
    // ASCII keeps the precision of its text.
    let s = parse(b"solid p\nfacet normal 0 0 1\nouter loop\nvertex 0.1 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid\n").unwrap();
    assert_eq!(s.meshes[0].positions[0][0], 0.1f64);
}

#[test]
fn binary_colours() {
    // VisCAM / SolidView: bit 15 set, red in bits 10–14.
    let red = 0x8000 | (31 << 10);
    let s = parse(&binary_stl(b"", &[([0.0; 3], SQUARE[0], red), ([0.0; 3], SQUARE[1], 0)])).unwrap();
    s.validate().unwrap();
    assert_eq!((s.meshes[0].parts.len(), s.materials.len()), (2, 2));
    assert!(s.materials[0].base_color.iter().zip([1.0, 0.0, 0.0, 1.0]).all(|(x, y)| (x - y).abs() < 1e-5));
    assert_eq!((s.materials[0].name.as_str(), s.materials[0].metallic), ("#FF0000", 0.0));
    assert_eq!(s.meshes[0].parts[1].material, Some(1));
    // Materialise Magics: a part colour in the header; bit 15 clear = the facet's own colour
    // with red in bits 0–4.
    let mut header = b"COLOR=".to_vec();
    header.extend([0, 255, 0, 255]);
    let s = parse(&binary_stl(&header, &[([0.0; 3], SQUARE[0], 31), ([0.0; 3], SQUARE[1], 0x8000)])).unwrap();
    let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
    assert!(close(s.materials[0].base_color, [1.0, 0.0, 0.0, 1.0]), "{:?}", s.materials[0]);
    assert!(close(s.materials[1].base_color, [0.0, 1.0, 0.0, 1.0]), "{:?}", s.materials[1]);
}

#[test]
fn quirks_and_hostile_input() {
    // Claims four billion facets but has one: truncated.
    let mut bytes = binary_stl(b"", &[([0.0; 3], SQUARE[0], 0)]);
    bytes.splice(80..84, u32::MAX.to_le_bytes());
    assert!(parse(&bytes).is_err());
    // A count left at 0 over a file with facets: they are read, with a warning.
    bytes.splice(80..84, 0u32.to_le_bytes());
    let s = parse(&bytes).unwrap();
    assert_eq!((s.meshes[0].triangle_count(), s.warnings.len()), (1, 1));
    assert!(parse(b"").is_err());
    assert!(parse(&[0u8; 83]).is_err());
    assert!(parse(b"solid").is_err());
    assert!(parse(&[0xff; 200]).is_err());
    // No facets, or only non-finite ones.
    assert!(parse(&binary_stl(b"", &[])).is_err());
    assert!(parse(&binary_stl(b"", &[([0.0; 3], [[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], 0)])).is_err());
    assert!(parse(b"solid empty\nendsolid empty\n").is_err());
    // A non-finite facet among good ones is left out, with a warning.
    let s =
        parse(&binary_stl(b"", &[([0.0; 3], SQUARE[0], 0), ([0.0; 3], [[f32::INFINITY, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], 0)])).unwrap();
    assert_eq!((s.meshes[0].triangle_count(), s.warnings.len()), (1, 1));
    // A degenerate facet keeps its stored normal; a garbage stored normal loses to the winding.
    let s = parse(&binary_stl(b"", &[([1.0, 0.0, 0.0], [[0.0; 3]; 3], 0), ([f32::NAN, 9.0, -7.0], SQUARE[0], 0)])).unwrap();
    assert_eq!(s.meshes[0].normals, [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
    hammer(&binary_stl(b"solid x", &[([0.0; 3], SQUARE[0], 0x8000 | 31), ([0.0; 3], SQUARE[1], 0)]), &parse);
    hammer(b"solid a\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid a\n", &parse);
}

#[test]
fn limits() {
    let two = binary_stl(b"", &[([0.0; 3], SQUARE[0], 0), ([0.0; 3], SQUARE[1], 0)]);
    let ascii = write_ascii(&parse(&two).unwrap(), "sq", &Limits::default()).unwrap();
    for bytes in [two.as_slice(), ascii.as_bytes()] {
        assert!(read(bytes, &Limits { max_triangles: 2, ..Limits::default() }).is_ok());
        assert!(matches!(read(bytes, &Limits { max_triangles: 1, ..Limits::default() }), Err(Error::TooLarge(_))));
        assert!(read(bytes, &Limits { max_vertices: 6, ..Limits::default() }).is_ok());
        assert!(matches!(read(bytes, &Limits { max_vertices: 5, ..Limits::default() }), Err(Error::TooLarge(_))));
    }
    let quad = "solid q\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 1 1 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid\n";
    assert!(read(quad.as_bytes(), &Limits { max_face_corners: 4, ..Limits::default() }).is_ok());
    assert!(matches!(read(quad.as_bytes(), &Limits { max_face_corners: 3, ..Limits::default() }), Err(Error::TooLarge(_))));
}

#[test]
fn written_files_read_back() {
    let mut scene = Scene::from_meshes(vec![super::tri_mesh()], vec![]);
    // The node's placement is applied: STL has no hierarchy.
    scene.nodes[0].transform[12] = 10.0;
    let limits = Limits::default();
    let binary = write_binary(&scene, "solid tri", &limits).unwrap();
    assert_eq!(binary.len(), 84 + 2 * 50);
    assert!(!binary.starts_with(b"solid"), "a binary header that starts with `solid` confuses readers");
    let ascii = write_ascii(&scene, "tri one", &limits).unwrap();
    assert!(ascii.starts_with("solid trione\n") && ascii.ends_with("endsolid trione\n"));
    for bytes in [binary.as_slice(), ascii.as_bytes()] {
        let back = parse(bytes).unwrap();
        let m = &back.meshes[0];
        assert_eq!(m.triangle_count(), 2);
        assert_eq!(m.positions, [[10.0, 0.0, 0.0], [11.0, 0.0, 0.0], [10.0, 1.0, 0.0], [10.0, 0.0, 0.0], [10.0, 0.0, 1.0], [11.0, 0.0, 0.0]]);
        assert_eq!(m.normals, [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0]]);
    }
    // A scene that cannot be walked or indexed is an error.
    let mut bad = scene.clone();
    bad.meshes[0].parts[0].corners[0].position = 99;
    assert!(write_binary(&bad, "x", &limits).is_err());
    bad.nodes[0].children = vec![0];
    assert!(write_ascii(&bad, "x", &limits).is_err());
}
