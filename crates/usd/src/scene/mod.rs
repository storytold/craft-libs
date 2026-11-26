//! Neutral USDA/USDC scene, prim, and layer read/write.
//!
//! Parses `.usda` (text) and `.usdc` (Crate binary) layers into a neutral [`Layer`](crate::scene::Layer)
//! representation (keeping `def`/`over`/`class` specifiers, internal references, coordinate
//! metadata, payload/sublayer/variant warnings, and decoded byte budgets), and writes neutral
//! [`UsdLayer`](crate::scene::UsdLayer) scenes to `.usda` text or `.usdc` Crate binary.

pub mod usda;
pub mod usdc;
pub mod write;

use std::collections::HashMap;

use crate::codec;

/// Default memory budget for decoded layer values (2 GiB).
pub const MAX_DECODED_BYTES: usize = 1 << 31;
/// Deepest prim nesting and reference chain followed.
pub const MAX_DEPTH: usize = 128;

/// Error returned when reading or writing a USD layer or scene.
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    /// Syntax, framing, or structural error in the input.
    #[error("malformed USD: {0}")]
    Malformed(String),
    /// Decoded data or table size exceeds the configured budget or limit.
    #[error("{0}")]
    TooLarge(String),
    /// Unsupported USD Crate version.
    #[error("unsupported USD: {0}")]
    Unsupported(String),
    /// Invalid scene structure when writing (for example, an unknown target path).
    #[error("invalid USD: {0}")]
    Invalid(String),
}

/// Result alias for USD scene operations.
pub type Result<T> = std::result::Result<T, SceneError>;

/// Charges decoded data against a byte ceiling.
#[derive(Clone, Debug)]
pub struct Budget(pub usize);

impl Budget {
    /// Create a budget with [`MAX_DECODED_BYTES`].
    pub fn new() -> Self {
        Self(MAX_DECODED_BYTES)
    }

    /// Create a budget with a custom byte limit.
    pub fn with_limit(limit: usize) -> Self {
        Self(limit)
    }

    /// Deduct `bytes` from the remaining budget, failing with [`SceneError::TooLarge`] if exceeded.
    pub fn take(&mut self, bytes: usize) -> Result<()> {
        self.0 = self
            .0
            .checked_sub(bytes)
            .ok_or_else(|| SceneError::TooLarge(format!("the USD file decodes to more than {} MiB", MAX_DECODED_BYTES >> 20)))?;
        Ok(())
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}

/// A decoded value: numbers (scalars, vectors, quaternions as `w x y z`, matrices row-major,
/// and bools all flattened), tokens / strings / asset paths, or prim/property paths.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Num(Vec<f64>),
    Tok(Vec<String>),
    Path(Vec<String>),
}

/// A decoded prim property (attribute or relationship).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prop {
    /// Declared type (`point3f[]`, `texCoord2f[]`, `rel`, ...).
    pub type_name: String,
    pub value: Option<Val>,
    pub interpolation: Option<String>,
    /// The first connection source (`name.connect`): an attribute path.
    pub connect: Option<String>,
}

/// Prim specifier (`def`, `over`, `class`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Specifier {
    #[default]
    Def,
    Over,
    Class,
}

/// One prim spec of a decoded [`Layer`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrimSpec {
    pub name: String,
    pub path: String,
    pub type_name: String,
    pub specifier: Specifier,
    pub active: Option<bool>,
    pub props: HashMap<String, Prop>,
    pub children: Vec<usize>,
    /// Internal references: absolute prim paths in this layer.
    pub references: Vec<String>,
}

/// A parsed USD layer: prim specs (index 0 is the pseudo-root `/`) and stage metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub prims: Vec<PrimSpec>,
    pub meters_per_unit: Option<f64>,
    pub up_axis: Option<String>,
    pub warnings: Vec<String>,
}

impl Layer {
    /// Create an empty layer with a pseudo-root prim at index 0 (`"/"`).
    pub fn new() -> Self {
        Self { prims: vec![PrimSpec { path: "/".into(), ..PrimSpec::default() }], meters_per_unit: None, up_axis: None, warnings: Vec::new() }
    }

    /// Add a prim spec named `name` under prim `parent` and return its index.
    pub fn add_prim(&mut self, parent: usize, name: &str) -> usize {
        let path = match self.prims.get(parent) {
            Some(p) if p.path == "/" => format!("/{name}"),
            Some(p) => format!("{}/{name}", p.path),
            None => format!("/{name}"),
        };
        let id = self.prims.len();
        self.prims.push(PrimSpec { name: name.into(), path, ..PrimSpec::default() });
        if let Some(p) = self.prims.get_mut(parent) {
            p.children.push(id);
        }
        id
    }

    /// Record a deduplicated warning message on the layer.
    pub fn warn(&mut self, msg: &str) {
        if !self.warnings.iter().any(|w| w == msg) {
            self.warnings.push(msg.into());
        }
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self::new()
    }
}

/// Read a `.usda` or `.usdc` layer from `bytes` using [`Budget::new()`].
pub fn read_layer(bytes: &[u8]) -> Result<Layer> {
    let mut budget = Budget::new();
    read_layer_with_budget(bytes, &mut budget)
}

/// Read a `.usda` or `.usdc` layer from `bytes`, charging decoded allocations to `budget`.
pub fn read_layer_with_budget(bytes: &[u8], budget: &mut Budget) -> Result<Layer> {
    if bytes.starts_with(codec::MAGIC) {
        usdc::read(bytes, budget)
    } else if bytes.trim_ascii_start().starts_with(b"#usda") {
        usda::read(bytes, budget)
    } else {
        Err(SceneError::Malformed("neither a USD text layer (#usda) nor a Crate binary (PXR-USDC)".into()))
    }
}

/// A value written to a [`UsdLayer`].
#[derive(Clone, Debug, PartialEq)]
pub enum UsdValue {
    Token(String),
    Str(String),
    Int(i32),
    Double(f64),
    Asset(String),
    Float4([f32; 4]),
    Ints(Vec<i32>),
    Vec2fs(Vec<[f32; 2]>),
    Vec3fs(Vec<[f32; 3]>),
    /// A relationship target or attribute connection (absolute path).
    Target(String),
}

/// An attribute (or, with type `"rel"`, a relationship) to write.
#[derive(Clone, Debug, PartialEq)]
pub struct UsdAttr {
    pub name: String,
    pub type_name: String,
    pub uniform: bool,
    pub value: Option<UsdValue>,
    /// Connection source of an attribute (`name.connect`).
    pub connect: Option<String>,
    pub interpolation: Option<String>,
}

impl UsdAttr {
    /// Create a new attribute with `name`, `type_name`, and optional `value`.
    pub fn new(name: &str, type_name: &str, value: Option<UsdValue>) -> Self {
        Self { name: name.into(), type_name: type_name.into(), uniform: false, value, connect: None, interpolation: None }
    }

    /// Mark this attribute as `uniform`.
    pub fn uniform(mut self) -> Self {
        self.uniform = true;
        self
    }

    /// Set the `interpolation` metadata on this attribute.
    pub fn with_interpolation(mut self, interpolation: &str) -> Self {
        self.interpolation = Some(interpolation.into());
        self
    }

    /// Create a relationship (`rel`) targeting `target`.
    pub fn rel(name: &str, target: String) -> Self {
        Self::new(name, "rel", Some(UsdValue::Target(target)))
    }

    /// Create a connected attribute (`name.connect = <source>`).
    pub fn connected(name: &str, type_name: &str, source: String) -> Self {
        Self { connect: Some(source), ..Self::new(name, type_name, None) }
    }
}

/// A prim to write into a [`UsdLayer`].
#[derive(Clone, Debug, PartialEq)]
pub struct UsdPrim {
    pub name: String,
    pub type_name: String,
    /// Applies `MaterialBindingAPI` (when the prim has a `material:binding`).
    pub binding_api: bool,
    pub attrs: Vec<UsdAttr>,
    pub children: Vec<UsdPrim>,
}

impl UsdPrim {
    /// Create a new `def` prim with `name` and `type_name`.
    pub fn new(name: &str, type_name: &str) -> Self {
        Self { name: name.into(), type_name: type_name.into(), binding_api: false, attrs: Vec::new(), children: Vec::new() }
    }
}

/// A neutral writable USD layer with its root prim and stage metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct UsdLayer {
    pub root: UsdPrim,
    pub doc: Option<String>,
    pub meters_per_unit: f64,
    pub up_axis: String,
}

impl UsdLayer {
    /// Create a writable layer with `root` as the default prim, `metersPerUnit = 1`, `upAxis = "Y"`.
    pub fn new(root: UsdPrim) -> Self {
        Self { root, doc: None, meters_per_unit: 1.0, up_axis: "Y".into() }
    }
}

/// Format `layer` as `.usda` text.
pub fn write_usda(layer: &UsdLayer) -> String {
    write::usda(layer)
}

/// Encode `layer` as a `.usdc` Crate binary.
pub fn write_usdc(layer: &UsdLayer) -> Result<Vec<u8>> {
    write::usdc(layer)
}
