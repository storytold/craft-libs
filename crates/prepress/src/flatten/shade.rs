//! Raster work for the PDF-in adapter, all in ink values: gradient stops sampled from a shading
//! function, shadings rasterized, tiling patterns past the cell cap drawn, stencil masks traced.

use std::collections::HashMap;

use craft_pdf::interpret::color::{CmykSimulation, ColorSpace};
use craft_pdf::interpret::pattern::ShadingPattern;
use craft_pdf::interpret::shading::{ShadingFunction, ShadingType, Triangle};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Vec2};

use super::{FillRule, GradientGeometry, ImageItem, ImageSpace, Item, Paint};
use crate::ink::{InkColor, InkId, InkSet, Overprint, Process};

/// Most a gradient's piecewise-linear stops may differ from the function, per ink.
const STOP_TOLERANCE: f32 = 0.5 / 255.0;

/// The narrowest interval between gradient stops; a discontinuous function gets a pair of stops
/// this close.
const MIN_STOP_STEP: f32 = 1.0 / 4096.0;

/// Sub-rows per pixel row when the canvas computes path coverage.
const SUBROWS: usize = 4;

/// Stops `(offset, values)` approximating `eval` on `0..=1` within [`STOP_TOLERANCE`]. `eval`
/// returns the same number of values everywhere; a failed evaluation reads as zeros.
pub(super) fn sample_stops(len: usize, eval: impl Fn(f32) -> Option<Vec<f32>>) -> Vec<(f32, Vec<f32>)> {
    let f = |s: f32| {
        let mut v = eval(s).unwrap_or_default();
        v.resize(len, 0.0);
        v
    };
    const START: usize = 8;
    let mut stops = vec![(0.0, f(0.0))];
    for k in 0..START {
        let (a, b) = (k as f32 / START as f32, (k + 1) as f32 / START as f32);
        let fa = stops.last().map(|s| s.1.clone()).unwrap_or_default();
        refine(a, &fa, b, f(b), &f, &mut stops);
    }
    stops
}

fn refine(a: f32, fa: &[f32], b: f32, fb: Vec<f32>, f: &impl Fn(f32) -> Vec<f32>, out: &mut Vec<(f32, Vec<f32>)>) {
    let m = (a + b) / 2.0;
    let fm = f(m);
    let err = |t: f32, ft: &[f32]| {
        let u = (t - a) / (b - a);
        fa.iter().zip(&fb).zip(ft).map(|((x, y), v)| (x + (y - x) * u - v).abs()).fold(0.0_f32, f32::max)
    };
    let within = err(m, &fm) <= STOP_TOLERANCE
        && [0.25, 0.75].iter().all(|q| {
            let t = a + (b - a) * q;
            err(t, &f(t)) <= STOP_TOLERANCE
        });
    if within || b - a <= MIN_STOP_STEP {
        out.push((b, fb));
    } else {
        refine(a, fa, m, fm.clone(), f, out);
        refine(m, &fm, b, fb, f, out);
    }
}

/// `function` at `input`.
pub(super) fn eval(function: &ShadingFunction, input: &[f32]) -> Option<Vec<f32>> {
    function.eval(&input.iter().copied().collect()).map(|v| v.to_vec())
}

/// A shading's colors sampled on a pixel grid.
pub(super) struct ShadeRaster {
    pub width: u32,
    pub height: u32,
    /// `ink_space.len()` values per pixel.
    pub values: Vec<f32>,
    /// 255 where the shading (or its background) paints.
    pub alpha: Vec<u8>,
}

/// The function-based or mesh shading `sp` over `area` (page space) at `scale` pixels per point,
/// with `/Background` under it when `background`. `None` for other shading types.
pub(super) fn rasterize_shading(
    sp: &ShadingPattern,
    area: Rect,
    scale: f64,
    background: bool,
    simulation: Option<&CmykSimulation>,
) -> Option<ShadeRaster> {
    let (w, h) = pixel_size(area, scale)?;
    let cs: &ColorSpace = &sp.shading.color_space;
    let n = cs.components().max(1);
    let mut comps = vec![0.0_f32; w * h * n];
    let mut covered = vec![false; w * h];
    let to_px = Affine::scale(scale) * Affine::translate(-area.origin().to_vec2());
    match sp.shading.shading_type.as_ref() {
        ShadingType::FunctionBased { domain, matrix, function } => {
            let inv = (to_px * sp.matrix * *matrix).inverse();
            for y in 0..h {
                for x in 0..w {
                    let r = inv * Point::new(x as f64 + 0.5, y as f64 + 0.5);
                    let (rx, ry) = (r.x as f32, r.y as f32);
                    if rx < domain[0] || rx > domain[1] || ry < domain[2] || ry > domain[3] {
                        continue;
                    }
                    if let Some(c) = eval(function, &[rx, ry]) {
                        set(&mut comps, &mut covered, n, y * w + x, &c);
                    }
                }
            }
        }
        ShadingType::TriangleMesh { triangles, function } => {
            let mut raster = Triangles { w, h, n, comps: &mut comps, covered: &mut covered, function: function.as_ref() };
            for t in triangles {
                raster.draw(t, to_px * sp.matrix);
            }
        }
        ShadingType::CoonsPatchMesh { patches, function } => {
            let mut raster = Triangles { w, h, n, comps: &mut comps, covered: &mut covered, function: function.as_ref() };
            let mut buffer = Vec::new();
            for patch in patches {
                buffer.clear();
                patch.to_triangles(&mut buffer);
                for t in &buffer {
                    raster.draw(t, to_px * sp.matrix);
                }
            }
        }
        ShadingType::TensorProductPatchMesh { patches, function } => {
            let mut raster = Triangles { w, h, n, comps: &mut comps, covered: &mut covered, function: function.as_ref() };
            let mut buffer = Vec::new();
            for patch in patches {
                buffer.clear();
                patch.to_triangles(&mut buffer);
                for t in &buffer {
                    raster.draw(t, to_px * sp.matrix);
                }
            }
        }
        ShadingType::RadialAxial { .. } | ShadingType::Dummy => return None,
    }
    if let Some(bg) = sp.shading.background.as_ref().filter(|_| background) {
        for i in 0..w * h {
            if !covered[i] {
                set(&mut comps, &mut covered, n, i, bg);
            }
        }
    }
    // Only covered pixels are converted.
    let mut colors = Vec::new();
    for (i, c) in covered.iter().enumerate() {
        if *c {
            colors.extend_from_slice(&comps[i * n..(i + 1) * n]);
        }
    }
    let mut converted = Vec::new();
    cs.ink_values(&colors, simulation, &mut converted);
    let len = cs.ink_space(simulation).len();
    let mut values = vec![0.0_f32; w * h * len];
    let mut src = converted.chunks_exact(len.max(1));
    for (i, c) in covered.iter().enumerate() {
        if *c
            && len > 0
            && let Some(v) = src.next()
        {
            values[i * len..(i + 1) * len].copy_from_slice(v);
        }
    }
    let alpha = covered.iter().map(|c| if *c { 255 } else { 0 }).collect();
    Some(ShadeRaster { width: w as u32, height: h as u32, values, alpha })
}

fn set(comps: &mut [f32], covered: &mut [bool], n: usize, i: usize, c: &[f32]) {
    for (k, v) in comps[i * n..(i + 1) * n].iter_mut().enumerate() {
        *v = c.get(k).copied().unwrap_or(0.0);
    }
    covered[i] = true;
}

/// The pixel grid covering `area` at `scale`: its size, or `None` when empty.
pub(super) fn pixel_size(area: Rect, scale: f64) -> Option<(usize, usize)> {
    let (w, h) = ((area.width() * scale).ceil(), (area.height() * scale).ceil());
    (w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0).then_some((w as usize, h as usize))
}

/// Mesh triangles drawn into component buffers, later triangles over earlier ones.
struct Triangles<'a> {
    w: usize,
    h: usize,
    n: usize,
    comps: &'a mut [f32],
    covered: &'a mut [bool],
    function: Option<&'a ShadingFunction>,
}

impl Triangles<'_> {
    fn draw(&mut self, t: &Triangle, to_px: Affine) {
        let (a, b, c) = (to_px * t.p0.point, to_px * t.p1.point, to_px * t.p2.point);
        let area2 = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        if !area2.is_finite() || area2.abs() < 1e-12 {
            return;
        }
        let bbox = Rect::from_points(a, b).union_pt(c);
        let x0 = (bbox.x0 - 0.5).ceil().max(0.0) as usize;
        let y0 = (bbox.y0 - 0.5).ceil().max(0.0) as usize;
        let x1 = ((bbox.x1 - 0.5).floor() + 1.0).clamp(0.0, self.w as f64) as usize;
        let y1 = ((bbox.y1 - 0.5).floor() + 1.0).clamp(0.0, self.h as f64) as usize;
        let mut color = Vec::with_capacity(self.n);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
                let la = ((b.x - p.x) * (c.y - p.y) - (c.x - p.x) * (b.y - p.y)) / area2;
                let lb = ((c.x - p.x) * (a.y - p.y) - (a.x - p.x) * (c.y - p.y)) / area2;
                let lc = 1.0 - la - lb;
                if la < -1e-9 || lb < -1e-9 || lc < -1e-9 {
                    continue;
                }
                color.clear();
                let (ca, cb, cc) = (&t.p0.colors, &t.p1.colors, &t.p2.colors);
                for k in 0..ca.len() {
                    let v =
                        la * f64::from(ca[k]) + lb * f64::from(cb.get(k).copied().unwrap_or(0.0)) + lc * f64::from(cc.get(k).copied().unwrap_or(0.0));
                    color.push(v as f32);
                }
                let value = match self.function {
                    Some(f) => match eval(f, &color[..color.len().min(1)]) {
                        Some(v) => v,
                        None => continue,
                    },
                    None => color.clone(),
                };
                set(self.comps, self.covered, self.n, y * self.w + x, &value);
            }
        }
    }
}

/// `items` with every coordinate moved by `by`.
pub(super) fn translate(items: &mut [Item], by: Vec2) {
    let t = Affine::translate(by);
    for item in items {
        match item {
            Item::Fill(f) => {
                f.path.apply_affine(t);
                translate_paint(&mut f.paint, t);
            }
            Item::Text(text) => {
                text.transform = t * text.transform;
                text.outlines.apply_affine(t);
                translate_paint(&mut text.paint, t);
            }
            Item::Image(im) => im.transform = t * im.transform,
            Item::Group(g) => {
                if let Some(clip) = g.clip.as_mut() {
                    clip.apply_affine(t);
                }
                if let Some(mask) = g.mask.as_mut() {
                    translate(&mut mask.group.children, by);
                    if let Some(clip) = mask.group.clip.as_mut() {
                        clip.apply_affine(t);
                    }
                }
                translate(&mut g.children, by);
            }
        }
    }
}

fn translate_paint(paint: &mut Paint, t: Affine) {
    if let Paint::Gradient { geometry, .. } = paint {
        match geometry {
            GradientGeometry::Axial { transform, .. } | GradientGeometry::Radial { transform, .. } => *transform = t * *transform,
        }
    }
}

/// The opaque samples (≥ 128) of a `w` × `h` mask as disjoint rectangles in sample space;
/// `None` past `cap` rectangles.
pub(super) fn trace_stencil(mask: &[u8], w: usize, h: usize, cap: usize) -> Option<Vec<Rect>> {
    let mut done = Vec::new();
    let mut open: HashMap<(usize, usize), usize> = HashMap::new();
    for y in 0..=h {
        let mut next = HashMap::new();
        if y < h {
            let row = mask.get(y * w..(y + 1) * w)?;
            let mut x = 0;
            while x < w {
                if row[x] >= 128 {
                    let start = x;
                    while x < w && row[x] >= 128 {
                        x += 1;
                    }
                    let top = open.remove(&(start, x)).unwrap_or(y);
                    next.insert((start, x), top);
                } else {
                    x += 1;
                }
            }
        }
        for ((x0, x1), top) in open.drain() {
            done.push(Rect::new(x0 as f64, top as f64, x1 as f64, y as f64));
            if done.len() > cap {
                return None;
            }
        }
        open = next;
    }
    Some(done)
}

/// `src` (`sw` × `sh`) resampled bilinearly to `dw` × `dh`.
pub(super) fn resample(src: &[u8], (sw, sh): (usize, usize), (dw, dh): (usize, usize)) -> Vec<u8> {
    let at = |x: usize, y: usize| f64::from(src.get(y.min(sh - 1) * sw + x.min(sw - 1)).copied().unwrap_or(0));
    let mut out = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        let fy = ((y as f64 + 0.5) * sh as f64 / dh as f64 - 0.5).max(0.0);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        for x in 0..dw {
            let fx = ((x as f64 + 0.5) * sw as f64 / dw as f64 - 0.5).max(0.0);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
            let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
            out.push((top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8);
        }
    }
    out
}

/// A raster of every ink in an [`InkSet`] with alpha (values premultiplied), for content drawn
/// as an image.
/// Transparency groups draw as plain groups: their clip and opacity apply; blend modes and soft
/// masks are left out.
pub(super) struct Canvas<'a> {
    w: usize,
    h: usize,
    inks: &'a InkSet,
    n: usize,
    /// Page space to pixels.
    to_px: Affine,
    values: Vec<f32>,
    alpha: Vec<f32>,
}

impl<'a> Canvas<'a> {
    pub fn new(area: Rect, scale: f64, inks: &'a InkSet) -> Option<Self> {
        let (w, h) = pixel_size(area, scale)?;
        let n = inks.len();
        let to_px = Affine::scale(scale) * Affine::translate(-area.origin().to_vec2());
        let pixels = w.checked_mul(h)?;
        Some(Self { w, h, inks, n, to_px, values: vec![0.0; pixels.checked_mul(n)?], alpha: vec![0.0; pixels] })
    }

    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// The unit square to page space, as [`ImageItem::transform`].
    pub fn image_transform(&self) -> Affine {
        self.to_px.inverse() * Affine::scale_non_uniform(self.w as f64, self.h as f64)
    }

    pub fn draw(&mut self, items: &[Item]) {
        self.draw_clipped(items, None, 1.0);
    }

    fn draw_clipped(&mut self, items: &[Item], clip: Option<&[f32]>, opacity: f32) {
        for item in items {
            match item {
                Item::Fill(f) => self.fill(&f.path, f.rule, &f.paint, f.overprint, clip, opacity),
                Item::Text(t) => self.fill(&t.outlines, FillRule::NonZero, &t.paint, t.overprint, clip, opacity),
                Item::Image(im) => self.image(im, clip, opacity),
                Item::Group(g) => {
                    let own = g.clip.as_ref().map(|c| {
                        let mut cov = self.coverage(c, FillRule::NonZero);
                        if let Some(outer) = clip {
                            cov.iter_mut().zip(outer).for_each(|(a, b)| *a *= b);
                        }
                        cov
                    });
                    self.draw_clipped(&g.children, own.as_deref().or(clip), opacity * g.opacity.clamp(0.0, 1.0));
                }
            }
        }
    }

    /// Dense values and the inks a paint sets, as the compositor reads a flat paint.
    fn dense(&self, color: &InkColor, overprint: Overprint) -> Option<(Vec<f32>, Vec<bool>)> {
        let n = self.n;
        let (mut value, mut named) = (vec![0.0; n], vec![false; n]);
        match color {
            InkColor::None => return None,
            InkColor::All(t) => {
                value.fill(t.clamp(0.0, 1.0));
                named.fill(true);
            }
            InkColor::Tints(tints) => {
                for &(id, t) in tints {
                    if let (Some(v), Some(k)) = (value.get_mut(id.index()), named.get_mut(id.index())) {
                        *v = t.clamp(0.0, 1.0);
                        *k = true;
                    }
                }
                if overprint.nonzero && color.as_cmyk().is_some() {
                    for (k, v) in named.iter_mut().zip(&value).take(4) {
                        *k = *v > 0.0;
                    }
                }
            }
        }
        if !overprint.fill {
            named.fill(true);
        }
        Some((value, named))
    }

    fn put(&mut self, i: usize, cov: f32, value: &[f32], named: &[bool]) {
        if cov <= 0.0 {
            return;
        }
        let cov = cov.min(1.0);
        for k in 0..self.n {
            if named[k] {
                let v = &mut self.values[i * self.n + k];
                *v = *v * (1.0 - cov) + value[k] * cov;
            }
        }
        self.alpha[i] += cov * (1.0 - self.alpha[i]);
    }

    fn fill(&mut self, path: &BezPath, rule: FillRule, paint: &Paint, overprint: Overprint, clip: Option<&[f32]>, opacity: f32) {
        let cov = self.coverage(path, rule);
        let factor = |i: usize| cov[i] * clip.map_or(1.0, |c| c[i]) * opacity;
        match paint {
            Paint::Solid(color) => {
                let Some((value, named)) = self.dense(color, overprint) else { return };
                for i in 0..self.w * self.h {
                    self.put(i, factor(i), &value, &named);
                }
            }
            Paint::Gradient { stops, geometry } => {
                let dense: Vec<(f32, Vec<f32>, Vec<bool>)> =
                    stops.iter().filter_map(|(s, c)| self.dense(c, overprint).map(|(v, k)| (*s, v, k))).collect();
                let Some(first) = dense.first() else { return };
                let mut named = first.2.clone();
                for d in &dense {
                    named.iter_mut().zip(&d.2).for_each(|(a, b)| *a |= b);
                }
                let from_px = self.to_px.inverse();
                let mut value = vec![0.0; self.n];
                for y in 0..self.h {
                    for x in 0..self.w {
                        let i = y * self.w + x;
                        let f = factor(i);
                        if f <= 0.0 {
                            continue;
                        }
                        let p = from_px * Point::new(x as f64 + 0.5, y as f64 + 0.5);
                        let Some(s) = gradient_offset(geometry, p) else { continue };
                        interpolate_stops(&dense, s, &mut value);
                        self.put(i, f, &value, &named);
                    }
                }
            }
        }
    }

    fn image(&mut self, im: &ImageItem, clip: Option<&[f32]>, opacity: f32) {
        let (iw, ih) = (im.size.0 as usize, im.size.1 as usize);
        let inv = (self.to_px * im.transform).inverse();
        let ids: Vec<InkId> = match &im.space {
            ImageSpace::Gray => vec![InkId::process(Process::Black)],
            ImageSpace::Cmyk => Process::ALL.iter().map(|p| InkId::process(*p)).collect(),
            ImageSpace::Inks(ids) => ids.clone(),
            ImageSpace::Rgb => return,
        };
        let comps = if im.space == ImageSpace::Gray { 1 } else { ids.len() };
        let mut named = vec![!im.overprint.fill; self.n];
        for id in &ids {
            if let Some(k) = named.get_mut(id.index()) {
                *k = true;
            }
        }
        if im.space == ImageSpace::Gray && !im.overprint.fill {
            named.fill(true);
        } else if im.space == ImageSpace::Gray {
            named.iter_mut().take(4).for_each(|k| *k = true);
        }
        let mut value = vec![0.0; self.n];
        for y in 0..self.h {
            for x in 0..self.w {
                let u = inv * Point::new(x as f64 + 0.5, y as f64 + 0.5);
                if !(0.0..1.0).contains(&u.x) || !(0.0..1.0).contains(&u.y) {
                    continue;
                }
                let (sx, sy) = ((u.x * iw as f64) as usize, (u.y * ih as f64) as usize);
                let s = sy * iw + sx;
                let Some(sample) = im.samples.get(s * comps..(s + 1) * comps) else { continue };
                value.fill(0.0);
                if im.space == ImageSpace::Gray {
                    value[Process::Black.index()] = 1.0 - f32::from(sample[0]) / 255.0;
                } else {
                    for (id, v) in ids.iter().zip(sample) {
                        if let Some(slot) = value.get_mut(id.index()) {
                            *slot = f32::from(*v) / 255.0;
                        }
                    }
                }
                let a = im.smask.as_ref().and_then(|m| m.get(s)).map_or(1.0, |a| f32::from(*a) / 255.0);
                let i = y * self.w + x;
                self.put(i, a * clip.map_or(1.0, |c| c[i]) * opacity, &value, &named);
            }
        }
    }

    /// The fraction of each pixel `path` covers.
    fn coverage(&self, path: &BezPath, rule: FillRule) -> Vec<f32> {
        let mut edges: Vec<(Point, Point)> = Vec::new();
        let (mut start, mut last) = (Point::ZERO, Point::ZERO);
        kurbo::flatten(self.to_px * path.clone(), 0.1, |el| match el {
            PathEl::MoveTo(p) => {
                if last != start {
                    edges.push((last, start));
                }
                (start, last) = (p, p);
            }
            PathEl::LineTo(p) => {
                edges.push((last, p));
                last = p;
            }
            PathEl::ClosePath => {
                if last != start {
                    edges.push((last, start));
                }
                last = start;
            }
            _ => {}
        });
        if last != start {
            edges.push((last, start));
        }
        let mut cov = vec![0.0_f32; self.w * self.h];
        let mut crossings: Vec<(f64, i32)> = Vec::new();
        for sy in 0..self.h * SUBROWS {
            let y = (sy as f64 + 0.5) / SUBROWS as f64;
            crossings.clear();
            for (a, b) in &edges {
                let (lo, hi, dir) = if a.y <= b.y { (a, b, 1) } else { (b, a, -1) };
                if y >= lo.y && y < hi.y {
                    crossings.push((lo.x + (y - lo.y) / (hi.y - lo.y) * (hi.x - lo.x), dir));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let row = sy / SUBROWS;
            let mut winding = 0;
            for k in 0..crossings.len() {
                winding += crossings[k].1;
                let inside = match rule {
                    FillRule::NonZero => winding != 0,
                    FillRule::EvenOdd => winding % 2 != 0,
                };
                if inside && let Some(next) = crossings.get(k + 1) {
                    self.span(&mut cov, row, crossings[k].0, next.0);
                }
            }
        }
        cov
    }

    fn span(&self, cov: &mut [f32], row: usize, xa: f64, xb: f64) {
        let (xa, xb) = (xa.clamp(0.0, self.w as f64), xb.clamp(0.0, self.w as f64));
        if xb <= xa {
            return;
        }
        let weight = 1.0 / SUBROWS as f32;
        let last = (xb.ceil() as usize).min(self.w);
        for px in xa.floor() as usize..last {
            let overlap = xb.min(px as f64 + 1.0) - xa.max(px as f64);
            if overlap > 0.0 {
                cov[row * self.w + px] += overlap as f32 * weight;
            }
        }
    }

    /// The canvas as an image in every ink of the set, and its alpha when not opaque throughout.
    pub fn into_image(self, overprint: Overprint) -> ImageItem {
        let transform = self.image_transform();
        let ids: Vec<InkId> = self.inks.iter().map(|(id, _)| id).collect();
        // Values are premultiplied by alpha; samples are not.
        let n = self.n.max(1);
        let samples: Vec<u8> = self
            .values
            .iter()
            .enumerate()
            .map(|(k, v)| {
                let a = self.alpha[k / n];
                let v = if a > 0.0 { v / a } else { 0.0 };
                (v.clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect();
        let opaque = self.alpha.iter().all(|a| *a >= 1.0 - 1e-4);
        let smask = (!opaque).then(|| self.alpha.iter().map(|a| (a.clamp(0.0, 1.0) * 255.0).round() as u8).collect());
        ImageItem {
            space: ImageSpace::Inks(ids),
            size: (self.w as u32, self.h as u32),
            samples: samples.into(),
            transform,
            smask,
            overprint,
            jpeg: None,
        }
    }

    /// The canvas sample nearest page point `p`: premultiplied values and alpha.
    pub fn sample(&self, p: Point) -> Option<(&[f32], f32)> {
        let q = self.to_px * p;
        if q.x < 0.0 || q.y < 0.0 {
            return None;
        }
        let (x, y) = (q.x as usize, q.y as usize);
        if x >= self.w || y >= self.h {
            return None;
        }
        let i = y * self.w + x;
        Some((&self.values[i * self.n..(i + 1) * self.n], self.alpha[i]))
    }

    /// Composites the premultiplied values `value` with alpha `a` over pixel (`x`, `y`),
    /// knocking out every ink.
    pub fn over(&mut self, x: usize, y: usize, value: &[f32], a: f32) {
        let i = y * self.w + x;
        for (v, s) in self.values[i * self.n..(i + 1) * self.n].iter_mut().zip(value) {
            *v = *v * (1.0 - a) + s;
        }
        self.alpha[i] += a * (1.0 - self.alpha[i]);
    }

    /// The page point at the center of pixel (`x`, `y`).
    pub fn center(&self, x: usize, y: usize) -> Point {
        self.to_px.inverse() * Point::new(x as f64 + 0.5, y as f64 + 0.5)
    }
}

/// Where page point `p` falls on the gradient, `0..=1`; `None` where it paints nothing.
fn gradient_offset(geometry: &GradientGeometry, p: Point) -> Option<f64> {
    let clamp = |s: f64, extend: [bool; 2]| {
        if s < 0.0 {
            extend[0].then_some(0.0)
        } else if s > 1.0 {
            extend[1].then_some(1.0)
        } else {
            Some(s)
        }
    };
    match geometry {
        GradientGeometry::Axial { start, end, extend, transform } => {
            let q = transform.inverse() * p;
            let d = *end - *start;
            let len2 = d.hypot2();
            if len2 <= f64::EPSILON {
                return None;
            }
            clamp((q - *start).dot(d) / len2, *extend)
        }
        GradientGeometry::Radial { start, start_radius, end, end_radius, extend, transform } => {
            let q = transform.inverse() * p;
            let (cd, pd, dr, r0) = (*end - *start, q - *start, end_radius - start_radius, *start_radius);
            let a = cd.hypot2() - dr * dr;
            let b = pd.dot(cd) + r0 * dr;
            let c = pd.hypot2() - r0 * r0;
            let candidates = if a.abs() < 1e-9 {
                if b.abs() < 1e-12 {
                    return None;
                }
                [c / (2.0 * b), f64::NAN]
            } else {
                let disc = b * b - a * c;
                if disc < 0.0 {
                    return None;
                }
                let root = disc.sqrt();
                let (s1, s2) = ((b + root) / a, (b - root) / a);
                [s1.max(s2), s1.min(s2)]
            };
            candidates.into_iter().filter(|s| s.is_finite() && r0 + s * dr >= 0.0).find_map(|s| clamp(s, *extend))
        }
    }
}

fn interpolate_stops(stops: &[(f32, Vec<f32>, Vec<bool>)], s: f64, out: &mut [f32]) {
    let s = s as f32;
    let k = stops.iter().position(|st| st.0 >= s).unwrap_or(stops.len() - 1);
    let (hi, lo) = (&stops[k], &stops[k.saturating_sub(1)]);
    let u = if hi.0 > lo.0 { ((s - lo.0) / (hi.0 - lo.0)).clamp(0.0, 1.0) } else { 1.0 };
    for (o, (a, b)) in out.iter_mut().zip(lo.1.iter().zip(&hi.1)) {
        *o = a + (b - a) * u;
    }
}

/// The unit square of a raster image placed over `area` at `scale`, as [`ImageItem::transform`].
pub(super) fn area_transform(area: Rect, scale: f64, (w, h): (usize, usize)) -> Affine {
    Affine::translate(area.origin().to_vec2()) * Affine::scale_non_uniform(w as f64 / scale, h as f64 / scale)
}

/// A gradient stop's color from ink values.
pub(super) fn stop_color(ids: &StopInks, values: &[f32]) -> InkColor {
    match ids {
        StopInks::Gray => {
            let k = 1.0 - values.first().copied().unwrap_or(1.0);
            InkColor::Tints(Process::ALL.iter().map(|p| (InkId::process(*p), if *p == Process::Black { k } else { 0.0 })).collect())
        }
        StopInks::Inks(ids) => InkColor::Tints(ids.iter().zip(values).map(|(id, v)| (*id, *v)).collect()),
    }
}

/// What ink values mean in a gradient stop.
pub(super) enum StopInks {
    /// A gray level, painted as black.
    Gray,
    Inks(Vec<InkId>),
}

/// Where a gradient with `extend` paints, in page space, as a clip for the same gradient
/// extended at both ends; `None` when it extends at both. `bounds` is the page area that can
/// show. Exact for axial gradients; for radial ones it is the convex hull of the circles the
/// gradient sweeps, which is where it paints.
pub(super) fn gradient_extent(geometry: &GradientGeometry, extend: [bool; 2], bounds: Rect) -> Option<BezPath> {
    if extend == [true, true] {
        return None;
    }
    let (transform, path) = match geometry {
        GradientGeometry::Axial { start, end, transform, .. } => {
            let inv = transform.inverse();
            let r = inv.transform_rect_bbox(bounds);
            let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
            let d = *end - *start;
            let len = d.hypot();
            if len.is_nan() || len <= 0.0 {
                return None;
            }
            let normal = Vec2::new(-d.y, d.x) / len;
            let s: Vec<f64> = corners.iter().map(|c| (*c - *start).dot(d) / (len * len)).collect();
            let t: Vec<f64> = corners.iter().map(|c| (*c - *start).dot(normal)).collect();
            let (smin, smax) = s.iter().fold((0.0_f64, 1.0_f64), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            let (tmin, tmax) = t.iter().fold((0.0_f64, 0.0_f64), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            let a = if extend[0] { smin - 1.0 } else { 0.0 };
            let b = if extend[1] { smax + 1.0 } else { 1.0 };
            let at = |s: f64, t: f64| *start + d * s + normal * t;
            let mut path = BezPath::new();
            path.move_to(at(a, tmin - 1.0));
            path.line_to(at(b, tmin - 1.0));
            path.line_to(at(b, tmax + 1.0));
            path.line_to(at(a, tmax + 1.0));
            path.close_path();
            (*transform, path)
        }
        GradientGeometry::Radial { start, start_radius, end, end_radius, transform, .. } => {
            let inv = transform.inverse();
            let r = inv.transform_rect_bbox(bounds);
            let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
            let circle = |s: f64| (*start + (*end - *start) * s, start_radius + s * (end_radius - start_radius));
            let dr = end_radius - start_radius;
            // How far an extended end reaches: to where the radius is 0, or until the circle
            // holds the visible area.
            let reach = |from: f64, dir: f64| {
                if dr * dir < 0.0 {
                    return -start_radius / dr;
                }
                let mut s = from + dir;
                for _ in 0..64 {
                    let (c, radius) = circle(s);
                    if corners.iter().all(|p| (*p - c).hypot() <= radius) {
                        break;
                    }
                    s = from + (s - from) * 2.0;
                }
                s
            };
            let a = if extend[0] { reach(0.0, -1.0) } else { 0.0 };
            let b = if extend[1] { reach(1.0, 1.0) } else { 1.0 };
            (*transform, hull_of_circles(circle(a), circle(b)))
        }
    };
    Some(transform * path)
}

/// The convex hull of two circles, circumscribed by a polygon.
fn hull_of_circles((c0, r0): (Point, f64), (c1, r1): (Point, f64)) -> BezPath {
    const SIDES: usize = 256;
    let grow = 1.0 / (std::f64::consts::PI / SIDES as f64).cos();
    let mut points = Vec::with_capacity(2 * SIDES);
    for (c, r) in [(c0, r0.max(0.0)), (c1, r1.max(0.0))] {
        for k in 0..SIDES {
            let a = k as f64 * std::f64::consts::TAU / SIDES as f64;
            points.push(c + Vec2::new(a.cos(), a.sin()) * r * grow);
        }
    }
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let cross = |o: Point, a: Point, b: Point| (a - o).cross(b - o);
    let mut hull: Vec<Point> = Vec::new();
    for pass in 0..2 {
        let floor = hull.len();
        let iter: Box<dyn Iterator<Item = &Point>> = if pass == 0 { Box::new(points.iter()) } else { Box::new(points.iter().rev()) };
        for p in iter {
            while hull.len() >= floor + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 0.0 {
                hull.pop();
            }
            hull.push(*p);
        }
        hull.pop();
    }
    let mut path = BezPath::new();
    for (k, p) in hull.iter().enumerate() {
        if k == 0 {
            path.move_to(*p);
        } else {
            path.line_to(*p);
        }
    }
    path.close_path();
    path
}

/// The rectangle's corners in page space through `t`, as a closed path.
pub(super) fn rect_path(r: Rect, t: Affine) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(t * Point::new(r.x0, r.y0));
    path.line_to(t * Point::new(r.x1, r.y0));
    path.line_to(t * Point::new(r.x1, r.y1));
    path.line_to(t * Point::new(r.x0, r.y1));
    path.close_path();
    path
}
