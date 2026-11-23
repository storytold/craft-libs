//! The neutral stroke model: points, pressure, brush, colour, width, draw order and key.
//!
//! Coordinates are centimetres, the unit of an Office ink part, quantised to the format's
//! 1/1000 cm resolution ([`TRACE_UNITS_PER_CM`]). A host that draws on a 96 dpi page converts its
//! sheet points with [`px_to_cm`] and [`cm_to_px`]; nothing here knows about a page, a cell or a
//! slide.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

/// Most points one stroke may hold. A longer stroke is not stored: the cap is the one the Office
/// ink writers use, and it bounds every per-point loop in this crate.
pub const MAX_POINTS: usize = 20_000;
/// Largest coordinate magnitude the format stores, in centimetres: an InkML channel value is an
/// `i32` in 1/1000 cm units.
pub const MAX_COORD_CM: f64 = 2_147_483.647;
/// Smallest brush dimension the format stores, in centimetres.
pub const MIN_BRUSH_CM: f32 = 0.001;
/// Largest brush dimension accepted, in centimetres.
pub const MAX_BRUSH_CM: f32 = 100.0;
/// Largest texture phase offset accepted, in the texture's own units.
pub const MAX_ANCHOR: f32 = 1_000_000.0;
/// Largest texture scale accepted.
pub const MAX_SCALE: f32 = 1_000.0;
/// Largest pressure value the format stores (a signed 16-bit channel).
pub const MAX_PRESSURE: u16 = 32_767;
/// Trace units per centimetre: InkML channel values are integers in 1/1000 cm.
pub const TRACE_UNITS_PER_CM: f64 = 1000.0;
/// Centimetres to CSS/Office sheet points at 96 dpi (1 cm = 96/2.54 px).
pub const PX_PER_CM: f64 = 96.0 / 2.54;

/// A brush kind. The five kinds are the ones an Office Draw tab offers; a texture
/// ([`Effect`]) is drawn by a brush, not a kind of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub enum Tool {
    /// A plain pen.
    #[default]
    Pen,
    /// A pencil: thin, hard-edged, taller across the stroke than along it.
    Pencil,
    /// A highlighter: wide, translucent, drawn under the ink.
    Highlighter,
    /// A fountain pen: pressure changes the stroke's width.
    FountainPen,
    /// A brush pen: a soft, wet edge.
    BrushPen,
}

impl Tool {
    /// Every brush kind, in the order the Draw tab shows them.
    pub const ALL: [Tool; 5] = [Tool::Pen, Tool::Pencil, Tool::Highlighter, Tool::FountainPen, Tool::BrushPen];

    /// The canonical name, in JSON's `camelCase` spelling.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Pen => "pen",
            Tool::Pencil => "pencil",
            Tool::Highlighter => "highlighter",
            Tool::FountainPen => "fountainPen",
            Tool::BrushPen => "brushPen",
        }
    }

    /// The name an ink part carries in its private `craftTool` brush property.
    pub fn inkml_name(self) -> &'static str {
        self.name()
    }

    /// Parses a brush kind case- and separator-insensitively (`fountain_pen`, `fountain-pen`,
    /// `fountainPen`). `"fountain"` and `"brush"` are accepted as the app commands do.
    pub fn from_name(s: &str) -> Option<Tool> {
        match s.trim().to_ascii_lowercase().replace(['_', '-', ' '], "").as_str() {
            "pen" => Some(Tool::Pen),
            "pencil" => Some(Tool::Pencil),
            "highlighter" => Some(Tool::Highlighter),
            "fountainpen" | "fountain" => Some(Tool::FountainPen),
            "brushpen" | "brush" => Some(Tool::BrushPen),
            _ => None,
        }
    }
}

/// A pen texture. The stroke's [`Color`] is the texture's base colour, as Office stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub enum Effect {
    /// A rainbow band.
    Rainbow,
    /// A star field.
    Galaxy,
    /// Molten rock.
    Lava,
    /// Water.
    Ocean,
    /// A warm metal.
    RoseGold,
    /// Gold.
    Gold,
    /// Silver.
    Silver,
    /// Bronze.
    Bronze,
}

impl Effect {
    /// The eight textures, in Office's palette order.
    pub const ALL: [Effect; 8] =
        [Effect::Rainbow, Effect::Galaxy, Effect::Lava, Effect::Ocean, Effect::RoseGold, Effect::Gold, Effect::Silver, Effect::Bronze];

    /// The canonical name in JSON's `camelCase` spelling (`roseGold`).
    pub fn name(self) -> &'static str {
        match self {
            Effect::Rainbow => "rainbow",
            Effect::Galaxy => "galaxy",
            Effect::Lava => "lava",
            Effect::Ocean => "ocean",
            Effect::RoseGold => "roseGold",
            Effect::Gold => "gold",
            Effect::Silver => "silver",
            Effect::Bronze => "bronze",
        }
    }

    /// The `inkEffects` name an ink part carries: Office's own lowercase spelling, without the
    /// camel hump (`rosegold`).
    pub fn inkml_name(self) -> &'static str {
        match self {
            Effect::RoseGold => "rosegold",
            other => other.name(),
        }
    }

    /// Parses a texture name case- and separator-insensitively. `"none"`, `"solid"` and unknown
    /// names are `None`.
    pub fn from_name(s: &str) -> Option<Effect> {
        match s.trim().to_ascii_lowercase().replace(['_', '-', ' '], "").as_str() {
            "rainbow" => Some(Effect::Rainbow),
            "galaxy" => Some(Effect::Galaxy),
            "lava" => Some(Effect::Lava),
            "ocean" => Some(Effect::Ocean),
            "rosegold" => Some(Effect::RoseGold),
            "gold" => Some(Effect::Gold),
            "silver" => Some(Effect::Silver),
            "bronze" => Some(Effect::Bronze),
            _ => None,
        }
    }

    /// The base colour Office stores for the texture's stroke, as `#RRGGBB`.
    pub fn base_hex(self) -> &'static str {
        match self {
            Effect::Rainbow => "#E83A6B",
            Effect::Galaxy => "#5B3EA8",
            Effect::Lava => "#D94F1E",
            Effect::Ocean => "#1B7FB5",
            Effect::RoseGold => "#C98A7A",
            Effect::Gold => "#C99A2E",
            Effect::Silver => "#9AA3AD",
            Effect::Bronze => "#9A673A",
        }
    }

    /// [`Effect::base_hex`] as a [`Color`]. The table is constant, so the parse cannot fail; the
    /// fallback keeps this a total function.
    pub fn base_color(self) -> Color {
        Color::from_hex(self.base_hex()).unwrap_or(Color::BLACK)
    }
}

/// An sRGB colour with no alpha: an ink stroke is opaque, and Office's brush `color` property is
/// `#RRGGBB`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Color {
    /// Black, the default brush colour.
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };

    /// A colour from its channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b }
    }

    /// Parses `#RRGGBB` or `RRGGBB`, case-insensitively. Anything else (including the `#AARRGGBB`
    /// form Office uses for shape fills) is `None` rather than silently losing a channel.
    pub fn from_hex(s: &str) -> Option<Color> {
        let h = s.trim();
        let h = h.strip_prefix('#').unwrap_or(h);
        if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        Some(Color { r: byte(0)?, g: byte(2)?, b: byte(4)? })
    }

    /// `#RRGGBB`, uppercase, the spelling an ink part carries.
    pub fn to_hex(self) -> String {
        let mut s = String::with_capacity(7);
        s.push('#');
        let _ = write!(s, "{:02X}{:02X}{:02X}", self.r, self.g, self.b);
        s
    }
}

/// A stroke's brush: its kind, thickness, texture and phase. Draw order and the creation key live
/// on the [`Stroke`], because they belong to the stroke rather than to the brush.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Brush {
    /// The brush kind.
    pub tool: Tool,
    /// Width along the stroke, in centimetres (InkML `width`).
    pub width_cm: f32,
    /// Thickness across the stroke, in centimetres (InkML `height`). A pencil is six times and a
    /// highlighter twice its width; the rest are round.
    pub height_cm: f32,
    /// The texture, or `None` for a solid stroke.
    pub effect: Option<Effect>,
    /// Texture phase (InkML `anchorX`/`anchorY`), in the texture's own units.
    pub anchor: [f32; 2],
    /// Texture scale (InkML `scaleFactor`).
    pub scale: f32,
    /// Brush properties this crate does not interpret, kept in document order so a round trip
    /// does not drop another writer's data. Names that mean something here are refused by
    /// [`Brush::is_valid`] to keep one meaning per name.
    pub extra: Vec<(String, String)>,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { tool: Tool::Pen, width_cm: 0.05, height_cm: 0.05, effect: None, anchor: [0.0, 0.0], scale: 1.0, extra: Vec::new() }
    }
}

impl Brush {
    /// A solid brush of `tool` at the default thickness.
    pub fn new(tool: Tool) -> Brush {
        Brush { tool, ..Brush::default() }
    }

    /// Whether every value is one the format stores and a reader would get back unchanged.
    pub fn is_valid(&self) -> bool {
        let dim = |v: f32| v.is_finite() && (MIN_BRUSH_CM..=MAX_BRUSH_CM).contains(&v);
        dim(self.width_cm)
            && dim(self.height_cm)
            && self.anchor.iter().all(|v| v.is_finite() && v.abs() <= MAX_ANCHOR)
            && self.scale.is_finite()
            && self.scale > 0.0
            && self.scale <= MAX_SCALE
            && self.extra.iter().all(|(name, _)| !name.trim().is_empty() && !is_known_property(name))
    }
}

/// The brush property names this crate reads. A preserved extra property may not reuse one: two
/// meanings under one name would make a round trip lossy or ambiguous.
pub const KNOWN_PROPERTIES: [&str; 14] = [
    "width",
    "height",
    "color",
    "tip",
    "rasterOp",
    "inkEffects",
    "ignorePressure",
    "anchorX",
    "anchorY",
    "scaleFactor",
    "craftOrder",
    "craftTool",
    // The names GridCraft wrote before this crate existed. Read as aliases, never written.
    "gridcraftOrder",
    "gridcraftTool",
];

/// Whether `name` is a brush property this crate interprets.
pub fn is_known_property(name: &str) -> bool {
    KNOWN_PROPERTIES.contains(&name.trim())
}

/// One point of a stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct StrokePoint {
    /// X in centimetres.
    pub x: f64,
    /// Y in centimetres.
    pub y: f64,
    /// Pen pressure, `0..=`[`MAX_PRESSURE`]; `None` when the device recorded none.
    pub pressure: Option<u16>,
}

impl StrokePoint {
    /// A point with no pressure.
    pub const fn new(x: f64, y: f64) -> StrokePoint {
        StrokePoint { x, y, pressure: None }
    }

    /// The point with a pressure value.
    pub const fn with_pressure(self, pressure: u16) -> StrokePoint {
        StrokePoint { pressure: Some(pressure), ..self }
    }

    /// Whether both coordinates are finite and inside the format's range.
    pub fn is_valid(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.x.abs() <= MAX_COORD_CM && self.y.abs() <= MAX_COORD_CM
    }
}

/// A freehand stroke: its points in centimetres, its brush and colour, its place in the drawing
/// order and its stable creation key.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Stroke {
    /// The points, in drawing order.
    pub points: Vec<StrokePoint>,
    /// The brush.
    pub brush: Brush,
    /// The stroke's colour.
    pub color: Color,
    /// The order the stroke was drawn in, ascending. All zero means the file records no order,
    /// and document order stands for it.
    pub order: u32,
    /// The stroke's stable `a16:creationId`, in braces and lowercase, when it has one. The key
    /// belongs to the drawing's content part, not to the InkML part, so the codec neither reads
    /// nor writes it; a host that maps a document keeps it here.
    pub creation_id: Option<String>,
}

impl Default for Stroke {
    fn default() -> Self {
        Stroke { points: Vec::new(), brush: Brush::default(), color: Color::BLACK, order: 0, creation_id: None }
    }
}

impl Stroke {
    /// A stroke of `points` in `color`, drawn with `brush`.
    pub fn new(points: Vec<StrokePoint>, brush: Brush, color: Color) -> Stroke {
        Stroke { points, brush, color, ..Stroke::default() }
    }

    /// Whether pressure was recorded for the whole stroke. The format gives a trace one `F`
    /// channel for every point, so a partially recorded stroke cannot round trip: a host either
    /// has pressure for all points or for none.
    pub fn has_pressure(&self) -> bool {
        self.points.first().is_some_and(|p| p.pressure.is_some())
    }

    /// Whether every value is one the format stores and a reader would get back unchanged.
    pub fn is_valid(&self) -> bool {
        self.invalid_reason().is_none()
    }

    /// The first reason this stroke cannot be written, or `None` when it can.
    pub fn invalid_reason(&self) -> Option<&'static str> {
        if self.points.is_empty() {
            return Some("a stroke needs at least one point");
        }
        if self.points.len() > MAX_POINTS {
            return Some("the stroke holds more than the point limit");
        }
        if !self.points.iter().all(StrokePoint::is_valid) {
            return Some("a point is not finite or is outside the format's range");
        }
        if self.points.iter().any(|p| p.pressure.is_some_and(|v| v > MAX_PRESSURE)) {
            return Some("a pressure is above the format's maximum");
        }
        if self.points.iter().all(|p| p.pressure.is_some()) != self.has_pressure() {
            return Some("pressure is recorded for only some points");
        }
        if !self.brush.is_valid() {
            return Some("the brush is not one the format stores");
        }
        None
    }

    /// The stroke's length in centimetres: the sum of its segment lengths, ignoring non-finite
    /// ends. A single point has length zero.
    pub fn length(&self) -> f64 {
        self.points
            .windows(2)
            .filter(|w| w.iter().all(|p| p.x.is_finite() && p.y.is_finite()))
            .map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y))
            .sum()
    }
}

/// A pen of a drawing tool gallery: the brush, colour, thickness and texture a host offers.
///
/// The six defaults are the gallery Office shows first; their ids are stable so a saved tool
/// selection survives a restart. Labels and icons belong to the application.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenPreset {
    /// The gallery id.
    pub id: u32,
    /// The brush kind.
    pub tool: Tool,
    /// The pen's colour.
    pub color: Color,
    /// The pen's thickness in millimetres.
    pub width_mm: f32,
    /// The texture, when the pen is textured.
    pub effect: Option<Effect>,
}

/// Office's six default pens, in gallery order: a textured pen, a red pen, a pencil, a
/// highlighter, a fountain pen and a brush pen.
pub const DEFAULT_PENS: [PenPreset; 6] = [
    PenPreset { id: 0, tool: Tool::Pen, color: Color { r: 0x5B, g: 0x3E, b: 0xA8 }, width_mm: 0.5, effect: Some(Effect::Galaxy) },
    PenPreset { id: 1, tool: Tool::Pen, color: Color { r: 0xD1, g: 0x3B, b: 0x2F }, width_mm: 0.5, effect: None },
    PenPreset { id: 2, tool: Tool::Pencil, color: Color { r: 0x76, g: 0x76, b: 0x76 }, width_mm: 0.5, effect: None },
    PenPreset { id: 3, tool: Tool::Highlighter, color: Color { r: 0xF2, g: 0xE3, b: 0x3A }, width_mm: 4.0, effect: None },
    PenPreset { id: 4, tool: Tool::FountainPen, color: Color { r: 0x1F, g: 0x38, b: 0x64 }, width_mm: 0.5, effect: None },
    PenPreset { id: 5, tool: Tool::BrushPen, color: Color { r: 0x2E, g: 0x7D, b: 0x4F }, width_mm: 0.5, effect: None },
];

/// Pen thicknesses in millimetres, the steps a Draw tab offers.
pub const PEN_WIDTHS_MM: [f32; 6] = [0.25, 0.35, 0.5, 1.0, 2.0, 3.5];

/// Highlighter thicknesses in millimetres: its own wider steps.
pub const HIGHLIGHTER_WIDTHS_MM: [f32; 5] = [2.0, 4.0, 6.0, 8.0, 10.0];

/// The thickness steps a brush kind offers.
pub fn widths_mm(tool: Tool) -> &'static [f32] {
    match tool {
        Tool::Highlighter => &HIGHLIGHTER_WIDTHS_MM,
        _ => &PEN_WIDTHS_MM,
    }
}

/// The InkML width and height in centimetres of a stroke `width_mm` millimetres thick in `tool`'s
/// brush: a pencil is six times as tall across the stroke, a highlighter twice; the rest are
/// round. Widths are clamped to the format's range so a caller cannot ask for an unwritable brush.
pub fn ink_size_cm(tool: Tool, width_mm: f32) -> (f32, f32) {
    let w = if width_mm.is_finite() { (width_mm / 10.0).clamp(MIN_BRUSH_CM, 5.0) } else { 0.05 };
    match tool {
        Tool::Pencil => (w, 6.0 * w),
        Tool::Highlighter => (w, 2.0 * w),
        _ => (w, w),
    }
}

/// Centimetres to sheet points at 96 dpi. Non-finite input passes through as the same non-finite
/// value, which callers' own bounds checks then refuse.
pub fn cm_to_px(cm: f64) -> f64 {
    cm * PX_PER_CM
}

/// Sheet points at 96 dpi to centimetres.
pub fn px_to_cm(px: f64) -> f64 {
    px / PX_PER_CM
}

/// The canonical form of an `a16:creationId` value: `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`
/// with hexadecimal digits (case-insensitive), in lowercase. `None` for anything else, so a key
/// read from a file or restored from JSON can never be written back as an invalid `ST_Guid`.
pub fn guid(value: &str) -> Option<String> {
    let v = value.trim();
    let b = v.as_bytes();
    if b.len() != 38 || b.first() != Some(&b'{') || b.last() != Some(&b'}') {
        return None;
    }
    for (i, byte) in b.iter().enumerate() {
        if i == 0 || i == 37 {
            continue;
        }
        if matches!(i, 9 | 14 | 19 | 24) {
            if *byte != b'-' {
                return None;
            }
        } else if !byte.is_ascii_hexdigit() {
            return None;
        }
    }
    Some(v.to_ascii_lowercase())
}

/// How many keys [`mint_free`] tries before it returns the last candidate anyway. A caller's
/// `taken` answers from a finite set (a document's strokes), so the bound is never reached in
/// practice; it keeps a faulty callback from hanging the caller.
const MAX_MINT_ATTEMPTS: u32 = 1 << 20;

/// A stroke's stable key: `existing` in its canonical lowercase form when it is an `ST_Guid`,
/// else a fresh one.
///
/// The key is deterministic for a run (a counter through a mixing function, so tests and repeated
/// saves agree and no clock or random source is read — a wasm build has neither). It is only
/// unique within one process, though; see [`mint_free`] before giving one to a new stroke.
pub fn mint(existing: Option<&str>) -> String {
    if let Some(guid) = existing.and_then(guid) {
        return guid;
    }
    format_guid(&next_guid_bytes())
}

/// A fresh key for a new stroke: the first one [`mint`] produces that `taken` does not already
/// hold.
///
/// A reopened document carries keys minted by an earlier process, whose counter started at zero,
/// so the first few keys this process mints are very likely among them. Handing one out twice
/// would name two strokes with one id. `taken` answers from the document's own strokes, a finite
/// set, so the loop only moves the counter forward and ends.
pub fn mint_free(mut taken: impl FnMut(&str) -> bool) -> String {
    let mut key = String::new();
    for _ in 0..MAX_MINT_ATTEMPTS {
        key = mint(None);
        if !taken(&key) {
            return key;
        }
    }
    key
}

/// The counter behind [`mint`], process-wide and starting at zero.
static NEXT_KEY: AtomicU64 = AtomicU64::new(0);

/// Sixteen key bytes from the process counter. Two independent mixed words give every key a
/// distinct, well-spread value without a hash dependency.
fn next_guid_bytes() -> [u8; 16] {
    let n = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    let a = mix64(n ^ 0x6372_6166_7469_6e6b);
    let b = mix64(a ^ 0x1d8e_4e27_c47d_124f);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a.to_be_bytes());
    out[8..].copy_from_slice(&b.to_be_bytes());
    // RFC 9562 version 4 (random) shape and the RFC 4122 variant, so the value is a well-formed
    // UUID even though its source is a counter. No randomness is claimed.
    out[6] = (out[6] & 0x0F) | 0x40;
    out[8] = (out[8] & 0x3F) | 0x80;
    out
}

/// SplitMix64's finaliser.
fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The braced, lowercase spelling of sixteen key bytes.
fn format_guid(bytes: &[u8; 16]) -> String {
    let mut s = String::with_capacity(38);
    s.push('{');
    for (i, byte) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            s.push('-');
        }
        let _ = write!(s, "{byte:02x}");
    }
    s.push('}');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_and_print_without_losing_a_channel() {
        assert_eq!(Color::from_hex("#A12BCD"), Some(Color::rgb(0xA1, 0x2B, 0xCD)));
        assert_eq!(Color::from_hex("a12bcd"), Some(Color::rgb(0xA1, 0x2B, 0xCD)));
        assert_eq!(Color::rgb(1, 2, 3).to_hex(), "#010203");
        for bad in ["", "#", "#12345", "#1234567", "#GGGGGG", "#FFA12BCD", "red"] {
            assert_eq!(Color::from_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn brush_kinds_and_textures_round_trip_their_names() {
        for tool in Tool::ALL {
            assert_eq!(Tool::from_name(tool.name()), Some(tool));
            assert_eq!(Tool::from_name(&tool.name().to_ascii_uppercase()), Some(tool));
        }
        assert_eq!(Tool::from_name("fountain_pen"), Some(Tool::FountainPen));
        assert_eq!(Tool::from_name("brush"), Some(Tool::BrushPen));
        assert_eq!(Tool::from_name("eraser"), None);
        for effect in Effect::ALL {
            assert_eq!(Effect::from_name(effect.name()), Some(effect));
            assert_eq!(Effect::from_name(effect.inkml_name()), Some(effect));
            assert_eq!(Color::from_hex(effect.base_hex()), Some(effect.base_color()));
        }
        assert_eq!(Effect::from_name("none"), None);
        assert_eq!(Effect::from_name("solid"), None);
    }

    #[test]
    fn widths_scale_with_the_brush_kind() {
        assert_eq!(widths_mm(Tool::Highlighter), &HIGHLIGHTER_WIDTHS_MM);
        assert_eq!(widths_mm(Tool::Pen), &PEN_WIDTHS_MM);
        assert_eq!(ink_size_cm(Tool::Pen, 1.0), (0.1, 0.1));
        assert_eq!(ink_size_cm(Tool::Pencil, 1.0), (0.1, 0.6));
        let (w, h) = ink_size_cm(Tool::Highlighter, 4.0);
        assert!((w - 0.4).abs() < 1e-6 && (h - 0.8).abs() < 1e-6);
        assert_eq!(ink_size_cm(Tool::Pen, f32::NAN).0, 0.05);
    }

    #[test]
    fn a_key_keeps_a_valid_value_and_mints_unique_ones() {
        let valid = "{11111111-2222-4333-8444-555555555555}";
        assert_eq!(mint(Some(valid)), valid);
        let a = mint(None);
        let b = mint(None);
        assert_ne!(a, b);
        for key in [a, b] {
            assert_eq!(guid(&key).as_deref(), Some(key.as_str()));
            let bytes = key.as_bytes();
            assert_eq!(bytes.get(15), Some(&b'4'), "version 4: {key}");
            assert!(matches!(bytes.get(20), Some(b'8' | b'9' | b'a' | b'b')), "RFC 4122 variant: {key}");
        }
        for bad in ["", "{}", "{x}", "not a guid", "{11111111-2222-4333-8444-55555555555}"] {
            assert_eq!(guid(bad), None, "{bad}");
            assert_ne!(mint(Some(bad)), bad);
        }
        assert_eq!(guid(" {ABCDEF01-2345-6789-ABCD-EF0123456789} ").as_deref(), Some("{abcdef01-2345-6789-abcd-ef0123456789}"));
    }

    #[test]
    fn a_free_key_steps_past_the_keys_a_document_holds() {
        let mut refused = 0u32;
        let key = mint_free(|_| {
            refused += 1;
            refused <= 3
        });
        assert_eq!(refused, 4);
        assert!(!mint_free(|_| false).is_empty());
        assert_ne!(key, mint(None));
    }

    #[test]
    fn stroke_validity_rejects_partial_pressure_and_out_of_range_values() {
        let brush = Brush::default();
        let mut stroke = Stroke::new(vec![StrokePoint::new(1.0, 2.0), StrokePoint::new(3.0, 4.0)], brush.clone(), Color::BLACK);
        assert!(stroke.is_valid());
        assert!(!stroke.has_pressure());
        stroke.points[0].pressure = Some(1);
        assert!(!stroke.is_valid(), "pressure on one point only");
        stroke.points[1].pressure = Some(MAX_PRESSURE);
        assert!(stroke.is_valid());
        stroke.points[1].pressure = Some(MAX_PRESSURE + 1);
        assert!(!stroke.is_valid());
        stroke.points[1].pressure = None;
        stroke.points[1].pressure = Some(0);
        stroke.points[0].x = f64::NAN;
        assert!(!stroke.is_valid());
        stroke.points[0].x = MAX_COORD_CM + 1.0;
        assert!(!stroke.is_valid());
        stroke.points[0].x = 0.0;
        assert!((stroke.length() - 13f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn a_malformed_brush_is_not_written() {
        let mut brush = Brush::default();
        assert!(brush.is_valid());
        for width in [0.0, -1.0, MAX_BRUSH_CM + 1.0, f32::NAN, f32::INFINITY] {
            brush.width_cm = width;
            assert!(!brush.is_valid(), "{width}");
        }
        brush.width_cm = 0.05;
        brush.scale = 0.0;
        assert!(!brush.is_valid());
        brush.scale = 1.0;
        brush.extra.push(("width".into(), "1".into()));
        assert!(!brush.is_valid(), "an extra property may not shadow a read one");
        brush.extra.clear();
        brush.extra.push(("vendorPhase".into(), "2".into()));
        assert!(brush.is_valid());
    }
}
