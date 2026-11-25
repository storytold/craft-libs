//! Centre-line measurements on code-drawn scenes. No external images or tracer required.
use super::centre_geom::{P, seg_dist};
use super::centre_scenes::{self as scenes, Scene};
use craft_trace::{Raster, TraceParams, TraceResult, trace_within};
use kurbo::{BezPath, ParamCurve, PathSeg};
use std::fmt::Write as _;
use std::time::Instant;

/// Nearest-point queries over a point set (uniform grid buckets).
struct Near {
    cell: f64,
    gw: usize,
    gh: usize,
    buckets: Vec<Vec<usize>>,
    pts: Vec<P>,
}

impl Near {
    fn new(pts: Vec<P>, w: usize, h: usize, cell: f64) -> Self {
        let gw = (w as f64 / cell).ceil() as usize + 2;
        let gh = (h as f64 / cell).ceil() as usize + 2;
        let mut buckets = vec![vec![]; gw * gh];
        for (i, p) in pts.iter().enumerate() {
            let (cx, cy) = (((p.x / cell).floor().max(0.0) as usize).min(gw - 1), ((p.y / cell).floor().max(0.0) as usize).min(gh - 1));
            buckets[cy * gw + cx].push(i);
        }
        Self { cell, gw, gh, buckets, pts }
    }
    /// Nearest point within `max` (index, distance).
    fn nearest(&self, q: P, max: f64) -> Option<(usize, f64)> {
        let r = (max / self.cell).ceil() as i64;
        let (cx, cy) = ((q.x / self.cell).floor() as i64, (q.y / self.cell).floor() as i64);
        let mut best: Option<(usize, f64)> = None;
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                if x < 0 || y < 0 || x >= self.gw as i64 || y >= self.gh as i64 {
                    continue;
                }
                for &i in &self.buckets[y as usize * self.gw + x as usize] {
                    let d = self.pts[i].dist(q);
                    if d <= max && best.is_none_or(|b| d < b.1) {
                        best = Some((i, d));
                    }
                }
            }
        }
        best
    }
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 }
}
fn p95(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((s.len() - 1) as f64 * 0.95).round() as usize]
}

#[derive(Default, Clone, Debug)]
struct Metrics {
    name: String,
    input: &'static str,
    strokes: usize,
    fills: usize,
    anchors: usize,
    ms: f64,
    c_mean: f64,
    c_p95: f64,
    g_mean: f64,
    g_p95: f64,
    complete: f64,
    w_uni: f64,
    w_uni_p95: f64,
    junc_total: usize,
    junc_ok: usize,
    junc_wrong: usize,
    junc_extra: usize,
    junc_pos: f64,
    ends_gt: usize,
    ends_found: usize,
    end_err: f64,
    end_signed: f64,
    spurs: usize,
    iou_uni: f64,
    area_uni: f64,
    fill_as_fill: f64,
}

fn iou(a: &[bool], m: &[bool]) -> (f64, f64) {
    let (mut i, mut u, mut ra, mut rm) = (0usize, 0usize, 0usize, 0usize);
    for (x, y) in a.iter().zip(m) {
        i += usize::from(*x && *y);
        u += usize::from(*x || *y);
        ra += usize::from(*x);
        rm += usize::from(*y);
    }
    (i as f64 / u.max(1) as f64, (ra as f64 - rm as f64) / rm.max(1) as f64)
}

fn evaluate(sc: &Scene, input: &'static str, cov: &[f32]) -> Metrics {
    let rgba = cov
        .iter()
        .flat_map(|&c| {
            let v = ((1.0 - c) * 255.0).round() as u8;
            [v, v, v, 255]
        })
        .collect();
    let img = Raster::new(sc.w as u32, sc.h as u32, rgba).unwrap();
    let params = TraceParams { strokes: true, fills: true, stroke_width: sc.wmax, ignore_white: true, noise: 4, ..TraceParams::default() };
    let t0 = Instant::now();
    let res = trace_within(&img, &params, super::MAX_ANCHORS).unwrap();
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    measure(sc, input, cov, &res, ms)
}

fn measure(sc: &Scene, input: &'static str, cov: &[f32], res: &TraceResult, ms: f64) -> Metrics {
    let t = geometry(res);
    let in_fill = |q: P, grow: f64| sc.fills.iter().any(|f| f.dist(q) <= grow);
    // Ground-truth centre samples (outside fills) and output samples.
    let mut gt_pts = vec![];
    let mut gt_w = vec![];
    for s in &sc.strokes {
        for (p, w) in s.pts.iter().zip(&s.w) {
            if !in_fill(*p, 0.0) {
                gt_pts.push(*p);
                gt_w.push(*w);
            }
        }
    }
    let gt = Near::new(gt_pts.clone(), sc.w, sc.h, 4.0);
    let mut out_pts = vec![];
    let (mut cd, mut wu) = (vec![], vec![]);
    for s in &t.strokes {
        for (p, _) in &s.samples {
            if in_fill(*p, 0.0) {
                continue;
            }
            out_pts.push(*p);
            if let Some((i, d)) = gt.nearest(*p, 30.0) {
                cd.push(d);
                // Width: only where the sample is on a stroke (not a stray piece).
                if d < 2.0 {
                    wu.push((s.width - gt_w[i]).abs());
                }
            } else {
                cd.push(30.0);
            }
        }
    }
    let out = Near::new(out_pts, sc.w, sc.h, 4.0);
    let gd: Vec<f64> = gt_pts.iter().map(|&p| out.nearest(p, 30.0).map_or(30.0, |x| x.1)).collect();
    let complete = gd.iter().filter(|&&d| d <= 1.5).count() as f64 / gd.len().max(1) as f64;
    // Junctions (away from fills).
    let outj: Vec<_> = t.junctions.iter().filter(|j| !in_fill(j.pos, 2.0)).collect();
    let mut used = vec![false; outj.len()];
    let (mut ok, mut wrong, mut pos) = (0, 0, vec![]);
    for &(p, deg) in &sc.junctions {
        let wl = gt.nearest(p, 5.0).map_or(4.0, |(i, _)| gt_w[i]);
        let tol = wl + 3.0;
        let best =
            outj.iter().enumerate().filter(|(k, j)| !used[*k] && j.pos.dist(p) <= tol).min_by(|a, b| a.1.pos.dist(p).total_cmp(&b.1.pos.dist(p)));
        if let Some((k, j)) = best {
            used[k] = true;
            pos.push(j.pos.dist(p));
            if j.degree == deg {
                ok += 1;
            } else {
                wrong += 1;
            }
        }
    }
    let extra = used.iter().filter(|u| !**u).count();
    // Free ends: ground truth = open stroke ends not at a junction, corner or fill.
    let mut gt_ends = vec![];
    for s in sc.strokes.iter().filter(|s| !s.closed) {
        for (p, w) in [(s.pts[0], s.w[0]), (*s.pts.last().unwrap_or(&s.pts[0]), *s.w.last().unwrap_or(&s.w[0]))] {
            let near_j = sc.junctions.iter().any(|(q, _)| q.dist(p) < 1.0) || sc.corners.iter().any(|q| q.dist(p) < 1.0);
            let on_other = sc.strokes.iter().filter(|o| !std::ptr::eq(*o, s)).any(|o| o.pts.iter().any(|q| q.dist(p) < 0.5));
            if !near_j && !on_other && !in_fill(p, w) {
                // Its direction (outwards), for the signed error.
                let k =
                    if p == s.pts[0] { (s.pts[0] - s.pts[s.pts.len().min(9) - 1]).unit() } else { (p - s.pts[s.pts.len().saturating_sub(9)]).unit() };
                gt_ends.push((p, w, k.unwrap_or(P::new(1.0, 0.0))));
            }
        }
    }
    let out_ends: Vec<P> = t.ends.iter().copied().filter(|p| !in_fill(*p, 2.0)).collect();
    let mut used_e = vec![false; out_ends.len()];
    let (mut found, mut err, mut signed) = (0, vec![], vec![]);
    for &(p, w, dir) in &gt_ends {
        let tol = 0.5 * w + 4.0;
        let best = out_ends.iter().enumerate().filter(|(k, q)| !used_e[*k] && q.dist(p) <= tol).min_by(|a, b| a.1.dist(p).total_cmp(&b.1.dist(p)));
        if let Some((k, q)) = best {
            used_e[k] = true;
            found += 1;
            err.push(q.dist(p));
            signed.push((*q - p).dot(dir));
        }
    }
    let spurs = used_e.iter().filter(|u| !**u).count();
    let r_uni = render(res, sc.w, sc.h, false);
    // Coverage against the exact shape (pixel centres inside it); the thresholded mask
    // over-counts pixels exactly half covered.
    let exact = sc.exact();
    let (iou_uni, area_uni) = iou(&r_uni, &exact);
    // Fill pixels traced as fills.
    let rf = render(res, sc.w, sc.h, true);
    let (mut fp, mut ff) = (0usize, 0usize);
    for y in 0..sc.h {
        for x in 0..sc.w {
            let q = P::new(x as f64 + 0.5, y as f64 + 0.5);
            if cov[y * sc.w + x] >= 0.5 && sc.fills.iter().any(|f| f.inside(q)) {
                fp += 1;
                ff += usize::from(rf[y * sc.w + x]);
            }
        }
    }
    Metrics {
        name: sc.name.clone(),
        input,
        strokes: res.paths.iter().filter(|p| p.stroke.is_some()).count(),
        fills: t.fills.len(),
        anchors: res.paths.iter().filter(|p| p.stroke.is_some()).map(|p| p.path.anchor_count()).sum(),
        ms,
        c_mean: mean(&cd),
        c_p95: p95(&cd),
        g_mean: mean(&gd),
        g_p95: p95(&gd),
        complete,
        w_uni: mean(&wu),
        w_uni_p95: p95(&wu),
        junc_total: sc.junctions.len(),
        junc_ok: ok,
        junc_wrong: wrong,
        junc_extra: extra,
        junc_pos: mean(&pos),
        ends_gt: gt_ends.len(),
        ends_found: found,
        end_err: mean(&err),
        end_signed: mean(&signed),
        spurs,
        iou_uni,
        area_uni,
        fill_as_fill: if fp > 0 { ff as f64 / fp as f64 } else { f64::NAN },
    }
}

struct StrokeSamples {
    samples: Vec<(P, f64)>,
    closed: bool,
    width: f64,
}
struct Junction {
    pos: P,
    degree: usize,
}
struct Geometry {
    strokes: Vec<StrokeSamples>,
    fills: Vec<()>,
    junctions: Vec<Junction>,
    ends: Vec<P>,
}

fn point(p: kurbo::Point) -> P {
    P::new(p.x, p.y)
}

/// Evaluate the emitted curves at <=0.5 px control-polygon steps (as for the scene
/// yardstick). Keep subpaths separate: today's compound strokes share one width.
fn samples(path: &BezPath) -> Vec<P> {
    let mut pts = vec![];
    for seg in path.segments() {
        let poly = match seg {
            PathSeg::Line(l) => l.p0.distance(l.p1),
            PathSeg::Quad(q) => q.p0.distance(q.p1) + q.p1.distance(q.p2),
            PathSeg::Cubic(c) => c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3),
        };
        let n = (poly / 0.5).ceil().max(1.0) as usize;
        if pts.is_empty() {
            pts.push(point(seg.eval(0.0)));
        }
        for i in 1..=n {
            pts.push(point(seg.eval(i as f64 / n as f64)));
        }
    }
    pts
}

/// Recover topology from emitted geometry, without consulting scene truth. Candidates
/// are open endpoints and intersections. Coincident points merge within 0.05 px;
/// each touching endpoint contributes one arm, each interior contributes two.
/// This also measures a bar chained through a T, or two chained strokes crossing.
fn geometry(res: &TraceResult) -> Geometry {
    let mut strokes = vec![];
    for path in &res.paths {
        if let Some(width) = path.stroke {
            for sp in &path.path.subpaths {
                let mut bez = BezPath::new();
                sp.to_bezpath_into(&mut bez);
                strokes.push(StrokeSamples { samples: samples(&bez).into_iter().map(|p| (p, width)).collect(), closed: sp.closed, width });
            }
        }
    }
    let mut candidates = vec![];
    for s in &strokes {
        if !s.closed && !s.samples.is_empty() {
            candidates.extend([s.samples[0].0, s.samples.last().unwrap().0]);
        }
    }
    for (i, a) in strokes.iter().enumerate() {
        for b in &strokes[i + 1..] {
            for aa in a.samples.windows(2) {
                let (p, r) = (aa[0].0, aa[1].0 - aa[0].0);
                for bb in b.samples.windows(2) {
                    let (q, s) = (bb[0].0, bb[1].0 - bb[0].0);
                    if aa[0].0.x.max(aa[1].0.x) < bb[0].0.x.min(bb[1].0.x)
                        || bb[0].0.x.max(bb[1].0.x) < aa[0].0.x.min(aa[1].0.x)
                        || aa[0].0.y.max(aa[1].0.y) < bb[0].0.y.min(bb[1].0.y)
                        || bb[0].0.y.max(bb[1].0.y) < aa[0].0.y.min(aa[1].0.y)
                    {
                        continue;
                    }
                    let cross = |v: P, w: P| v.x * w.y - v.y * w.x;
                    let d = cross(r, s);
                    if d.abs() < 1e-12 {
                        continue;
                    }
                    let t = cross(q - p, s) / d;
                    let u = cross(q - p, r) / d;
                    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                        candidates.push(p + r * t);
                    }
                }
            }
        }
    }
    let mut nodes: Vec<P> = vec![];
    for p in candidates {
        if !nodes.iter().any(|q| q.dist(p) <= 0.05) {
            nodes.push(p);
        }
    }
    let (mut junctions, mut ends) = (vec![], vec![]);
    for pos in nodes {
        let degree = strokes
            .iter()
            .map(|s| {
                if s.samples.is_empty() {
                    return 0;
                }
                if !s.closed {
                    // A run returning to its junction contributes both endpoint arms.
                    let arms = usize::from(pos.dist(s.samples[0].0) <= 0.05) + usize::from(pos.dist(s.samples.last().unwrap().0) <= 0.05);
                    if arms > 0 {
                        return arms;
                    }
                }
                if s.samples.windows(2).any(|p| seg_dist(pos, p[0].0, p[1].0).0 <= 0.05) { 2 } else { 0 }
            })
            .sum();
        if degree >= 3 {
            junctions.push(Junction { pos, degree });
        }
        if degree == 1 {
            ends.push(pos);
        }
    }
    Geometry { strokes, fills: vec![(); res.paths.iter().filter(|p| p.stroke.is_none()).count()], junctions, ends }
}

/// Pixel-centre nonzero-winding fill of the actual painted paths. Strokes retain
/// their emitted square/round caps. Overlapping paths are unioned, never cancelled.
fn render(res: &TraceResult, w: usize, h: usize, fills_only: bool) -> Vec<bool> {
    let mut cov = vec![false; w * h];
    for path in &res.paths {
        if fills_only && path.stroke.is_some() {
            continue;
        }
        let bez = super::measure::painted(path);
        let polys = super::raster::polylines(&bez, 0.02);
        let mut xs = vec![];
        for y in 0..h {
            let yc = y as f64 + 0.5;
            xs.clear();
            for poly in &polys {
                for i in 0..poly.len() {
                    let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                    if (a.y <= yc) != (b.y <= yc) {
                        xs.push((a.x + (b.x - a.x) * (yc - a.y) / (b.y - a.y), if b.y > a.y { 1 } else { -1 }));
                    }
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0;
            for pair in xs.windows(2) {
                wind += pair[0].1;
                if wind != 0 {
                    let xa = (pair[0].0 - 0.5).ceil().max(0.0) as usize;
                    let xb = ((pair[1].0 - 0.5).floor() + 1.0).max(0.0) as usize;
                    for x in xa.min(w)..xb.min(w) {
                        cov[y * w + x] = true;
                    }
                }
            }
        }
    }
    cov
}

fn f(v: f64, d: usize) -> String {
    if v.is_nan() { "–".into() } else { format!("{v:.d$}") }
}

pub fn report() -> String {
    let mut rows = vec![];
    for sc in scenes::all() {
        let cov = sc.rasterise();
        let hard: Vec<f32> = cov.iter().map(|&c| if c >= 0.5 { 1.0 } else { 0.0 }).collect();
        for (input, c) in [("aa", &cov), ("hard", &hard)] {
            rows.push(evaluate(&sc, input, c));
        }
    }
    let mut md = String::from(
        "## Centre lines\n\nCode-drawn scenes only: 24 scenes, each anti-aliased (8×8 coverage samples) and hard (that coverage thresholded at 0.5). Coordinates, widths and scene Stroke values are fixed; junction/letter/crossing/mixed-width body coordinates include the 0.27/0.41 px offset. Create Fills and Strokes are on, Stroke is the scene's limit (13 px, mix and mixed-width body 6 px, varying 12 px), Black and White threshold 128, Noise 4, Ignore White on, other settings at their defaults. Today's tracer has no minimum-length setting (the dashed scenes specify 0 for future use).\n\nDistances use dense exact centre samples (≤0.25 px), emitted curves sampled at ≤0.5 px control-polygon steps, and nearest-point searches capped at 30 px. Found is the fraction of true samples within 1.5 px; width errors use output samples within 2 px of a true stroke. Samples inside exact fills are excluded. Topology is recovered from emitted subpath endpoints and intersections, merging coincident positions within 0.05 px; an endpoint contributes one arm, an interior two. Junction matching radius is local true width + 3 px; ends use half-width + 4 px. Each match is used once. Free ends exclude true junctions, corners, other strokes and fills; unmatched emitted ends are spurs. Junctions/ends within 2 px of fills are excluded. End error is distance, with signed error along the outward tangent in parentheses. Coverage is a union of painted output at pixel centres (0.02 px flattening tolerance, actual emitted caps), compared with the exact swept shape at pixel centres, for both input variants. Area error is signed relative to exact area. Anchors counts stroke anchors, including all compound subpaths; paths counts stroked + filled paths. Time covers tracing only, one scene at a time. Profile-dependent columns are unavailable (–).\n\n### Centre line and width (px)\n\n| scene | input | paths | out→true mean / p95 | true→out mean / p95 | found % | width err uniform mean / p95 | variable mean / p95 | profile mean |\n|---|---|---|---|---|---|---|---|---|\n",
    );
    for m in &rows {
        writeln!(
            md,
            "| {} | {} | {}+{} | {} / {} | {} / {} | {} | {} / {} | – | – |",
            m.name,
            m.input,
            m.strokes,
            m.fills,
            f(m.c_mean, 2),
            f(m.c_p95, 2),
            f(m.g_mean, 2),
            f(m.g_p95, 2),
            f(100.0 * m.complete, 1),
            f(m.w_uni, 2),
            f(m.w_uni_p95, 2)
        )
        .unwrap();
    }
    md += "\n### Topology, ends, coverage\n\n| scene | input | junctions ok / wrong deg / missed / extra | junction pos err | ends found | end err (signed) | spurs | IoU uniform / profile | area err uniform / profile | fill→fill % | anchors | width pts | ms |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|\n";
    for m in &rows {
        writeln!(
            md,
            "| {} | {} | {} / {} / {} / {} | {} | {}/{} | {} ({}) | {} | {} / – | {}% / – | {} | {} | – | {} |",
            m.name,
            m.input,
            m.junc_ok,
            m.junc_wrong,
            m.junc_total - m.junc_ok - m.junc_wrong,
            m.junc_extra,
            f(m.junc_pos, 2),
            m.ends_found,
            m.ends_gt,
            f(m.end_err, 2),
            f(m.end_signed, 2),
            m.spurs,
            f(m.iou_uni, 3),
            f(100.0 * m.area_uni, 1),
            f(100.0 * m.fill_as_fill, 1),
            m.anchors,
            f(m.ms, 1)
        )
        .unwrap();
    }
    let sum = |g: &dyn Fn(&Metrics) -> usize, inp: &str| rows.iter().filter(|m| m.input == inp).map(g).sum::<usize>();
    for inp in ["aa", "hard"] {
        writeln!(
            md,
            "\n**{inp} totals**: junctions {} correct, {} wrong degree, {} missed, {} extra (of {}); ends {} of {}; spurs {}.",
            sum(&|m| m.junc_ok, inp),
            sum(&|m| m.junc_wrong, inp),
            sum(&|m| m.junc_total - m.junc_ok - m.junc_wrong, inp),
            sum(&|m| m.junc_extra, inp),
            sum(&|m| m.junc_total, inp),
            sum(&|m| m.ends_found, inp),
            sum(&|m| m.ends_gt, inp),
            sum(&|m| m.spurs, inp)
        )
        .unwrap();
    }
    md
}

#[test]
fn known_geometry_measures_junctions_ends_missing_ink_and_caps() {
    use craft_geom::PathData;
    use craft_trace::TracedPath;
    let make = |points: &[(f64, f64)], square_caps: bool| {
        let mut path = BezPath::new();
        path.move_to(points[0]);
        for &p in &points[1..] {
            path.line_to(p);
        }
        TracedPath { path: PathData::from_bezpath(&path), color: [0; 3], pixels: 0, stroke: Some(2.0), shape: None, gradient: None, square_caps }
    };
    // An interior meeting must count as two arms, irrespective of path chaining.
    let bar = make(&[(10.27, 20.41), (50.27, 20.41)], false);
    for (stem, degree, ends) in [(make(&[(30.27, 20.41), (30.27, 40.41)], false), 3, 3), (make(&[(30.27, 10.41), (30.27, 40.41)], false), 4, 4)] {
        let res = TraceResult { paths: vec![bar.clone(), stem], ..Default::default() };
        let g = geometry(&res);
        assert_eq!(g.junctions.len(), 1);
        assert_eq!(g.junctions[0].degree, degree);
        assert!(g.junctions[0].pos.dist(P::new(30.27, 20.41)) < 1e-9);
        assert_eq!(g.ends.len(), ends);
    }
    let returning = make(&[(30.27, 20.41), (45.27, 35.41), (15.27, 35.41), (30.27, 20.41)], false);
    let stem = make(&[(30.27, 20.41), (30.27, 10.41)], false);
    let g = geometry(&TraceResult { paths: vec![returning, stem], ..Default::default() });
    assert_eq!(g.junctions.len(), 1);
    assert_eq!(g.junctions[0].degree, 3);
    assert_eq!(g.ends.len(), 1);
    // Closed subpaths have no free ends, even if their first sample is repeated.
    let mut ring = BezPath::new();
    ring.move_to((10.0, 10.0));
    ring.line_to((20.0, 10.0));
    ring.line_to((20.0, 20.0));
    ring.close_path();
    let res = TraceResult { paths: vec![TracedPath { path: PathData::from_bezpath(&ring), ..bar.clone() }], ..Default::default() };
    let g = geometry(&res);
    assert!(g.ends.is_empty() && g.junctions.is_empty());

    let mut sc = scenes::lines(2.0);
    sc.strokes.truncate(1);
    let st = &sc.strokes[0];
    let (a, b) = (st.pts[0], *st.pts.last().unwrap());
    let res = TraceResult { paths: vec![make(&[(a.x, a.y), (b.x, b.y)], false)], ..Default::default() };
    let cov = sc.rasterise();
    let m = measure(&sc, "aa", &cov, &res, 0.0);
    assert!(m.c_mean < 0.01 && m.g_p95 <= 0.251 && m.w_uni < 1e-9, "{m:?}");
    assert_eq!((m.ends_found, m.ends_gt, m.spurs), (2, 2, 0));
    assert!(m.end_err < 1e-9 && m.iou_uni > 0.999 && m.area_uni.abs() < 1e-9, "{m:?}");
    let missing = measure(&sc, "aa", &cov, &TraceResult::default(), 0.0);
    assert_eq!((missing.g_mean, missing.complete, missing.ends_found, missing.iou_uni, missing.area_uni), (30.0, 0.0, 0, 0.0, -1.0));
    let diagonal = make(&[(10.27, 10.41), (30.27, 30.41)], false);
    let round = render(&TraceResult { paths: vec![diagonal.clone()], ..Default::default() }, 50, 50, false);
    let square = render(&TraceResult { paths: vec![TracedPath { square_caps: true, ..diagonal }], ..Default::default() }, 50, 50, false);
    assert!(square.iter().filter(|p| **p).count() > round.iter().filter(|p| **p).count());
    // Preserve the exact catalogue and special scene limits.
    assert_eq!(scenes::all().len(), 24);
    assert_eq!((scenes::mix().wmax, scenes::mixed_width_body().wmax, scenes::varying().wmax, scenes::dashed(2.0).min_len), (6.0, 6.0, 12.0, 0.0));
    assert_eq!(scenes::junctions(4.0).junctions[0].0, P::new(60.27, 40.41));
}

#[test]
fn mixed_width_body_is_one_shape_traced_as_a_fill_and_strokes() {
    let sc = scenes::mixed_width_body();
    assert_eq!((sc.fills.len(), sc.strokes.len(), sc.junctions.len(), sc.corners.len()), (1, 3, 0, 0));
    let cov = sc.rasterise();
    let hard: Vec<f32> = cov.iter().map(|&c| if c >= 0.5 { 1.0 } else { 0.0 }).collect();
    // The inked pixels form one 8-connected component.
    let ink: Vec<bool> = hard.iter().map(|&c| c > 0.0).collect();
    let start = ink.iter().position(|&v| v).unwrap();
    let mut seen = vec![false; ink.len()];
    let mut stack = vec![start];
    seen[start] = true;
    let mut reached = 0;
    while let Some(i) = stack.pop() {
        reached += 1;
        let (x, y) = ((i % sc.w) as i64, (i / sc.w) as i64);
        for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= sc.w as i64 || ny >= sc.h as i64 {
                continue;
            }
            let j = ny as usize * sc.w + nx as usize;
            if ink[j] && !seen[j] {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    assert_eq!(reached, ink.iter().filter(|&&v| v).count());
    // Create Strokes splits the shape: its body is one fill and each arm is a stroke reaching its free end.
    for (input, c) in [("aa", &cov), ("hard", &hard)] {
        let m = evaluate(&sc, input, c);
        assert_eq!((m.strokes, m.fills, m.ends_gt, m.ends_found), (3, 1, 3, 3), "{m:?}");
        assert!(m.complete > 0.95 && m.fill_as_fill > 0.95, "{m:?}");
    }
}
