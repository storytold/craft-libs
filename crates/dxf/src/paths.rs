//! Bounded conversion of common planar DXF entities into neutral Bézier paths.
//!
//! This is geometry extraction, not a full CAD scene renderer: unsupported entities such as
//! text, dimensions, meshes, and block inserts are reported as warnings and omitted.

use kurbo::{BezPath, Point, Rect, Shape as _};

/// Limits on the number of paths returned by [`import`]. Parsing also uses the crate's tag cap.
pub const MAX_PATHS: usize = 200_000;
/// Aggregate number of Bézier elements a drawing may generate during conversion.
pub const MAX_PATH_ELEMENTS: usize = 2_000_000;

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
        let converted = if let crate::model::EntityKind::Spline(spline) = &entity.kind {
            spline_path(spline)?.into_iter().collect()
        } else {
            entity_paths(&entity.kind, entity.common.extrusion)
        };
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
        // `spline_path` already caps subdivision at MAX_SPLINE_ELEMENTS. Its actual output is
        // checked against the drawing-wide budget after conversion.
        E::Spline(_) => 0,
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

fn entity_paths(kind: &crate::model::EntityKind, normal: crate::model::Point) -> Vec<BezPath> {
    use crate::model::EntityKind as E;
    match kind {
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
        _ => Vec::new(),
    }
}

const MAX_SPLINE_ELEMENTS: usize = 200_000;
const SPLINE_TOLERANCE: f64 = 1e-6;
const MAX_SPLINE_DEPTH: u32 = 12;
const MAX_SPLINE_DEGREE: usize = 11;
const MIN_RELATIVE_WEIGHT: f64 = 1e-100;

#[derive(Clone, Copy)]
struct Homogeneous {
    x: f64,
    y: f64,
    w: f64,
}

impl Homogeneous {
    fn lerp(self, other: Self, t: f64) -> Self {
        let s = 1.0 - t;
        Self { x: self.x * s + other.x * t, y: self.y * s + other.y * t, w: self.w * s + other.w * t }
    }
    fn point(self) -> Option<Point> {
        (self.w.abs() > 1e-300).then(|| Point::new(self.x / self.w, self.y / self.w)).filter(|point| point.x.is_finite() && point.y.is_finite())
    }
}

// Keep the NURBS conversion semantics of the previous importer: exact polynomial Bezier
// spans through cubic degree, tolerance-bounded rational/high-degree spans, invalid-weight
// fallback, and fit-point fallback when control data is malformed.
fn spline_path(spline: &crate::model::Spline) -> Result<Option<BezPath>, Error> {
    let weights = (spline.flags & 4 != 0 && spline.weights.len() == spline.control.len()).then_some(spline.weights.as_slice());
    let mut limit_hit = false;
    let converted = usize::try_from(spline.degree).ok().and_then(|degree| nurbs(degree, &spline.knots, &spline.control, weights, &mut limit_hit));
    let path = converted
        .map(|mut path| {
            if spline.flags & 1 != 0 {
                path.close_path();
            }
            path
        })
        .or_else(|| {
            let closed = spline.flags & 1 != 0;
            // `through` emits one move, n-1 (open) or n (closed) cubics, and
            // an additional close element for closed paths. Check before allocating
            // its converted point vector.
            let max_points = MAX_SPLINE_ELEMENTS.saturating_sub(if closed { 2 } else { 0 });
            if spline.fit.len() > max_points {
                limit_hit = true;
                return None;
            }
            through(&spline.fit.iter().map(|p| Point::new(p[0], p[1])).collect::<Vec<_>>(), closed)
        });
    if limit_hit {
        return Err(Error::TooManySegments);
    }
    Ok(path)
}

fn nurbs(degree: usize, knots: &[f64], control: &[[f64; 3]], weights: Option<&[f64]>, limit_hit: &mut bool) -> Option<BezPath> {
    let n = control.len();
    if !(1..=MAX_SPLINE_DEGREE).contains(&degree)
        || n <= degree
        || knots.len() != n + degree + 1
        || knots.windows(2).any(|w| w[1] < w[0])
        || control.iter().any(|p| !p[0].is_finite() || !p[1].is_finite())
        || knots.iter().any(|k| !k.is_finite())
    {
        return None;
    }
    let raw = |i: usize| weights.and_then(|w| w.get(i)).copied().filter(|w| *w > 0.0 && w.is_finite()).unwrap_or(1.0);
    let max_w = (0..n).map(raw).fold(0.0, f64::max);
    if max_w == 0.0 {
        return None;
    }
    let weight = |i: usize| (raw(i) / max_w).max(MIN_RELATIVE_WEIGHT);
    let rational = (0..n).any(|i| (weight(i) - weight(0)).abs() > 1e-12);
    let (lo, hi) = control.iter().fold((Point::new(f64::MAX, f64::MAX), Point::new(f64::MIN, f64::MIN)), |(lo, hi), p| {
        (Point::new(lo.x.min(p[0]), lo.y.min(p[1])), Point::new(hi.x.max(p[0]), hi.y.max(p[1])))
    });
    let tolerance = ((hi - lo).hypot() * SPLINE_TOLERANCE).max(1e-12);
    let mut path = BezPath::new();
    for span in degree..n {
        let (Some(&t0), Some(&t1)) = (knots.get(span), knots.get(span + 1)) else { continue };
        if t1 <= t0 {
            continue;
        }
        let bezier: Vec<Homogeneous> = (0..=degree)
            .map(|j| {
                let args: Vec<_> = (0..degree).map(|k| if k < degree - j { t0 } else { t1 }).collect();
                blossom(degree, knots, control, span, &args, &weight)
            })
            .collect::<Option<_>>()?;
        let start = bezier.first()?.point()?;
        let needs_new_subpath = path_endpoint(&path).is_none_or(|end| end.distance(start) > 1e-12 * (1.0 + start.to_vec2().hypot()));
        if needs_new_subpath {
            path.move_to(start);
        }
        // A rational degree-one span changes its parameterization with its weights, but its locus
        // is still the straight segment between its endpoints. Preserve that exact geometry.
        let polynomial: Option<Vec<Point>> = if rational && degree > 1 { None } else { bezier.iter().map(|p| p.point()).collect() };
        match polynomial.as_deref() {
            Some([_, p]) => path.line_to(*p),
            Some([_, q, p]) => path.quad_to(*q, *p),
            Some([_, a, b, p]) => path.curve_to(*a, *b, *p),
            _ => fit_spline(&mut path, &bezier, tolerance, 0, limit_hit)?,
        }
        if path.elements().len() > MAX_SPLINE_ELEMENTS {
            *limit_hit = true;
            return None;
        }
    }
    (path.elements().len() > 1).then_some(path)
}

fn path_endpoint(path: &BezPath) -> Option<Point> {
    match path.elements().last()? {
        kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => Some(*p),
        kurbo::PathEl::QuadTo(_, p) => Some(*p),
        kurbo::PathEl::CurveTo(_, _, p) => Some(*p),
        kurbo::PathEl::ClosePath => None,
    }
}

fn blossom(degree: usize, knots: &[f64], control: &[[f64; 3]], span: usize, args: &[f64], weight: &impl Fn(usize) -> f64) -> Option<Homogeneous> {
    // Only the active degree+1 control points are needed for one span. Convert this small window
    // here instead of making a second vector proportional to an untrusted entity's full controls.
    let first = span.checked_sub(degree)?;
    let mut points: Vec<_> = control
        .get(first..=span)?
        .iter()
        .enumerate()
        .map(|(offset, p)| {
            let w = weight(first + offset);
            Homogeneous { x: p[0] * w, y: p[1] * w, w }
        })
        .collect();
    for r in 1..=degree {
        let u = *args.get(r - 1)?;
        for j in (r..=degree).rev() {
            let i = span - degree + j;
            let (a, b) = (*knots.get(i)?, *knots.get(i + degree + 1 - r)?);
            let alpha = if b - a > 1e-300 { (u - a) / (b - a) } else { 0.0 };
            let previous = *points.get(j - 1)?;
            let current = points.get_mut(j)?;
            *current = previous.lerp(*current, alpha);
        }
    }
    points.get(degree).copied()
}

fn eval_spline_bezier(bezier: &[Homogeneous], t: f64) -> Option<(Point, kurbo::Vec2)> {
    let mut points = bezier.to_vec();
    let n = points.len().checked_sub(1)?;
    for level in (1..=n).rev() {
        if level == 1 {
            let (a, b) = (*points.first()?, *points.get(1)?);
            let p = a.lerp(b, t);
            let point = p.point()?;
            let k = n as f64;
            let (dx, dy, dw) = (k * (b.x - a.x), k * (b.y - a.y), k * (b.w - a.w));
            let w2 = p.w * p.w;
            return Some((point, kurbo::Vec2::new((dx * p.w - p.x * dw) / w2, (dy * p.w - p.y * dw) / w2)));
        }
        for i in 0..level {
            let next = *points.get(i + 1)?;
            let current = points.get_mut(i)?;
            *current = current.lerp(next, t);
        }
    }
    Some((points.first()?.point()?, kurbo::Vec2::ZERO))
}

fn fit_spline(path: &mut BezPath, bezier: &[Homogeneous], tolerance: f64, depth: u32, limit_hit: &mut bool) -> Option<()> {
    let (p0, d0) = eval_spline_bezier(bezier, 0.0)?;
    let (p3, d3) = eval_spline_bezier(bezier, 1.0)?;
    let (c1, c2) = (p0 + d0 / 3.0, p3 - d3 / 3.0);
    if ![c1, c2].iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        path.line_to(p3);
        return Some(());
    }
    let candidate = [p0, c1, c2, p3];
    if cubic_error_bound(bezier, &candidate).is_some_and(|error| error <= tolerance) {
        if path.elements().len() >= MAX_SPLINE_ELEMENTS {
            *limit_hit = true;
            return None;
        }
        path.curve_to(c1, c2, p3);
        return Some(());
    }
    if depth >= MAX_SPLINE_DEPTH || path.elements().len() >= MAX_SPLINE_ELEMENTS {
        *limit_hit = true;
        return None;
    }
    let (left, right) = split_homogeneous_bezier(bezier)?;
    fit_spline(path, &left, tolerance, depth + 1, limit_hit)?;
    fit_spline(path, &right, tolerance, depth + 1, limit_hit)
}

fn split_homogeneous_bezier(bezier: &[Homogeneous]) -> Option<(Vec<Homogeneous>, Vec<Homogeneous>)> {
    let count = bezier.len();
    if count < 2 {
        return None;
    }
    let mut row = bezier.to_vec();
    let mut left = vec![*bezier.first()?; count];
    let mut right = vec![*bezier.last()?; count];
    left[0] = row[0];
    right[count - 1] = row[count - 1];
    for level in 1..count {
        for index in 0..count - level {
            row[index] = row[index].lerp(row[index + 1], 0.5);
        }
        left[level] = row[0];
        right[count - 1 - level] = row[count - 1 - level];
    }
    Some((left, right))
}

// Bound the rational curve/cubic difference over the whole parameter interval. In Bernstein
// form, X(t)-W(t)C(t) has degree p+3, so its norm is bounded by the largest coefficient norm;
// W(t) is a convex combination of positive weights and is at least their minimum.
fn cubic_error_bound(bezier: &[Homogeneous], cubic: &[Point; 4]) -> Option<f64> {
    let degree = bezier.len().checked_sub(1)?;
    let total = degree.checked_add(3)?;
    let min_weight = bezier.iter().map(|point| point.w).fold(f64::INFINITY, f64::min);
    if !(min_weight > 0.0 && min_weight.is_finite()) {
        return None;
    }
    let mut max_error_numerator: f64 = 0.0;
    for k in 0..=total {
        let mut x = 0.0;
        let mut y = 0.0;
        for (i, point) in bezier.iter().enumerate() {
            let Some(j) = k.checked_sub(i).filter(|j| *j < 4) else { continue };
            let factor = binomial(degree, i) * binomial(3, j) / binomial(total, k);
            x += (point.x - point.w * cubic[j].x) * factor;
            y += (point.y - point.w * cubic[j].y) * factor;
        }
        max_error_numerator = max_error_numerator.max(x.hypot(y));
    }
    Some(max_error_numerator / min_weight)
}

fn binomial(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    (1..=k).fold(1.0, |value, i| value * (n + 1 - i) as f64 / i as f64)
}

fn through(points: &[Point], closed: bool) -> Option<BezPath> {
    let n = points.len();
    let first = *points.first()?;
    if n < 2 {
        return None;
    }
    let at = |i: isize| {
        let len = n as isize;
        let j = if closed { i.rem_euclid(len) } else { i.clamp(0, len - 1) };
        points.get(j as usize).copied().unwrap_or(first)
    };
    let mut path = BezPath::new();
    path.move_to(first);
    let segments = if closed { n } else { n - 1 };
    for i in 0..segments as isize {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        path.curve_to(p1 + (p2 - p0) / 6.0, p2 - (p3 - p1) / 6.0, p2);
    }
    if closed {
        path.close_path();
    }
    Some(path)
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

#[cfg(test)]
mod spline_tests {
    use super::*;
    use crate::model::Spline;
    use kurbo::ParamCurveNearest;

    // The previous importer accepted cubic fits based on seven fixed probes, then bisected the
    // parameter interval. Keep this test-only reference to compare real legacy output with the
    // conservative fit used by craft-dxf.
    fn prior_fit(path: &mut BezPath, bezier: &[Homogeneous], t0: f64, t1: f64, tolerance: f64, depth: u32) -> Option<()> {
        let (p0, d0) = eval_spline_bezier(bezier, t0)?;
        let (p3, d3) = eval_spline_bezier(bezier, t1)?;
        let h = (t1 - t0) / 3.0;
        let (c1, c2) = (p0 + d0 * h, p3 - d3 * h);
        if ![c1, c2].iter().all(|point| point.x.is_finite() && point.y.is_finite()) {
            path.line_to(p3);
            return Some(());
        }
        let cubic = kurbo::CubicBez::new(p0, c1, c2, p3);
        let close = (1..8).all(|i| {
            let t = f64::from(i) / 8.0;
            eval_spline_bezier(bezier, t0 + (t1 - t0) * t).is_some_and(|(point, _)| kurbo::ParamCurve::eval(&cubic, t).distance(point) <= tolerance)
        });
        if close || depth >= MAX_SPLINE_DEPTH || path.elements().len() >= MAX_SPLINE_ELEMENTS {
            path.curve_to(c1, c2, p3);
            return Some(());
        }
        let midpoint = (t0 + t1) / 2.0;
        prior_fit(path, bezier, t0, midpoint, tolerance, depth + 1)?;
        prior_fit(path, bezier, midpoint, t1, tolerance, depth + 1)
    }

    fn legacy_path(bezier: &[Homogeneous], tolerance: f64) -> BezPath {
        let mut path = BezPath::new();
        let start = eval_spline_bezier(bezier, 0.0).expect("legacy start point").0;
        path.move_to(start);
        prior_fit(&mut path, bezier, 0.0, 1.0, tolerance, 0).expect("legacy fit succeeds");
        path
    }

    fn sampled_path_distance(from: &BezPath, to: &BezPath, tolerance: f64) -> f64 {
        let target: Vec<_> = to.segments().collect();
        from.segments()
            .flat_map(|segment| (0..=32).map(move |i| kurbo::ParamCurve::eval(&segment, f64::from(i) / 32.0)))
            .map(|point| target.iter().map(|segment| segment.nearest(point, tolerance / 100.0).distance_sq.sqrt()).fold(f64::INFINITY, f64::min))
            .fold(0.0_f64, f64::max)
    }

    fn cubic() -> Spline {
        Spline {
            degree: 3,
            flags: 8,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control: vec![[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [2.0, 2.0, 0.0], [3.0, 0.0, 0.0]],
            weights: Vec::new(),
            fit: Vec::new(),
            start_tangent: None,
            end_tangent: None,
            fit_tolerance: 0.0,
        }
    }

    fn xyz(points: &[Point]) -> Vec<[f64; 3]> {
        points.iter().map(|point| [point.x, point.y, 0.0]).collect()
    }

    #[test]
    fn spline_path_follows_clamped_control_curve_and_bounds_work() {
        let path = spline_path(&cubic()).unwrap().expect("valid cubic spline");
        assert_eq!(path.elements().len(), 2);
        assert!(matches!(path.elements()[0], kurbo::PathEl::MoveTo(p) if p == Point::new(0.0, 0.0)));
        assert!(matches!(path.elements()[1], kurbo::PathEl::CurveTo(a, b, p)
            if a == Point::new(1.0, 2.0) && b == Point::new(2.0, 2.0) && p == Point::new(3.0, 0.0)));
        assert_eq!(estimated_elements(&crate::model::EntityKind::Spline(cubic())), 0);
    }

    #[test]
    fn rational_closed_periodic_and_fit_point_splines_keep_compatibility() {
        let mut rational_line = Spline {
            degree: 1,
            flags: 4,
            knots: vec![0.0, 0.0, 1.0, 1.0],
            control: vec![[2.0, 3.0, 0.0], [5.0, 7.0, 0.0]],
            weights: vec![1.0, 100.0],
            fit: Vec::new(),
            start_tangent: None,
            end_tangent: None,
            fit_tolerance: 0.0,
        };
        rational_line.flags |= 8;
        let line = spline_path(&rational_line).unwrap().expect("rational degree-one spline");
        assert!(matches!(line.elements(), [kurbo::PathEl::MoveTo(a), kurbo::PathEl::LineTo(b)]
            if *a == Point::new(2.0, 3.0) && *b == Point::new(5.0, 7.0)));

        let mut rational = cubic();
        rational.flags |= 4;
        rational.weights = vec![1.0, 0.5, 0.5, 1.0];
        let rational_path = spline_path(&rational).unwrap().expect("rational spline");
        assert!(rational_path.elements().iter().any(|el| matches!(el, kurbo::PathEl::CurveTo(..))));

        let mut closed = cubic();
        closed.flags |= 1;
        assert!(matches!(spline_path(&closed).unwrap().unwrap().elements().last(), Some(kurbo::PathEl::ClosePath)));
        let mut periodic = cubic();
        periodic.flags |= 2;
        assert!(!matches!(spline_path(&periodic).unwrap().unwrap().elements().last(), Some(kurbo::PathEl::ClosePath)));

        let mut fit = cubic();
        fit.knots.clear();
        fit.fit = vec![[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [3.0, 0.0, 0.0]];
        let path = spline_path(&fit).unwrap().expect("fit-point fallback");
        assert!(matches!(path.elements()[0], kurbo::PathEl::MoveTo(p) if p == Point::new(0.0,0.0)));
        assert!(matches!(path.elements().last(), Some(kurbo::PathEl::CurveTo(_, _, p)) if *p == Point::new(3.0,0.0)));

        let mut negative = cubic();
        negative.degree = -1;
        negative.fit = fit.fit;
        assert!(spline_path(&negative).unwrap().is_some(), "invalid degree reaches the prior fit-point fallback");
    }

    #[test]
    fn spline_path_rejects_malformed_knots_and_defaults_invalid_weights() {
        let mut spline = cubic();
        spline.knots[4] = f64::NAN;
        assert!(spline_path(&spline).unwrap().is_none());
        let mut spline = cubic();
        spline.flags |= 4;
        spline.weights = vec![1.0, 0.0, 1.0, 1.0];
        assert!(spline_path(&spline).unwrap().is_some(), "invalid weights historically fall back to unit weights");
    }

    #[test]
    fn ascii_document_import_keeps_spline_as_supported_geometry() {
        let bytes = b"0\nSECTION\n2\nENTITIES\n0\nSPLINE\n8\n0\n70\n8\n71\n3\n72\n8\n73\n4\n40\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n2\n10\n2\n20\n2\n10\n3\n20\n0\n0\nENDSEC\n0\nEOF\n";
        let imported = import(bytes).expect("valid spline drawing");
        assert_eq!(imported.paths.len(), 1);
        assert!(imported.warnings.is_empty());
        let bounds = imported.bounds.expect("spline bounds");
        assert!((bounds.x0 - 0.0).abs() < 1e-12 && (bounds.x1 - 3.0).abs() < 1e-12);
        assert!((bounds.y0 - 0.0).abs() < 1e-12 && (bounds.y1 - 1.5).abs() < 1e-12);
    }

    #[test]
    fn discontinuous_nurbs_spans_remain_separate_subpaths() {
        let mut limit_hit = false;
        let path = nurbs(
            1,
            &[0.0, 0.0, 0.5, 0.5, 1.0, 1.0],
            &xyz(&[Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(5.0, 5.0), Point::new(6.0, 5.0)]),
            None,
            &mut limit_hit,
        )
        .expect("piecewise linear spline");
        assert!(!limit_hit);
        assert!(matches!(path.elements(), [kurbo::PathEl::MoveTo(a), kurbo::PathEl::LineTo(b), kurbo::PathEl::MoveTo(c), kurbo::PathEl::LineTo(d)]
            if *a == Point::new(0.0,0.0) && *b == Point::new(1.0,0.0) && *c == Point::new(5.0,5.0) && *d == Point::new(6.0,5.0)));
    }

    #[test]
    fn positive_knot_spans_at_large_offsets_are_kept() {
        let start = 1.0e12;
        let delta = 1.0e-3;
        let mut limit_hit = false;
        let path = nurbs(1, &[start, start, start + delta, start + delta], &xyz(&[Point::new(2.0, 3.0), Point::new(5.0, 7.0)]), None, &mut limit_hit)
            .expect("positive-width span");
        assert!(!limit_hit);
        assert!(matches!(path.elements(), [kurbo::PathEl::MoveTo(a), kurbo::PathEl::LineTo(b)]
            if *a == Point::new(2.0, 3.0) && *b == Point::new(5.0, 7.0)));
    }

    #[test]
    fn fit_point_fallback_checks_element_budget_before_collecting() {
        let mut spline = cubic();
        spline.degree = -1;
        spline.fit = vec![[0.0, 0.0, 0.0]; MAX_SPLINE_ELEMENTS + 1];
        assert!(matches!(spline_path(&spline), Err(Error::TooManySegments)));
    }

    #[test]
    fn bernstein_error_bound_covers_rational_and_high_degree_deviations() {
        for control in [
            vec![Homogeneous { x: 0.0, y: 0.0, w: 1.0 }, Homogeneous { x: 0.2, y: 2.0, w: 0.5 }, Homogeneous { x: 1.0, y: 0.0, w: 1.0 }],
            vec![
                Homogeneous { x: 0.0, y: 0.0, w: 1.0 },
                Homogeneous { x: 0.2, y: 2.0, w: 0.7 },
                Homogeneous { x: 0.5, y: -2.0, w: 0.9 },
                Homogeneous { x: 0.8, y: 2.0, w: 1.2 },
                Homogeneous { x: 1.0, y: 0.0, w: 1.0 },
            ],
        ] {
            let degree = control.len() - 1;
            let bezier = [Point::new(0.0, 0.0), Point::new(0.25, 1.0), Point::new(0.75, 1.0), Point::new(1.0, 0.0)];
            let bound = cubic_error_bound(&control, &bezier).unwrap();
            let cubic = kurbo::CubicBez::new(bezier[0], bezier[1], bezier[2], bezier[3]);
            for sample in 0..=1024 {
                let t = sample as f64 / 1024.0;
                let mut points = control.clone();
                for level in (1..=degree).rev() {
                    for i in 0..level {
                        points[i] = points[i].lerp(points[i + 1], t);
                    }
                }
                let source = points[0].point().unwrap();
                let approximation = kurbo::ParamCurve::eval(&cubic, t);
                assert!(source.distance(approximation) <= bound * (1.0 + 1e-12), "sample {t}: bound {bound}");
            }
        }
    }

    #[test]
    fn rational_quadratic_circle_matches_the_prior_importer_tolerance() {
        let weight = std::f64::consts::FRAC_1_SQRT_2;
        let control = xyz(&[(1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (-1.0, 1.0), (-1.0, 0.0), (-1.0, -1.0), (0.0, -1.0), (1.0, -1.0), (1.0, 0.0)]
            .map(|(x, y)| Point::new(200.0 + x * 100.0, 200.0 + y * 100.0)));
        let knots = [0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 4.0];
        let weights = [1.0, weight, 1.0, weight, 1.0, weight, 1.0, weight, 1.0];
        let mut limit_hit = false;
        let path = nurbs(2, &knots, &control, Some(&weights), &mut limit_hit).expect("circle NURBS");
        assert!(!limit_hit);
        let center = Point::new(200.0, 200.0);
        let max_radial_error = path
            .segments()
            .flat_map(|segment| (0..=32).map(move |i| kurbo::ParamCurve::eval(&segment, i as f64 / 32.0)))
            .map(|point| (point.distance(center) - 100.0).abs())
            .fold(0.0_f64, f64::max);
        assert!(max_radial_error < 0.01, "prior importer tolerance was 0.01; got {max_radial_error}");
    }

    #[test]
    fn bounded_spline_fit_matches_prior_rational_and_high_degree_geometry() {
        let weight = std::f64::consts::FRAC_1_SQRT_2;
        let curves = [
            vec![Homogeneous { x: 1.0, y: 0.0, w: 1.0 }, Homogeneous { x: weight, y: weight, w: weight }, Homogeneous { x: 0.0, y: 1.0, w: 1.0 }],
            vec![
                Homogeneous { x: 0.0, y: 0.0, w: 1.0 },
                Homogeneous { x: 0.2, y: 2.0, w: 0.7 },
                Homogeneous { x: 0.5, y: -2.0, w: 0.9 },
                Homogeneous { x: 0.8, y: 2.0, w: 1.2 },
                Homogeneous { x: 1.0, y: 0.0, w: 1.0 },
            ],
        ];
        for bezier in curves {
            let points: Vec<_> = bezier.iter().map(|point| point.point().expect("finite source point")).collect();
            let (lo, hi) = points.iter().fold((points[0], points[0]), |(lo, hi), point| {
                (Point::new(lo.x.min(point.x), lo.y.min(point.y)), Point::new(hi.x.max(point.x), hi.y.max(point.y)))
            });
            let tolerance = ((hi - lo).hypot() * SPLINE_TOLERANCE).max(1e-12);
            let mut current = BezPath::new();
            current.move_to(points[0]);
            let mut limit_hit = false;
            fit_spline(&mut current, &bezier, tolerance, 0, &mut limit_hit).expect("bounded fit succeeds");
            assert!(!limit_hit);
            let prior = legacy_path(&bezier, tolerance);
            let difference = sampled_path_distance(&prior, &current, tolerance).max(sampled_path_distance(&current, &prior, tolerance));
            assert!(difference <= 2.1 * tolerance, "legacy/new Hausdorff sample distance {difference} exceeds 2.1 × tolerance {tolerance}");
        }
    }
}
