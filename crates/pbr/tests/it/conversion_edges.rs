use craft_materialx::{Document, Resolved};
use craft_mesh::Material as Mesh;
use craft_pbr::{ConvertError, ReportItem, TextureInput, TextureSource, try_from_materialx, try_from_mesh};

fn document(shader: &str, inputs: &str, nodes: &str, material_inputs: &str) -> Document {
    let text = format!(
        r#"<materialx><{shader} name="S">{inputs}</{shader}>{nodes}<surfacematerial name="M"><input name="surfaceshader" nodename="S"/>{material_inputs}</surfacematerial></materialx>"#
    );
    Document::parse(text.as_bytes()).unwrap()
}

#[test]
fn inactive_emission_still_validates_the_color_field() {
    let doc = document("standard_surface", r#"<input name="emission" value="0"/><input name="emission_color" value="2,0,0"/>"#, "", "");
    let material = doc.read_material_strict(None).unwrap();
    assert!(matches!(try_from_materialx(&material), Err(ConvertError::BadValue { input, .. }) if input == "emission_color"));
    let legacy = doc.read_material(None).unwrap();
    assert!(!legacy.inputs.iter().any(|(name, _)| name == "emission_color"));
    let valid = document("standard_surface", r#"<input name="emission" value="0"/><input name="emission_color" value="0.2,0.3,0.4"/>"#, "", "");
    let converted = try_from_materialx(&valid.read_material_strict(None).unwrap()).unwrap();
    assert_eq!(converted.surface.emissive, [0.0; 3]);
    assert!(converted.report.items.iter().any(|item| matches!(item, ReportItem::Dropped { input, .. } if input == "emission_color")));
}

#[test]
fn displacement_scale_is_reported_when_it_is_not_applied() {
    for scale in ["", r#"<input name="scale" value="1"/>"#, r#"<input name="scale" value="2"/>"#] {
        let node = format!(r#"<displacement name="D"><input name="displacement" type="float" value="0.25"/>{scale}</displacement>"#);
        let doc = document("standard_surface", "", &node, r#"<input name="displacementshader" nodename="D"/>"#);
        let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
        assert_eq!(converted.surface.other_inputs, vec![("displacement".into(), vec![0.25])]);
        assert_eq!(
            converted.report.items.iter().any(|item| matches!(item, ReportItem::Warning(message) if message.contains("scale"))),
            scale.contains('2')
        );
    }
    for scale in ["1,2", "NaN", ""] {
        let node = format!(r#"<displacement name="D"><input name="displacement" value="0.25"/><input name="scale" value="{scale}"/></displacement>"#);
        assert!(document("standard_surface", "", &node, r#"<input name="displacementshader" nodename="D"/>"#).read_material_strict(None).is_err());
    }
}

#[test]
fn graph_surface_connections_preserve_the_shader_scope() {
    let doc = Document::parse(
        br#"<materialx><image name="T" type="color3"><input name="file" value="root.png"/></image>
        <nodegraph name="g"><image name="T" type="color3"><input name="file" value="graph.png"/></image>
        <standard_surface name="S"><input name="base_color" nodename="T"/></standard_surface>
        <output name="out" nodename="S"/></nodegraph>
        <surfacematerial name="M"><input name="surfaceshader" nodegraph="g" output="out"/></surfacematerial></materialx>"#,
    )
    .unwrap();
    let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
    assert_eq!(converted.surface.textures[0].source, TextureSource::File("graph.png".into()));
    assert!(doc.read_material(None).is_err());
}

#[test]
fn broken_authored_connections_do_not_become_shader_defaults() {
    for reference in [r#"nodename="missing""#, r#"nodegraph="missing" output="out""#] {
        let doc = document("open_pbr_surface", &format!(r#"<input name="base_color" {reference}/>"#), "", "");
        assert!(doc.read_material_strict(None).is_err(), "{reference}");
    }
}

#[test]
fn a_nonzero_tiled_image_offset_is_reported() {
    for offset in ["0,0", "0.25,0"] {
        let nodes = format!(
            r#"<tiledimage name="T" type="color3"><input name="file" value="tile.png"/><input name="uvtiling" value="2,2"/><input name="uvoffset" value="{offset}"/></tiledimage>"#
        );
        let doc = document("open_pbr_surface", r#"<input name="base_color" nodename="T"/>"#, &nodes, "");
        let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
        assert_eq!(converted.surface.textures[0].transform.unwrap().scale, [2.0, 2.0]);
        assert_eq!(
            converted.report.items.iter().any(|item| matches!(item, ReportItem::Warning(message) if message.contains("uvoffset"))),
            offset != "0,0"
        );
    }
}

#[test]
fn an_omitted_color_constant_value_is_black() {
    let doc = document("open_pbr_surface", r#"<input name="base_color" nodename="K"/>"#, r#"<constant name="K" type="color3"/>"#, "");
    let material = doc.read_material_strict(None).unwrap();
    assert!(
        matches!(material.inputs.iter().find(|(name, _)| name == "base_color"), Some((_, Resolved::Constant { values, .. })) if values == &[0.0; 3])
    );
    assert_eq!(try_from_materialx(&material).unwrap().surface.material.base_color(), [0.0; 3]);
}

#[test]
fn an_authored_cutoff_is_reported_when_the_alpha_mode_does_not_use_it() {
    for mode in [0, 1, 2] {
        let doc = document("gltf_pbr", &format!(r#"<input name="alpha_mode" value="{mode}"/><input name="alpha_cutoff" value="0.25"/>"#), "", "");
        let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
        assert_eq!(converted.report.items.iter().any(|item| matches!(item, ReportItem::Dropped { input, .. } if input == "alpha_cutoff")), mode != 1);
        if mode == 1 {
            assert_eq!(converted.surface.alpha_mode, craft_pbr::AlphaMode::Mask(0.25));
        }
        let absent = document("gltf_pbr", &format!(r#"<input name="alpha_mode" value="{mode}"/>"#), "", "");
        let converted = try_from_materialx(&absent.read_material_strict(None).unwrap()).unwrap();
        assert!(!converted.report.items.iter().any(|item| matches!(item, ReportItem::Dropped { input, .. } if input == "alpha_cutoff")));
    }
}

#[test]
fn vector_displacement_is_reported_without_a_scalar_height_binding() {
    for source in [r#"type="vector3" value="0.1,0.2,0.3""#, r#"type="vector3" nodename="T""#, r#"value="0.1,0.2,0.3""#] {
        let nodes = format!(
            r#"<image name="T" type="vector3"><input name="file" value="vector.png"/></image><displacement name="D"><input name="displacement" {source}/></displacement>"#
        );
        let doc = document("standard_surface", "", &nodes, r#"<input name="displacementshader" nodename="D"/>"#);
        let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
        assert!(!converted.surface.textures.iter().any(|texture| texture.input == TextureInput::Displacement));
        assert!(!converted.surface.other_inputs.iter().any(|(name, _)| name == "displacement"));
        assert!(
            converted
                .report
                .items
                .iter()
                .any(|item| matches!(item, ReportItem::Warning(message) if message.contains("vector") && message.contains("displacement")))
        );
    }
}

#[test]
fn inactive_mesh_layers_report_nondefault_roughness() {
    assert!(try_from_mesh(&Mesh::default()).unwrap().report.is_empty());
    let converted = try_from_mesh(&Mesh { clearcoat_roughness: 0.5, sheen_roughness: 0.5, ..Mesh::default() }).unwrap();
    for field in ["clearcoat_roughness", "sheen_roughness"] {
        assert!(converted.report.items.iter().any(|item| matches!(item, ReportItem::Dropped { input, .. } if input == field)), "{field}");
    }
    let active =
        try_from_mesh(&Mesh { clearcoat: 1.0, clearcoat_roughness: 0.5, sheen_color: [0.2; 3], sheen_roughness: 0.5, ..Mesh::default() }).unwrap();
    assert_eq!(active.surface.clearcoat.unwrap().roughness, 0.5);
    assert_eq!(active.surface.sheen.unwrap().roughness, 0.5);
}

#[test]
fn omitted_numeric_constants_use_their_typed_zero_values() {
    for (kind, count) in [("float", 1), ("integer", 1), ("boolean", 1), ("vector2", 2), ("vector3", 3), ("vector4", 4), ("color3", 3), ("color4", 4)]
    {
        let doc = document("open_pbr_surface", r#"<input name="base_color" nodename="K"/>"#, &format!(r#"<constant name="K" type="{kind}"/>"#), "");
        let material = doc.read_material_strict(None).unwrap();
        assert!(
            matches!(material.inputs.iter().find(|(name, _)| name == "base_color"), Some((_, Resolved::Constant { values, .. })) if values == &vec![0.0; count]),
            "{kind}"
        );
    }
    let unsupported = document("open_pbr_surface", r#"<input name="base_color" nodename="K"/>"#, r#"<constant name="K" type="matrix33"/>"#, "");
    let material = unsupported.read_material_strict(None).unwrap();
    assert!(material.inputs.is_empty() && !material.warnings.is_empty());
}

#[test]
fn strict_graph_output_selection_rejects_ambiguity() {
    let read = |outputs: &str, selection: &str| {
        Document::parse(
            format!(
                r#"<materialx><nodegraph name="g">
        <standard_surface name="A"><input name="base_color" value="0.2,0.2,0.2"/></standard_surface>
        <standard_surface name="B"><input name="base_color" value="0.8,0.8,0.8"/></standard_surface>{outputs}</nodegraph>
        <surfacematerial name="M"><input name="surfaceshader" nodegraph="g" {selection}/></surfacematerial></materialx>"#
            )
            .as_bytes(),
        )
        .unwrap()
        .read_material_strict(None)
    };
    let both = r#"<output name="a" nodename="A"/><output name="b" nodename="B"/>"#;
    assert!(read(both, "").is_err());
    assert!(read(both, r#"output="missing""#).is_err());
    assert_eq!(try_from_materialx(&read(both, r#"output="b""#).unwrap()).unwrap().surface.material.base_color(), [0.8; 3]);
    let sole = read(r#"<output name="a" nodename="A"/>"#, r#"output="unused""#).unwrap();
    assert_eq!(try_from_materialx(&sole).unwrap().surface.material.base_color(), [0.2; 3]);
}

#[test]
fn omitted_node_parameters_keep_their_identity_defaults() {
    let displaced = document(
        "standard_surface",
        "",
        r#"<displacement name="D"><input name="displacement" value="0.25"/><input name="scale" type="float"/></displacement>"#,
        r#"<input name="displacementshader" nodename="D"/>"#,
    );
    assert!(displaced.read_material_strict(None).unwrap().warnings.is_empty());
    let image = document(
        "open_pbr_surface",
        r#"<input name="base_color" nodename="T"/>"#,
        r#"<tiledimage name="T" type="color3"><input name="file" value="tile.png"/><input name="uvoffset" type="vector2"/></tiledimage>"#,
        "",
    );
    assert!(image.read_material_strict(None).unwrap().warnings.is_empty());
    let normal = document(
        "open_pbr_surface",
        r#"<input name="geometry_normal" nodename="N"/>"#,
        r#"<normalmap name="N"><input name="in" nodename="T"/><input name="scale" type="float"/></normalmap><image name="T" type="vector3"><input name="file" value="n.png"/></image>"#,
        "",
    );
    assert!(normal.read_material_strict(None).unwrap().warnings.is_empty());
}

#[test]
fn interface_connections_report_the_unsupported_value() {
    let doc = Document::parse(
        br#"<materialx><nodegraph name="g"><input name="tint" type="color3" value="0.2,0.3,0.4"/>
        <standard_surface name="S"><input name="base_color" interfacename="tint" value="0.8,0.8,0.8"/></standard_surface>
        <output name="out" nodename="S"/></nodegraph><surfacematerial name="M">
        <input name="surfaceshader" nodegraph="g"/></surfacematerial></materialx>"#,
    )
    .unwrap();
    let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
    assert!(
        converted
            .report
            .items
            .iter()
            .any(|item| matches!(item, ReportItem::Warning(message) if message.contains("interface") && message.contains("tint")))
    );
}

#[test]
fn connected_image_tiling_keeps_the_resolved_scale() {
    for connection in [r#"nodename="K""#, r#"nodegraph="g""#] {
        let nodes = format!(
            r#"<tiledimage name="T" type="color3"><input name="file" value="tile.png"/><input name="uvtiling" {connection}/></tiledimage>
            <constant name="K" type="vector2"><input name="value" value="2,3"/></constant>
            <nodegraph name="g"><constant name="K" type="vector2"><input name="value" value="2,3"/></constant><output name="out" nodename="K"/></nodegraph>"#
        );
        let doc = document("standard_surface", r#"<input name="base_color" nodename="T"/>"#, &nodes, "");
        let material = doc.read_material_strict(None).unwrap();
        assert!(material.warnings.is_empty(), "{:?}", material.warnings);
        assert_eq!(material.uv_tiling, [2.0, 3.0]);
        let converted = try_from_materialx(&material).unwrap();
        let binding = converted.surface.textures.iter().find(|binding| binding.input == TextureInput::BaseColor).unwrap();
        assert_eq!(binding.transform.as_ref().map(|transform| transform.scale), Some([2.0, 3.0]));
    }
}

#[test]
fn images_without_literal_file_values_report_the_skipped_texture() {
    for kind in ["image", "tiledimage"] {
        for file in ["", r#"<input name="file"/>"#, r#"<input name="file" value=""/>"#, r#"<input name="file" interfacename="filename"/>"#] {
            let nodes = format!(r#"<{kind} name="T" type="color3">{file}</{kind}>"#);
            let doc = document("standard_surface", r#"<input name="base_color" nodename="T"/>"#, &nodes, "");
            let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
            assert!(converted.surface.textures.is_empty());
            assert!(converted.report.items.iter().any(|item| matches!(item, ReportItem::Warning(message) if message.contains("file"))));
        }
    }
}
