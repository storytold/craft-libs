use super::{Deserialize, Serialize, logo};

/// Tracing mode (Illustrator's Mode popup).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    #[serde(alias = "bw", alias = "blackWhite", alias = "Black and White")]
    BlackAndWhite,
    #[serde(alias = "gray", alias = "Grayscale")]
    Grayscale,
    #[serde(alias = "Color")]
    Color,
    /// Flat-colour logos: sub-pixel edges, sharp corners restored, true lines and circles.
    #[serde(alias = "Logo", alias = "flat", alias = "flatLogo")]
    Logo,
}

/// How colour layers relate (Advanced → Method).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    /// Each colour traced on its own; neighbouring shapes share their edge.
    #[default]
    Abutting,
    /// Stacked: every layer extends under the layers above it, within the areas that hold its own
    /// color. With Fills (without Create Strokes) each shape is filled whole, and what it encloses
    /// is stacked above it.
    Overlapping,
}

/// Where Color mode's colours come from (the Palette popup).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Palette {
    /// The `colors` colours that best represent the image.
    #[default]
    #[serde(alias = "Limited")]
    Limited,
    /// As many colours as the image's tones need, up to [`FULL_TONE_MAX`] (photos, gradients):
    /// `color_detail` sets how close two colours may be and stay apart. Before merging colors that
    /// look alike, it splits to at least one color per step of `color_detail` where the image has that many.
    #[serde(alias = "Full Tone")]
    FullTone,
    /// Flat art (a few colours cover nearly all of it) traces with exactly those colours, anything
    /// else with as many as its tones need, without Full Tone's least count and merging close RGB colors.
    #[serde(alias = "Automatic")]
    Automatic,
    /// The most used colours of a swatch library ([`TraceParams::swatches`]), at most `colors` of
    /// them, exactly as they are. Each pixel takes the library color nearest in lightness and
    /// hue, with hue and chroma counted three times as much as lightness, so that neutral tones
    /// stay off saturated colors.
    #[serde(alias = "library", alias = "Document Library")]
    DocumentLibrary,
}

/// Image Trace parameters (the Image Trace panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TraceParams {
    pub mode: Mode,
    /// Black and White: pixels darker than this (luminance 0–255) are black.
    pub threshold: u8,
    /// Color: where the colours come from.
    pub palette: Palette,
    /// Palette size for Color (Limited; at most this many for Document Library), number of grays
    /// for Grayscale (2–256).
    pub colors: u32,
    /// Full Tone and Automatic: how many colours, 0 (few) – 100 (many).
    pub color_detail: f64,
    /// Document Library: the swatch library, `"document"` (the document's swatches) or a library id
    /// or name. The caller looks its colours up into `swatches`.
    pub library: String,
    /// Document Library with the document's swatches: the name of the color group whose colors are
    /// used. `None` (or empty) uses every swatch. `imageTrace.make` refuses a name with any other
    /// palette, library or mode.
    pub color_group: Option<String>,
    /// Document Library: the library's colours (sRGB). Not saved: they are looked up from
    /// `library` (and `color_group`) at every trace.
    #[serde(skip)]
    pub swatches: Vec<[u8; 3]>,
    /// Paths (fidelity) 0–100: higher follows the pixels more tightly.
    pub paths: f64,
    /// Corners 0–100: higher keeps more corners.
    pub corners: f64,
    /// Noise: areas smaller than this many pixels are ignored: each joins the color it shares the
    /// longest border with, or, when that color is far from its own, the color beside it nearest
    /// its source pixels. (Flat Logo mode: areas smaller than `noise / 10` pixels of the source
    /// image.)
    pub noise: u32,
    pub method: Method,
    /// Ignore Color: drop the areas of one colour (no background shape): white, unless
    /// `ignore_color` names another.
    pub ignore_white: bool,
    /// The colour Ignore Color leaves out (`#rrggbb`): the palette colours within about 30 levels
    /// of it. `None` is white. (Flat Logo mode leaves out white only.)
    pub ignore_color: Option<String>,
    /// With Ignore Color: leave out only the areas of that color that reach the image's edge (a
    /// background), through neighbors above, below and to either side, and trace the areas of it
    /// that other colors enclose. (Flat Logo mode leaves out every white area.)
    pub ignore_background: bool,
    /// Keep Colors: the colors to trace (`#rrggbb`). Only the palette colors within about 30
    /// levels of one of them (as Ignore Color matches a color) are traced, a gradient by its middle
    /// color, and their paths are the ones the trace of every color has. Empty traces every color.
    /// (Flat Logo mode does not use it: it traces the colors named in `logo_colors`.)
    pub keep_colors: Vec<String>,
    /// Replace nearly-straight curves with straight lines. In Logo mode this also snaps tight arcs
    /// to true circles.
    pub snap_curves_to_lines: bool,
    /// Shapes: a filled outline that is a circle, an ellipse, a rectangle (square or rounded
    /// corners) or a regular polygon comes out as exactly that shape ([`TracedPath::shape`]).
    pub shapes: bool,
    /// Auto Grouping: say which path lies inside which ([`TraceResult::parents`]).
    pub auto_grouping: bool,
    /// Gradients (Grayscale and Color): an area whose colour changes evenly along one direction
    /// is one path filled with a linear gradient ([`TracedPath::gradient`]), not bands of colour.
    /// Grayscale finds them in the image's grays, and their stops are gray.
    pub gradients: bool,
    /// Create: trace areas as filled shapes.
    pub fills: bool,
    /// Create: trace lines no wider than `stroke_width`, and such thin features of wider shapes, as
    /// stroked center lines.
    pub strokes: bool,
    /// Stroke: the widest feature (px) traced as a stroke.
    pub stroke_width: f64,
    /// Flat Logo mode: the flat colours (`#rrggbb`) to use; empty finds them automatically. White is
    /// traced on its own and cannot be named unless [`logo_white`](Self::logo_white) is off, colours
    /// closer than about 12 (RGB distance) count as one, and at most ten are used. Whether the image
    /// is flat art (and not refused) is judged by its own colors, which are found the same way as when
    /// no colors are named; as many are looked for as are named, with white, or as
    /// [`colors`](Self::colors) gives, whichever is more. (Not `palette`: that is Color mode's colour source.)
    pub logo_colors: Vec<String>,
    /// Flat Logo mode with named `logo_colors`: `true` traces white (every channel 245 or more) as a
    /// color of its own beside the named ones. `false` traces the named colors only: white areas
    /// take the nearest of them, a named white or near-white is used as named, and enclosed
    /// transparent areas stay holes. With the colors found automatically, white is always its own,
    /// and with [`ignore_white`](Self::ignore_white) white areas are left out whatever this says.
    pub logo_white: bool,
}

impl Default for TraceParams {
    /// Illustrator's [Default]: black and white, threshold 128.
    fn default() -> Self {
        Self {
            mode: Mode::BlackAndWhite,
            threshold: 128,
            palette: Palette::Limited,
            colors: 6,
            color_detail: 50.0,
            library: "document".into(),
            color_group: None,
            swatches: vec![],
            paths: 50.0,
            corners: 75.0,
            noise: 25,
            method: Method::Abutting,
            ignore_white: false,
            ignore_color: None,
            ignore_background: false,
            keep_colors: Vec::new(),
            snap_curves_to_lines: false,
            shapes: false,
            auto_grouping: false,
            gradients: false,
            fills: true,
            strokes: false,
            stroke_width: 10.0,
            logo_colors: Vec::new(),
            logo_white: true,
        }
    }
}

impl TraceParams {
    /// How outlines are fitted: Paths sets how closely the curves follow the edge (0.08 px at 100,
    /// 0.33 px at 50, 0.78 px at 0), Corners the turn that makes a corner (35° at 100, 90° at 0).
    /// Straight edges are always lines; Snap Curves To Lines also straightens slightly curved ones,
    /// makes parts of circles true arcs, and makes lines within 2.5° of level or upright exactly so.
    pub(crate) fn outline_style(&self) -> logo::Style {
        // Parameters come from users and agents: a NaN or infinity must not reach the geometry.
        let unit = |v: f64, default: f64| if v.is_finite() { (v / 100.0).clamp(0.0, 1.0) } else { default };
        let snap = self.snap_curves_to_lines;
        logo::Style {
            step: 0.25,
            tol: 0.08 + 0.7 * (1.0 - unit(self.paths, 0.5)).powf(1.5),
            corner_deg: 90.0 - 55.0 * unit(self.corners, 0.75),
            snap: true,
            arcs: snap,
            line_tol: if snap { 0.5 } else { 0.2 },
            circle_tol: if snap { 0.3 } else { 0.15 },
            restore: Some(logo::RestoreParams::default()),
            check_step: 0.15,
            upright: snap,
            ..logo::Style::default()
        }
    }
}

/// Built-in preset names in panel order (our own parameter sets).
pub const PRESET_NAMES: &[&str] = &[
    "Default",
    "Auto Color",
    "High Fidelity Photo",
    "Low Fidelity Photo",
    "3 Colors",
    "6 Colors",
    "16 Colors",
    "Shades of Gray",
    "Black and White Logo",
    "Sketched Art",
    "Silhouettes",
    "Line Art",
    "Technical Drawing",
    "Outline",
    "Flat Logo",
];

/// A built-in preset by name (case-insensitive; `[Default]` is accepted).
pub fn preset(name: &str) -> Option<TraceParams> {
    let key = name.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    let d = TraceParams::default();
    let color = |colors: u32, paths: f64, corners: f64, noise: u32, method: Method| TraceParams {
        mode: Mode::Color,
        colors,
        paths,
        corners,
        noise,
        method,
        ..TraceParams::default()
    };
    let bw = |threshold: u8, paths: f64, corners: f64, noise: u32, snap: bool| TraceParams {
        mode: Mode::BlackAndWhite,
        threshold,
        paths,
        corners,
        noise,
        ignore_white: true,
        snap_curves_to_lines: snap,
        ..TraceParams::default()
    };
    Some(match key.as_str() {
        "default" => d,
        // A posterised image of a photo or of artwork: its own colours where it is flat art, as
        // many as its tones need where it is not.
        // (Color detail 85: a colour on a hundredth of the picture is still one of its colours.)
        "auto color" => TraceParams { palette: Palette::Automatic, color_detail: 85.0, ..color(6, 65.0, 60.0, 16, Method::Abutting) },
        // A photo's tones: as many colors as they need (Full Tone), split until no color spans more
        // than about 21 levels of a channel and into at least 85 (Color detail 85) before colors that
        // look alike merge, as abutting shapes. VectorCraft's own values.
        "high fidelity photo" => TraceParams { palette: Palette::FullTone, color_detail: 85.0, ..color(64, 50.0, 50.0, 5, Method::Abutting) },
        "low fidelity photo" => color(20, 50.0, 50.0, 10, Method::Overlapping),
        "3 colors" => color(3, 50.0, 50.0, 15, Method::Abutting),
        "6 colors" => color(6, 50.0, 50.0, 8, Method::Abutting),
        "16 colors" => color(16, 50.0, 50.0, 8, Method::Abutting),
        "shades of gray" => TraceParams { mode: Mode::Grayscale, colors: 8, paths: 50.0, corners: 50.0, noise: 15, method: Method::Overlapping, ..d },
        "black and white logo" => bw(128, 50.0, 75.0, 5, true),
        "sketched art" => bw(150, 50.0, 50.0, 20, false),
        "silhouettes" => bw(200, 50.0, 50.0, 90, false),
        "line art" => bw(128, 50.0, 75.0, 20, false),
        "technical drawing" => bw(128, 50.0, 100.0, 1, true),
        // Black outlines: lines as stroked centre lines, wider areas as stroked outlines.
        "outline" => TraceParams { fills: false, strokes: true, ..bw(128, 50.0, 75.0, 20, false) },
        // Flat-colour logos: colours found automatically, edges placed to a fraction of a pixel.
        "flat logo" => TraceParams { mode: Mode::Logo, colors: 8, paths: 60.0, corners: 50.0, noise: 6, snap_curves_to_lines: true, ..d },
        _ => return None,
    })
}

/// All built-in presets in panel order.
pub fn presets() -> Vec<(&'static str, TraceParams)> {
    PRESET_NAMES.iter().filter_map(|n| Some((*n, preset(n)?))).collect()
}

/// The enhanced version of built-in preset `name`: the preset with the options the enhanced
/// presets have built in: Auto Grouping in all of them, Shapes in all but High Fidelity Photo.
/// Gradients stay an option to turn on. Flat Logo, a preset of our own, has none.
pub fn enhanced_preset(name: &str) -> Option<TraceParams> {
    let p = preset(name).filter(|p| p.mode != Mode::Logo)?;
    Some(TraceParams { auto_grouping: true, shapes: p != preset("High Fidelity Photo")?, ..p })
}
