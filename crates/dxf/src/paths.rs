//! Bounded conversion of common planar DXF entities into neutral Bézier paths.
//!
//! This is geometry extraction, not a full CAD scene renderer: unsupported entities such as
//! text, dimensions, meshes, and block inserts are reported as warnings and omitted.

use kurbo::{BezPath, Point, Rect, Shape as _};

/// Limits on the number of paths returned by [`import`]. Parsing also uses the crate's tag cap.
pub const MAX_PATHS: usize = 200_000;
/// Aggregate number of Bézier elements a drawing may generate during conversion.
pub const MAX_PATH_ELEMENTS: usize = 2_000_000;
/// Maximum line segments used to approximate one DXF spline.
pub const MAX_SPLINE_SEGMENTS: usize = 4096;
const MAX_SPLINE_DEPTH: u8 = 12;
const MAX_SPLINE_DEGREE: usize = 16;
const DEFAULT_SPLINE_TOLERANCE: f64 = 0.01;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("DXF parse failed: {0}")]
    Parse(#[from] crate::DxfError),
    #[error("DXF contains more than {MAX_PATHS} paths")]
    TooManyPaths,
    #[error("DXF curve exceeds the subdivision limit")]
    TooManySegments,
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
    let layer_visibility: std::collections::HashMap<_, _> =
        drawing.layers().into_iter().map(|layer| (layer.name, layer.on && !layer.frozen)).collect();
    let mut paths = Vec::new();
    let mut warnings = Vec::new();
    let mut unsupported = 0usize;
    let mut elements = 0usize;
    for record in &drawing.entities {
        let entity = crate::model::Entity::read(record);
        if !entity.common.visible || record.paper_space() || layer_visibility.get(&entity.common.layer) == Some(&false) {
            continue;
        }
        if excessive_sweep(&entity.kind) {
            return Err(Error::TooManySegments);
        }
        let remaining = MAX_PATH_ELEMENTS.saturating_sub(elements);
        if estimated_elements(&entity.kind) > remaining {
            return Err(Error::TooManySegments);
        }
        let converted = entity_paths(&entity.kind, entity.common.extrusion)?;
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
            unsupported = unsupported.saturating_add(1);
            continue;
        }
        elements = elements.saturating_add(converted.iter().map(|path| path.elements().len()).sum::<usize>());
        if elements > MAX_PATH_ELEMENTS {
            return Err(Error::TooManySegments);
        }
        if paths.len().saturating_add(converted.len()) > MAX_PATHS {
            return Err(Error::TooManyPaths);
        }
        paths.extend(converted);
    }
    if unsupported > 0 {
        warnings.push(format!("{unsupported} visible DXF entities are unsupported and were omitted"));
    }
    if drawing.skipped.iter().any(|(_, n)| *n > 0) {
        warnings.push("some unsupported or stray DXF records were skipped".into());
    }
    let bounds = paths.iter().map(|path| path.bounding_box()).reduce(|a, b| a.union(b));
    Ok(Imported { paths, bounds, unit_to_mm, warnings })
}

fn estimated_elements(kind: &crate::model::EntityKind) -> usize {
    use crate::model::EntityKind as E;
    let arc_count = |sweep: f64| (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let bulge_count = |bulge: f64| {
        if bulge.is_finite() && bulge.abs() > 1e-12 { arc_count(4.0 * bulge.atan()) } else { 1 }
    };
    let polyline_count = |vertices: &[(Point, f64)], closed: bool| {
        if vertices.is_empty() {
            return 0;
        }
        let mut count = 1usize;
        for pair in vertices.windows(2) {
            count = count.saturating_add(bulge_count(pair[0].1));
        }
        if closed && vertices.len() > 1 {
            count = count.saturating_add(bulge_count(vertices.last().map_or(0.0, |vertex| vertex.1)));
            count = count.saturating_add(1);
        }
        count
    };
    match kind {
        E::Line { .. } => 2,
        E::Circle { .. } => 6,
        E::Arc { start_angle, end_angle, .. } => 1usize.saturating_add(arc_count((end_angle - start_angle).to_radians())),
        E::LwPolyline(poly) => {
            let vertices: Vec<_> = poly.vertices.iter().map(|v| (Point::new(v.x, v.y), v.bulge)).collect();
            polyline_count(&vertices, poly.closed)
        }
        E::Polyline(poly) if !poly.is_mesh() && !poly.is_polyface() => {
            let vertices: Vec<_> = poly
                .vertices
                .iter()
                .filter(|vertex| !vertex.is_face())
                .map(|vertex| (Point::new(vertex.location[0], vertex.location[1]), vertex.bulge))
                .collect();
            polyline_count(&vertices, poly.closed())
        }
        E::Ellipse { start, end, .. } => 1usize.saturating_add(arc_count(end - start)),
        E::Spline(spline) => spline_spans(spline).map_or(0, |spans| spans.saturating_mul(8)),
        _ => 0,
    }
}

fn units_to_mm(code: i64, _measurement: Option<i64>) -> f64 {
    match code {
        0 => 1.0,
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
        17 => 1e12,
        18 => 149_597_870_700_000.0,
        19 => 9.460_730_472_580_8e18,
        20 => 3.085_677_581_491_367e19,
        21 => 1200.0 / 3937.0 * 1000.0,
        22 => 1200.0 / 3937.0 * 1000.0 / 12.0,
        23 => 1200.0 / 3937.0 * 1000.0 * 3.0,
        24 => 1200.0 / 3937.0 * 1000.0 * 5280.0,
        _ => 1.0,
    }
}

fn entity_paths(kind: &crate::model::EntityKind, normal: crate::model::Point) -> Result<Vec<BezPath>, Error> {
    use crate::model::EntityKind as E;
    Ok(match kind {
        E::Line { start, end } => vec![line(*start, *end)],
        E::Circle { center, radius } if radius.is_finite() && *radius > 0.0 => {
            vec![map_ocs_path(circle(Point::new(center[0], center[1]), *radius), normal, center[2])]
        }
        E::Arc { center, radius, start_angle, end_angle }
            if radius.is_finite() && *radius > 0.0 && start_angle.is_finite() && end_angle.is_finite() =>
        {
            vec![map_ocs_path(arc(Point::new(center[0], center[1]), *radius, start_angle.to_radians(), end_angle.to_radians()), normal, center[2])]
        }
        E::LwPolyline(poly) => {
            vec![map_ocs_path(polyline(poly.vertices.iter().map(|v| (Point::new(v.x, v.y), v.bulge)), poly.closed), normal, poly.elevation)]
        }
        E::Polyline(poly) if !poly.is_mesh() && !poly.is_polyface() => {
            let is_3d = poly.flags & 8 != 0;
            let z = if is_3d { 0.0 } else { poly.elevation };
            let path =
                polyline(poly.vertices.iter().filter(|v| !v.is_face()).map(|v| (Point::new(v.location[0], v.location[1]), v.bulge)), poly.closed());
            vec![if is_3d { path } else { map_ocs_path(path, normal, z) }]
        }
        E::Ellipse { center, major, ratio, start, end } if ratio.is_finite() && *ratio > 0.0 => {
            vec![ellipse(*center, *major, normal, *ratio, *start, *end)]
        }
        E::Spline(spline) => spline_path(spline)?.into_iter().collect(),
        _ => Vec::new(),
    })
}

fn spline_spans(spline: &crate::model::Spline) -> Option<usize> {
    let degree = usize::try_from(spline.degree).ok()?;
    let count = spline.control.len();
    if degree == 0 || degree > MAX_SPLINE_DEGREE || degree >= count || spline.knots.len() != count.checked_add(degree)?.checked_add(1)? {
        return None;
    }
    let start = spline.knots[degree];
    let end = spline.knots[count];
    if !(start.is_finite() && end.is_finite() && start < end) {
        return None;
    }
    let spans = (degree..count).filter(|&i| spline.knots[i].is_finite() && spline.knots[i] < spline.knots[i + 1]).count();
    (spans > 0).then_some(spans)
}

fn spline_path(spline: &crate::model::Spline) -> Result<Option<BezPath>, Error> {
    let Some(spans) = spline_spans(spline) else {
        // Fit-point-only and malformed splines are reported as unsupported by the caller.
        return Ok(None);
    };
    if spans > MAX_SPLINE_SEGMENTS {
        return Err(Error::TooManySegments);
    }
    let degree = usize::try_from(spline.degree).map_err(|_| Error::TooManySegments)?;
    let count = spline.control.len();
    if !spline.weights.is_empty() && spline.weights.len() != count {
        return Ok(None);
    }
    if spline.knots.iter().any(|knot| !knot.is_finite())
        || spline.control.iter().flatten().any(|coordinate| !coordinate.is_finite())
        || spline.weights.iter().any(|weight| !weight.is_finite())
    {
        return Err(Error::NonFinite);
    }
    if spline.knots.windows(2).any(|pair| pair[0] > pair[1]) {
        return Ok(None);
    }
    let tolerance = if spline.fit_tolerance.is_finite() && spline.fit_tolerance > 0.0 { spline.fit_tolerance } else { DEFAULT_SPLINE_TOLERANCE };
    let mut path = BezPath::new();
    let mut sampler = SplineSampler { spline, tolerance, segments: 0, output: Vec::new() };
    let mut first = None;
    let mut last_point: Option<[f64; 3]> = None;
    for span in degree..count {
        let t0 = spline.knots[span];
        let t1 = spline.knots[span + 1];
        if !(t0.is_finite() && t1.is_finite() && t0 < t1) {
            continue;
        }
        let Some(p0) = spline_point(spline, span, t0) else {
            return Ok(None);
        };
        let Some(p1) = spline_point(spline, span, t1) else {
            return Ok(None);
        };
        if p0.iter().any(|coordinate| !coordinate.is_finite()) || p1.iter().any(|coordinate| !coordinate.is_finite()) {
            return Err(Error::NonFinite);
        }
        if first.is_none() {
            first = Some(p0);
            path.move_to((p0[0], p0[1]));
        } else if last_point.is_some_and(|previous| (previous[0] - p0[0]).abs() > tolerance || (previous[1] - p0[1]).abs() > tolerance) {
            // Discontinuous knot spans form distinct open contours.
            path.move_to((p0[0], p0[1]));
        }
        sampler.append(span, t0, p0, t1, p1, 0)?;
        for point in sampler.output.drain(..) {
            path.line_to((point[0], point[1]));
            last_point = Some(point);
        }
    }
    if spline.flags & 1 != 0
        && let Some(first) = first
    {
        path.line_to((first[0], first[1]));
        path.close_path();
    }
    if path.elements().is_empty() { Ok(None) } else { Ok(Some(path)) }
}

struct SplineSampler<'a> {
    spline: &'a crate::model::Spline,
    tolerance: f64,
    segments: usize,
    output: Vec<[f64; 3]>,
}

impl SplineSampler<'_> {
    fn append(&mut self, span: usize, t0: f64, p0: [f64; 3], t1: f64, p1: [f64; 3], depth: u8) -> Result<(), Error> {
        let quarter_t = t0 + (t1 - t0) * 0.25;
        let middle_t = t0 + (t1 - t0) * 0.5;
        let three_quarter_t = t0 + (t1 - t0) * 0.75;
        let Some(quarter) = spline_point(self.spline, span, quarter_t) else { return Err(Error::NonFinite) };
        let Some(middle) = spline_point(self.spline, span, middle_t) else { return Err(Error::NonFinite) };
        let Some(three_quarter) = spline_point(self.spline, span, three_quarter_t) else { return Err(Error::NonFinite) };
        if [quarter, middle, three_quarter].iter().flatten().any(|coordinate| !coordinate.is_finite()) {
            return Err(Error::NonFinite);
        }
        let flatness = [quarter, middle, three_quarter]
            .iter()
            .map(|point| distance_to_segment(Point::new(point[0], point[1]), Point::new(p0[0], p0[1]), Point::new(p1[0], p1[1])))
            .fold(0.0_f64, f64::max);
        if flatness <= self.tolerance {
            self.segments = self.segments.saturating_add(1);
            if self.segments > MAX_SPLINE_SEGMENTS {
                return Err(Error::TooManySegments);
            }
            self.output.push(p1);
            return Ok(());
        }
        if depth >= MAX_SPLINE_DEPTH || self.segments >= MAX_SPLINE_SEGMENTS {
            return Err(Error::TooManySegments);
        }
        self.append(span, t0, p0, middle_t, middle, depth + 1)?;
        self.append(span, middle_t, middle, t1, p1, depth + 1)
    }
}

fn distance_to_segment(point: Point, start: Point, end: Point) -> f64 {
    let chord = end - start;
    let length_sq = chord.hypot2();
    if length_sq == 0.0 {
        return (point - start).hypot();
    }
    let t = ((point - start).dot(chord) / length_sq).clamp(0.0, 1.0);
    (point - (start + chord * t)).hypot()
}

fn spline_point(spline: &crate::model::Spline, span: usize, t: f64) -> Option<[f64; 3]> {
    let degree = usize::try_from(spline.degree).ok()?;
    let mut values = Vec::with_capacity(degree + 1);
    for j in 0..=degree {
        let index = span.checked_sub(degree)?.checked_add(j)?;
        let weight = spline.weights.get(index).copied().unwrap_or(1.0);
        let point = *spline.control.get(index)?;
        values.push([point[0] * weight, point[1] * weight, point[2] * weight, weight]);
    }
    for level in 1..=degree {
        for j in (level..=degree).rev() {
            let index = span.checked_sub(degree)?.checked_add(j)?;
            let left = *spline.knots.get(index)?;
            let right = *spline.knots.get(index + degree + 1 - level)?;
            let denominator = right - left;
            let alpha = if denominator == 0.0 { 0.0 } else { (t - left) / denominator };
            let (before, after) = values.split_at_mut(j);
            for (previous, current) in before[j - 1].iter().copied().zip(after[0].iter_mut()) {
                *current = (1.0 - alpha) * previous + alpha * *current;
            }
        }
    }
    let value = values[degree];
    (value[3].is_finite() && value[3].abs() > f64::EPSILON).then(|| [value[0] / value[3], value[1] / value[3], value[2] / value[3]])
}

fn map_ocs_path(path: BezPath, normal: crate::model::Point, z: f64) -> BezPath {
    let [x, y, n] = crate::model::ocs_axes(normal);
    let transform = kurbo::Affine::new([x[0], x[1], y[0], y[1], n[0] * z, n[1] * z]);
    transform * path
}

fn excessive_sweep(kind: &crate::model::EntityKind) -> bool {
    let cap = 4096.0 * std::f64::consts::FRAC_PI_2;
    let over = |sweep: f64| !sweep.is_finite() || sweep.abs() > cap;
    match kind {
        crate::model::EntityKind::Arc { start_angle, end_angle, .. } => over((end_angle - start_angle).to_radians()),
        crate::model::EntityKind::Ellipse { start, end, .. } => over(end - start),
        _ => false,
    }
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
    if count > 4096 {
        return;
    }
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

fn ellipse(center: crate::model::Point, major: crate::model::Point, normal: crate::model::Point, ratio: f64, start: f64, end: f64) -> BezPath {
    let minor = cross(normal, major);
    let minor_len = minor.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !(minor_len.is_finite() && minor_len > 0.0) {
        return BezPath::new();
    }
    let major_len = major.iter().map(|v| v * v).sum::<f64>().sqrt();
    let bx = [minor[0] / minor_len * major_len * ratio, minor[1] / minor_len * major_len * ratio, minor[2] / minor_len * major_len * ratio];
    let point = |t: f64| Point::new(center[0] + major[0] * t.cos() + bx[0] * t.sin(), center[1] + major[1] * t.cos() + bx[1] * t.sin());
    let mut path = BezPath::new();
    path.move_to(point(start));
    let sweep = if end >= start { end - start } else { end - start + std::f64::consts::TAU };
    let count = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    if count > 4096 {
        return path;
    }
    let step = sweep / count as f64;
    for i in 0..count {
        let a = start + step * i as f64;
        let b = a + step;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let derivative = |t: f64| Point::new(-major[0] * t.sin() + bx[0] * t.cos(), -major[1] * t.sin() + bx[1] * t.cos());
        let p1 = point(a) + derivative(a).to_vec2() * k;
        let p2 = point(b) - derivative(b).to_vec2() * k;
        path.curve_to(p1, p2, point(b));
    }
    path
}

fn cross(a: crate::model::Point, b: crate::model::Point) -> crate::model::Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
