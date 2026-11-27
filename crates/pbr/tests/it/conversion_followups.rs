use craft_materialx::Document;
use craft_mesh::Material;
use craft_pbr::{ReportItem, TextureInput, try_from_materialx, try_from_mesh};

fn document(input: &str, nodes: &str) -> Document {
    Document::parse(
        format!(
            r#"<materialx><open_pbr_surface name="S">{input}</open_pbr_surface>{nodes}<surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#
        )
        .as_bytes(),
    )
    .unwrap()
}

#[test]
fn a_selected_output_without_a_target_does_not_select_another_output() {
    let doc = Document::parse(
        br#"<materialx><nodegraph name="g"><open_pbr_surface name="S"/>
        <output name="a" nodename="S"/><output name="b"/></nodegraph>
        <surfacematerial name="M"><input name="surfaceshader" nodegraph="g" output="b"/></surfacematerial></materialx>"#,
    )
    .unwrap();
    assert!(doc.read_material_strict(None).is_err());
}

#[test]
fn nondefault_image_addressing_modes_are_reported() {
    for name in ["uaddressmode", "vaddressmode"] {
        for mode in ["constant", "clamp", "mirror"] {
            let node =
                format!(r#"<image name="T" type="color3"><input name="file" value="color.png"/><input name="{name}" value="{mode}"/></image>"#);
            let doc = document(r#"<input name="base_color" nodename="T"/>"#, &node);
            let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
            assert!(converted.report.items.iter().any(|item| matches!(item, ReportItem::Warning(message) if message.contains(name))));
        }
    }
}

#[test]
fn an_identified_convert_node_uses_its_omitted_input_default() {
    let doc = document(r#"<input name="base_color" nodename="C"/>"#, r#"<convert name="C" type="color3" nodedef="ND_convert_float_color3"/>"#);
    let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
    assert_eq!(converted.surface.material.base_color(), [0.0; 3]);
    assert!(converted.report.is_empty(), "{:?}", converted.report);
}

#[test]
fn constant_literals_use_the_nodes_type_when_the_input_omits_it() {
    for (ty, value) in [("integer", "0.5"), ("float", "1,2"), ("color3", "1,2")] {
        let node = format!(r#"<constant name="C" type="{ty}"><input name="value" value="{value}"/></constant>"#);
        let doc = document(r#"<input name="base_color" nodename="C"/>"#, &node);
        assert!(doc.read_material_strict(None).is_err(), "{ty} {value}");
    }
}

#[test]
fn a_missing_graph_local_node_does_not_bind_a_root_node() {
    let doc = Document::parse(
        br#"<materialx><image name="T" type="color3"><input name="file" value="root.png"/></image>
        <nodegraph name="g"><open_pbr_surface name="S"><input name="base_color" nodename="T"/></open_pbr_surface>
        <output name="out" nodename="S"/></nodegraph><surfacematerial name="M">
        <input name="surfaceshader" nodegraph="g"/></surfacematerial></materialx>"#,
    )
    .unwrap();
    assert!(doc.read_material_strict(None).is_err());
}

#[test]
fn direct_normal_vectors_are_not_bound_as_tangent_space_maps() {
    let image = r#"<image name="T" type="vector3"><input name="file" value="normal.png"/></image>"#;
    let direct = document(r#"<input name="geometry_normal" nodename="T"/>"#, image);
    let converted = try_from_materialx(&direct.read_material_strict(None).unwrap()).unwrap();
    assert!(!converted.surface.textures.iter().any(|binding| binding.input == TextureInput::Normal));
    assert!(converted.report.items.iter().any(|item| matches!(item, ReportItem::Dropped { input, .. } if input == "geometry_normal")));
    let nodes = format!(r#"{image}<normalmap name="N" type="vector3"><input name="in" nodename="T"/></normalmap>"#);
    let wrapped = document(r#"<input name="geometry_normal" nodename="N"/>"#, &nodes);
    let converted = try_from_materialx(&wrapped.read_material_strict(None).unwrap()).unwrap();
    assert!(converted.surface.textures.iter().any(|binding| binding.input == TextureInput::Normal));
}

#[test]
fn nondefault_factors_without_textures_are_reported() {
    let material = Material { normal_scale: 0.5, occlusion_strength: 0.25, ..Material::default() };
    let converted = try_from_mesh(&material).unwrap();
    for name in ["normal_scale", "occlusion_strength"] {
        assert!(converted.report.items.iter().any(|item| matches!(item, ReportItem::Dropped { input, .. } if input == name)));
    }
    assert!(try_from_mesh(&Material::default()).unwrap().report.is_empty());
}

#[test]
fn nested_normalmaps_are_reported_and_skipped() {
    let nodes = r#"<image name="T" type="vector3"><input name="file" value="normal.png"/></image>
        <normalmap name="Inner" type="vector3"><input name="in" nodename="T"/></normalmap>
        <normalmap name="Outer" type="vector3"><input name="in" nodename="Inner"/></normalmap>"#;
    let doc = document(r#"<input name="geometry_normal" nodename="Outer"/>"#, nodes);
    let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
    assert!(!converted.surface.textures.iter().any(|binding| binding.input == TextureInput::Normal));
    assert!(
        converted
            .report
            .items
            .iter()
            .any(|item| matches!(item, ReportItem::Warning(message) if message.contains("nested") && message.contains("Outer")))
    );
}

#[test]
fn normalmap_literals_do_not_become_untransformed_world_normals() {
    for value in ["1,0.5,1", "0.5,0.5,1"] {
        let doc = document(
            r#"<input name="geometry_normal" nodename="N"/>"#,
            &format!(r#"<normalmap name="N" type="vector3"><input name="in" value="{value}"/></normalmap>"#),
        );
        let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
        assert!(!converted.surface.other_inputs.iter().any(|(name, _)| name == "geometry_normal"));
        assert!(converted.report.items.iter().any(|item| matches!(item, ReportItem::Warning(message) if message.contains("normalmap"))));
    }
}

#[test]
fn authored_normalmap_frames_are_reported() {
    for name in ["normal", "tangent", "bitangent"] {
        let nodes = format!(
            r#"<image name="T" type="vector3"><input name="file" value="normal.png"/></image>
            <normalmap name="N" type="vector3"><input name="in" nodename="T"/><input name="{name}" type="vector3" value="1,0,0"/></normalmap>"#
        );
        let doc = document(r#"<input name="geometry_normal" nodename="N"/>"#, &nodes);
        let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
        assert!(converted.report.items.iter().any(|item| matches!(item, ReportItem::Warning(message) if message.contains(name))));
    }
}

#[test]
fn coat_normals_keep_tangent_maps_and_report_direct_world_images() {
    for (shader, weight, normal) in [
        ("open_pbr_surface", "coat_weight", "geometry_coat_normal"),
        ("standard_surface", "coat", "coat_normal"),
        ("gltf_pbr", "clearcoat", "clearcoat_normal"),
    ] {
        for (node, wrapped) in [("T", false), ("N", true)] {
            let xml = format!(
                r#"<materialx><{shader} name="S"><input name="{weight}" value="1"/><input name="{normal}" nodename="{node}"/></{shader}>
                <image name="T" type="vector3"><input name="file" value="normal.png"/></image>
                <normalmap name="N" type="vector3"><input name="in" nodename="T"/></normalmap>
                <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#
            );
            let doc = Document::parse(xml.as_bytes()).unwrap();
            let converted = try_from_materialx(&doc.read_material_strict(None).unwrap()).unwrap();
            assert_eq!(
                converted.surface.textures.iter().any(|binding| matches!(&binding.input, TextureInput::Other(name) if name == normal)),
                wrapped,
                "{shader} {node}"
            );
            assert_eq!(
                converted.report.items.iter().any(|item| matches!(item, ReportItem::Dropped { input, .. } if input == normal)),
                !wrapped,
                "{shader} {node}"
            );
        }
    }
}
