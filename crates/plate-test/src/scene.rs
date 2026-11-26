//! The scene format: an app-neutral description of one test page, what each plate should
//! carry at sample points, and what the PDF's structure should look like.
//!
//! Coordinates are points from the page's top-left corner. Ink values are percentages
//! (0 = no ink, 100 = solid). Rectangles are `[x, y, width, height]`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::structure::StructureExpect;

/// How deep placed scenes may nest.
pub const MAX_NESTING: usize = 4;

/// The most pixels an image source may ask for on either side.
pub const MAX_IMAGE_PX: u32 = 4096;

/// The plate text points find their glyphs on unless they name another.
pub const TEXT_PLATE: &str = "Black";

/// The process plates, named as separation tools (Ghostscript `tiffsep`) name them.
pub const PROCESS: [&str; 4] = ["Cyan", "Magenta", "Yellow", "Black"];

/// One test page: what to draw, which plates a correct separation produces, and the PDF structure
/// to expect.
///
/// Loaded from a JSON file by [`load`]; check it with [`Scene::validate`].
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    /// Short id, also the file name stem's prefix (`S01`, `D04`).
    pub id: String,
    #[allow(missing_docs)]
    pub title: String,
    /// Where the scene comes from (research case or app regression).
    pub source: String,
    /// Page size `[width, height]` in pt.
    pub page: [f32; 2],
    /// Spot inks the scene may use. Their order is the colorant order after the process inks.
    #[serde(default)]
    pub inks: Vec<Ink>,
    #[serde(default)]
    #[allow(missing_docs)]
    pub settings: Settings,
    /// Objects drawn in order, back to front.
    pub objects: Vec<Object>,
    #[allow(missing_docs)]
    pub expect: Expect,
    #[serde(default)]
    #[allow(missing_docs)]
    pub structure: StructureExpect,
    /// The task that makes this scene writable, e.g. `"T4"`. Pending scenes are parsed but not
    /// written or checked.
    #[serde(default)]
    pub pending: Option<String>,
    #[serde(default)]
    #[allow(missing_docs)]
    pub note: Option<String>,
}

/// A spot ink with its alternate (exactly one of `cmyk` or `lab`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ink {
    /// Ink name as it appears in the PDF and in plate file names.
    pub name: String,
    #[serde(default)]
    #[allow(missing_docs)]
    pub cmyk: Option<[f32; 4]>,
    #[serde(default)]
    #[allow(missing_docs)]
    pub lab: Option<[f32; 3]>,
}

/// Output intents a scene can name: `fogra39` is craft-cms's `COATED_FOGRA39_ICC`.
pub const OUTPUT_INTENTS: &[&str] = &["fogra39"];

/// Per-scene writer settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Blending color space of transparency groups; absent means the writer's default.
    #[serde(default)]
    pub group_cs: Option<GroupCs>,
    /// Name of the print output intent, one of [`OUTPUT_INTENTS`]; the scene is then written
    /// with craft-pdf's print settings. Absent means the writer's defaults.
    #[serde(default)]
    pub output_intent: Option<String>,
}

/// Blending colour space of transparency groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupCs {
    #[allow(missing_docs)]
    Rgb,
    #[allow(missing_docs)]
    Cmyk,
}

/// The type of an [`Object`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[allow(missing_docs)]
    Rect,
    #[allow(missing_docs)]
    Ellipse,
    #[allow(missing_docs)]
    Text,
    #[allow(missing_docs)]
    Image,
    /// Another scene, written and placed as a PDF page.
    Pdf,
    #[allow(missing_docs)]
    Group,
}

/// One drawable object. Which geometry fields apply depends on `type`; [`Scene::validate`]
/// reports missing or stray ones.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    #[serde(rename = "type")]
    #[allow(missing_docs)]
    pub kind: Kind,
    /// rect, ellipse (bounding box), image, pdf.
    #[serde(default)]
    pub rect: Option<[f32; 4]>,
    /// text: baseline start.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    /// text: font size in pt.
    #[serde(default)]
    pub size: Option<f32>,
    /// text: the string to draw.
    #[serde(default)]
    pub text: Option<String>,
    /// image: the synthetic image to fill `rect` with.
    #[serde(default)]
    pub image: Option<ImageSource>,
    /// pdf: id of the scene to place.
    #[serde(default)]
    pub scene: Option<String>,
    /// group: children, drawn in order.
    #[serde(default)]
    pub objects: Vec<Object>,
    /// group: write the group as isolated.
    #[serde(default)]
    pub isolated: bool,
    /// group: write the group as a knockout group.
    #[serde(default)]
    pub knockout: bool,
    #[serde(default)]
    #[allow(missing_docs)]
    pub fill: Option<Paint>,
    #[serde(default)]
    #[allow(missing_docs)]
    pub stroke: Option<Stroke>,
    /// Whole-object opacity, 0–1 (a transparency group).
    #[serde(default)]
    pub opacity: Option<f32>,
    #[serde(default)]
    #[allow(missing_docs)]
    pub blend: Option<Blend>,
    #[serde(default)]
    #[allow(missing_docs)]
    pub overprint: Option<Overprint>,
    #[serde(default)]
    #[allow(missing_docs)]
    pub mask: Option<Mask>,
    #[serde(default)]
    #[allow(missing_docs)]
    pub note: Option<String>,
}

/// A paint. Ink values are percentages.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Paint {
    /// `"registration"`: the Separation `/All` colorant (every plate).
    Named(Named),
    #[allow(missing_docs)]
    Cmyk {
        #[allow(missing_docs)]
        cmyk: [f32; 4],
    },
    /// Black ink percentage, written as DeviceGray `1 - gray / 100`.
    Gray {
        #[allow(missing_docs)]
        gray: f32,
    },
    /// 0–255 per channel.
    Rgb {
        #[allow(missing_docs)]
        rgb: [f32; 3],
    },
    /// A declared spot ink at `tint` percent.
    Spot {
        /// Name of a spot ink declared in [`Scene::inks`].
        spot: String,
        /// Tint percentage; 100 when absent.
        #[serde(default = "full")]
        tint: f32,
    },
    /// Several named inks at once (DeviceN); names are process plates or declared spots.
    Inks {
        #[allow(missing_docs)]
        inks: BTreeMap<String, f32>,
    },
    #[allow(missing_docs)]
    Linear {
        #[allow(missing_docs)]
        linear: Gradient,
    },
    #[allow(missing_docs)]
    Mesh {
        #[allow(missing_docs)]
        mesh: Box<Mesh>,
    },
}

/// A named colorant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Named {
    #[allow(missing_docs)]
    Registration,
}

fn full() -> f32 {
    100.0
}

/// A linear gradient in page coordinates.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gradient {
    /// Start point in page pt.
    pub from: [f32; 2],
    /// End point in page pt.
    pub to: [f32; 2],
    #[allow(missing_docs)]
    pub stops: Vec<GradientStop>,
}

/// A mesh gradient of one Coons patch with straight edges over `rect` (page coordinates).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mesh {
    #[allow(missing_docs)]
    pub rect: [f32; 4],
    /// Solid paints at the top-left, top-right, bottom-right and bottom-left corners.
    pub corners: [Paint; 4],
}

/// One colour stop of a [`Gradient`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradientStop {
    /// 0–1 along the gradient.
    pub at: f32,
    #[allow(missing_docs)]
    pub paint: Paint,
}

/// An outline paint and width.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    #[allow(missing_docs)]
    pub paint: Paint,
    /// Line width in pt.
    pub width: f32,
}

/// A synthetic image filling the object's `rect` (exactly one of `ramp`, `split`, `feather`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSource {
    /// Pixel size `[width, height]`.
    pub px: [u32; 2],
    /// Horizontal ramp from the first paint (left) to the second (right).
    #[serde(default)]
    pub ramp: Option<[Paint; 2]>,
    /// Left half the first paint, right half the second.
    #[serde(default)]
    pub split: Option<[Paint; 2]>,
    /// Gray image for soft masks: white inside, fading to black over this many pt at the edges.
    #[serde(default)]
    pub feather: Option<f32>,
    /// An alpha channel, written as the image's `/SMask` (a PNG with transparency).
    #[serde(default)]
    pub alpha: Option<ImageAlpha>,
}

/// Image alpha: `opacity` (0–1) for pixels whose centers lie inside `rect` (page pt), 0 for the rest.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageAlpha {
    #[allow(missing_docs)]
    pub rect: [f32; 4],
    #[allow(missing_docs)]
    pub opacity: f32,
}

/// A PDF blend mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Blend {
    #[allow(missing_docs)]
    Normal,
    #[allow(missing_docs)]
    Multiply,
    #[allow(missing_docs)]
    Screen,
    #[allow(missing_docs)]
    Overlay,
    #[allow(missing_docs)]
    Darken,
    #[allow(missing_docs)]
    Lighten,
    #[allow(missing_docs)]
    ColorDodge,
    #[allow(missing_docs)]
    ColorBurn,
    #[allow(missing_docs)]
    HardLight,
    #[allow(missing_docs)]
    SoftLight,
    #[allow(missing_docs)]
    Difference,
    #[allow(missing_docs)]
    Exclusion,
    #[allow(missing_docs)]
    Hue,
    #[allow(missing_docs)]
    Saturation,
    #[allow(missing_docs)]
    Color,
    #[allow(missing_docs)]
    Luminosity,
}

/// Overprint flags of an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overprint {
    #[serde(default)]
    #[allow(missing_docs)]
    pub fill: bool,
    #[serde(default)]
    #[allow(missing_docs)]
    pub stroke: bool,
    /// OPM: 1 = zero components leave the backdrop (nonzero overprint), 0 = all components paint.
    #[serde(default = "opm_default")]
    pub mode: u8,
}

fn opm_default() -> u8 {
    1
}

/// A soft mask over the object.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    #[allow(missing_docs)]
    pub kind: MaskKind,
    /// Objects drawn to make the mask.
    pub content: Vec<Object>,
    /// `/BC`: the color outside the mask content (luminosity masks).
    #[serde(default)]
    pub backdrop: Option<Paint>,
    /// `/TR`: a sampled transfer function, values 0–1 over the input range.
    #[serde(default)]
    pub transfer: Option<Vec<f32>>,
}

/// Whether a soft mask reads the luminosity or the alpha of its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskKind {
    #[allow(missing_docs)]
    Luminosity,
    #[allow(missing_docs)]
    Alpha,
}

/// What a correct separation gives.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// Every plate the separation must produce (process plates included), and no others.
    pub plates: Vec<String>,
    /// Where to measure the plates.
    pub points: Vec<SamplePoint>,
}

/// One place on the page where plate ink is measured, and the ink expected there.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SamplePoint {
    /// Position in page pt. Area points sample around it; a text point with a `region` samples that
    /// region instead.
    pub at: [f32; 2],
    #[serde(default)]
    #[allow(missing_docs)]
    pub kind: PointKind,
    /// text: the glyph area to sample, `[x, y, w, h]`.
    #[serde(default)]
    pub region: Option<[f32; 4]>,
    /// text: the plate the glyphs are found on; defaults to [`TEXT_PLATE`].
    #[serde(default)]
    pub plate: Option<String>,
    /// Expected ink per plate; plates not listed expect 0.
    #[serde(default)]
    pub inks: BTreeMap<String, Level>,
    /// Allowed difference in ink percentage points.
    pub tol: f32,
    /// Measured and reported, not judged.
    #[serde(default)]
    pub info: bool,
    #[serde(default)]
    #[allow(missing_docs)]
    pub note: Option<String>,
}

/// How a [`SamplePoint`] is measured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PointKind {
    #[default]
    #[allow(missing_docs)]
    Area,
    #[allow(missing_docs)]
    Text,
}

/// An expected ink value or an inclusive `[lo, hi]` range.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Level {
    #[allow(missing_docs)]
    Value(f32),
    #[allow(missing_docs)]
    Range([f32; 2]),
}

/// The directory holding this crate's scenes.
pub fn scenes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes")
}

/// Parse every `*.json` scene in `dir`, sorted by file name.
pub fn load_dir(dir: &Path) -> Result<Vec<Scene>, Error> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| Error::Scene(format!("{}: {e}", dir.display())))? {
        let path = entry.map_err(|e| Error::Scene(e.to_string()))?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            paths.push(path);
        }
    }
    paths.sort();
    paths.iter().map(|path| load(path)).collect()
}

/// Parse one scene file.
pub fn load(path: &Path) -> Result<Scene, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::Scene(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| Error::Scene(format!("{}: {e}", path.display())))
}

impl Scene {
    /// The declared spot ink called `name`, if any.
    pub fn ink(&self, name: &str) -> Option<&Ink> {
        self.inks.iter().find(|ink| ink.name == name)
    }

    /// Consistency problems a JSON schema can't catch; empty when the scene is well formed.
    /// `all` resolves placed scenes.
    pub fn validate(&self, all: &[Scene]) -> Vec<String> {
        let mut problems = Vec::new();
        let mut say = |message: String| problems.push(format!("{}: {message}", self.id));
        if !(self.page[0] > 0.0 && self.page[1] > 0.0) {
            say("page size must be positive".into());
        }
        if let Some(name) = &self.settings.output_intent
            && !OUTPUT_INTENTS.contains(&name.as_str())
        {
            say(format!("unknown output intent {name}"));
        }
        for ink in &self.inks {
            if ink.cmyk.is_some() == ink.lab.is_some() {
                say(format!("ink {} needs exactly one of cmyk or lab", ink.name));
            }
            if PROCESS.contains(&ink.name.as_str()) || ink.name == "All" || ink.name == "None" {
                say(format!("ink {} shadows a process or reserved colorant", ink.name));
            }
        }
        let mut objects = Vec::new();
        collect(&self.objects, &mut objects);
        for object in objects {
            for problem in self.object_problems(object, all) {
                say(problem);
            }
        }
        for plate in &self.expect.plates {
            if !PROCESS.contains(&plate.as_str()) && !self.declares_spot(plate, all, 0) {
                say(format!("plate {plate} is neither process nor a declared spot"));
            }
        }
        for name in PROCESS {
            if !self.expect.plates.iter().any(|plate| plate == name) {
                say(format!("plates must list the process plate {name}"));
            }
        }
        for point in &self.expect.points {
            if point.tol <= 0.0 {
                say(format!("point {:?}: tol must be positive", point.at));
            }
            if (point.kind == PointKind::Text) != point.region.is_some() {
                say(format!("point {:?}: text points need a region, area points none", point.at));
            }
            if let Some(plate) = &point.plate
                && (point.kind != PointKind::Text || !self.expect.plates.contains(plate))
            {
                say(format!("point {:?}: plate {plate} must be a text point's plate from plates", point.at));
            }
            for name in point.inks.keys() {
                if !self.expect.plates.contains(name) {
                    say(format!("point {:?}: ink {name} is not in plates", point.at));
                }
            }
        }
        problems
    }

    /// Declared here or in a placed scene (placements nest at most [`MAX_NESTING`] deep).
    fn declares_spot(&self, name: &str, all: &[Scene], depth: usize) -> bool {
        self.ink(name).is_some()
            || (depth < MAX_NESTING
                && placed(&self.objects).iter().any(|id| all.iter().any(|s| s.id == **id && s.declares_spot(name, all, depth + 1))))
    }

    fn object_problems(&self, object: &Object, all: &[Scene]) -> Vec<String> {
        let mut out = Vec::new();
        let kind = object.kind;
        let needs_rect = matches!(kind, Kind::Rect | Kind::Ellipse | Kind::Image | Kind::Pdf);
        if needs_rect != object.rect.is_some() {
            out.push(format!("{kind:?}: rect is {}", if needs_rect { "required" } else { "not used" }));
        }
        if let Some([_, _, w, h]) = object.rect
            && !(w > 0.0 && h > 0.0)
        {
            out.push(format!("{kind:?}: rect needs a positive size"));
        }
        let is_text = kind == Kind::Text;
        if is_text != (object.at.is_some() && object.size.is_some() && object.text.is_some()) {
            out.push(format!("{kind:?}: at, size and text belong to (and are required by) text"));
        }
        if (kind == Kind::Image) != object.image.is_some() {
            out.push(format!("{kind:?}: image belongs to (and is required by) image objects"));
        }
        if let Some(image) = &object.image {
            let sources = [image.ramp.is_some(), image.split.is_some(), image.feather.is_some()];
            if sources.iter().filter(|s| **s).count() != 1 {
                out.push("image needs exactly one of ramp, split or feather".into());
            }
            if image.px.iter().any(|px| !(1..=MAX_IMAGE_PX).contains(px)) {
                out.push(format!("image px must be 1–{MAX_IMAGE_PX}"));
            }
            if image.alpha.as_ref().is_some_and(|a| image.feather.is_some() || !(0.0..=1.0).contains(&a.opacity)) {
                out.push("image alpha needs a color image and an opacity of 0–1".into());
            }
        }
        if (kind == Kind::Pdf) != object.scene.is_some() {
            out.push(format!("{kind:?}: scene belongs to (and is required by) pdf objects"));
        }
        if let Some(id) = &object.scene
            && (id == &self.id || !all.iter().any(|s| &s.id == id))
        {
            out.push(format!("pdf: unknown scene {id}"));
        }
        if kind != Kind::Group && (!object.objects.is_empty() || object.isolated || object.knockout) {
            out.push(format!("{kind:?}: objects, isolated and knockout belong to groups"));
        }
        if matches!(kind, Kind::Image | Kind::Pdf | Kind::Group) && (object.fill.is_some() || object.stroke.is_some()) {
            out.push(format!("{kind:?}: fill and stroke apply to shapes and text"));
        }
        if let Some(opacity) = object.opacity
            && !(0.0..=1.0).contains(&opacity)
        {
            out.push("opacity must be 0–1".into());
        }
        let mut paints: Vec<&Paint> = object.fill.iter().chain(object.stroke.iter().map(|s| &s.paint)).collect();
        if let Some(image) = &object.image {
            paints.extend(image.ramp.iter().flatten());
            paints.extend(image.split.iter().flatten());
        }
        if let Some(mask) = &object.mask {
            paints.extend(mask.backdrop.iter());
            let mut nested = Vec::new();
            collect(&mask.content, &mut nested);
            for inner in nested {
                out.extend(self.object_problems(inner, all));
            }
        }
        for paint in paints {
            self.paint_problems(paint, &mut out);
        }
        out
    }

    fn paint_problems(&self, paint: &Paint, out: &mut Vec<String>) {
        let percent = |v: &f32| (0.0..=100.0).contains(v);
        match paint {
            Paint::Named(_) => {}
            Paint::Cmyk { cmyk } if !cmyk.iter().all(percent) => out.push("cmyk values must be 0–100".into()),
            Paint::Gray { gray } if !percent(gray) => out.push("gray must be 0–100".into()),
            Paint::Rgb { rgb } if !rgb.iter().all(|v| (0.0..=255.0).contains(v)) => out.push("rgb values must be 0–255".into()),
            Paint::Spot { spot, tint } => {
                if self.ink(spot).is_none() {
                    out.push(format!("spot {spot} is not declared in inks"));
                }
                if !percent(tint) {
                    out.push("tint must be 0–100".into());
                }
            }
            Paint::Inks { inks } => {
                for (name, value) in inks {
                    if !PROCESS.contains(&name.as_str()) && self.ink(name).is_none() {
                        out.push(format!("ink {name} is not process or declared"));
                    }
                    if !percent(value) {
                        out.push("ink values must be 0–100".into());
                    }
                }
            }
            Paint::Linear { linear } => {
                if linear.stops.len() < 2 {
                    out.push("gradients need at least two stops".into());
                }
                for stop in &linear.stops {
                    if matches!(stop.paint, Paint::Linear { .. } | Paint::Mesh { .. }) {
                        out.push("gradient stops must be solid paints".into());
                    }
                    self.paint_problems(&stop.paint, out);
                }
            }
            Paint::Mesh { mesh } => {
                for corner in &mesh.corners {
                    if matches!(corner, Paint::Linear { .. } | Paint::Mesh { .. }) {
                        out.push("mesh corners must be solid paints".into());
                    }
                    self.paint_problems(corner, out);
                }
            }
            _ => {}
        }
    }
}

/// Every object in the tree, depth first (mask contents excluded).
fn collect<'a>(objects: &'a [Object], out: &mut Vec<&'a Object>) {
    for object in objects {
        out.push(object);
        collect(&object.objects, out);
    }
}

fn placed(objects: &[Object]) -> Vec<&String> {
    let mut all = Vec::new();
    collect(objects, &mut all);
    all.into_iter().filter_map(|o| o.scene.as_ref()).collect()
}
