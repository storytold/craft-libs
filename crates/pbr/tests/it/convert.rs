//! Conversions into a `Surface`: glTF 2.0 materials from craft-mesh and MaterialX shaders from
//! craft-materialx, checked against the defaults and ranges the public definitions give.

use craft_materialx::{Document, Resolved as Value, Shader, StrictMaterial as Mx, Texture, write_open_pbr};
use craft_mesh::{AlphaMode as MeshAlpha, Material as Mesh, TextureRef, UvTransform as MeshUv, Wrap as MeshWrap};
use craft_pbr::{
    AlphaMode, Clearcoat, Conversion, ConvertError, ReportItem, Sheen, TextureEncoding, TextureInput, TextureSource, UvTransform, Wrap,
    try_from_materialx, try_from_mesh,
};

fn close(a: f32, b: f32) -> bool {
    a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn close_is_false_for_values_that_are_not_finite() {
    assert!(close(1.0, 1.0 + 1e-7));
    assert!(!close(f32::INFINITY, 1.0) && !close(f32::INFINITY, f32::INFINITY) && !close(f32::NAN, 1.0));
}

#[track_caller]
fn assert_close3(a: [f32; 3], b: [f32; 3]) {
    assert!(a.iter().zip(b).all(|(x, y)| close(*x, y)), "{a:?} against {b:?}");
}

fn unshaded(c: &Conversion, input: &str) -> bool {
    c.report.items.iter().any(|i| matches!(i, ReportItem::Unshaded { input: n } if n == input))
}

fn approximated(c: &Conversion, input: &str) -> bool {
    c.report.items.iter().any(|i| matches!(i, ReportItem::Approximated { input: n, .. } if n == input))
}

fn dropped(c: &Conversion, input: &str) -> bool {
    c.report.items.iter().any(|i| matches!(i, ReportItem::Dropped { input: n, .. } if n == input))
}

#[track_caller]
fn bad_value(r: Result<Conversion, ConvertError>, input: &str) {
    match r {
        Err(ConvertError::BadValue { input: n, .. }) => assert!(n.contains(input), "{n} is not {input}"),
        other => panic!("{input}: expected a bad value, got {other:?}"),
    }
}

fn tex(image: usize) -> Option<TextureRef> {
    Some(TextureRef { image, ..TextureRef::default() })
}

// ------------------------------------------------------------------ craft-mesh (glTF 2.0)

#[test]
fn mesh_factors_map_as_they_are() {
    let m = Mesh {
        name: "Paint".into(),
        base_color: [0.25, 0.5, 0.75, 0.6],
        metallic: 0.3,
        roughness: 0.7,
        emissive: [2.0, 0.5, 0.0],
        alpha_mode: MeshAlpha::Mask(0.4),
        double_sided: true,
        ..Mesh::default()
    };
    let c = try_from_mesh(&m).unwrap();
    let s = &c.surface;
    assert_eq!(s.name, "Paint");
    assert_eq!(s.material.base_color(), [0.25, 0.5, 0.75]);
    assert_eq!((s.material.metallic(), s.material.roughness()), (0.3, 0.7));
    assert_eq!((s.material.specular(), s.material.diffuse()), (1.0, 1.0));
    assert_eq!(s.alpha, 0.6);
    assert_eq!(s.alpha_mode, AlphaMode::Mask(0.4));
    assert_eq!(s.emissive, [2.0, 0.5, 0.0]);
    assert!(s.double_sided && !s.unlit);
    assert_eq!((s.clearcoat, s.sheen), (None, None));
    assert!(s.textures.is_empty() && s.other_inputs.is_empty());
    assert!(c.report.is_empty(), "{:?}", c.report);
}

#[test]
fn mesh_defaults_are_the_gltf_defaults() {
    // glTF 2.0 gives baseColorFactor [1, 1, 1, 1], metallicFactor 1, roughnessFactor 1,
    // emissiveFactor [0, 0, 0] and alphaMode OPAQUE
    // (https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#reference-material).
    let c = try_from_mesh(&Mesh::default()).unwrap();
    let s = &c.surface;
    assert_eq!(s.material.base_color(), [1.0; 3]);
    assert_eq!((s.material.metallic(), s.material.roughness(), s.alpha), (1.0, 1.0, 1.0));
    assert_eq!((s.emissive, s.alpha_mode), ([0.0; 3], AlphaMode::Opaque));
    assert!(!s.double_sided);
    assert!(c.report.is_empty());
}

#[test]
fn mesh_extensions_that_are_not_converted_are_reported_as_dropped() {
    let mut m = Mesh::default();
    for name in ["KHR_materials_transmission", "KHR_materials_clearcoat", "KHR_materials_unlit", "KHR_materials_emissive_strength"] {
        m.extensions.insert(name.into(), craft_mesh::serde_json::json!({}));
    }
    let c = try_from_mesh(&m).unwrap();
    assert!(dropped(&c, "KHR_materials_transmission"), "{:?}", c.report);
    assert_eq!(c.report.items.len(), 1, "{:?}", c.report);
    // craft-mesh reads part of these two.
    m.extensions.insert("KHR_materials_clearcoat".into(), craft_mesh::serde_json::json!({ "clearcoatNormalTexture": { "index": 0 } }));
    m.extensions.insert("KHR_materials_pbrSpecularGlossiness".into(), craft_mesh::serde_json::json!({ "specularFactor": [1, 0, 0] }));
    let c = try_from_mesh(&m).unwrap();
    assert!(dropped(&c, "clearcoatNormalTexture"), "{:?}", c.report);
    assert!(approximated(&c, "KHR_materials_pbrSpecularGlossiness"), "{:?}", c.report);
    assert_eq!(c.report.items.len(), 3, "{:?}", c.report);
}

#[test]
fn mesh_textures_keep_their_uv_state_scale_and_encoding() {
    let mr = TextureRef {
        image: 2,
        uv_set: 1,
        wrap: [MeshWrap::Clamp, MeshWrap::Mirror],
        transform: Some(MeshUv { offset: [0.25, 0.5], rotation: 0.5, scale: [2.0, 3.0] }),
    };
    let m = Mesh {
        base_color_texture: tex(1),
        metallic_roughness_texture: Some(mr),
        normal_texture: tex(3),
        normal_scale: 0.5,
        occlusion_texture: tex(4),
        occlusion_strength: 0.25,
        emissive_texture: tex(5),
        clearcoat: 0.5,
        clearcoat_texture: tex(6),
        clearcoat_roughness_texture: tex(7),
        sheen_color: [0.1, 0.0, 0.0],
        sheen_color_texture: tex(8),
        sheen_roughness_texture: tex(9),
        ..Mesh::default()
    };
    let c = try_from_mesh(&m).unwrap();
    // Base color, emissive and sheen color textures are sRGB, the rest linear (glTF 2.0 and
    // KHR_materials_sheen); the packed texture holds roughness in G and metallic in B.
    let expected = [
        ("base_color_texture", TextureInput::BaseColor, TextureEncoding::Srgb),
        ("metallic_roughness_texture", TextureInput::MetallicRoughness, TextureEncoding::Linear),
        ("normal_texture", TextureInput::Normal, TextureEncoding::Linear),
        ("occlusion_texture", TextureInput::Occlusion, TextureEncoding::Linear),
        ("emissive_texture", TextureInput::Emissive, TextureEncoding::Srgb),
        ("clearcoat_texture", TextureInput::Clearcoat, TextureEncoding::Linear),
        ("clearcoat_roughness_texture", TextureInput::ClearcoatRoughness, TextureEncoding::Linear),
        ("sheen_color_texture", TextureInput::SheenColor, TextureEncoding::Srgb),
        ("sheen_roughness_texture", TextureInput::SheenRoughness, TextureEncoding::Linear),
    ];
    assert_eq!(c.surface.textures.len(), expected.len());
    for (i, ((name, input, encoding), b)) in expected.into_iter().zip(&c.surface.textures).enumerate() {
        assert_eq!((&b.input, &b.encoding, &b.source), (&input, &encoding, &TextureSource::Image(i + 1)), "{name}");
        assert!(unshaded(&c, name), "{name}");
    }
    let mr = &c.surface.textures[1];
    assert_eq!((mr.uv_set, mr.wrap), (1, [Wrap::Clamp, Wrap::Mirror]));
    assert_eq!(mr.transform, Some(UvTransform { offset: [0.25, 0.5], rotation: 0.5, scale: [2.0, 3.0] }));
    assert_eq!((c.surface.textures[2].scale, c.surface.textures[3].scale, c.surface.textures[0].scale), (0.5, 0.25, 1.0));
    assert_eq!(c.surface.textures[0].transform, None);
    // The core holds the factors the textures multiply: glTF's defaults here.
    assert_eq!((c.surface.material.metallic(), c.surface.material.roughness()), (1.0, 1.0));
    assert!(unshaded(&c, "clearcoat") && unshaded(&c, "sheen"));
}

#[test]
fn mesh_clearcoat_and_sheen_are_kept_and_layers_that_are_off_drop_their_textures() {
    let on = Mesh { clearcoat: 0.75, clearcoat_roughness: 0.25, sheen_color: [0.5, 0.25, 0.0], sheen_roughness: 0.6, ..Mesh::default() };
    let c = try_from_mesh(&on).unwrap();
    assert_eq!(c.surface.clearcoat, Some(Clearcoat { weight: 0.75, roughness: 0.25 }));
    assert_eq!(c.surface.sheen, Some(Sheen { color: [0.5, 0.25, 0.0], roughness: 0.6 }));
    // A clearcoat factor of 0 and a black sheen color each turn their layer off
    // (KHR_materials_clearcoat, KHR_materials_sheen).
    let off = Mesh {
        clearcoat_roughness: 0.5,
        clearcoat_texture: tex(1),
        clearcoat_roughness_texture: tex(2),
        sheen_roughness: 0.5,
        sheen_color_texture: tex(3),
        sheen_roughness_texture: tex(4),
        ..Mesh::default()
    };
    let c = try_from_mesh(&off).unwrap();
    assert_eq!((c.surface.clearcoat, c.surface.sheen), (None, None));
    assert!(c.surface.textures.is_empty());
    for name in ["clearcoat_texture", "clearcoat_roughness_texture", "sheen_color_texture", "sheen_roughness_texture"] {
        assert!(dropped(&c, name), "{name}");
    }
    assert!(!unshaded(&c, "clearcoat") && !unshaded(&c, "sheen"));
}

#[test]
fn mesh_textures_of_a_layer_that_is_off_are_checked_before_they_are_dropped() {
    let t = TextureRef { transform: Some(MeshUv { rotation: f32::NAN, ..MeshUv::default() }), ..TextureRef::default() };
    bad_value(try_from_mesh(&Mesh { clearcoat_roughness_texture: Some(t), ..Mesh::default() }), "clearcoat_roughness_texture transform");
    bad_value(try_from_mesh(&Mesh { sheen_color_texture: Some(t), ..Mesh::default() }), "sheen_color_texture transform");
    let fine = TextureRef { transform: Some(MeshUv { rotation: 0.5, ..MeshUv::default() }), ..TextureRef::default() };
    let c = try_from_mesh(&Mesh { clearcoat_texture: Some(fine), ..Mesh::default() }).unwrap();
    assert!(c.surface.textures.is_empty() && dropped(&c, "clearcoat_texture"));
}

#[test]
fn mesh_unlit_is_kept_and_reported() {
    let c = try_from_mesh(&Mesh { unlit: true, base_color: [0.2, 0.4, 0.6, 1.0], ..Mesh::default() }).unwrap();
    assert!(c.surface.unlit);
    assert_eq!(c.surface.material.base_color(), [0.2, 0.4, 0.6]);
    assert!(unshaded(&c, "unlit"));
}

#[test]
fn mesh_values_that_are_not_finite_or_out_of_range_are_errors() {
    type Set = fn(&mut Mesh, f32);
    let unit: [(&str, Set); 8] = [
        ("base_color", |m, v| m.base_color[1] = v),
        ("base_color alpha", |m, v| m.base_color[3] = v),
        ("metallic", |m, v| m.metallic = v),
        ("roughness", |m, v| m.roughness = v),
        ("occlusion_strength", |m, v| m.occlusion_strength = v),
        ("clearcoat", |m, v| m.clearcoat = v),
        ("clearcoat_roughness", |m, v| m.clearcoat_roughness = v),
        ("sheen_color", |m, v| m.sheen_color[2] = v),
    ];
    for (name, set) in unit.iter().chain([("sheen_roughness", (|m: &mut Mesh, v| m.sheen_roughness = v) as Set)].iter()) {
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
            let mut m = Mesh::default();
            set(&mut m, v);
            bad_value(try_from_mesh(&m), name);
        }
        for v in [0.0, 1.0] {
            let mut m = Mesh::default();
            set(&mut m, v);
            assert!(try_from_mesh(&m).is_ok(), "{name} {v}");
        }
    }
    for v in [f32::NAN, f32::INFINITY, -1.0] {
        bad_value(try_from_mesh(&Mesh { emissive: [0.0, v, 0.0], ..Mesh::default() }), "emissive");
        bad_value(try_from_mesh(&Mesh { alpha_mode: MeshAlpha::Mask(v), ..Mesh::default() }), "alpha cutoff");
    }
    for v in [f32::NAN, f32::INFINITY] {
        bad_value(try_from_mesh(&Mesh { normal_scale: v, ..Mesh::default() }), "normal_scale");
        let t = TextureRef { transform: Some(MeshUv { rotation: v, ..MeshUv::default() }), ..TextureRef::default() };
        bad_value(try_from_mesh(&Mesh { emissive_texture: Some(t), ..Mesh::default() }), "emissive_texture transform");
    }
    // Emission above 1, a cutoff above 1 and a negative normal scale are in range.
    let wide = Mesh { emissive: [40.0; 3], alpha_mode: MeshAlpha::Mask(2.0), normal_scale: -1.0, ..Mesh::default() };
    let c = try_from_mesh(&wide).unwrap();
    assert_eq!((c.surface.emissive, c.surface.alpha_mode), ([40.0; 3], AlphaMode::Mask(2.0)));
}

// ------------------------------------------------------------------ craft-materialx

fn mx(shader: Shader, inputs: &[(&str, Value)]) -> Mx {
    Mx {
        name: "M".into(),
        shader,
        inputs: inputs.iter().map(|(n, v)| ((*n).to_owned(), v.clone())).collect(),
        displacement: None,
        uv_tiling: [1.0, 1.0],
        warnings: Vec::new(),
    }
}

fn k(values: &[f32]) -> Value {
    Value::Constant { values: values.to_vec(), colorspace: None }
}

fn ks(values: &[f32], space: &str) -> Value {
    Value::Constant { values: values.to_vec(), colorspace: Some(space.into()) }
}

fn img(file: &str, space: Option<&str>, tiling: Option<[f32; 2]>) -> Value {
    Value::Image { file: file.into(), colorspace: space.map(str::to_owned), uv_tiling: tiling }
}

/// Schlick's reflectance at normal incidence for an index of refraction, as the specular weight
/// of the core: `((n − 1) / (n + 1))² / 0.04`.
fn specular_for_ior(n: f64) -> f32 {
    (((n - 1.0) / (n + 1.0)).powi(2) / 0.04) as f32
}

#[test]
fn materialx_empty_inputs_give_each_shaders_defaults() {
    // The defaults of the node definitions in MaterialX's libraries/bxdf: open_pbr_surface.mtlx,
    // standard_surface.mtlx (version 1.0.1) and gltf_pbr.mtlx
    // (https://github.com/AcademySoftwareFoundation/MaterialX/tree/main/libraries/bxdf).
    let open = try_from_materialx(&mx(Shader::OpenPbr, &[])).unwrap();
    let s = &open.surface;
    assert_close3(s.material.base_color(), [0.8; 3]);
    assert_eq!((s.material.metallic(), s.material.roughness(), s.material.specular()), (0.0, 0.3, 1.0));
    assert_eq!((s.alpha, s.alpha_mode, s.emissive), (1.0, AlphaMode::Opaque, [0.0; 3]));
    assert_eq!((s.clearcoat, s.sheen), (None, None));
    assert!(open.report.is_empty(), "{:?}", open.report);

    let standard = try_from_materialx(&mx(Shader::StandardSurface, &[])).unwrap();
    let s = &standard.surface;
    assert_close3(s.material.base_color(), [0.8; 3]);
    assert_eq!((s.material.metallic(), s.material.roughness(), s.material.specular()), (0.0, 0.2, 1.0));
    assert_eq!((s.alpha, s.alpha_mode, s.emissive), (1.0, AlphaMode::Opaque, [0.0; 3]));
    assert_eq!((s.clearcoat, s.sheen), (None, None));
    assert!(standard.report.is_empty(), "{:?}", standard.report);

    let gltf = try_from_materialx(&mx(Shader::GltfPbr, &[])).unwrap();
    let s = &gltf.surface;
    assert_eq!(s.material.base_color(), [1.0; 3]);
    assert_eq!((s.material.metallic(), s.material.roughness(), s.material.specular()), (1.0, 1.0, 1.0));
    assert_eq!((s.alpha, s.alpha_mode, s.emissive), (1.0, AlphaMode::Opaque, [0.0; 3]));
    assert_eq!((s.clearcoat, s.sheen), (None, None));
    assert!(gltf.report.is_empty(), "{:?}", gltf.report);
}

#[test]
fn materialx_base_weight_scales_the_base_color_and_specular_follows_weight_and_ior() {
    let c =
        try_from_materialx(&mx(Shader::OpenPbr, &[("base_color", k(&[0.5, 0.4, 0.2])), ("base_weight", k(&[0.5])), ("base_metalness", k(&[0.25]))]))
            .unwrap();
    assert_close3(c.surface.material.base_color(), [0.25, 0.2, 0.1]);
    assert_eq!(c.surface.material.metallic(), 0.25);
    assert!(c.report.is_empty(), "{:?}", c.report);
    let standard = try_from_materialx(&mx(Shader::StandardSurface, &[("base", k(&[0.5])), ("specular", k(&[0.5]))])).unwrap();
    assert_close3(standard.surface.material.base_color(), [0.4; 3]);
    assert!(close(standard.surface.material.specular(), 0.5));
    assert!(approximated(&standard, "specular"));

    let weight = try_from_materialx(&mx(Shader::OpenPbr, &[("specular_weight", k(&[2.0]))])).unwrap();
    assert!(close(weight.surface.material.specular(), 2.0));
    assert!(approximated(&weight, "specular_weight"));
    let water = try_from_materialx(&mx(Shader::OpenPbr, &[("specular_ior", k(&[1.33]))])).unwrap();
    assert!(close(water.surface.material.specular(), specular_for_ior(1.33)), "{}", water.surface.material.specular());
    let gltf = try_from_materialx(&mx(Shader::GltfPbr, &[("ior", k(&[2.0])), ("specular", k(&[0.5]))])).unwrap();
    assert!(close(gltf.surface.material.specular(), 0.5 * specular_for_ior(2.0)));
    // 25 is the largest weight the core takes: at IOR 1.5 a weight of 25 fits, 25.5 does not.
    let max = try_from_materialx(&mx(Shader::OpenPbr, &[("specular_weight", k(&[25.0]))])).unwrap();
    assert!(close(max.surface.material.specular(), 25.0));
    bad_value(try_from_materialx(&mx(Shader::OpenPbr, &[("specular_weight", k(&[25.5]))])), "specular_weight");
    bad_value(try_from_materialx(&mx(Shader::OpenPbr, &[("specular_weight", k(&[2.0])), ("specular_ior", k(&[30.0]))])), "specular_weight");
}

#[test]
fn materialx_color_constants_are_decoded_from_the_color_space_in_effect() {
    let base = |v: Value| try_from_materialx(&mx(Shader::GltfPbr, &[("base_color", v)])).map(|c| c.surface.material.base_color());
    let c = [0.5, 0.2, 0.02];
    // The sRGB transfer function of IEC 61966-2-1, written out here independently.
    let srgb = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    for space in ["srgb_texture", "srgb_rec709_scene"] {
        let got = base(ks(&c, space)).unwrap();
        assert_close3(got, c.map(srgb));
        assert_close3(got, c.map(craft_texture::srgb_to_linear));
    }
    for space in ["lin_rec709", "lin_rec709_scene", "data", "none"] {
        assert_eq!(base(ks(&c, space)).unwrap(), c, "{space}");
    }
    assert_eq!(base(k(&c)).unwrap(), c);
    assert_close3(base(ks(&c, "g22_rec709")).unwrap(), c.map(|v| v.powf(2.2)));
    assert_close3(base(ks(&c, "g18_rec709_scene")).unwrap(), c.map(|v| v.powf(1.8)));
    assert_close3(base(ks(&c, "g24_rec709_scene")).unwrap(), c.map(|v| v.powf(2.4)));
    for space in ["acescg", "lin_ap1_scene", "srgb_p3d65_scene", "bogus"] {
        match base(ks(&c, space)) {
            Err(ConvertError::ColorSpace { input, colorspace }) => assert_eq!((input.as_str(), colorspace.as_str()), ("base_color", space)),
            other => panic!("{space}: {other:?}"),
        }
    }
    // Scalars are never decoded, whatever the color space.
    let rough = try_from_materialx(&mx(Shader::GltfPbr, &[("roughness", ks(&[0.5], "srgb_texture"))])).unwrap();
    assert_eq!(rough.surface.material.roughness(), 0.5);
}

#[test]
fn materialx_images_become_bindings_and_the_core_holds_the_factor() {
    let m = mx(
        Shader::OpenPbr,
        &[
            ("base_color", img("tex/albedo.png", Some("srgb_texture"), Some([2.0, 3.0]))),
            ("base_weight", k(&[0.5])),
            ("base_metalness", img("metal.png", Some("srgb_texture"), None)),
            ("specular_roughness", img("rough.png", None, None)),
            ("geometry_opacity", img("cut.png", None, None)),
            ("geometry_normal", Value::NormalMap { file: "n.png".into(), colorspace: Some("lin_rec709".into()), uv_tiling: None }),
            ("emission_color", img("glow.png", Some("acescg"), None)),
            ("emission_luminance", k(&[4.0])),
            ("specular_ior", img("ior.png", None, None)),
        ],
    );
    let c = try_from_materialx(&m).unwrap();
    let s = &c.surface;
    assert_close3(s.material.base_color(), [0.5; 3]);
    assert_eq!((s.material.metallic(), s.material.roughness(), s.material.specular()), (1.0, 1.0, 1.0));
    assert_eq!((s.alpha, s.alpha_mode), (1.0, AlphaMode::Blend));
    assert_eq!(s.emissive, [4.0; 3]);
    let find = |input: TextureInput| s.textures.iter().find(|b| b.input == input).unwrap();
    let albedo = find(TextureInput::BaseColor);
    assert_eq!((&albedo.source, &albedo.encoding), (&TextureSource::File("tex/albedo.png".into()), &TextureEncoding::Srgb));
    assert_eq!(albedo.transform, Some(UvTransform { offset: [0.0; 2], rotation: 0.0, scale: [2.0, 3.0] }));
    assert_eq!((albedo.uv_set, albedo.wrap, albedo.scale), (0, [Wrap::Repeat; 2], 1.0));
    // Data inputs are linear whatever color space the file carries; color inputs keep theirs.
    assert_eq!(find(TextureInput::Metallic).encoding, TextureEncoding::Linear);
    assert_eq!(find(TextureInput::Roughness).encoding, TextureEncoding::Linear);
    assert_eq!(find(TextureInput::Normal).encoding, TextureEncoding::Linear);
    assert_eq!(find(TextureInput::Opacity).source, TextureSource::File("cut.png".into()));
    assert_eq!(find(TextureInput::Emissive).encoding, TextureEncoding::Named("acescg".into()));
    assert_eq!(find(TextureInput::Other("specular_ior".into())).source, TextureSource::File("ior.png".into()));
    for name in ["base_color", "base_metalness", "specular_roughness", "geometry_opacity", "geometry_normal", "emission_color", "specular_ior"] {
        assert!(unshaded(&c, name), "{name}");
    }
    assert!(approximated(&c, "specular_ior"));
}

#[test]
fn materialx_coat_and_sheen_are_kept_only_when_they_are_on() {
    let c = try_from_materialx(&mx(
        Shader::OpenPbr,
        &[("coat_weight", k(&[0.5])), ("coat_roughness", k(&[0.2])), ("fuzz_weight", k(&[0.5])), ("fuzz_color", k(&[1.0, 0.5, 0.0]))],
    ))
    .unwrap();
    assert_eq!(c.surface.clearcoat, Some(Clearcoat { weight: 0.5, roughness: 0.2 }));
    // fuzz_roughness defaults to 0.5.
    assert_eq!(c.surface.sheen, Some(Sheen { color: [0.5, 0.25, 0.0], roughness: 0.5 }));
    assert!(approximated(&c, "fuzz_weight") && unshaded(&c, "clearcoat") && unshaded(&c, "sheen"));

    let standard = try_from_materialx(&mx(Shader::StandardSurface, &[("coat", k(&[1.0])), ("sheen", k(&[0.5]))])).unwrap();
    // coat_roughness 0.1, sheen_color white and sheen_roughness 0.3 by default.
    assert_eq!(standard.surface.clearcoat, Some(Clearcoat { weight: 1.0, roughness: 0.1 }));
    assert_eq!(standard.surface.sheen, Some(Sheen { color: [0.5; 3], roughness: 0.3 }));

    let gltf = try_from_materialx(&mx(Shader::GltfPbr, &[("sheen_color", k(&[0.2, 0.1, 0.0])), ("sheen_roughness", k(&[0.4]))])).unwrap();
    assert_eq!(gltf.surface.sheen, Some(Sheen { color: [0.2, 0.1, 0.0], roughness: 0.4 }));
    assert!(!approximated(&gltf, "sheen_color"));

    let off = try_from_materialx(&mx(
        Shader::OpenPbr,
        &[("coat_roughness", img("coat_r.png", None, None)), ("fuzz_color", img("fuzz.png", None, None)), ("coat_weight", k(&[0.0]))],
    ))
    .unwrap();
    assert_eq!((off.surface.clearcoat, off.surface.sheen), (None, None));
    assert!(off.surface.textures.is_empty());
    assert!(dropped(&off, "coat_roughness") && dropped(&off, "fuzz_color"));
    let textured = try_from_materialx(&mx(Shader::OpenPbr, &[("coat_weight", img("coat.png", None, None))])).unwrap();
    assert_eq!(textured.surface.clearcoat, Some(Clearcoat { weight: 1.0, roughness: 0.0 }));
    assert_eq!(textured.surface.textures[0].input, TextureInput::Clearcoat);
}

#[test]
fn materialx_coat_and_sheen_roughness_images_read_their_first_channel() {
    let cases = [
        (Shader::OpenPbr, [("coat_weight", k(&[1.0])), ("fuzz_weight", k(&[1.0]))], ["coat_roughness", "fuzz_roughness"]),
        (Shader::StandardSurface, [("coat", k(&[1.0])), ("sheen", k(&[1.0]))], ["coat_roughness", "sheen_roughness"]),
        (Shader::GltfPbr, [("clearcoat", k(&[1.0])), ("sheen_color", k(&[1.0, 1.0, 1.0]))], ["clearcoat_roughness", "sheen_roughness"]),
    ];
    for (shader, [coat, sheen], [coat_roughness, sheen_roughness]) in cases {
        let c = try_from_materialx(&mx(
            shader,
            &[coat, sheen, (coat_roughness, img("coat_r.png", None, None)), (sheen_roughness, img("sheen_r.png", None, None))],
        ))
        .unwrap();
        let inputs: Vec<_> = c.surface.textures.iter().map(|t| (t.input.clone(), t.source.clone())).collect();
        assert_eq!(
            inputs,
            [
                (TextureInput::ClearcoatRoughnessFirst, TextureSource::File("coat_r.png".into())),
                (TextureInput::SheenRoughnessFirst, TextureSource::File("sheen_r.png".into())),
            ],
            "{shader:?}"
        );
    }
}

#[test]
fn materialx_coat_only_inputs_are_dropped_while_the_coat_is_off() {
    let coat_only = [
        (Shader::OpenPbr, "coat_weight", ["geometry_coat_normal", "coat_color", "coat_ior"]),
        (Shader::StandardSurface, "coat", ["coat_normal", "coat_color", "coat_IOR"]),
    ];
    for (shader, weight, [normal, color, ior]) in coat_only {
        let inputs = [
            (normal, Value::NormalMap { file: "coat_n.png".into(), colorspace: None, uv_tiling: None }),
            (color, img("coat_c.png", None, None)),
            (ior, k(&[1.4])),
        ];
        // The coat weight is left at its default of 0.
        let off = try_from_materialx(&mx(shader, &inputs)).unwrap();
        assert_eq!(off.surface.clearcoat, None);
        assert!(off.surface.textures.is_empty(), "{:?}", off.surface.textures);
        assert!(off.surface.other_inputs.is_empty(), "{:?}", off.surface.other_inputs);
        for name in [normal, color, ior] {
            assert!(dropped(&off, name) && !unshaded(&off, name), "{name}: {:?}", off.report);
        }
        // With the coat on, the same inputs are kept and reported as unshaded.
        let mut with_coat = inputs.to_vec();
        with_coat.push((weight, k(&[0.5])));
        let on = try_from_materialx(&mx(shader, &with_coat)).unwrap();
        assert!(on.surface.clearcoat.is_some());
        assert_eq!(on.surface.textures.len(), 2);
        assert_eq!(on.surface.other_inputs, vec![(ior.to_owned(), vec![1.4])]);
        for name in [normal, color, ior] {
            assert!(unshaded(&on, name) && !dropped(&on, name), "{name}");
        }
    }
    let gltf = try_from_materialx(&mx(
        Shader::GltfPbr,
        &[("clearcoat_normal", Value::NormalMap { file: "cc_n.png".into(), colorspace: None, uv_tiling: None })],
    ))
    .unwrap();
    assert!(gltf.surface.textures.is_empty() && dropped(&gltf, "clearcoat_normal"));
}

#[test]
fn materialx_constants_of_a_layer_that_is_off_are_dropped() {
    // The coat weight defaults to 0, so the coat is off and its roughness is dropped.
    let coat = try_from_materialx(&mx(Shader::OpenPbr, &[("coat_roughness", k(&[0.4]))])).unwrap();
    assert_eq!(coat.surface.clearcoat, None);
    assert!(dropped(&coat, "coat_roughness"), "{:?}", coat.report);
    // The weight that turns the layer off is not reported as dropped.
    let zero = try_from_materialx(&mx(Shader::OpenPbr, &[("coat_weight", k(&[0.0])), ("coat_roughness", k(&[0.4]))])).unwrap();
    assert!(dropped(&zero, "coat_roughness") && !dropped(&zero, "coat_weight"), "{:?}", zero.report);
    let on = try_from_materialx(&mx(Shader::OpenPbr, &[("coat_weight", k(&[0.5])), ("coat_roughness", k(&[0.4]))])).unwrap();
    assert!(!dropped(&on, "coat_roughness"));
    // fuzz_weight defaults to 0: the fuzz color and roughness are dropped.
    let fuzz = try_from_materialx(&mx(Shader::OpenPbr, &[("fuzz_color", k(&[1.0, 0.5, 0.0])), ("fuzz_roughness", k(&[0.2]))])).unwrap();
    assert!(dropped(&fuzz, "fuzz_color") && dropped(&fuzz, "fuzz_roughness"), "{:?}", fuzz.report);
    // A black color turns the sheen off, so its weight is dropped and the color is not.
    let black = try_from_materialx(&mx(Shader::StandardSurface, &[("sheen", k(&[0.5])), ("sheen_color", k(&[0.0; 3]))])).unwrap();
    assert!(dropped(&black, "sheen") && !dropped(&black, "sheen_color"), "{:?}", black.report);
    let gltf = try_from_materialx(&mx(Shader::GltfPbr, &[("sheen_roughness", k(&[0.3]))])).unwrap();
    assert!(dropped(&gltf, "sheen_roughness"), "{:?}", gltf.report);
}

#[test]
fn materialx_emission_is_the_color_times_its_strength() {
    let dark = try_from_materialx(&mx(Shader::OpenPbr, &[("emission_color", k(&[1.0, 0.5, 0.0]))])).unwrap();
    assert_eq!(dark.surface.emissive, [0.0; 3]);
    assert!(dropped(&dark, "emission_color"), "{:?}", dark.report);
    let lit = try_from_materialx(&mx(Shader::OpenPbr, &[("emission_color", k(&[1.0, 0.5, 0.0])), ("emission_luminance", k(&[10.0]))])).unwrap();
    assert_eq!(lit.surface.emissive, [10.0, 5.0, 0.0]);
    assert!(approximated(&lit, "emission_luminance"));
    let standard = try_from_materialx(&mx(Shader::StandardSurface, &[("emission", k(&[2.0])), ("emission_color", k(&[0.5, 0.25, 1.0]))])).unwrap();
    assert_eq!(standard.surface.emissive, [1.0, 0.5, 2.0]);
    assert!(standard.report.is_empty());
    let gltf = try_from_materialx(&mx(Shader::GltfPbr, &[("emissive", k(&[0.5, 0.25, 0.0])), ("emissive_strength", k(&[2.0]))])).unwrap();
    assert_eq!(gltf.surface.emissive, [1.0, 0.5, 0.0]);
}

#[test]
fn materialx_standard_surface_opacity_color_gives_its_luminance() {
    let standard = |opacity: &[f32]| try_from_materialx(&mx(Shader::StandardSurface, &[("opacity", k(opacity))]));
    let gray = standard(&[0.5; 3]).unwrap();
    assert_eq!(gray.surface.alpha, 0.5);
    assert!(gray.report.is_empty(), "{:?}", gray.report);
    // 0.2126 * 0.2 + 0.7152 * 0.4 + 0.0722 * 0.6, the Rec.709 luminance; the mean would be 0.4.
    let colored = standard(&[0.2, 0.4, 0.6]).unwrap();
    assert!(close(colored.surface.alpha, 0.37192), "{}", colored.surface.alpha);
    assert_eq!(colored.surface.alpha_mode, AlphaMode::Blend);
    assert!(approximated(&colored, "opacity"));
    let green = standard(&[0.0, 1.0, 0.0]).unwrap();
    assert!(close(green.surface.alpha, 0.7152), "{}", green.surface.alpha);
    let near_white = standard(&[1.0, 1.0, 0.999_999_9]).unwrap();
    assert!(near_white.surface.alpha <= 1.0 && near_white.surface.alpha >= 0.999_999_9);
    for v in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        bad_value(standard(&[0.5, v, 0.5]), "opacity");
    }
    // An image on the color opacity binds as RGB; the float opacities bind their first channel.
    let textured = try_from_materialx(&mx(Shader::StandardSurface, &[("opacity", img("o.png", None, None))])).unwrap();
    assert_eq!(textured.surface.textures[0].input, TextureInput::OpacityRgb);
    for (shader, name) in [(Shader::OpenPbr, "geometry_opacity"), (Shader::GltfPbr, "alpha")] {
        let c = try_from_materialx(&mx(shader, &[(name, img("o.png", None, None))])).unwrap();
        assert_eq!(c.surface.textures[0].input, TextureInput::Opacity, "{name}");
    }
    // OpenPBR's geometry_opacity is a float in its node definition.
    assert_eq!(
        try_from_materialx(&mx(Shader::OpenPbr, &[("geometry_opacity", k(&[0.5; 3]))])).unwrap_err(),
        ConvertError::Components { input: "geometry_opacity".into(), expected: 1, found: 3 }
    );
}

#[test]
fn materialx_alpha_and_its_mode() {
    let open = try_from_materialx(&mx(Shader::OpenPbr, &[("geometry_opacity", k(&[0.5]))])).unwrap();
    assert_eq!((open.surface.alpha, open.surface.alpha_mode), (0.5, AlphaMode::Blend));

    let gltf = |inputs: &[(&str, Value)]| try_from_materialx(&mx(Shader::GltfPbr, inputs));
    assert_eq!(gltf(&[("alpha_mode", k(&[1.0])), ("alpha_cutoff", k(&[0.25]))]).unwrap().surface.alpha_mode, AlphaMode::Mask(0.25));
    assert_eq!(gltf(&[("alpha_mode", k(&[1.0]))]).unwrap().surface.alpha_mode, AlphaMode::Mask(0.5));
    assert_eq!(gltf(&[("alpha_mode", k(&[2.0]))]).unwrap().surface.alpha_mode, AlphaMode::Blend);
    let opaque = gltf(&[("alpha", k(&[0.5]))]).unwrap();
    assert_eq!((opaque.surface.alpha, opaque.surface.alpha_mode), (0.5, AlphaMode::Opaque));
    for bad in [3.0, 1.5, -1.0] {
        bad_value(gltf(&[("alpha_mode", k(&[bad]))]), "alpha_mode");
    }
    bad_value(gltf(&[("alpha_mode", img("m.png", None, None))]), "alpha_mode");
    bad_value(gltf(&[("alpha_cutoff", img("c.png", None, None))]), "alpha_cutoff");
}

#[test]
fn materialx_bad_inputs_are_errors_and_unknown_ones_are_dropped() {
    let open = |inputs: &[(&str, Value)]| try_from_materialx(&mx(Shader::OpenPbr, inputs));
    assert_eq!(open(&[("base_metalness", k(&[0.0])), ("base_metalness", k(&[1.0]))]).unwrap_err(), ConvertError::Duplicate("base_metalness".into()));
    assert_eq!(open(&[("base_color", k(&[0.5]))]).unwrap_err(), ConvertError::Components { input: "base_color".into(), expected: 3, found: 1 });
    assert_eq!(
        open(&[("base_metalness", k(&[0.5; 3]))]).unwrap_err(),
        ConvertError::Components { input: "base_metalness".into(), expected: 1, found: 3 }
    );
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
        for name in
            ["base_weight", "base_metalness", "specular_roughness", "coat_weight", "fuzz_roughness", "geometry_opacity", "transmission_weight"]
        {
            bad_value(open(&[(name, k(&[v]))]), name);
        }
        bad_value(open(&[("base_color", k(&[0.5, v, 0.5]))]), "base_color");
    }
    for v in [f32::NAN, f32::INFINITY, -1.0] {
        bad_value(open(&[("emission_luminance", k(&[v]))]), "emission_luminance");
        bad_value(open(&[("specular_ior", k(&[v]))]), "specular_ior");
        bad_value(open(&[("geometry_normal", k(&[0.0, 0.0, if v < 0.0 { f32::NAN } else { v }]))]), "geometry_normal");
    }
    bad_value(try_from_materialx(&mx(Shader::GltfPbr, &[("ior", k(&[0.9]))])), "ior");
    // glTF PBR's specular_color has no upper bound, so a large encoded value overflows to
    // infinity when it is decoded to linear; that is an error, not an infinite input.
    for space in ["g22_rec709", "srgb_texture", "srgb_rec709_scene", "g24_rec709_scene"] {
        bad_value(try_from_materialx(&mx(Shader::GltfPbr, &[("specular_color", ks(&[1e20, 1.0, 1.0], space))])), "specular_color");
    }
    let wide = try_from_materialx(&mx(Shader::GltfPbr, &[("specular_color", ks(&[4.0, 1.0, 1.0], "g22_rec709"))])).unwrap();
    assert_eq!(wide.surface.other_inputs.len(), 1);
    assert!(close(wide.surface.other_inputs[0].1[0], 4f32.powf(2.2)));
    bad_value(open(&[("base_color", img("a.png", None, Some([0.0, 1.0])))]), "base_color");
    bad_value(open(&[("base_color", img("a.png", None, Some([f32::NAN, 1.0])))]), "base_color");
    let unknown = open(&[("thin_film_weight", k(&[0.5])), ("not_an_input", img("x.png", None, None))]).unwrap();
    assert!(dropped(&unknown, "thin_film_weight") && dropped(&unknown, "not_an_input"));
    assert!(unknown.surface.textures.is_empty() && unknown.surface.other_inputs.is_empty());
}

#[test]
fn materialx_other_inputs_and_the_displacement_are_kept() {
    let mut m = mx(
        Shader::OpenPbr,
        &[
            ("specular_color", ks(&[1.0, 0.5, 0.5], "srgb_texture")),
            ("transmission_weight", k(&[0.5])),
            ("subsurface_weight", img("sss.png", None, None)),
        ],
    );
    m.displacement = Some(k(&[0.25]));
    let c = try_from_materialx(&m).unwrap();
    let srgb_half = craft_texture::srgb_to_linear(0.5);
    assert_eq!(
        c.surface.other_inputs,
        vec![
            ("specular_color".into(), vec![1.0, srgb_half, srgb_half]),
            ("transmission_weight".into(), vec![0.5]),
            ("displacement".into(), vec![0.25])
        ]
    );
    assert_eq!(c.surface.textures[0].input, TextureInput::Other("subsurface_weight".into()));
    for name in ["specular_color", "transmission_weight", "subsurface_weight", "displacement"] {
        assert!(unshaded(&c, name), "{name}");
    }
    m.displacement = Some(img("h.png", Some("srgb_texture"), Some([4.0, 4.0])));
    let c = try_from_materialx(&m).unwrap();
    let h = c.surface.textures.iter().find(|b| b.input == TextureInput::Displacement).unwrap();
    assert_eq!((&h.source, &h.encoding), (&TextureSource::File("h.png".into()), &TextureEncoding::Linear));
    assert_eq!(h.transform.map(|t| t.scale), Some([4.0, 4.0]));
    m.displacement = Some(k(&[f32::NAN]));
    bad_value(try_from_materialx(&m), "displacement");
}

#[test]
fn materialx_reader_warnings_reach_the_report() {
    let mut m = mx(Shader::OpenPbr, &[]);
    m.warnings = vec!["input thin_film_weight is not read".into()];
    let c = try_from_materialx(&m).unwrap();
    assert_eq!(c.report.items, vec![ReportItem::Warning("input thin_film_weight is not read".into())]);
}

#[test]
fn materialx_float_converted_to_a_color_is_not_decoded_from_the_nodes_color_space() {
    // The float is not a color value, so the node's srgb_texture does not apply to it.
    let doc = Document::parse(
        br#"<materialx colorspace="lin_rec709">
          <open_pbr_surface name="S"><input name="base_color" nodename="C"/></open_pbr_surface>
          <convert name="C" type="color3" colorspace="srgb_texture"><input name="in" type="float" value="0.5"/></convert>
          <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial>
        </materialx>"#,
    )
    .unwrap();
    let c = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
    assert_close3(c.surface.material.base_color(), [0.5; 3]);
}

#[test]
fn materialx_documents_convert_from_xml() {
    let doc = Document::parse(
        br#"<materialx colorspace="srgb_texture">
          <gltf_pbr name="S">
            <input name="base_color" type="color3" value="0.5, 0.5, 0.5"/>
            <input name="emissive" type="color3" value="1, 0, 0" colorspace="lin_rec709"/>
            <input name="emissive_strength" type="float" value="3"/>
            <input name="iridescence" type="float" value="0.5"/>
          </gltf_pbr>
          <surfacematerial name="Red"><input name="surfaceshader" nodename="S"/></surfacematerial>
        </materialx>"#,
    )
    .unwrap();
    let c = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
    assert_eq!(c.surface.name, "Red");
    assert_close3(c.surface.material.base_color(), [craft_texture::srgb_to_linear(0.5); 3]);
    assert_eq!(c.surface.emissive, [3.0, 0.0, 0.0]);
    assert_eq!(c.report.items, vec![ReportItem::Warning("input iridescence is not read".into())]);

    // craft-materialx's OpenPBR writer and this conversion agree on every texture it writes.
    let textures = [
        Texture { input: "base_color".into(), file: "albedo.png".into(), colorspace: "srgb_texture".into() },
        Texture { input: "specular_roughness".into(), file: "rough.png".into(), colorspace: "srgb_texture".into() },
        Texture { input: "geometry_normal".into(), file: "normal.png".into(), colorspace: "lin_rec709".into() },
        Texture { input: "emission_color".into(), file: "glow.png".into(), colorspace: "lin_rec709".into() },
        Texture { input: "height".into(), file: "height.png".into(), colorspace: "lin_rec709".into() },
    ];
    let written = Document::parse(write_open_pbr("Stone", &textures, None).as_bytes()).unwrap();
    let c = try_from_materialx(&written.read_material_strict(None).unwrap()).unwrap();
    let s = &c.surface;
    let by = |input: TextureInput| s.textures.iter().find(|b| b.input == input).map(|b| (b.source.clone(), b.encoding.clone()));
    assert_eq!(by(TextureInput::BaseColor), Some((TextureSource::File("albedo.png".into()), TextureEncoding::Srgb)));
    assert_eq!(by(TextureInput::Roughness), Some((TextureSource::File("rough.png".into()), TextureEncoding::Linear)));
    assert_eq!(by(TextureInput::Normal), Some((TextureSource::File("normal.png".into()), TextureEncoding::Linear)));
    assert_eq!(by(TextureInput::Emissive), Some((TextureSource::File("glow.png".into()), TextureEncoding::Linear)));
    assert_eq!(by(TextureInput::Displacement), Some((TextureSource::File("height.png".into()), TextureEncoding::Linear)));
    // The writer sets emission_luminance to 1 with an emission texture, so the texture shows.
    assert_eq!(s.emissive, [1.0; 3]);
    assert_eq!(s.name, "Stone");
}
