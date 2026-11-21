use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::layers::{Layers, parallel_map_at_most};
use super::params::{Method, Mode, TraceParams};
use super::raster::{Raster, TraceError};
use super::{AtomicUsize, PathData, Quantized, TRANSPARENT, TracedGradient, TracedShape, deblock, gradient, group, logo, quantize, smooth};

/// One traced shape: an outer contour plus its holes, filled with `color`, or (with `stroke`)
/// lines stroked with it.
#[derive(Clone, Debug, PartialEq)]
pub struct TracedPath {
    /// In pixel coordinates (x right, y down; the image spans `0..width × 0..height`).
    pub path: PathData,
    pub color: [u8; 3],
    /// Filled pixel count of the component the path came from.
    pub pixels: usize,
    /// Create Strokes: the path is stroked this wide (px), not filled (centre lines, or the
    /// outlines of wider areas without Fills).
    pub stroke: Option<f64>,
    /// Shapes: the shape the path is, exactly (`path` is that shape's outline).
    pub shape: Option<TracedShape>,
    /// Gradients: the linear gradient the path is filled with (`color` is its middle colour).
    pub gradient: Option<TracedGradient>,
    /// Create Strokes: the stroke's ends are square (a cap that goes half the width past the
    /// path's end, cut straight), as the line's ink is; otherwise round.
    pub square_caps: bool,
}

/// The traced result, bottom-most path first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceResult {
    pub paths: Vec<TracedPath>,
    /// Colours actually used.
    pub palette: Vec<[u8; 3]>,
    /// Auto Grouping: for each of `paths`, the path it lies inside, if any (empty without the
    /// option). Each path drawn together with those inside it, in the order they have here, paints
    /// what `paths` paints in its order.
    pub parents: Vec<Option<usize>>,
    /// Keep Colors: the colors listed in [`TraceParams::keep_colors`] (as written) that no color
    /// the trace found matches, so that nothing of them is traced. (Flat Logo mode: empty.)
    pub keep_unmatched: Vec<String>,
    /// Keep Colors: whether Ignore Color left out every color the trace found that a listed color
    /// matches, so that none of them was traced. (Flat Logo mode: false.)
    pub keep_ignored: bool,
    /// Flat Logo: the size in pixels the image was traced at when it was averaged down first;
    /// `None` when it was traced at its own size.
    pub worked_at: Option<(u32, u32)>,
}

impl TraceResult {
    pub fn anchor_count(&self) -> usize {
        self.paths.iter().map(|p| p.path.anchor_count()).sum()
    }
}

fn is_white(c: [u8; 3]) -> bool {
    c.iter().all(|&v| v >= 245)
}

/// Whether Ignore Color leaves palette colour `c` out: `target` is the colour picked (`None`: white).
pub(crate) fn is_ignored(c: [u8; 3], target: Option<[u8; 3]>) -> bool {
    match target {
        None => is_white(c),
        Some(t) => (0..3).map(|k| (f64::from(c[k]) - f64::from(t[k])).powi(2)).sum::<f64>() <= 30.0 * 30.0 || (is_white(t) && is_white(c)),
    }
}

/// Whether Keep Colors traces palette color `c`: every color when `keep` is empty, else those
/// Ignore Color would match to one of `keep`.
fn is_kept(c: [u8; 3], keep: &[[u8; 3]]) -> bool {
    keep.is_empty() || keep.iter().any(|&k| is_ignored(c, Some(k)))
}

/// For each color of `palette`, whether it is left out of every area: Ignore Color's when it
/// leaves out the whole color (`ignore`, `None` for white) and those Keep Colors does not list
/// (`keep`).
pub(crate) fn left_out(palette: &[[u8; 3]], params: &TraceParams, ignore: Option<[u8; 3]>, keep: &[[u8; 3]]) -> Vec<bool> {
    palette.iter().map(|&c| (drops_color(params) && is_ignored(c, ignore)) || !is_kept(c, keep)).collect()
}

/// Whether Ignore Color leaves out every area of its color (not only the background).
pub(crate) fn drops_color(params: &TraceParams) -> bool {
    params.ignore_white && !params.ignore_background
}

/// Ignore Color, background only: clears to [`TRANSPARENT`] every area of a label `ignored` leaves
/// out that reaches the image's edge, through neighbors above, below and to either side. It fills
/// one run of a row at a time, and fills from one edge pixel at a time.
pub(crate) fn clear_background(labels: &mut [u16], w: usize, h: usize, ignored: &[bool]) {
    let (last_row, last_col) = (h.saturating_sub(1) * w, w.saturating_sub(1));
    let mut seeds = Vec::new();
    for edge in (0..w).flat_map(|x| [x, last_row + x]).chain((0..h).flat_map(|y| [y * w, y * w + last_col])) {
        seeds.push(edge);
        fill_left_out(labels, w, h, ignored, &mut seeds);
    }
}

/// Clears to [`TRANSPARENT`] the left-out areas that `seeds` reach, and empties `seeds`.
fn fill_left_out(labels: &mut [u16], w: usize, h: usize, ignored: &[bool], seeds: &mut Vec<usize>) {
    let left_out = |labels: &[u16], i: usize| labels.get(i).is_some_and(|&l| ignored.get(l as usize).copied().unwrap_or(false));
    while let Some(i) = seeds.pop() {
        if !left_out(labels, i) {
            continue;
        }
        // The run of left-out pixels along the row through `i`.
        let row = i - i % w;
        let (mut a, mut b) = (i, i + 1);
        while a > row && left_out(labels, a - 1) {
            a -= 1;
        }
        while b < row + w && left_out(labels, b) {
            b += 1;
        }
        if let Some(run) = labels.get_mut(a..b) {
            run.fill(TRANSPARENT);
        }
        // A seed at the start of each run of left-out pixels next to it in the rows above and below.
        for next in [row.checked_sub(w), Some(row + w).filter(|&r| r < w * h)].into_iter().flatten() {
            let mut inside = false;
            for x in a - row..b - row {
                let here = left_out(labels, next + x);
                if here && !inside {
                    seeds.push(next + x);
                }
                inside = here;
            }
        }
    }
}

/// A box of pixels: `(x0, y0, x1, y1)`, the far sides excluded.
type PixelBox = (usize, usize, usize, usize);

/// Each of `n` palette entries' pixel count in `labels`, and the box its pixels lie in within
/// `traced` (`labels` with Ignore Color's background [`TRANSPARENT`]). A layer is traced within its
/// box, so a color whose background is left out is traced within the box of its remaining areas.
pub(crate) fn label_extents(labels: &[u16], traced: &[u16], w: usize, n: usize) -> (Vec<usize>, Vec<Option<PixelBox>>) {
    let mut counts = vec![0usize; n];
    for &l in labels {
        if let Some(c) = counts.get_mut(l as usize) {
            *c += 1;
        }
    }
    let mut boxes: Vec<Option<PixelBox>> = vec![None; n];
    for (i, &l) in traced.iter().enumerate() {
        if let Some(b) = boxes.get_mut(l as usize) {
            let (x, y) = (i % w, i / w);
            *b = Some(b.map_or((x, y, x + 1, y + 1), |b| (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1))));
        }
    }
    (counts, boxes)
}

/// Trace `img` with `params`. Failures (an image Logo mode refuses, say) give an empty result: use
/// [`trace_within`] to see why.
pub fn trace(img: &Raster, params: &TraceParams) -> TraceResult {
    // No trace has more than `usize::MAX` anchors.
    trace_within(img, params, usize::MAX).unwrap_or_default()
}

/// Trace `img` with `params`, giving up with [`TraceError::TooComplex`] once the paths have more
/// than `max_anchors` anchor points (a photo traced at high fidelity can make millions). The
/// colour layers are traced in parallel where threads are available.
pub fn trace_within(img: &Raster, params: &TraceParams, max_anchors: usize) -> Result<TraceResult, TraceError> {
    trace_watched(img, params, max_anchors, &|_, _| true)
}

/// [`trace_within`], telling `steps` how far it has got: `steps(done, total)` after each stage
/// that prepares the colour layers (`total` 0 while they aren't known yet) and after each colour
/// layer, from whichever thread traced it. Overlapping with Fills (without Create Strokes) counts
/// in finer units instead: a share of `total` after each pass of each layer, in every round of
/// enclosed areas (see `Layers::stacked`). Once `steps` answers `false` the trace stops at the
/// next stage or layer with [`TraceError::Cancelled`].
pub fn trace_watched(
    img: &Raster,
    params: &TraceParams,
    max_anchors: usize,
    steps: &(dyn Fn(u64, u64) -> bool + Sync),
) -> Result<TraceResult, TraceError> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(img.rgba.len()) {
        return Ok(TraceResult::default());
    }
    let mut out = trace_paths(img, params, max_anchors, steps)?;
    if params.auto_grouping {
        // Abutting areas cannot overlap; stacked layers, Logo's shapes and stroked lines can.
        let stacked = params.method == Method::Overlapping || params.mode == Mode::Logo || params.strokes;
        out.parents = group::parents(&out.paths, img.width as usize, img.height as usize, stacked);
    }
    Ok(out)
}

fn trace_paths(img: &Raster, params: &TraceParams, max_anchors: usize, steps: &(dyn Fn(u64, u64) -> bool + Sync)) -> Result<TraceResult, TraceError> {
    let (w, h) = (img.width as usize, img.height as usize);
    // The colors Keep Colors traces (checked in every mode).
    let listed = params
        .keep_colors
        .iter()
        .map(|name| logo::parse_hex(name).ok_or_else(|| TraceError::BadPalette(name.clone())))
        .collect::<Result<Vec<_>, _>>()?;
    // After each stage before the colour layers, whose number isn't known yet.
    let go_on = || if steps(0, 0) { Ok(()) } else { Err(TraceError::Cancelled) };
    if params.mode == Mode::Logo {
        go_on()?;
        return logo::trace_logo(img, params, max_anchors);
    }
    // The colour Ignore Color leaves out, if not white.
    let ignore = match &params.ignore_color {
        Some(name) if params.ignore_white => Some(logo::parse_hex(name).ok_or_else(|| TraceError::BadPalette(name.clone()))?),
        _ => None,
    };
    // A JPEG's 8 x 8 block edges are smoothed away first (see `deblock`).
    let strength = match params.mode {
        Mode::BlackAndWhite => deblock::BLACK_AND_WHITE,
        _ if params.colors > 16 => deblock::MANY_COLORS,
        _ => deblock::FEW_COLORS,
    };
    // A photo is traced from a copy with its noise smoothed away (see `smooth`); the deblocked copy is
    // dropped once it is smoothed.
    let photo = |img: &Raster| params.mode != Mode::BlackAndWhite && quantize::continuous_tone(img);
    let prepared = match deblock::deblocked(img, strength) {
        Some(deblocked) if photo(&deblocked) => Some(smooth::smoothed(&deblocked)),
        Some(deblocked) => Some(deblocked),
        None => photo(img).then(|| smooth::smoothed(img)),
    };
    let img = prepared.as_ref().unwrap_or(img);
    go_on()?;
    // Gradients are found first and kept out of the palette: no colour is spent on bands of them.
    // Grayscale finds them in the image's grays (as its flat colours are), so their stops are gray.
    let grays = (params.gradients && params.mode == Mode::Grayscale).then(|| {
        let mut grays = img.clone();
        for p in grays.rgba.as_chunks_mut::<4>().0 {
            let g = quantize::luma(*p);
            p[..3].fill(g);
        }
        grays
    });
    let traced_as = grays.as_ref().unwrap_or(img);
    let found = (params.gradients && params.mode != Mode::BlackAndWhite).then(|| gradient::detect(traced_as)).filter(|f| !f.1.is_empty());
    go_on()?;
    let without = found.as_ref().map(|(of, _)| {
        let mut rest = img.clone();
        for (p, &g) in rest.rgba.as_chunks_mut::<4>().0.iter_mut().zip(of) {
            if g != u32::MAX {
                p[3] = 0;
            }
        }
        rest
    });
    let (mut q, rare) = quantize::quantize_keeping(without.as_ref().unwrap_or(img), params);
    go_on()?;
    // The semi-transparent edge of a cut-out image traces as the color beside it.
    let blended = if matches!(params.mode, Mode::Color | Mode::Grayscale) {
        quantize::settle_cut_edges(&mut q, img, params.mode == Mode::Grayscale)
    } else {
        Vec::new()
    };
    drop(without);
    // Each gradient is a palette entry of its own (its colour: the gradient's middle).
    let mut gradients: Vec<Option<TracedGradient>> = vec![None; q.palette.len()];
    if let Some((mut of, found)) = found {
        gradient::settle_edges(traced_as, &mut of, &found, &mut q.labels);
        let first = q.palette.len();
        for (l, &g) in q.labels.iter_mut().zip(&of) {
            if g != u32::MAX {
                *l = u16::try_from(first + g as usize).unwrap_or(TRANSPARENT);
            }
        }
        q.palette.extend(found.iter().map(|g| g.color_at(g.start.midpoint(g.end)).map(|v| v.round().clamp(0.0, 255.0) as u8)));
        gradients.extend(found.into_iter().map(Some));
    }
    drop(grays);
    // Noise clears specks, not the regions of a rare colour Automatic kept on purpose.
    let keep: Vec<bool> = q.palette.iter().map(|c| rare.contains(c)).collect();
    // A speck far from the color around it joins the color beside it nearest its own pixels.
    let labs = quantize::palette_labs(&q.palette);
    let source =
        quantize::Source { labs: &labs, rgba: &img.rgba, blended: &blended, gray: matches!(params.mode, Mode::Grayscale | Mode::BlackAndWhite) };
    quantize::denoise_nearest(&mut q.labels, w, h, params.noise as usize, &keep, Some(&source));
    drop(blended);
    go_on()?;
    // Ignore Color, background only: the labels traced, with the ignored color's areas that reach
    // the edge left out. Its other areas are traced with the rest. Edges are still placed by
    // `q.labels`, as when the whole color is left out.
    let cleared = (params.ignore_white && params.ignore_background).then(|| {
        let ignored: Vec<bool> = q.palette.iter().map(|&c| is_ignored(c, ignore)).collect();
        let mut labels = q.labels.clone();
        clear_background(&mut labels, w, h, &ignored);
        labels
    });
    // Whether any of a palette entry's pixels are left after the background.
    let mut remains = vec![false; q.palette.len()];
    for &l in cleared.iter().flatten() {
        if let Some(r) = remains.get_mut(l as usize) {
            *r = true;
        }
    }
    let (counts, boxes) = label_extents(&q.labels, cleared.as_deref().unwrap_or(&q.labels), w, q.palette.len());
    // Layers bottom → top: largest area first.
    let mut order: Vec<usize> = (0..q.palette.len()).filter(|&i| counts[i] > 0).collect();
    order.sort_by(|a, b| counts[*b].cmp(&counts[*a]).then(a.cmp(b)));
    if params.method == Method::Overlapping {
        // Stacked layers go by lightness instead, from the largest colour's end of the scale: each
        // layer reaches under those above it in the areas that hold its own color, so where two
        // colours meet, every layer between them in the stack that has a pixel in that area ends
        // on that same edge and shows through its anti-aliasing. Neighbours in a picture are
        // mostly neighbours in tone, and in this order they are neighbours in the stack too (by
        // area, an unrelated colour's edge would shine through as a hairline).
        let light = |i: usize| q.palette.get(i).map_or(0, |c| u32::from(c[0]) * 299 + u32::from(c[1]) * 587 + u32::from(c[2]) * 114);
        let first = order.first().copied();
        order.sort_by(|a, b| light(*a).cmp(&light(*b)).then(a.cmp(b)));
        if first.is_some_and(|f| order.iter().position(|&i| i == f).is_some_and(|at| at * 2 >= order.len())) {
            order.reverse();
        }
    }
    let mut rank = vec![usize::MAX; q.palette.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    // Layers left out: Ignore Color's (all of a color's areas, unless only its background is left
    // out and some area remains), and those Keep Colors does not list. Abutting, the others are
    // traced as they are in the trace of every color; stacked, they do not reach under the colors
    // left out (see `Layers::ignored`).
    let left_out = left_out(&q.palette, params, ignore, &listed);
    let dropped = |ci: usize| {
        (params.ignore_white && is_ignored(q.palette[ci], ignore) && !remains.get(ci).copied().unwrap_or(false))
            || left_out.get(ci).copied().unwrap_or(false)
    };
    let layers: Vec<(usize, usize)> = order.iter().copied().enumerate().filter(|&(_, ci)| !dropped(ci)).collect();
    // The listed colors that match none of the colors found.
    let keep_unmatched = params
        .keep_colors
        .iter()
        .zip(&listed)
        .filter(|&(_, &k)| !order.iter().any(|&ci| is_kept(q.palette[ci], &[k])))
        .map(|(name, _)| name.clone())
        .collect();
    // Every layer is left out although a listed color matches one: Ignore Color left those out.
    let keep_ignored = !listed.is_empty() && layers.is_empty() && order.iter().any(|&ci| is_kept(q.palette[ci], &listed));
    let job = Layers {
        q: &q,
        traced: cleared.as_deref().unwrap_or(&q.labels),
        rank: &rank,
        params,
        img,
        left_out,
        boxes,
        gradients: &gradients,
        bw_tones: bw_tones(img, &q),
        style: params.outline_style(),
        w,
        h,
        anchors: AtomicUsize::new(0),
        max_anchors,
    };
    let mut out = TraceResult { keep_unmatched, keep_ignored, ..TraceResult::default() };
    // A layer takes about ten bytes a pixel of its box while it is traced (its mask, the labels
    // of its components, the stack that fills them, its edges): as many layers at a time as fit
    // in half a gigabyte, so that a large image is not traced at the cost of gigabytes.
    let largest = job.boxes.iter().flatten().map(|b| (b.2 - b.0) * (b.3 - b.1)).max().unwrap_or(1).max(1);
    let at_a_time = ((512 << 20) / (largest * 10)).max(1);
    // Overlapping with Fills: each shape is filled whole, and what it encloses is traced again above it (see
    // `Layers::stacked`). Lines (Create Strokes) and outlines (no Fills) are stacked as they always were.
    if params.method == Method::Overlapping && params.fills && !params.strokes {
        out.paths = job.stacked(&layers, largest, steps)?;
        // The colors used, bottom layer first.
        for &(_, ci) in &layers {
            if let Some(&color) = q.palette.get(ci)
                && !out.palette.contains(&color)
                && out.paths.iter().any(|p| p.color == color)
            {
                out.palette.push(color);
            }
        }
        return Ok(out);
    }
    let total = layers.len() as u64;
    if !steps(0, total) {
        return Err(TraceError::Cancelled);
    }
    // Once stopped, the layers not started yet are skipped.
    let (done, stopped) = (AtomicU64::new(0), AtomicBool::new(false));
    let traced = parallel_map_at_most(at_a_time, &layers, |&(r, ci)| {
        if stopped.load(Ordering::Relaxed) {
            return None;
        }
        let paths = job.layer(r, ci);
        if !steps(done.fetch_add(1, Ordering::Relaxed) + 1, total) {
            stopped.store(true, Ordering::Relaxed);
        }
        paths
    });
    if stopped.load(Ordering::Relaxed) {
        return Err(TraceError::Cancelled);
    }
    for (paths, &(_, ci)) in traced.into_iter().zip(&layers) {
        let paths = paths.ok_or(TraceError::TooComplex { max: max_anchors })?;
        // A layer with no paths adds no color to the palette.
        if paths.is_empty() {
            continue;
        }
        out.paths.extend(paths);
        let color = q.palette[ci];
        if !out.palette.contains(&color) {
            out.palette.push(color);
        }
    }
    Ok(out)
}

/// The luminance of white and of black in a Black and White quantisation: what its lightest
/// white pixels and its darkest black ones have (the 98th and 2nd percentiles; pure white and
/// black where a side has no pixel). Not the means: where the black is thin lines, blurred or
/// scanned, most black pixels are part paper, their mean is a gray, and read against it the middle
/// of every line would count as more than all ink, now more and now less as the line crosses the
/// pixel rows, which put a ripple on its edges.
pub(crate) fn bw_tones(img: &Raster, q: &Quantized) -> [f64; 2] {
    let mut hist = [[0u64; 256]; 2];
    for (p, &l) in img.rgba.as_chunks::<4>().0.iter().zip(&q.labels) {
        if let Some(h) = hist.get_mut(l as usize) {
            h[((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114 + 500) / 1000).min(255) as usize] += 1;
        }
    }
    // The luminance a share `at` of the side's pixels are darker than.
    let percentile = |h: &[u64; 256], at: f64, none: f64| {
        let total: u64 = h.iter().sum();
        let mut seen = 0;
        for (v, n) in h.iter().enumerate() {
            seen += n;
            if total > 0 && seen as f64 >= at * total as f64 {
                return v as f64;
            }
        }
        none
    };
    [percentile(&hist[0], 0.98, 255.0), percentile(&hist[1], 0.02, 0.0)]
}
