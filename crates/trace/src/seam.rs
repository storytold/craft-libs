//! Abutting with Fills: seam fills, thin strokes under the filled paths along the edges they share.
//!
//! Abutting paths meet edge to edge, so a renderer that anti-aliases each path on its own leaves
//! the pixels along a shared edge partly covered by each path and lets what is behind them show
//! through as a light hairline. A stroke along the edge in the color of the path drawn first,
//! under every path, fills those pixels with that color: drawing the two paths over it then gives
//! each pixel the colors of the two paths in the shares each covers. The paths themselves are not
//! changed, and an edge a path shares with no other path (the image's border, a transparent area,
//! a color left out) gets no stroke. The strokes are clipped to the image's box. Where a shared
//! edge ends at, or runs within half the stroke's width of, a transparent area or a color left
//! out, its stroke can reach that far into that area.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use craft_geom::{Anchor, PathData, Point, SubPath};

use crate::{Method, Mode, TraceParams, TracedPath};

/// How wide each seam fill's stroke is, in pixels: it covers every pixel the shared edge passes
/// through, at any angle (a pixel's corner is up to 0.71 px from a line through it).
const SEAM_WIDTH: f64 = 1.5;

/// The seam fills of an Abutting trace ([`seam_fills`]).
#[derive(Clone, Debug, PartialEq)]
pub struct SeamFills {
    /// For each color, one path stroked 1.5 pixels wide along the edges where a filled path of
    /// that color meets a filled path drawn after it, with `color` and `gradient` that path's; the
    /// paths with a gradient get one each. Bottom-most first, as the paths they lie under.
    pub strokes: Vec<TracedPath>,
    /// The area to clip `strokes` to, in the same pixel coordinates: the image's box.
    pub clip: PathData,
}

impl SeamFills {
    /// The anchors of the strokes and the clip together.
    pub fn anchor_count(&self) -> usize {
        self.strokes.iter().map(|s| s.path.anchor_count()).sum::<usize>() + self.clip.anchor_count()
    }
}

/// A segment, the same for the segment and the segment run backwards: two 64-bit hashes of its
/// points' bits, 16 bytes a segment instead of 64. Two different segments with the same key would
/// only give the lower one's color a stroke along a piece it does not share.
type Key = u128;

fn key(a: &Anchor, b: &Anchor) -> Key {
    let bits = |[p, q, r, s]: [Point; 4]| [p.x, p.y, q.x, q.y, r.x, r.y, s.x, s.y].map(f64::to_bits);
    let points = bits([a.p, a.h_out, b.h_in, b.p]).min(bits([b.p, b.h_in, a.h_out, a.p]));
    let hash = |seed: u8| {
        let mut h = DefaultHasher::new();
        (seed, points).hash(&mut h);
        h.finish()
    };
    (u128::from(hash(0)) << 64) | u128::from(hash(1))
}

/// The seam fills of a trace of a `size` (width, height in pixels) image made with `params`, for
/// Abutting with Fills (not Flat Logo): strokes to draw under every path of `paths` (bottom-most
/// first), clipped to the image's box. Filled paths that became a live shape (Shapes) take no part:
/// their outline is not the traced one. The strokes have at most as many anchors as `paths` (each
/// of their segments is a segment of a path), and the clip has four; finding them keeps about 40
/// bytes for each segment of `paths` while it runs. `None` for any other trace, or when no filled
/// paths share an edge.
pub fn seam_fills(paths: &[TracedPath], params: &TraceParams, size: (u32, u32)) -> Option<SeamFills> {
    if params.method != Method::Abutting || !params.fills || params.mode == Mode::Logo {
        return None;
    }
    fills(paths, size)
}

/// Whether `p` takes part in the seam fills: filled, and drawn with its traced outline.
fn filled(p: &TracedPath) -> bool {
    p.stroke.is_none() && p.shape.is_none()
}

/// The segments of `sp`: each anchor and the one after it (back to the first, if closed).
fn segments(sp: &SubPath) -> impl Iterator<Item = (&Anchor, &Anchor)> {
    let n = sp.anchors.len();
    let count = if sp.closed { n } else { n.saturating_sub(1) };
    (0..count).filter_map(move |k| Some((sp.anchors.get(k)?, sp.anchors.get((k + 1) % n)?)))
}

/// [`seam_fills`] of Abutting `paths` traced from a `size` image, whatever the settings.
fn fills(paths: &[TracedPath], size: (u32, u32)) -> Option<SeamFills> {
    // For each segment, the first path it is in, and whether another path has it too.
    let mut owner: HashMap<Key, (usize, bool)> = HashMap::new();
    for (i, p) in paths.iter().enumerate().filter(|(_, p)| filled(p)) {
        for sp in &p.path.subpaths {
            for (a, b) in segments(sp).filter(|(a, b)| a.p != b.p) {
                let e = owner.entry(key(a, b)).or_insert((i, false));
                if e.0 != i {
                    e.1 = true;
                }
            }
        }
    }
    // One fill per color (per path, for a gradient), in the order of the first path it lies under.
    let mut out: Vec<TracedPath> = vec![];
    let mut by_color: HashMap<[u8; 3], usize> = HashMap::new();
    for (i, p) in paths.iter().enumerate().filter(|(_, p)| filled(p)) {
        let mut runs: Vec<SubPath> = vec![];
        for sp in &p.path.subpaths {
            let marks: Vec<bool> = segments(sp).map(|(a, b)| a.p != b.p && owner.get(&key(a, b)) == Some(&(i, true))).collect();
            let n = sp.anchors.len();
            if marks.is_empty() {
                continue;
            }
            if sp.closed && marks.iter().all(|&m| m) {
                runs.push(sp.clone());
                continue;
            }
            // Start after an unmarked segment, so that a run across the subpath's first anchor
            // stays one run.
            let start = if sp.closed { marks.iter().position(|&m| !m).map_or(0, |k| k + 1) } else { 0 };
            let mut run: Vec<Anchor> = vec![];
            for j in 0..marks.len() {
                let k = (start + j) % marks.len();
                let (Some(&a), Some(&b)) = (sp.anchors.get(k), sp.anchors.get((k + 1) % n)) else { continue };
                if marks.get(k).copied().unwrap_or(false) {
                    if run.is_empty() {
                        run.push(Anchor { h_in: a.p, ..a });
                    }
                    run.push(b);
                } else if !run.is_empty() {
                    runs.push(open_run(std::mem::take(&mut run)));
                }
            }
            if !run.is_empty() {
                runs.push(open_run(run));
            }
        }
        if runs.is_empty() {
            continue;
        }
        let slot = if p.gradient.is_some() { None } else { by_color.get(&p.color).copied() };
        match slot.and_then(|s| out.get_mut(s)) {
            Some(fill) => fill.path.subpaths.extend(runs),
            None => {
                if p.gradient.is_none() {
                    by_color.insert(p.color, out.len());
                }
                out.push(TracedPath {
                    path: PathData::new(runs),
                    color: p.color,
                    pixels: 0,
                    stroke: Some(SEAM_WIDTH),
                    shape: None,
                    gradient: p.gradient.clone(),
                    square_caps: false,
                });
            }
        }
    }
    if out.is_empty() {
        return None;
    }
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    let clip = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)], true));
    Some(SeamFills { strokes: out, clip })
}

/// An open run of anchors: the last one has no handle out.
fn open_run(mut run: Vec<Anchor>) -> SubPath {
    if let Some(last) = run.last_mut() {
        last.h_out = last.p;
    }
    SubPath::new(run, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Mode, Raster, TraceParams, TracedGradient, TracedShape, trace};

    fn polygon(rings: &[&[(f64, f64)]], color: u8) -> TracedPath {
        let subpaths = rings.iter().map(|r| SubPath::polyline(&r.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), true)).collect();
        TracedPath { path: PathData::new(subpaths), color: [color; 3], pixels: 1, stroke: None, shape: None, gradient: None, square_caps: false }
    }

    /// The strokes of [`fills`] for `paths` in a box that no test path reaches the border of.
    fn strokes(paths: &[TracedPath]) -> Vec<TracedPath> {
        fills(paths, (100, 100)).map(|f| f.strokes).unwrap_or_default()
    }

    fn points(sp: &SubPath) -> Vec<(f64, f64)> {
        sp.anchors.iter().map(|a| (a.p.x, a.p.y)).collect()
    }

    /// Each segment of `p`'s subpaths, by [`key`].
    fn keys(p: &TracedPath) -> Vec<Key> {
        p.path.subpaths.iter().flat_map(|sp| segments(sp).map(|(a, b)| key(a, b)).collect::<Vec<_>>()).collect()
    }

    #[test]
    fn a_shared_edge_gets_a_stroke_in_the_color_of_the_path_drawn_first() {
        let left = polygon(&[&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]], 10);
        let right = polygon(&[&[(2.0, 0.0), (4.0, 0.0), (4.0, 2.0), (2.0, 2.0)]], 20);
        let fills = strokes(&[left.clone(), right.clone()]);
        assert_eq!(fills.len(), 1, "{fills:?}");
        assert_eq!((fills[0].color, fills[0].stroke, fills[0].gradient.as_ref()), ([10; 3], Some(SEAM_WIDTH), None));
        assert_eq!(fills[0].path.subpaths.len(), 1);
        let sp = &fills[0].path.subpaths[0];
        assert!(!sp.closed);
        assert_eq!(points(sp), [(2.0, 0.0), (2.0, 2.0)]);
        // Drawn the other way round, the stroke takes the other color.
        let swapped = strokes(&[right, left]);
        assert_eq!(swapped.len(), 1);
        assert_eq!((swapped[0].color, points(&swapped[0].path.subpaths[0])), ([20; 3], vec![(2.0, 2.0), (2.0, 0.0)]));
    }

    #[test]
    fn an_island_gets_a_closed_stroke_in_the_color_around_it() {
        let hole: &[(f64, f64)] = &[(2.0, 2.0), (2.0, 4.0), (4.0, 4.0), (4.0, 2.0)];
        let around = polygon(&[&[(0.0, 0.0), (6.0, 0.0), (6.0, 6.0), (0.0, 6.0)], hole], 10);
        let island = polygon(&[&[(2.0, 2.0), (4.0, 2.0), (4.0, 4.0), (2.0, 4.0)]], 20);
        let fills = strokes(&[around, island]);
        assert_eq!(fills.len(), 1, "{fills:?}");
        assert_eq!(fills[0].color, [10; 3]);
        assert_eq!(fills[0].path.subpaths.len(), 1);
        assert!(fills[0].path.subpaths[0].closed);
        assert_eq!(points(&fills[0].path.subpaths[0]), hole);
    }

    #[test]
    fn a_shared_run_across_a_subpaths_first_anchor_stays_one_stroke() {
        // The left square starts halfway along the edge it shares; the right one splits that edge
        // at the same point.
        let left = polygon(&[&[(2.0, 1.0), (2.0, 2.0), (0.0, 2.0), (0.0, 0.0), (2.0, 0.0)]], 10);
        let right = polygon(&[&[(2.0, 0.0), (4.0, 0.0), (4.0, 2.0), (2.0, 2.0), (2.0, 1.0)]], 20);
        let fills = strokes(&[left, right]);
        assert_eq!(fills.len(), 1, "{fills:?}");
        assert_eq!(fills[0].path.subpaths.len(), 1);
        assert_eq!(points(&fills[0].path.subpaths[0]), [(2.0, 0.0), (2.0, 1.0), (2.0, 2.0)]);
    }

    #[test]
    fn edges_shared_with_no_other_filled_path_get_no_stroke() {
        let square = polygon(&[&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]], 10);
        assert_eq!(fills(std::slice::from_ref(&square), (100, 100)), None);
        // A stroked path along the square's edge is not a filled neighbor.
        let mut stroked = polygon(&[&[(2.0, 0.0), (4.0, 0.0), (4.0, 2.0), (2.0, 2.0)]], 20);
        stroked.stroke = Some(1.0);
        assert_eq!(fills(&[square.clone(), stroked], (100, 100)), None);
        // Nor is a path drawn as a live shape, whose outline is not the traced one.
        let mut shape = polygon(&[&[(2.0, 0.0), (4.0, 0.0), (4.0, 2.0), (2.0, 2.0)]], 20);
        shape.shape = Some(TracedShape::Rectangle { center: Point::new(3.0, 1.0), width: 2.0, height: 2.0, radius: 0.0, rotation: 0.0 });
        assert_eq!(fills(&[square, shape], (100, 100)), None);
    }

    #[test]
    fn paths_of_one_color_share_one_stroke_and_gradients_keep_their_own() {
        let a = polygon(&[&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]], 10);
        let b = polygon(&[&[(0.0, 4.0), (2.0, 4.0), (2.0, 6.0), (0.0, 6.0)]], 10);
        let c = polygon(&[&[(2.0, 0.0), (4.0, 0.0), (4.0, 6.0), (2.0, 6.0), (2.0, 4.0), (2.0, 2.0)]], 20);
        let fills = strokes(&[a.clone(), b.clone(), c.clone()]);
        assert_eq!(fills.len(), 1, "{fills:?}");
        assert_eq!(fills[0].path.subpaths.iter().map(points).collect::<Vec<_>>(), [vec![(2.0, 0.0), (2.0, 2.0)], vec![(2.0, 4.0), (2.0, 6.0)]]);
        // With a gradient each, the two paths of one color get a stroke each, with their gradient.
        let ramp = |to: u8| TracedGradient { start: Point::new(0.0, 0.0), end: Point::new(2.0, 0.0), stops: vec![(0.0, [10; 3]), (1.0, [to; 3])] };
        let (mut a, mut b) = (a, b);
        (a.gradient, b.gradient) = (Some(ramp(30)), Some(ramp(40)));
        let fills = strokes(&[a.clone(), b.clone(), c]);
        assert_eq!(fills.len(), 2, "{fills:?}");
        assert_eq!(fills.iter().map(|f| f.gradient.clone()).collect::<Vec<_>>(), [a.gradient, b.gradient]);
        assert_eq!(
            fills.iter().map(|f| points(&f.path.subpaths[0])).collect::<Vec<_>>(),
            [vec![(2.0, 0.0), (2.0, 2.0)], vec![(2.0, 4.0), (2.0, 6.0)]]
        );
    }

    #[test]
    fn the_strokes_are_clipped_to_the_images_box() {
        // A diagonal edge meeting the border: a corner of the stroke's butt end reaches past the
        // box, and the clip is the box.
        let lower = polygon(&[&[(0.0, 0.0), (3.0, 0.0), (1.0, 2.0), (0.0, 2.0)]], 10);
        let upper = polygon(&[&[(3.0, 0.0), (4.0, 0.0), (4.0, 2.0), (1.0, 2.0)]], 20);
        let diagonal = fills(&[lower, upper], (4, 2)).unwrap();
        assert_eq!(points(&diagonal.strokes[0].path.subpaths[0]), [(3.0, 0.0), (1.0, 2.0)]);
        assert_eq!(diagonal.clip.subpaths.len(), 1);
        assert!(diagonal.clip.subpaths[0].closed);
        assert_eq!(points(&diagonal.clip.subpaths[0]), [(0.0, 0.0), (4.0, 0.0), (4.0, 2.0), (0.0, 2.0)]);
        assert_eq!(diagonal.anchor_count(), 2 + 4);
    }

    /// Two colors side by side, the edge between them anti-aliased at `x = 10.4`.
    fn two_colors() -> Raster {
        Raster::from_fn(24, 16, |x, _| {
            let share = (10.4 - f64::from(x)).clamp(0.0, 1.0);
            let mix = |a: f64, b: f64| (a * share + b * (1.0 - share)).round() as u8;
            [mix(200.0, 30.0), mix(40.0, 90.0), mix(50.0, 200.0), 255]
        })
    }

    fn color(method: Method) -> TraceParams {
        TraceParams { mode: Mode::Color, colors: 2, method, ignore_white: false, ..TraceParams::default() }
    }

    #[test]
    fn an_abutting_trace_strokes_the_edges_its_filled_paths_share() {
        let params = color(Method::Abutting);
        let res = trace(&two_colors(), &params);
        assert_eq!(res.paths.len(), 2, "{res:?}");
        let fills = seam_fills(&res.paths, &params, (24, 16)).unwrap();
        assert_eq!(fills.strokes.len(), 1, "{fills:?}");
        let seam = &fills.strokes[0];
        assert_eq!((seam.color, seam.stroke), (res.paths[0].color, Some(SEAM_WIDTH)));
        // Every piece of the stroke is a piece of both paths, and it runs from the image's top to
        // its bottom along the edge, not along the image's border.
        let (lower, upper) = (keys(&res.paths[0]), keys(&res.paths[1]));
        assert!(keys(seam).iter().all(|k| lower.contains(k) && upper.contains(k)));
        assert!(seam.path.anchor_count() <= res.anchor_count());
        for a in seam.path.subpaths.iter().flat_map(|sp| &sp.anchors) {
            assert!((a.p.x - 10.4).abs() < 0.5, "{a:?}");
        }
        let ys: Vec<f64> = seam.path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p.y)).collect();
        assert_eq!((ys.iter().copied().fold(f64::INFINITY, f64::min), ys.iter().copied().fold(f64::NEG_INFINITY, f64::max)), (0.0, 16.0));
        // The clip is the image's box.
        let corners: Vec<(f64, f64)> = fills.clip.subpaths.iter().flat_map(points).collect();
        assert_eq!(corners, [(0.0, 0.0), (24.0, 0.0), (24.0, 16.0), (0.0, 16.0)]);
    }

    #[test]
    fn only_abutting_fills_get_seam_strokes() {
        let seams = |img: &Raster, params: &TraceParams| seam_fills(&trace(img, params).paths, params, (img.width, img.height));
        let img = two_colors();
        assert_eq!(seams(&img, &color(Method::Overlapping)), None);
        assert_eq!(seams(&img, &TraceParams { fills: false, strokes: true, ..color(Method::Abutting) }), None);
        // Flat Logo's shapes are stacked, whatever the method says.
        let abutting = trace(&img, &color(Method::Abutting));
        assert_eq!(seam_fills(&abutting.paths, &TraceParams { mode: Mode::Logo, ..color(Method::Abutting) }, (24, 16)), None);
        // A color left out leaves no neighbor: a dark bar on white, with Ignore White.
        let bar = Raster::from_fn(24, 16, |x, _| if (8..16).contains(&x) { [0, 0, 0, 255] } else { [255; 4] });
        let bw = TraceParams { mode: Mode::BlackAndWhite, method: Method::Abutting, ignore_white: true, ..TraceParams::default() };
        let res = trace(&bar, &bw);
        assert_eq!(res.paths.len(), 1, "{res:?}");
        assert_eq!(seam_fills(&res.paths, &bw, (24, 16)), None);
        // Both colors traced, the bar's two sides are shared.
        let both = seams(&bar, &TraceParams { ignore_white: false, ..bw }).unwrap();
        assert_eq!(both.strokes.iter().map(|s| s.path.subpaths.len()).sum::<usize>(), 2, "{both:?}");
    }
}
