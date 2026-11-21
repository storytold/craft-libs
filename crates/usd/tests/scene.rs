//! Known-answer USDA and USDC (Crate binary) corpus and round-trip integration tests.

#![allow(clippy::unwrap_used)]

use craft_usd::codec;
use craft_usd::scene::{
    Budget, MAX_DEPTH, SceneError, Specifier, UsdAttr, UsdLayer, UsdPrim, UsdValue, Val, read_layer, read_layer_with_budget, write_usda, write_usdc,
};

const CORPUS_USDA: &str = r#"#usda 1.0
(
    defaultPrim = "World"
    doc = "Corpus test"
    metersPerUnit = 0.01
    upAxis = "Z"
    subLayers = [@./base.usda@]
)

class Xform "Proto"
{
    def Mesh "Leaf" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    )
    {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (100, 0, 0), (0, 100, 0)]
        normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1)] (
            interpolation = "vertex"
        )
        texCoord2f[] primvars:st = [(0, 0), (1, 0), (0, 1)] (
            interpolation = "vertex"
        )
        rel material:binding = </World/Materials/LeafMat>
    }
}

def Xform "World"
{
    double3 xformOp:translate = (0, 0, 100)
    float3 xformOp:rotateXYZ = (0, 0, 90)
    float3 xformOp:scale = (2, 2, 2)
    uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ", "xformOp:scale"]

    def Xform "Instance" (
        prepend references = </Proto>
    )
    {
        matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (10, 0, 0, 1) )
        uniform token[] xformOpOrder = ["xformOp:transform"]
    }

    over "Overlay" (
        active = false
    )
    {
        float3 xformOp:translate.timeSamples = {
            0: (1, 2, 3),
            10: (4, 5, 6),
        }
    }

    def Xform "External" (
        prepend references = @./other.usda@
    )
    {
        variantSet "look" = {
            "a" {
                def Mesh "V" {}
            }
        }
    }
}
"#;

#[test]
fn scalar_external_arcs_keep_following_metadata_and_prims() {
    // Original generated layers: external targets must never resolve to the local class.
    for key in ["references", "payload"] {
        for arc in [
            "@external.usda@",
            "@external.usda@</LocalTemplate>",
            "@external.usda@ (offset = 10; scale = 2)",
            "@external.usda@</LocalTemplate> (offset = 10; scale = 2)",
        ] {
            let source = format!(
                r#"#usda 1.0
class Xform "LocalTemplate" {{}}
def Xform "External" (
    prepend {key} = {arc}
    active = false
) {{}}
def Xform "Mixed" (
    prepend {key} = [@external.usda@</LocalTemplate>, </LocalTemplate>]
) {{}}
def Xform "Later" {{}}
"#
            );
            let layer = read_layer(source.as_bytes()).unwrap_or_else(|error| panic!("{key} = {arc}: {error}"));
            let external = layer.prims.iter().find(|p| p.path == "/External").unwrap();
            assert!(external.references.is_empty(), "{key} = {arc}");
            assert_eq!(external.active, Some(false), "{key} = {arc}");
            let mixed = layer.prims.iter().find(|p| p.path == "/Mixed").unwrap();
            assert_eq!(mixed.references, ["/LocalTemplate"], "{key} = {arc}");
            assert!(layer.prims.iter().any(|p| p.path == "/Later"), "{key} = {arc}");
            assert!(layer.warnings.iter().any(|w| w.contains("external references and payloads")), "{key} = {arc}");
        }
    }
}

#[test]
fn usda_corpus_overlays_references_coordinates_transforms_and_meshes() {
    let layer = read_layer(CORPUS_USDA.as_bytes()).unwrap();
    assert_eq!(layer.meters_per_unit, Some(0.01));
    assert_eq!(layer.up_axis.as_deref(), Some("Z"));
    assert!(layer.warnings.iter().any(|w| w.contains("sublayers")));
    assert!(layer.warnings.iter().any(|w| w.contains("external references")));
    assert!(layer.warnings.iter().any(|w| w.contains("variant sets")));

    let proto = layer.prims.iter().find(|p| p.path == "/Proto").unwrap();
    assert_eq!(proto.specifier, Specifier::Class);
    assert_eq!(proto.type_name, "Xform");

    let leaf = layer.prims.iter().find(|p| p.path == "/Proto/Leaf").unwrap();
    assert_eq!(leaf.specifier, Specifier::Def);
    assert_eq!(leaf.type_name, "Mesh");
    assert_eq!(leaf.props.get("faceVertexCounts").and_then(|p| p.value.as_ref()), Some(&Val::Num(vec![3.0])));
    assert_eq!(leaf.props.get("faceVertexIndices").and_then(|p| p.value.as_ref()), Some(&Val::Num(vec![0.0, 1.0, 2.0])));
    assert_eq!(leaf.props.get("points").and_then(|p| p.value.as_ref()), Some(&Val::Num(vec![0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, 100.0, 0.0])));
    assert_eq!(leaf.props.get("normals").and_then(|p| p.interpolation.as_deref()), Some("vertex"));
    assert_eq!(leaf.props.get("primvars:st").and_then(|p| p.interpolation.as_deref()), Some("vertex"));
    assert_eq!(leaf.props.get("material:binding").and_then(|p| p.value.as_ref()), Some(&Val::Path(vec!["/World/Materials/LeafMat".into()])));

    let world = layer.prims.iter().find(|p| p.path == "/World").unwrap();
    assert_eq!(world.specifier, Specifier::Def);
    assert_eq!(world.props.get("xformOp:translate").and_then(|p| p.value.as_ref()), Some(&Val::Num(vec![0.0, 0.0, 100.0])));
    assert_eq!(
        world.props.get("xformOpOrder").and_then(|p| p.value.as_ref()),
        Some(&Val::Tok(vec!["xformOp:translate".into(), "xformOp:rotateXYZ".into(), "xformOp:scale".into()]))
    );

    let instance = layer.prims.iter().find(|p| p.path == "/World/Instance").unwrap();
    assert_eq!(instance.references, vec!["/Proto"]);
    assert_eq!(
        instance.props.get("xformOp:transform").and_then(|p| p.value.as_ref()),
        Some(&Val::Num(vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 10.0, 0.0, 0.0, 1.0]))
    );

    let overlay = layer.prims.iter().find(|p| p.path == "/World/Overlay").unwrap();
    assert_eq!(overlay.specifier, Specifier::Over);
    assert_eq!(overlay.active, Some(false));
    assert_eq!(overlay.props.get("xformOp:translate").and_then(|p| p.value.as_ref()), Some(&Val::Num(vec![1.0, 2.0, 3.0])));
}

fn sample_scene_layer() -> UsdLayer {
    let mut root = UsdPrim::new("Scene", "Xform");
    let mut mesh = UsdPrim::new("QuadMesh", "Mesh");
    mesh.binding_api = true;
    mesh.attrs = vec![
        UsdAttr::new("extent", "float3[]", Some(UsdValue::Vec3fs(vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]]))),
        UsdAttr::new("points", "point3f[]", Some(UsdValue::Vec3fs(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]))),
        UsdAttr::new("faceVertexCounts", "int[]", Some(UsdValue::Ints(vec![3, 3]))),
        UsdAttr::new("faceVertexIndices", "int[]", Some(UsdValue::Ints(vec![0, 1, 2, 0, 2, 3]))),
        UsdAttr::new("normals", "normal3f[]", Some(UsdValue::Vec3fs(vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]])))
            .with_interpolation("vertex"),
        UsdAttr::new("primvars:st", "texCoord2f[]", Some(UsdValue::Vec2fs(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])))
            .with_interpolation("vertex"),
        UsdAttr::new("subdivisionScheme", "token", Some(UsdValue::Token("none".into()))).uniform(),
        UsdAttr::rel("material:binding", "/Scene/Materials/Default".into()),
    ];

    let mut subset = UsdPrim::new("Sub0", "GeomSubset");
    subset.binding_api = true;
    subset.attrs = vec![
        UsdAttr::new("elementType", "token", Some(UsdValue::Token("face".into()))).uniform(),
        UsdAttr::new("familyName", "token", Some(UsdValue::Token("materialBind".into()))).uniform(),
        UsdAttr::new("indices", "int[]", Some(UsdValue::Ints(vec![1]))),
        UsdAttr::rel("material:binding", "/Scene/Materials/Accent".into()),
    ];
    mesh.children.push(subset);
    root.children.push(mesh);

    let mut materials = UsdPrim::new("Materials", "Scope");
    for mat_name in ["Default", "Accent"] {
        let mut mat = UsdPrim::new(mat_name, "Material");
        mat.attrs.push(UsdAttr::connected("outputs:surface", "token", format!("/Scene/Materials/{mat_name}/PBR.outputs:surface")));
        let mut pbr = UsdPrim::new("PBR", "Shader");
        pbr.attrs = vec![
            UsdAttr::new("info:id", "token", Some(UsdValue::Token("UsdPreviewSurface".into()))).uniform(),
            UsdAttr::new("inputs:useSpecularWorkflow", "int", Some(UsdValue::Int(0))),
            UsdAttr::new("inputs:scale", "float4", Some(UsdValue::Float4([2.0, 2.0, 2.0, 1.0]))),
            UsdAttr::new("inputs:file", "asset", Some(UsdValue::Asset("./tex.png".into()))),
            UsdAttr::new("inputs:varname", "string", Some(UsdValue::Str("st".into()))),
            UsdAttr::new("outputs:surface", "token", None),
        ];
        mat.children.push(pbr);
        materials.children.push(mat);
    }
    root.children.push(materials);

    let mut layer = UsdLayer::new(root);
    layer.doc = Some("Integration round-trip".into());
    layer.meters_per_unit = 0.01;
    layer.up_axis = "Z".into();
    layer
}

#[test]
fn usda_and_usdc_round_trip_known_answers() {
    let scene = sample_scene_layer();
    let text = write_usda(&scene);
    assert!(text.starts_with("#usda 1.0"));
    assert!(text.contains("defaultPrim = \"Scene\""));
    assert!(text.contains("doc = \"Integration round-trip\""));
    assert!(text.contains("metersPerUnit = 0.01"));
    assert!(text.contains("upAxis = \"Z\""));

    let crate_bytes = write_usdc(&scene).unwrap();
    assert!(crate_bytes.starts_with(codec::MAGIC));
    assert_eq!(&crate_bytes[8..11], &[0, 8, 0]);

    for (label, bytes) in [("usda", text.as_bytes()), ("usdc", crate_bytes.as_slice())] {
        let parsed = read_layer(bytes).unwrap();
        assert_eq!(parsed.meters_per_unit, Some(0.01), "{label}");
        assert_eq!(parsed.up_axis.as_deref(), Some("Z"), "{label}");

        let mesh = parsed.prims.iter().find(|p| p.path == "/Scene/QuadMesh").unwrap();
        assert_eq!(mesh.type_name, "Mesh", "{label}");
        assert_eq!(mesh.specifier, Specifier::Def, "{label}");
        assert_eq!(mesh.props.get("faceVertexCounts").and_then(|p| p.value.as_ref()), Some(&Val::Num(vec![3.0, 3.0])), "{label}");
        assert_eq!(
            mesh.props.get("faceVertexIndices").and_then(|p| p.value.as_ref()),
            Some(&Val::Num(vec![0.0, 1.0, 2.0, 0.0, 2.0, 3.0])),
            "{label}"
        );
        assert_eq!(mesh.props.get("primvars:st").and_then(|p| p.interpolation.as_deref()), Some("vertex"), "{label}");
        assert_eq!(
            mesh.props.get("material:binding").and_then(|p| p.value.as_ref()),
            Some(&Val::Path(vec!["/Scene/Materials/Default".into()])),
            "{label}"
        );

        let subset = parsed.prims.iter().find(|p| p.path == "/Scene/QuadMesh/Sub0").unwrap();
        assert_eq!(subset.type_name, "GeomSubset", "{label}");
        assert_eq!(subset.props.get("indices").and_then(|p| p.value.as_ref()), Some(&Val::Num(vec![1.0])), "{label}");

        let pbr = parsed.prims.iter().find(|p| p.path == "/Scene/Materials/Default/PBR").unwrap();
        assert_eq!(pbr.props.get("info:id").and_then(|p| p.value.as_ref()), Some(&Val::Tok(vec!["UsdPreviewSurface".into()])), "{label}");
        assert_eq!(pbr.props.get("inputs:file").and_then(|p| p.value.as_ref()), Some(&Val::Tok(vec!["./tex.png".into()])), "{label}");
        assert_eq!(pbr.props.get("inputs:varname").and_then(|p| p.value.as_ref()), Some(&Val::Tok(vec!["st".into()])), "{label}");
        assert_eq!(pbr.props.get("inputs:scale").and_then(|p| p.value.as_ref()), Some(&Val::Num(vec![2.0, 2.0, 2.0, 1.0])), "{label}");
    }
}

#[test]
fn decoded_budget_and_invalid_inputs_are_enforced() {
    let scene = sample_scene_layer();
    let crate_bytes = write_usdc(&scene).unwrap();
    let mut tiny_budget = Budget::with_limit(64);
    assert!(matches!(read_layer_with_budget(&crate_bytes, &mut tiny_budget), Err(SceneError::TooLarge(_))));

    assert!(matches!(read_layer(b"not a usd file"), Err(SceneError::Malformed(_))));
    assert!(matches!(read_layer(b"PXR-USDC\x00\x03\x00\x00"), Err(SceneError::Unsupported(_))));

    let mut bad_target = UsdLayer::new(UsdPrim::new("Root", "Xform"));
    bad_target.root.attrs.push(UsdAttr::rel("material:binding", "/Does/Not/Exist".into()));
    assert!(matches!(write_usdc(&bad_target), Err(SceneError::Invalid(_))));
    assert_eq!(MAX_DEPTH, 128);
}
