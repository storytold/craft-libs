//! Bounded conversion of common planar DXF entities into neutral Bézier paths.
//!
//! This is geometry extraction, not a full CAD scene renderer: unsupported entities such as
//! text, dimensions, meshes, and block inserts are reported as warnings and omitted.

use kurbo::{BezPath, Point, Rect, Shape as _};

/// Limits on the number of paths returned by [`import`]. Parsing also uses the crate's tag cap.
pub const MAX_PATHS: usize = 200_000;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("DXF parse failed: {0}")]
    Parse(#[from] crate::DxfError),
    #[error("DXF contains more than {MAX_PATHS} paths")]
    TooManyPaths,
    #[error("DXF contains a non-finite path coordinate")]
    NonFinite,
}

/// Geometry in source DXF coordinates. `unit_to_mm` maps a source coordinate to millimetres;
/// unitless drawings use `1.0`, as the coordinates are then interpreted as millimetres.
#[derive(Debug, Clone)]
pub struct Imported {
    pub paths: Vec<BezPath>,
    pub bounds: Option<Rect>,
    pub unit_to_mm: f64,
    pub warnings: Vec<String>,
}

/// Parse bounded DXF input and extract visible planar linework.
pub fn import(bytes: &[u8]) -> Result<Imported, Error> {
    let tags = crate::parse_with_limit(bytes, crate::max_tags())?;
    let drawing = crate::model::Document::read(&tags);
    let unit_to_mm = units_to_mm(drawing.header.insunits(), drawing.header.measurement());
    let mut paths = Vec::new();
    let mut warnings = Vec::new();
    for record in &drawing.entities {
        let entity = crate::model::Entity::read(record);
        if !entity.common.visible || record.paper_space() {
            continue;
        }
        let converted = entity_paths(&entity.kind, entity.common.extrusion);
        if converted.iter().flat_map(|path| path.elements()).any(|element| {
            let finite = |p: Point| p.x.is_finite() && p.y.is_finite();
            match *element {
                kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => !finite(p),
                kurbo::PathEl::QuadTo(a, b) => !finite(a) || !finite(b),
                kurbo::PathEl::CurveTo(a, b, c) => !finite(a) || !finite(b) || !finite(c),
                kurbo::PathEl::ClosePath => false,
            }
        }) {
            return Err(Error::NonFinite);
        }
        if converted.is_empty() {
            continue;
        }
        if paths.len().saturating_add(converted.len()) > MAX_PATHS {
            return Err(Error::TooManyPaths);
        }
        paths.extend(converted);
    }
    if drawing.skipped.iter().any(|(_, n)| *n > 0) {
        warnings.push("some unsupported or stray DXF records were skipped".into());
    }
    let bounds = paths.iter().map(|path| path.bounding_box()).reduce(|a, b| a.union(b));
    Ok(Imported { paths, bounds, unit_to_mm, warnings })
}

fn units_to_mm(code: i64, measurement: Option<i64>) -> f64 {
    match code {
        0 if measurement == Some(0) => 25.4,
        1 => 25.4,
        2 => 304.8,
        3 => 1_609_344.0,
        5 => 10.0,
        6 => 1_000.0,
        7 => 1_000_000.0,
        8 => 0.000_025_4,
        9 => 0.0254,
        10 => 914.4,
        11 => 1e-7,
        12 => 1e-6,
        13 => 0.001,
        14 => 100.0,
        15 => 10_000.0,
        16 => 100_000.0,
        17 => 1e9,
        _ => 1.0,
    }
}

fn entity_paths(kind: &crate::model::EntityKind, normal: crate::model::Point) -> Vec<BezPath> {
    use crate::model::EntityKind as E;
    match kind {
        E::Line { start, end } => vec![line(*start, *end)],
        E::Circle { center, radius } if radius.is_finite() && *radius > 0.0 => {
            vec![circle(ocs(normal, *center), *radius)]
        }
        E::Arc { center, radius, start_angle, end_angle }
            if radius.is_finite() && *radius > 0.0 && start_angle.is_finite() && end_angle.is_finite() =>
        {
            vec![arc(ocs(normal, *center), *radius, start_angle.to_radians(), end_angle.to_radians())]
        }
        E::LwPolyline(poly) => vec![polyline(poly.vertices.iter().map(|v| (ocs(normal, [v.x, v.y, poly.elevation]), v.bulge)), poly.closed)],
        E::Polyline(poly) if !poly.is_mesh() && !poly.is_polyface() => {
            vec![polyline(poly.vertices.iter().filter(|v| !v.is_face()).map(|v| (ocs(normal, v.location), v.bulge)), poly.closed())]
        }
        E::Ellipse { center, major, ratio, start, end } if ratio.is_finite() && *ratio > 0.0 => {
            vec![ellipse(*center, *major, *ratio, *start, *end)]
        }
        _ => Vec::new(),
    }
}

fn ocs(normal: crate::model::Point, point: crate::model::Point) -> Point {
    let p = crate::model::ocs_to_wcs(normal, point);
    Point::new(p[0], p[1])
}

fn line(start: crate::model::Point, end: crate::model::Point) -> BezPath {
    let mut path = BezPath::new();
    path.move_to((start[0], start[1]));
    path.line_to((end[0], end[1]));
    path
}

fn polyline(vertices: impl IntoIterator<Item = (Point, f64)>, closed: bool) -> BezPath {
    let vertices = vertices.into_iter().collect::<Vec<_>>();
    let mut path = BezPath::new();
    let Some((first, _)) = vertices.first().copied() else { return path };
    path.move_to(first);
    for i in 1..vertices.len() {
        let (a, bulge) = vertices[i - 1];
        let (b, _) = vertices[i];
        if bulge.is_finite() && bulge.abs() > 1e-12 {
            append_bulge(&mut path, a, b, bulge);
        } else {
            path.line_to(b);
        }
    }
    if closed && vertices.len() > 1 {
        let (a, bulge) = *vertices.last().unwrap_or(&(first, 0.0));
        if bulge.is_finite() && bulge.abs() > 1e-12 {
            append_bulge(&mut path, a, first, bulge);
        } else {
            path.line_to(first);
        }
        path.close_path();
    }
    path
}

fn append_bulge(path: &mut BezPath, a: Point, b: Point, bulge: f64) {
    let chord = b - a;
    let length = chord.hypot();
    if !(length.is_finite() && length > 0.0) {
        path.line_to(b);
        return;
    }
    let center = a + chord * 0.5 + kurbo::Vec2::new(-chord.y, chord.x) * ((1.0 - bulge * bulge) / (4.0 * bulge));
    let start = (a.y - center.y).atan2(a.x - center.x);
    let sweep = 4.0 * bulge.atan();
    let radius = length * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    arc_segments(path, center, start, sweep, radius);
}

fn circle(center: Point, radius: f64) -> BezPath {
    let mut path = BezPath::new();
    path.move_to((center.x + radius, center.y));
    arc_segments(&mut path, center, 0.0, std::f64::consts::TAU, radius);
    path.close_path();
    path
}

fn arc(center: Point, radius: f64, start: f64, end: f64) -> BezPath {
    let mut path = BezPath::new();
    path.move_to((center.x + radius * start.cos(), center.y + radius * start.sin()));
    let sweep = if end >= start { end - start } else { end - start + std::f64::consts::TAU };
    arc_segments(&mut path, center, start, sweep, radius);
    path
}

fn arc_segments(path: &mut BezPath, center: Point, start: f64, sweep: f64, radius: f64) {
    let count = (sweep.abs() / (std::f64::consts::FRAC_PI_2)).ceil().max(1.0) as usize;
    let step = sweep / count as f64;
    for i in 0..count {
        let a = start + step * i as f64;
        let b = a + step;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let p1 = Point::new(center.x + radius * (a.cos() - k * a.sin()), center.y + radius * (a.sin() + k * a.cos()));
        let p2 = Point::new(center.x + radius * (b.cos() + k * b.sin()), center.y + radius * (b.sin() - k * b.cos()));
        let p3 = Point::new(center.x + radius * b.cos(), center.y + radius * b.sin());
        path.curve_to(p1, p2, p3);
    }
}

fn ellipse(center: crate::model::Point, major: crate::model::Point, ratio: f64, start: f64, end: f64) -> BezPath {
    let (cx, cy) = (center[0], center[1]);
    let (ax, ay) = (major[0], major[1]);
    let (bx, by) = (-ay * ratio, ax * ratio);
    let point = |t: f64| Point::new(cx + ax * t.cos() + bx * t.sin(), cy + ay * t.cos() + by * t.sin());
    let mut path = BezPath::new();
    path.move_to(point(start));
    let sweep = if end >= start { end - start } else { end - start + std::f64::consts::TAU };
    let count = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / count as f64;
    for i in 0..count {
        let a = start + step * i as f64;
        let b = a + step;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let derivative = |t: f64| Point::new(-ax * t.sin() + bx * t.cos(), -ay * t.sin() + by * t.cos());
        let p1 = point(a) + derivative(a).to_vec2() * k;
        let p2 = point(b) - derivative(b).to_vec2() * k;
        path.curve_to(p1, p2, point(b));
    }
    path
}
