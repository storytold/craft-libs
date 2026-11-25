//! StencilCraft document model and domain validation.
//!
//! A document holds an artboard in millimetres, kerf settings and layers of shapes.
//! Every read path returns `Result` with a calm, printable error: a malformed file
//! must never crash (`AGENTS.md` › "Never crash"), because it is untrusted input.

pub mod canvas;
pub mod geom;
pub mod metadata;
pub mod migrate;
pub mod wire_v1;

use crate::geom::{GeomError, bounds_of_shape, check_finite_mm};
pub use canvas::{Grid, Group, GroupChild, Guide, GuideAxis, Metadata, Orientation, Page, PagePreset, PageSetup, SnapSettings};
use kurbo::BezPath;
use serde::{Deserialize, Serialize};

pub mod production;
pub use metadata::{LibraryIndex, LibraryTrashEntry, MAX_RECENT_FILES, RecentFiles};
pub use migrate::OperationDefaults;
pub use production::{
    CombineOp, GroupKind, Operation, OperationKind, OperationSettings, Process, SendAssign, ShapeProduction, SourceGroup, Strategy, Tab,
};

/// Native format identity and supported version, defined by the shared wire-format crate.
pub use craft_stencilcraft_format::{FORMAT_NAME, FORMAT_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },

    #[error("{path}: not a stencilcraft file")]
    NotAStencilcraftFile { path: String },

    #[error("{path}: format version {found}, this build reads {expected}")]
    FormatVersion { path: String, found: u64, expected: u64 },

    #[error("{path}: {source}")]
    Parse { path: String, source: serde_json::Error },

    #[error("{path}: {message}")]
    Structure { path: String, message: String },

    #[error("document: {0}")]
    Model(String),

    #[error(transparent)]
    Geom(#[from] GeomError),
}

/// How a shape is physically produced on a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CutStyle {
    /// Trace the closed outline with the cutting tool.
    Cut,
    /// Trace without cutting: scoring, engraving outlines, marking.
    Engrave,
}

/// Kerf settings held in the document; applying them is `stencilcraft-kerf`'s job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KerfSettings {
    /// Nominal kerf (material removed by the tool), in millimetres. Compensation
    /// offsets contours by `width_mm / 2`.
    pub width_mm: f64,
    /// Whether exports should carry compensated contours already.
    pub apply_to_exports: bool,
}

impl Default for KerfSettings {
    fn default() -> Self {
        Self { width_mm: 0.0, apply_to_exports: false }
    }
}

/// The cutting sheet: artboard size in millimetres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Board {
    pub width_mm: f64,
    pub height_mm: f64,
}

impl Default for Board {
    fn default() -> Self {
        // A default sheet for vinyl work: 100 × 150 mm.
        Self { width_mm: 100.0, height_mm: 150.0 }
    }
}

/// One geometry layer of the document: a named container of shapes (GEO-05).
/// Production lives in `Document::operations`; a layer only groups geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    /// A locked layer's shapes cannot be selected or edited.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// Member shape ids, in intra-layer order (D-20). Geometry lives once in
    /// `Document::shapes`; a shape may be a member of many layers (D-02).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<u64>,
}

impl Layer {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), visible: true, locked: false, members: Vec::new() }
    }

    /// Whether newly created shapes may land here: not locked (D-18).
    pub fn takes_new_shapes(&self) -> bool {
        !self.locked
    }
}

/// A path. kurbo's `BezPath` serialises with the workspace's `serde` feature on —
/// no wrapper needed. The v1 `style`/`tool` fields are legacy read-only (D-16),
/// deserialized by `wire_v1` for migration only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: u64,
    pub path: BezPath,
    /// Hidden shapes are not drawn, selected or exported.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Locked shapes cannot be selected or edited until unlocked.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// Live text: the parameters `path` was laid out from, so the text stays editable
    /// (`text.edit` lays it out again). Absent for plain paths and in older files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Box<TextSpec>>,
    /// Production metadata (colours, attach, contours, combine sources).
    #[serde(default, skip_serializing_if = "ShapeProduction::is_default")]
    pub production: ShapeProduction,
}

/// Score and emboss tool tip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tip {
    #[default]
    Fine,
    Wide,
}

fn one_pass() -> u32 {
    1
}

/// A tool-specific operation a machine backend runs instead of a plain cut/engrave pass.
/// Paths tagged with one live on an engrave layer (they must never be cut through), except
/// `punch`, whose path is the hole outline for display and fallback cutting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ToolOp {
    /// Crease along the path (scoring stylus/wheel).
    Score {
        #[serde(default)]
        tip: Tip,
        #[serde(default = "one_pass")]
        passes: u32,
    },
    /// Press the path into the material (emboss, or deboss from the other side).
    Emboss {
        #[serde(default)]
        tip: Tip,
        #[serde(default = "one_pass")]
        passes: u32,
        #[serde(default)]
        deboss: bool,
    },
    /// Rotary/power engraving tool following the path.
    PowerEngrave,
    /// Heat pen pressing foil along the path.
    HeatPen,
    /// One punch per contour, at the contour's bounds centre: a backend with a punch tool
    /// plunges there instead of cutting the circle.
    Punch { diameter_mm: f64 },
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A live text object's parameters. The shape's `path` always holds the laid-out outlines,
/// so readers that ignore `text` (older builds, every export) still cut the right thing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextSpec {
    pub text: String,
    pub font: FontSpec,
    /// Em size in points (1 pt = 25.4/72 mm).
    pub size: f64,
    /// Anchor in mm: the first line's baseline at the alignment point.
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub alignment: TextAlign,
    /// Space added between letters, in percent of the em (0 = the font's own spacing).
    #[serde(default)]
    pub letter_spacing: f64,
    /// Distance between lines in percent of the normal line distance (100 = normal).
    #[serde(default = "hundred")]
    pub line_spacing: f64,
    /// Pair kerning from the font's tables.
    #[serde(default = "yes")]
    pub kerning: bool,
    /// Top-to-bottom columns instead of left-to-right lines.
    #[serde(default, skip_serializing_if = "is_false")]
    pub vertical: bool,
    /// Merge overlapping letter outlines into one cut path (script fonts).
    #[serde(default, skip_serializing_if = "is_false")]
    pub weld: bool,
    #[serde(default, skip_serializing_if = "TextFlow::is_straight")]
    pub flow: TextFlow,
    /// Per-letter nudges, by character index into `text` (newlines count).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub letters: Vec<LetterOffset>,
    /// Affine `[a, b, c, d, e, f]` applied after layout: the moves, turns and scales made to
    /// the shape since its text was laid out.
    #[serde(default = "identity", skip_serializing_if = "is_identity")]
    pub transform: [f64; 6],
}

/// A font by family and style, optionally from a font file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSpec {
    pub family: String,
    #[serde(default = "regular")]
    pub style: String,
    /// The font file the family came from, loaded again when the text is laid out again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextAlign {
    #[default]
    Left,
    #[serde(alias = "center", alias = "centre")]
    Centre,
    Right,
    /// Every line but the last is stretched to the widest line.
    Justify,
}

/// How the text runs: straight lines, one line bent round a circle, or one line along a path.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TextFlow {
    #[default]
    Straight,
    /// One line bent into an arc of this diameter in mm: positive arches up, negative dips.
    Curve { diameter_mm: f64 },
    /// One line along `path` (mm). `start` is where the anchor sits, as a fraction of the
    /// path's length; `offset_mm` lifts the letters off the path; `flip` runs the text the
    /// other way round, on the other side. `shape` names the shape the path was copied from.
    OnPath {
        #[serde(default)]
        path: BezPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shape: Option<u64>,
        #[serde(default)]
        start: f64,
        #[serde(default)]
        offset_mm: f64,
        #[serde(default)]
        flip: bool,
    },
}

impl TextFlow {
    pub fn is_straight(&self) -> bool {
        matches!(self, TextFlow::Straight)
    }
}

/// A nudge to one letter: moved by `dx_mm`, `dy_mm` and turned `rotate_deg` about its origin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct LetterOffset {
    pub index: usize,
    #[serde(default)]
    pub dx_mm: f64,
    #[serde(default)]
    pub dy_mm: f64,
    #[serde(default)]
    pub rotate_deg: f64,
}

fn hundred() -> f64 {
    100.0
}

fn yes() -> bool {
    true
}

fn regular() -> String {
    "Regular".to_string()
}

/// The identity affine in `TextSpec::transform` form.
pub const IDENTITY_TRANSFORM: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn identity() -> [f64; 6] {
    IDENTITY_TRANSFORM
}

fn is_identity(xf: &[f64; 6]) -> bool {
    *xf == IDENTITY_TRANSFORM
}

impl Shape {
    pub fn bounds(&self) -> Option<kurbo::Rect> {
        bounds_of_shape(&self.path)
    }
}

/// Most placed raster images one document may hold.
pub const MAX_IMAGES: usize = 256;
/// Most base64 PNG bytes one placed image may carry (about 96 MiB of PNG).
pub const MAX_IMAGE_DATA: usize = 128 * 1024 * 1024;

/// A placed raster image (a trace source, never cut). Pixels are PNG, base64-encoded so
/// the document stays JSON; `Arc` keeps undo snapshots from copying the bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RasterImage {
    /// Shares the shape id counter, so an id names one object in the document.
    pub id: u64,
    pub name: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub width_px: u32,
    pub height_px: u32,
    pub png_base64: std::sync::Arc<str>,
}

impl RasterImage {
    fn validate(&self) -> Result<(), DocError> {
        for (value, what) in [(self.x_mm, "image x"), (self.y_mm, "image y"), (self.width_mm, "image width"), (self.height_mm, "image height")] {
            check_finite_mm(value, what)?;
        }
        if self.width_mm <= 0.0 || self.height_mm <= 0.0 || self.width_px == 0 || self.height_px == 0 {
            return Err(DocError::Model(format!("image {}: width and height must be positive", self.id)));
        }
        if self.png_base64.len() > MAX_IMAGE_DATA {
            return Err(DocError::Model(format!("image {}: pixel data exceeds the 128 MiB limit", self.id)));
        }
        Ok(())
    }
}

/// A StencilCraft document: artboard, kerf configuration, geometry layers, production
/// operations, and the shape store. Geometry is stored once in `shapes`; layers and
/// operations reference shapes by id through `members` (D-02, GEO-05).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub board: Board,
    pub kerf: KerfSettings,
    /// Geometry-only layers (containers of shapes).
    pub layers: Vec<Layer>,
    /// Production operations, in pass order: the pass order is the operations order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<Operation>,
    /// All shapes, stored once; layers reference them by id.
    #[serde(default)]
    pub shapes: Vec<Shape>,
    /// Next free shape id; maintained by whoever mutates the document.
    #[serde(default)]
    pub next_shape_id: u64,
    /// Placed raster images, bottom first, drawn under the cut layers.
    #[serde(default)]
    pub images: Vec<RasterImage>,
    /// Mat, media, colour and name of the main board (page 0).
    #[serde(default)]
    pub board_setup: PageSetup,
    /// Extra pages beside the main board.
    #[serde(default)]
    pub pages: Vec<Page>,
    #[serde(default)]
    pub guides: Vec<Guide>,
    #[serde(default)]
    pub grid: Grid,
    #[serde(default)]
    pub snap: SnapSettings,
    /// Groups; members that no longer exist are ignored and dropped on save.
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub meta: Metadata,
}

/// Typed fields in the native v2 wire format. Field names and defaults are owned by
/// `craft-stencilcraft-format`; this alias supplies only StencilCraft's domain types.
type NativeFileFields = craft_stencilcraft_format::DocumentFields<
    Board,
    KerfSettings,
    Vec<Layer>,
    Vec<Operation>,
    RasterImage,
    PageSetup,
    Vec<Page>,
    Vec<Guide>,
    Grid,
    SnapSettings,
    Vec<Group>,
    Metadata,
    Vec<Shape>,
>;

/// Typed fields of a version 1 file: v1 layers embedded inline, no top-level shapes.
type V1FileFields = craft_stencilcraft_format::DocumentFields<
    Board,
    KerfSettings,
    Vec<wire_v1::V1Layer>,
    Vec<Operation>,
    RasterImage,
    PageSetup,
    Vec<Page>,
    Vec<Guide>,
    Grid,
    SnapSettings,
    Vec<Group>,
    Metadata,
>;

impl Default for Document {
    fn default() -> Self {
        Self {
            board: Board::default(),
            kerf: KerfSettings::default(),
            // One base geometry layer: a workspace container that accepts new
            // shapes (D-18). Production starts empty in `operations`.
            layers: vec![Layer::new("base")],
            operations: Vec::new(),
            shapes: Vec::new(),
            next_shape_id: 1,
            images: Vec::new(),
            board_setup: PageSetup::default(),
            pages: Vec::new(),
            guides: Vec::new(),
            grid: Grid::default(),
            snap: SnapSettings::default(),
            groups: Vec::new(),
            meta: Metadata::default(),
        }
    }
}

impl Document {
    /// All shapes in document order.
    pub fn shapes(&self) -> impl Iterator<Item = &Shape> {
        self.shapes.iter()
    }

    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    pub fn get_shape(&self, id: u64) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.id == id)
    }

    pub fn get_shape_mut(&mut self, id: u64) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| s.id == id)
    }

    /// Shapes that belong to no operation (D-04). A shape that is a member of any
    /// operation — a Guide operation included — counts as assigned; a shape in a
    /// geometry layer only is unassigned.
    pub fn unassigned_shapes(&self) -> impl Iterator<Item = &Shape> {
        let assigned: std::collections::BTreeSet<u64> = self.operations.iter().flat_map(|o| o.members.iter().copied()).collect();
        self.shapes.iter().filter(move |s| !assigned.contains(&s.id))
    }

    /// The production passes in document order (D-19): every visible, non-Guide
    /// operation, paired with its member shapes in membership order. Pass order is
    /// the `operations` order.
    pub fn production_sequence(&self) -> impl Iterator<Item = (&Operation, Vec<&Shape>)> {
        self.operations.iter().filter(|o| o.is_output()).map(|op| {
            let members = op.members.iter().filter_map(|id| self.get_shape(*id)).collect();
            (op, members)
        })
    }

    /// A free operation id.
    pub fn next_operation_id(&self) -> u64 {
        self.operations.iter().map(|o| o.id).max().map_or(1, |m| m.saturating_add(1))
    }

    /// The operation whose members contain shape `id`, first in pass order.
    pub fn operation_of_shape(&self, id: u64) -> Option<&Operation> {
        self.operations.iter().find(|o| o.members.contains(&id))
    }

    /// Union of the member shapes' bounds; `None` for an unknown or empty layer.
    pub fn layer_bounds(&self, layer_index: usize) -> Option<kurbo::Rect> {
        let layer = self.layers.get(layer_index)?;
        let mut bounds: Option<kurbo::Rect> = None;
        for id in &layer.members {
            if let Some(b) = self.get_shape(*id).and_then(Shape::bounds) {
                bounds = Some(match bounds {
                    Some(acc) => acc.union(b),
                    None => b,
                });
            }
        }
        bounds
    }

    /// Removes a layer and its memberships, keeping every shape's geometry in `shapes`
    /// (MODEL-07): former members become unassigned. Removing the last layer is allowed;
    /// the model has no minimum-layer invariant.
    pub fn remove_layer(&mut self, layer_index: usize) -> Result<Layer, DocError> {
        if layer_index >= self.layers.len() {
            return Err(DocError::Model(format!("layer {layer_index}: no such layer (document has {})", self.layers.len())));
        }
        Ok(self.layers.remove(layer_index))
    }

    /// Drops member ids that reference no shape and deduplicates repeats, keeping
    /// first-seen order within each layer and operation.
    pub fn prune_members(&mut self) {
        let live: std::collections::BTreeSet<u64> = self.shapes.iter().map(|s| s.id).collect();
        for layer in &mut self.layers {
            let mut seen = std::collections::BTreeSet::new();
            layer.members.retain(|id| live.contains(id) && seen.insert(*id));
        }
        for op in &mut self.operations {
            let mut seen = std::collections::BTreeSet::new();
            op.members.retain(|id| live.contains(id) && seen.insert(*id));
        }
    }

    /// Allocates the next shape id, stores the shape once in `self.shapes`, and records
    /// its membership in `layer_index`. Errors come out printable: messages name the
    /// layer and the fault.
    pub fn push_shape(&mut self, layer_index: usize, path: BezPath) -> Result<Shape, DocError> {
        if layer_index >= self.layers.len() {
            return Err(DocError::Model(format!("layer {layer_index}: no such layer (document has {})", self.layers.len())));
        }
        let Some(layer) = self.layers.get_mut(layer_index) else {
            return Err(DocError::Model(format!("layer {layer_index}: disappeared")));
        };
        let id = self.next_shape_id;
        self.next_shape_id = self.next_shape_id.checked_add(1).ok_or_else(|| DocError::Model("shape ids exhausted".to_string()))?;
        let shape = Shape { id, path, hidden: false, locked: false, text: None, production: ShapeProduction::default() };
        layer.members.push(id);
        self.shapes.push(shape.clone());
        Ok(shape)
    }

    /// Places a raster image with the next object id and returns that id.
    pub fn push_image(&mut self, mut image: RasterImage) -> Result<u64, DocError> {
        if self.images.len() >= MAX_IMAGES {
            return Err(DocError::Model(format!("a document holds at most {MAX_IMAGES} images")));
        }
        image.id = self.next_shape_id;
        image.validate()?;
        self.next_shape_id = self.next_shape_id.checked_add(1).ok_or_else(|| DocError::Model("shape ids exhausted".to_string()))?;
        let id = image.id;
        self.images.push(image);
        Ok(id)
    }

    pub fn get_image(&self, id: u64) -> Option<&RasterImage> {
        self.images.iter().find(|image| image.id == id)
    }

    /// Full validation: model invariants, then canvas fields.
    pub fn validate(&self) -> Result<(), DocError> {
        self.validate_model()?;
        self.validate_canvas()
    }

    /// Model-level invariants: a sane board and kerf, sane images, unique shape ids with
    /// `next_shape_id` above them all, memberships referencing existing shapes, and valid
    /// operation settings on every layer.
    pub fn validate_model(&self) -> Result<(), DocError> {
        check_finite_mm(self.board.width_mm, "board width")?;
        check_finite_mm(self.board.height_mm, "board height")?;
        check_finite_mm(self.kerf.width_mm, "kerf width")?;
        if self.board.width_mm <= 0.0 || self.board.height_mm <= 0.0 {
            return Err(DocError::Model("board: width and height must be positive".to_string()));
        }
        if self.kerf.width_mm < 0.0 {
            return Err(DocError::Model("kerf: width cannot be negative".to_string()));
        }
        if self.images.len() > MAX_IMAGES {
            return Err(DocError::Model(format!("a document holds at most {MAX_IMAGES} images")));
        }
        self.images.iter().try_for_each(RasterImage::validate)?;
        let mut ids = std::collections::BTreeSet::new();
        for shape in &self.shapes {
            if !ids.insert(shape.id) {
                return Err(DocError::Model(format!("shape {}: duplicate id", shape.id)));
            }
        }
        if let Some(max) = ids.last()
            && self.next_shape_id <= *max
        {
            return Err(DocError::Model(format!("next_shape_id {}: not above the highest shape id {max}", self.next_shape_id)));
        }
        for (i, layer) in self.layers.iter().enumerate() {
            for id in &layer.members {
                if !ids.contains(id) {
                    return Err(DocError::Model(format!("layer {i} ({}): member {id} is not a shape", layer.name)));
                }
            }
        }
        let mut op_ids = std::collections::BTreeSet::new();
        for (i, op) in self.operations.iter().enumerate() {
            if !op_ids.insert(op.id) {
                return Err(DocError::Model(format!("operation {}: duplicate id {}", i, op.id)));
            }
            for id in &op.members {
                if !ids.contains(id) {
                    return Err(DocError::Model(format!("operation {i} ({}): member {id} is not a shape", op.name)));
                }
            }
            op.validate().map_err(|e| match e {
                DocError::Model(m) => DocError::Model(format!("operation {i} ({}): {m}", op.name)),
                other => other,
            })?;
        }
        Ok(())
    }

    /// Serialises with the format header. The header wraps the document so a reader
    /// knows what it is before reading the rest. Canvas fields are written only when
    /// set, so a plain document saves exactly as it did before they existed.
    pub fn save_json(&self) -> Result<String, DocError> {
        // Stale members are repaired, not rejected: validate and serialise a pruned
        // clone so `save_json` can stay `&self`.
        let mut clean = self.clone();
        clean.prune_members();
        clean.validate()?;
        let fields = NativeFileFields {
            board: clean.board.clone(),
            kerf: clean.kerf.clone(),
            layers: clean.layers.clone(),
            operations: clean.operations.clone(),
            shapes: clean.shapes.clone(),
            next_shape_id: clean.next_shape_id,
            images: clean.images.clone(),
            board_setup: clean.board_setup.clone(),
            pages: clean.pages.clone(),
            guides: clean.guides.clone(),
            grid: clean.grid.clone(),
            snap: clean.snap.clone(),
            groups: clean.pruned_groups(),
            meta: clean.meta.clone(),
        };
        craft_stencilcraft_format::File::<NativeFileFields>::new(fields)
            .to_json_pretty()
            .map_err(|e| DocError::Structure { path: "output".to_string(), message: e.to_string() })
    }

    /// Parses a document from JSON with calm, positioned error messages, using neutral
    /// migration defaults for v1 files.
    pub fn load_json(json: &str) -> Result<Document, DocError> {
        Document::load_json_with_seed(json, &OperationDefaults::default())
    }

    /// Like [`Document::load_json`], but v1 files are migrated with `seed` supplying the
    /// default per-operation settings (D-15). v1 is migrated in memory only; the file is
    /// rewritten as v2 on the next save (D-13).
    pub fn load_json_with_seed(json: &str, seed: &OperationDefaults) -> Result<Document, DocError> {
        load_json_named(json, "input", seed)
    }
}

fn format_err(path: &str, e: craft_stencilcraft_format::Error) -> DocError {
    match e {
        craft_stencilcraft_format::Error::Json(source) => DocError::Parse { path: path.to_string(), source },
        craft_stencilcraft_format::Error::NotStencilCraft => DocError::NotAStencilcraftFile { path: path.to_string() },
        craft_stencilcraft_format::Error::Version { found, supported } => {
            DocError::FormatVersion { path: path.to_string(), found, expected: supported }
        }
        craft_stencilcraft_format::Error::Document { source } => structure_err(path, &source),
        _ => DocError::NotAStencilcraftFile { path: path.to_string() },
    }
}

fn load_json_named(json: &str, path: &str, seed: &OperationDefaults) -> Result<Document, DocError> {
    // Version dispatch: the format crate accepts every supported header version and
    // reports which one it found; v1 bodies migrate, v2 bodies parse directly.
    let (version, raw) =
        craft_stencilcraft_format::File::<craft_stencilcraft_format::DocumentFields>::read_versioned(json).map_err(|e| format_err(path, e))?;
    let raw_value = serde_json::to_value(raw.document).map_err(|source| DocError::Parse { path: path.to_string(), source })?;
    let mut doc = match version {
        1 => {
            let fields: V1FileFields = serde_json::from_value(raw_value).map_err(|e| structure_err(path, &e))?;
            let migrated = migrate::migrate_v1_layers(fields.layers, fields.next_shape_id, seed)?;
            Document {
                board: fields.board,
                kerf: fields.kerf,
                layers: migrated.layers,
                operations: migrated.operations,
                shapes: migrated.shapes,
                next_shape_id: migrated.next_shape_id,
                images: fields.images,
                board_setup: fields.board_setup,
                pages: fields.pages,
                guides: fields.guides,
                grid: fields.grid,
                snap: fields.snap,
                groups: fields.groups,
                meta: fields.meta,
            }
        }
        _ => {
            let fields: NativeFileFields = serde_json::from_value(raw_value).map_err(|e| structure_err(path, &e))?;
            Document {
                board: fields.board,
                kerf: fields.kerf,
                layers: fields.layers,
                operations: fields.operations,
                shapes: fields.shapes,
                next_shape_id: fields.next_shape_id,
                images: fields.images,
                board_setup: fields.board_setup,
                pages: fields.pages,
                guides: fields.guides,
                grid: fields.grid,
                snap: fields.snap,
                groups: fields.groups,
                meta: fields.meta,
            }
        }
    };
    // Dangling and duplicate memberships are pruned, never fatal, before validation.
    doc.prune_members();
    doc.validate()?;
    doc.groups = doc.pruned_groups();
    Ok(doc)
}

fn structure_err(path: &str, e: &serde_json::Error) -> DocError {
    DocError::Structure { path: path.to_string(), message: e.to_string() }
}

/// File helpers: errors keep the path so the message names the file.
pub fn save_to(doc: &Document, path: &str) -> Result<(), DocError> {
    let json = doc.save_json()?;
    craft_atomic::atomic_write(std::path::Path::new(path), json.as_bytes()).map_err(|source| DocError::Io { path: path.to_string(), source })
}

pub fn load_from(path: &str) -> Result<Document, DocError> {
    let json = std::fs::read_to_string(path).map_err(|source| DocError::Io { path: path.to_string(), source })?;
    load_json_named(&json, path, &OperationDefaults::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;

    fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to(Point::new(x, y));
        p.line_to(Point::new(x + w, y));
        p.line_to(Point::new(x + w, y + h));
        p.line_to(Point::new(x, y + h));
        p.close_path();
        p
    }

    fn sample() -> Document {
        let mut doc = Document::default();
        doc.kerf.width_mm = 0.2;
        let op_id = doc.next_operation_id();
        doc.operations.push(Operation {
            id: op_id,
            name: "cut".to_string(),
            kind: OperationKind::Cut,
            settings: OperationSettings::default(),
            color: None,
            visible: true,
            locked: false,
            members: Vec::new(),
            icon: None,
        });
        doc.push_shape(0, rect_path(10.0, 20.0, 30.0, 40.0)).unwrap();
        doc.operations[0].members.push(1);
        doc
    }

    #[test]
    fn round_trip() {
        let doc = sample();
        let json = doc.save_json().unwrap();
        let back = Document::load_json(&json).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn flags_round_trip_and_default_off() {
        let mut doc = sample();
        let plain = doc.save_json().unwrap();
        assert!(!plain.contains("hidden") && !plain.contains("locked"));
        doc.shapes[0].hidden = true;
        doc.shapes[0].locked = true;
        let back = Document::load_json(&doc.save_json().unwrap()).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn text_round_trips_and_minimal_text_loads() {
        let mut doc = sample();
        let plain = doc.save_json().unwrap();
        assert!(!plain.contains("\"text\""));
        doc.shapes[0].text = Some(Box::new(TextSpec {
            text: "Hi".into(),
            font: FontSpec { family: "Sans".into(), style: "Bold".into(), file: None },
            size: 72.0,
            x: 1.0,
            y: 2.0,
            alignment: TextAlign::Centre,
            letter_spacing: -5.0,
            line_spacing: 120.0,
            kerning: false,
            vertical: false,
            weld: true,
            flow: TextFlow::Curve { diameter_mm: 50.0 },
            letters: vec![LetterOffset { index: 1, dx_mm: 1.0, ..LetterOffset::default() }],
            transform: IDENTITY_TRANSFORM,
        }));
        let back = Document::load_json(&doc.save_json().unwrap()).unwrap();
        assert_eq!(doc, back);
        // A minimal hand-written text object fills in the defaults.
        let json =
            plain.replace("\"id\": 1", "\"id\": 1, \"text\": {\"text\": \"A\", \"font\": {\"family\": \"X\"}, \"size\": 12, \"x\": 0, \"y\": 0}");
        let spec = Document::load_json(&json).unwrap().shapes[0].text.clone().unwrap();
        assert_eq!(spec.line_spacing, 100.0);
        assert!(spec.kerning && spec.flow.is_straight());
        assert_eq!(spec.font.style, "Regular");
    }

    #[test]
    fn header_written() {
        let json = sample().save_json().unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let h = &v["stencilcraft"];
        assert_eq!(h["name"].as_str(), Some(FORMAT_NAME));
        assert_eq!(h["version"].as_u64(), Some(FORMAT_VERSION));
    }

    #[test]
    fn junk_is_an_error_not_a_crash() {
        assert!(Document::load_json("not json at all").is_err());
        assert!(matches!(Document::load_json("{}"), Err(DocError::NotAStencilcraftFile { .. })));
    }

    #[test]
    fn future_version_is_rejected_calmly() {
        let json = r#"{"stencilcraft": {"name": "stencilcraft", "version": 99}}"#;
        assert!(matches!(Document::load_json(json), Err(DocError::FormatVersion { found: 99, expected: FORMAT_VERSION, .. })));
    }

    #[test]
    fn missing_field_has_structure_error() {
        let json = r#"{"stencilcraft": {"name": "stencilcraft", "version": 2},
            "board": {"width_mm": 1, "height_mm": 1}}"#;
        assert!(matches!(
            Document::load_json(json),
            Err(DocError::Structure { ref message, .. }) if message.contains("kerf")
        ));
    }

    #[test]
    fn images_round_trip_and_share_the_id_counter() {
        let mut doc = canvas_sample();
        let id = doc
            .push_image(RasterImage {
                id: 0,
                name: "photo".to_string(),
                x_mm: 0.0,
                y_mm: 0.0,
                width_mm: 10.0,
                height_mm: 5.0,
                width_px: 2,
                height_px: 1,
                png_base64: "AAAA".into(),
            })
            .unwrap();
        assert_eq!(id, 3);
        let back = Document::load_json(&doc.save_json().unwrap()).unwrap();
        assert_eq!(back, doc);
        let mut bad = doc.clone();
        bad.images[0].width_mm = -1.0;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn push_shape_assigns_ids_and_records_membership() {
        let mut doc = Document::default();
        // The base geometry layer accepts new shapes (D-18).
        assert!(doc.layers[0].takes_new_shapes());
        let s = doc.push_shape(0, rect_path(0.0, 0.0, 5.0, 5.0)).unwrap();
        assert_eq!(s.id, 1);
        let s2 = doc.push_shape(0, rect_path(5.0, 5.0, 5.0, 5.0)).unwrap();
        assert_eq!(s2.id, 2);
        assert_eq!(doc.layers[0].members, vec![1, 2]);
        assert_eq!(doc.shape_count(), 2);
        assert!(matches!(doc.push_shape(9, rect_path(0.0, 0.0, 1.0, 1.0)), Err(DocError::Model(_))));
        // One shape id can be a member of more than one layer and of many
        // operations (N:M); geometry is stored once.
        doc.layers.push(Layer::new("second"));
        doc.layers[1].members.push(1);
        doc.operations.push(Operation {
            id: doc.next_operation_id(),
            name: "score".to_string(),
            kind: OperationKind::Score,
            settings: OperationSettings::default(),
            color: None,
            visible: true,
            locked: false,
            members: vec![1],
            icon: None,
        });
        let back = Document::load_json(&doc.save_json().unwrap()).unwrap();
        assert_eq!(back.layers[1].members, vec![1]);
        assert_eq!(back.operations.len(), 1);
        assert_eq!(back.operations[0].members, vec![1]);
        assert_eq!(back.shapes.len(), 2);
    }

    #[test]
    fn operations_round_trip_with_multi_pass_order_and_n_m_membership() {
        let mut doc = sample();
        doc.push_shape(0, rect_path(0.0, 0.0, 5.0, 5.0)).unwrap();
        doc.operations.push(Operation {
            id: doc.next_operation_id(),
            name: "engrave".to_string(),
            kind: OperationKind::Engrave,
            settings: OperationSettings { power: 40.0, ..OperationSettings::default() },
            color: Some("#c02020".to_string()),
            visible: true,
            locked: false,
            members: vec![2, 1], // shape 1 is in both passes (N:M)
            icon: Some("pen".to_string()),
        });
        doc.operations.push(Operation {
            id: doc.next_operation_id(),
            name: "guide".to_string(),
            kind: OperationKind::Guide,
            settings: OperationSettings::default(),
            color: None,
            visible: true,
            locked: true,
            members: vec![2],
            icon: None,
        });
        let json = doc.save_json().unwrap();
        let back = Document::load_json(&json).unwrap();
        assert_eq!(doc, back);
        // Pass order is the operations order; Guide is not an output pass.
        let passes: Vec<&str> = back.production_sequence().map(|(op, _)| op.kind_name()).collect();
        assert_eq!(passes, vec!["cut", "engrave"]);
        // Geometry is unchanged and stored once.
        assert_eq!(back.shapes.len(), 2);
        assert!(back.unassigned_shapes().next().is_none());
    }

    fn canvas_sample() -> Document {
        let mut doc = sample();
        doc.push_shape(0, rect_path(50.0, 50.0, 5.0, 5.0)).unwrap();
        doc.operations[0].members.push(2);
        doc.board_setup.mat = Some("12x12".to_string());
        doc.pages.push(Page {
            x_mm: 120.0,
            y_mm: 0.0,
            width_mm: 210.0,
            height_mm: 297.0,
            setup: PageSetup { name: "A4".to_string(), color: Some("#ffeedd".to_string()), ..PageSetup::default() },
        });
        doc.guides.push(Guide { axis: GuideAxis::Vertical, position_mm: 12.5, locked: true });
        doc.grid.spacing_mm = 5.0;
        doc.grid.subdivisions = 4;
        doc.snap.grid = true;
        doc.operations[0].color = Some("#112233".to_string());
        doc.layers.push(Layer { locked: true, ..Layer::new("guide") });
        doc.operations.push(Operation {
            id: doc.next_operation_id(),
            name: "guide".to_string(),
            kind: OperationKind::Guide,
            settings: OperationSettings::default(),
            color: None,
            visible: true,
            locked: true,
            members: vec![1],
            icon: None,
        });
        doc.groups = vec![
            Group { id: 1, name: "inner".to_string(), children: vec![GroupChild::Shape(1)] },
            Group { id: 2, name: String::new(), children: vec![GroupChild::Group(1), GroupChild::Shape(2)] },
        ];
        doc.meta.title = "Card".to_string();
        doc.meta.notes = "for mum".to_string();
        doc.meta.thumbnail_svg = Some(doc.render_thumbnail(4096));
        doc
    }

    #[test]
    fn canvas_fields_round_trip() {
        let doc = canvas_sample();
        let back = Document::load_json(&doc.save_json().unwrap()).unwrap();
        assert_eq!(doc, back);
        assert_eq!(back.group_chain(GroupChild::Shape(1)), vec![2, 1]);
        assert_eq!(back.group_shapes(2), vec![1, 2]);
    }

    #[test]
    fn plain_documents_save_without_canvas_keys() {
        let json = sample().save_json().unwrap();
        for key in ["pages", "guides", "grid", "snap", "groups", "meta", "board-setup", "template"] {
            assert!(!json.contains(key), "{key} written for a plain document");
        }
    }

    #[test]
    fn version_1_files_migrate_in_memory_and_resave_as_v2() {
        for json in [
            include_str!("../tests/fixtures/starter.stencilcraft"),
            include_str!("../tests/fixtures/poster.stencilcraft"),
            include_str!("../tests/fixtures/stencil-frame.stencilcraft"),
        ] {
            let doc = Document::load_json(json).unwrap();
            assert!(doc.pages.is_empty() && doc.groups.is_empty() && doc.guides.is_empty());
            assert_eq!(doc.grid, Grid::default());
            assert!(doc.layers.iter().all(|l| !l.locked));
            // Migration hoists geometry once and gives every v1 shape an operation
            // membership, preserving colour/visibility/lock/order.
            assert!(doc.shape_count() > 0);
            assert!(!doc.operations.is_empty());
            assert!(doc.operations.iter().all(|o| o.visible && !o.locked));
            let membered: usize = doc.operations.iter().map(|o| o.members.len()).sum();
            assert!(membered >= doc.shape_count());
            assert_eq!(doc.unassigned_shapes().count(), 0);
            // The in-memory migration is not written back until save, which writes v2.
            let saved = doc.save_json().unwrap();
            let v: serde_json::Value = serde_json::from_str(&saved).unwrap();
            assert_eq!(v["stencilcraft"]["version"].as_u64(), Some(FORMAT_VERSION));
            assert!(v.get("shapes").is_some());
            assert_eq!(Document::load_json(&saved).unwrap(), doc);
        }
    }

    #[test]
    fn dangling_group_members_are_pruned_not_fatal() {
        let mut doc = canvas_sample();
        doc.shapes.retain(|s| s.id != 1);
        // Group 1 loses its only member and disappears; group 2 keeps shape 2.
        let back = Document::load_json(&doc.save_json().unwrap()).unwrap();
        assert_eq!(back.groups.len(), 1);
        assert_eq!(back.groups[0].children, vec![GroupChild::Shape(2)]);
    }

    #[test]
    fn bad_canvas_data_is_rejected_calmly() {
        type DocumentMutation = Box<dyn Fn(&mut Document)>;
        let cases: Vec<DocumentMutation> = vec![
            Box::new(|d| d.groups[0].children.push(GroupChild::Group(2))),
            Box::new(|d| d.groups[1].children.push(GroupChild::Shape(1))),
            Box::new(|d| d.groups[0].id = 2),
            Box::new(|d| d.pages[0].width_mm = f64::NAN),
            Box::new(|d| d.guides[0].position_mm = f64::INFINITY),
            Box::new(|d| d.grid.subdivisions = 0),
            Box::new(|d| d.operations[0].color = Some("red".to_string())),
            Box::new(|d| d.meta.notes = "x".repeat(canvas::MAX_NOTES_BYTES + 1)),
            Box::new(|d| d.snap.tolerance_mm = f64::NAN),
        ];
        for (i, corrupt) in cases.iter().enumerate() {
            let mut doc = canvas_sample();
            corrupt(&mut doc);
            assert!(doc.validate().is_err(), "case {i} validated");
            assert!(doc.save_json().is_err(), "case {i} saved");
        }
        // The same faults in a file are load errors.
        let json = canvas_sample().save_json().unwrap().replace("#112233", "#zz");
        assert!(Document::load_json(&json).is_err());
    }

    #[test]
    fn page_document_takes_the_shapes_on_that_page() {
        let mut doc = canvas_sample();
        doc.push_shape(0, rect_path(130.0, 10.0, 10.0, 10.0)).unwrap();
        let page = doc.page_document(1).unwrap();
        assert_eq!((page.board.width_mm, page.board.height_mm), (210.0, 297.0));
        assert_eq!(page.shape_count(), 1);
        assert_eq!(page.layers[0].members, vec![3]);
        let b = page.shapes[0].bounds().unwrap();
        assert!((b.x0 - 10.0).abs() < 1e-9 && (b.y0 - 10.0).abs() < 1e-9);
        assert_eq!(doc.page_document(0).unwrap().shape_count(), 2);
        assert!(doc.page_document(5).is_none());
    }

    #[test]
    fn thumbnail_respects_its_budget() {
        let doc = canvas_sample();
        let full = doc.render_thumbnail(64 * 1024);
        assert!(full.starts_with("<svg") && full.ends_with("</svg>"));
        assert_eq!(full.matches("<path").count(), 2);
        let small = doc.render_thumbnail(full.len() - 1);
        assert!(small.len() < full.len() && small.ends_with("</svg>"));
    }
}
