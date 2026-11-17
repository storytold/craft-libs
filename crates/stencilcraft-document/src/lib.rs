//! StencilCraft document model and domain validation.
//!
//! A document holds an artboard in millimetres, kerf settings and layers of shapes.
//! Every read path returns `Result` with a calm, printable error: a malformed file
//! must never crash (`AGENTS.md` › "Never crash"), because it is untrusted input.

pub mod canvas;
pub mod geom;
pub mod metadata;

use crate::geom::{GeomError, bounds_of_shape, check_finite_mm};
pub use canvas::{Grid, Group, GroupChild, Guide, GuideAxis, Metadata, Orientation, Page, PagePreset, PageSetup, SnapSettings};
use kurbo::BezPath;
use serde::{Deserialize, Serialize};

pub mod production;
pub use metadata::{LibraryIndex, LibraryTrashEntry, MAX_RECENT_FILES, RecentFiles};
pub use production::{CombineOp, GroupKind, Operation, SendAssign, ShapeProduction, SourceGroup};

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

/// One layer of the document: a group of shapes with a production style and a
/// visibility flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub kind: CutStyle,
    pub visible: bool,
    pub shapes: Vec<Shape>,
    /// A locked layer's shapes cannot be selected or edited.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// A template (guide) layer is shown on the canvas but never exported or cut.
    #[serde(default, skip_serializing_if = "is_false")]
    pub template: bool,
    /// Material colour label, `#rrggbb`; Color Sync merges similar ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl Layer {
    pub fn new(name: impl Into<String>, kind: CutStyle) -> Self {
        Self { name: name.into(), kind, visible: true, shapes: Vec::new(), locked: false, template: false, color: None }
    }

    /// Whether exports and cuts include this layer: visible and not a template.
    pub fn is_output(&self) -> bool {
        self.visible && !self.template
    }

    /// Whether newly created shapes may land here: not locked, not a template.
    pub fn takes_new_shapes(&self) -> bool {
        !self.locked && !self.template
    }

    /// Sum of per-shape bounds; `None` when the layer has no shapes or only empty
    /// paths.
    pub fn bounds(&self) -> Option<kurbo::Rect> {
        let mut acc: Option<kurbo::Rect> = None;
        for shape in &self.shapes {
            if let Some(r) = shape.bounds() {
                acc = Some(match acc {
                    Some(prev) => prev.union(r),
                    None => r,
                });
            }
        }
        acc
    }
}

/// A path plus its production style. kurbo's `BezPath` serialises with the
/// workspace's `serde` feature on — no wrapper needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: u64,
    pub path: BezPath,
    pub style: CutStyle,
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
    /// A tool-specific operation for this path (score, emboss, engrave, foil, punch);
    /// `None` is a plain cut or engrave pass by `style`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolOp>,
    /// Production metadata (operation, colours, attach, contours, combine sources).
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

/// A StencilCraft document: artboard, kerf configuration, layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub board: Board,
    pub kerf: KerfSettings,
    pub layers: Vec<Layer>,
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

/// Typed fields in the native wire format. Field names and version-1 defaults are owned by
/// `craft-stencilcraft-format`; this alias supplies only StencilCraft's domain types.
type NativeFileFields = craft_stencilcraft_format::DocumentFields<
    Board,
    KerfSettings,
    Vec<Layer>,
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
            layers: vec![Layer::new("cut", CutStyle::Cut)],
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
    /// Shapes across all layers, in order, paired with their layer index.
    pub fn shapes(&self) -> impl Iterator<Item = (usize, &Shape)> {
        self.layers.iter().enumerate().flat_map(|(li, l)| l.shapes.iter().map(move |s| (li, s)))
    }

    pub fn shape_count(&self) -> usize {
        self.layers.iter().map(|l| l.shapes.len()).sum()
    }

    pub fn get_shape(&self, id: u64) -> Option<&Shape> {
        self.shapes().map(|(_, s)| s).find(|s| s.id == id)
    }

    pub fn get_shape_mut(&mut self, id: u64) -> Option<&mut Shape> {
        self.layers.iter_mut().flat_map(|l| l.shapes.iter_mut()).find(|s| s.id == id)
    }

    /// Allocates the next shape id and records the shape in `layer_index`.
    /// Errors come out printable: messages name the layer and the fault.
    pub fn push_shape(&mut self, layer_index: usize, path: BezPath, style: CutStyle) -> Result<Shape, DocError> {
        if layer_index >= self.layers.len() {
            return Err(DocError::Model(format!("layer {layer_index}: no such layer (document has {})", self.layers.len())));
        }
        let Some(layer) = self.layers.get_mut(layer_index) else {
            return Err(DocError::Model(format!("layer {layer_index}: disappeared")));
        };
        let id = self.next_shape_id;
        self.next_shape_id = self.next_shape_id.checked_add(1).ok_or_else(|| DocError::Model("shape ids exhausted".to_string()))?;
        let shape = Shape { id, path, style, hidden: false, locked: false, tool: None, text: None, production: ShapeProduction::default() };
        layer.shapes.push(shape.clone());
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

    /// Checks the board is a sane sheet: positive, finite dimensions.
    pub fn validate(&self) -> Result<(), DocError> {
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
        self.validate_canvas()
    }

    /// Serialises with the format header. The header wraps the document so a reader
    /// knows what it is before reading the rest. Canvas fields are written only when
    /// set, so a plain document saves exactly as it did before they existed.
    pub fn save_json(&self) -> Result<String, DocError> {
        self.validate()?;
        let fields = NativeFileFields {
            board: self.board.clone(),
            kerf: self.kerf.clone(),
            layers: self.layers.clone(),
            next_shape_id: self.next_shape_id,
            images: self.images.clone(),
            board_setup: self.board_setup.clone(),
            pages: self.pages.clone(),
            guides: self.guides.clone(),
            grid: self.grid.clone(),
            snap: self.snap.clone(),
            groups: self.pruned_groups(),
            meta: self.meta.clone(),
        };
        craft_stencilcraft_format::File::<NativeFileFields>::new(fields)
            .to_json_pretty()
            .map_err(|e| DocError::Structure { path: "output".to_string(), message: e.to_string() })
    }

    /// Parses a document from JSON with calm, positioned error messages.
    pub fn load_json(json: &str) -> Result<Document, DocError> {
        load_json_named(json, "input")
    }
}

fn load_json_named(json: &str, path: &str) -> Result<Document, DocError> {
    let file = craft_stencilcraft_format::File::<NativeFileFields>::read_typed(json).map_err(|e| match e {
        craft_stencilcraft_format::Error::Json(source) => DocError::Parse { path: path.to_string(), source },
        craft_stencilcraft_format::Error::NotStencilCraft => DocError::NotAStencilcraftFile { path: path.to_string() },
        craft_stencilcraft_format::Error::Version { found, supported } => {
            DocError::FormatVersion { path: path.to_string(), found, expected: supported }
        }
        craft_stencilcraft_format::Error::Document { source } => structure_err(path, &source),
        _ => DocError::NotAStencilcraftFile { path: path.to_string() },
    })?;
    let fields = file.document;
    let mut doc = Document {
        board: fields.board,
        kerf: fields.kerf,
        layers: fields.layers,
        next_shape_id: fields.next_shape_id,
        images: fields.images,
        board_setup: fields.board_setup,
        pages: fields.pages,
        guides: fields.guides,
        grid: fields.grid,
        snap: fields.snap,
        groups: fields.groups,
        meta: fields.meta,
    };
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
    load_json_named(&json, path)
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
        doc.push_shape(0, rect_path(10.0, 20.0, 30.0, 40.0), CutStyle::Cut).unwrap();
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
        doc.layers[0].shapes[0].hidden = true;
        doc.layers[0].shapes[0].locked = true;
        let back = Document::load_json(&doc.save_json().unwrap()).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn text_round_trips_and_minimal_text_loads() {
        let mut doc = sample();
        let plain = doc.save_json().unwrap();
        assert!(!plain.contains("\"text\""));
        doc.layers[0].shapes[0].text = Some(Box::new(TextSpec {
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
        let json = plain.replace(
            "\"style\": \"cut\"",
            "\"style\": \"cut\", \"text\": {\"text\": \"A\", \"font\": {\"family\": \"X\"}, \"size\": 12, \"x\": 0, \"y\": 0}",
        );
        let spec = Document::load_json(&json).unwrap().layers[0].shapes[0].text.clone().unwrap();
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
        assert!(matches!(Document::load_json(json), Err(DocError::FormatVersion { found: 99, expected: 1, .. })));
    }

    #[test]
    fn missing_field_has_structure_error() {
        let json = r#"{"stencilcraft": {"name": "stencilcraft", "version": 1},
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
    fn push_shape_assigns_ids() {
        let mut doc = Document::default();
        let s = doc.push_shape(0, rect_path(0.0, 0.0, 5.0, 5.0), CutStyle::Cut).unwrap();
        assert_eq!(s.id, 1);
        let s2 = doc.push_shape(0, rect_path(5.0, 5.0, 5.0, 5.0), CutStyle::Cut).unwrap();
        assert_eq!(s2.id, 2);
        assert!(matches!(doc.push_shape(9, rect_path(0.0, 0.0, 1.0, 1.0), CutStyle::Cut), Err(DocError::Model(_))));
    }

    fn canvas_sample() -> Document {
        let mut doc = sample();
        doc.push_shape(0, rect_path(50.0, 50.0, 5.0, 5.0), CutStyle::Cut).unwrap();
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
        doc.layers[0].color = Some("#112233".to_string());
        doc.layers.push(Layer { template: true, locked: true, ..Layer::new("guide", CutStyle::Cut) });
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
    fn version_1_files_without_canvas_fields_still_load() {
        for json in [
            include_str!("../tests/fixtures/starter.stencilcraft"),
            include_str!("../tests/fixtures/poster.stencilcraft"),
            include_str!("../tests/fixtures/stencil-frame.stencilcraft"),
        ] {
            let doc = Document::load_json(json).unwrap();
            assert!(doc.pages.is_empty() && doc.groups.is_empty() && doc.guides.is_empty());
            assert_eq!(doc.grid, Grid::default());
            assert!(doc.layers.iter().all(|l| !l.template && !l.locked));
        }
    }

    #[test]
    fn dangling_group_members_are_pruned_not_fatal() {
        let mut doc = canvas_sample();
        doc.layers[0].shapes.retain(|s| s.id != 1);
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
            Box::new(|d| d.layers[0].color = Some("red".to_string())),
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
        doc.push_shape(0, rect_path(130.0, 10.0, 10.0, 10.0), CutStyle::Cut).unwrap();
        let page = doc.page_document(1).unwrap();
        assert_eq!((page.board.width_mm, page.board.height_mm), (210.0, 297.0));
        assert_eq!(page.shape_count(), 1);
        let b = page.layers[0].shapes[0].bounds().unwrap();
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
