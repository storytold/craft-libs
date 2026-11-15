//! A bounded SVG document walk that preserves geometric primitive kinds for CAD import.
//!
//! Coordinates remain in SVG user units. Each shape carries the composed transform into
//! millimetres with the page Y axis pointing up; consumers map shapes into their own model.
use crate::path_data::{self, Point, Subpath};
use quick_xml::events::{BytesStart, Event as XmlEvent};
use quick_xml::{Reader, XmlVersion};
use std::collections::BTreeMap;

const MAX_DEPTH: usize = 64;
const MAX_SHAPES: usize = 500_000;
const MAX_POINTS: usize = 8_000_000;
const MAX_POLY_POINTS: usize = 100_001;
const SKIP: &[&[u8]] = &[
    b"defs",
    b"symbol",
    b"clipPath",
    b"mask",
    b"pattern",
    b"marker",
    b"linearGradient",
    b"radialGradient",
    b"filter",
    b"style",
    b"text",
    b"image",
    b"title",
    b"desc",
    b"metadata",
];
const CONTAINERS: &[&[u8]] = &[b"g", b"svg", b"a", b"switch"];

/// Input failure or resource bound.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);
/// A shape's original geometry before its affine transform.
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    Rect { x: f64, y: f64, width: f64, height: f64 },
    Circle { center: Point, radius: f64 },
    Ellipse { center: Point, rx: f64, ry: f64 },
    Line { a: Point, b: Point },
    Polyline { points: Vec<Point>, closed: bool },
    Path(Vec<Subpath>),
}
/// A primitive with its layer and SVG affine matrix `[a,b,c,d,e,f]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub primitive: Primitive,
    pub transform: [f64; 6],
    pub layer: String,
}
/// The shapes, encountered layer names, and skipped-feature counts. `visit` streams shapes
/// to its sink, so its returned `shapes` is empty; `import` collects them.
#[derive(Debug, Default, PartialEq)]
pub struct Document {
    pub shapes: Vec<Shape>,
    pub layers: Vec<String>,
    pub skipped: BTreeMap<Skipped, usize>,
}
/// Features left out by this geometry-only import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Skipped {
    Text,
    Image,
    Use,
    Style,
    RoundedCorners,
}

fn attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    for a in e.attributes() {
        let Ok(a) = a else { return None };
        if a.key.into_inner() == name {
            return a.normalized_value(XmlVersion::Explicit1_0).ok().map(|v| v.into_owned());
        }
    }
    None
}
fn hidden(e: &BytesStart<'_>) -> bool {
    attr(e, b"display").as_deref() == Some("none") || attr(e, b"style").is_some_and(|s| s.replace(' ', "").contains("display:none"))
}
fn length(s: &str) -> Option<f64> {
    let s = s.trim();
    let cut = s.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(s.len());
    let (n, unit) = s.split_at(cut);
    let v: f64 = n.trim().parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(
        v * match unit.trim() {
            "" | "px" | "mm" => 1.0,
            "cm" => 10.0,
            "in" => 25.4,
            "pt" => 25.4 / 72.0,
            "pc" => 25.4 / 6.0,
            _ => 25.4 / 96.0,
        },
    )
}
fn num(e: &BytesStart<'_>, name: &[u8], default: f64) -> f64 {
    attr(e, name).as_deref().and_then(length).unwrap_or(default)
}
const ID: [f64; 6] = [1., 0., 0., 1., 0., 0.];
fn mul(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}
fn translate(x: f64, y: f64) -> [f64; 6] {
    [1., 0., 0., 1., x, y]
}
fn scale(x: f64, y: f64) -> [f64; 6] {
    [x, 0., 0., y, 0., 0.]
}
fn rotate(a: f64) -> [f64; 6] {
    let (s, c) = a.to_radians().sin_cos();
    [c, s, -s, c, 0., 0.]
}
fn numbers(s: &str) -> impl Iterator<Item = f64> + '_ {
    path_data::Numbers::new(s)
}
fn transform(s: &str) -> Option<[f64; 6]> {
    let mut out = None;
    let mut rest = s.trim();
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let name = rest[..open].trim();
        let close = rest[open..].find(')')? + open;
        // No transform function accepts more than six arguments.
        let args: Vec<_> = numbers(&rest[open + 1..close]).take(7).collect();
        let m = match (name, args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => [*a, *b, *c, *d, *e, *f],
            ("translate", [x]) => translate(*x, 0.),
            ("translate", [x, y]) => translate(*x, *y),
            ("scale", [x]) => scale(*x, *x),
            ("scale", [x, y]) => scale(*x, *y),
            ("rotate", [a]) => rotate(*a),
            ("rotate", [a, x, y]) => mul(mul(translate(*x, *y), rotate(*a)), translate(-x, -y)),
            ("skewX", [a]) => [1., 0., a.to_radians().tan(), 1., 0., 0.],
            ("skewY", [a]) => [1., a.to_radians().tan(), 0., 1., 0., 0.],
            _ => return None,
        };
        out = Some(out.map_or(m, |prev| mul(prev, m)));
        rest = rest[close + 1..].trim_start_matches(|c: char| c == ',' || c.is_whitespace());
    }
    out
}
fn viewport(e: &BytesStart<'_>) -> [f64; 6] {
    let vb = attr(e, b"viewBox").and_then(|raw| {
        let n: Vec<_> = numbers(&raw).take(5).collect();
        match n.as_slice() {
            [x, y, w, h] if *w > 0. && *h > 0. => Some((*x, *y, *w, *h)),
            _ => None,
        }
    });
    let root = |name: &[u8]| {
        let raw = attr(e, name)?;
        let mm = length(&raw)?;
        let unit = raw.trim().trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')).trim();
        Some(if matches!(unit, "" | "px") { mm * 25.4 / 96. } else { mm })
    };
    let unit = match (vb, root(b"width"), root(b"height")) {
        (Some((_, _, w, _)), Some(x), _) if x > 0. => x / w,
        (Some((_, _, _, h)), _, Some(y)) if y > 0. => y / h,
        _ => 25.4 / 96.,
    };
    let (x, y, h) = vb.map_or((0., 0., 0.), |(x, y, _, h)| (x, y, h));
    [unit, 0., 0., -unit, -x * unit, (y + h) * unit]
}
fn layer_name(raw: &str) -> String {
    let s: String = raw.trim().chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '$') { c } else { '_' }).take(128).collect();
    let s = s.trim_matches('_');
    if s.is_empty() { "SVG".into() } else { s.into() }
}
/// A layer declaration or a drawable primitive in document order.
pub enum Event {
    Layer(String),
    Shape(Shape),
}

struct Walker<'a> {
    doc: Document,
    stack: Vec<[f64; 6]>,
    layer: String,
    skip: usize,
    points: usize,
    shapes: usize,
    sink: &'a mut dyn FnMut(Event) -> Result<(), Error>,
}
impl Walker<'_> {
    fn current(&self) -> [f64; 6] {
        self.stack.last().copied().unwrap_or(ID)
    }
    fn note(&mut self, what: Skipped) {
        *self.doc.skipped.entry(what).or_default() += 1;
    }
    fn skipped(name: &[u8]) -> Option<Skipped> {
        match name {
            b"text" => Some(Skipped::Text),
            b"image" => Some(Skipped::Image),
            _ => None,
        }
    }
    fn start(&mut self, name: &[u8], e: &BytesStart<'_>) -> Result<(), Error> {
        if name == b"style" {
            self.note(Skipped::Style);
        }
        if self.skip > 0 {
            self.skip += 1;
            return Ok(());
        }
        if hidden(e) {
            self.skip = 1;
            return Ok(());
        }
        if SKIP.contains(&name) {
            if let Some(s) = Self::skipped(name) {
                self.note(s);
            }
            self.skip = 1;
            return Ok(());
        }
        if !CONTAINERS.contains(&name) {
            return self.element(name, e);
        }
        if name == b"svg"
            && self.stack.len() == 1
            && let Some(root) = self.stack.first_mut()
        {
            *root = viewport(e);
        }
        if name == b"g"
            && attr(e, b"inkscape:groupmode").as_deref() == Some("layer")
            && let Some(raw) = attr(e, b"inkscape:label").or_else(|| attr(e, b"id"))
        {
            let layer = layer_name(&raw);
            if layer == "0" || self.doc.layers.contains(&layer) {
                self.layer = layer;
            } else if self.doc.layers.len() < 255 {
                self.doc.layers.push(layer.clone());
                (self.sink)(Event::Layer(layer.clone()))?;
                self.layer = layer;
            }
        }
        if self.stack.len() >= MAX_DEPTH {
            return Err(Error(format!("SVG: more than {MAX_DEPTH} nested groups")));
        }
        let next = attr(e, b"transform").as_deref().and_then(transform).map_or(self.current(), |t| mul(self.current(), t));
        self.stack.push(next);
        Ok(())
    }
    fn element(&mut self, name: &[u8], e: &BytesStart<'_>) -> Result<(), Error> {
        if self.skip > 0 {
            return Ok(());
        }
        if SKIP.contains(&name) || hidden(e) {
            if !hidden(e)
                && let Some(s) = Self::skipped(name)
            {
                self.note(s);
            }
            return Ok(());
        }
        let p = |x, y| Point::new(num(e, x, 0.), num(e, y, 0.));
        let primitive = match name {
            b"use" => {
                self.note(Skipped::Use);
                return Ok(());
            }
            b"rect" => {
                let (x, y, w, h) = (num(e, b"x", 0.), num(e, b"y", 0.), num(e, b"width", 0.), num(e, b"height", 0.));
                if w <= 0. || h <= 0. {
                    return Ok(());
                }
                if num(e, b"rx", 0.) > 0. || num(e, b"ry", 0.) > 0. {
                    self.note(Skipped::RoundedCorners);
                }
                Primitive::Rect { x, y, width: w, height: h }
            }
            b"circle" => {
                let r = num(e, b"r", 0.);
                if r <= 0. {
                    return Ok(());
                }
                Primitive::Circle { center: p(b"cx", b"cy"), radius: r }
            }
            b"ellipse" => {
                let (rx, ry) = (num(e, b"rx", 0.), num(e, b"ry", 0.));
                if rx <= 0. || ry <= 0. {
                    return Ok(());
                }
                Primitive::Ellipse { center: p(b"cx", b"cy"), rx, ry }
            }
            b"line" => Primitive::Line { a: p(b"x1", b"y1"), b: p(b"x2", b"y2") },
            b"polyline" | b"polygon" => {
                let Some(raw) = attr(e, b"points") else { return Ok(()) };
                let n: Vec<_> = numbers(&raw).take(MAX_POLY_POINTS * 2).collect();
                let pts: Vec<_> = n.as_chunks::<2>().0.iter().map(|[x, y]| Point::new(*x, *y)).collect();
                if pts.len() < 2 {
                    return Ok(());
                }
                Primitive::Polyline { points: pts, closed: name == b"polygon" }
            }
            b"path" => {
                let Some(raw) = attr(e, b"d") else { return Ok(()) };
                Primitive::Path(path_data::parse_path(&raw).map_err(|e| Error(e.to_string()))?)
            }
            _ => return Ok(()),
        };
        let cost = match &primitive {
            Primitive::Polyline { points, .. } => points.len(),
            Primitive::Path(paths) => paths.iter().map(|p| p.segs.len()).sum(),
            _ => 1,
        };
        self.points = self.points.checked_add(cost).filter(|n| *n <= MAX_POINTS).ok_or_else(|| Error("SVG: too much generated geometry".into()))?;
        if self.shapes >= MAX_SHAPES {
            return Err(Error(format!("SVG: more than {MAX_SHAPES} elements")));
        }
        let matrix = attr(e, b"transform").as_deref().and_then(transform).map_or(self.current(), |t| mul(self.current(), t));
        self.shapes += 1;
        (self.sink)(Event::Shape(Shape { primitive, transform: matrix, layer: self.layer.clone() }))
    }
}
/// Walk supported SVG geometry without retaining a second copy of the document's shapes.
/// The sink receives each layer before its first shape and each shape in input order.
pub fn visit(text: &str, sink: &mut dyn FnMut(Event) -> Result<(), Error>) -> Result<Document, Error> {
    let mut walker = Walker { doc: Document::default(), stack: vec![ID], layer: "0".into(), skip: 0, points: 0, shapes: 0, sink };
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event().map_err(|e| Error(format!("SVG: {e}")))? {
            XmlEvent::Start(e) => walker.start(e.name().into_inner(), &e)?,
            XmlEvent::Empty(e) => walker.element(e.name().into_inner(), &e)?,
            XmlEvent::End(e) => {
                if walker.skip > 0 {
                    walker.skip -= 1;
                } else if CONTAINERS.contains(&e.name().into_inner()) && walker.stack.len() > 1 {
                    walker.stack.pop();
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(walker.doc)
}
/// Read the supported geometry into an owned document.
pub fn import(text: &str) -> Result<Document, Error> {
    let mut shapes = Vec::new();
    let mut doc = visit(text, &mut |event| {
        if let Event::Shape(shape) = event {
            shapes.push(shape);
        }
        Ok(())
    })?;
    doc.shapes = shapes;
    Ok(doc)
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn keeps_primitives_transform_layers_and_skips() {
        let svg = r##"<svg width="20mm" height="10mm" viewBox="0 0 20 10"><g inkscape:groupmode="layer" inkscape:label="cut" transform="translate(2,1)"><circle cx="3" cy="4" r="2"/><ellipse cx="1" cy="2" rx="3" ry="1"/><path d="M0 0 A1 1 0 011 1"/></g><text>x</text><use href="#x"/></svg>"##;
        let d = import(svg).unwrap();
        assert_eq!(d.layers, ["cut"]);
        assert_eq!(d.shapes.len(), 3);
        assert_eq!(d.shapes[0].primitive, Primitive::Circle { center: Point::new(3., 4.), radius: 2. });
        assert_eq!(d.shapes[0].transform, [1., 0., 0., -1., 2., 9.]);
        assert!(matches!(d.shapes[1].primitive, Primitive::Ellipse { .. }));
        assert!(matches!(&d.shapes[2].primitive,Primitive::Path(p) if matches!(p[0].segs[0],path_data::Segment::Arc { .. })));
        assert_eq!(d.skipped.get(&Skipped::Text), Some(&1));
        assert_eq!(d.skipped.get(&Skipped::Use), Some(&1));
    }
    #[test]
    fn streaming_delivers_layers_before_shapes_and_stops_on_error() {
        let s = r#"<svg><g inkscape:groupmode="layer" inkscape:label="cut"><circle r="1"/><circle r="2"/></g></svg>"#;
        let mut events = Vec::new();
        let result = visit(s, &mut |event| {
            match event {
                Event::Layer(name) => events.push(name),
                Event::Shape(_) => {
                    events.push("shape".into());
                    return Err(Error("stop".into()));
                }
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(events, ["cut", "shape"]);
    }

    #[test]
    fn refuses_excessive_nesting() {
        let s = format!("<svg>{}{}</svg>", "<g>".repeat(100), "</g>".repeat(100));
        assert!(import(&s).is_err());
    }
}
