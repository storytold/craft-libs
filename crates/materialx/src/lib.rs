//! MaterialX XML documents and node graphs.
//!
//! This crate owns the format boundary: bounded parsing of `.mtlx` documents, ordered access to
//! their nodes and nodegraph outputs, and XML serialization. Applications map shader inputs to
//! their own material models.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};

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

/// The value connected to a shader input, as [`Document::read_material_strict`] resolves it.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolved {
    /// One to four finite numbers, kept in the source color space. The numbers may be separated
    /// by commas, spaces or both, a `boolean` input's `true` and `false` are 1 and 0, and an
    /// `integer` input holds one whole number. A `convert` node's output holds the converted channels. `colorspace` is the color
    /// space in effect where the value is written: the input's own `colorspace`, else that of the
    /// nearest enclosing element (the node, then its nodegraph), else the document's. A value
    /// written on an input with a boolean, integer, float or vector type, on such a `constant`
    /// node, or with no type on a `convert` node's input, is in the document's working color
    /// space (the root element's `colorspace`). A `convert` node, and a `constant` node whose
    /// value is connected, keep the color space of their input.
    Constant { values: Vec<f32>, colorspace: Option<String> },
    /// A file from an image node, with the file prefix in effect applied: the `file` input's
    /// `fileprefix`, else the node's, its nodegraph's or the document's. `colorspace` is the
    /// color space in effect on the node's `file` input, found the same way as for a constant,
    /// and `uv_tiling` is the node's `uvtiling` as written. A `uvtiling` value that is not two
    /// finite numbers is [`Error::Malformed`]. An image node with a boolean, integer, float or
    /// vector type is in the document's working color space.
    Image { file: String, colorspace: Option<String>, uv_tiling: Option<[f32; 2]> },
    /// This variant stores an encoded tangent-space image connected through a `normalmap` node.
    /// The fields have the same meaning as [`Self::Image`].
    NormalMap { file: String, colorspace: Option<String>, uv_tiling: Option<[f32; 2]> },
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

/// A surface material resolved by [`Document::read_material_strict`].
#[derive(Clone, Debug, PartialEq)]
pub struct StrictMaterial {
    /// Material name, or `Material` when omitted.
    pub name: String,
    /// Shader dialect represented by the material.
    pub shader: Shader,
    /// Resolved inputs in the shader table's deterministic order.
    pub inputs: Vec<(String, Resolved)>,
    /// Resolved scalar displacement input, if present. Vector displacement is reported as unsupported.
    pub displacement: Option<Resolved>,
    /// First image tiling of two positive numbers, kept in `0.001..=1000`, or `[1, 1]`.
    pub uv_tiling: [f32; 2],
    /// Unsupported or ignored constructs encountered while resolving values, and shader inputs
    /// that are not in the shader's table and so are not read.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
struct Found {
    value: Value,
    tiling: Option<[f32; 2]>,
}

/// The material, its surface shader and the document's nodes, found for a read.
struct Located<'a> {
    mat: &'a El,
    shader: &'a El,
    scope: Option<String>,
    kind: Shader,
    nodes: HashMap<String, &'a El>,
    graphs: HashMap<&'a str, &'a El>,
    outputs: Vec<(String, String)>,
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

const OPEN_PBR_STRICT_INPUTS: &[&str] = &[
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
    "base_weight",
    "specular_weight",
    "specular_color",
    "specular_ior",
    "coat_color",
    "coat_ior",
    "emission_luminance",
];
const STANDARD_STRICT_INPUTS: &[&str] = &[
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
    "base",
    "specular",
    "specular_color",
    "specular_IOR",
    "coat_color",
    "coat_IOR",
    "emission",
];
const GLTF_STRICT_INPUTS: &[&str] = &[
    "base_color",
    "metallic",
    "roughness",
    "normal",
    "alpha",
    "emissive",
    "occlusion",
    "clearcoat",
    "clearcoat_roughness",
    "clearcoat_normal",
    "sheen_color",
    "sheen_roughness",
    "transmission",
    "emissive_strength",
    "ior",
    "specular",
    "specular_color",
    "alpha_mode",
    "alpha_cutoff",
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
        let Located { mat, shader, kind, nodes, outputs, .. } = self.locate(material, false)?;
        let inputs = match kind {
            Shader::OpenPbr => OPEN_PBR_INPUTS,
            Shader::StandardSurface => STANDARD_INPUTS,
            Shader::GltfPbr => GLTF_INPUTS,
        };
        let emission_enabled = emission_enabled(kind, shader, parse_numbers);
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
            .and_then(|i| resolve_value(&self.root, i, None, &nodes, &outputs, 0, &mut warnings))
            .map(|found| {
                if tiling.is_none() {
                    tiling = found.tiling;
                }
                found.value
            });
        Ok(Material {
            name: mat.attr("name").unwrap_or("Material").to_owned(),
            shader: kind,
            inputs: resolved,
            displacement,
            uv_tiling: tiling.unwrap_or([1.0, 1.0]),
            warnings,
        })
    }

    /// Resolves a surface material like [`Document::read_material`], with a wider input table, the
    /// color space in effect and the UV tiling of each image, and stricter checks.
    ///
    /// The tables add each shader's weight, specular, IOR and coat color inputs, OpenPBR's
    /// `emission_luminance`, and glTF PBR's `emissive_strength` and alpha mode inputs; glTF PBR's
    /// emission color is read from `emissive`, the name its node definition gives it. Values may
    /// be separated by commas, spaces or both, `convert` nodes follow the Standard Nodes rules,
    /// and an image inside a nodegraph takes the nodegraph's `fileprefix`. A repeated shader
    /// input, a value that is not one to four finite numbers or does not fit its type, and a
    /// `uvtiling` that is not two finite numbers are [`Error::Malformed`]. A shader input that is
    /// not in the table gives the warning `input <name> is not read`; the base weight warning of
    /// [`Document::read_material`] is not given, since the weight is read. A `normalmap` node's
    /// `scale` other than 1 and authored `normal`, `tangent`, and `bitangent` inputs are not applied
    /// and give warnings. Image inputs of `normalmap` nodes resolve as [`Resolved::NormalMap`].
    /// Three-component literal normal-map inputs are unsupported and give warnings. Other literal
    /// sizes return [`Error::Malformed`].
    /// Surface shaders may connect through graph outputs. A graph with several outputs needs a
    /// named selection; a graph with one output uses it regardless of the selection attribute.
    /// Missing node and graph-output references are errors. Supported numeric `constant` nodes
    /// use their typed zero values when `value` is omitted. Emission colors remain resolved when
    /// their strength is zero. Nonzero image `uvoffset`, nonidentity displacement `scale`, and
    /// vector displacement are reported as unsupported.
    pub fn read_material_strict(&self, material: Option<&str>) -> Result<StrictMaterial, Error> {
        let Located { mat, shader, scope, kind, nodes, graphs, outputs } = self.locate(material, true)?;
        let inputs = match kind {
            Shader::OpenPbr => OPEN_PBR_STRICT_INPUTS,
            Shader::StandardSurface => STANDARD_STRICT_INPUTS,
            Shader::GltfPbr => GLTF_STRICT_INPUTS,
        };
        let mut warnings = Vec::new();
        let mut seen = HashSet::new();
        for name in shader.children("input").filter_map(|i| i.attr("name")) {
            if !seen.insert(name) {
                return Err(Error::Malformed(format!("the shader has more than one {name} input")));
            }
            if !inputs.contains(&name) {
                warnings.push(format!("input {name} is not read"));
            }
        }
        let resolver = Resolver { root: &self.root, nodes: &nodes, graphs: &graphs, outputs: &outputs };
        let shader_space = resolver.space_of(shader, scope.as_deref());
        let mut resolved = Vec::new();
        let mut tiling = None;
        for name in inputs {
            let Some(input) = shader.children("input").find(|i| i.attr("name") == Some(*name)) else { continue };
            if let Some(value) = resolver.value(input, scope.as_deref(), shader_space, 0, &mut warnings)? {
                if tiling.is_none() {
                    tiling = image_tiling(&value);
                }
                resolved.push(((*name).to_owned(), value));
            }
        }
        let mut displacement = None;
        if let Some(connection) = mat.children("input").find(|i| i.attr("name") == Some("displacementshader")) {
            let target = resolver.target(connection, None)?;
            if let Some(node) = target.as_ref().and_then(|name| nodes.get(name).copied()) {
                let node_scope = target.as_deref().and_then(|name| name.split_once('/')).map(|(graph, _)| graph);
                let space = resolver.space_of(node, node_scope);
                if let Some(input) = node.children("input").find(|i| i.attr("name") == Some("displacement")) {
                    let value = resolver.value(input, node_scope, space, 0, &mut warnings)?;
                    if resolver.value_type(input, node_scope)? == Some("vector3")
                        || matches!(&value, Some(Resolved::Constant { values, .. }) if values.len() == 3)
                    {
                        warnings.push("vector displacement is not supported; its input was skipped".into());
                    } else {
                        displacement = value;
                    }
                }
                if let Some(scale) = node.children("input").find(|i| i.attr("name") == Some("scale")).filter(|i| has_authored_value(i)) {
                    let value = resolver.value(scale, node_scope, space, 0, &mut warnings)?;
                    match value {
                        Some(Resolved::Constant { values, .. }) if values.len() != 1 => {
                            return Err(Error::Malformed("displacement scale is not one finite number".into()));
                        }
                        Some(Resolved::Constant { values, .. }) if values == [1.0] => {}
                        _ => warnings.push("the scale of the displacement node is not read".into()),
                    }
                }
                if tiling.is_none() {
                    tiling = displacement.as_ref().and_then(image_tiling);
                }
            }
        }
        Ok(StrictMaterial {
            name: mat.attr("name").unwrap_or("Material").to_owned(),
            shader: kind,
            inputs: resolved,
            displacement,
            uv_tiling: tiling.unwrap_or([1.0, 1.0]),
            warnings,
        })
    }

    /// The surface material named `material` (or the first one), its surface shader, and the
    /// document's nodes, nodegraphs and graph outputs.
    fn locate(&self, material: Option<&str>, strict: bool) -> Result<Located<'_>, Error> {
        let materials: Vec<&El> = self.root.children("surfacematerial").collect();
        let mat = match material {
            Some(name) => materials.iter().copied().find(|m| m.attr("name") == Some(name)).ok_or_else(|| Error::MissingMaterial(name.to_owned()))?,
            None => materials.first().copied().ok_or_else(|| Error::Unsupported("document has no surfacematerial".into()))?,
        };
        let mut nodes = HashMap::new();
        let mut graphs = HashMap::new();
        let mut outputs = Vec::new();
        for child in self.root.elements() {
            if child.name == "nodegraph" {
                let Some(graph) = child.attr("name") else { continue };
                graphs.insert(graph, child);
                for node in child.elements() {
                    let Some(name) = node.attr("name") else { continue };
                    if node.name == "output" {
                        if let Some(from) = node.attr("nodename") {
                            outputs.push((format!("{graph}/{name}"), format!("{graph}/{from}")));
                        } else if strict {
                            outputs.push((format!("{graph}/{name}"), String::new()));
                        }
                    } else {
                        nodes.insert(format!("{graph}/{name}"), node);
                    }
                }
            } else if let Some(name) = child.attr("name") {
                nodes.insert(name.to_owned(), child);
            }
        }
        let connection = mat.children("input").find(|i| i.attr("name") == Some("surfaceshader"));
        let shader_name = if strict {
            let resolver = Resolver { root: &self.root, nodes: &nodes, graphs: &graphs, outputs: &outputs };
            connection.map(|input| resolver.target(input, None)).transpose()?.flatten()
        } else {
            connection.and_then(|input| input.attr("nodename")).map(str::to_owned)
        };
        let shader =
            shader_name.as_ref().and_then(|n| nodes.get(n).copied()).ok_or_else(|| Error::Malformed("material has no surface shader".into()))?;
        let scope = shader_name.as_deref().and_then(|name| name.split_once('/')).map(|(graph, _)| graph.to_owned());
        let kind = match shader.name.as_str() {
            "open_pbr_surface" => Shader::OpenPbr,
            "standard_surface" => Shader::StandardSurface,
            "gltf_pbr" => Shader::GltfPbr,
            other => return Err(Error::Unsupported(format!("surface shader is {other}"))),
        };
        Ok(Located { mat, shader, scope, kind, nodes, graphs, outputs })
    }
}

/// Whether the shader's emission is on: its strength input (`emission_luminance` or
/// `emission`) is connected or holds a value above 0, read with `parse`. glTF PBR has no such
/// input, so its emission is always on.
fn emission_enabled(kind: Shader, shader: &El, parse: fn(&str) -> Option<Vec<f32>>) -> bool {
    let gate = match kind {
        Shader::OpenPbr => Some("emission_luminance"),
        Shader::StandardSurface => Some("emission"),
        Shader::GltfPbr => None,
    };
    gate.is_none_or(|name| {
        shader
            .children("input")
            .find(|i| i.attr("name") == Some(name))
            .is_some_and(|i| i.attr("value").is_none_or(|v| parse(v).and_then(|n| n.first().copied()).is_some_and(|v| v > 0.0)))
    })
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

fn image_tiling(value: &Resolved) -> Option<[f32; 2]> {
    match value {
        Resolved::Image { uv_tiling, .. } | Resolved::NormalMap { uv_tiling, .. } => {
            uv_tiling.filter(|n| n.iter().all(|v| *v > 0.0)).map(|n| [n[0].clamp(0.001, 1000.0), n[1].clamp(0.001, 1000.0)])
        }
        Resolved::Constant { .. } => None,
    }
}

fn has_authored_value(input: &El) -> bool {
    ["value", "nodename", "nodegraph", "interfacename"].iter().any(|name| input.attr(name).is_some())
}

/// One to four finite numbers separated by commas, spaces or both, as MaterialX's own value
/// reader splits vector and color values.
fn parse_values(text: &str) -> Option<Vec<f32>> {
    let nums: Option<Vec<f32>> = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<f32>().ok().filter(|v| v.is_finite()))
        .collect();
    nums.filter(|v| !v.is_empty() && v.len() <= 4)
}

/// The value of the input `input`, read as a `boolean` (`true` or `false`, as 1 or 0) or an
/// `integer` (one whole number) when its type says so, and as numbers otherwise. A `float`,
/// vector or color type needs as many numbers as it has channels.
fn input_numbers(input: &El, text: &str) -> Option<Vec<f32>> {
    match input.attr("type") {
        Some("boolean") => match text.trim() {
            "true" => Some(vec![1.0]),
            "false" => Some(vec![0.0]),
            _ => None,
        },
        Some("integer") => text.trim().parse::<i32>().ok().map(|v| vec![v as f32]),
        ty => parse_values(text).filter(|v| ty.and_then(channels).is_none_or(|n| v.len() == n)),
    }
}

/// The number of channels of a MaterialX numeric type.
fn channels(ty: &str) -> Option<usize> {
    match ty {
        "float" | "integer" | "boolean" => Some(1),
        "vector2" => Some(2),
        "color3" | "vector3" => Some(3),
        "color4" | "vector4" => Some(4),
        _ => None,
    }
}

/// Whether `ty` is a numeric MaterialX type that is not a color: a boolean, integer, float or
/// vector. A `colorspace` does not apply to values of these types.
fn is_numeric_non_color(ty: Option<&str>) -> bool {
    ty.is_some_and(|ty| channels(ty).is_some() && !matches!(ty, "color3" | "color4"))
}

/// The type of `count` numbers whose type is not written: a float, or a vector.
fn untyped(count: usize) -> &'static str {
    match count {
        1 => "float",
        2 => "vector2",
        3 => "vector3",
        _ => "vector4",
    }
}

/// `values` of type `from` converted to type `to` by the `convert` node's rules in the MaterialX
/// Standard Nodes: a boolean becomes a float or integer of 0 or 1, an integer becomes a float or
/// a boolean (1 for any value that is not 0), a boolean, float or integer is copied to every
/// channel of a color or vector, and a color or vector keeps its first channels and fills
/// missing ones with 0 (1 in the fourth). `None` for a pair the node does not define, such as a
/// float to an integer or a color to a float, and for values that do not fit `from`.
fn converted(values: &[f32], from: &str, to: &str) -> Option<Vec<f32>> {
    if channels(from)? != values.len() {
        return None;
    }
    let out = channels(to)?;
    match (from, to) {
        _ if from == to => Some(values.to_vec()),
        ("boolean", "float" | "integer") | ("integer", "float") => Some(values.to_vec()),
        ("integer", "boolean") => Some(vec![if values[0] != 0.0 { 1.0 } else { 0.0 }]),
        ("boolean" | "float" | "integer", _) if out > 1 => Some(vec![values[0]; out]),
        _ if values.len() > 1 && out > 1 => Some((0..out).map(|c| values.get(c).copied().unwrap_or(if c == 3 { 1.0 } else { 0.0 })).collect()),
        _ => None,
    }
}

/// The document's nodes, nodegraphs and graph outputs, for following input connections.
struct Resolver<'a> {
    root: &'a El,
    nodes: &'a HashMap<String, &'a El>,
    graphs: &'a HashMap<&'a str, &'a El>,
    outputs: &'a [(String, String)],
}

impl<'a> Resolver<'a> {
    /// The color space in effect on `element`: its own, else its nodegraph's when `scope` names
    /// one, else the document's.
    fn space_of(&self, element: &'a El, scope: Option<&str>) -> Option<&'a str> {
        element
            .attr("colorspace")
            .or_else(|| scope.and_then(|g| self.graphs.get(g)).and_then(|g| g.attr("colorspace")))
            .or_else(|| self.root.attr("colorspace"))
    }

    /// The document's working color space, the root element's `colorspace`.
    fn working(&self) -> Option<&'a str> {
        self.root.attr("colorspace")
    }

    /// The value of `input`, an input of an element in nodegraph `scope` (or at the top level)
    /// whose color space in effect is `space`. A `value` that is not one to four finite numbers,
    /// or that does not fit a `boolean` or `integer` type, is [`Error::Malformed`]; an
    /// unsupported connection is skipped with a warning.
    fn value(
        &self,
        input: &'a El,
        scope: Option<&str>,
        space: Option<&'a str>,
        hops: usize,
        warnings: &mut Vec<String>,
    ) -> Result<Option<Resolved>, Error> {
        if hops > 16 {
            warnings.push("a node chain is longer than 16 nodes (or loops); skipped".into());
            return Ok(None);
        }
        if let Some(interface) = input.attr("interfacename") {
            warnings.push(format!("interface input {interface:?} is not supported; its value was skipped"));
            return Ok(None);
        }
        if let Some(value) = input.attr("value") {
            let Some(values) = input_numbers(input, value) else {
                let name = input.attr("name").unwrap_or("");
                return Err(Error::Malformed(format!("input {name} has a value that is not one to four finite numbers or does not fit its type")));
            };
            let colorspace = if is_numeric_non_color(input.attr("type")) { self.working() } else { input.attr("colorspace").or(space) };
            return Ok(Some(Resolved::Constant { values, colorspace: colorspace.map(str::to_owned) }));
        }
        let target = self.target(input, scope)?;
        let Some(node) = target.as_ref().and_then(|t| self.nodes.get(t).copied()) else { return Ok(None) };
        let node_scope = target.as_deref().and_then(|t| t.split_once('/')).map(|(g, _)| g);
        let node_space = self.space_of(node, node_scope);
        let child = |name: &str| node.children("input").find(|i| i.attr("name") == Some(name));
        let node_name = node.attr("name").unwrap_or("");
        match node.name.as_str() {
            "image" | "tiledimage" => {
                let file = child("file").and_then(|input| input.attr("value").filter(|file| !file.is_empty()).map(|file| (input, file)));
                let Some((file_input, file)) = file else {
                    warnings.push(format!("image node {node_name:?} has no nonempty literal file value; its image was skipped"));
                    return Ok(None);
                };
                let prefix = file_input
                    .attr("fileprefix")
                    .or_else(|| node.attr("fileprefix"))
                    .or_else(|| node_scope.and_then(|g| self.graphs.get(g)).and_then(|g| g.attr("fileprefix")))
                    .or_else(|| self.root.attr("fileprefix"))
                    .unwrap_or("");
                let uv_tiling = match child("uvtiling").filter(|input| has_authored_value(input)) {
                    None => None,
                    Some(input) => match self.value(input, node_scope, node_space, hops + 1, warnings)? {
                        Some(Resolved::Constant { values, .. }) if values.len() == 2 => Some([values[0], values[1]]),
                        Some(Resolved::Constant { .. }) => {
                            return Err(Error::Malformed(format!("the uvtiling of image {node_name:?} is not two finite numbers")));
                        }
                        _ => {
                            warnings.push(format!("the uvtiling of image {node_name:?} is not read"));
                            None
                        }
                    },
                };
                if let Some(offset) = child("uvoffset").filter(|input| has_authored_value(input)) {
                    match self.value(offset, node_scope, node_space, hops + 1, warnings)? {
                        Some(Resolved::Constant { values, .. }) if values.len() != 2 => {
                            return Err(Error::Malformed(format!("the uvoffset of image {node_name:?} is not two finite numbers")));
                        }
                        Some(Resolved::Constant { values, .. }) if values == [0.0, 0.0] => {}
                        _ => warnings.push(format!("the uvoffset of image {node_name:?} is not read")),
                    }
                }
                for mode in ["uaddressmode", "vaddressmode"] {
                    if let Some(input) = child(mode).filter(|input| has_authored_value(input)) {
                        match input.attr("value") {
                            Some("periodic") => {}
                            Some("constant" | "clamp" | "mirror") | None => warnings.push(format!("the {mode} of image {node_name:?} is not read")),
                            Some(_) => return Err(Error::Malformed(format!("the {mode} of image {node_name:?} is not a supported addressing mode"))),
                        }
                    }
                }
                let colorspace = if is_numeric_non_color(node.attr("type")) { self.working() } else { file_input.attr("colorspace").or(node_space) }
                    .map(str::to_owned);
                Ok(Some(Resolved::Image { file: format!("{prefix}{file}"), colorspace, uv_tiling }))
            }
            "normalmap" => {
                for name in ["normal", "tangent", "bitangent"] {
                    if child(name).is_some_and(has_authored_value) {
                        warnings.push(format!("the {name} of normalmap node {node_name:?} is not read"));
                    }
                }
                // The normal is passed on without the node's scale, so a scale other than 1 is
                // reported.
                if let Some(scale) = child("scale").filter(|input| has_authored_value(input)) {
                    let resolved = self.value(scale, node_scope, node_space, hops + 1, &mut Vec::new())?;
                    if !matches!(resolved, Some(Resolved::Constant { ref values, .. }) if values.iter().all(|s| *s == 1.0)) {
                        warnings.push(format!("the scale of normalmap node {node_name:?} is not read"));
                    }
                }
                match child("in") {
                    Some(i) => Ok(match self.value(i, node_scope, node_space, hops + 1, warnings)? {
                        Some(Resolved::Image { file, colorspace, uv_tiling }) => Some(Resolved::NormalMap { file, colorspace, uv_tiling }),
                        Some(Resolved::Constant { values, .. }) if values.len() != 3 => {
                            return Err(Error::Malformed(format!("the input of normalmap node {node_name:?} is not three finite numbers")));
                        }
                        Some(Resolved::Constant { .. }) => {
                            warnings.push(format!("the literal input of normalmap node {node_name:?} is not supported; its value was skipped"));
                            None
                        }
                        Some(Resolved::NormalMap { .. }) => {
                            warnings.push(format!("the nested normalmap input of node {node_name:?} is not supported; its value was skipped"));
                            None
                        }
                        value => value,
                    }),
                    None => Ok(None),
                }
            }
            "convert" => {
                let Some(i) = child("in").filter(|input| has_authored_value(input)) else {
                    let definition = node.attr("nodedef").and_then(|name| name.strip_prefix("ND_convert_")).and_then(|name| name.split_once('_'));
                    if let Some((from, to)) = definition
                        && from != to
                        && node.attr("type").is_none_or(|ty| ty == to)
                        && let Some(count) = channels(from)
                        && let Some(values) = converted(&vec![0.0; count], from, to)
                    {
                        return Ok(Some(Resolved::Constant { values, colorspace: self.working().map(str::to_owned) }));
                    }
                    warnings.push(format!("the input default of convert node {node_name:?} cannot be identified; its value was skipped"));
                    return Ok(None);
                };
                let Some(value) = self.value(i, node_scope, node_space, hops + 1, warnings)? else { return Ok(None) };
                let to = node.attr("type").unwrap_or("");
                let from = self.value_type(i, node_scope)?;
                match value {
                    Resolved::Constant { values, colorspace } => {
                        // A value with no type is a float or a vector, so it is in the working
                        // color space like a typed one.
                        let colorspace = if from.is_none() && i.attr("value").is_some() { self.working().map(str::to_owned) } else { colorspace };
                        let from = from.unwrap_or_else(|| untyped(values.len()));
                        match converted(&values, from, to) {
                            Some(values) => Ok(Some(Resolved::Constant { values, colorspace })),
                            None => {
                                warnings.push(format!(
                                    "a convert node ({node_name:?}) from type {from:?} to type {to:?} is not supported; its input was skipped"
                                ));
                                Ok(None)
                            }
                        }
                    }
                    // An image keeps its own channels, so only a convert that copies every
                    // channel as it is passes it on.
                    image @ (Resolved::Image { .. } | Resolved::NormalMap { .. }) => {
                        let copies = from.is_some_and(|from| from == to || channels(from).is_some_and(|n| n > 1 && channels(to) == Some(n)));
                        if copies {
                            Ok(Some(image))
                        } else {
                            warnings.push(format!(
                                "a convert node ({node_name:?}) changes an image's channels to type {to:?}, which is not supported; its input was skipped"
                            ));
                            Ok(None)
                        }
                    }
                }
            }
            "constant" => {
                let value = child("value").filter(|input| has_authored_value(input));
                match value {
                    // A connected value keeps the color space it resolved; only a written value takes
                    // the working color space.
                    Some(i) => match self.constant_value(i, node, node_scope, node_space, hops + 1, warnings)? {
                        Some(Resolved::Constant { values, .. }) if is_numeric_non_color(node.attr("type")) && i.attr("value").is_some() => {
                            Ok(Some(Resolved::Constant { values, colorspace: self.working().map(str::to_owned) }))
                        }
                        value => Ok(value),
                    },
                    None => match node.attr("type").and_then(channels) {
                        Some(count) => {
                            let space = if is_numeric_non_color(node.attr("type")) { self.working() } else { node_space };
                            Ok(Some(Resolved::Constant { values: vec![0.0; count], colorspace: space.map(str::to_owned) }))
                        }
                        None => {
                            warnings.push(format!("the default of constant node {node_name:?} has an unsupported type"));
                            Ok(None)
                        }
                    },
                }
            }
            other => {
                warnings.push(format!("a {other} node ({node_name:?}) is not supported; its input was skipped"));
                Ok(None)
            }
        }
    }

    fn constant_value(
        &self,
        input: &'a El,
        node: &'a El,
        scope: Option<&str>,
        space: Option<&'a str>,
        hops: usize,
        warnings: &mut Vec<String>,
    ) -> Result<Option<Resolved>, Error> {
        if input.attr("type").is_none()
            && input.attr("interfacename").is_none()
            && let Some(text) = input.attr("value")
            && let Some(ty) = node.attr("type")
        {
            let typed = El::new("input").with_attr("type", ty);
            let values = input_numbers(&typed, text)
                .ok_or_else(|| Error::Malformed(format!("constant {:?} has a value that does not fit {ty}", node.attr("name").unwrap_or(""))))?;
            let colorspace = if is_numeric_non_color(Some(ty)) { self.working() } else { input.attr("colorspace").or(space) };
            return Ok(Some(Resolved::Constant { values, colorspace: colorspace.map(str::to_owned) }));
        }
        self.value(input, scope, space, hops, warnings)
    }

    /// The key in `nodes` of the node that `input`, an input of an element in nodegraph `scope`
    /// (or at the top level), connects to.
    fn target(&self, input: &El, scope: Option<&str>) -> Result<Option<String>, Error> {
        let target = if let Some(graph) = input.attr("nodegraph") {
            let prefix = format!("{graph}/");
            let outputs: Vec<_> = self.outputs.iter().filter(|(name, _)| name.starts_with(&prefix)).collect();
            let selected = if outputs.len() == 1 {
                outputs.first().copied()
            } else {
                input.attr("output").and_then(|output| outputs.iter().rev().copied().find(|(name, _)| name == &format!("{graph}/{output}")))
            };
            Some(
                selected
                    .ok_or_else(|| {
                        Error::Malformed(format!("input {:?} has no selected output in nodegraph {graph:?}", input.attr("name").unwrap_or("")))
                    })?
                    .1
                    .clone(),
            )
        } else if let Some(name) = input.attr("nodename") {
            let qualified = scope.map(|g| format!("{g}/{name}"));
            Some(qualified.unwrap_or_else(|| name.to_owned()))
        } else {
            None
        };
        if let Some(name) = target.as_ref()
            && !self.nodes.contains_key(name)
        {
            return Err(Error::Malformed(format!("input {:?} references missing node {name:?}", input.attr("name").unwrap_or(""))));
        }
        Ok(target)
    }

    fn value_type(&self, input: &'a El, scope: Option<&str>) -> Result<Option<&'a str>, Error> {
        if input.attr("type").is_some() || input.attr("value").is_some() {
            return Ok(input.attr("type"));
        }
        Ok(self.target(input, scope)?.as_ref().and_then(|name| self.nodes.get(name).copied()).and_then(|node| node.attr("type")))
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
    fn displacement_image_supplies_tiling_when_surface_inputs_do_not() {
        let xml = br#"<materialx>
          <open_pbr_surface name="S"><input name="base_color" value="0.3,0.4,0.5"/></open_pbr_surface>
          <tiledimage name="H"><input name="file" value="height.png"/><input name="uvtiling" value="4,2"/></tiledimage>
          <displacement name="D"><input name="displacement" nodename="H"/></displacement>
          <surfacematerial name="M"><input name="surfaceshader" nodename="S"/><input name="displacementshader" nodename="D"/></surfacematerial>
        </materialx>"#;
        let doc = Document::parse(xml).unwrap();
        let material = doc.read_material(None).unwrap();
        assert_eq!(material.displacement, Some(Value::Image("height.png".into())));
        assert_eq!(material.uv_tiling, [4.0, 2.0]);
        let strict = doc.read_material_strict(None).unwrap();
        assert_eq!(strict.displacement, Some(Resolved::Image { file: "height.png".into(), colorspace: None, uv_tiling: Some([4.0, 2.0]) }));
        assert_eq!(strict.uv_tiling, [4.0, 2.0]);
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
        assert!(matches!(doc.read_material_strict(None), Err(Error::Malformed(_))));
        let mut explicit = doc.clone();
        let normalmap = explicit.root.elements_mut().find(|node| node.attr("name") == Some("N")).unwrap();
        normalmap.child_mut("input").unwrap().set_attr("output", "first");
        let strict = explicit.read_material_strict(None).unwrap();
        assert_eq!(
            strict.inputs[0],
            ("base_color".into(), Resolved::Image { file: "tex/albedo.png".into(), colorspace: None, uv_tiling: Some([2.0, 4.0]) })
        );
        assert_eq!(
            strict.inputs[1],
            ("geometry_normal".into(), Resolved::NormalMap { file: "tex/normal.png".into(), colorspace: None, uv_tiling: None })
        );
        assert_eq!(strict.inputs[2].1, Resolved::Constant { values: vec![1.0, 0.5, 0.25], colorspace: None });
        assert_eq!(strict.displacement, Some(Resolved::Constant { values: vec![0.25], colorspace: None }));
        assert_eq!(strict.uv_tiling, [2.0, 4.0]);
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
        let strict = doc.read_material_strict(None).unwrap();
        let image = |file: &str, space: &str| Resolved::Image { file: file.into(), colorspace: Some(space.into()), uv_tiling: None };
        assert_eq!(strict.inputs[0], ("base_color".into(), image("albedo<&.png", "srgb_texture")));
        assert_eq!(strict.inputs[1], ("specular_roughness".into(), image("rough.png", "lin_rec709")));
        assert_eq!(
            strict.inputs[2],
            ("geometry_normal".into(), Resolved::NormalMap { file: "normal.png".into(), colorspace: Some("lin_rec709".into()), uv_tiling: None })
        );
        assert_eq!(strict.displacement, Some(image("height.png", "lin_rec709")));
        assert!(xml.contains("colorspace=\"srgb_texture\""));
        assert!(xml.contains("doc=\"Tiles every 100 x 50 cm\""));
        assert!(!xml.contains("colorspace=\"ignored\""));
    }

    #[test]
    fn read_material_keeps_its_input_table_values_and_lenient_checks() {
        // `read_material` reads glTF PBR's `emission`, keeps the base weight warning, skips
        // repeated inputs, bad values and bad `uvtiling` instead of failing, and gives a
        // constant only its own `colorspace`; `read_material_strict` is the stricter read.
        let xml = r#"<materialx colorspace="srgb_texture">
          <gltf_pbr name="G"><input name="emission" value="1,0.5,0"/><input name="emissive" value="0.5,0.5,0"/><input name="roughness" value="0.25"/>
            <input name="roughness" value="0.75"/><input name="metallic" value="nan"/><input name="base_color" nodename="T"/><input name="alpha" value="0.5"/></gltf_pbr>
          <tiledimage name="T"><input name="file" value="a.png"/><input name="uvtiling" value="x"/></tiledimage>
          <open_pbr_surface name="O"><input name="base_weight" value="0.5"/><input name="base_color" value="0.5 0.5 0.5"/></open_pbr_surface>
          <surfacematerial name="M"><input name="surfaceshader" nodename="G"/></surfacematerial>
          <surfacematerial name="N"><input name="surfaceshader" nodename="O"/></surfacematerial>
        </materialx>"#;
        let doc = Document::parse(xml.as_bytes()).unwrap();
        let gltf = doc.read_material(Some("M")).unwrap();
        assert_eq!(
            gltf.inputs,
            vec![
                ("base_color".into(), Value::Image("a.png".into())),
                ("roughness".into(), Value::Constant { values: vec![0.25], colorspace: None }),
                ("alpha".into(), Value::Constant { values: vec![0.5], colorspace: None }),
                ("emission".into(), Value::Constant { values: vec![1.0, 0.5, 0.0], colorspace: None }),
            ]
        );
        assert_eq!(gltf.uv_tiling, [1.0, 1.0]);
        assert!(gltf.warnings.is_empty(), "{:?}", gltf.warnings);
        assert!(matches!(doc.read_material_strict(Some("M")), Err(Error::Malformed(_))));
        let open = doc.read_material(Some("N")).unwrap();
        assert_eq!(open.warnings, vec!["the base weight isn't 1; it was ignored".to_owned()]);
        assert!(open.inputs.is_empty(), "{:?}", open.inputs);
        let strict = doc.read_material_strict(Some("N")).unwrap();
        assert!(strict.warnings.is_empty(), "{:?}", strict.warnings);
        assert_eq!(input(&strict, "base_weight"), Some(&constant(&[0.5], Some("srgb_texture"))));
        assert_eq!(input(&strict, "base_color"), Some(&constant(&[0.5; 3], Some("srgb_texture"))));
    }

    fn read(xml: &str) -> Result<StrictMaterial, Error> {
        Document::parse(xml.as_bytes())?.read_material_strict(None)
    }

    fn input<'m>(material: &'m StrictMaterial, name: &str) -> Option<&'m Resolved> {
        material.inputs.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    fn constant(values: &[f32], space: Option<&str>) -> Resolved {
        Resolved::Constant { values: values.to_vec(), colorspace: space.map(str::to_owned) }
    }

    #[test]
    fn gltf_pbr_reads_the_emissive_input_its_definition_names() {
        // The gltf_pbr node definition calls its emission color `emissive`
        // (https://github.com/AcademySoftwareFoundation/MaterialX/blob/main/libraries/bxdf/gltf_pbr.mtlx).
        let m = read(
            r#"<materialx><gltf_pbr name="S"><input name="emissive" type="color3" value="0.5,0.25,0"/><input name="emissive_strength" type="float" value="2"/></gltf_pbr>
            <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#,
        )
        .unwrap();
        assert_eq!(input(&m, "emissive"), Some(&constant(&[0.5, 0.25, 0.0], None)));
        assert_eq!(input(&m, "emissive_strength"), Some(&constant(&[2.0], None)));
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    }

    #[test]
    fn weights_ior_and_alpha_inputs_are_read_for_each_shader() {
        let open = read(
            r#"<materialx><open_pbr_surface name="S"><input name="base_weight" value="0.5"/><input name="specular_weight" value="2"/>
            <input name="specular_color" value="1,0.5,0.5"/><input name="specular_ior" value="1.33"/><input name="coat_color" value="0.9,0.9,1"/>
            <input name="coat_ior" value="1.4"/><input name="emission_luminance" value="10"/></open_pbr_surface>
            <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#,
        )
        .unwrap();
        for (name, values) in [
            ("base_weight", &[0.5][..]),
            ("specular_weight", &[2.0]),
            ("specular_color", &[1.0, 0.5, 0.5]),
            ("specular_ior", &[1.33]),
            ("coat_color", &[0.9, 0.9, 1.0]),
            ("coat_ior", &[1.4]),
            ("emission_luminance", &[10.0]),
        ] {
            assert_eq!(input(&open, name), Some(&constant(values, None)), "{name}");
        }
        assert!(open.warnings.is_empty(), "{:?}", open.warnings);
        let standard = read(
            r#"<materialx><standard_surface name="S"><input name="base" value="0.8"/><input name="specular" value="0.5"/><input name="specular_IOR" value="1.6"/>
            <input name="coat_IOR" value="1.3"/><input name="emission" value="0"/></standard_surface>
            <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#,
        )
        .unwrap();
        for (name, value) in [("base", 0.8), ("specular", 0.5), ("specular_IOR", 1.6), ("coat_IOR", 1.3), ("emission", 0.0)] {
            assert_eq!(input(&standard, name), Some(&constant(&[value], None)), "{name}");
        }
        let gltf = read(
            r#"<materialx><gltf_pbr name="S"><input name="alpha_mode" type="integer" value="1"/><input name="alpha_cutoff" value="0.25"/><input name="ior" value="1.7"/>
            <input name="specular" value="0.5"/><input name="specular_color" value="1,1,0"/></gltf_pbr>
            <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#,
        )
        .unwrap();
        for (name, values) in
            [("alpha_mode", &[1.0][..]), ("alpha_cutoff", &[0.25]), ("ior", &[1.7]), ("specular", &[0.5]), ("specular_color", &[1.0, 1.0, 0.0])]
        {
            assert_eq!(input(&gltf, name), Some(&constant(values, None)), "{name}");
        }
    }

    #[test]
    fn constants_and_images_carry_the_color_space_in_effect_where_they_are_written() {
        // An untagged value takes the nearest enclosing element's color space, then the
        // document's (MaterialX specification, Color Spaces and Color Management Systems).
        let m = read(
            r#"<materialx colorspace="srgb_texture">
              <open_pbr_surface name="S">
                <input name="base_color" value="0.5,0.5,0.5"/>
                <input name="specular_color" value="0.5,0.5,0.5" colorspace="lin_rec709"/>
                <input name="coat_color" nodegraph="g" output="c"/>
                <input name="fuzz_color" nodename="T"/>
              </open_pbr_surface>
              <tiledimage name="T" colorspace="lin_rec709"><input name="file" value="fuzz.png"/></tiledimage>
              <nodegraph name="g" colorspace="g22_rec709"><constant name="k"><input name="value" value="0.1,0.2,0.3"/></constant><output name="c" nodename="k"/></nodegraph>
              <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial>
            </materialx>"#,
        )
        .unwrap();
        assert_eq!(input(&m, "base_color"), Some(&constant(&[0.5; 3], Some("srgb_texture"))));
        assert_eq!(input(&m, "specular_color"), Some(&constant(&[0.5; 3], Some("lin_rec709"))));
        assert_eq!(input(&m, "coat_color"), Some(&constant(&[0.1, 0.2, 0.3], Some("g22_rec709"))));
        assert_eq!(
            input(&m, "fuzz_color"),
            Some(&Resolved::Image { file: "fuzz.png".into(), colorspace: Some("lin_rec709".into()), uv_tiling: None })
        );
        let doc = Document::parse(
            br#"<materialx colorspace="srgb_texture"><open_pbr_surface name="Tagged" colorspace="acescg"><input name="base_color" value="0.2,0.2,0.2"/></open_pbr_surface>
            <surfacematerial name="N"><input name="surfaceshader" nodename="Tagged"/></surfacematerial></materialx>"#,
        )
        .unwrap();
        let tagged = doc.read_material_strict(Some("N")).unwrap();
        assert_eq!(input(&tagged, "base_color"), Some(&constant(&[0.2; 3], Some("acescg"))));
    }

    #[test]
    fn duplicate_inputs_and_values_that_are_not_finite_numbers_are_malformed() {
        let shader = |inputs: &str| {
            format!(
                r#"<materialx><open_pbr_surface name="S">{inputs}</open_pbr_surface><surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#
            )
        };
        let dup = read(&shader(r#"<input name="base_metalness" value="0"/><input name="base_metalness" value="1"/>"#));
        assert!(matches!(dup, Err(Error::Malformed(ref m)) if m.contains("base_metalness")), "{dup:?}");
        for bad in ["nan", "inf", "-inf", "1e40", "0.5,x", "", "1,2,3,4,5"] {
            let r = read(&shader(&format!(r#"<input name="specular_roughness" value="{bad}"/>"#)));
            assert!(matches!(r, Err(Error::Malformed(ref m)) if m.contains("specular_roughness")), "{bad}: {r:?}");
        }
    }

    #[test]
    fn a_normalmap_scale_other_than_1_gives_a_warning() {
        let with_scale = |scale: &str| {
            read(&open_pbr(
                r#"<input name="geometry_normal" nodename="N"/>"#,
                &format!(r#"<normalmap name="N" type="vector3">{scale}<input name="in" nodename="T"/></normalmap><image name="T" type="vector3"><input name="file" value="n.png"/></image>"#),
            ))
            .unwrap()
        };
        let scaled = with_scale(r#"<input name="scale" type="float" value="0.5"/>"#);
        assert_eq!(scaled.warnings, vec![r#"the scale of normalmap node "N" is not read"#.to_owned()]);
        assert!(matches!(input(&scaled, "geometry_normal"), Some(Resolved::NormalMap { .. })));
        for plain in ["", r#"<input name="scale" type="float" value="1"/>"#, r#"<input name="scale" type="vector2" value="1, 1"/>"#] {
            assert!(with_scale(plain).warnings.is_empty(), "{plain}");
        }
        // A scale connected to a constant node is read through it.
        let connected = |value: &str| {
            read(&open_pbr(
                r#"<input name="geometry_normal" nodename="N"/>"#,
                &format!(
                    r#"<normalmap name="N" type="vector3"><input name="scale" nodename="K"/><input name="in" nodename="T"/></normalmap><image name="T" type="vector3"><input name="file" value="n.png"/></image><constant name="K" type="float"><input name="value" value="{value}"/></constant>"#
                ),
            ))
            .unwrap()
        };
        assert!(connected("1").warnings.is_empty(), "{:?}", connected("1").warnings);
        assert_eq!(connected("2").warnings.len(), 1, "{:?}", connected("2").warnings);
    }

    #[test]
    fn shader_inputs_that_are_not_read_give_a_warning() {
        let m = read(
            r#"<materialx><open_pbr_surface name="S"><input name="thin_film_weight" value="0.5"/><input name="base_metalness" value="1"/></open_pbr_surface>
            <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#,
        )
        .unwrap();
        assert_eq!(m.warnings, vec!["input thin_film_weight is not read".to_owned()]);
        assert_eq!(input(&m, "base_metalness"), Some(&constant(&[1.0], None)));
    }

    fn open_pbr(inputs: &str, rest: &str) -> String {
        format!(
            r#"<materialx><open_pbr_surface name="S">{inputs}</open_pbr_surface>{rest}<surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#
        )
    }

    #[test]
    fn vector_values_may_be_separated_by_commas_spaces_or_both() {
        // MaterialX's value reader splits vector and color values on commas and spaces
        // (ARRAY_VALID_SEPARATORS, ", ", in source/MaterialXCore/Types.cpp).
        for text in ["0.5 0.25 0.125", "0.5, 0.25, 0.125", "0.5,0.25,0.125", " 0.5  0.25 ,0.125 "] {
            let m = read(&open_pbr(&format!(r#"<input name="base_color" type="color3" value="{text}"/>"#), "")).unwrap();
            assert_eq!(input(&m, "base_color"), Some(&constant(&[0.5, 0.25, 0.125], None)), "{text:?}");
        }
        // A color3 holds three numbers.
        for bad in ["0.5 x 1", "0.5 0.25 0.125 1 2", " ", "0.5", "0.5, 0.25", "0.5 0.25 0.125 1"] {
            let r = read(&open_pbr(&format!(r#"<input name="base_color" type="color3" value="{bad}"/>"#), ""));
            assert!(matches!(r, Err(Error::Malformed(ref m)) if m.contains("base_color")), "{bad:?}: {r:?}");
        }
    }

    #[test]
    fn a_nodegraph_file_prefix_applies_to_the_images_inside_it() {
        // The nearest enclosing fileprefix applies (MaterialX specification, File Prefixes).
        let m = read(&open_pbr(
            r#"<input name="base_color" nodegraph="g" output="c"/><input name="specular_roughness" nodegraph="g" output="r"/><input name="base_metalness" nodename="T"/>"#,
            r#"<nodegraph name="g" fileprefix="stone/">
                 <tiledimage name="a" type="color3"><input name="file" type="filename" value="albedo.png"/></tiledimage>
                 <tiledimage name="b" type="float" fileprefix="own/"><input name="file" type="filename" value="rough.png"/></tiledimage>
                 <output name="c" nodename="a"/><output name="r" nodename="b"/>
               </nodegraph>
               <tiledimage name="T" type="float"><input name="file" type="filename" value="metal.png"/></tiledimage>"#,
        )
        .replacen("<materialx>", r#"<materialx fileprefix="shared/">"#, 1))
        .unwrap();
        let file = |name: &str| match input(&m, name) {
            Some(Resolved::Image { file, .. }) => file.clone(),
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(
            (file("base_color"), file("specular_roughness"), file("base_metalness")),
            ("stone/albedo.png".into(), "own/rough.png".into(), "shared/metal.png".into())
        );
    }

    #[test]
    fn uv_tiling_is_kept_as_written_and_malformed_tiling_is_an_error() {
        let tiled = |tiling: &str| {
            read(&open_pbr(
                r#"<input name="base_color" nodename="T"/>"#,
                &format!(
                    r#"<tiledimage name="T" type="color3"><input name="file" value="a.png"/><input name="uvtiling" type="vector2" value="{tiling}"/></tiledimage>"#
                ),
            ))
        };
        let tiling_of = |m: &StrictMaterial| match input(m, "base_color") {
            Some(Resolved::Image { uv_tiling, .. }) => *uv_tiling,
            other => panic!("{other:?}"),
        };
        let wide = tiled("2000, 2").unwrap();
        assert_eq!((tiling_of(&wide), wide.uv_tiling), (Some([2000.0, 2.0]), [1000.0, 2.0]));
        let flat = tiled("0 2").unwrap();
        assert_eq!((tiling_of(&flat), flat.uv_tiling), (Some([0.0, 2.0]), [1.0, 1.0]));
        for bad in ["x", "1", "1,2,3", "nan,1"] {
            let r = tiled(bad);
            assert!(matches!(r, Err(Error::Malformed(ref m)) if m.contains("uvtiling")), "{bad:?}: {r:?}");
        }
    }

    #[test]
    fn convert_nodes_change_the_number_of_channels() {
        // The convert node's rules in the MaterialX Standard Nodes: a float is copied to every
        // channel, a shorter color or vector is filled with 0 (1 in the fourth channel), a
        // longer one keeps its first channels, and a boolean is 0 or 1.
        let via = |ty: &str, in_ty: &str, value: &str| {
            read(&open_pbr(
                r#"<input name="base_color" nodename="C"/>"#,
                &format!(r#"<convert name="C" type="{ty}"><input name="in" type="{in_ty}" value="{value}"/></convert>"#),
            ))
            .unwrap()
        };
        assert_eq!(input(&via("color3", "float", "0.25"), "base_color"), Some(&constant(&[0.25; 3], None)));
        assert_eq!(input(&via("color4", "color3", "0.1,0.2,0.3"), "base_color"), Some(&constant(&[0.1, 0.2, 0.3, 1.0], None)));
        assert_eq!(input(&via("color3", "color4", "0.1,0.2,0.3,0.4"), "base_color"), Some(&constant(&[0.1, 0.2, 0.3], None)));
        assert_eq!(input(&via("vector3", "vector2", "0.1,0.2"), "base_color"), Some(&constant(&[0.1, 0.2, 0.0], None)));
        assert_eq!(input(&via("float", "boolean", "true"), "base_color"), Some(&constant(&[1.0], None)));
        assert_eq!(input(&via("float", "integer", "3"), "base_color"), Some(&constant(&[3.0], None)));
        assert_eq!(input(&via("boolean", "integer", "2"), "base_color"), Some(&constant(&[1.0], None)));
        // An input with no type is a float or a vector by its count.
        let untyped = read(&open_pbr(
            r#"<input name="base_color" nodename="C"/>"#,
            r#"<convert name="C" type="color3"><input name="in" value="0.5"/></convert>"#,
        ))
        .unwrap();
        assert_eq!(input(&untyped, "base_color"), Some(&constant(&[0.5; 3], None)));
        // A color to a float and a float to an integer are not conversions the node defines.
        for (ty, in_ty, value) in [("float", "color3", "0.1,0.2,0.3"), ("integer", "float", "0.5"), ("boolean", "float", "1")] {
            let unsupported = via(ty, in_ty, value);
            assert_eq!(input(&unsupported, "base_color"), None, "{in_ty} to {ty}");
            assert!(unsupported.warnings.iter().any(|w| w.contains("convert node")), "{:?}", unsupported.warnings);
        }
        // An integer is one whole number.
        for bad in ["1.5", "1.00000001", "1 2"] {
            let r = read(&open_pbr(
                r#"<input name="base_color" nodename="C"/>"#,
                &format!(r#"<convert name="C" type="boolean"><input name="in" type="integer" value="{bad}"/></convert>"#),
            ));
            assert!(matches!(r, Err(Error::Malformed(ref m)) if m.contains("input in ")), "{bad:?}: {r:?}");
        }
        let image = |ty: &str, image_ty: &str| {
            read(&open_pbr(
                r#"<input name="base_color" nodename="C"/>"#,
                &format!(
                    r#"<convert name="C" type="{ty}"><input name="in" nodename="T"/></convert><image name="T" type="{image_ty}"><input name="file" value="g.png"/></image>"#
                ),
            ))
            .unwrap()
        };
        let g = Resolved::Image { file: "g.png".into(), colorspace: None, uv_tiling: None };
        assert_eq!(input(&image("float", "float"), "base_color"), Some(&g));
        assert_eq!(input(&image("vector3", "color3"), "base_color"), Some(&g));
        for (ty, image_ty) in [("color3", "float"), ("color3", "color4")] {
            let changed = image(ty, image_ty);
            assert_eq!(input(&changed, "base_color"), None, "{image_ty} to {ty}");
            assert!(changed.warnings.iter().any(|w| w.contains("convert node")), "{:?}", changed.warnings);
        }
    }

    #[test]
    fn a_color_converted_from_another_type_is_in_the_working_color_space() {
        // A colorspace applies to color values and color image files (MaterialX specification,
        // Color Spaces and Color Management Systems), so a float or vector copied into a color
        // by a convert node holds working-space channels; a color keeps its own color space.
        let m = read(
            r#"<materialx colorspace="lin_rec709">
              <open_pbr_surface name="S">
                <input name="base_color" nodename="F"/>
                <input name="specular_color" nodegraph="g" output="v"/>
                <input name="coat_color" nodename="K"/>
                <input name="fuzz_color" nodename="I"/>
                <input name="emission_color" nodename="J"/>
                <input name="emission_luminance" value="1"/>
              </open_pbr_surface>
              <convert name="F" type="color3" colorspace="srgb_texture"><input name="in" type="float" value="0.5"/></convert>
              <nodegraph name="g" colorspace="g22_rec709"><convert name="c" type="color3"><input name="in" type="vector3" value="0.1,0.2,0.3"/></convert><output name="v" nodename="c"/></nodegraph>
              <convert name="K" type="color3" colorspace="srgb_texture"><input name="in" type="color4" value="0.1,0.2,0.3,1"/></convert>
              <convert name="I" type="color3"><input name="in" nodename="V"/></convert>
              <image name="V" type="vector3" colorspace="srgb_texture"><input name="file" value="v.png"/></image>
              <convert name="J" type="color3"><input name="in" nodename="C"/></convert>
              <image name="C" type="color3" colorspace="srgb_texture"><input name="file" value="c.png"/></image>
              <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial>
            </materialx>"#,
        )
        .unwrap();
        let image = |file: &str, space: &str| Resolved::Image { file: file.into(), colorspace: Some(space.into()), uv_tiling: None };
        assert_eq!(input(&m, "base_color"), Some(&constant(&[0.5; 3], Some("lin_rec709"))));
        assert_eq!(input(&m, "specular_color"), Some(&constant(&[0.1, 0.2, 0.3], Some("lin_rec709"))));
        assert_eq!(input(&m, "coat_color"), Some(&constant(&[0.1, 0.2, 0.3], Some("srgb_texture"))));
        assert_eq!(input(&m, "fuzz_color"), Some(&image("v.png", "lin_rec709")));
        assert_eq!(input(&m, "emission_color"), Some(&image("c.png", "srgb_texture")));
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        // A color passed through a vector, also by way of a vector constant node, keeps its color
        // space, and a float constant node or an untyped value is in the working color space.
        let chain = read(
            r#"<materialx colorspace="lin_rec709">
              <open_pbr_surface name="S">
                <input name="base_color" nodename="B"/>
                <input name="specular_color" nodename="K2"/>
                <input name="coat_color" nodename="U"/>
                <input name="fuzz_color" nodename="B3"/>
              </open_pbr_surface>
              <convert name="A" type="vector3"><input name="in" type="color3" value="0.5,0.5,0.5" colorspace="srgb_texture"/></convert>
              <convert name="B" type="color3"><input name="in" nodename="A"/></convert>
              <convert name="A3" type="vector3"><input name="in" type="color3" value="0.4,0.4,0.4" colorspace="srgb_texture"/></convert>
              <constant name="V3" type="vector3"><input name="value" nodename="A3"/></constant>
              <convert name="B3" type="color3"><input name="in" nodename="V3"/></convert>
              <constant name="K" type="float" colorspace="srgb_texture"><input name="value" value="0.25"/></constant>
              <convert name="K2" type="color3" colorspace="srgb_texture"><input name="in" nodename="K"/></convert>
              <convert name="U" type="color3" colorspace="srgb_texture"><input name="in" value="0.75"/></convert>
              <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial>
            </materialx>"#,
        )
        .unwrap();
        assert_eq!(input(&chain, "base_color"), Some(&constant(&[0.5; 3], Some("srgb_texture"))));
        assert_eq!(input(&chain, "specular_color"), Some(&constant(&[0.25; 3], Some("lin_rec709"))));
        assert_eq!(input(&chain, "coat_color"), Some(&constant(&[0.75; 3], Some("lin_rec709"))));
        assert_eq!(input(&chain, "fuzz_color"), Some(&constant(&[0.4; 3], Some("srgb_texture"))));
        assert!(chain.warnings.is_empty(), "{:?}", chain.warnings);
        // A document without a working color space leaves the converted color untagged.
        let plain = read(&open_pbr(
            r#"<input name="base_color" nodename="F"/>"#,
            r#"<convert name="F" type="color3" colorspace="srgb_texture"><input name="in" type="float" value="0.5"/></convert>"#,
        ))
        .unwrap();
        assert_eq!(input(&plain, "base_color"), Some(&constant(&[0.5; 3], None)));
    }

    #[test]
    fn an_emission_color_is_checked_while_the_emission_is_0() {
        let standard = |color: &str| {
            read(&format!(
                r#"<materialx><standard_surface name="S"><input name="emission" value="0"/><input name="emission_color" value="{color}"/></standard_surface>
                <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#
            ))
        };
        let r = standard("0.5,x,1");
        assert!(matches!(r, Err(Error::Malformed(ref m)) if m.contains("emission_color")), "{r:?}");
        let m = standard("0.5,0.5,1").unwrap();
        assert_eq!(input(&m, "emission_color"), Some(&constant(&[0.5, 0.5, 1.0], None)));
        assert!(m.warnings.is_empty());
    }
}
