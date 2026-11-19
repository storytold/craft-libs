//! MaterialX XML documents and node graphs.
//!
//! This crate owns the format boundary: bounded parsing of `.mtlx` documents, ordered access to
//! their nodes and nodegraph outputs, and XML serialization. Applications map shader inputs to
//! their own material models.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::collections::HashMap;

use craft_xml::{Doctype, El, Error as XmlError, Limits, Options, UnknownEntity};

/// Maximum source document size accepted by the parser.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Maximum XML nesting depth accepted by the parser.
pub const MAX_DEPTH: usize = 64;
/// Maximum XML element count accepted by the parser.
pub const MAX_NODES: usize = 200_000;

/// An invalid or unsupported MaterialX document.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The document is not valid MaterialX XML.
    #[error("invalid MaterialX document: {0}")]
    Xml(#[from] XmlError),
    /// The XML root element is not `<materialx>`.
    #[error("the XML root element is not <materialx>")]
    Root,
    /// The document does not contain the requested surface material.
    #[error("no MaterialX material named {0:?}")]
    MissingMaterial(String),
    /// The document has an unsupported or incomplete shader connection.
    #[error("unsupported MaterialX material: {0}")]
    Unsupported(String),
    /// The document's material references are malformed.
    #[error("malformed MaterialX material: {0}")]
    Malformed(String),
}

/// A MaterialX document, with XML elements kept in source order.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    root: El,
}

/// A recognized MaterialX surface shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shader {
    /// OpenPBR Surface.
    OpenPbr,
    /// Autodesk Standard Surface.
    StandardSurface,
    /// glTF PBR Surface.
    GltfPbr,
}

/// The value connected to a shader input.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A finite numeric literal, kept in the source color space. `colorspace` is the input's
    /// explicit tag; document-level defaults remain available on [`Document::root`].
    Constant { values: Vec<f32>, colorspace: Option<String> },
    /// A file path from an image node, with the effective file prefix applied.
    Image(String),
}

/// A surface material resolved from a MaterialX document.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    /// Material name, or `Material` when omitted.
    pub name: String,
    /// Shader dialect represented by the material.
    pub shader: Shader,
    /// Resolved inputs in the supported shader table's deterministic order.
    pub inputs: Vec<(String, Value)>,
    /// Resolved displacement input, if present.
    pub displacement: Option<Value>,
    /// First valid image tiling value, or `[1, 1]`.
    pub uv_tiling: [f32; 2],
    /// Unsupported or ignored constructs encountered while resolving values.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
struct Found {
    value: Value,
    tiling: Option<[f32; 2]>,
}

const OPEN_PBR_INPUTS: &[&str] = &[
    "base_color",
    "base_metalness",
    "specular_roughness",
    "geometry_normal",
    "geometry_opacity",
    "emission_color",
    "coat_weight",
    "coat_roughness",
    "geometry_coat_normal",
    "fuzz_weight",
    "fuzz_color",
    "fuzz_roughness",
    "specular_roughness_anisotropy",
    "transmission_weight",
    "subsurface_weight",
];
const STANDARD_INPUTS: &[&str] = &[
    "base_color",
    "metalness",
    "specular_roughness",
    "normal",
    "opacity",
    "emission_color",
    "coat",
    "coat_roughness",
    "coat_normal",
    "sheen",
    "sheen_color",
    "sheen_roughness",
    "specular_anisotropy",
    "transmission",
    "subsurface",
];
const GLTF_INPUTS: &[&str] = &[
    "base_color",
    "metallic",
    "roughness",
    "normal",
    "alpha",
    "emission",
    "occlusion",
    "clearcoat",
    "clearcoat_roughness",
    "clearcoat_normal",
    "sheen_color",
    "sheen_roughness",
    "transmission",
];

/// OpenPBR input kind accepted by [`write_open_pbr`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// Color value or color texture.
    Color,
    /// Scalar value or data texture.
    Float,
    /// Vector normal texture.
    Normal,
    /// Displacement map, serialized as the MaterialX displacement shader input.
    Height,
}

/// A texture connected to an OpenPBR input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    /// MaterialX/OpenPBR input name, such as `base_color` or `geometry_normal`.
    pub input: String,
    /// Texture filename relative to the MaterialX document.
    pub file: String,
    /// Color space used for color inputs; data maps ignore this field.
    pub colorspace: String,
}

const OPENPBR_WRITABLE: &[(&str, InputKind)] = &[
    ("base_color", InputKind::Color),
    ("base_metalness", InputKind::Float),
    ("specular_roughness", InputKind::Float),
    ("specular_roughness_anisotropy", InputKind::Float),
    ("coat_weight", InputKind::Float),
    ("coat_roughness", InputKind::Float),
    ("fuzz_weight", InputKind::Float),
    ("fuzz_color", InputKind::Color),
    ("fuzz_roughness", InputKind::Float),
    ("transmission_weight", InputKind::Float),
    ("subsurface_weight", InputKind::Float),
    ("emission_color", InputKind::Color),
    ("geometry_opacity", InputKind::Float),
    ("geometry_normal", InputKind::Normal),
    ("height", InputKind::Height),
];

impl Document {
    /// Parses a MaterialX document while enforcing size, nesting and node-count limits.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let limits = Limits {
            max_input: MAX_BYTES,
            max_depth: MAX_DEPTH,
            max_nodes: MAX_NODES,
            max_elements: MAX_NODES,
            max_attrs: 256,
            max_attrs_total: 400_000,
        };
        let options = Options {
            doctype: Doctype::Reject,
            unknown_entity: UnknownEntity::Error,
            drop_blank_text: true,
            names: craft_xml::Names::Local,
            ns_decls: false,
            ..Options::default()
        }
        .strict()
        .with_limits(limits);
        let root = options.parse(bytes)?;
        if root.name.rsplit(':').next() != Some("materialx") {
            return Err(Error::Root);
        }
        Ok(Document { root })
    }

    /// Creates a document from a `<materialx>` element.
    pub fn from_root(root: El) -> Result<Self, Error> {
        if root.name.rsplit(':').next() != Some("materialx") {
            return Err(Error::Root);
        }
        Ok(Document { root })
    }

    /// The root `<materialx>` element.
    pub fn root(&self) -> &El {
        &self.root
    }

    /// The root `<materialx>` element, for application-specific editing.
    pub fn root_mut(&mut self) -> &mut El {
        &mut self.root
    }

    /// Serializes the document as XML, preserving element and attribute order.
    pub fn to_xml(&self) -> String {
        format!("{}{}", craft_xml::DECL, self.root.to_xml_pretty("  "))
    }

    /// Nodegraphs in document order.
    pub fn nodegraphs(&self) -> impl Iterator<Item = &El> {
        self.root.children("nodegraph")
    }

    /// Outputs in a named nodegraph, in document order.
    pub fn outputs(&self, graph: &str) -> impl Iterator<Item = &El> {
        self.nodegraphs().filter(move |g| g.attr("name") == Some(graph)).flat_map(|g| g.children("output"))
    }

    /// The output of a named graph. An omitted output name selects the first output in source
    /// order. For duplicate output names, the last definition wins, matching the consumer's
    /// existing named lookup behavior.
    pub fn output(&self, graph: &str, output: Option<&str>) -> Option<&El> {
        match output {
            Some(name) => self.outputs(graph).filter(|node| node.attr("name") == Some(name)).last(),
            None => self.outputs(graph).next(),
        }
    }

    /// Resolves a surface material into shader inputs, image paths and warnings.
    pub fn read_material(&self, material: Option<&str>) -> Result<Material, Error> {
        let materials: Vec<&El> = self.root.children("surfacematerial").collect();
        let mat = match material {
            Some(name) => materials.iter().copied().find(|m| m.attr("name") == Some(name)).ok_or_else(|| Error::MissingMaterial(name.to_owned()))?,
            None => materials.first().copied().ok_or_else(|| Error::Unsupported("document has no surfacematerial".into()))?,
        };
        let mut nodes = HashMap::new();
        let mut outputs = Vec::new();
        for child in self.root.elements() {
            if child.name == "nodegraph" {
                let Some(graph) = child.attr("name") else { continue };
                for node in child.elements() {
                    let Some(name) = node.attr("name") else { continue };
                    if node.name == "output" {
                        if let Some(from) = node.attr("nodename") {
                            outputs.push((format!("{graph}/{name}"), format!("{graph}/{from}")));
                        }
                    } else {
                        nodes.insert(format!("{graph}/{name}"), node);
                    }
                }
            } else if let Some(name) = child.attr("name") {
                nodes.insert(name.to_owned(), child);
            }
        }
        let shader_name = mat.children("input").find(|i| i.attr("name") == Some("surfaceshader")).and_then(|i| i.attr("nodename"));
        let shader = shader_name.and_then(|n| nodes.get(n).copied()).ok_or_else(|| Error::Malformed("material has no surface shader".into()))?;
        let (kind, inputs) = match shader.name.as_str() {
            "open_pbr_surface" => (Shader::OpenPbr, OPEN_PBR_INPUTS),
            "standard_surface" => (Shader::StandardSurface, STANDARD_INPUTS),
            "gltf_pbr" => (Shader::GltfPbr, GLTF_INPUTS),
            other => return Err(Error::Unsupported(format!("surface shader is {other}"))),
        };
        let emission_gate = match kind {
            Shader::OpenPbr => Some(("emission_luminance", true)),
            Shader::StandardSurface => Some(("emission", true)),
            Shader::GltfPbr => None,
        };
        let emission_enabled = emission_gate.is_none_or(|(name, _)| {
            shader
                .children("input")
                .find(|i| i.attr("name") == Some(name))
                .is_some_and(|i| i.attr("value").is_none_or(|v| parse_numbers(v).and_then(|n| n.first().copied()).is_some_and(|v| v > 0.0)))
        });
        let mut warnings = Vec::new();
        let base_weight_name = match kind {
            Shader::OpenPbr => Some("base_weight"),
            Shader::StandardSurface => Some("base"),
            Shader::GltfPbr => None,
        };
        if base_weight_name.and_then(|n| numeric_input(shader, n)).is_some_and(|w| (w - 1.0).abs() > 1e-6) {
            warnings.push("the base weight isn't 1; it was ignored".into());
        }
        let mut resolved = Vec::new();
        let mut tiling = None;
        for name in inputs {
            let Some(input) = shader.children("input").find(|i| i.attr("name") == Some(*name)) else { continue };
            let is_emission = matches!((kind, *name), (Shader::OpenPbr, "emission_color") | (Shader::StandardSurface, "emission_color"));
            if is_emission && !emission_enabled {
                warnings.push(format!("{name} was skipped: the shader's emission is 0"));
                continue;
            }
            if let Some(found) = resolve_value(&self.root, input, None, &nodes, &outputs, 0, &mut warnings) {
                if tiling.is_none() {
                    tiling = found.tiling;
                }
                resolved.push(((*name).to_owned(), found.value));
            }
        }
        let displacement = mat
            .children("input")
            .find(|i| i.attr("name") == Some("displacementshader"))
            .and_then(|i| i.attr("nodename"))
            .and_then(|n| nodes.get(n).copied())
            .and_then(|d| d.children("input").find(|i| i.attr("name") == Some("displacement")))
            .and_then(|i| resolve_value(&self.root, i, None, &nodes, &outputs, 0, &mut warnings).map(|f| f.value));
        Ok(Material {
            name: mat.attr("name").unwrap_or("Material").to_owned(),
            shader: kind,
            inputs: resolved,
            displacement,
            uv_tiling: tiling.unwrap_or([1.0, 1.0]),
            warnings,
        })
    }
}

fn numeric_input(shader: &El, name: &str) -> Option<f32> {
    shader
        .children("input")
        .find(|i| i.attr("name") == Some(name))
        .and_then(|i| i.attr("value"))
        .and_then(parse_numbers)
        .and_then(|n| n.first().copied())
}

fn parse_numbers(text: &str) -> Option<Vec<f32>> {
    let nums: Option<Vec<f32>> = text.split(',').map(|part| part.trim().parse::<f32>().ok().filter(|v| v.is_finite())).collect();
    nums.filter(|v| !v.is_empty() && v.len() <= 4)
}

fn resolve_value(
    root: &El,
    input: &El,
    scope: Option<&str>,
    nodes: &HashMap<String, &El>,
    outputs: &[(String, String)],
    hops: usize,
    warnings: &mut Vec<String>,
) -> Option<Found> {
    if hops > 16 {
        warnings.push("a node chain is longer than 16 nodes (or loops); skipped".into());
        return None;
    }
    if let Some(value) = input.attr("value") {
        return parse_numbers(value)
            .map(|values| Found { value: Value::Constant { values, colorspace: input.attr("colorspace").map(str::to_owned) }, tiling: None });
    }
    let target = if let Some(graph) = input.attr("nodegraph") {
        let selected = match input.attr("output") {
            Some(output) => outputs.iter().rev().find(|(k, _)| k == &format!("{graph}/{output}")),
            None => outputs.iter().find(|(k, _)| k.starts_with(&format!("{graph}/"))),
        };
        selected.map(|(_, node)| node.clone())
    } else if let Some(name) = input.attr("nodename") {
        let qualified = scope.map(|g| format!("{g}/{name}"));
        qualified.filter(|q| nodes.contains_key(q)).or_else(|| Some(name.to_owned()))
    } else {
        None
    }?;
    let node = nodes.get(&target).copied()?;
    let node_scope = target.split_once('/').map(|(g, _)| g);
    match node.name.as_str() {
        "image" | "tiledimage" => {
            let file = node.children("input").find(|i| i.attr("name") == Some("file")).and_then(|i| i.attr("value"))?;
            let prefix = node
                .children("input")
                .find(|i| i.attr("name") == Some("file"))
                .and_then(|i| i.attr("fileprefix"))
                .or_else(|| node.attr("fileprefix"))
                .or_else(|| root.attr("fileprefix"))
                .unwrap_or("");
            let uv = node
                .children("input")
                .find(|i| i.attr("name") == Some("uvtiling"))
                .and_then(|i| i.attr("value"))
                .and_then(parse_numbers)
                .filter(|n| n.len() == 2 && n.iter().all(|v| *v > 0.0))
                .map(|n| [n[0].clamp(0.001, 1000.0), n[1].clamp(0.001, 1000.0)]);
            Some(Found { value: Value::Image(format!("{prefix}{file}")), tiling: uv })
        }
        "normalmap" | "convert" => node
            .children("input")
            .find(|i| i.attr("name") == Some("in"))
            .and_then(|i| resolve_value(root, i, node_scope, nodes, outputs, hops + 1, warnings)),
        "constant" => node
            .children("input")
            .find(|i| i.attr("name") == Some("value"))
            .and_then(|i| resolve_value(root, i, node_scope, nodes, outputs, hops + 1, warnings)),
        other => {
            warnings.push(format!("a {other} node ({:?}) is not supported; its input was skipped", node.attr("name").unwrap_or("")));
            None
        }
    }
}

/// Writes an OpenPBR MaterialX document for the supplied textures.
///
/// Unknown inputs and duplicate inputs are skipped; the first texture for an input wins. Color
/// spaces are written for color textures only, while data textures are left linear.
pub fn write_open_pbr(name: &str, textures: &[Texture], physical_size_cm: Option<[f32; 2]>) -> String {
    let base = node_name(name);
    let mut seen = Vec::new();
    let selected: Vec<(&Texture, InputKind)> = textures
        .iter()
        .filter_map(|t| OPENPBR_WRITABLE.iter().find(|(input, _)| *input == t.input).map(|(_, kind)| (t, *kind)))
        .filter(|(texture, _)| {
            if seen.iter().any(|name: &&str| *name == texture.input) {
                false
            } else {
                seen.push(texture.input.as_str());
                true
            }
        })
        .collect();
    let has = |input: &str| selected.iter().any(|(texture, _)| texture.input == input);
    let mut root = El::new("materialx").with_attr("version", "1.39").with_attr("colorspace", "lin_rec709");
    let shader_name = format!("OPBR_{base}");
    let mut shader = El::new("open_pbr_surface").with_attr("name", &shader_name).with_attr("type", "surfaceshader");
    for (texture, kind) in &selected {
        let (ty, node) = match kind {
            InputKind::Color => ("color3", image_node_name(&texture.input)),
            InputKind::Float => ("float", image_node_name(&texture.input)),
            InputKind::Normal => ("vector3", "normalmap".to_owned()),
            InputKind::Height => continue,
        };
        shader.push(El::new("input").with_attr("name", &texture.input).with_attr("type", ty).with_attr("nodename", node));
    }
    if has("emission_color") {
        shader.push(El::new("input").with_attr("name", "emission_luminance").with_attr("type", "float").with_attr("value", "1"));
    }
    root.push(shader);
    for (texture, kind) in &selected {
        let (ty, color) = match kind {
            InputKind::Color => ("color3", Some(texture.colorspace.as_str())),
            InputKind::Normal => ("vector3", None),
            InputKind::Float | InputKind::Height => ("float", None),
        };
        let mut image = El::new("tiledimage").with_attr("name", image_node_name(&texture.input)).with_attr("type", ty);
        let mut file = El::new("input").with_attr("name", "file").with_attr("type", "filename").with_attr("value", &texture.file);
        if let Some(space) = color {
            file.set_attr("colorspace", space);
        }
        image.push(file);
        root.push(image);
    }
    if has("geometry_normal") {
        root.push(El::new("normalmap").with_attr("name", "normalmap").with_attr("type", "vector3").with_child(
            El::new("input").with_attr("name", "in").with_attr("type", "vector3").with_attr("nodename", image_node_name("geometry_normal")),
        ));
    }
    if has("height") {
        root.push(
            El::new("displacement")
                .with_attr("name", format!("DISP_{base}"))
                .with_attr("type", "displacementshader")
                .with_child(
                    El::new("input").with_attr("name", "displacement").with_attr("type", "float").with_attr("nodename", image_node_name("height")),
                )
                .with_child(El::new("input").with_attr("name", "scale").with_attr("type", "float").with_attr("value", "1")),
        );
    }
    let mut material = El::new("surfacematerial").with_attr("name", &base).with_attr("type", "material");
    if let Some([width, height]) = physical_size_cm.filter(|[w, h]| w.is_finite() && h.is_finite()) {
        material.set_attr("doc", format!("Tiles every {width} x {height} cm"));
    }
    material.push(El::new("input").with_attr("name", "surfaceshader").with_attr("type", "surfaceshader").with_attr("nodename", shader_name));
    if has("height") {
        material.push(
            El::new("input")
                .with_attr("name", "displacementshader")
                .with_attr("type", "displacementshader")
                .with_attr("nodename", format!("DISP_{base}")),
        );
    }
    root.push(material);
    format!("{}{}", craft_xml::DECL, root.to_xml_pretty("  "))
}

fn image_node_name(input: &str) -> String {
    format!("image_{}", node_name(input))
}

fn node_name(name: &str) -> String {
    let mut out: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_serializes_a_materialx_document() {
        let doc = Document::parse(br#"<materialx version="1.39"><surfacematerial name="mat"/></materialx>"#).unwrap();
        assert_eq!(doc.root().attr("version"), Some("1.39"));
        let roundtrip = Document::parse(doc.to_xml().as_bytes()).unwrap();
        assert_eq!(roundtrip, doc);
    }

    #[test]
    fn graph_outputs_keep_source_order_and_named_selection() {
        let doc = Document::parse(
            br#"<materialx><nodegraph name="g"><output name="second" nodename="b"/><output name="first" nodename="a"/></nodegraph></materialx>"#,
        )
        .unwrap();
        assert_eq!(doc.output("g", None).and_then(|o| o.attr("name")), Some("second"));
        assert_eq!(doc.output("g", Some("first")).and_then(|o| o.attr("nodename")), Some("a"));
    }

    #[test]
    fn duplicate_named_outputs_keep_last_definition() {
        let doc = Document::parse(
            br#"<materialx><nodegraph name="g"><output name="o" nodename="first"/><output name="o" nodename="last"/></nodegraph></materialx>"#,
        )
        .unwrap();
        assert_eq!(doc.output("g", None).and_then(|o| o.attr("nodename")), Some("first"));
        assert_eq!(doc.output("g", Some("o")).and_then(|o| o.attr("nodename")), Some("last"));
    }

    #[test]
    fn rejects_wrong_root_doctypes_and_oversized_documents() {
        assert_eq!(Document::parse(b"<root/>").err(), Some(Error::Root));
        assert!(Document::parse(b"<!DOCTYPE materialx [<!ENTITY x 'y'>]><materialx/>").is_err());
        assert!(matches!(Document::parse(&vec![b' '; MAX_BYTES + 1]), Err(Error::Xml(XmlError::TooLarge(_)))));
    }

    #[test]
    fn resolves_openpbr_values_images_graphs_and_displacement() {
        let doc = Document::parse(br#"<materialx fileprefix="tex/">
          <open_pbr_surface name="S" type="surfaceshader">
            <input name="base_color" type="color3" nodename="C"/>
            <input name="geometry_normal" type="vector3" nodename="N"/>
            <input name="emission_color" type="color3" value="1,0.5,0.25"/>
            <input name="emission_luminance" type="float" value="1"/>
          </open_pbr_surface>
          <tiledimage name="C" type="color3"><input name="file" value="albedo.png"/><input name="uvtiling" value="2,4"/></tiledimage>
          <normalmap name="N" type="vector3"><input name="in" nodegraph="g"/></normalmap>
          <nodegraph name="g"><image name="normal" type="vector3"><input name="file" value="normal.png"/></image><image name="other" type="vector3"><input name="file" value="wrong.png"/></image><output name="first" nodename="normal"/><output name="second" nodename="other"/></nodegraph>
          <displacement name="D" type="displacementshader"><input name="displacement" value="0.25"/></displacement>
          <surfacematerial name="M" type="material"><input name="surfaceshader" nodename="S"/><input name="displacementshader" nodename="D"/></surfacematerial>
        </materialx>"#).unwrap();
        let material = doc.read_material(None).unwrap();
        assert_eq!(material.name, "M");
        assert_eq!(material.shader, Shader::OpenPbr);
        assert_eq!(material.inputs[0], ("base_color".into(), Value::Image("tex/albedo.png".into())));
        assert_eq!(material.inputs[1], ("geometry_normal".into(), Value::Image("tex/normal.png".into())));
        assert_eq!(material.inputs[2].1, Value::Constant { values: vec![1.0, 0.5, 0.25], colorspace: None });
        assert_eq!(material.displacement, Some(Value::Constant { values: vec![0.25], colorspace: None }));
        assert_eq!(material.uv_tiling, [2.0, 4.0]);
    }

    #[test]
    fn openpbr_writer_emits_texture_connections_and_roundtrips() {
        let textures = vec![
            Texture { input: "base_color".into(), file: "albedo<&.png".into(), colorspace: "srgb_texture".into() },
            Texture { input: "specular_roughness".into(), file: "rough.png".into(), colorspace: "ignored".into() },
            Texture { input: "geometry_normal".into(), file: "normal.png".into(), colorspace: "ignored".into() },
            Texture { input: "height".into(), file: "height.png".into(), colorspace: "ignored".into() },
        ];
        let xml = write_open_pbr("Stone", &textures, Some([100.0, 50.0]));
        let doc = Document::parse(xml.as_bytes()).unwrap();
        let material = doc.read_material(None).unwrap();
        assert_eq!(material.name, "Stone");
        assert_eq!(material.inputs[0], ("base_color".into(), Value::Image("albedo<&.png".into())));
        assert_eq!(material.inputs[1], ("specular_roughness".into(), Value::Image("rough.png".into())));
        assert_eq!(material.inputs[2], ("geometry_normal".into(), Value::Image("normal.png".into())));
        assert_eq!(material.displacement, Some(Value::Image("height.png".into())));
        assert!(xml.contains("colorspace=\"srgb_texture\""));
        assert!(xml.contains("doc=\"Tiles every 100 x 50 cm\""));
        assert!(!xml.contains("colorspace=\"ignored\""));
    }
}
