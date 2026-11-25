use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64};
use std::sync::{Arc, Mutex};

use super::params::{Method, Mode, TraceParams};
use super::pipeline::TracedPath;
#[cfg(test)]
use super::pipeline::{bw_tones, left_out};
#[cfg(test)]
use super::quantize;
use super::raster::{Raster, TraceError};
use super::{
    AtomicUsize, Crack, Loop, Ordering, PathData, Quantized, TRANSPARENT, TracedGradient, TracedShape, centerline, contour, fit, logo, shapes,
    trace_mask,
};

/// What tracing one colour layer needs.
pub(crate) struct Layers<'a> {
    pub(crate) q: &'a Quantized,
    /// The label each layer traces a pixel as: `q.labels`, or with Ignore Color for the background
    /// only, those with the background [`TRANSPARENT`]. Edges are placed by `q.labels`.
    pub(crate) traced: &'a [u16],
    /// Each palette entry's layer, bottom (0) up.
    pub(crate) rank: &'a [usize],
    pub(crate) params: &'a TraceParams,
    pub(crate) img: &'a Raster,
    /// For each palette entry, whether Ignore Color or Keep Colors leaves it out (see `pipeline::left_out`).
    pub(crate) left_out: Vec<bool>,
    /// The box each palette entry's pixels lie in: `(x0, y0, x1, y1)`, the far sides excluded.
    pub(crate) boxes: Vec<Option<(usize, usize, usize, usize)>>,
    /// Gradients: the gradient each palette entry is, if it is one.
    pub(crate) gradients: &'a [Option<TracedGradient>],
    /// Black and White: the luminance of white and of black (see `bw_tones`).
    pub(crate) bw_tones: [f64; 2],
    /// How outlines and centre lines are fitted.
    pub(crate) style: logo::Style,
    pub(crate) w: usize,
    pub(crate) h: usize,
    /// Anchor points made so far, by every layer.
    pub(crate) anchors: AtomicUsize,
    pub(crate) max_anchors: usize,
}

impl Layers<'_> {
    /// Ignore White, and the colors Keep Colors does not list: such a color is no part of any
    /// layer (stacked layers reach under the layers above them, and must not reach under a color
    /// that is left out). With `ignore_background` the background is [`TRANSPARENT`] in `traced`,
    /// and the color's other areas are traced.
    fn ignored(&self, label: u16) -> bool {
        self.left_out.get(label as usize).copied().unwrap_or(false)
    }
    /// The region across a crack, for cutting outlines at their junctions: the outside pixel's
    /// colour. Stacked layers end on many colours at once and share no edge: one neighbour.
    fn neighbour(&self, c: Crack) -> u32 {
        if self.params.method == Method::Overlapping {
            return 0;
        }
        c.at(1).and_then(|i| self.q.labels.get(i)).map_or(u32::from(TRANSPARENT), |&l| u32::from(l))
    }
    /// The junction at pixel corner `(x, y)`, if there is one: where three or four regions
    /// meet, or two parts of a region touch by the corner only, or an edge reaches the image's
    /// border. It lies at the corner, moved across by what the edges that run up and down from it
    /// say, and up or down by what those to its left and right say (the mean, where there are
    /// two). It depends on the corner alone, so every region around it gets the same point; on the
    /// image's border it stays on the border.
    fn junction(&self, x: i32, y: i32) -> Option<craft_geom::Point> {
        let label = |px: i32, py: i32| Crack::new((px, py), (1, 0), self.w, self.h).at(0).and_then(|i| self.q.labels.get(i).copied());
        // The four pixels around the corner (`None` off the image), and the edges between them.
        let around = [label(x - 1, y - 1), label(x, y - 1), label(x, y), label(x - 1, y)];
        let edges = (0..4).filter(|&i| around[i] != around[(i + 1) % 4]).count();
        let apart = |a: usize, b: usize, c: usize, d: usize| around[a] == around[c] && around[b] != around[a] && around[d] != around[a];
        let touching = apart(0, 1, 2, 3) || apart(1, 2, 3, 0);
        // (Stacked layers share no edges: they are cut at the image's border only.)
        if edges < 3 && !touching || self.params.method == Method::Overlapping {
            return None;
        }
        // The shift of the crack between `a` and the pixel `step` from it, if both are in the
        // image and of different regions.
        let shift = |a: (i32, i32), step: (i32, i32)| {
            let (la, lb) = (label(a.0, a.1)?, label(a.0 + step.0, a.1 + step.1)?);
            (la != lb).then(|| self.shift(Crack::new(a, step, self.w, self.h)).clamp(-0.5, 0.5))
        };
        let mean = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => (a + b) / 2.0,
            (Some(v), None) | (None, Some(v)) => v,
            (None, None) => 0.0,
        };
        let dx = mean(shift((x - 1, y - 1), (1, 0)), shift((x - 1, y), (1, 0)));
        let dy = mean(shift((x - 1, y - 1), (0, 1)), shift((x, y - 1), (0, 1)));
        Some(craft_geom::Point::new(f64::from(x) + dx, f64::from(y) + dy))
    }
    /// How far the edge lies from a crack between two pixels, towards the outside one (−0.5 to 0.5).
    ///
    /// Each pixel of the column across the crack is read as a blend of the tones on its two sides
    /// (the two pixels' palette colours; in Black and White the luminance of the image's black and of
    /// the white pixels). The edge is where the blends, summed, put it: as far out as there is
    /// inside tone in the outside pixels, less the outside tone in the inside ones (two pixels each
    /// way, while they are on the same side and no more blended than the pixel before: more would be
    /// another edge's doing).
    ///
    /// Where the inside is at most three pixels deep with the outside's colour behind it (a thin
    /// line), its two edges share those pixels and cannot be read apart; the inside tone of the
    /// whole column gives the line's middle (its centroid) and width (its sum) instead. A thin gap
    /// between two parts of the inside is read the same way, with the tones swapped.
    ///
    /// Black and White then moves the edge to where the luminance crosses the threshold: by the
    /// threshold's distance from the middle tone, over how steep the edge is (nothing for a
    /// threshold in the middle, more the softer the edge).
    ///
    /// The estimate is read from each side of the crack and the two readings averaged, so that
    /// the shift is exactly antisymmetric (`a - b == -(b - a)` in floating point) and the two
    /// regions of a crack get the same points. A single reading is not: it looks for a thin run on
    /// the inside before one on the outside, so where both sides are thin (a one- or two-pixel
    /// sliver beside another, as in soft gradients and noisy scans) each region got its own shift,
    /// their shared piece different points, and the abutting curves left a sliver uncovered.
    fn shift(&self, c: Crack) -> f64 {
        (self.shift_one_side(c) - self.shift_one_side(c.flipped())) / 2.0
    }

    /// [`Self::shift`] read from the crack's inside pixel only.
    fn shift_one_side(&self, c: Crack) -> f64 {
        let label = |k: i32| c.at(k).and_then(|i| self.q.labels.get(i).copied()).filter(|&l| l != TRANSPARENT);
        let (Some(la), Some(lb)) = (label(0), label(1)) else { return 0.0 };
        if la == lb {
            return 0.0;
        }
        let bw = self.params.mode == Mode::BlackAndWhite;
        // Black and White: black is the image's black; white is the lightest pixel within a few
        // pixels of the crack, the paper as it is here (a scan's paper is darker in one corner than
        // in another, and read against the image's lightest white it would count as part ink).
        let black = self.bw_tones[1];
        let white = if bw {
            let light = (-3..=4)
                .filter_map(|k| c.at(k).and_then(|i| self.img.rgba.get(i * 4..i * 4 + 3)))
                .map(|p| (f64::from(p[0]) * 299.0 + f64::from(p[1]) * 587.0 + f64::from(p[2]) * 114.0) / 1000.0);
            light.fold(f64::NEG_INFINITY, f64::max).clamp((black + 32.0).min(self.bw_tones[0]), self.bw_tones[0])
        } else {
            self.bw_tones[0]
        };
        let (ca, cb) = if bw {
            let tone = |l: u16| [if l == 1 { black } else { white }; 3];
            (tone(la), tone(lb))
        } else {
            let (Some(ca), Some(cb)) = (self.own_color(la, c, 0), self.own_color(lb, c, 1)) else { return 0.0 };
            (ca, cb)
        };
        let d = [0, 1, 2].map(|k| cb[k] - ca[k]);
        let len2: f64 = d.iter().map(|v| v * v).sum();
        if len2 <= 0.0 {
            return 0.0;
        }
        let gray = self.params.mode == Mode::Grayscale;
        // How much of the inside tone (0 to 1) the pixel `k` steps out has; none off the image.
        // Grayscale reads each pixel as the gray it traces it as.
        let inside = |k: i32| {
            let Some(&[r, g, b, a]) = c.at(k).and_then(|i| self.img.rgba.get(i * 4..i * 4 + 4)) else { return 0.0 };
            let p = if gray { [f64::from(super::quantize::luma([r, g, b, a])); 3] } else { [f64::from(r), f64::from(g), f64::from(b)] };
            let p = if bw { [(p[0] * 299.0 + p[1] * 587.0 + p[2] * 114.0) / 1000.0; 3] } else { p };
            1.0 - ((0..3).map(|k| (p[k] - ca[k]) * d[k]).sum::<f64>() / len2).clamp(0.0, 1.0)
        };
        // The edge from a thin run of `n` pixels starting at pixel `from` and going `dir`, by the
        // amounts `tone` of the run's own tone in it and in the two pixels at either end of it.
        // Returns the run's middle and width, measured from the crack towards the outside.
        let thin = |from: i32, dir: i32, n: i32, tone: &dyn Fn(i32) -> f64| {
            let (near, far) = (from - dir, from + n * dir);
            let mut cells: Vec<(i32, f64)> = (0..n).map(|j| from + j * dir).chain([near, far]).map(|k| (k, tone(k))).collect();
            for (edge, out) in [(near, -dir), (far, dir)] {
                if label(edge + out) == label(edge) {
                    cells.push((edge + out, tone(edge + out).min(tone(edge))));
                }
            }
            let width: f64 = cells.iter().map(|c| c.1).sum();
            // Pixel `k`'s centre is `k - 0.5` from the crack.
            (width > 1e-6).then(|| (cells.iter().map(|c| c.1 * (f64::from(c.0) - 0.5)).sum::<f64>() / width, width))
        };
        // How deep each side is here: up to three pixels of its own label, then the other's.
        let depth = |dir: i32, own: u16, other: u16| {
            let start = if dir < 0 { 0 } else { 1 };
            let n = (1..=3).take_while(|j| label(start + j * dir) == Some(own)).count() as i32 + 1;
            (n <= 3 && label(start + n * dir) == Some(other)).then_some(n)
        };
        let mut steep = 1.0;
        let mut shift = if let Some((middle, width)) = depth(-1, la, lb).and_then(|n| thin(0, -1, n, &inside)) {
            middle + width / 2.0
        } else if let Some((middle, width)) = depth(1, lb, la).and_then(|n| thin(1, 1, n, &|k| 1.0 - inside(k))) {
            middle - width / 2.0
        } else {
            let (a1, b1) = (1.0 - inside(0), inside(1));
            let a2 = (label(-1) == Some(la)).then(|| (1.0 - inside(-1)).min(a1));
            let b2 = (label(2) == Some(lb)).then(|| inside(2).min(b1));
            // The edge's steepness per pixel: the square of the change over two pixels (1 for an
            // anti-aliased edge, less for a blurred one).
            let change = [a2.map(|v| 1.0 - v - b1), b2.map(|v| 1.0 - a1 - v)].into_iter().flatten().fold(f64::NAN, f64::max);
            if !change.is_nan() {
                steep = (change * change).clamp(0.35, 1.0);
            }
            b1 + b2.unwrap_or(0.0) - a1 - a2.unwrap_or(0.0)
        };
        if bw {
            // The share of black at which a pixel turns black.
            let turn = ((white - f64::from(self.params.threshold)) / (white - black).max(1.0)).clamp(0.02, 0.98);
            shift += if la == 1 { 1.0 } else { -1.0 } * (0.5 - turn) / steep;
        }
        shift
    }

    /// The color of label `l` at the pixel `k` steps out from the inside pixel of crack `c`: its
    /// palette entry's, or its gradient's color there.
    fn own_color(&self, l: u16, c: Crack, k: i32) -> Option<[f64; 3]> {
        let flat = self.q.palette.get(l as usize)?.map(f64::from);
        let Some(g) = self.gradients.get(l as usize).and_then(Option::as_ref) else { return Some(flat) };
        let i = c.at(k)?;
        Some(g.color_at(craft_geom::Point::new((i % self.w) as f64 + 0.5, (i / self.w) as f64 + 0.5)))
    }

    /// What the image does across a crack, for `subpixel::low_contrast`. Distances between colors
    /// are RGB distances (0 to 255 a channel), and Grayscale reads each pixel as the gray it traces
    /// it as. In order:
    ///
    /// - the share of the step from the inside color to the outside one that the image makes near
    ///   the crack, measured along the line between the two colors: from the mean of the two pixels
    ///   behind the inside pixel to the mean of the two beyond the outside one, each side only as
    ///   far as its own label goes;
    /// - the share further out, from the next two pixels on one side to the next two on the other,
    ///   whatever their labels (not a number where a side has none);
    /// - the near change along that line, as a distance;
    /// - the whole near change, in any direction, as a distance;
    /// - how much the image changes within each side: the distance between the two pixels behind
    ///   the inside pixel and between the two beyond the outside one, where both have the side's
    ///   label, averaged (not a number where neither side has two such pixels).
    ///
    /// Where a side has no pixel near the crack, and in Black and White, whose two tones are not the
    /// colors of its areas, it is [`subpixel::EDGE_STEP`]. Exactly the same read from either side.
    fn step(&self, c: Crack) -> [f64; 5] {
        use super::subpixel::EDGE_STEP;
        let label = |k: i32| c.at(k).and_then(|i| self.q.labels.get(i).copied()).filter(|&l| l != TRANSPARENT);
        let (Some(la), Some(lb)) = (label(0), label(1)) else { return EDGE_STEP };
        if la == lb || self.params.mode == Mode::BlackAndWhite {
            return EDGE_STEP;
        }
        let (Some(ca), Some(cb)) = (self.own_color(la, c, 0), self.own_color(lb, c, 1)) else { return EDGE_STEP };
        let d = [0, 1, 2].map(|k| cb[k] - ca[k]);
        let len2: f64 = d.iter().map(|v| v * v).sum();
        if len2 <= 0.0 {
            return EDGE_STEP;
        }
        let gray = self.params.mode == Mode::Grayscale;
        // The pixel `k`, while it is in the image and not transparent (and has label `l`, if one is
        // given).
        let pixel = |k: i32, l: Option<u16>| {
            label(k).filter(|&x| l.is_none_or(|l| x == l))?;
            let &[r, g, b, a] = c.at(k).and_then(|i| self.img.rgba.get(i * 4..i * 4 + 4))? else { return None };
            Some(if gray { [f64::from(super::quantize::luma([r, g, b, a])); 3] } else { [f64::from(r), f64::from(g), f64::from(b)] })
        };
        // The mean color of the pixels `ks` up to the first one `pixel` refuses.
        let mean = |ks: [i32; 2], l: Option<u16>| {
            let (mut sum, mut n) = ([0.0; 3], 0.0);
            for p in ks.iter().map_while(|&k| pixel(k, l)) {
                for (s, v) in sum.iter_mut().zip(p) {
                    *s += v;
                }
                n += 1.0;
            }
            (n > 0.0).then(|| sum.map(|s| s / n))
        };
        let distance = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (b[k] - a[k]).powi(2)).sum::<f64>().sqrt();
        // From `a` to `b` along the line from the inside color to the outside one, in its lengths.
        let along = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (b[k] - a[k]) * d[k]).sum::<f64>() / len2;
        let (Some(a), Some(b)) = (mean([-1, -2], Some(la)), mean([2, 3], Some(lb))) else { return EDGE_STEP };
        let far = match (mean([-3, -4], None), mean([4, 5], None)) {
            (Some(a), Some(b)) => along(a, b),
            _ => f64::NAN,
        };
        let (mut within, mut sides) = (0.0, 0.0);
        for ([j, k], l) in [([-1, -2], la), ([2, 3], lb)] {
            if let (Some(p), Some(q)) = (pixel(j, Some(l)), pixel(k, Some(l))) {
                within += distance(p, q);
                sides += 1.0;
            }
        }
        let within = if sides > 0.0 { within / sides } else { f64::NAN };
        let near = along(a, b);
        [near, far, near * len2.sqrt(), distance(a, b), within]
    }

    /// Whether the pixels inside `l` (a loop along pixel edges) all have one label.
    fn one_colour_within(&self, l: &Loop) -> bool {
        let mut first = None;
        let (y0, y1) = l.pts.iter().fold((i32::MAX, i32::MIN), |m, p| (m.0.min(p.1), m.1.max(p.1)));
        for y in y0..y1 {
            // The loop's upright edges that pass this row of pixels, left to right: the pixels
            // between the first and the second are inside, and so on.
            let mut xs: Vec<i32> = l
                .pts
                .iter()
                .zip(l.pts.iter().cycle().skip(1))
                .filter(|(a, b)| a.0 == b.0 && a.1.min(b.1) <= y && y < a.1.max(b.1))
                .map(|(a, _)| a.0)
                .collect();
            xs.sort_unstable();
            for pair in xs.as_chunks::<2>().0 {
                for x in pair[0]..pair[1] {
                    let label = usize::try_from(y)
                        .ok()
                        .zip(usize::try_from(x).ok())
                        .and_then(|(y, x)| self.q.labels.get(y * self.w + x).copied().filter(|_| x < self.w));
                    if label.is_none() || *first.get_or_insert(label) != label {
                        return false;
                    }
                }
            }
        }
        first.is_some()
    }

    /// Shapes, Abutting: the shape the outline `sp` (fitted from the loop `l`, `hard` as that
    /// loop is) is, where it may be one. The edge is shared by the regions on its two sides, so
    /// both must become that same shape or neither: only a closed edge between two regions, a
    /// region whose whole outline is the hole of one other (no junction on it, not on the image's
    /// border), and only a shape inside the image. The shape is read from the outline run one way
    /// from one start, so the region and the hole it fills get the very same one.
    fn shared_shape(&self, l: &Loop, sp: &craft_geom::SubPath, hard: bool) -> Option<TracedShape> {
        let (w, h) = (i32::try_from(self.w).ok()?, i32::try_from(self.h).ok()?);
        if l.pts.iter().any(|&(x, y)| x <= 0 || y <= 0 || x >= w || y >= h || self.junction(x, y).is_some()) {
            return None;
        }
        let mut one_way = sp.clone();
        if one_way.signed_area() < 0.0 {
            one_way.reverse();
        }
        let first = one_way.anchors.iter().enumerate().min_by(|a, b| a.1.p.x.total_cmp(&b.1.p.x).then(a.1.p.y.total_cmp(&b.1.p.y)))?.0;
        one_way.anchors.rotate_left(first);
        let shape = shapes::recognise(&one_way, hard)?;
        let (wf, hf) = (self.w as f64, self.h as f64);
        let inside = shape
            .path()
            .subpaths
            .iter()
            .flat_map(|s| &s.anchors)
            .flat_map(|a| [a.p, a.h_in, a.h_out])
            .all(|p| (0.0..=wf).contains(&p.x) && (0.0..=hf).contains(&p.y));
        inside.then_some(shape)
    }

    /// The paths of layer `r` (palette entry `ci`), or `None` once the anchors made pass the
    /// maximum.
    pub(crate) fn layer(&self, r: usize, ci: usize) -> Option<Vec<TracedPath>> {
        let params = self.params;
        let color = *self.q.palette.get(ci)?;
        // Create Strokes: this colour's lines, left out of its areas.
        let (thin, lines) = if params.strokes {
            // The ink the lines are read against: the layer's colour; Black and White's own black.
            let ink = if params.mode == Mode::BlackAndWhite { [self.bw_tones[usize::from(ci == 1)]; 3] } else { color.map(f64::from) };
            centerline::lines(self.traced, u16::try_from(ci).unwrap_or(TRANSPARENT), self.w, self.h, params.stroke_width, self.img, ink, &self.style)
        } else {
            (vec![], vec![])
        };
        let comps = self.contours(r, ci, &thin, None, &[])?;
        self.fit_layer(ci, &comps, lines)
    }

    /// Whether pixel `(x, y)` is one that the layers trace: in the image, and neither transparent nor a color left out.
    fn live(&self, x: usize, y: usize) -> bool {
        x < self.w && self.traced.get(y * self.w + x).is_some_and(|&l| l != TRANSPARENT && !self.ignored(l))
    }

    /// The highest level of the area `a` whose mask holds every pixel along the inside of its edge (the image's
    /// border, or the hole's outline), or 0 when a pixel there is not one the layers trace. That level's shape may
    /// run around the whole edge and enclose the rest of the area, so [`Self::stacked`] finds its components (and those
    /// of the levels above it) first.
    fn rim_level(&self, a: &Area) -> usize {
        // The lowest rank along the edge; `None` once a pixel there is not traced.
        let mut lowest = Some(usize::MAX);
        let mut see = |x: i32, y: i32| {
            let rank = usize::try_from(x).ok().zip(usize::try_from(y).ok()).filter(|&(x, y)| self.live(x, y)).and_then(|(x, y)| {
                let l = *self.traced.get(y * self.w + x)?;
                self.rank.get(l as usize).copied()
            });
            lowest = lowest.zip(rank).map(|(a, b)| a.min(b));
            lowest.is_some()
        };
        match &a.outline {
            None => {
                let (w, h) = (i32::try_from(self.w).unwrap_or(0), i32::try_from(self.h).unwrap_or(0));
                // (Rows first, then the columns between them.)
                let _ = (0..w).all(|x| see(x, 0) && see(x, h - 1)) && (1..h - 1).all(|y| see(0, y) && see(w - 1, y));
            }
            Some(l) => {
                // A hole's outline has the shape's pixels on its right and the area's on its left (see `contour`).
                let _ = l.pts.iter().zip(l.pts.iter().cycle().skip(1)).all(|(&(x0, y0), &(x1, y1))| {
                    let (dx, dy) = ((x1 - x0).signum(), (y1 - y0).signum());
                    // (Outlines run along pixel edges: never both ways at once.)
                    if dx != 0 && dy != 0 {
                        return false;
                    }
                    let mut p = (x0, y0);
                    while p != (x1, y1) {
                        let left = match (dx, dy) {
                            (1, _) => (p.0, p.1 - 1),
                            (_, 1) => (p.0, p.1),
                            (-1, _) => (p.0 - 1, p.1),
                            _ => (p.0 - 1, p.1 - 1),
                        };
                        if !see(left.0, left.1) {
                            return false;
                        }
                        p = (p.0 + dx, p.1 + dy);
                    }
                    true
                });
            }
        }
        let Some(lowest) = lowest.filter(|&r| r != usize::MAX) else { return 0 };
        a.levels.iter().rposition(|&(r, _)| r <= lowest).unwrap_or(0)
    }

    /// The components of layer `r`'s mask (palette entry `ci`), in image pixels: `thin` pixels (Create Strokes' lines)
    /// are left out, and `within` an area, only that area's pixels are in it.
    ///
    /// The pixels inside `covered` (holes that higher levels fill, so that their shapes cover them) are left out too:
    /// a component that reaches into one has it as a hole instead, which [`Self::fill_layer`] drops, and its other
    /// loops are the same either way. A layer with no pixel of its own color left has nothing to trace.
    fn contours(&self, r: usize, ci: usize, thin: &[bool], within: Option<&Area>, covered: &[&Loop]) -> Option<Vec<contour::Component>> {
        let overlapping = self.params.method == Method::Overlapping;
        let w = self.w;
        // The layer is traced within the box its pixels lie in, not the whole image.
        let part = |l: u16| if overlapping { self.rank.get(l as usize).is_some_and(|&lr| lr >= r) && !self.ignored(l) } else { l as usize == ci };
        let (mut x0, mut y0, mut x1, mut y1) = (w, self.h, 0, 0);
        for (l, b) in self.boxes.iter().enumerate() {
            if let Some(b) = b.filter(|_| part(l as u16)) {
                (x0, y0, x1, y1) = (x0.min(b.0), y0.min(b.1), x1.max(b.2), y1.max(b.3));
            }
        }
        if let Some(a) = within {
            (x0, y0, x1, y1) = (x0.max(a.bx.0), y0.max(a.bx.1), x1.min(a.bx.2), y1.min(a.bx.3));
        }
        if x0 >= x1 || y0 >= y1 {
            return Some(vec![]);
        }
        let (bw, bh) = (x1 - x0, y1 - y0);
        // Each row's runs of pixels that may be in the mask: in the area, outside what is covered.
        let mut mask = vec![false; bw * bh];
        // (Whether any pixel of the layer's own color is in it: none, and the layer has nothing to trace.)
        let (mut own_color, mut off_image) = (!overlapping, false);
        row_runs(x0, x1, y0, bh, within.and_then(|a| a.outline.as_ref()), covered, |k, runs| {
            let y = y0 + k;
            for &(a, b) in runs {
                let (Some(row), Some(out)) = (self.traced.get(y * w + a..y * w + b), mask.get_mut(k * bw + a - x0..k * bw + b - x0)) else {
                    off_image = true;
                    return;
                };
                for (j, (m, &l)) in out.iter_mut().zip(row).enumerate() {
                    let i = y * w + a + j;
                    *m = l != TRANSPARENT && !thin.get(i).copied().unwrap_or(false) && part(l);
                    own_color |= *m && self.q.labels.get(i).is_some_and(|&l| l as usize == ci);
                }
            }
        });
        if off_image {
            return None;
        }
        if !own_color {
            return Some(vec![]);
        }
        let mut comps = trace_mask(&mask, bw, bh);
        drop(mask);
        // Overlapping: a part of the mask with no pixel of this color holds only colors above it,
        // whose own layers fill it, and is left out. The layer reaches under the colors above it
        // only within the areas that hold its own color. (Leaving a component out changes no other
        // component's loops: a walk turns by the pixels of its own component only.)
        if overlapping {
            let own = |x: usize, y: usize| self.q.labels.get((y0 + y) * w + x0 + x).is_some_and(|&l| l as usize == ci);
            comps.retain(|c| !pixels_of(c, |x, y| !own(x, y)));
        }
        for comp in &mut comps {
            for l in std::iter::once(&mut comp.outer).chain(&mut comp.holes) {
                for p in &mut l.pts {
                    *p = (p.0 + x0 as i32, p.1 + y0 as i32);
                }
            }
        }
        Some(comps)
    }

    /// The first pass of [`Self::stacked`] at level `level` of the area `a`: marks in `round.marks` the pixels inside
    /// each hole of `comps` (that level's components) that its shape fills. A shape fills a hole of at least Noise
    /// pixels when every pixel inside it is one the area traces (no transparent pixel, no color left out). Smaller
    /// holes are filled with the shape's own color, as they always were. A hole inside an area that this level or a
    /// higher one has already marked is passed over: every pixel in it is marked, and the component around it lies
    /// inside that area, traced with it. Whether a hole holds a pixel that is not traced is read from `round.dead` a
    /// run at a time, not a pixel at a time. Returns the holes marked.
    fn mark_filled(&self, round: &Round, a: &Area, level: usize, comps: &[contour::Component]) -> Vec<Loop> {
        let Some(mark) = u16::try_from(level + 1).ok().filter(|_| a.fill) else { return vec![] };
        let min_hole = i64::from(self.params.noise.max(1));
        let all_live = |hole: &Loop| {
            let Some(dead) = round.dead else { return true };
            runs_inside([hole], |y, a, b| {
                #[cfg(test)]
                round.live_reads.fetch_add(1, Ordering::Relaxed);
                let row = y.saturating_mul(self.w);
                !meets(dead, row.saturating_add(a), row.saturating_add(b))
            })
        };
        let mut marked = vec![];
        for hole in comps.iter().flat_map(|c| &c.holes) {
            if self.params.noise > 0 && hole.area2.abs() / 2 < min_hole
                || first_pixel(hole).and_then(|(x, y)| round.marks.get(y * self.w + x)).is_none_or(|m| m.load(Ordering::Relaxed) >= mark)
                || !all_live(hole)
            {
                continue;
            }
            interior(hole, |x, y| {
                if let Some(m) = round.marks.get(y * self.w + x) {
                    m.fetch_max(mark, Ordering::Relaxed);
                }
                true
            });
            marked.push(hole.clone());
        }
        marked
    }

    /// The second pass of [`Self::stacked`] at level `level` (palette entry `ci`) of the area `a`, once every level of
    /// the round has marked what it fills: the paths of `comps` (from the first pass), and the holes that the shapes
    /// fill (see [`Self::inside`]).
    ///
    /// An area filled at a higher level is covered by that level's shape, which this one lies under: it is no hole
    /// here, and a component whose own color lies only inside such areas is left out. A component inside an area filled
    /// at any level is traced with that area. A hole this level fills is no hole. Every other hole stays.
    ///
    /// `covered`: whether a higher level of the area has filled a hole (without one, every component holds its own
    /// color outside such areas, as `contours` keeps only those that hold it).
    fn fill_layer(
        &self,
        round: &Round,
        a: &Area,
        level: usize,
        ci: usize,
        mut comps: Vec<contour::Component>,
        covered: bool,
    ) -> Option<(Vec<TracedPath>, Vec<Loop>)> {
        let mark = u16::try_from(level + 1).unwrap_or(u16::MAX);
        let marked = |(x, y): (usize, usize)| round.marks.get(y * self.w + x).map_or(0, |m| m.load(Ordering::Relaxed));
        comps.retain(|comp| {
            first_pixel(&comp.outer).is_some_and(|p| marked(p) == 0)
                && !(covered && pixels_of(comp, |x, y| self.q.labels.get(y * self.w + x).is_none_or(|&l| l as usize != ci) || marked((x, y)) > mark))
        });
        let mut filled = vec![];
        for comp in &mut comps {
            for hole in std::mem::take(&mut comp.holes) {
                match first_pixel(&hole).map(marked) {
                    Some(m) if m > mark => {}
                    Some(m) if m == mark && a.fill => filled.push(hole),
                    _ => comp.holes.push(hole),
                }
            }
            // The pixels the shape fills (the same whichever covered areas `contours` left out).
            let area2 = comp.holes.iter().fold(comp.outer.area2, |a, h| a + h.area2);
            comp.pixels = usize::try_from(area2 / 2).unwrap_or(0);
        }
        Some((self.fit_layer(ci, &comps, vec![])?, filled))
    }

    /// What [`Self::stacked`] draws inside the hole `hole` that a shape fills: an area of several colors, traced in the
    /// next round, or the paths of an area of one color, traced here (as that round would trace it: its one level's
    /// shape is the whole area, without holes). `None` once the anchors made pass the maximum.
    fn inside(&self, hole: Loop) -> Option<Inside> {
        let Some(area) = self.area_in(hole) else { return Some(Inside::Done(vec![])) };
        let &[(_, ci)] = area.levels.as_slice() else { return Some(Inside::Next(area)) };
        let comps = area.outline.as_ref().map(shapes_in_hole).unwrap_or_default();
        Some(Inside::Done(self.fit_layer(ci, &comps, vec![])?))
    }

    /// The area inside the hole `hole`, with its box and the colors in it (`None` when it holds no pixel of a color).
    fn area_in(&self, hole: Loop) -> Option<Area> {
        let mut present = vec![false; self.q.palette.len()];
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        runs_inside([&hole], |y, a, b| {
            for &l in self.traced.get(y * self.w + a..y * self.w + b).unwrap_or_default() {
                if let Some(p) = present.get_mut(l as usize) {
                    *p = true;
                }
            }
            (x0, y0, x1, y1) = (x0.min(a), y0.min(y), x1.max(b), y1.max(y + 1));
            true
        });
        let mut levels: Vec<(usize, usize)> =
            present.iter().enumerate().filter(|&(_, &p)| p).filter_map(|(ci, _)| Some((*self.rank.get(ci)?, ci))).collect();
        levels.sort_unstable();
        (x0 < x1 && !levels.is_empty()).then_some(Area { bx: (x0, y0, x1, y1), outline: Some(hole), levels, fill: true })
    }

    /// The paths of the components `comps` of palette entry `ci`'s layer, then of its `lines` (Create Strokes), or
    /// `None` once the anchors made pass the maximum.
    fn fit_layer(&self, ci: usize, comps: &[contour::Component], lines: Vec<centerline::Line>) -> Option<Vec<TracedPath>> {
        let params = self.params;
        let color = *self.q.palette.get(ci)?;
        let overlapping = params.method == Method::Overlapping;
        let (w, h) = (self.w, self.h);
        // Without Fills, areas are outlined with a 1 px stroke.
        let area_stroke = (params.strokes && !params.fills).then_some(1.0);
        let min_hole = params.noise.max(1) as i64;
        let mut paths = vec![];
        let mut add = |path: PathData, pixels: usize, stroke: Option<f64>, shape: Option<TracedShape>, square_caps: bool| {
            let n = path.anchor_count();
            if self.anchors.fetch_add(n, Ordering::Relaxed).saturating_add(n) > self.max_anchors {
                return false;
            }
            // (A gradient fills; a stroked line of it is its middle colour.)
            let gradient = self.gradients.get(ci).cloned().flatten().filter(|_| stroke.is_none());
            paths.push(TracedPath { path, color, pixels, stroke, shape, gradient, square_caps });
            true
        };
        // One component's paths: a pure function of its loops and the shared read-only state, so components are fitted in
        // parallel and then added in order.
        // A loop past the crack cap refuses the whole layer: skipping it would silently drop art.
        let too_many_cracks = AtomicBool::new(false);
        let fit_comp = |comp: &contour::Component| -> Option<(PathData, Option<TracedShape>)> {
            // (`hard`: some of the outline has no anti-aliasing, and is known less exactly.)
            let hard = std::cell::Cell::new(false);
            // (Each loop fitted, and whether its own outline is hard.)
            let fit = |l: &Loop| {
                if cracks_of(l) > MAX_LOOP_CRACKS {
                    too_many_cracks.store(true, Ordering::Relaxed);
                    return None;
                }
                let (sp, is_hard) =
                    fit::fit_loop(l, w, h, &|c| (self.shift(c), self.neighbour(c), self.step(c)), &|x, y| self.junction(x, y), &self.style)?;
                // Every trace straightens the pieces of an outline that lie close to a line (Snap
                // Curves To Lines straightens more of them), so the outline of a single pixel or
                // of a region one pixel thick can have all its pieces on one chord, and the loop
                // then has no area. Filled, such a loop paints nothing, and it is left out.
                // Without Fills its 1 px stroke draws it, so it is kept. (`fit::fit_closed` puts
                // back the positioned outline of a loop that is one closed piece when its fit has
                // no area.) The area is summed about the loop's first anchor.
                if area_stroke.is_none() {
                    use kurbo::ParamCurveArea;
                    let origin = sp.anchors.first()?.p.to_vec2();
                    let area: f64 = (0..sp.segment_count()).map(|i| (kurbo::Affine::translate(-origin) * sp.segment(i)).signed_area()).sum();
                    if area.abs() < 1e-6 {
                        return None;
                    }
                }
                hard.set(hard.get() || is_hard);
                Some((sp, is_hard))
            };
            let (outer, _) = fit(&comp.outer)?;
            let mut subs = vec![outer];
            for hole in &comp.holes {
                if hole.area2.abs() / 2 < min_hole && params.noise > 0 {
                    continue;
                }
                let Some((mut sp, own_hard)) = fit(hole) else { continue };
                // Shapes: a hole that is a shape is that shape, as what fills it is (the two are
                // one outline: they stay one). Only where one colour fills it: the parts of
                // several are not that shape, and would no longer meet it. Abutting, only where the
                // region that fills it is that shape too (see `shared_shape`).
                let exact = if !params.shapes || area_stroke.is_some() || !self.one_colour_within(hole) {
                    None
                } else if overlapping {
                    shapes::recognise(&sp, hard.get())
                } else {
                    // (As hard as this outline is, as the region that fills it reads it.)
                    self.shared_shape(hole, &sp, own_hard)
                };
                if let Some(exact) = exact.and_then(|shape| shape.path().subpaths.into_iter().next()) {
                    // (Which way round each goes: by the area of the polygon through points along it.)
                    let turn = |sp: &craft_geom::SubPath| {
                        use craft_geom::ParamCurve;
                        let pts: Vec<_> =
                            (0..sp.segment_count()).flat_map(|i| (0..6).map(move |k| sp.segment(i).eval(f64::from(k) / 6.0).to_vec2())).collect();
                        pts.iter().zip(pts.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b)).sum::<f64>()
                    };
                    let mut exact = exact;
                    if (turn(&exact) < 0.0) != (turn(&sp) < 0.0) {
                        exact.reverse();
                    }
                    sp = exact;
                }
                subs.push(sp);
            }
            // Shapes: a filled outline without holes that is a shape becomes that shape.
            let shape = match subs.as_slice() {
                [only] if params.shapes && area_stroke.is_none() && overlapping => shapes::recognise(only, hard.get()),
                // Abutting: only an island in one other region, whose hole it is (see `shared_shape`).
                [only] if params.shapes && area_stroke.is_none() && comp.holes.is_empty() => self.shared_shape(&comp.outer, only, hard.get()),
                _ => None,
            };
            let path = shape.map_or_else(|| PathData::new(subs), |s| s.path());
            Some((path, shape))
        };
        // In chunks, so that the anchor budget still stops the work early: a hostile image is refused after fitting at most
        // the rest of one chunk, not after every component is fitted.
        let accepted = fit_in_chunks(comps, FIT_CHUNK, fit_comp, |comp, fitted| {
            let Some((path, shape)) = fitted else { return true };
            add(path, comp.pixels, area_stroke, shape, false)
        });
        if !accepted || too_many_cracks.load(Ordering::Relaxed) {
            return None;
        }
        for line in lines {
            if !add(line.path, line.pixels, Some(line.width), None, line.square) {
                return None;
            }
        }
        Some(paths)
    }

    /// Overlapping with Fills (and without Create Strokes): the stacked layers of `layers` (`(rank, palette entry)`,
    /// bottom up), with each shape filled whole. `largest` is the largest of the layers' boxes, in pixels.
    ///
    /// A layer's shape covers the colors above it in its area, so a part of other colors that it encloses is a hole in
    /// it, and every layer whose shape encloses that part would otherwise have its own copy of the same outline.
    /// Instead the shape fills the hole, and the area inside the hole is traced again in the next round, as the whole
    /// image is, and drawn after everything around it. A hole stays where something inside it must stay unpainted (a
    /// transparent pixel, a color left out).
    ///
    /// Each round takes two passes over its levels, each pass all levels in parallel. The first finds each level's
    /// components and marks the pixels inside the holes it fills. The second leaves out what lies inside a marked area
    /// (traced in the next round), treats an area filled at a higher level as covered (no hole, and no part of a lower
    /// shape whose own color lies only inside it), and fits the rest. Every pixel inside a filled area is then fitted
    /// in that area's round only. An area of one color is traced as soon as its hole is known (its one shape has no
    /// holes), so only areas of several colors make another round. The levels that hold every pixel along an area's
    /// edge go first in the first pass (see [`Self::rim_level`]).
    ///
    /// The pixels of nested areas are scanned once more in each round that holds them: past a budget of four times the
    /// work of the first round (of a 256 × 256 image, at least), an area keeps its holes and ends its branch (an image
    /// of thousands of nested rings). An area is charged its box times its levels only when it fits in what is left; an
    /// area that does not fit is charged nothing.
    ///
    /// The marks take two bytes a pixel of the image. In an image with a transparent pixel or a color left out, the
    /// runs of such pixels take up to four bytes a pixel more during the first round (eight bytes a run). Both count
    /// against the half gigabyte that sets how many levels are traced at a time. The components kept for the second pass
    /// take up to [`KEPT_BYTES`] more, outside that half gigabyte.
    pub(crate) fn stacked(
        &self,
        layers: &[(usize, usize)],
        largest: usize,
        steps: &(dyn Fn(u64, u64) -> bool + Sync),
    ) -> Result<Vec<TracedPath>, TraceError> {
        let n = self.w * self.h;
        let progress = Progress { steps, done: AtomicU64::new(0), total: 3 * layers.len().max(1) as u64 * STEP, stopped: AtomicBool::new(false) };
        if !progress.add(0) {
            return Err(TraceError::Cancelled);
        }
        let too_complex = || TraceError::TooComplex { max: self.max_anchors };
        // Two bytes a pixel for the marks.
        let marks: Vec<AtomicU16> = (0..n).map(|_| AtomicU16::new(0)).collect();
        let untraced = |l: u16| l == TRANSPARENT || self.ignored(l);
        // Eight bytes a run of untraced pixels, at most four a pixel (where every other pixel is one).
        let mut dead = self.traced.iter().any(|&l| untraced(l)).then(|| dead_runs(self.traced, untraced));
        let dead_bytes = dead.as_ref().map_or(0, |runs| runs.capacity().saturating_mul(size_of::<(u32, u32)>()));
        #[cfg(test)]
        let probe = PROBE.with(|p| {
            let mut p = p.borrow_mut();
            (p.live_reads, p.areas, p.dead_bytes) = (0, vec![], dead_bytes);
            p.clone()
        });
        #[cfg(test)]
        let (live_reads, mut decided) = (AtomicUsize::new(0), vec![]);
        #[cfg(test)]
        let mut round_number = 0usize;
        let mut areas = vec![Area { bx: (0, 0, self.w, self.h), outline: None, levels: layers.to_vec(), fill: true }];
        // The areas as a tree: each one's paths and the areas inside it.
        let mut tree: Vec<(Vec<TracedPath>, Vec<usize>)> = vec![];
        let mut nodes = vec![0usize];
        tree.push((vec![], vec![]));
        let mut budget = 4 * n.max(1 << 16) as u64 * layers.len().max(1) as u64;
        #[cfg(test)]
        if let Some(b) = probe.budget {
            budget = b;
        }
        // What each round's levels may add to the progress, in units: the first round's levels a step each per pass,
        // and every later round half of what is left.
        let mut pot = layers.len() as u64 * STEP;
        let mut first = true;
        while !areas.is_empty() {
            // The levels of every area, the highest first: a level's components are found after the levels above it
            // have marked what they fill, where they could start first, and leave those areas out. The levels at and
            // above an area's rim level (`rim_level`) go first of all, and the others start once they have marked: a
            // shape that runs around the whole area (a frame) may enclose nearly all of it.
            let rims: Vec<usize> = areas.iter().map(|a| self.rim_level(a)).collect();
            let early = |&(k, i): &(usize, usize)| rims.get(k).is_some_and(|&r| r > 0 && i >= r);
            let mut jobs: Vec<(usize, usize)> = areas.iter().enumerate().flat_map(|(k, a)| (0..a.levels.len()).map(move |i| (k, i))).collect();
            jobs.sort_by_key(|job| (!early(job), std::cmp::Reverse(job.1), job.0));
            let first_part = jobs.iter().take_while(|job| early(job)).count();
            // The holes each level has marked, once it has.
            let published: Mutex<Vec<Marked>> = Mutex::new(vec![]);
            let per_job = if first {
                STEP
            } else {
                let share = pot / 2;
                pot -= share;
                share / 2 / jobs.len().max(1) as u64
            };
            let round = Round {
                marks: &marks,
                dead: dead.as_deref().filter(|_| first),
                #[cfg(test)]
                live_reads: &live_reads,
            };
            #[cfg(test)]
            {
                round_number += 1;
            }
            // A level takes about ten bytes a pixel of its box while its components are found (as `pipeline` counts
            // them): as many at a time as fit in half a gigabyte with the marks and the runs of untraced pixels.
            let biggest =
                jobs.iter().filter_map(|&(k, _)| areas.get(k)).map(|a| (a.bx.2 - a.bx.0) * (a.bx.3 - a.bx.1)).max().unwrap_or(1).min(largest).max(1);
            let held = if first { dead_bytes } else { 0 };
            let at_a_time = ((512usize << 20).saturating_sub(2 * n).saturating_sub(held) / (biggest * 10)).max(1);
            // Threads cost time to start: one for each `PIXELS_PER_THREAD` pixels of the levels' boxes (the later rounds
            // often hold hundreds of small areas).
            let work = jobs.iter().filter_map(|&(k, _)| areas.get(k)).fold(0usize, |s, a| s.saturating_add((a.bx.2 - a.bx.0) * (a.bx.3 - a.bx.1)));
            let at_a_time = at_a_time.min(work.div_ceil(PIXELS_PER_THREAD).max(1));
            #[cfg(test)]
            let at_a_time = if probe.one_level_at_a_time { 1 } else { at_a_time };
            // The components found are kept for the second pass while they take at most `KEPT_BYTES`; past that, a
            // level's are dropped and found again in the second pass.
            let kept = AtomicUsize::new(0);
            let find = |&(k, i): &(usize, usize)| {
                let (a, &(r, ci)) = areas.get(k).and_then(|a| Some((a, a.levels.get(i)?)))?;
                if progress.stopped.load(Ordering::Relaxed) {
                    return None;
                }
                let above: Vec<Arc<Vec<Loop>>> =
                    published.lock().map(|p| p.iter().filter(|e| e.0 == k && e.1 > i).map(|e| Arc::clone(&e.2)).collect()).unwrap_or_default();
                let covered: Vec<&Loop> = above.iter().flat_map(|l| l.iter()).collect();
                #[cfg(test)]
                let covered = if probe.keep_covered { vec![] } else { covered };
                let comps = self.contours(r, ci, &[], Some(a), &covered)?;
                let marked = self.mark_filled(&round, a, i, &comps);
                if !marked.is_empty()
                    && let Ok(mut p) = published.lock()
                {
                    p.push((k, i, Arc::new(marked)));
                }
                progress.add(per_job);
                let bytes = comps.iter().fold(0usize, |s, c| {
                    std::iter::once(&c.outer).chain(&c.holes).fold(s.saturating_add(size_of::<contour::Component>()), |s, l| {
                        s.saturating_add(size_of::<Loop>()).saturating_add(l.pts.len().saturating_mul(size_of::<(i32, i32)>()))
                    })
                });
                let keep = kept.fetch_add(bytes, Ordering::Relaxed).saturating_add(bytes) <= KEPT_BYTES;
                #[cfg(test)]
                let keep = keep && !probe.keep_none;
                Some(Mutex::new(keep.then_some(comps)))
            };
            let (early_jobs, later_jobs) = jobs.split_at(first_part);
            let mut found = parallel_map_at_most(at_a_time, early_jobs, find);
            found.extend(parallel_map_at_most(at_a_time, later_jobs, find));
            if progress.stopped.load(Ordering::Relaxed) {
                return Err(TraceError::Cancelled);
            }
            // (And one for each `POINTS_PER_THREAD` corners of the outlines to fit; as many as the first pass when a
            // level's components are found again.)
            let mut again = false;
            let mut points = 0usize;
            for c in found.iter().flatten() {
                match c.lock().ok().as_deref() {
                    Some(Some(comps)) => {
                        points =
                            comps.iter().flat_map(|c| std::iter::once(&c.outer).chain(&c.holes)).fold(points, |s, l| s.saturating_add(l.pts.len()));
                    }
                    _ => again = true,
                }
            }
            let threads = points.div_ceil(POINTS_PER_THREAD).max(1);
            let threads = if again { threads.min(at_a_time) } else { threads };
            // The highest level of each area that has marked a hole (none: nothing in the area is covered).
            let mut highest: Vec<Option<usize>> = vec![None; areas.len()];
            if let Ok(p) = published.lock() {
                for &(k, i, _) in p.iter() {
                    if let Some(h) = highest.get_mut(k) {
                        *h = Some(h.map_or(i, |h| h.max(i)));
                    }
                }
            }
            let items: Vec<_> = jobs.iter().zip(&found).collect();
            let fitted = parallel_map_at_most(threads, &items, |&(&(k, i), comps)| {
                let (a, &(r, ci)) = areas.get(k).and_then(|a| Some((a, a.levels.get(i)?)))?;
                if progress.stopped.load(Ordering::Relaxed) {
                    return None;
                }
                let comps = match comps.as_ref()?.lock().ok()?.take() {
                    Some(comps) => comps,
                    None => {
                        // (Found again, leaving out every area the levels above have filled.)
                        let above: Vec<Arc<Vec<Loop>>> = published
                            .lock()
                            .map(|p| p.iter().filter(|e| e.0 == k && e.1 > i).map(|e| Arc::clone(&e.2)).collect())
                            .unwrap_or_default();
                        self.contours(r, ci, &[], Some(a), &above.iter().flat_map(|l| l.iter()).collect::<Vec<_>>())?
                    }
                };
                let covered = highest.get(k).copied().flatten().is_some_and(|h| h > i);
                let out = self.fill_layer(&round, a, i, ci, comps, covered);
                progress.add(per_job);
                out
            });
            drop(found);
            if progress.stopped.load(Ordering::Relaxed) {
                return Err(TraceError::Cancelled);
            }
            drop(published);
            // Each area's paths, its levels bottom up, and the holes filled in it.
            let mut by_level = jobs.iter().zip(fitted).map(|(&job, out)| Some((job, out?))).collect::<Option<Vec<_>>>().ok_or_else(too_complex)?;
            by_level.sort_by_key(|e| e.0);
            // The marks of this round lie inside its areas' boxes.
            for a in &areas {
                for y in a.bx.1..a.bx.3 {
                    for m in marks.get(y * self.w + a.bx.0..y * self.w + a.bx.2).unwrap_or_default() {
                        m.store(0, Ordering::Relaxed);
                    }
                }
            }
            // What is drawn inside the holes the shapes fill, all levels' holes at once: the paths of an area of one
            // color, or an area of the next round. (Threads by the pixels inside, by the corners of the outlines to fit,
            // and by the number of holes.)
            let mut holes: Vec<(usize, Loop)> = vec![];
            for (n, (_, (_, filled))) in by_level.iter_mut().enumerate() {
                holes.extend(std::mem::take(filled).into_iter().map(|l| (n, l)));
            }
            let pixels = holes.iter().fold(0usize, |s, (_, l)| s.saturating_add(usize::try_from(l.area2.unsigned_abs() / 2).unwrap_or(usize::MAX)));
            let corners = holes.iter().fold(0usize, |s, (_, l)| s.saturating_add(l.pts.len()));
            let threads = pixels.div_ceil(PIXELS_PER_THREAD).max(corners.div_ceil(POINTS_PER_THREAD)).max(holes.len().div_ceil(64)).max(1);
            // (Once one is refused, past the anchors allowed, the rest are not fitted.)
            let refused = AtomicBool::new(false);
            let insides = parallel_map_at_most(threads, &holes, |(_, l)| {
                if progress.stopped.load(Ordering::Relaxed) || refused.load(Ordering::Relaxed) {
                    return None;
                }
                let inside = self.inside(l.clone());
                if inside.is_none() {
                    refused.store(true, Ordering::Relaxed);
                }
                inside
            });
            if progress.stopped.load(Ordering::Relaxed) {
                return Err(TraceError::Cancelled);
            }
            if refused.load(Ordering::Relaxed) {
                return Err(too_complex());
            }
            let mut by_job: Vec<Vec<Inside>> = (0..by_level.len()).map(|_| vec![]).collect();
            for ((n, _), inside) in holes.into_iter().zip(insides) {
                if let Some(v) = by_job.get_mut(n) {
                    v.push(inside.ok_or_else(too_complex)?);
                }
            }
            // Each area's paths, then what is drawn inside the holes its shapes fill.
            let (mut next, mut next_nodes) = (vec![], vec![]);
            for (((k, _), (paths, _)), inside) in by_level.into_iter().zip(by_job) {
                let up = *nodes.get(k).ok_or_else(too_complex)?;
                if let Some(t) = tree.get_mut(up) {
                    t.0.extend(paths);
                }
                for item in inside {
                    let node = tree.len();
                    if let Some(t) = tree.get_mut(up) {
                        t.1.push(node);
                    }
                    match item {
                        Inside::Done(paths) => tree.push((paths, vec![])),
                        Inside::Next(mut area) => {
                            tree.push((vec![], vec![]));
                            let cost = ((area.bx.2 - area.bx.0) * (area.bx.3 - area.bx.1)) as u64 * area.levels.len() as u64;
                            // (An area past what is left keeps its holes and is charged nothing: a smaller one after
                            // it may still fit.)
                            area.fill = cost <= budget;
                            if area.fill {
                                budget -= cost;
                            }
                            #[cfg(test)]
                            decided.push((round_number + 1, cost, area.fill));
                            next.push(area);
                            next_nodes.push(node);
                        }
                    }
                }
            }
            (areas, nodes) = (next, next_nodes);
            // (Only the first round reads the runs of untraced pixels.)
            dead = None;
            first = false;
        }
        // Each area's paths, then the areas inside it.
        let mut out = vec![];
        let mut stack = vec![0];
        while let Some(k) = stack.pop() {
            let Some((p, inner)) = tree.get_mut(k) else { continue };
            out.append(p);
            stack.extend(inner.iter().rev());
        }
        #[cfg(test)]
        PROBE.with(|p| {
            let mut p = p.borrow_mut();
            p.live_reads = live_reads.load(Ordering::Relaxed);
            p.areas = decided;
        });
        if !progress.add(progress.total) {
            return Err(TraceError::Cancelled);
        }
        Ok(out)
    }
}

/// Test probe: every crack between two labels of `img` quantised as `trace` does (no gradients),
/// as `[shift, shift from the other side, one-sided reading, one-sided reading from the other
/// side]`.
#[cfg(test)]
pub(crate) fn shift_probe(img: &Raster, params: &TraceParams) -> Vec<[f64; 4]> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut q = quantize(img, params);
    let labs = super::quantize::palette_labs(&q.palette);
    let source =
        super::quantize::Source { labs: &labs, rgba: &img.rgba, blended: &[], gray: matches!(params.mode, Mode::Grayscale | Mode::BlackAndWhite) };
    super::quantize::denoise_nearest(&mut q.labels, w, h, params.noise as usize, &[], Some(&source));
    let n = q.palette.len();
    let rank: Vec<usize> = (0..n).collect();
    let gradients = vec![None; n];
    let job = Layers {
        q: &q,
        traced: &q.labels,
        rank: &rank,
        params,
        img,
        left_out: left_out(&q.palette, params, None, &[]),
        boxes: vec![None; n],
        gradients: &gradients,
        bw_tones: bw_tones(img, &q),
        style: params.outline_style(),
        w,
        h,
        anchors: AtomicUsize::new(0),
        max_anchors: usize::MAX,
    };
    let mut out = vec![];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            for step in [(1, 0), (0, 1)] {
                let c = Crack::new((x, y), step, w, h);
                let (Some(a), Some(b)) = (c.at(0), c.at(1)) else { continue };
                if q.labels[a] != q.labels[b] {
                    out.push([job.shift(c), job.shift(c.flipped()), job.shift_one_side(c), job.shift_one_side(c.flipped())]);
                }
            }
        }
    }
    out
}

/// Overlapping with Fills: a part of the image whose layers are traced together and stacked as the whole image's are
/// (see [`Layers::stacked`]). The whole image is one; the area inside a hole that a shape fills is another, traced in
/// the next round and drawn above the part it lies in.
struct Area {
    /// Its box in image pixels, `(x0, y0, x1, y1)`, the far sides excluded.
    bx: (usize, usize, usize, usize),
    /// The hole whose inside it is (the whole image has none).
    outline: Option<Loop>,
    /// The layers with pixels here, bottom up: `(rank, palette entry)`.
    levels: Vec<(usize, usize)>,
    /// Whether its shapes fill the holes they may fill (past the work budget, they keep them).
    fill: bool,
}

/// What [`Layers::stacked`] draws above a shape inside a hole that the shape fills.
enum Inside {
    /// The paths of an area of one color, traced at once: it has no holes, and so nothing more inside it.
    Done(Vec<TracedPath>),
    /// An area of several colors, traced in the next round (its `fill` not set yet).
    Next(Area),
}

/// The holes a level of [`Layers::stacked`] has marked: `(area, level, holes)`.
type Marked = (usize, usize, Arc<Vec<Loop>>);

/// One round of [`Layers::stacked`]: the areas traced at the same time. Areas of one round never share a pixel.
struct Round<'a> {
    /// For each pixel of the image, one more than the highest level of its area at which a shape fills a hole around
    /// it; 0 where none does.
    marks: &'a [AtomicU16],
    /// In the first round, where a hole may hold a pixel that the layers do not trace (transparent, or a color left
    /// out), the runs of such pixels (see [`dead_runs`]): an area is filled only where it holds none. `None` in later
    /// rounds and in images without such pixels.
    dead: Option<&'a [(u32, u32)]>,
    /// Tests: the reads made to check a hole against `dead`.
    #[cfg(test)]
    live_reads: &'a AtomicUsize,
}

/// How far a stacked trace has got, for its `steps` callback: each level of the first round counts one step in each
/// of the two passes, and the rounds after it share as many steps again as the first round has levels, each round
/// half of what the rounds before it left (their number is not known in advance).
struct Progress<'a> {
    steps: &'a (dyn Fn(u64, u64) -> bool + Sync),
    done: AtomicU64,
    total: u64,
    stopped: AtomicBool,
}

impl Progress<'_> {
    /// Adds `n` units and says whether to go on.
    fn add(&self, n: u64) -> bool {
        let done = self.done.fetch_add(n, Ordering::Relaxed) + n;
        if !(self.steps)(done.min(self.total), self.total) {
            self.stopped.store(true, Ordering::Relaxed);
        }
        !self.stopped.load(Ordering::Relaxed)
    }
}

/// One step of [`Progress`], in its own units (a level's step can then be split among the levels of later rounds).
const STEP: u64 = 1 << 20;

/// [`Layers::stacked`]: how many bytes of components found in the first pass of a round are kept for the second.
const KEPT_BYTES: usize = 256 << 20;

/// [`Layers::stacked`]: how many pixels of the levels' boxes, and how many corners of the outlines to fit, are worth a
/// thread of their own.
const PIXELS_PER_THREAD: usize = 1 << 16;
const POINTS_PER_THREAD: usize = 1 << 6;

/// Tests: switches and counts of [`Layers::stacked`], one set per test thread ([`PROBE`]). A trace reads the switches
/// when it starts and writes the counts when it ends, so tests that run at the same time do not see each other's.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct Probe {
    /// Keep the areas that higher levels have filled in a level's components (the result is the same either way).
    pub(crate) keep_covered: bool,
    /// Find one level's components at a time, so that every level leaves out all the areas the levels above it fill.
    pub(crate) one_level_at_a_time: bool,
    /// Drop every level's components after the first pass and find them again in the second (as past `KEPT_BYTES`).
    pub(crate) keep_none: bool,
    /// The work budget of nested areas, in place of the one the image gives.
    pub(crate) budget: Option<u64>,
    /// Counted: the reads the first round made to check that a hole holds only pixels the layers trace.
    pub(crate) live_reads: usize,
    /// Counted: the bytes the runs of untraced pixels take.
    pub(crate) dead_bytes: usize,
    /// Counted: each area of a later round, as `(round, cost, filled)`, in the order the budget was charged.
    pub(crate) areas: Vec<(usize, u64, bool)>,
}

#[cfg(test)]
thread_local! {
    pub(crate) static PROBE: std::cell::RefCell<Probe> = std::cell::RefCell::new(Probe::default());
}

/// The runs of the pixels of `labels` for which `dead` holds, in order as `(i0, i1)` (indices into `labels`, so that a
/// run goes on into the next row; one list for the image, whatever its height).
fn dead_runs(labels: &[u16], dead: impl Fn(u16) -> bool) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = vec![];
    for (i, &l) in labels.iter().enumerate() {
        let i = u32::try_from(i).unwrap_or(u32::MAX);
        if !dead(l) {
            continue;
        }
        match runs.last_mut() {
            Some(r) if r.1 == i => r.1 = i.saturating_add(1),
            _ => runs.push((i, i.saturating_add(1))),
        }
    }
    runs
}

/// Whether any of the runs `runs` (left to right, apart) meets the pixels `a..b`.
fn meets(runs: &[(u32, u32)], a: usize, b: usize) -> bool {
    let (Ok(a), Ok(b)) = (u32::try_from(a), u32::try_from(b)) else { return true };
    runs.get(runs.partition_point(|r| r.1 <= a)).is_some_and(|r| r.0 < b)
}

/// Calls `f(y, x0, x1)` with each run `x0..x1` of the pixels in row `y` inside the loops `loops` (along pixel edges;
/// inside an odd number of them), row by row, until it answers false; whether it never did.
fn runs_inside<'l>(loops: impl IntoIterator<Item = &'l Loop>, mut f: impl FnMut(usize, usize, usize) -> bool) -> bool {
    // The loops' upright edges `(top, bottom, x)`, by their top. Going down the rows, the edges that pass the row are
    // kept: the pixels between the first and the second are inside, and so on.
    let mut edges: Vec<(i32, i32, i32)> = loops
        .into_iter()
        .flat_map(|l| l.pts.iter().zip(l.pts.iter().cycle().skip(1)))
        .filter(|(a, b)| a.0 == b.0 && a.1 != b.1)
        .map(|(a, b)| (a.1.min(b.1), a.1.max(b.1), a.0))
        .collect();
    edges.sort_unstable_by_key(|e| e.0);
    let (Some(top), Some(bottom)) = (edges.first().map(|e| e.0), edges.iter().map(|e| e.1).max()) else { return true };
    let (mut next, mut passing, mut xs) = (0, Vec::<(i32, i32)>::new(), Vec::<i32>::new());
    for y in top..bottom {
        while let Some(&(_, end, x)) = edges.get(next).filter(|e| e.0 <= y) {
            passing.push((end, x));
            next += 1;
        }
        passing.retain(|e| e.0 > y);
        xs.clear();
        xs.extend(passing.iter().map(|e| e.1));
        xs.sort_unstable();
        let Ok(row) = usize::try_from(y) else { continue };
        for pair in xs.as_chunks::<2>().0 {
            let (Ok(a), Ok(b)) = (usize::try_from(pair[0].max(0)), usize::try_from(pair[1].max(0))) else { continue };
            if a < b && !f(row, a, b) {
                return false;
            }
        }
    }
    true
}

/// Calls `f` with each pixel `(x, y)` inside the loops `loops` (see [`runs_inside`]), row by row, until it answers
/// false; whether it never did. A component's outer loop and holes give its own pixels.
fn pixels_inside<'l>(loops: impl IntoIterator<Item = &'l Loop>, mut f: impl FnMut(usize, usize) -> bool) -> bool {
    runs_inside(loops, |y, a, b| (a..b).all(|x| f(x, y)))
}

/// Calls `f(k, runs)` with the runs of row `y0 + k`, for each `k` below `rows` in order, of the pixels inside `outline`
/// (all of `x0..x1` without one) and inside none of `covered`, within `x0..x1`, left to right. Rows without such a
/// pixel may be passed over. Only the covered loops' runs are held, not a list for every row.
fn row_runs(x0: usize, x1: usize, y0: usize, rows: usize, outline: Option<&Loop>, covered: &[&Loop], mut f: impl FnMut(usize, &[(usize, usize)])) {
    // The covered runs `(k, a, b)`, by row and then left to right.
    let mut gone: Vec<(usize, usize, usize)> = vec![];
    for l in covered {
        runs_inside([*l], |y, a, b| {
            if let Some(k) = y.checked_sub(y0).filter(|&k| k < rows) {
                gone.push((k, a, b));
            }
            true
        });
    }
    gone.sort_unstable();
    let (mut next, mut left) = (0, vec![]);
    // Row `k`'s runs `keep` less its covered runs.
    let mut row = |k: usize, keep: &[(usize, usize)]| {
        let from = next;
        while gone.get(next).is_some_and(|e| e.0 <= k) {
            next += 1;
        }
        let cut = gone.get(from..next).unwrap_or_default();
        if cut.iter().all(|e| e.0 < k) {
            f(k, keep);
            return;
        }
        left.clear();
        for &(a, b) in keep {
            let mut x = a;
            for &(_, ga, gb) in cut.iter().filter(|e| e.0 == k) {
                if gb <= x || ga >= b {
                    continue;
                }
                if ga > x {
                    left.push((x, ga));
                }
                x = x.max(gb);
            }
            if x < b {
                left.push((x, b));
            }
        }
        f(k, &left);
    };
    match outline {
        None => (0..rows).for_each(|k| row(k, &[(x0, x1)])),
        Some(l) => {
            // (`runs_inside` goes down the rows, each left to right.)
            let (mut at, mut runs) = (None, vec![]);
            runs_inside([l], |y, a, b| {
                let Some(k) = y.checked_sub(y0).filter(|&k| k < rows) else { return true };
                if at != Some(k) {
                    if let Some(prev) = at {
                        row(prev, &runs);
                    }
                    at = Some(k);
                    runs.clear();
                }
                if a.max(x0) < b.min(x1) {
                    runs.push((a.max(x0), b.min(x1)));
                }
                true
            });
            if let Some(prev) = at {
                row(prev, &runs);
            }
        }
    }
}

/// [`pixels_inside`] the loop `l`: for a hole, the pixels it encloses, those of what lies in it included.
fn interior(l: &Loop, f: impl FnMut(usize, usize) -> bool) -> bool {
    pixels_inside([l], f)
}

/// [`pixels_inside`] the component `c`: its own pixels.
fn pixels_of(c: &contour::Component, f: impl FnMut(usize, usize) -> bool) -> bool {
    pixels_inside(std::iter::once(&c.outer).chain(&c.holes), f)
}

/// The components that [`trace_mask`] finds in a mask of exactly the pixels inside the hole `hole`, without tracing
/// one: the hole's outline walked the other way round, split where it touches itself (where two pixels inside it meet
/// only at a corner, which are not connected), each part started at its top left corner and in the order of those
/// corners, as `trace_mask` gives them.
fn shapes_in_hole(hole: &Loop) -> Vec<contour::Component> {
    let Some((&first, rest)) = hole.pts.split_first() else { return vec![] };
    // The corners walked the other way, back to the first; a corner met again closes the part since its first visit.
    let mut seen: std::collections::HashMap<(i32, i32), usize> = std::collections::HashMap::new();
    let mut stack: Vec<(i32, i32)> = vec![];
    let mut parts: Vec<Vec<(i32, i32)>> = vec![];
    for p in std::iter::once(first).chain(rest.iter().rev().copied()).chain(std::iter::once(first)) {
        if let Some(&at) = seen.get(&p) {
            let part = stack.split_off(at + 1);
            for q in &part {
                seen.remove(q);
            }
            let start = stack.last().copied();
            parts.push(start.into_iter().chain(part).collect());
        } else {
            seen.insert(p, stack.len());
            stack.push(p);
        }
    }
    let mut comps: Vec<contour::Component> = parts
        .into_iter()
        .filter(|pts| pts.len() >= 4)
        .map(|mut pts| {
            // From the top left corner (the walk turns right there).
            if let Some(at) = pts.iter().enumerate().min_by_key(|(_, p)| (p.1, p.0)).map(|(i, _)| i) {
                pts.rotate_left(at);
            }
            let area2 = pts
                .iter()
                .zip(pts.iter().cycle().skip(1))
                .map(|(a, b)| i64::from(a.0) * i64::from(b.1) - i64::from(b.0) * i64::from(a.1))
                .sum::<i64>();
            let pixels = usize::try_from(area2 / 2).unwrap_or(0);
            contour::Component { outer: Loop { pts, area2 }, holes: vec![], pixels }
        })
        .collect();
    comps.sort_by_key(|c| c.outer.pts.first().map(|p| (p.1, p.0)));
    comps
}

/// The most cracks (pixel edges) one loop's outline may have: `subpixel::outline` holds about
/// 90 bytes a crack while it places the points, and a comb-shaped region of a large image has
/// one crack per pixel, gigabytes in all. A loop above this refuses the layer instead. The
/// test cap is small so a test image can cross it.
const MAX_LOOP_CRACKS: usize = if cfg!(test) { 1 << 14 } else { 4 << 20 };

/// How many cracks `l` expands into: its edge lengths in pixels (its vertices are only its
/// turns, so the vertex count says nothing about the storage the outline takes).
fn cracks_of(l: &Loop) -> usize {
    l.pts
        .iter()
        .zip(l.pts.iter().cycle().skip(1))
        .fold(0usize, |s, (a, b)| s.saturating_add((b.0 - a.0).unsigned_abs().max((b.1 - a.1).unsigned_abs()) as usize))
}

/// The pixel at the top left of the area a loop goes around: its topmost row's leftmost.
fn first_pixel(l: &Loop) -> Option<(usize, usize)> {
    let &(x, y) = l.pts.iter().min_by_key(|p| (p.1, p.0))?;
    Some((usize::try_from(x).ok()?, usize::try_from(y).ok()?))
}

/// How many components of a layer are fitted at a time (see `fit_in_chunks`).
const FIT_CHUNK: usize = 256;

/// `fit` of each of `items` over the available cores, `chunk` items at a time, each result handed to `accept` in item
/// order. Stops after the chunk in which `accept` returns false, and says whether it never did.
fn fit_in_chunks<T: Sync, R: Send>(items: &[T], chunk: usize, fit: impl Fn(&T) -> R + Sync, mut accept: impl FnMut(&T, R) -> bool) -> bool {
    for part in items.chunks(chunk.max(1)) {
        // (A few components are not worth starting threads for.)
        let fitted = if part.len() < 16 { part.iter().map(&fit).collect() } else { parallel_map(part, &fit) };
        for (item, result) in part.iter().zip(fitted) {
            if !accept(item, result) {
                return false;
            }
        }
    }
    true
}

/// `f` of each of `items`, in order, spread over the available cores.
pub(crate) fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    parallel_map_at_most(usize::MAX, items, f)
}

#[cfg(not(target_arch = "wasm32"))]
std::thread_local! {
    /// Whether this thread is already a worker of a group: a group started inside each worker
    /// would ask for `available_parallelism` threads again (cores² in all), and `Scope::spawn`
    /// panics when the host refuses a thread. A layer that runs inline (the outer call found one
    /// core's worth of work) still gets a full group for its components.
    static IN_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `f` of each of `items`, in order, spread over the available cores, at most `at_a_time` at once.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn parallel_map_at_most<T: Sync, R: Send>(at_a_time: usize, items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = if IN_WORKER.get() { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len()).min(at_a_time) };
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    IN_WORKER.set(true);
                    let mut mine = vec![];
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        mine.push((i, f(item)));
                    }
                    mine
                })
            })
            .collect();
        // A worker's panic goes on in this thread, as if it had run here (the engine's guard
        // reports it).
        workers.into_iter().flat_map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// `f` of each of `items`, in order (no threads on the web).
#[cfg(target_arch = "wasm32")]
pub(crate) fn parallel_map_at_most<T: Sync, R: Send>(_: usize, items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    items.iter().map(f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `f` of the layers of a `w × 8` image whose pixels in column `x` are the color `color(x)` and
    /// have the label `label(x)`, with the palette `palette`, and of the cracks between columns
    /// `split - 1` and `split`.
    fn across<R>(
        w: u32,
        split: u32,
        color: impl Fn(u32) -> [u8; 3],
        label: impl Fn(u32) -> u16,
        palette: &[[u8; 3]],
        mode: Mode,
        f: impl FnOnce(&Layers, Vec<Crack>) -> R,
    ) -> R {
        let img = Raster::from_fn(w, 8, |x, _| {
            let [r, g, b] = color(x);
            [r, g, b, 255]
        });
        let (w, h) = (w as usize, 8);
        let q = Quantized { labels: (0..w * h).map(|i| label((i % w) as u32)).collect(), palette: palette.to_vec() };
        let params = TraceParams { mode, ..TraceParams::default() };
        let n = palette.len();
        let (rank, gradients): (Vec<usize>, Vec<Option<TracedGradient>>) = ((0..n).collect(), vec![None; n]);
        let job = Layers {
            q: &q,
            traced: &q.labels,
            rank: &rank,
            params: &params,
            img: &img,
            left_out: vec![false; n],
            boxes: vec![None; n],
            gradients: &gradients,
            bw_tones: [255.0, 0.0],
            style: params.outline_style(),
            w,
            h,
            anchors: AtomicUsize::new(0),
            max_anchors: usize::MAX,
        };
        let cracks = (0..h as i32).map(|y| Crack::new((split as i32 - 1, y), (1, 0), w, h)).collect();
        f(&job, cracks)
    }

    /// [`steps_labeled`] with the label 0 left of column `split` and 1 from it.
    fn steps(w: u32, split: u32, color: impl Fn(u32) -> [u8; 3], palette: &[[u8; 3]], mode: Mode) -> (Vec<[f64; 5]>, bool) {
        steps_labeled(w, split, color, |x| u16::from(x >= split), palette, mode)
    }

    /// [`Layers::step`] of every crack between columns `split - 1` and `split` of [`across`]'s image,
    /// and whether `subpixel::low_contrast` calls them a contour. Each is read from both sides, which
    /// must agree exactly.
    fn steps_labeled(
        w: u32,
        split: u32,
        color: impl Fn(u32) -> [u8; 3],
        label: impl Fn(u32) -> u16,
        palette: &[[u8; 3]],
        mode: Mode,
    ) -> (Vec<[f64; 5]>, bool) {
        across(w, split, color, label, palette, mode, |job, cracks| {
            let shares: Vec<[f64; 5]> = cracks
                .into_iter()
                .map(|c| {
                    assert_eq!(job.step(c).map(f64::to_bits), job.step(c.flipped()).map(f64::to_bits));
                    job.step(c)
                })
                .collect();
            let contour = crate::subpixel::low_contrast(&shares);
            (shares, contour)
        })
    }

    #[test]
    fn a_crack_in_a_smooth_area_shows_a_share_of_the_step_that_grows_further_out() {
        // Two levels a pixel between colors 40 apart: 135 and 143 two pixels either side (8 levels
        // in each channel), 131 and 147 four pixels either side.
        let ramp = |x: u32| [(100 + 2 * x) as u8; 3];
        for mode in [Mode::Color, Mode::Grayscale] {
            let (shares, contour) = steps(40, 20, ramp, &[[120; 3], [160; 3]], mode);
            // (The image changes along the line between the two colors, so its whole change is
            // the change along that line; it changes by 2 levels a channel from pixel to pixel.)
            let fits = |s: &[f64; 5]| {
                (s[0] - 0.2).abs() < 1e-9
                    && (s[1] - 0.4).abs() < 1e-9
                    && [s[2], s[3]].iter().all(|v| (v - 8.0 * 3f64.sqrt()).abs() < 1e-9)
                    && (s[4] - 2.0 * 3f64.sqrt()).abs() < 1e-9
            };
            assert!(shares.iter().all(fits), "{mode:?}: {shares:?}");
            assert!(contour);
        }
        // Black and White's tones are no area's colors.
        let (shares, contour) = steps(40, 20, ramp, &[[120; 3], [160; 3]], Mode::BlackAndWhite);
        assert!(shares.iter().all(|s| s[0] == 1.0 && s[1] == 1.0 && s[2..].iter().all(|v| v.is_nan())) && !contour, "{shares:?}");
        // By the image's border there is nothing further out.
        let (shares, contour) = steps(6, 3, ramp, &[[90; 3], [130; 3]], Mode::Color);
        assert!(shares.iter().all(|s| s[1].is_nan()) && contour, "{shares:?}");
    }

    #[test]
    fn an_edge_between_flat_colors_is_an_edge_however_far_their_palette_entries_are() {
        // A flat gray beside white, the gray in an entry with black: 115 of 245 levels, less than
        // half the step between the two entries, but the same further out.
        let flat = |x: u32| if x < 10 { [140; 3] } else { [255; 3] };
        let (shares, contour) = steps(20, 10, flat, &[[10; 3], [255; 3]], Mode::Color);
        assert!(shares.iter().all(|s| s[0] < 0.5 && s[0] == s[1] && s[4] == 0.0), "{shares:?}");
        assert!(!contour);
        // Two flat grays 6 levels apart, in entries 25 apart: under a half of the step and a
        // change of under `FLAT`, but nothing changes within either side.
        let close = |split: u32| move |x: u32| if x < split { [100; 3] } else { [106; 3] };
        let (shares, contour) = steps(20, 10, close(10), &[[90; 3], [115; 3]], Mode::Color);
        assert!(shares.iter().all(|s| (s[0] - 0.24).abs() < 1e-9 && s[0] == s[1] && s[4] == 0.0), "{shares:?}");
        assert!(!contour);
        // The same by the image's border, where there is nothing further out.
        let (shares, contour) = steps(6, 3, close(3), &[[90; 3], [115; 3]], Mode::Color);
        assert!(shares.iter().all(|s| s[1].is_nan() && s[4] == 0.0), "{shares:?}");
        assert!(!contour);
        // Two flat colors whose difference is square to the line between their entries: none of
        // the step near the crack or further out.
        let square = |x: u32| if x < 10 { [100, 100, 100] } else { [100, 106, 94] };
        let (shares, contour) = steps(20, 10, square, &[[90; 3], [115; 3]], Mode::Color);
        assert!(shares.iter().all(|s| s[0] == 0.0 && s[1] == 0.0 && s[4] == 0.0), "{shares:?}");
        assert!(!contour);
    }

    #[test]
    fn an_edge_beside_a_thin_flat_stroke_is_an_edge() {
        // White, then a gray stroke two pixels wide in an entry far darker than it (under a half
        // of the step from white), then black: four pixels out from the crack the image reaches
        // black, more than the whole step.
        let color = |x: u32| match x {
            0..10 => [255; 3],
            10..12 => [200; 3],
            _ => [0; 3],
        };
        let label = |x: u32| match x {
            0..10 => 0,
            10..12 => 1,
            _ => 2,
        };
        let (shares, contour) = steps_labeled(20, 10, color, label, &[[255; 3], [140; 3], [0; 3]], Mode::Color);
        assert!(shares.iter().all(|s| s[0] < 0.5 && s[1] > 1.25 * s[0] && s[4] == 0.0), "{shares:?}");
        assert!(!contour);
    }

    #[test]
    fn grayscale_reads_an_edge_between_two_colors_by_their_grays() {
        // Red beside green: grays 60 and 117 (the palette's entries), whose channels add up to the
        // same 200, so read as colors the image would make none of the step between them.
        let flat = |x: u32| if x < 10 { [200, 0, 0] } else { [0, 200, 0] };
        let (shares, contour) = steps(20, 10, flat, &[[60; 3], [117; 3]], Mode::Grayscale);
        assert!(shares.iter().all(|s| s[0] == 1.0 && s[1] == 1.0), "{shares:?}");
        assert!(!contour);
    }

    #[test]
    fn grayscale_places_an_edge_between_two_colors_by_their_grays() {
        // The same red and green: by their grays, each is all of its own entry, so the edge lies on
        // the crack. Read as colors, each would be mostly the other's.
        let flat = |x: u32| if x < 10 { [200, 0, 0] } else { [0, 200, 0] };
        let shifts: Vec<f64> = across(
            20,
            10,
            flat,
            |x| u16::from(x >= 10),
            &[[60; 3], [117; 3]],
            Mode::Grayscale,
            |job, cracks| cracks.into_iter().map(|c| job.shift(c)).collect(),
        );
        assert!(shifts.iter().all(|&s| s == 0.0), "{shifts:?}");
    }

    #[test]
    fn an_edge_between_colors_off_the_line_between_the_palettes_two_is_an_edge() {
        // Red beside green traced with black and white entries: a change of 226 levels, none of it
        // from black towards white.
        let flat = |x: u32| if x < 10 { [160, 0, 0] } else { [0, 160, 0] };
        let (shares, contour) = steps(20, 10, flat, &[[0; 3], [255; 3]], Mode::Color);
        assert!(shares.iter().all(|s| s[0] == 0.0 && s[1] == 0.0 && (s[3] - 160.0 * 2f64.sqrt()).abs() < 1e-9), "{shares:?}");
        assert!(!contour);
    }

    #[test]
    fn fit_in_chunks_keeps_the_order_and_stops_at_the_first_refusal() {
        let items: Vec<usize> = (0..1000).collect();
        let fitted = AtomicUsize::new(0);
        let mut seen = vec![];
        let all = fit_in_chunks(
            &items,
            16,
            |&i| {
                fitted.fetch_add(1, Ordering::Relaxed);
                i * 2
            },
            |&i, r| {
                assert_eq!(r, i * 2);
                seen.push(i);
                seen.len() <= 10
            },
        );
        assert!(!all);
        // Items come in order, up to and including the one refused.
        assert_eq!(seen, (0..=10).collect::<Vec<_>>());
        // The early stop is kept: only the chunk holding the refusal was fitted, not the other 984 items.
        assert_eq!(fitted.load(Ordering::Relaxed), 16);
    }

    #[test]
    fn fit_in_chunks_accepts_everything_in_order() {
        let items: Vec<usize> = (0..1000).collect();
        let mut out = vec![];
        assert!(fit_in_chunks(
            &items,
            7,
            |&i| i + 1,
            |_, r| {
                out.push(r);
                true
            }
        ));
        assert_eq!(out, (1..=1000).collect::<Vec<_>>());
        assert!(fit_in_chunks(&[] as &[usize], 7, |&i| i, |_, _| false));
    }

    /// Seeded 23 × 17 masks of several densities.
    fn masks() -> impl Iterator<Item = (u32, Vec<bool>)> {
        (0..200u32).map(|seed| {
            let mask = (0..23 * 17)
                .map(|i| ((i as u32).wrapping_mul(2_654_435_761) ^ seed.wrapping_mul(40_503)).wrapping_mul(2_246_822_519) >> 28 < (seed % 15) + 1)
                .collect();
            (seed, mask)
        })
    }

    /// `interior` visits each pixel a loop encloses once, outer loops and holes alike, and stops when told to;
    /// `pixels_of` visits a component's own pixels.
    #[test]
    fn interior_visits_each_enclosed_pixel_once() {
        let (w, h) = (23, 17);
        for (seed, mask) in masks() {
            for comp in trace_mask(&mask, w, h) {
                for l in std::iter::once(&comp.outer).chain(&comp.holes) {
                    let mut seen = vec![false; w * h];
                    let mut n = 0;
                    assert!(interior(l, |x, y| {
                        assert!(!std::mem::replace(&mut seen[y * w + x], true), "seed {seed}: ({x}, {y}) twice");
                        n += 1;
                        true
                    }));
                    assert_eq!(n, l.area2.abs() / 2, "seed {seed}");
                }
                let mut visits = 0;
                assert!(!interior(&comp.outer, |_, _| {
                    visits += 1;
                    false
                }));
                assert_eq!(visits, 1);
                let mut own = 0;
                assert!(pixels_of(&comp, |x, y| {
                    assert!(mask[y * w + x], "seed {seed}: ({x}, {y}) is not in the mask");
                    own += 1;
                    true
                }));
                assert_eq!(own, comp.pixels, "seed {seed}");
            }
        }
    }

    /// `shapes_in_hole` gives what `trace_mask` finds in a mask of the pixels inside a hole, holes that touch
    /// themselves at a corner included.
    #[test]
    fn shapes_in_a_hole_are_those_traced() {
        let (w, h) = (23, 17);
        let (mut holes, mut touching) = (0, 0);
        for (seed, mask) in masks() {
            for hole in trace_mask(&mask, w, h).iter().flat_map(|c| &c.holes) {
                let mut inside = vec![false; w * h];
                interior(hole, |x, y| {
                    inside[y * w + x] = true;
                    true
                });
                assert_eq!(shapes_in_hole(hole), trace_mask(&inside, w, h), "seed {seed}: {hole:?}");
                holes += 1;
                let mut corners = hole.pts.clone();
                corners.sort_unstable();
                touching += usize::from(corners.windows(2).any(|p| p[0] == p[1]));
            }
        }
        assert!(holes > 500 && touching > 20, "{holes} holes, {touching} touching themselves");
    }

    /// A component left out of the traced components (as `contours` leaves out those without the layer's own color)
    /// gives the same loops as leaving its pixels out of the mask.
    #[test]
    fn leaving_a_component_out_keeps_the_other_loops() {
        let (w, h) = (23, 17);
        for (seed, mask) in masks() {
            let (labels, _) = contour::label(&mask, w, h);
            let rules: [fn(u32) -> bool; 2] = [|c| c % 2 == 0, |c| c % 3 != 1];
            for keep in rules {
                let fewer: Vec<bool> = mask.iter().zip(&labels).map(|(&m, &c)| m && keep(c)).collect();
                let kept: Vec<contour::Component> =
                    trace_mask(&mask, w, h).into_iter().enumerate().filter(|&(c, _)| keep(c as u32)).map(|(_, comp)| comp).collect();
                assert_eq!(kept, trace_mask(&fewer, w, h), "seed {seed}");
            }
        }
    }

    /// `row_runs` gives the pixels inside an outline and inside none of the covered loops, clipped to the box.
    #[test]
    fn row_runs_keep_the_outline_less_what_is_covered() {
        let (w, h) = (23, 17);
        for (seed, mask) in masks() {
            let comps = trace_mask(&mask, w, h);
            let Some(outline) = comps.iter().max_by_key(|c| c.outer.area2).map(|c| &c.outer) else { continue };
            let covered: Vec<&Loop> = comps.iter().flat_map(|c| &c.holes).collect();
            let (x0, x1, y0, rows) = (2, 20, 1, 14);
            for outline in [Some(outline), None] {
                let mut want = vec![outline.is_none(); w * h];
                if let Some(outline) = outline {
                    interior(outline, |x, y| {
                        want[y * w + x] = true;
                        true
                    });
                }
                for l in &covered {
                    interior(l, |x, y| {
                        want[y * w + x] = false;
                        true
                    });
                }
                let (mut got, mut last) = (vec![false; w * h], None);
                row_runs(x0, x1, y0, rows, outline, &covered, |k, runs| {
                    assert!(last.is_none_or(|l| l < k) && k < rows, "seed {seed}: row {k} after {last:?}");
                    last = Some(k);
                    for w2 in runs.windows(2) {
                        assert!(w2[0].1 <= w2[1].0, "seed {seed}: {runs:?}");
                    }
                    for &(a, b) in runs {
                        for x in a..b {
                            got[(y0 + k) * w + x] = true;
                        }
                    }
                });
                for y in 0..h {
                    for x in 0..w {
                        let inside = (x0..x1).contains(&x) && (y0..y0 + rows).contains(&y);
                        assert_eq!(got[y * w + x], want[y * w + x] && inside, "seed {seed}: ({x}, {y}), outline {}", outline.is_some());
                    }
                }
            }
        }
    }

    /// A group started inside a worker runs inline: one worker budget for the whole trace, never
    /// cores² threads. Thread ids are never reused, so counting the distinct ids seen is exact.
    #[test]
    fn a_worker_does_not_start_its_own_worker_group() {
        let ids = Mutex::new(std::collections::HashSet::new());
        let note = || {
            if let Ok(mut seen) = ids.lock() {
                seen.insert(std::thread::current().id());
            }
        };
        let items: Vec<usize> = (0..32).collect();
        let out = parallel_map(&items, |i| {
            note();
            let inner: Vec<usize> = (0..32).collect();
            parallel_map(&inner, |x| {
                note();
                x + i
            })
            .into_iter()
            .sum::<usize>()
        });
        assert_eq!(out.len(), 32);
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let seen = ids.lock().map(|s| s.len()).unwrap_or(0);
        assert!(seen >= 1 && seen <= cores + 1, "{seen} threads for {cores} cores");
    }
}
