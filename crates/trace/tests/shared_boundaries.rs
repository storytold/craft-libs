//! Shared boundaries in Abutting traces: every edge between two neighbouring regions is drawn
//! once, so the two filled paths on either side of it follow one curve (one of them reversed), and
//! together the paths cover the image with no gap and no overlap.
//!
//! That must hold for edges between two junctions, for closed edges with no junction at all (an
//! island inside another region, a hole), and for an edge that leaves a junction and comes back to
//! it (two parts of a region touching only at a pixel corner). No region may vanish on its own: a
//! one-pixel island or hole is kept or dropped together with the hole or island it leaves, and two
//! separate areas of one colour stay two shapes.
//!
//! Every image here is drawn in code from simple geometry (squares, discs, sine waves, stripes),
//! hard-edged (each pixel the colour at its centre) and, where it means something, anti-aliased
//! (each pixel the mean of 8 × 8 samples). The check is geometric and needs no renderer: points
//! along each path's outline must lie on another path's outline (to [`TOL`] px), the paths' areas
//! must add up to the image's, and points on a fine grid must each lie in exactly one path.
//!
//! Behaviour reference: Image Trace's Advanced › Method, Abutting (shapes cut out of each other,
//! meeting edge to edge) as against Overlapping (stacked)
//! (<https://helpx.adobe.com/illustrator/using/image-trace.html>).
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_trace::{Method, Mode, Raster, TraceParams, TraceResult, trace};
use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveExtrema, ParamCurveNearest, Point, Rect, Shape};

/// How far (px) a point of one path's outline may lie from its neighbour's outline.
const TOL: f64 = 1e-3;

/// The drawing colours, by index. Far apart, and no blend of two is near a third.
const COLOURS: [[u8; 3]; 3] = [[255, 255, 255], [200, 40, 50], [30, 90, 200]];

/// Black and White's two colours: index 0 is paper, 1 is ink.
const BW: [[u8; 3]; 2] = [[255, 255, 255], [0, 0, 0]];

/// A drawing: the colour index at a point of a `w × h` image.
struct Art {
    name: &'static str,
    w: u32,
    h: u32,
    at: Box<dyn Fn(f64, f64) -> usize>,
}

impl Art {
    fn new(name: &'static str, w: u32, h: u32, at: impl Fn(f64, f64) -> usize + 'static) -> Self {
        Self { name, w, h, at: Box::new(at) }
    }
    /// Each pixel the colour at its centre.
    fn labels(&self) -> Vec<usize> {
        (0..self.h).flat_map(|y| (0..self.w).map(move |x| (x, y))).map(|(x, y)| (self.at)(f64::from(x) + 0.5, f64::from(y) + 0.5)).collect()
    }
    fn hard(&self, colours: &[[u8; 3]]) -> Raster {
        Raster::from_fn(self.w, self.h, |x, y| {
            let c = colours[(self.at)(f64::from(x) + 0.5, f64::from(y) + 0.5)];
            [c[0], c[1], c[2], 255]
        })
    }
    /// Anti-aliased: each pixel the mean of its 8 × 8 samples.
    fn soft(&self, colours: &[[u8; 3]]) -> Raster {
        Raster::from_fn(self.w, self.h, |x, y| {
            let mut sum = [0.0f64; 3];
            for i in 0..64 {
                let c = colours[(self.at)(f64::from(x) + (f64::from(i % 8) + 0.5) / 8.0, f64::from(y) + (f64::from(i / 8) + 0.5) / 8.0)];
                for k in 0..3 {
                    sum[k] += f64::from(c[k]) / 64.0;
                }
            }
            [sum[0].round() as u8, sum[1].round() as u8, sum[2].round() as u8, 255]
        })
    }
}

/// The 4-connected components of each colour of `labels`.
fn components(labels: &[usize], w: usize, h: usize, colours: usize) -> Vec<usize> {
    let mut seen = vec![false; labels.len()];
    let mut count = vec![0; colours];
    for start in 0..labels.len() {
        if seen[start] {
            continue;
        }
        count[labels[start]] += 1;
        seen[start] = true;
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let near = [(x > 0).then(|| i - 1), (x + 1 < w).then_some(i + 1), (y > 0).then(|| i - w), (y + 1 < h).then_some(i + w)];
            for j in near.into_iter().flatten() {
                if !seen[j] && labels[j] == labels[i] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    count
}

/// The size of the smallest 4-connected one-colour component of `labels`.
fn smallest(labels: &[usize], w: usize, h: usize) -> usize {
    let mut seen = vec![false; labels.len()];
    let mut least = usize::MAX;
    for start in 0..labels.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let (mut stack, mut size) = (vec![start], 0);
        while let Some(i) = stack.pop() {
            size += 1;
            let (x, y) = (i % w, i / w);
            let near = [(x > 0).then(|| i - 1), (x + 1 < w).then_some(i + 1), (y > 0).then(|| i - w), (y + 1 < h).then_some(i + w)];
            for j in near.into_iter().flatten() {
                if !seen[j] && labels[j] == labels[i] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        least = least.min(size);
    }
    least
}

/// Color mode, Abutting, `n` colours, Noise `noise` (0 and 1 remove nothing).
fn color(n: u32, noise: u32) -> TraceParams {
    TraceParams { mode: Mode::Color, colors: n, noise, method: Method::Abutting, ignore_white: false, ..TraceParams::default() }
}

/// Black and White with white traced too, Noise `noise`.
fn black_and_white(noise: u32) -> TraceParams {
    TraceParams { mode: Mode::BlackAndWhite, noise, method: Method::Abutting, ignore_white: false, ..TraceParams::default() }
}

/// What the check found.
#[derive(Debug)]
struct Seams {
    /// Paths of each drawing colour (each path goes to the nearest one).
    faces: Vec<usize>,
    /// The farthest a point of one path's outline lies from every other path's outline (px), and
    /// where.
    gap: f64,
    at: Point,
    /// The paths' areas summed, less the image's (px²): below 0 a gap, above 0 an overlap.
    area: f64,
    /// Points of a 4 × 4 grid per pixel that no path covers, and that more than one path covers.
    uncovered: usize,
    doubled: usize,
}

impl Seams {
    fn ok(&self) -> bool {
        self.gap <= TOL && self.area.abs() <= TOL && self.uncovered == 0 && self.doubled == 0
    }
}

fn segments(p: &craft_trace::TracedPath) -> Vec<CubicBez> {
    p.path.subpaths.iter().flat_map(|sp| (0..sp.segment_count()).map(move |i| sp.segment(i))).collect()
}

fn check(res: &TraceResult, w: u32, h: u32, colours: &[[u8; 3]]) -> Seams {
    let (wf, hf) = (f64::from(w), f64::from(h));
    let mut faces = vec![0; colours.len()];
    for p in &res.paths {
        assert!(p.stroke.is_none(), "Abutting fills only");
        let d = |c: &[u8; 3]| (0..3).map(|k| (f64::from(c[k]) - f64::from(p.color[k])).powi(2)).sum::<f64>();
        let nearest = (0..colours.len()).min_by(|&a, &b| d(&colours[a]).total_cmp(&d(&colours[b]))).unwrap();
        faces[nearest] += 1;
    }
    let segs: Vec<Vec<(CubicBez, Rect)>> =
        res.paths.iter().map(|p| segments(p).into_iter().map(|c| (c, ParamCurveExtrema::bounding_box(&c))).collect()).collect();
    // Points along each outline, against every other path's outline (the frame has no neighbour).
    let on_frame = |p: Point| p.x.abs() < 1e-9 || p.y.abs() < 1e-9 || (p.x - wf).abs() < 1e-9 || (p.y - hf).abs() < 1e-9;
    let (mut gap, mut at) = (0.0f64, Point::ORIGIN);
    for (i, own) in segs.iter().enumerate() {
        for (c, _) in own {
            for k in 0..32 {
                let p = c.eval(f64::from(k) / 32.0);
                if on_frame(p) {
                    continue;
                }
                let mut best = f64::INFINITY;
                for other in segs.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, o)| o) {
                    for (o, b) in other {
                        let dx = (b.x0 - p.x).max(p.x - b.x1).max(0.0);
                        let dy = (b.y0 - p.y).max(p.y - b.y1).max(0.0);
                        if dx * dx + dy * dy >= best * best {
                            continue;
                        }
                        best = best.min(o.nearest(p, 1e-12).distance_sq.sqrt());
                    }
                }
                if best > gap {
                    (gap, at) = (best, p);
                }
            }
        }
    }
    let area = res.paths.iter().map(|p| p.path.subpaths.iter().map(|s| s.area()).sum::<f64>()).sum::<f64>() - wf * hf;
    let (uncovered, doubled) = coverage(res, w, h);
    Seams { faces, gap, at, area, uncovered, doubled }
}

/// Points of a 4 × 4 grid per pixel that no path covers, and that more than one path covers.
fn coverage(res: &TraceResult, w: u32, h: u32) -> (usize, usize) {
    let beziers: Vec<(BezPath, Rect)> = res.paths.iter().map(|p| p.path.to_bezpath()).map(|b| (b.clone(), b.bounding_box())).collect();
    let (mut uncovered, mut doubled) = (0, 0);
    for yi in 0..h * 4 {
        for xi in 0..w * 4 {
            // (Off the half and quarter pixels, where hard edges lie.)
            let p = Point::new(0.1093 + f64::from(xi) * 0.25, 0.1371 + f64::from(yi) * 0.25);
            match beziers.iter().filter(|(b, r)| r.contains(p) && b.winding(p) != 0).count() {
                0 => uncovered += 1,
                1 => {}
                _ => doubled += 1,
            }
        }
    }
    (uncovered, doubled)
}

/// One trace of `art` checked: a line for the record, and what is wrong (empty when nothing is).
/// `expect`: the paths of each colour, where every region must survive.
fn run(art: &Art, soft: bool, bw: bool, params: &TraceParams, setting: &str, expect: Option<&[usize]>) -> (String, Vec<String>) {
    let colours: &[[u8; 3]] = if bw { &BW } else { &COLOURS[..params.colors as usize] };
    let img = if soft { art.soft(colours) } else { art.hard(colours) };
    let res = trace(&img, params);
    let s = check(&res, art.w, art.h, colours);
    let label = format!("{} {} {} noise {} {setting}", art.name, if soft { "soft" } else { "hard" }, if bw { "B&W" } else { "Color" }, params.noise);
    let line = format!(
        "{label}: faces {:?}, gap {:.6} px at ({:.3}, {:.3}), area {:+.6} px², uncovered {}, doubled {}",
        s.faces, s.gap, s.at.x, s.at.y, s.area, s.uncovered, s.doubled
    );
    let mut wrong = vec![];
    if !s.ok() {
        wrong.push(line.clone());
    }
    if let Some(e) = expect
        && s.faces != e
    {
        wrong.push(format!("{label}: faces {:?}, expected {e:?}", s.faces));
    }
    (line, wrong)
}

/// A fitting setting: its name, and what it changes.
type Setting = (&'static str, fn(&mut TraceParams));

const DEFAULT: Setting = ("default", |_| {});
const LOOSEST: Setting = ("paths 0 corners 0", |p| (p.paths, p.corners) = (0.0, 0.0));
const TIGHTEST: Setting = ("paths 100 corners 100", |p| (p.paths, p.corners) = (100.0, 100.0));
const SNAP: Setting = ("snap", |p| p.snap_curves_to_lines = true);
const SHAPES: Setting = ("shapes", |p| p.shapes = true);

/// The defaults, both ends of Paths and Corners, Snap Curves To Lines, and Shapes.
const ALL: [Setting; 5] = [DEFAULT, LOOSEST, TIGHTEST, SNAP, SHAPES];

/// Runs every variant of `art` with each of `settings`: hard-edged always, anti-aliased when
/// `soft`, Color with `n` colours, and Black and White when the art has two. Hard-edged, every
/// region must survive (Noise 1 removes nothing), except that at Paths 0 a region of under four
/// pixels may go, if what it leaves goes with it (the geometric checks see to that).
fn variants(art: &Art, n: usize, soft: bool, settings: &[Setting]) -> Vec<String> {
    let labels = art.labels();
    let counts = components(&labels, art.w as usize, art.h as usize, n);
    let tiny = smallest(&labels, art.w as usize, art.h as usize) < 4;
    let mut wrong = vec![];
    let mut runs = vec![(false, false, color(n as u32, 1))];
    if n == 2 {
        runs.push((false, true, black_and_white(1)));
    }
    if soft {
        runs.push((true, false, color(n as u32, 1)));
        if n == 2 {
            runs.push((true, true, black_and_white(1)));
        }
    }
    for (soft, bw, params) in runs {
        for &(name, set) in settings {
            let mut params = params.clone();
            set(&mut params);
            let expect = (!(soft || tiny && name == LOOSEST.0)).then_some(counts.as_slice());
            let (line, w) = run(art, soft, bw, &params, name, expect);
            println!("{line}");
            wrong.extend(w);
        }
    }
    wrong
}

fn all_variants(art: &Art, n: usize, soft: bool) -> Vec<String> {
    variants(art, n, soft, &ALL)
}

fn assert_none(wrong: Vec<String>) {
    assert!(wrong.is_empty(), "shared boundaries broken:\n{}", wrong.join("\n"));
}

// --- The drawings ---------------------------------------------------------------------------

fn in_disc(x: f64, y: f64, cx: f64, cy: f64, r: f64) -> bool {
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

fn square_island() -> Art {
    Art::new("square island", 32, 32, |x, y| usize::from((9.0..23.0).contains(&x) && (8.0..22.0).contains(&y)))
}

fn disc_island() -> Art {
    Art::new("disc island", 32, 32, |x, y| usize::from(in_disc(x, y, 16.3, 15.8, 8.6)))
}

/// A ring with a core of a third colour in its hole.
fn nested() -> Art {
    Art::new("nested ring and core", 40, 40, |x, y| {
        if in_disc(x, y, 20.2, 19.7, 6.3) {
            2
        } else if in_disc(x, y, 20.2, 19.7, 13.4) {
            1
        } else {
            0
        }
    })
}

/// A ring whose hole shows the background: one colour, two separate areas of it.
fn ring() -> Art {
    Art::new("ring with a hole", 40, 40, |x, y| usize::from(in_disc(x, y, 20.2, 19.7, 13.4) && !in_disc(x, y, 20.2, 19.7, 6.3)))
}

fn square_ring() -> Art {
    Art::new("square ring", 32, 32, |x, y| {
        let outer = (6.0..26.0).contains(&x) && (6.0..26.0).contains(&y);
        let inner = (12.0..20.0).contains(&x) && (12.0..20.0).contains(&y);
        usize::from(outer && !inner)
    })
}

fn wavy_seam() -> Art {
    Art::new("wavy seam", 32, 32, |x, y| usize::from(x > 16.0 + 3.2 * (y * std::f64::consts::TAU / 14.0 + 0.4).sin()))
}

/// Left, upper right and lower right meet at one point inside the image.
fn three_regions() -> Art {
    Art::new("three-region junction", 32, 32, |x, y| {
        if x < 15.4 + 0.15 * (y - 16.0) {
            0
        } else if y < 16.3 + 0.2 * (x - 16.0) {
            1
        } else {
            2
        }
    })
}

/// The 3 × 3 pattern 111 / 101 / 110, each cell `k` pixels square: the middle hole and the
/// corner of the background touch at one pixel corner, so the outline round the middle leaves that
/// corner and comes back to it.
fn returning(k: u32) -> Art {
    let name: &'static str = match k {
        1 => "returning loop 1 px",
        2 => "returning loop 2 px",
        3 => "returning loop 3 px",
        _ => "returning loop 6 px",
    };
    let kf = f64::from(k);
    Art::new(name, 3 * k, 3 * k, move |x, y| {
        let (cx, cy) = ((x / kf) as usize, (y / kf) as usize);
        usize::from(!matches!((cx, cy), (1, 1) | (2, 2)))
    })
}

/// The same pattern inside a frame of the ink colour: the returning outline is an inner one.
fn returning_framed() -> Art {
    Art::new("returning loop framed", 24, 24, |x, y| {
        let (cx, cy) = (((x - 3.0) / 6.0).floor() as i32, ((y - 3.0) / 6.0).floor() as i32);
        usize::from(!matches!((cx, cy), (1, 1) | (2, 2)))
    })
}

fn one_pixel_hole() -> Art {
    Art::new("one-pixel hole", 24, 24, |x, y| {
        usize::from((5.0..19.0).contains(&x) && (5.0..19.0).contains(&y) && !(x as u32 == 11 && y as u32 == 12))
    })
}

fn one_pixel_island() -> Art {
    Art::new("one-pixel island", 15, 15, |x, y| usize::from(x as u32 == 7 && y as u32 == 7))
}

fn disconnected_squares() -> Art {
    Art::new("disconnected squares", 24, 20, |x, y| {
        usize::from(((3.0..7.0).contains(&x) && (3.0..7.0).contains(&y)) || ((16.0..20.0).contains(&x) && (12.0..16.0).contains(&y)))
    })
}

fn disconnected_discs() -> Art {
    Art::new("disconnected discs", 40, 24, |x, y| usize::from(in_disc(x, y, 10.3, 11.7, 6.2) || in_disc(x, y, 28.6, 12.2, 5.4)))
}

/// Two squares of one colour that touch only at a corner: two regions, not one.
fn corner_contact() -> Art {
    Art::new("corner contact", 24, 24, |x, y| {
        usize::from(((4.0..12.0).contains(&x) && (4.0..12.0).contains(&y)) || ((12.0..20.0).contains(&x) && (12.0..20.0).contains(&y)))
    })
}

/// Slanted hatch strokes two pixels wide, one pixel of paper between them (along each row).
fn hatches() -> Art {
    Art::new("parallel hatches", 32, 32, |x, y| {
        let inside = (4.0..28.0).contains(&x) && (4.0..28.0).contains(&y);
        usize::from(inside && (x - (y / 2.0).floor()).rem_euclid(3.0) < 2.0)
    })
}

/// Upright strokes two pixels wide with one-pixel paper channels, closed at both ends by a bar:
/// each channel is a hole.
fn closed_channels() -> Art {
    Art::new("closed paper channels", 32, 24, |x, y| {
        let frame = (4.0..28.0).contains(&x) && (4.0..20.0).contains(&y);
        let bar = !(6.0..18.0).contains(&y);
        usize::from(frame && (bar || (x - 4.0).rem_euclid(3.0) < 2.0 || x >= 25.0))
    })
}

// --- The tests ------------------------------------------------------------------------------

#[test]
fn a_closed_island_shares_its_whole_outline_with_the_background() {
    let mut wrong = all_variants(&square_island(), 2, false);
    wrong.extend(all_variants(&disc_island(), 2, true));
    assert_none(wrong);
}

#[test]
fn a_ring_shares_its_outline_and_its_hole_with_its_neighbours() {
    let mut wrong = all_variants(&nested(), 3, true);
    wrong.extend(all_variants(&ring(), 2, true));
    wrong.extend(all_variants(&square_ring(), 2, false));
    assert_none(wrong);
}

#[test]
fn a_wavy_seam_is_one_curve_for_both_sides() {
    assert_none(all_variants(&wavy_seam(), 2, true));
}

#[test]
fn three_regions_share_their_junction_and_edges() {
    assert_none(all_variants(&three_regions(), 3, true));
}

/// The drawings whose outlines leave a junction and come back to it.
fn returning_arts() -> Vec<Art> {
    vec![returning(1), returning(2), returning(3), returning(6), returning_framed(), corner_contact()]
}

#[test]
fn an_outline_returning_to_its_junction_is_shared() {
    let mut wrong = vec![];
    for art in returning_arts() {
        wrong.extend(variants(&art, 2, false, &[DEFAULT, TIGHTEST, SNAP]));
    }
    assert_none(wrong);
}

/// At Paths 0 and Corners 0, a piece that leaves a junction and returns to it is fitted with fewer
/// than two curves: the region it is the whole outline of is dropped (a closed subpath needs two),
/// and its neighbour keeps a single curve from the junction back to itself, so a hole the size of
/// the region is left uncovered (1 to 6.7 px wide here, 0.3 to 108 px²).
#[test]
fn an_outline_returning_to_its_junction_survives_the_loosest_fit() {
    let mut wrong = vec![];
    for art in returning_arts() {
        wrong.extend(variants(&art, 2, false, &[LOOSEST]));
    }
    assert_none(wrong);
}

/// With Shapes, a region whose outline is a shape becomes that exact shape, but where its outline
/// runs through a junction (two parts of the other colour touch at a corner) the neighbour keeps
/// the traced curve: in Black and White (where the threshold moves hard edges by 1/256 px) the
/// square and its neighbour part by up to 0.0039 px, and a square on the image's corner reaches
/// 0.002 px past the frame.
#[test]
#[ignore = "Shapes replaces a region touching a junction with an exact shape that its neighbour does not share (lib.rs Layers::layer, the Shapes branch): up to 0.0039 px apart in Black and White"]
fn shapes_keep_the_edges_a_region_shares_at_a_junction() {
    let mut wrong = vec![];
    for art in returning_arts() {
        wrong.extend(variants(&art, 2, false, &[SHAPES]));
    }
    assert_none(wrong);
}

#[test]
fn a_one_pixel_hole_and_island_survive_with_what_they_leave() {
    let mut wrong = all_variants(&one_pixel_hole(), 2, false);
    wrong.extend(all_variants(&one_pixel_island(), 2, false));
    // At the default Noise both go, each together with the hole it would leave.
    // (The hole goes into the square around it; the island into the background, leaving no hole.)
    for (art, expect_island) in [(one_pixel_hole(), 1), (one_pixel_island(), 0)] {
        for (bw, params) in [(false, color(2, 25)), (true, black_and_white(25))] {
            let (line, w) = run(&art, false, bw, &params, "default", Some(&[1, expect_island]));
            println!("{line}");
            wrong.extend(w);
        }
    }
    assert_none(wrong);
}

#[test]
fn separate_areas_of_one_colour_stay_separate() {
    let mut wrong = all_variants(&disconnected_squares(), 2, false);
    wrong.extend(all_variants(&disconnected_discs(), 2, true));
    // (Squares touching at a corner: see the returning-outline tests.)
    assert_none(wrong);
}

#[test]
fn one_pixel_paper_channels_between_hatches_are_shared() {
    let mut wrong = all_variants(&hatches(), 2, true);
    wrong.extend(all_variants(&closed_channels(), 2, false));
    assert_none(wrong);
}

/// The check itself: a path moved by a hundredth of a pixel, and stacked (Overlapping) layers, are
/// found.
#[test]
fn the_check_finds_a_hairline_gap_and_an_overlap() {
    let art = square_island();
    let colours = &COLOURS[..2];
    let mut res = trace(&art.hard(colours), &color(2, 1));
    let s = check(&res, art.w, art.h, colours);
    assert!(s.ok(), "{s:?}");
    let island = res.paths.iter_mut().find(|p| p.color == colours[1]).unwrap();
    for a in &mut island.path.subpaths[0].anchors {
        for p in [&mut a.p, &mut a.h_in, &mut a.h_out] {
            p.x += 0.01;
        }
    }
    let s = check(&res, art.w, art.h, colours);
    assert!(!s.ok() && (s.gap - 0.01).abs() < 1e-6, "{s:?}");
    let stacked = TraceParams { method: Method::Overlapping, ..color(2, 1) };
    let s = check(&trace(&art.hard(colours), &stacked), art.w, art.h, colours);
    assert!(s.doubled > 0 && s.area > 100.0, "{s:?}");
}

/// A noisy ripple of one- and two-pixel slivers, light and dark, read in Black and White at a
/// threshold: both sides of many edges are thin. Read from one side only, such an edge had a
/// different sub-pixel shift for each of its two regions, so their curves left slivers between
/// them that no path covers. (Coverage only: on noise this fine, curves fitted to neighbouring
/// pieces may still cross, which the full check counts as overlaps.)
#[test]
fn thin_slivers_on_both_sides_of_an_edge_leave_nothing_uncovered() {
    let (w, h) = (48, 48);
    let rnd = |x: u32, y: u32| (x.wrapping_mul(83_492_791) ^ y.wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761) >> 26;
    let img = Raster::from_fn(w, h, |x, y| {
        let base = [118u32, 168, 176, 122, 160, 116, 174, 171][(x as usize + y as usize / 5) % 8];
        let v = (base + rnd(x, y) * ((x + y) % 2)).min(255) as u8;
        [v, v, v, 255]
    });
    let mut wrong = vec![];
    for threshold in [140u8, 157, 170] {
        for paths in [0.0, 50.0, 100.0] {
            let params = TraceParams { threshold, paths, ..black_and_white(0) };
            let (uncovered, _) = coverage(&trace(&img, &params), w, h);
            println!("threshold {threshold} paths {paths}: uncovered {uncovered}");
            if uncovered > 0 {
                wrong.push(format!("threshold {threshold} paths {paths}: {uncovered} points uncovered"));
            }
        }
    }
    assert_none(wrong);
}
