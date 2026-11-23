//! A bounded reader and writer for the Office InkML part behind a drawing's `xdr:contentPart`.
//!
//! The markup is the subset Office writes, with the W3C InkML trace encoding ([W3C InkML 1.0],
//! <https://www.w3.org/TR/InkML/>): a `<inkml:ink>` holding `<inkml:definitions>` (a `context`
//! with an `inkSource`/`traceFormat`/`channelProperties`, and a `brush` with `brushProperty`
//! elements) and one `<inkml:trace>` per stroke. Values are integers in 1/1000 cm; a trace's
//! points use the difference-order grammar `!` (explicit), `'` (first difference), `"` (second
//! difference), `*` (unchanged), with an unprefixed value inheriting its channel's previous
//! prefix.
//!
//! The container is the application's: the `mc:AlternateContent` choice, the `xdr:contentPart`,
//! its `customXml` relationship, the `[Content_Types].xml` `application/inkml+xml` declaration,
//! the `a16:creationId` extension and the mandatory PNG fallback picture. See *[MS-ODRAWXML]:
//! Office Drawing Extensions to Office Open XML Structure*,
//! <https://learn.microsoft.com/openspecs/office_standards/ms-odrawxml/>. This module reads and
//! writes the part itself and nothing around it.
//!
//! # What survives a round trip
//!
//! Every field the crate models: the trace's points (at 1/1000 cm), pressure, the brush's kind,
//! width and height, colour, texture, texture phase and scale, and the draw order. Two private
//! brush properties carry what Office cannot express: `craftTool` (the pen kinds Office does not
//! distinguish) and `craftOrder` (draw order, for replay). Files GridCraft wrote call these
//! `gridcraftTool` and `gridcraftOrder`; both spellings are read, only the neutral one is written,
//! and Office drops unknown brush properties without refusing the part.
//!
//! An unknown brush property is kept in [`crate::stroke::Brush::extra`] and written back, so
//! another producer's data is not silently dropped. Unknown *elements* are refused
//! ([`Error::Unsupported`]): the alternative is to guess their meaning or lose them, and the spec
//! lets a reader reject markup it does not process. Unknown attributes are ignored.
//!
//! # Bounds
//!
//! [`read`] refuses more than [`Limits::max_bytes`] of input, more than [`Limits::max_strokes`]
//! traces or [`Limits::max_points`] points, and trace groups nested deeper than
//! [`Limits::max_depth`]. The XML reader ([`craft_xml`]) additionally bounds nesting, node and
//! attribute counts. Coordinates, brush dimensions, texture phase and pressure are range-checked,
//! arithmetic is checked or saturating, and nothing here panics on hostile input.

use craft_xml::{Canonical, Doctype, El, Options};
use thiserror::Error;

use crate::stroke::{
    Brush, Color, Effect, MAX_ANCHOR, MAX_BRUSH_CM, MAX_COORD_CM, MAX_PRESSURE, MAX_SCALE, MIN_BRUSH_CM, Stroke, StrokePoint, TRACE_UNITS_PER_CM,
    Tool,
};

/// The InkML namespace.
pub const NS: &str = "http://www.w3.org/2003/InkML";
/// Content type an InkML part is declared under in `[Content_Types].xml`.
pub const CONTENT_TYPE: &str = "application/inkml+xml";
/// Most channels one trace format may declare.
const MAX_CHANNELS: usize = 16;
/// Default channel resolution (values per centimetre) when a context does not set one.
const DEFAULT_RESOLUTION: f64 = 1000.0;
/// Most brush properties this crate preserves verbatim.
const MAX_EXTRA_PROPS: usize = 64;
/// Longest preserved brush property name and value.
const MAX_EXTRA_NAME: usize = 256;
const MAX_EXTRA_VALUE: usize = 4096;
/// Longest timestamp text read or written.
const MAX_TIMESTAMP: usize = 1024;

/// Limits for one InkML part. [`Limits::default`] is the set Office ink is read under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Limits {
    /// Most bytes of markup.
    pub max_bytes: usize,
    /// Most strokes (traces) in one part.
    pub max_strokes: usize,
    /// Most points in one part, over all its strokes.
    pub max_points: usize,
    /// Deepest `<traceGroup>` nesting.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_bytes: 16 * 1024 * 1024, max_strokes: 1024, max_points: 20_000, max_depth: 8 }
    }
}

/// An ink part: its strokes and the optional timestamp Office may write.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Ink {
    /// The strokes, in trace document order.
    pub strokes: Vec<Stroke>,
    /// The part's `<inkml:timestamp>`, when it has one. Office accepts a part without it, and a
    /// clock would make saves non-deterministic, so [`write()`] emits one only when asked.
    pub timestamp: Option<String>,
}

/// Why an ink part was not read or written.
#[derive(Clone, Debug, Error, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// The markup is longer than the byte limit.
    #[error("the InkML part is {size} bytes, over the {max}-byte limit")]
    TooLarge {
        /// The size read.
        size: usize,
        /// The configured limit.
        max: usize,
    },
    /// The XML could not be read.
    #[error("the InkML part does not parse: {0}")]
    Xml(#[from] craft_xml::Error),
    /// The root element is not `<inkml:ink>` in the InkML namespace.
    #[error("the part is not InkML: its root element is <{0}>")]
    NotInkml(String),
    /// More strokes than the limit allows.
    #[error("the part holds more than {max} strokes")]
    TooManyStrokes {
        /// The configured limit.
        max: usize,
    },
    /// More points than the limit allows.
    #[error("the ink holds more than {max} points")]
    TooManyPoints {
        /// The configured limit.
        max: usize,
    },
    /// Trace groups nest deeper than the limit allows.
    #[error("trace groups nest deeper than {max} levels")]
    TooDeep {
        /// The configured limit.
        max: usize,
    },
    /// An element outside the supported Office subset.
    #[error("the element <{0}> is not part of the supported ink subset")]
    Unsupported(String),
    /// A required element is absent.
    #[error("the InkML part has no {0} element")]
    Missing(&'static str),
    /// A `#id` reference names no definition.
    #[error("the reference `{0}` names no definition")]
    DanglingRef(String),
    /// A value the format cannot store.
    #[error("{what} has the unusable value `{value}`")]
    BadValue {
        /// What was being read.
        what: &'static str,
        /// The offending text.
        value: String,
    },
    /// A stroke the writer refuses; writing it would lose data.
    #[error("stroke {index} cannot be written: {reason}")]
    InvalidStroke {
        /// The stroke's index in [`Ink::strokes`].
        index: usize,
        /// Why it is not writable.
        reason: &'static str,
    },
}

/// Reads an InkML part.
///
/// # Errors
///
/// [`Error::TooLarge`], [`Error::Xml`], [`Error::NotInkml`], the limit errors, and the markup
/// errors ([`Error::Unsupported`], [`Error::Missing`], [`Error::DanglingRef`],
/// [`Error::BadValue`]). Nothing is guessed: a part that names a brush or context it does not
/// define is refused rather than drawn with a default.
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Ink, Error> {
    if bytes.len() > limits.max_bytes {
        return Err(Error::TooLarge { size: bytes.len(), max: limits.max_bytes });
    }
    let options = Options {
        doctype: Doctype::Reject,
        ..Options::canonical(&CANONICAL).strict().with_limits(craft_xml::Limits {
            max_input: limits.max_bytes,
            max_depth: limits.max_depth.saturating_add(16).max(16),
            ..craft_xml::Limits::default()
        })
    };
    let root = options.parse(bytes)?;
    if root.name != "inkml:ink" {
        return Err(Error::NotInkml(root.name));
    }
    only_children(&["inkml:definitions", "inkml:trace", "inkml:traceGroup", "inkml:timestamp", "inkml:annotation"], &root)?;
    let timestamp = root.child("inkml:timestamp").map(|e| e.text().into_owned());
    if let Some(element) = root.child("inkml:timestamp") {
        only_children(&[], element)?;
    }
    if let Some(text) = &timestamp
        && text.len() > MAX_TIMESTAMP
    {
        return Err(Error::BadValue { what: "timestamp", value: text.chars().take(32).collect() });
    }
    let has_traces = root.child("inkml:trace").is_some() || root.child("inkml:traceGroup").is_some();
    let Some(definitions) = root.child("inkml:definitions") else {
        if has_traces {
            return Err(Error::Missing("inkml:definitions"));
        }
        return Ok(Ink { strokes: Vec::new(), timestamp });
    };
    let definitions = Definitions::read(definitions)?;
    let mut traces = Vec::new();
    collect_traces(&root, None, None, 0, limits, &mut traces)?;
    let mut strokes = Vec::with_capacity(traces.len());
    let mut total = 0usize;
    for trace in traces {
        let context = definitions.context(trace.context)?;
        let (brush, color, order) = definitions.brush(trace.brush)?;
        let points = decode_trace(trace.element, context, limits.max_points.saturating_sub(total)).map_err(|error| match error {
            Error::TooManyPoints { .. } => Error::TooManyPoints { max: limits.max_points },
            other => other,
        })?;
        total = total.checked_add(points.len()).ok_or(Error::TooManyPoints { max: limits.max_points })?;
        if total > limits.max_points {
            return Err(Error::TooManyPoints { max: limits.max_points });
        }
        strokes.push(Stroke { points, brush, color, order, creation_id: None });
    }
    Ok(Ink { strokes, timestamp })
}

/// Writes an InkML part.
///
/// # Errors
///
/// The limit errors when the ink is too large, and [`Error::InvalidStroke`] when a stroke holds
/// values the format cannot store (no points, out-of-range coordinates or brush dimensions, a
/// pressure on only some points, an extra property shadowing one this crate reads). Clamping such
/// a stroke silently would make a save lossy.
pub fn write(ink: &Ink, limits: &Limits) -> Result<String, Error> {
    if ink.strokes.len() > limits.max_strokes {
        return Err(Error::TooManyStrokes { max: limits.max_strokes });
    }
    let mut total = 0usize;
    for (index, stroke) in ink.strokes.iter().enumerate() {
        if let Some(reason) = stroke.invalid_reason() {
            return Err(Error::InvalidStroke { index, reason });
        }
        total = total.checked_add(stroke.points.len()).ok_or(Error::TooManyPoints { max: limits.max_points })?;
    }
    if total > limits.max_points {
        return Err(Error::TooManyPoints { max: limits.max_points });
    }
    if let Some(text) = &ink.timestamp
        && text.len() > MAX_TIMESTAMP
    {
        return Err(Error::BadValue { what: "timestamp", value: text.chars().take(32).collect() });
    }

    let mut out = String::with_capacity(512 + total * 12);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    out.push_str("<inkml:ink xmlns:inkml=\"");
    out.push_str(NS);
    out.push('"');
    out.push('>');
    if let Some(text) = &ink.timestamp {
        out.push_str("<inkml:timestamp>");
        out.push_str(&craft_xml::esc(text));
        out.push_str("</inkml:timestamp>");
    }
    if !ink.strokes.is_empty() {
        out.push_str("<inkml:definitions>");
        // One context per channel set: a trace with pressure needs the F channel, one without
        // must not gain a zero-pressure channel it would read back as recorded.
        out.push_str(&context_xml("ctx0", false));
        if ink.strokes.iter().any(Stroke::has_pressure) {
            out.push_str(&context_xml("ctx1", true));
        }
        for (i, stroke) in ink.strokes.iter().enumerate() {
            out.push_str(&brush_xml(&format!("br{i}"), stroke));
        }
        out.push_str("</inkml:definitions>");
        for (i, stroke) in ink.strokes.iter().enumerate() {
            let context = if stroke.has_pressure() { "ctx1" } else { "ctx0" };
            let trace = encode_trace(stroke);
            out.push_str("<inkml:trace contextRef=\"#");
            out.push_str(context);
            out.push_str("\" brushRef=\"#br");
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{i}\">"));
            out.push_str(&trace);
            out.push_str("</inkml:trace>");
        }
    }
    out.push_str("</inkml:ink>");
    Ok(out)
}

/// Canonical prefixes: every InkML element reads as `inkml:<local>`, whatever prefix the file
/// used, and an element in another namespace never matches.
static NAMESPACES: [(&str, &str); 1] = [("inkml", NS)];
const CANONICAL: Canonical<'static> = Canonical::new(&NAMESPACES);

/// The definitions of a part: its contexts and brushes.
struct Definitions {
    contexts: Vec<Context>,
    brushes: Vec<BrushDef>,
}

/// One context's trace format.
struct Context {
    id: Option<String>,
    channels: Vec<ChannelDef>,
    x: usize,
    y: usize,
    f: Option<usize>,
}

/// One channel of a trace format.
struct ChannelDef {
    name: String,
    resolution: f64,
}

/// One brush element and the stroke data its properties carry.
struct BrushDef {
    id: Option<String>,
    brush: Brush,
    color: Color,
    order: u32,
}

/// A trace with the references in force at it, inherited from its groups.
struct TraceRef<'a> {
    element: &'a El,
    context: Option<&'a str>,
    brush: Option<&'a str>,
}

impl Definitions {
    fn read(el: &El) -> Result<Definitions, Error> {
        only_children(&["inkml:context", "inkml:brush"], el)?;
        let contexts = el.children("inkml:context").map(read_context).collect::<Result<Vec<_>, _>>()?;
        let brushes = el.children("inkml:brush").map(read_brush).collect::<Result<Vec<_>, _>>()?;
        Ok(Definitions { contexts, brushes })
    }

    fn context(&self, reference: Option<&str>) -> Result<&Context, Error> {
        match reference {
            Some(reference) => {
                let id = reference_id(reference)?;
                self.contexts.iter().find(|c| c.id.as_deref() == Some(id)).ok_or_else(|| Error::DanglingRef(reference.to_owned()))
            }
            None => self.contexts.first().ok_or(Error::Missing("inkml:context")),
        }
    }

    fn brush(&self, reference: Option<&str>) -> Result<(Brush, Color, u32), Error> {
        let found = match reference {
            Some(reference) => {
                let id = reference_id(reference)?;
                self.brushes.iter().find(|b| b.id.as_deref() == Some(id)).ok_or_else(|| Error::DanglingRef(reference.to_owned()))?
            }
            None => self.brushes.first().ok_or(Error::Missing("inkml:brush"))?,
        };
        Ok((found.brush.clone(), found.color, found.order))
    }
}

/// The id a `#id` reference names.
fn reference_id(reference: &str) -> Result<&str, Error> {
    let trimmed = reference.trim();
    match trimmed.strip_prefix('#') {
        Some(id) if !id.is_empty() => Ok(id),
        _ => Err(Error::DanglingRef(reference.to_owned())),
    }
}

/// The element's id: `xml:id` as InkML defines it, or a plain `id` some writers use.
fn element_id(el: &El) -> Option<String> {
    el.attr("xml:id").or_else(|| el.attr("id")).map(str::to_owned)
}

/// Refuses any child element not named in `allowed`.
fn only_children(allowed: &[&str], el: &El) -> Result<(), Error> {
    for child in el.elements() {
        if !allowed.contains(&child.name.as_str()) {
            return Err(Error::Unsupported(child.name.clone()));
        }
    }
    Ok(())
}

fn read_context(el: &El) -> Result<Context, Error> {
    only_children(&["inkml:inkSource"], el)?;
    let source = el.child("inkml:inkSource").ok_or(Error::Missing("inkml:inkSource"))?;
    only_children(&["inkml:traceFormat", "inkml:channelProperties"], source)?;
    let format = source.child("inkml:traceFormat").ok_or(Error::Missing("inkml:traceFormat"))?;
    only_children(&["inkml:channel"], format)?;
    let properties = source.child("inkml:channelProperties");
    if let Some(properties) = properties {
        only_children(&["inkml:channelProperty"], properties)?;
    }
    let mut channels: Vec<ChannelDef> = Vec::new();
    for channel in format.children("inkml:channel") {
        let name = channel.attr("name").ok_or(Error::BadValue { what: "channel", value: String::new() })?;
        let kind = channel.attr("type").unwrap_or("integer");
        if kind != "integer" {
            return Err(Error::BadValue { what: "channel type", value: kind.to_owned() });
        }
        if channels.iter().any(|c| c.name == name) {
            return Err(Error::BadValue { what: "duplicate channel", value: name.to_owned() });
        }
        let resolution = properties
            .and_then(|p| {
                p.children("inkml:channelProperty")
                    .find(|c| c.attr("channel") == Some(name) && c.attr("name") == Some("resolution"))
                    .and_then(|c| c.attr("value"))
            })
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0 && *v <= 1e9)
            .unwrap_or(DEFAULT_RESOLUTION);
        channels.push(ChannelDef { name: name.to_owned(), resolution });
    }
    if channels.is_empty() || channels.len() > MAX_CHANNELS {
        return Err(Error::BadValue { what: "channel count", value: channels.len().to_string() });
    }
    let position = |name: &str| channels.iter().position(|c| c.name == name);
    let (x, y) = (position("X").ok_or(Error::Missing("X channel"))?, position("Y").ok_or(Error::Missing("Y channel"))?);
    let f = position("F");
    Ok(Context { id: element_id(el), channels, x, y, f })
}

fn read_brush(el: &El) -> Result<BrushDef, Error> {
    only_children(&["inkml:brushProperty"], el)?;
    let mut brush = Brush::default();
    let mut color = Color::BLACK;
    let mut order = 0u32;
    let mut explicit_tool: Option<Tool> = None;
    let mut pencil = false;
    let mut mask_pen = false;
    for property in el.children("inkml:brushProperty") {
        let name = property.attr("name").ok_or(Error::BadValue { what: "brush property", value: String::new() })?;
        let value = property.attr("value").ok_or(Error::BadValue { what: "brush property value", value: name.to_owned() })?;
        match name {
            "width" => brush.width_cm = brush_cm(value)?,
            "height" => brush.height_cm = brush_cm(value)?,
            "color" => color = Color::from_hex(value).ok_or_else(|| Error::BadValue { what: "brush colour", value: value.to_owned() })?,
            "tip" => mask_pen |= value.trim() == "rectangle",
            "rasterOp" => mask_pen |= value.trim() == "maskPen",
            "inkEffects" => {
                let name = value.trim().to_ascii_lowercase();
                if name == "pencil" {
                    pencil = true;
                } else {
                    brush.effect = Some(Effect::from_name(&name).ok_or(Error::BadValue { what: "inkEffects", value: value.to_owned() })?);
                }
            }
            "ignorePressure" => {}
            "anchorX" => brush.anchor[0] = finite(value).map_or(0.0, |v| v.clamp(-MAX_ANCHOR, MAX_ANCHOR)),
            "anchorY" => brush.anchor[1] = finite(value).map_or(0.0, |v| v.clamp(-MAX_ANCHOR, MAX_ANCHOR)),
            "scaleFactor" => brush.scale = finite(value).filter(|v| *v > 0.0).map_or(1.0, |v| v.clamp(f32::MIN_POSITIVE, MAX_SCALE)),
            "craftOrder" | "gridcraftOrder" => order = value.trim().parse::<u32>().unwrap_or(0),
            "craftTool" | "gridcraftTool" => {
                explicit_tool = Some(Tool::from_name(value).unwrap_or(Tool::Pen));
            }
            other => {
                if brush.extra.len() == MAX_EXTRA_PROPS || other.len() > MAX_EXTRA_NAME || value.len() > MAX_EXTRA_VALUE {
                    return Err(Error::BadValue { what: "brush property", value: other.to_owned() });
                }
                brush.extra.push((other.to_owned(), value.to_owned()));
            }
        }
    }
    if brush.height_cm <= 0.0 {
        brush.height_cm = brush.width_cm;
    }
    brush.tool = match explicit_tool {
        Some(tool) => tool,
        None if pencil => Tool::Pencil,
        None if mask_pen => Tool::Highlighter,
        None => Tool::Pen,
    };
    Ok(BrushDef { id: element_id(el), brush, color, order })
}

/// A brush dimension in cm, clamped to the range the format stores.
fn brush_cm(value: &str) -> Result<f32, Error> {
    finite(value).map(|v| v.clamp(MIN_BRUSH_CM, MAX_BRUSH_CM)).ok_or_else(|| Error::BadValue { what: "brush size", value: value.to_owned() })
}

/// The first finite `f32` in `value`, ignoring surrounding whitespace.
fn finite(value: &str) -> Option<f32> {
    value.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Collects every trace below `el` in document order, with the references inherited from its
/// groups.
fn collect_traces<'a>(
    el: &'a El,
    context: Option<&'a str>,
    brush: Option<&'a str>,
    depth: usize,
    limits: &Limits,
    out: &mut Vec<TraceRef<'a>>,
) -> Result<(), Error> {
    if depth > limits.max_depth {
        return Err(Error::TooDeep { max: limits.max_depth });
    }
    for child in el.elements() {
        match child.name.as_str() {
            "inkml:trace" => {
                out.push(TraceRef { element: child, context: child.attr("contextRef").or(context), brush: child.attr("brushRef").or(brush) });
                if out.len() > limits.max_strokes {
                    return Err(Error::TooManyStrokes { max: limits.max_strokes });
                }
            }
            "inkml:traceGroup" => {
                collect_traces(child, child.attr("contextRef").or(context), child.attr("brushRef").or(brush), depth.saturating_add(1), limits, out)?
            }
            "inkml:annotation" | "inkml:definitions" | "inkml:timestamp" => {}
            other => return Err(Error::Unsupported(other.to_owned())),
        }
    }
    Ok(())
}

/// One channel's difference-decoding state: its last value, its last first difference, and the
/// difference order written for that value. A `*` stores nothing here: it repeats the value and
/// changes no order.
#[derive(Clone, Copy, Default)]
struct Channel {
    value: i64,
    diff: i64,
    prefix: Option<char>,
}

/// The decoded points of a trace.
fn decode_trace(el: &El, context: &Context, max_points: usize) -> Result<Vec<StrokePoint>, Error> {
    only_children(&[], el)?;
    let mut state = vec![Channel::default(); context.channels.len()];
    let mut points: Vec<StrokePoint> = Vec::new();
    let text = el.text();
    for point in text.split(',') {
        if point.trim().is_empty() {
            continue;
        }
        if points.len() == max_points {
            return Err(Error::TooManyPoints { max: max_points });
        }
        let mut rest = point;
        let mut values: Vec<i64> = Vec::with_capacity(context.channels.len());
        for channel in state.iter_mut() {
            let (written, raw, tail) = scan_value(rest).ok_or_else(|| Error::BadValue { what: "trace value", value: point.trim().to_owned() })?;
            rest = tail;
            if written == Some('*') {
                // `*` repeats the value and leaves the channel's difference order as it was, so
                // the first difference is zero from here on.
                channel.diff = 0;
                if channel.value.unsigned_abs() > i32::MAX as u64 {
                    return Err(Error::BadValue { what: "trace value", value: point.trim().to_owned() });
                }
                values.push(channel.value);
                continue;
            }
            // A value with no prefix of its own carries the channel's previous difference order.
            let prefix = written.or(channel.prefix).unwrap_or('!');
            let raw = raw.ok_or_else(|| Error::BadValue { what: "trace value", value: point.trim().to_owned() })?;
            let value = match prefix {
                '!' => Some(raw),
                '\'' => channel.value.checked_add(raw),
                '"' => channel.diff.checked_add(raw).and_then(|d| channel.value.checked_add(d)),
                _ => None,
            }
            .ok_or_else(|| Error::BadValue { what: "trace value", value: point.trim().to_owned() })?;
            if value.unsigned_abs() > i32::MAX as u64 {
                return Err(Error::BadValue { what: "trace value", value: point.trim().to_owned() });
            }
            let diff = match prefix {
                '!' => {
                    let d =
                        value.checked_sub(channel.value).ok_or_else(|| Error::BadValue { what: "trace value", value: point.trim().to_owned() })?;
                    if channel.prefix.is_none() { 0 } else { d }
                }
                '\'' => raw,
                '"' => channel.diff.checked_add(raw).ok_or_else(|| Error::BadValue { what: "trace value", value: point.trim().to_owned() })?,
                _ => channel.diff,
            };
            *channel = Channel { value, diff, prefix: Some(prefix) };
            values.push(value);
        }
        if !rest.bytes().all(is_wsp) {
            return Err(Error::BadValue { what: "trace point", value: point.trim().to_owned() });
        }
        let coordinate = |index: usize| -> Result<f64, Error> {
            let value = values.get(index).copied().unwrap_or(0) as f64;
            let resolution = context.channels.get(index).map_or(DEFAULT_RESOLUTION, |c| c.resolution);
            let cm = value / resolution;
            if !cm.is_finite() || cm.abs() > MAX_COORD_CM {
                return Err(Error::BadValue { what: "trace coordinate", value: point.trim().to_owned() });
            }
            Ok(cm)
        };
        let (x, y) = (coordinate(context.x)?, coordinate(context.y)?);
        let pressure = context.f.map(|index| values.get(index).copied().unwrap_or(0).clamp(0, i64::from(MAX_PRESSURE)) as u16);
        points.push(StrokePoint { x, y, pressure });
    }
    if points.is_empty() {
        return Err(Error::Missing("trace point"));
    }
    Ok(points)
}

/// An ASCII whitespace byte, as the trace grammar allows it between values.
fn is_wsp(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

/// Scans one value of a trace point as InkML allows it: an optional difference-order prefix
/// (`!`, `'`, `"` or `*`), optional whitespace, an optional minus sign and digits
/// (`value ::= difference_order? wsp* "-"? wsp* integer`). Whitespace before a value is optional,
/// so Excel's compact `'2117'529` holds two values; a prefix, a sign or whitespace separates them.
/// Returns the prefix (when written), its number (`None` for `*`) and the rest of the point.
fn scan_value(s: &str) -> Option<(Option<char>, Option<i64>, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_wsp(bytes[i]) {
        i += 1;
    }
    let prefix = match bytes.get(i) {
        Some(b @ (b'!' | b'\'' | b'"' | b'*')) => {
            i += 1;
            Some(*b as char)
        }
        _ => None,
    };
    while i < bytes.len() && is_wsp(bytes[i]) {
        i += 1;
    }
    let rest = s.get(i..)?;
    if prefix == Some('*') {
        return Some((prefix, None, rest));
    }
    let b = rest.as_bytes();
    let negative = b.first() == Some(&b'-');
    let mut j = usize::from(negative);
    while j < b.len() && is_wsp(b[j]) {
        j += 1;
    }
    let digits = j;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    if j == digits {
        return None;
    }
    // Parse the sign with the digits (keeps i64::MIN representable); whitespace between them,
    // which Excel never writes, needs a tiny copy.
    let number = if digits == usize::from(negative) {
        rest.get(..j)?.parse::<i64>().ok()?
    } else {
        let mut text = String::with_capacity(j - digits + 1);
        if negative {
            text.push('-');
        }
        text.push_str(rest.get(digits..j)?);
        text.parse::<i64>().ok()?
    };
    Some((prefix, Some(number), rest.get(j..)?))
}

/// One context element: its trace format and channel properties.
fn context_xml(id: &str, with_pressure: bool) -> String {
    let mut out = String::with_capacity(900);
    out.push_str("<inkml:context xml:id=\"");
    out.push_str(id);
    out.push_str("\"><inkml:inkSource xml:id=\"src-");
    out.push_str(id);
    out.push_str("\"><inkml:traceFormat>");
    out.push_str(
        "<inkml:channel name=\"X\" type=\"integer\" min=\"-2.14748E9\" max=\"2.14748E9\" units=\"cm\"/><inkml:channel name=\"Y\" type=\"integer\" min=\"-2.14748E9\" max=\"2.14748E9\" units=\"cm\"/>",
    );
    if with_pressure {
        out.push_str("<inkml:channel name=\"F\" type=\"integer\" min=\"0\" max=\"32767\" units=\"dev\"/>");
    }
    out.push_str("</inkml:traceFormat><inkml:channelProperties>");
    out.push_str(
        "<inkml:channelProperty channel=\"X\" name=\"resolution\" value=\"1000\" units=\"1/cm\"/><inkml:channelProperty channel=\"Y\" name=\"resolution\" value=\"1000\" units=\"1/cm\"/>",
    );
    if with_pressure {
        out.push_str("<inkml:channelProperty channel=\"F\" name=\"resolution\" value=\"0\" units=\"1/dev\"/>");
    }
    out.push_str("</inkml:channelProperties></inkml:inkSource></inkml:context>");
    out
}

/// One brush element for a stroke: its dimensions and colour, the Office markers for the kinds
/// Office can draw, the texture and phase, and the two neutral properties Office cannot express.
fn brush_xml(id: &str, stroke: &Stroke) -> String {
    let brush = &stroke.brush;
    let mut out = String::with_capacity(400);
    out.push_str("<inkml:brush xml:id=\"");
    out.push_str(id);
    out.push_str("\"><inkml:brushProperty name=\"width\" value=\"");
    out.push_str(&brush.width_cm.to_string());
    out.push_str("\" units=\"cm\"/><inkml:brushProperty name=\"height\" value=\"");
    out.push_str(&brush.height_cm.to_string());
    out.push_str("\" units=\"cm\"/><inkml:brushProperty name=\"color\" value=\"");
    out.push_str(&stroke.color.to_hex());
    out.push_str("\"/>");
    match brush.tool {
        Tool::Pencil => out.push_str("<inkml:brushProperty name=\"ignorePressure\" value=\"1\"/>"),
        Tool::Highlighter => out.push_str(
            "<inkml:brushProperty name=\"tip\" value=\"rectangle\"/><inkml:brushProperty name=\"rasterOp\" value=\"maskPen\"/><inkml:brushProperty name=\"ignorePressure\" value=\"1\"/>",
        ),
        Tool::Pen | Tool::FountainPen | Tool::BrushPen => {}
    }
    if brush.tool == Tool::Pencil && brush.effect.is_none() {
        out.push_str("<inkml:brushProperty name=\"inkEffects\" value=\"pencil\"/>");
    }
    if let Some(effect) = brush.effect {
        out.push_str("<inkml:brushProperty name=\"inkEffects\" value=\"");
        out.push_str(effect.inkml_name());
        out.push_str("\"/>");
    }
    let phase = brush.anchor != [0.0, 0.0] || (brush.scale - 1.0).abs() > f32::EPSILON;
    if brush.effect.is_some() || phase {
        out.push_str("<inkml:brushProperty name=\"anchorX\" value=\"");
        out.push_str(&brush.anchor[0].to_string());
        out.push_str("\"/><inkml:brushProperty name=\"anchorY\" value=\"");
        out.push_str(&brush.anchor[1].to_string());
        out.push_str("\"/><inkml:brushProperty name=\"scaleFactor\" value=\"");
        out.push_str(&brush.scale.to_string());
        out.push_str("\"/>");
    }
    let tool = match brush.tool {
        Tool::FountainPen => Some("fountainPen"),
        Tool::BrushPen => Some("brushPen"),
        Tool::Pencil if brush.effect.is_some() => Some("pencil"),
        Tool::Pen | Tool::Pencil | Tool::Highlighter => None,
    };
    if let Some(tool) = tool {
        out.push_str("<inkml:brushProperty name=\"craftTool\" value=\"");
        out.push_str(tool);
        out.push_str("\"/>");
    }
    out.push_str("<inkml:brushProperty name=\"craftOrder\" value=\"");
    out.push_str(&stroke.order.to_string());
    out.push_str("\"/>");
    for (name, value) in &brush.extra {
        out.push_str("<inkml:brushProperty name=\"");
        out.push_str(&craft_xml::esc_attr(name));
        out.push_str("\" value=\"");
        out.push_str(&craft_xml::esc_attr(value));
        out.push_str("\"/>");
    }
    out.push_str("</inkml:brush>");
    out
}

/// The trace text: the W3C difference encoding. The first point is explicit, the second holds
/// its first differences, every later point its second differences (prefixed once per channel,
/// then inherited).
fn encode_trace(stroke: &Stroke) -> String {
    let pressure: Vec<i64> = stroke.points.iter().map(|p| i64::from(p.pressure.unwrap_or(0))).collect();
    let has_pressure = stroke.has_pressure();
    let channels = 2 + usize::from(has_pressure);
    let points: Vec<[i64; 2]> = stroke
        .points
        .iter()
        .map(|point| {
            let unit = |cm: f64| (cm * TRACE_UNITS_PER_CM).round() as i64;
            [unit(point.x), unit(point.y)]
        })
        .collect();
    let value = |i: usize, c: usize| -> i64 { if c < 2 { points.get(i).map_or(0, |q| q[c]) } else { pressure.get(i).copied().unwrap_or(0) } };
    let mut out = String::with_capacity(points.len() * channels * 5);
    for (i, _) in points.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        for c in 0..channels {
            if c > 0 {
                out.push(' ');
            }
            use std::fmt::Write as _;
            let _ = match i {
                0 => write!(out, "!{}", value(i, c)),
                1 => write!(out, "'{}", value(i, c).saturating_sub(value(0, c))),
                2 => {
                    let second = second_difference(&points, &pressure, c, 2);
                    write!(out, "\"{second}")
                }
                _ => {
                    let second = second_difference(&points, &pressure, c, i);
                    write!(out, "{second}")
                }
            };
        }
    }
    out
}

/// The second difference at index `i`: `(v[i] - v[i-1]) - (v[i-1] - v[i-2])`, saturating so a
/// point at the cap cannot wrap.
fn second_difference(points: &[[i64; 2]], pressure: &[i64], channel: usize, i: usize) -> i64 {
    let value = |k: usize| -> i64 { if channel < 2 { points.get(k).map_or(0, |q| q[channel]) } else { pressure.get(k).copied().unwrap_or(0) } };
    let prev = i.saturating_sub(1);
    let prev2 = i.saturating_sub(2);
    value(i).saturating_sub(value(prev)).saturating_sub(value(prev).saturating_sub(value(prev2)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_difference_grammar_scans_compact_and_spaced_values() {
        assert_eq!(scan_value("!12"), Some((Some('!'), Some(12), "")));
        assert_eq!(scan_value("'2117'529"), Some((Some('\''), Some(2117), "'529")));
        assert_eq!(scan_value(" - 42 rest"), Some((None, Some(-42), " rest")));
        assert_eq!(scan_value("*"), Some((Some('*'), None, "")));
        assert_eq!(scan_value(""), None);
        assert_eq!(scan_value("x"), None);
        assert_eq!(scan_value("!-"), None);
    }

    #[test]
    fn canonical_names_keep_the_inkml_namespace() {
        let doc = craft_xml::Options::canonical(&CANONICAL)
            .parse(br#"<i:ink xmlns:i="http://www.w3.org/2003/InkML"><i:definitions/></i:ink>"#)
            .expect("plain inkml");
        assert_eq!(doc.name, "inkml:ink");
        assert_eq!(doc.child("inkml:definitions").map(|e| e.name.as_str()), Some("inkml:definitions"));
    }
}
