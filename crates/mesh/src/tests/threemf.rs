//! 3MF: a package with every property kind, placements, limits and hostile files.

use super::hammer;
use crate::threemf::{Model, Unit, is_zip, read, read_model, read_model_part};
use crate::{AlphaMode, Corner, Error, IDENTITY, Limits, NONE, Scene, Wrap, no_external, transform_point};

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"#;

const TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>"#;

/// Object 1: a square of two triangles, one in a base material and one textured. Object 2: a
/// triangle with a colour per corner and one without a property. Object 3 places 1 and 2.
/// The build places 3 (moved) and 1 again.
const MODEL: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<model unit="inch" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:m="http://schemas.microsoft.com/3dmanufacturing/material/2015/02">
 <resources>
  <basematerials id="10">
   <base name="Red" displaycolor="#FF0000"/>
   <base name="Glass" displaycolor="#0000FF40"/>
  </basematerials>
  <m:colorgroup id="20">
   <m:color color="#FF0000"/>
   <m:color color="#00FF0080"/>
  </m:colorgroup>
  <m:texture2d id="30" path="/3D/Textures/wood.png" contenttype="image/png" tilestyleu="mirror" tilestylev="clamp"/>
  <m:texture2dgroup id="40" texid="30">
   <m:tex2coord u="0" v="0"/>
   <m:tex2coord u="1" v="0.25"/>
   <m:tex2coord u="1" v="1"/>
  </m:texture2dgroup>
  <object id="1" name="Square" type="model" pid="10" pindex="1">
   <mesh>
    <vertices>
     <vertex x="0" y="0" z="0"/>
     <vertex x="1" y="0" z="0"/>
     <vertex x="1" y="1" z="0"/>
     <vertex x="0" y="1" z="0"/>
    </vertices>
    <triangles>
     <triangle v1="0" v2="1" v3="2"/>
     <triangle v1="0" v2="2" v3="3" pid="40" p1="0" p2="2" p3="1"/>
     <triangle v1="0" v2="2" v3="9"/>
    </triangles>
   </mesh>
  </object>
  <object id="2" name="Tri">
   <mesh>
    <vertices>
     <vertex x="0" y="0" z="1"/>
     <vertex x="1" y="0" z="1"/>
     <vertex x="0" y="1" z="1"/>
    </vertices>
    <triangles>
     <triangle v1="0" v2="1" v3="2" pid="20" p1="0" p2="1" p3="0"/>
     <triangle v1="2" v2="1" v3="0" pid="20" p1="1"/>
     <triangle v1="0" v2="1" v3="2"/>
    </triangles>
   </mesh>
  </object>
  <object id="3" name="Pair">
   <components>
    <component objectid="1"/>
    <component objectid="2" transform="2 0 0 0 2 0 0 0 2 0 0 5"/>
    <component objectid="77"/>
   </components>
  </object>
 </resources>
 <build>
  <item objectid="3" transform="1 0 0 0 1 0 0 0 1 10 20 30"/>
  <item objectid="1"/>
 </build>
</model>"##;

fn package(model: &str) -> Vec<u8> {
    let part = |name: &str, data: &[u8]| (name.to_string(), data.to_vec());
    craft_opc::zip_entries(&[
        part("[Content_Types].xml", TYPES.as_bytes()),
        part("_rels/.rels", RELS.as_bytes()),
        part("3D/3dmodel.model", model.as_bytes()),
        part("3D/Textures/wood.png", b"wood image"),
    ])
    .unwrap()
}

fn parse(bytes: &[u8]) -> crate::Result<Scene> {
    read(bytes, &Limits::default())
}

fn part(model: &str, limits: &Limits) -> crate::Result<Model> {
    read_model_part(model.as_bytes(), limits, &mut no_external)
}

#[test]
fn a_package_with_every_property_kind() {
    let bytes = package(MODEL);
    assert!(is_zip(&bytes));
    let Model { scene: s, unit } = read_model(&bytes, &Limits::default()).unwrap();
    s.validate().unwrap();
    assert_eq!((unit, unit.metres()), (Unit::Inch, 0.0254));

    // Placements: the build's items are roots, components their children; one mesh per object.
    let names: Vec<&str> = s.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["Pair", "Square", "Tri", "Square"]);
    assert_eq!((s.roots.as_slice(), s.nodes[0].children.as_slice()), (&[0, 3][..], &[1, 2][..]));
    assert_eq!(s.nodes.iter().map(|n| n.mesh).collect::<Vec<_>>(), [None, Some(0), Some(1), Some(0)]);
    assert_eq!(s.meshes.len(), 2);
    let placed = s.instances(&Limits::default()).unwrap();
    assert_eq!(placed.iter().map(|p| p.0).collect::<Vec<_>>(), [0, 1, 0]);
    // The item's move, then the component's scale and lift (a row vector times the matrix).
    assert_eq!(transform_point(&placed[1].1, [1.0, 0.0, 1.0]), [12.0, 20.0, 37.0]);
    assert_eq!(placed[2].1, IDENTITY);

    // The square: the object's default base material, then a textured triangle. The triangle
    // that names vertex 9 is left out. Positions are the file's vertices.
    let square = &s.meshes[0];
    assert_eq!((square.name.as_str(), square.positions.len(), square.colors.len(), square.parts.len()), ("Square", 4, 0, 2));
    let glass = &s.materials[square.parts[0].material.unwrap()];
    assert_eq!((glass.name.as_str(), glass.base_color, glass.alpha_mode), ("Glass", [0.0, 0.0, 1.0, 64.0 / 255.0], AlphaMode::Blend));
    assert_eq!(square.parts[0].corners, [0, 1, 2].map(Corner::at));
    // Coordinates through p1 p2 p3, v flipped; the pool holds only what is used, once.
    assert_eq!(square.parts[1].corners, [(0, 0), (2, 1), (3, 2)].map(|(position, uv)| Corner { position, normal: NONE, uv }));
    assert_eq!(square.uv_sets, [vec![[0.0, 1.0], [1.0, 0.0], [1.0, 0.75]]]);
    let wood = &s.materials[square.parts[1].material.unwrap()];
    let texture = wood.base_color_texture.unwrap();
    assert_eq!((texture.wrap, wood.base_color), ([Wrap::Mirror, Wrap::Clamp], [1.0; 4]));
    let image = &s.images[texture.image];
    assert_eq!(
        (image.uri.as_deref(), image.mime_type.as_deref(), image.data.as_deref()),
        (Some("/3D/Textures/wood.png"), Some("image/png"), Some(&b"wood image"[..]))
    );

    // The triangle: a colour per corner makes a position per vertex and colour; a triangle
    // without a property is white.
    let tri = &s.meshes[1];
    let (red, green, white) = ([1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 128.0 / 255.0], [1.0; 4]);
    assert_eq!(tri.colors, [red, green, red, green, green, white, white, white]);
    assert_eq!((tri.positions[0], tri.positions[2]), (tri.positions[4], tri.positions[3]));
    assert_eq!(tri.parts.len(), 2);
    assert_eq!(tri.parts[0].corners, [0, 1, 2, 3, 1, 4].map(Corner::at));
    assert_eq!((tri.parts[1].material, tri.parts[1].corners.len()), (None, 3));
    assert_eq!(s.materials[tri.parts[0].material.unwrap()].base_color, [1.0; 4]);

    assert_eq!(s.warnings.len(), 2, "{:?}", s.warnings);
    assert!(s.warnings[0].starts_with("1 triangle") && s.warnings[1].starts_with("1 build item"));
}

#[test]
fn the_model_part_alone_and_what_is_not_read() {
    // No package: textures are asked of the caller, who may decline.
    let m = part(MODEL, &Limits::default()).unwrap();
    assert_eq!((m.scene.images[0].data.as_ref(), m.scene.external.as_slice()), (None, &["/3D/Textures/wood.png".to_string()][..]));
    // A model without build items places every object once; the default unit is the millimetre.
    let bare = "<model><resources><object id='5'><mesh><vertices><vertex x='0' y='0' z='0'/><vertex x='1' y='0' z='0'/><vertex x='0' y='1' z='0'/></vertices>\
                <triangles><triangle v1='0' v2='1' v3='2'/></triangles></mesh></object><object id='6'><components><component objectid='5' path='/3D/other.model'/>\
                <component objectid='5' transform='1 2 3'/></components></object><m:compositematerials id='9' matid='1' matindices='0 1'/></resources></model>";
    let m = part(bare, &Limits::default()).unwrap();
    m.scene.validate().unwrap();
    assert_eq!((m.unit, m.scene.roots.len(), m.scene.nodes.len()), (Unit::Millimeter, 2, 3));
    // A transform that is not twelve numbers is the identity.
    assert_eq!(m.scene.nodes[2].transform, IDENTITY);
    assert_eq!(m.scene.warnings.len(), 2, "{:?}", m.scene.warnings);
    // A package without the relationship still finds its model part; one without a model part,
    // and XML that is not a model, are refused.
    let entry = |name: &str, data: &str| (name.to_string(), data.as_bytes().to_vec());
    let loose = craft_opc::zip_entries(&[entry("Model/thing.model", bare)]).unwrap();
    assert_eq!(parse(&loose).unwrap().meshes.len(), 1);
    assert!(matches!(parse(&craft_opc::zip_entries(&[entry("readme.txt", "hello")]).unwrap()), Err(Error::Malformed(_))));
    assert!(matches!(part("<svg/>", &Limits::default()), Err(Error::Malformed(_))));
    assert!(matches!(part("", &Limits::default()), Err(Error::Malformed(_))));
    assert!(matches!(parse(b"not a zip"), Err(Error::Malformed(_))));
}

#[test]
fn every_limit_is_kept() {
    let d = Limits::default;
    let refused = |limits: Limits| matches!(read(&package(MODEL), &limits), Err(Error::TooLarge(_)));
    // 7 vertices, 2 colours and 3 coordinates; 6 triangles; 3 objects; 4 parts; 2 base
    // materials and a texture; 4 placed objects (5 components and items); components 1 deep.
    for (at, under) in [
        (Limits { max_vertices: 12, ..d() }, Limits { max_vertices: 11, ..d() }),
        (Limits { max_triangles: 6, ..d() }, Limits { max_triangles: 5, ..d() }),
        (Limits { max_meshes: 3, ..d() }, Limits { max_meshes: 2, ..d() }),
        (Limits { max_parts: 4, ..d() }, Limits { max_parts: 3, ..d() }),
        (Limits { max_materials: 3, ..d() }, Limits { max_materials: 2, ..d() }),
        (Limits { max_nodes: 4, ..d() }, Limits { max_nodes: 3, ..d() }),
        (Limits { max_depth: 1, ..d() }, Limits { max_depth: 0, ..d() }),
    ] {
        assert!(!refused(at.clone()), "{at:?}");
        assert!(refused(under.clone()), "{under:?}");
    }
    // The package may not inflate past the data limit, and a texture counts against it.
    assert!(refused(Limits { max_data_bytes: MODEL.len() - 1, ..d() }));
    let texture = |max_data_bytes: usize| read_model_part(MODEL.as_bytes(), &Limits { max_data_bytes, ..d() }, &mut |_: &str| Some(vec![0; 100]));
    assert!(texture(100).is_ok());
    assert!(matches!(texture(99), Err(Error::TooLarge(_))));
    // An object that is its own component ends at the depth limit instead of going round.
    let cycle = "<model><resources><object id='1'><components><component objectid='1'/></components></object></resources><build><item objectid='1'/></build></model>";
    assert!(matches!(part(cycle, &d()), Err(Error::TooLarge(_))));
    // Two objects placing each other twice double at every level: the node limit ends it.
    let fan = "<model><resources><object id='1'><components><component objectid='2'/><component objectid='2'/></components></object>\
               <object id='2'><components><component objectid='1'/><component objectid='1'/></components></object></resources><build><item objectid='1'/></build></model>";
    assert!(matches!(part(fan, &Limits { max_nodes: 1000, max_depth: 64, ..d() }), Err(Error::TooLarge(_))));
}

#[test]
fn hostile_files_fail_cleanly() {
    hammer(MODEL.as_bytes(), &|b| read_model_part(b, &Limits::default(), &mut no_external).map(|m| m.scene));
    // The package: every 7th truncation and mutation (inflating each is slow).
    let good = package(MODEL);
    for i in (0..good.len()).step_by(7) {
        let mut b = good.clone();
        b[i] ^= 0xA5;
        for bytes in [&good[..i], &b[..]] {
            if let Ok(s) = parse(bytes) {
                s.validate().unwrap();
            }
        }
    }
}
