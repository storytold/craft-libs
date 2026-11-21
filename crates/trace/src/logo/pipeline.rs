//! The logo pipeline end to end: raster in, painted vector shapes out.

use super::field::{blur_plane, upsample};
use super::geom::{Cubic, finite};
use super::march::{area2, contours};
use super::outline::{Stats, Style, fit_outline};
use super::palette::{WHITE_MIN, auto_palette, flat_fraction, fold_tones, parse_hex, unexplained_colours};
use super::restore::RestoreParams;
use super::unmix::{Px, Switched, count_ambiguous, unmix};
use crate::{Raster, TraceError, TraceParams, TraceResult, TracedPath};
use craft_geom::{Anchor, AnchorKind, PathData, Point, SubPath};

/// Longest side the tracer works at; bigger images are averaged down first and the result scaled up.
const WORK_MAX_SIDE: usize = 1024;
/// Transparent border added around the image so every outline closes.
const PAD: usize = 3;
/// Most cells of the smooth field (about 24 MB per plane).
const FIELD_CELLS_MAX: usize = 6_000_000;
/// Flat-colour score below which the image is refused.
const MIN_FLAT: f64 = 0.93;
/// Most outlines of one colour (above the noise threshold) we will fit; more is refused, not truncated.
const MAX_LOOPS_PER_CLASS: usize = if cfg!(test) { 500 } else { 20_000 };
/// Most distinct pixel values `unmix` may have to work on (`count_ambiguous`). A busier image is
/// averaged down further first, which bounds the time on pathological input.
const EDGE_BUDGET: usize = 150_000;
/// Palette colours closer than this (RGB distance) are one colour.
const SAME_COLOUR: f64 = 12.0;
/// Most colours a palette may hold (white and transparency come on top).
const MAX_COLOURS: usize = 10;
/// Neighbouring colours overlap by this many working pixels so no hairline gap can open.
const OVERLAP: f64 = 0.06;
/// A traced outline whose area (in source pixels squared) is below this paints nothing.
const NO_AREA: f64 = 1e-6;

/// The raster as premultiplied RGBA, averaged down so its longest side is at most [`WORK_MAX_SIDE`].
fn working_copy(img: &Raster, at_least: usize) -> (Vec<Px>, usize, usize, usize) {
    let (w, h) = (img.width as usize, img.height as usize);
    let factor = w.max(h).div_ceil(WORK_MAX_SIDE).max(at_least).max(1);
    let (ow, oh) = (w.div_ceil(factor), h.div_ceil(factor));
    let mut out = Vec::with_capacity(ow * oh);
    for oy in 0..oh {
        for ox in 0..ow {
            let mut acc = [0.0f32; 4];
            let mut n = 0.0f32;
            for y in oy * factor..((oy + 1) * factor).min(h) {
                for x in ox * factor..((ox + 1) * factor).min(w) {
                    let i = (y * w + x) * 4;
                    if let Some(p) = img.rgba.get(i..i + 4) {
                        let a = f32::from(p[3]) / 255.0;
                        acc[0] += f32::from(p[0]) / 255.0 * a;
                        acc[1] += f32::from(p[1]) / 255.0 * a;
                        acc[2] += f32::from(p[2]) / 255.0 * a;
                        acc[3] += a;
                        n += 1.0;
                    }
                }
            }
            let n = n.max(1.0);
            out.push([acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n]);
        }
    }
    (out, ow, oh, factor)
}

fn style(params: &TraceParams) -> Style {
    // Parameters come from users and agents: a NaN or infinity must not reach the geometry.
    let unit = |v: f64, default: f64| if v.is_finite() { (v / 100.0).clamp(0.0, 1.0) } else { default };
    let paths = unit(params.paths, 0.6);
    let corners = unit(params.corners, 0.5);
    Style {
        tol: 0.04 + 0.2 * (1.0 - paths),
        corner_deg: 90.0 - 55.0 * corners,
        snap: params.snap_curves_to_lines,
        restore: Some(RestoreParams::default()),
        ..Style::default()
    }
}

/// Transparent regions fully enclosed by artwork (letters knocked out of a badge) become white, so
/// the logo is still right on a dark background. Works on coverage planes in place.
fn fill_knockouts(planes: &mut [Vec<f32>], w: usize, h: usize, white: usize, transparent: usize) {
    let n = w * h;
    let Some(t) = planes.get(transparent) else { return };
    let seed: Vec<bool> = t.iter().take(n).map(|&v| v > 0.2).collect();
    // Components of "mostly transparent" pixels; the ones touching the border are the outside.
    let mut comp = vec![0u32; n];
    let mut next_id = 0u32;
    let mut outside: Vec<bool> = vec![false];
    let mut stack = Vec::new();
    for s in 0..n {
        if !seed[s] || comp[s] != 0 {
            continue;
        }
        next_id += 1;
        let mut touches = false;
        comp[s] = next_id;
        stack.push(s);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                touches = true;
            }
            let mut push = |j: usize| {
                if seed[j] && comp[j] == 0 {
                    comp[j] = next_id;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        outside.push(touches);
    }
    let enclosed: Vec<bool> = comp.iter().map(|&c| c != 0 && !outside.get(c as usize).copied().unwrap_or(true)).collect();
    // Grow by one pixel so the blended edge pixels around a knockout come with it.
    let mut grown = enclosed.clone();
    for y in 0..h {
        for x in 0..w {
            if enclosed[y * w + x] {
                for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                    for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                        grown[yy * w + xx] = true;
                    }
                }
            }
        }
    }
    for (i, &is_grown) in grown.iter().enumerate() {
        if is_grown {
            let tv = planes.get(transparent).and_then(|p| p.get(i)).copied().unwrap_or(0.0);
            if tv > 0.02 {
                if let Some(v) = planes.get_mut(white).and_then(|p| p.get_mut(i)) {
                    *v += tv;
                }
                if let Some(v) = planes.get_mut(transparent).and_then(|p| p.get_mut(i)) {
                    *v = 0.0;
                }
            }
        }
    }
}

fn pad_plane(plane: &[f32], w: usize, h: usize, outside: f32) -> Vec<f32> {
    let (wp, hp) = (w + 2 * PAD, h + 2 * PAD);
    let mut out = vec![outside; wp * hp];
    for y in 0..h {
        if let (Some(dst), Some(src)) = (out.get_mut((y + PAD) * wp + PAD..(y + PAD) * wp + PAD + w), plane.get(y * w..(y + 1) * w)) {
            dst.copy_from_slice(src);
        }
    }
    out
}

/// A loop's `(x, y)` grid points as working-pixel coordinates.
fn to_pixels(loop_pts: &[Point], u: usize) -> Vec<Point> {
    loop_pts.iter().map(|p| Point::new((p.x + 0.5) / u as f64 - PAD as f64, (p.y + 0.5) / u as f64 - PAD as f64)).collect()
}

fn point_in_poly(p: Point, poly: &[Point]) -> bool {
    let n = poly.len();
    let mut inside = false;
    for i in 0..n {
        let (Some(&a), Some(&b)) = (poly.get(i), poly.get((i + 1) % n)) else { continue };
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

/// What the working-resolution class map says about the pixels around an outline.
struct Around<'a> {
    dominant: &'a [u8],
    w: usize,
    h: usize,
    white: usize,
    transparent: usize,
}

/// Grow the vertices of an outer outline that touch another coloured region.
fn grow_into_neighbours(p: &mut [Point], class: usize, map: &Around) {
    let n = p.len();
    if class == map.white || n < 3 {
        return;
    }
    let src = p.to_vec();
    for i in 0..n {
        let (Some(&a), Some(&b), Some(&c)) = (src.get((i + n - 1) % n), src.get((i + 1) % n), src.get(i)) else { continue };
        let t = b - a;
        let len = t.hypot();
        if len.is_nan() || len <= 1e-12 {
            continue;
        }
        let t = t / len;
        // Interior is on the left of travel, so the outward normal is on the right.
        let out = kurbo::Vec2::new(-t.y, t.x);
        let q = c + out * 0.45;
        if q.x < 0.0 || q.y < 0.0 {
            continue;
        }
        let (ix, iy) = (q.x as usize, q.y as usize);
        if ix >= map.w || iy >= map.h {
            continue;
        }
        let d = map.dominant.get(iy * map.w + ix).copied().map_or(map.transparent, usize::from);
        if d != class
            && d != map.white
            && d != map.transparent
            && let Some(v) = p.get_mut(i)
        {
            *v = c + out * OVERLAP;
        }
    }
}

/// Where working-pixel coordinates land in the source image. Working pixels are `factor` source
/// pixels wide except the last in each direction, which may be narrower: mapping the whole axis
/// with one factor would overshoot the image by up to `factor - 1` pixels.
#[derive(Clone, Copy)]
struct ToSource {
    factor: f64,
    work: (usize, usize),
    source: (usize, usize),
}

impl ToSource {
    fn axis(&self, v: f64, work: usize, source: usize) -> f64 {
        let last = work.saturating_sub(1) as f64;
        if v <= last { v * self.factor } else { last * self.factor + (v - last) * (source as f64 - last * self.factor) }
    }
    fn point(&self, p: Point) -> Point {
        Point::new(self.axis(p.x, self.work.0, self.source.0), self.axis(p.y, self.work.1, self.source.1))
    }
}

fn to_subpath(segs: &[Cubic], map: &ToSource) -> Option<SubPath> {
    let n = segs.len();
    if n == 0 {
        return None;
    }
    let sc = |p: Point| map.point(p);
    let mut anchors = Vec::with_capacity(n);
    for i in 0..n {
        let (cur, prev) = (segs.get(i)?, segs.get((i + n - 1) % n)?);
        let (p, h_out, h_in) = (sc(cur[0]), sc(cur[1]), sc(prev[2]));
        let (vi, vo) = (p - h_in, h_out - p);
        let kind = if vi.hypot() > 1e-9 && vo.hypot() > 1e-9 && vi.cross(vo).abs() < 0.07 * vi.hypot() * vo.hypot() && vi.dot(vo) > 0.0 {
            AnchorKind::Smooth
        } else {
            AnchorKind::Corner
        };
        anchors.push(Anchor { p, h_in, h_out, kind });
    }
    anchors.iter().all(|a| finite(a.p) && finite(a.h_in) && finite(a.h_out)).then(|| SubPath::new(anchors, true))
}

/// The area `sp` paints, summed about its first anchor. A subpath of one anchor has no segment and
/// paints nothing.
fn painted_area(sp: &SubPath) -> f64 {
    use kurbo::ParamCurveArea;
    let Some(origin) = sp.anchors.first().map(|a| a.p.to_vec2()) else { return 0.0 };
    (0..sp.segment_count()).map(|i| (kurbo::Affine::translate(-origin) * sp.segment(i)).signed_area()).sum()
}

struct Shape {
    class: usize,
    area: f64,
    outer: Vec<Cubic>,
    holes: Vec<Vec<Cubic>>,
}

/// The subpaths of `s` in source pixels: its outline, then its holes. The fit of a thin outline
/// can have no area: two anchors whose segments run along one line, or a single anchor, which has
/// no segment. Such an outline paints nothing and is left out with its holes, and a hole with no
/// area is left out too.
fn shape_subpaths(s: &Shape, map: &ToSource) -> Vec<SubPath> {
    let outer = to_subpath(&s.outer, map);
    if outer.as_ref().is_some_and(|sp| painted_area(sp).abs() < NO_AREA) {
        return Vec::new();
    }
    let mut subs: Vec<SubPath> = Vec::new();
    subs.extend(outer);
    for hole in &s.holes {
        subs.extend(to_subpath(hole, map).filter(|sp| painted_area(sp).abs() >= NO_AREA));
    }
    subs
}

/// A shape's subpaths in source pixels, with no outline or hole that paints nothing.
struct Kept {
    class: usize,
    area: f64,
    subs: Vec<SubPath>,
}

/// Adds `s` to `kept` unless it paints nothing, and counts the anchors of the subpaths it keeps
/// in `anchors_made`: the trace is too complex once they come to more than `max_anchors`.
fn keep_shape(s: &Shape, map: &ToSource, kept: &mut Vec<Kept>, anchors_made: &mut usize, max_anchors: usize) -> Result<(), TraceError> {
    let subs = shape_subpaths(s, map);
    if subs.is_empty() {
        return Ok(());
    }
    *anchors_made = anchors_made.saturating_add(subs.iter().map(|sp| sp.anchors.len()).sum::<usize>());
    if *anchors_made > max_anchors {
        return Err(TraceError::TooComplex { max: max_anchors });
    }
    kept.push(Kept { class: s.class, area: s.area, subs });
    Ok(())
}

/// A hard-edged source (a GIF, say) has no anti-aliasing to read an edge position from, and its
/// outlines would follow the pixel staircase. Blurring the coverage by under a pixel gives the
/// outline a smooth edge to find. Sources with plenty of blended edge pixels are left alone. The
/// pixels that took another mix than their best one (`switched`) are judged by their best mix.
fn soften_if_aliased(planes: &mut [Vec<f32>], switched: &Switched, w: usize, h: usize) -> bool {
    let n = w * h;
    let aliased = {
        let at = |c: usize, i: usize| match switched.binary_search_by_key(&i, |&(j, _)| j) {
            Ok(s) => switched.get(s).and_then(|(_, col)| col.get(c)).copied().unwrap_or(0.0),
            Err(_) => plane_at(planes, c, i),
        };
        let dominant: Vec<usize> = (0..n).map(|i| (0..planes.len()).max_by(|&a, &b| at(a, i).total_cmp(&at(b, i))).unwrap_or(0)).collect();
        let (mut boundary, mut blended) = (0usize, 0usize);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let d = dominant.get(i).copied().unwrap_or(0);
                let strongest = (0..planes.len()).map(|c| at(c, i)).fold(0.0f32, f32::max);
                if strongest < 0.9 {
                    blended += 1;
                }
                let differs = |j: usize| dominant.get(j).is_some_and(|&o| o != d);
                if (x + 1 < w && differs(i + 1)) || (y + 1 < h && differs(i + w)) {
                    boundary += 1;
                }
            }
        }
        boundary > 0 && blended * 4 < boundary
    };
    if aliased {
        for p in planes.iter_mut() {
            blur_plane(p, w, h, 0.8);
        }
    }
    aliased
}

fn plane_at(planes: &[Vec<f32>], class: usize, i: usize) -> f32 {
    planes.get(class).and_then(|p| p.get(i)).copied().unwrap_or(0.0)
}

/// The unmixing classes for a palette: each colour, then white (when `own_white`), then transparent.
fn classes_for(palette: &[[u8; 3]], own_white: bool) -> Vec<[f64; 4]> {
    let mut classes: Vec<[f64; 4]> =
        palette.iter().map(|c| [f64::from(c[0]) / 255.0, f64::from(c[1]) / 255.0, f64::from(c[2]) / 255.0, 1.0]).collect();
    if own_white {
        classes.push([1.0, 1.0, 1.0, 1.0]);
    }
    classes.push([0.0, 0.0, 0.0, 0.0]);
    classes
}

/// Whether white is traced as a class of its own: always, unless the caller named the colors and
/// turned [`TraceParams::logo_white`] off. With Ignore White it is, so white areas are left out.
fn own_white(params: &TraceParams) -> bool {
    params.logo_white || params.logo_colors.is_empty() || params.ignore_white
}

/// Without a white class of its own: the palette color that takes white's coverage, the one
/// nearest white (the first of equally near ones). `None` when a named color is white already
/// (closer than [`SAME_COLOUR`]), so white needs no class to unmix with.
fn white_goes_to(palette: &[[u8; 3]]) -> Option<usize> {
    let (i, d) = palette.iter().enumerate().map(|(i, &c)| (i, distance(c, [255; 3]))).min_by(|a, b| a.1.total_cmp(&b.1))?;
    (d >= SAME_COLOUR).then_some(i)
}

fn distance(a: [u8; 3], b: [u8; 3]) -> f64 {
    (0..3).map(|k| (f64::from(a[k]) - f64::from(b[k])).powi(2)).sum::<f64>().sqrt()
}

type Colours = Vec<[u8; 3]>;

/// The colours to trace: those the caller named (white is skipped when it is its own class;
/// colours closer than [`SAME_COLOUR`] count as one; at most [`MAX_COLOURS`]) or the ones found,
/// the tones of one colour folded into it ([`fold_tones`]); and with them the image's own colors
/// before folding. With named colors, the image's own colors are found the same way as when none are
/// named, and as many are looked for as are named, with white, or as the Colors setting gives,
/// whichever is more.
fn choose_palette(params: &TraceParams, pix: &[Px], w: usize, h: usize) -> Result<(Colours, Colours), TraceError> {
    if params.logo_colors.is_empty() {
        let found = auto_palette(pix, w, h, (params.colors as usize).clamp(2, MAX_COLOURS));
        return Ok((fold_tones(pix, found.clone()), found));
    }
    let mut palette: Vec<[u8; 3]> = Vec::new();
    for s in &params.logo_colors {
        // Every entry is checked, but only the first few distinct colours are kept.
        let c = parse_hex(s).ok_or_else(|| TraceError::BadPalette(s.clone()))?;
        let white = own_white(params) && c.iter().all(|&v| v >= WHITE_MIN);
        if palette.len() < MAX_COLOURS && !white && !palette.iter().any(|&q| distance(q, c) < SAME_COLOUR) {
            palette.push(c);
        }
    }
    if palette.is_empty() {
        return Ok((palette, Vec::new()));
    }
    // (One more than named: white takes a slot while the colors are found, and is left out after.)
    let tones = auto_palette(pix, w, h, (palette.len() + 1).max(params.colors as usize).clamp(2, MAX_COLOURS + 1));
    Ok((palette, tones))
}

/// Trace a flat-colour logo.
pub(crate) fn trace_logo(img: &Raster, params: &TraceParams, max_anchors: usize) -> Result<TraceResult, TraceError> {
    let (w0, h0) = (img.width as usize, img.height as usize);
    if w0 == 0 || h0 == 0 || w0.checked_mul(h0).and_then(|n| n.checked_mul(4)) != Some(img.rgba.len()) {
        return Ok(TraceResult::default());
    }
    let (mut pix, mut w, mut h, mut factor) = working_copy(img, 1);
    // (`tones`: the image's own colors as found, each tone of an unevenly lit colour on its own,
    // also when colors are named. Whether the image is flat art is asked of them: a paper's tones
    // are flat colour too, and how far the named colors are from the image's does not change the answer.)
    let (mut palette, tones) = choose_palette(params, &pix, w, h)?;
    if palette.is_empty() {
        return Ok(TraceResult::default());
    }
    let fraction = flat_fraction(&pix, w, h, &tones);
    if fraction < MIN_FLAT {
        return Err(TraceError::NotFlat { fraction });
    }
    let own_white = own_white(params);
    // Without a white of its own, white is still unmixed as a class, so blended edges split where
    // they are, and its coverage is then added to the color it goes to.
    let merge_white = if own_white { None } else { white_goes_to(&palette) };
    let mut classes = classes_for(&palette, own_white || merge_white.is_some());
    // A very busy image is averaged down further until the unmixing work is bounded.
    for _ in 0..6 {
        if count_ambiguous(&pix, w, h, &classes) <= EDGE_BUDGET || w * h <= 16_384 {
            break;
        }
        (pix, w, h, factor) = working_copy(img, factor * 2);
    }
    let (mut planes, residual, switched) = unmix(&pix, w, h, &classes);
    let (mut planes, mut switched) = if params.logo_colors.is_empty() && palette.len() < MAX_COLOURS {
        // Colours too small to be found at first show up as pixels nothing explains: add them.
        let extra = unexplained_colours(&pix, w, h, &residual, &palette);
        if extra.is_empty() {
            (planes, switched)
        } else {
            palette.extend(extra.into_iter().take(MAX_COLOURS - palette.len()));
            // (A colour nothing explained may be one more tone of a colour that is there.)
            palette = fold_tones(&pix, palette);
            classes = classes_for(&palette, own_white);
            let (planes, _, switched) = unmix(&pix, w, h, &classes);
            (planes, switched)
        }
    } else {
        (std::mem::take(&mut planes), switched)
    };
    if let Some(to) = merge_white
        && palette.len() < planes.len()
    {
        let white_plane = planes.remove(palette.len());
        if let Some(target) = planes.get_mut(to) {
            for (t, v) in target.iter_mut().zip(&white_plane) {
                *t += *v;
            }
        }
        for (_, col) in &mut switched {
            if palette.len() < col.len() {
                let white_weight = col.remove(palette.len());
                if let Some(t) = col.get_mut(to) {
                    *t += white_weight;
                }
            }
        }
    }
    // Without a white class, `white` is an index no class has.
    let (white, transparent) = if own_white { (palette.len(), palette.len() + 1) } else { (usize::MAX, palette.len()) };
    let aliased = soften_if_aliased(&mut planes, &switched, w, h);
    if own_white {
        fill_knockouts(&mut planes, w, h, white, transparent);
    }
    let n = w * h;
    let dominant: Vec<u8> = (0..n)
        .map(|i| (0..planes.len()).max_by(|&a, &b| plane_at(&planes, a, i).total_cmp(&plane_at(&planes, b, i))).unwrap_or(transparent) as u8)
        .collect();
    let around = Around { dominant: &dominant, w, h, white, transparent };
    // Smooth fields: where each class is the most present, and by how much.
    let (wp, hp) = (w + 2 * PAD, h + 2 * PAD);
    let u = (((FIELD_CELLS_MAX / (wp * hp).max(1)) as f64).sqrt() as usize).clamp(2, 8);
    let (ow, oh) = (wp * u, hp * u);
    let padded: Vec<Vec<f32>> = planes.iter().enumerate().map(|(c, p)| pad_plane(p, w, h, if c == transparent { 1.0 } else { 0.0 })).collect();
    let mut best = vec![f32::MIN; ow * oh];
    let mut second = vec![f32::MIN; ow * oh];
    let mut best_id = vec![0u8; ow * oh];
    for (c, pl) in padded.iter().enumerate() {
        let m = upsample(pl, wp, hp, u);
        if m.len() != ow * oh {
            return Ok(TraceResult::default());
        }
        for (i, &v) in m.iter().enumerate() {
            if v > best[i] {
                second[i] = best[i];
                best[i] = v;
                best_id[i] = c as u8;
            } else if v > second[i] {
                second[i] = v;
            }
        }
    }
    let mut style = style(params);
    if aliased {
        // No anti-aliasing means the edge position is only known to about half a pixel.
        style.tol *= 2.5;
        style.circle_tol *= 1.8;
        style.line_tol *= 1.8;
        style.sigma = 0.9;
    }
    // `noise` is in source pixels (areas under noise/10 are ignored), whatever the working size.
    let min_area = ((f64::from(params.noise) / 10.0) / (factor * factor) as f64).max(0.3);
    let mut stats = Stats::default();
    let to_source = ToSource { factor: factor as f64, work: (w, h), source: (w0, h0) };
    let mut shapes: Vec<Kept> = Vec::new();
    let mut anchors_made = 0usize;
    for class in 0..transparent {
        if class == white && params.ignore_white {
            continue;
        }
        let m = upsample(padded.get(class).map_or(&[][..], Vec::as_slice), wp, hp, u);
        let f: Vec<f32> = m.iter().enumerate().map(|(i, &v)| v - if usize::from(best_id[i]) == class { second[i] } else { best[i] }).collect();
        drop(m);
        let mut outers: Vec<(f64, Vec<Point>)> = Vec::new();
        let mut holes: Vec<Vec<Point>> = Vec::new();
        for lp in contours(&f, ow, oh) {
            let a2 = area2(&lp) / (u * u) as f64;
            if a2.abs() / 2.0 < min_area {
                continue;
            }
            // Holes are cut only under Ignore White or with no white class; otherwise white and the other colors are
            // traced as their own shapes.
            if a2 >= 0.0 && !(params.ignore_white || !own_white) {
                continue;
            }
            // More real outlines than we will fit is refused, not silently truncated.
            if outers.len() + holes.len() >= MAX_LOOPS_PER_CLASS {
                return Err(TraceError::TooComplex { max: max_anchors });
            }
            let pts = to_pixels(&lp, u);
            if a2 < 0.0 { outers.push((a2.abs() / 2.0, pts)) } else { holes.push(pts) }
        }
        // Each hole belongs to the smallest outline that contains it, and to no other. (Holes are
        // cut where white does not paint over them: with Ignore White, or with no white class.)
        let owner: Vec<Option<usize>> = if params.ignore_white || !own_white {
            holes
                .iter()
                .map(|hole| {
                    let p = *hole.first()?;
                    outers.iter().enumerate().filter(|(_, (_, o))| point_in_poly(p, o)).min_by(|a, b| a.1.0.total_cmp(&b.1.0)).map(|(i, _)| i)
                })
                .collect()
        } else {
            Vec::new()
        };
        for (idx, (area, mut pts)) in outers.into_iter().enumerate() {
            grow_into_neighbours(&mut pts, class, &around);
            let Some(outer) = fit_outline(&pts, &style, &mut stats) else { continue };
            let hole_fits: Vec<Vec<Cubic>> =
                holes.iter().zip(&owner).filter(|(_, o)| **o == Some(idx)).filter_map(|(hole, _)| fit_outline(hole, &style, &mut stats)).collect();
            keep_shape(&Shape { class, area, outer, holes: hole_fits }, &to_source, &mut shapes, &mut anchors_made, max_anchors)?;
        }
    }
    // Painter's order: biggest shapes first, so smaller ones sit on top of what they belong to.
    shapes.sort_by(|a, b| b.area.total_cmp(&a.area).then(a.class.cmp(&b.class)));
    let mut out = TraceResult { worked_at: (factor > 1).then_some((w as u32, h as u32)), ..TraceResult::default() };
    for s in shapes {
        let color = palette.get(s.class).copied().unwrap_or([255, 255, 255]);
        if !out.palette.contains(&color) {
            out.palette.push(color);
        }
        out.paths.push(TracedPath {
            path: PathData::new(s.subs),
            color,
            pixels: (s.area * (factor * factor) as f64) as usize,
            stroke: None,
            shape: None,
            gradient: None,
            square_caps: false,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed outline of straight cubics through `pts`.
    fn polygon(pts: &[(f64, f64)]) -> Vec<Cubic> {
        (0..pts.len())
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                let (a, b) = (Point::new(a.0, a.1), Point::new(b.0, b.1));
                [a, a, b, b]
            })
            .collect()
    }

    #[test]
    fn a_hole_without_area_is_left_out() {
        let map = ToSource { factor: 1.0, work: (40, 40), source: (40, 40) };
        let shape = Shape {
            class: 0,
            area: 900.0,
            outer: polygon(&[(5.0, 5.0), (35.0, 5.0), (35.0, 35.0), (5.0, 35.0)]),
            holes: vec![polygon(&[(10.0, 20.0), (20.0, 20.0)]), polygon(&[(20.0, 10.0), (30.0, 10.0), (30.0, 30.0)])],
        };
        let subs = shape_subpaths(&shape, &map);
        let areas: Vec<f64> = subs.iter().map(painted_area).collect();
        assert_eq!(subs.len(), 2, "areas {areas:?}");
        assert!(areas.iter().all(|a| a.abs() >= 100.0), "areas {areas:?}");
    }

    #[test]
    fn an_outline_without_area_is_left_out_with_its_holes() {
        let map = ToSource { factor: 1.0, work: (40, 40), source: (40, 40) };
        let shape = Shape {
            class: 0,
            area: 1.0,
            outer: polygon(&[(5.0, 5.0), (35.0, 5.0)]),
            holes: vec![polygon(&[(20.0, 10.0), (30.0, 10.0), (30.0, 30.0)])],
        };
        assert!(shape_subpaths(&shape, &map).is_empty());
    }

    #[test]
    fn the_anchor_limit_counts_only_the_subpaths_kept() {
        let map = ToSource { factor: 1.0, work: (40, 40), source: (40, 40) };
        let square = || polygon(&[(5.0, 5.0), (35.0, 5.0), (35.0, 35.0), (5.0, 35.0)]);
        let with_flat_hole = Shape { class: 0, area: 900.0, outer: square(), holes: vec![polygon(&[(10.0, 20.0), (20.0, 20.0)])] };
        let flat_outline = Shape { class: 0, area: 1.0, outer: polygon(&[(5.0, 5.0), (35.0, 5.0)]), holes: vec![square()] };
        let (mut kept, mut anchors_made) = (Vec::new(), 0usize);
        assert!(keep_shape(&with_flat_hole, &map, &mut kept, &mut anchors_made, 4).is_ok());
        assert!(keep_shape(&flat_outline, &map, &mut kept, &mut anchors_made, 4).is_ok());
        assert_eq!((kept.len(), anchors_made), (1, 4));
        assert!(matches!(keep_shape(&with_flat_hole, &map, &mut kept, &mut anchors_made, 4), Err(TraceError::TooComplex { max: 4 })));
    }
}
