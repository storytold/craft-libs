//! What the harness measures of one trace: fidelity, geometric accuracy, economy and artefacts.

use std::collections::HashMap;

use craft_geom::SubPath;
use craft_trace::{Method, Mode, Raster, TraceParams, TraceResult, TracedGradient, TracedPath};
use kurbo::{BezPath, ParamCurveArclen, Point, Shape, Stroke, StrokeOpts};

use crate::inputs::{Class, Input, Variant};
use crate::raster::{coverage_into, polylines, render_painted};

/// sRGB to Oklab (Björn Ottosson, "A perceptual color space for image processing", 2020).
pub fn oklab(c: [u8; 3]) -> [f64; 3] {
    let lin = c.map(|v| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    });
    let l = (0.412_221_470_8 * lin[0] + 0.536_332_536_3 * lin[1] + 0.051_445_992_9 * lin[2]).cbrt();
    let m = (0.211_903_498_2 * lin[0] + 0.680_699_545_1 * lin[1] + 0.107_396_956_6 * lin[2]).cbrt();
    let s = (0.088_302_461_9 * lin[0] + 0.281_718_837_6 * lin[1] + 0.629_978_700_5 * lin[2]).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ]
}

/// Colour difference: Euclidean in Oklab, times 100 (black to white is 100).
pub fn delta(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (oklab(a), oklab(b));
    100.0 * ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn rgb(img: &Raster, x: usize, y: usize) -> [u8; 3] {
    let p = img.pixel(x as u32, y as u32);
    [p[0], p[1], p[2]]
}

/// The mean colour difference between `src` and `out`, over every pixel and weighted by how
/// strong an edge `src` has at the pixel (its lightness gradient).
pub fn fidelity(src: &Raster, out: &Raster) -> (f64, f64) {
    let (w, h) = (src.width as usize, src.height as usize);
    let light: Vec<f64> = (0..w * h).map(|i| oklab(rgb(src, i % w, i / w))[0]).collect();
    let (mut sum, mut edge_sum, mut edge_weight) = (0.0, 0.0, 0.0);
    for y in 0..h {
        for x in 0..w {
            let d = delta(rgb(src, x, y), rgb(out, x, y));
            let at = |x: usize, y: usize| light[y.min(h - 1) * w + x.min(w - 1)];
            let g = (at(x + 1, y) - at(x.saturating_sub(1), y)).hypot(at(x, y + 1) - at(x, y.saturating_sub(1)));
            sum += d;
            edge_sum += d * g;
            edge_weight += g;
        }
    }
    (sum / (w * h) as f64, if edge_weight > 0.0 { edge_sum / edge_weight } else { 0.0 })
}

/// A traced path as the shape it paints: stroked paths become their stroke's outline.
pub(super) fn painted(tp: &TracedPath) -> BezPath {
    let bez = tp.path.to_bezpath();
    match tp.stroke {
        Some(width) => {
            let cap = if tp.square_caps { kurbo::Cap::Square } else { kurbo::Cap::Round };
            kurbo::stroke(bez, &Stroke::new(width).with_caps(cap), &StrokeOpts::default(), 0.05)
        }
        None => bez,
    }
}

/// Outlines as short segments in a grid, for nearest-distance queries.
pub struct Outline {
    segs: Vec<(Point, Point)>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

const CELL: f64 = 4.0;

fn seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let l2 = ab.hypot2();
    let t = if l2 > 0.0 { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

impl Outline {
    /// The outlines of `paths`, cut into pieces at most half a pixel long. With `inside`, pieces
    /// within a pixel of the edge of an image that size are left out (the background's frame).
    pub fn new<'a>(paths: impl IntoIterator<Item = &'a BezPath>, inside: Option<f64>) -> Self {
        let mut out = Self { segs: vec![], grid: HashMap::new() };
        for path in paths {
            for line in polylines(path, 0.02) {
                for i in 0..line.len() {
                    let (a, b) = (line[i], line[(i + 1) % line.len()]);
                    let n = (a.distance(b) / 0.5).ceil().max(1.0) as usize;
                    for k in 0..n {
                        let (p, q) = (a.lerp(b, k as f64 / n as f64), a.lerp(b, (k + 1) as f64 / n as f64));
                        let m = p.midpoint(q);
                        if inside.is_some_and(|s| m.x < 1.0 || m.y < 1.0 || m.x > s - 1.0 || m.y > s - 1.0) || p == q {
                            continue;
                        }
                        out.grid.entry(cell(m)).or_default().push(out.segs.len() as u32);
                        out.segs.push((p, q));
                    }
                }
            }
        }
        out
    }
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }
    /// The distance from `p` to the nearest outline (infinite when there is none).
    pub fn dist(&self, p: Point) -> f64 {
        let (cx, cy) = cell(p);
        let mut best = f64::INFINITY;
        for dy in -1..=1 {
            for dx in -1..=1 {
                for &i in self.grid.get(&(cx + dx, cy + dy)).into_iter().flatten() {
                    let (a, b) = self.segs[i as usize];
                    best = best.min(seg_dist(p, a, b));
                }
            }
        }
        // Anything nearer than a found piece under 3.5 px away is in the cells looked at.
        if best > 3.5 {
            best = self.segs.iter().map(|&(a, b)| seg_dist(p, a, b)).fold(f64::INFINITY, f64::min);
        }
        best
    }
    /// (mean, max) distance from this outline's pieces to `other`.
    pub fn distance_to(&self, other: &Outline) -> (f64, f64) {
        if self.is_empty() {
            return (f64::NAN, f64::NAN);
        }
        let (mut sum, mut max) = (0.0, 0.0f64);
        for &(a, b) in &self.segs {
            let d = other.dist(a.midpoint(b));
            sum += d;
            max = max.max(d);
        }
        (sum / self.segs.len() as f64, max)
    }
}

fn cell(p: Point) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

/// The angle (degrees) the path turns by at anchor `i` of `sp`, or `None` at an open end.
fn turn(sp: &SubPath, i: usize) -> Option<f64> {
    let n = sp.anchors.len();
    if n < 2 || (!sp.closed && (i == 0 || i + 1 == n)) {
        return None;
    }
    let (before, after) = (sp.segment((i + n - 1) % n), sp.segment(i));
    let first = |v: [kurbo::Vec2; 3]| v.into_iter().find(|v| v.hypot() > 1e-9);
    let a = first([before.p3 - before.p2, before.p3 - before.p1, before.p3 - before.p0])?;
    let b = first([after.p1 - after.p0, after.p2 - after.p0, after.p3 - after.p0])?;
    Some(a.cross(b).atan2(a.dot(b)).abs().to_degrees())
}

/// Everything measured of one trace. `NAN` marks what does not apply.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Mean colour difference of the trace drawn again, against the source.
    pub delta: f64,
    /// The same, weighted by the source's edge strength.
    pub edge_delta: f64,
    /// Mean and largest distance (px) from the traced outlines to the true ones.
    pub geo_mean: f64,
    pub geo_max: f64,
    /// Largest distance (px) from a true outline to the nearest traced one: what was missed.
    pub miss_max: f64,
    /// Paths and anchor points, the background's frame aside.
    pub paths: f64,
    pub anchors: f64,
    /// Anchors per anchor of the art itself.
    pub anchor_ratio: f64,
    /// Share (%) of the outline length made of straight segments.
    pub lines: f64,
    /// Share (%) of the outline length that is exactly level or upright.
    pub upright: f64,
    /// Share (%) of the art's sharp corners that have a traced corner within 1.5 px.
    pub corners: f64,
    /// Anchors where the path turns by 10° or more away from any true corner.
    pub kinks: f64,
    /// Paths (and their area, px) in a colour the art doesn't have.
    pub slivers: f64,
    pub sliver_area: f64,
    /// Area (px) no path covers, and area covered twice.
    pub gap: f64,
    pub overlap: f64,
    /// Stroked centre lines: their mean distance (px) from the true line's centre line, the mean
    /// error (px) of their widths, and anchors per stroked path.
    pub centre_off: f64,
    pub width_off: f64,
    pub stroke_anchors: f64,
    /// Share (%) of the art's plain shapes that came out as a shape lying within 0.6 px of it.
    pub shapes: f64,
    /// Shapes: paths made a shape that lie more than 0.6 px off the art.
    pub false_shapes: f64,
    /// Auto Grouping: share (%) of the paths that lie inside another.
    pub grouped: f64,
    /// Gradients: paths filled with one; and where the input is a known gradient, whether one was
    /// found (%), the angle (°) between its line and the largest traced gradient's, and the largest
    /// difference (levels of a channel) between their colours along the true line.
    pub gradients: f64,
    pub ramp_found: f64,
    pub ramp_angle: f64,
    pub ramp_off: f64,
    /// Paths under 6 px of area.
    pub specks: f64,
    /// How far (px) the trace reaches outside the image: a stray curve.
    pub outside: f64,
}

/// The order a grouped result is painted in: each path together with those inside it, in the
/// order they were traced in (as the app builds its groups).
fn draw_order(res: &TraceResult) -> Vec<usize> {
    fn draw(i: usize, inside: &[Vec<usize>], order: &mut Vec<usize>) {
        let below = inside[i].partition_point(|&c| c < i);
        inside[i][..below].iter().for_each(|&c| draw(c, inside, order));
        order.push(i);
        inside[i][below..].iter().for_each(|&c| draw(c, inside, order));
    }
    let mut inside = vec![vec![]; res.paths.len()];
    let mut roots = vec![];
    for i in 0..res.paths.len() {
        match res.parents.get(i).copied().flatten() {
            Some(p) => inside[p].push(i),
            None => roots.push(i),
        }
    }
    let mut order = vec![];
    roots.into_iter().for_each(|i| draw(i, &inside, &mut order));
    order
}

/// Measure `res`, the trace of `src` (`input` drawn `size` px square as `variant`) with `params`.
pub fn measure(input: &Input, variant: Variant, size: u32, src: &Raster, params: &TraceParams, res: &TraceResult) -> Metrics {
    let n = size as usize;
    let shapes: Vec<(BezPath, [u8; 3])> = res.paths.iter().map(|tp| (painted(tp), tp.color)).collect();
    // Painted as the result is grouped.
    let drawn: Vec<(BezPath, [u8; 3], Option<&TracedGradient>)> =
        draw_order(res).into_iter().map(|i| (shapes[i].0.clone(), shapes[i].1, res.paths[i].gradient.as_ref())).collect();
    let (delta_mean, edge_delta) = fidelity(src, &render_painted(&drawn, [255; 3], n, n));
    // Gradients: how many paths have one; and the largest of them against the gradient the input
    // is: the angle between their lines, and the largest difference of a channel between their
    // colours at 17 points along the true line.
    let gradients = res.paths.iter().filter(|tp| tp.gradient.is_some()).count() as f64;
    let (mut ramp_found, mut ramp_angle, mut ramp_off) = (f64::NAN, f64::NAN, f64::NAN);
    if let Some(ramp) = &input.ramp {
        let k = f64::from(size) / 256.0;
        let found = res.paths.iter().filter(|tp| tp.gradient.is_some()).max_by_key(|tp| tp.pixels).and_then(|tp| tp.gradient.as_ref());
        ramp_found = if found.is_some() { 100.0 } else { 0.0 };
        if let Some(g) = found {
            let (truth, own) = (ramp.end - ramp.start, g.end - g.start);
            let between = (truth.atan2() - own.atan2()).to_degrees().rem_euclid(180.0);
            ramp_angle = between.min(180.0 - between);
            ramp_off = (0..=16)
                .map(|j| {
                    let p = ramp.start + truth * (f64::from(j) / 16.0);
                    // (The true colour as this variant shows it: a scan's are its own.)
                    let a = variant.color(ramp.color_at(p).map(|v| v.round() as u8)).map(f64::from);
                    let b = g.color_at((p.to_vec2() * k).to_point());
                    (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f64::max)
                })
                .fold(0.0, f64::max);
        }
    }
    let grouped = if res.parents.is_empty() { f64::NAN } else { 100.0 * res.parents.iter().flatten().count() as f64 / res.paths.len().max(1) as f64 };

    // The background's frame: a path as large as the image.
    let is_frame = |p: &BezPath| {
        let b = p.bounding_box();
        b.width() >= 0.98 * f64::from(size) && b.height() >= 0.98 * f64::from(size)
    };
    let art: Vec<(&TracedPath, &BezPath)> = res.paths.iter().zip(&shapes).filter(|(_, (p, _))| !is_frame(p)).map(|(tp, (p, _))| (tp, p)).collect();
    let anchors: usize = art.iter().map(|(tp, _)| tp.path.anchor_count()).sum();

    let nan = f64::NAN;
    let (mut geo_mean, mut geo_max, mut miss_max, mut corners, mut kinks, mut slivers, mut sliver_area) = (nan, nan, nan, nan, nan, nan, nan);
    let mut false_shapes = if params.shapes { 0.0 } else { nan };
    if !matches!(input.class, Class::Tone | Class::Ramp) {
        let truth_shapes = input.scaled(size);
        let truth = Outline::new(truth_shapes.iter().map(|(p, _)| p), None);
        let traced = Outline::new(shapes.iter().map(|(p, _)| p), Some(f64::from(size)));
        (geo_mean, geo_max) = traced.distance_to(&truth);
        // Shapes that are not the art's: a path made a shape that strays more than 0.6 px from it.
        false_shapes =
            res.paths.iter().filter(|tp| tp.shape.is_some() && Outline::new([&tp.path.to_bezpath()], None).distance_to(&truth).1 > 0.6).count()
                as f64;
        miss_max = truth.distance_to(&traced).1;

        let true_corners = input.scaled_corners(size);
        let turns: Vec<(Point, f64)> = art
            .iter()
            .flat_map(|(tp, _)| &tp.path.subpaths)
            .flat_map(|sp| (0..sp.anchors.len()).filter_map(move |i| Some((sp.anchors[i].p, turn(sp, i)?))))
            .collect();
        if !true_corners.is_empty() {
            let kept = true_corners.iter().filter(|c| turns.iter().any(|(p, t)| *t >= 20.0 && p.distance(**c) <= 1.5)).count();
            corners = 100.0 * kept as f64 / true_corners.len() as f64;
        }
        if !matches!(input.class, Class::Lines | Class::Askew | Class::Plain) {
            kinks = turns.iter().filter(|(p, t)| *t >= 10.0 && true_corners.iter().all(|c| p.distance(*c) > 2.0)).count() as f64;
        }
        if params.mode != Mode::BlackAndWhite {
            let colors: Vec<[u8; 3]> = truth_shapes.iter().map(|(_, c)| *c).chain([input.bg]).map(|c| variant.color(c)).collect();
            let odd: Vec<f64> = shapes.iter().filter(|(_, c)| colors.iter().all(|t| delta(*t, *c) > 10.0)).map(|(p, _)| p.area().abs()).collect();
            (slivers, sliver_area) = (odd.len() as f64, odd.iter().sum());
        }
    }

    let (mut length, mut straight, mut upright) = (0.0, 0.0, 0.0);
    for sp in art.iter().flat_map(|(tp, _)| &tp.path.subpaths) {
        for i in 0..sp.segment_count() {
            let len = sp.segment(i).arclen(0.05);
            length += len;
            // Straight is judged by the geometry: a line may be stored with handles along it.
            let c = sp.segment(i);
            if seg_dist(c.p1, c.p0, c.p3) < 0.02 && seg_dist(c.p2, c.p0, c.p3) < 0.02 {
                straight += len;
                if (c.p0.x - c.p3.x).abs() < 1e-9 || (c.p0.y - c.p3.y).abs() < 1e-9 {
                    upright += len;
                }
            }
        }
    }

    // Coverage summed over the paths: under 1 is a gap, over 1 an overlap.
    let (mut gap, mut overlap) = (nan, nan);
    if !params.ignore_white {
        let (mut sum, mut cov) = (vec![0.0f32; n * n], vec![0.0f32; n * n]);
        for (p, _) in &shapes {
            let (r0, r1) = coverage_into(p, n, n, &mut cov);
            for i in r0 * n..r1 * n {
                sum[i] += cov[i].min(1.0);
                cov[i] = 0.0;
            }
        }
        gap = sum.iter().map(|&s| f64::from((1.0 - s).max(0.0))).sum();
        // Overlapping layers and Flat Logo's shapes are stacked on purpose.
        if params.method == Method::Abutting && params.mode != Mode::Logo {
            overlap = sum.iter().map(|&s| f64::from((s - 1.0).max(0.0))).sum();
        }
    }

    // Stroked paths against the lines they trace: each against the true line nearest its middle.
    let (mut centre_off, mut width_off, mut stroke_anchors) = (nan, nan, nan);
    let truth = input.scaled_strokes(size);
    let stroked: Vec<&TracedPath> = res.paths.iter().filter(|tp| tp.stroke.is_some()).collect();
    if !truth.is_empty() && !stroked.is_empty() {
        let (mut off, mut samples, mut width) = (0.0, 0.0, 0.0);
        for tp in &stroked {
            let line = Outline::new([&tp.path.to_bezpath()], None);
            let Some(middle) = line.segs.get(line.segs.len() / 2).map(|s| s.0.midpoint(s.1)) else { continue };
            let Some((a, b, w)) = truth.iter().copied().min_by(|p, q| seg_dist(middle, p.0, p.1).total_cmp(&seg_dist(middle, q.0, q.1))) else {
                continue;
            };
            for s in &line.segs {
                // The distance to the line through the true ends: a centre line may be shorter or
                // longer than the true one by how its caps are drawn.
                let d = b - a;
                off += (s.0.midpoint(s.1) - a).cross(d).abs() / d.hypot().max(1e-9);
                samples += 1.0;
            }
            width += (tp.stroke.unwrap_or(0.0) - w).abs();
        }
        if samples > 0.0 {
            (centre_off, width_off) = (off / samples, width / stroked.len() as f64);
            stroke_anchors = stroked.iter().map(|tp| tp.path.anchor_count()).sum::<usize>() as f64 / stroked.len() as f64;
        }
    }

    // Plain shapes: each is found when some path that is a shape lies on it.
    let plain = input.scaled_plain(size);
    let mut shapes_found = nan;
    if !plain.is_empty() {
        let made: Vec<Outline> = res.paths.iter().filter(|tp| tp.shape.is_some()).map(|tp| Outline::new([&tp.path.to_bezpath()], None)).collect();
        let found = plain.iter().filter(|p| {
            let truth = Outline::new([*p], None);
            made.iter().any(|m| m.distance_to(&truth).1 <= 0.6 && truth.distance_to(m).1 <= 0.6)
        });
        shapes_found = 100.0 * found.count() as f64 / plain.len() as f64;
    }

    Metrics {
        grouped,
        gradients,
        ramp_found,
        ramp_angle,
        ramp_off,
        shapes: shapes_found,
        false_shapes,
        centre_off,
        width_off,
        stroke_anchors,
        delta: delta_mean,
        edge_delta,
        geo_mean,
        geo_max,
        miss_max,
        paths: art.len() as f64,
        anchors: anchors as f64,
        anchor_ratio: if input.anchors > 0 { anchors as f64 / input.anchors as f64 } else { nan },
        lines: if length > 0.0 { 100.0 * straight / length } else { nan },
        upright: if length > 0.0 { 100.0 * upright / length } else { nan },
        corners,
        kinks,
        slivers,
        sliver_area,
        gap,
        overlap,
        specks: art.iter().filter(|(_, p)| p.area().abs() < 6.0).count() as f64,
        outside: shapes
            .iter()
            .map(|(p, _)| p.bounding_box())
            .filter(|b| b.width() > 0.0 || b.height() > 0.0)
            .map(|b| (-b.x0).max(-b.y0).max(b.x1 - f64::from(size)).max(b.y1 - f64::from(size)).max(0.0))
            .fold(0.0, f64::max),
    }
}
