//! Moving a mask's coordinates between frames: [`attach`] puts an old mask's coordinates (on the
//! corrected frame) on the photo before the warp ([`craft_develop_model::Mask::on_photo`]), and
//! [`turn`] follows a quarter turn or flip of the photo (Rotate Left / Right). Gradients, brush
//! strokes and clicks move point by point; stored AI segmentations are resampled.

use craft_develop_model::{Mask, MaskShape, SegMask};
use craft_geom::{Orientation, Point};

use crate::geometry::Frame;
use crate::masks::SegGrid;

/// Put mask `m`, drawn on the corrected (transformed) image of `frame`, on the photo before the
/// warp: it then renders where it did under `frame` and stays on that content whatever lens
/// corrections, Upright or Transform do later. A mask already on the photo is left alone.
///
/// Points move exactly; a gradient's shape (a radial's radii and angle, a linear's direction and
/// width) goes through the warp's local linear part at the gradient, so it covers the same region
/// as long as the warp is close to linear across it (lens and perspective warps are smooth).
pub fn attach(m: &mut Mask, frame: &Frame) {
    if m.on_photo {
        return;
    }
    m.on_photo = true;
    let f = frame.for_mask(true);
    if f.mask_warp().is_none() {
        // without a warp both are the same coordinates
        return;
    }
    let fwd = |p: Point| f.norm_to_mask(p);
    let back = |p: Point| f.mask_to_norm(p);
    let l = f.ow.max(f.oh).max(1e-9);
    let (sx, sy) = (f.ow / l, f.oh / l);
    for c in &mut m.components {
        match &mut c.shape {
            MaskShape::Linear { start, end } => (*start, *end) = attach_linear(*start, *end, &fwd, sx, sy),
            MaskShape::Radial { center, rx, ry, angle, .. } => {
                let mid = *center;
                *center = fwd(mid);
                if let Some(j) = jacobian(&fwd, mid, sx, sy)
                    && let Some((nrx, nry, na)) = attach_ellipse(j, *rx, *ry, *angle)
                {
                    (*rx, *ry, *angle) = (nrx, nry, na);
                }
            }
            _ => {}
        }
    }
    remap(m, &fwd, &back, 0.0, false);
}

/// The local linear part of `f` (normalized → normalized) at `p`, in long-edge units (isotropic;
/// `sx`, `sy` are the frame's sides over its long side): `[a, b, c, d]` takes `(dx, dy)` to
/// `(a dx + b dy, c dx + d dy)`. `None` where `f` isn't finite or collapses.
fn jacobian(f: &dyn Fn(Point) -> Point, p: Point, sx: f64, sy: f64) -> Option<[f64; 4]> {
    const H: f64 = 1e-4;
    let (o, px, py) = (f(p), f(Point::new(p.x + H, p.y)), f(Point::new(p.x, p.y + H)));
    let j = [(px.x - o.x) / H, (py.x - o.x) * sx / (H * sy), (px.y - o.y) * sy / (H * sx), (py.y - o.y) / H];
    let det = j[0] * j[3] - j[1] * j[2];
    (j.iter().all(|v| v.is_finite()) && det.is_finite() && det.abs() > 1e-9).then_some(j)
}

/// A linear gradient from `start` to `end` (normalized, on the corrected frame) on the photo:
/// its start moves through `f`, and its direction and length follow the local linear part, so
/// its bands (lines at right angles to it) land on the same content. The end moves through `f`
/// alone where that part isn't defined.
fn attach_linear(start: Point, end: Point, f: &dyn Fn(Point) -> Point, sx: f64, sy: f64) -> (Point, Point) {
    let (a, plain_end) = (f(start), f(end));
    let mid = Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
    let Some([ja, jb, jc, jd]) = jacobian(f, mid, sx, sy) else { return (a, plain_end) };
    // the bands are level sets of (p - start)·d / |d|²: through J they keep that form with the
    // normal J^-T d, and the same |d|² sets the length
    let d = ((end.x - start.x) * sx, (end.y - start.y) * sy);
    let det = ja * jd - jb * jc;
    let n = ((jd * d.0 - jc * d.1) / det, (-jb * d.0 + ja * d.1) / det);
    let (dd, nn) = (d.0 * d.0 + d.1 * d.1, n.0 * n.0 + n.1 * n.1);
    if !(dd.is_finite() && nn.is_finite()) || nn < 1e-18 {
        return (a, plain_end);
    }
    let k = dd / nn;
    let e = Point::new(a.x + n.0 * k / sx, a.y + n.1 * k / sy);
    if e.x.is_finite() && e.y.is_finite() { (a, e) } else { (a, plain_end) }
}

/// An ellipse of radii `rx`, `ry` (long-edge units) turned by `angle` degrees, through the linear
/// map `j` (long-edge units): the radii and angle of the ellipse it becomes, `rx` staying the
/// radius along the image of the old `rx` axis. `None` when the result isn't usable.
fn attach_ellipse(j: [f64; 4], rx: f64, ry: f64, angle: f64) -> Option<(f64, f64, f64)> {
    let (s, c) = angle.to_radians().sin_cos();
    // M = J · R(angle) · diag(rx, ry): its columns are the images of the two semi-axes
    let m = [(j[0] * c + j[1] * s) * rx, (j[1] * c - j[0] * s) * ry, (j[2] * c + j[3] * s) * rx, (j[3] * c - j[2] * s) * ry];
    // the new ellipse's axes: eigenvectors of M Mᵀ, its radii the square roots of the eigenvalues
    let (p, q, r) = (m[0] * m[0] + m[1] * m[1], m[0] * m[2] + m[1] * m[3], m[2] * m[2] + m[3] * m[3]);
    let (half, det) = ((p + r) / 2.0, p * r - q * q);
    let root = (half * half - det).max(0.0).sqrt();
    let (l1, l2) = (half + root, (half - root).max(0.0));
    let v1 = if q.abs() > 1e-15 * (p.abs() + r.abs()) {
        let (x, y) = (q, l1 - p);
        let n = x.hypot(y);
        (x / n, y / n)
    } else if p >= r {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    let v2 = (-v1.1, v1.0);
    // the old rx axis, mapped
    let a = (m[0], m[2]);
    let along1 = (a.0 * v1.0 + a.1 * v1.1).abs() >= (a.0 * v2.0 + a.1 * v2.1).abs();
    let (axis, nrx, nry) = if along1 { (v1, l1.sqrt(), l2.sqrt()) } else { (v2, l2.sqrt(), l1.sqrt()) };
    // (pointing the way the old axis went)
    let axis = if axis.0 * a.0 + axis.1 * a.1 < 0.0 { (-axis.0, -axis.1) } else { axis };
    let na = axis.1.atan2(axis.0).to_degrees();
    (nrx.is_finite() && nry.is_finite() && na.is_finite() && nrx > 0.0 && nry > 0.0).then_some((nrx, nry, na))
}

/// Follow `turn` (the change of the user orientation, e.g. a quarter turn clockwise): mask
/// coordinates are on the oriented photo, so they turn with it. Radial angles turn by the same
/// quarter turns; sizes (long-edge units) don't change.
pub fn turn(m: &mut Mask, turn: Orientation) {
    if turn == Orientation::Normal {
        return;
    }
    let fwd = |p: Point| {
        let (x, y) = turn.map(p.x, p.y, 1.0, 1.0);
        Point::new(x, y)
    };
    let inv = turn.inverse();
    let back = |p: Point| {
        let (x, y) = inv.map(p.x, p.y, 1.0, 1.0);
        Point::new(x, y)
    };
    let (flip, quarters) = turn.to_parts();
    // a radial's angle is clockwise on screen: a quarter turn clockwise adds 90°; a mirror negates it first
    let delta = 90.0 * quarters as f64;
    if flip {
        for c in &mut m.components {
            if let MaskShape::Radial { angle, .. } = &mut c.shape {
                *angle = -*angle;
            }
        }
    }
    remap(m, &fwd, &back, delta, true);
}

/// Move every place `m` stores: `fwd` maps an old point to the new frame, `back` a new point to
/// the old one (for resampling segmentations); radial angles change by `delta` degrees. With
/// `gradients` false, linear and radial gradients are left to the caller.
fn remap(m: &mut Mask, fwd: &dyn Fn(Point) -> Point, back: &dyn Fn(Point) -> Point, delta: f64, gradients: bool) {
    for c in &mut m.components {
        match &mut c.shape {
            MaskShape::Linear { start, end } if gradients => {
                *start = fwd(*start);
                *end = fwd(*end);
            }
            MaskShape::Radial { center, angle, .. } if gradients => {
                *center = fwd(*center);
                if delta != 0.0 {
                    // (kept within −180..180°)
                    *angle = (*angle + delta + 180.0).rem_euclid(360.0) - 180.0;
                }
            }
            MaskShape::Linear { .. } | MaskShape::Radial { .. } => {}
            MaskShape::Brush { strokes } => {
                for s in strokes {
                    s.points.iter_mut().for_each(|p| *p = fwd(*p));
                }
            }
            MaskShape::Object { hint, exclude, seg, detail, .. } => {
                hint.iter_mut().chain(exclude.iter_mut()).for_each(|p| *p = fwd(*p));
                remap_segs(seg, detail, fwd, back);
            }
            MaskShape::Prompt { seg, detail, .. } => remap_segs(seg, detail, fwd, back),
            // computed from the photo at each render (its mattes follow the warp), or colours,
            // tones and depths, which have no place (a colour range's samples are colours)
            MaskShape::Subject
            | MaskShape::Sky
            | MaskShape::Background
            | MaskShape::People { .. }
            | MaskShape::Landscape { .. }
            | MaskShape::ColorRange { .. }
            | MaskShape::LuminanceRange { .. }
            | MaskShape::DepthRange { .. } => {}
        }
    }
}

/// A stored selection and its zoomed-in passes, resampled onto the new frame. Where a pass's new
/// box reaches past what it covered, it takes the whole-photo selection there (what the render
/// showed), not a copy of its own edge.
fn remap_segs(seg: &mut Option<SegMask>, detail: &mut [SegMask], fwd: &dyn Fn(Point) -> Point, back: &dyn Fn(Point) -> Point) {
    let coarse = seg.as_ref().and_then(SegGrid::new);
    for d in detail.iter_mut() {
        if let Some(m) = moved(d, fwd, back, coarse.as_ref()) {
            *d = m;
        }
    }
    if let Some(s) = seg.as_ref()
        && let Some(m) = moved(s, fwd, back, None)
    {
        *seg = Some(m);
    }
}

/// Points along each edge of a box [`map_rect`] maps: a lens warp bends an edge between its ends,
/// most where it crosses the centre lines.
const RECT_EDGE_SAMPLES: usize = 16;

/// The box around `r` mapped by `f` (points along its edges), inside 0..1.
fn map_rect(r: [f64; 4], f: &dyn Fn(Point) -> Point) -> [f64; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for i in 0..=RECT_EDGE_SAMPLES {
        let t = i as f64 / RECT_EDGE_SAMPLES as f64;
        let (x, y) = (r[0] + t * (r[2] - r[0]), r[1] + t * (r[3] - r[1]));
        for q in [(x, r[1]), (x, r[3]), (r[0], y), (r[2], y)] {
            let p = f(Point::new(q.0, q.1));
            if p.x.is_finite() && p.y.is_finite() {
                (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
            }
        }
    }
    if x0 > x1 || y0 > y1 {
        return r;
    }
    [x0.clamp(0.0, 1.0), y0.clamp(0.0, 1.0), x1.clamp(0.0, 1.0), y1.clamp(0.0, 1.0)]
}

/// Segmentation `s` resampled onto the new frame: a grid of the same side over the mapped box
/// (the whole image for a whole-image one), each cell taking the old logit at its centre mapped
/// back. A cell whose centre maps back outside what `s` covered takes `outside`'s logit there
/// (for a zoomed-in pass: the whole-photo selection), else selects nothing. `None` when the data
/// is damaged (it is kept as it is).
fn moved(s: &SegMask, fwd: &dyn Fn(Point) -> Point, back: &dyn Fn(Point) -> Point, outside: Option<&SegGrid>) -> Option<SegMask> {
    let old = SegGrid::new(s)?;
    let side = s.side as usize;
    let rect = match s.rect {
        None => None,
        Some(r) => Some(map_rect(r, fwd)).filter(|b| b[2] > b[0] && b[3] > b[1]),
    };
    let b = rect.unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let logits: Vec<f32> = (0..side * side)
        .map(|i| {
            let (x, y) = ((i % side) as f64 + 0.5, (i / side) as f64 + 0.5);
            let n = Point::new(b[0] + x / side as f64 * (b[2] - b[0]), b[1] + y / side as f64 * (b[3] - b[1]));
            let o = back(n);
            if old.contains(o) { old.logit(o) } else { outside.map_or(-16.0, |g| g.logit(o)) }
        })
        .collect();
    Some(match rect {
        Some(r) => SegMask::from_logits_in(side, &logits, r),
        None => SegMask::from_logits(side, &logits),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_develop_model::{BrushStroke, DevelopSettings, MaskComponent, MaskOp};

    fn comp(shape: MaskShape) -> MaskComponent {
        MaskComponent { name: None, op: MaskOp::Add, invert: false, shape }
    }

    fn mask(shape: MaskShape) -> Mask {
        Mask { components: vec![comp(shape)], ..Default::default() }
    }

    fn seg_at(p: Point) -> SegMask {
        const SIDE: usize = 32;
        let l: Vec<f32> = (0..SIDE * SIDE)
            .map(|i| {
                let (x, y) = (((i % SIDE) as f64 + 0.5) / SIDE as f64, ((i / SIDE) as f64 + 0.5) / SIDE as f64);
                if (x - p.x).hypot(y - p.y) < 0.08 { 8.0 } else { -8.0 }
            })
            .collect();
        SegMask::from_logits(SIDE, &l)
    }

    fn object_at(p: Point) -> MaskShape {
        MaskShape::Object { hint: vec![p], exclude: Vec::new(), seg: Some(seg_at(p)), detail: Vec::new(), edge: 0.0 }
    }

    /// The centre of what `s` selects (sampled on a fine grid).
    fn centre_of(s: &SegMask) -> Point {
        let g = SegGrid::new(s).unwrap();
        let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
        for i in 0..200 * 200 {
            let p = Point::new(((i % 200) as f64 + 0.5) / 200.0, ((i / 200) as f64 + 0.5) / 200.0);
            if g.contains(p) && g.logit(p) > 0.0 {
                (sx, sy, n) = (sx + p.x, sy + p.y, n + 1.0);
            }
        }
        assert!(n > 0.0, "nothing selected");
        Point::new(sx / n, sy / n)
    }

    fn seg_of(m: &Mask, i: usize) -> &SegMask {
        match &m.components[i].shape {
            MaskShape::Object { seg: Some(s), .. } => s,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_quarter_turn_turns_every_place_a_mask_keeps() {
        let p = Point::new(0.2, 0.3);
        let mut m = mask(MaskShape::Radial { center: p, rx: 0.1, ry: 0.05, angle: 10.0, feather: 50.0, invert: false });
        m.components.push(comp(MaskShape::Brush { strokes: vec![BrushStroke { points: vec![p], ..Default::default() }] }));
        m.components.push(comp(object_at(p)));
        turn(&mut m, Orientation::Rotate90);
        // a quarter turn clockwise: (x, y) → (1 − y, x)
        let want = Point::new(0.7, 0.2);
        let MaskShape::Radial { center, angle, .. } = &m.components[0].shape else { panic!() };
        assert!((center.x - want.x).abs() < 1e-9 && (center.y - want.y).abs() < 1e-9, "{center:?}");
        assert_eq!(*angle, 100.0);
        let MaskShape::Brush { strokes } = &m.components[1].shape else { panic!() };
        assert!((strokes[0].points[0].x - want.x).abs() < 1e-9);
        let MaskShape::Object { hint, .. } = &m.components[2].shape else { panic!() };
        assert!((hint[0].x - want.x).abs() < 1e-9 && (hint[0].y - want.y).abs() < 1e-9, "{hint:?}");
        let c = centre_of(seg_of(&m, 2));
        assert!((c.x - want.x).abs() < 0.04 && (c.y - want.y).abs() < 0.04, "{c:?}");
        // and back
        turn(&mut m, Orientation::Rotate270);
        let MaskShape::Radial { center, angle, .. } = &m.components[0].shape else { panic!() };
        assert!((center.x - p.x).abs() < 1e-9 && (center.y - p.y).abs() < 1e-9 && *angle == 10.0);
    }

    #[test]
    fn attaching_an_old_mask_keeps_where_it_renders() {
        let mut s = DevelopSettings::default();
        s.geometry.vertical = 30.0;
        s.geometry.rotate = 5.0;
        let frame = Frame::new(300, 200, &s, false);
        let p = Point::new(0.3, 0.4);
        let mut m = mask(MaskShape::Linear { start: p, end: Point::new(0.6, 0.4) });
        m.components.push(comp(object_at(p)));
        m.on_photo = false;
        attach(&mut m, &frame);
        assert!(m.on_photo);
        let on = frame.for_mask(true);
        let MaskShape::Linear { start, .. } = &m.components[0].shape else { panic!() };
        let back = on.mask_to_norm(*start);
        assert!((back.x - p.x).abs() < 1e-6 && (back.y - p.y).abs() < 1e-6, "{back:?}");
        assert!((start.x - p.x).hypot(start.y - p.y) > 0.005, "the warp moved it on the photo: {start:?}");
        let c = on.mask_to_norm(centre_of(seg_of(&m, 1)));
        assert!((c.x - p.x).abs() < 0.04 && (c.y - p.y).abs() < 0.04, "{c:?}");
        // attaching again changes nothing
        let before = m.clone();
        attach(&mut m, &frame);
        assert_eq!(m, before);
    }

    #[test]
    fn damaged_or_hostile_masks_move_without_a_panic() {
        let mut m =
            mask(MaskShape::Radial { center: Point::new(f64::NAN, f64::INFINITY), rx: 0.1, ry: 0.1, angle: f64::NAN, feather: 0.0, invert: false });
        m.components.push(comp(MaskShape::Object {
            hint: vec![Point::new(1e300, -1e300)],
            exclude: Vec::new(),
            seg: Some(SegMask { side: 1024, data: "not base64".into(), rect: Some([f64::NAN, 0.0, 2.0, -1.0]), grid: None }),
            detail: vec![SegMask { side: 0, data: String::new(), rect: None, grid: None }],
            edge: 0.0,
        }));
        m.on_photo = false;
        let mut s = DevelopSettings::default();
        s.geometry.vertical = 80.0;
        s.optics.distortion = 100.0;
        attach(&mut m, &Frame::new(300, 200, &s, false));
        for o in [Orientation::Rotate90, Orientation::FlipH, Orientation::Rotate180] {
            turn(&mut m, o);
        }
        assert!(m.on_photo);
    }

    /// A Transform and lens warp strong enough to bend shapes across a gradient.
    fn warped() -> DevelopSettings {
        let mut s = DevelopSettings::default();
        s.geometry.vertical = 40.0;
        s.geometry.horizontal = -25.0;
        s.geometry.rotate = 8.0;
        s.optics.distortion = 60.0;
        s
    }

    /// The mean and largest difference between mask `a` and `b` rendered under `s`.
    fn alpha_diff(a: &Mask, b: &Mask, s: &DevelopSettings) -> (f32, f32) {
        let frame = Frame::new(300, 200, s, true);
        let (w, h) = frame.fit(300, 200);
        let img = crate::raster::Rgb32f::new(w, h);
        let log_l = crate::raster::Plane::new(w, h);
        let pa = crate::masks::evaluate_one(a, &frame, w, h, &img, &log_l, 0.0, None);
        let pb = crate::masks::evaluate_one(b, &frame, w, h, &img, &log_l, 0.0, None);
        let d: Vec<f32> = pa.data.iter().zip(&pb.data).map(|(x, y)| (x - y).abs()).collect();
        (d.iter().sum::<f32>() / d.len().max(1) as f32, d.iter().copied().fold(0.0, f32::max))
    }

    /// An old gradient keeps its region when it goes onto the photo: its shape goes through the
    /// warp's local linear part, not only its centre or ends (review of storytold/lightcraft#3550).
    #[test]
    fn attaching_an_old_gradient_keeps_its_region() {
        let s = warped();
        let frame = Frame::new(300, 200, &s, false);
        for shape in [
            MaskShape::Radial { center: Point::new(0.3, 0.35), rx: 0.18, ry: 0.07, angle: 30.0, feather: 30.0, invert: false },
            MaskShape::Linear { start: Point::new(0.2, 0.25), end: Point::new(0.45, 0.6) },
        ] {
            let mut old = mask(shape.clone());
            old.on_photo = false;
            let mut on = old.clone();
            attach(&mut on, &frame);
            // what moving the points alone would give
            let mut naive = old.clone();
            naive.on_photo = true;
            remap(&mut naive, &|p| frame.for_mask(true).norm_to_mask(p), &|p| frame.for_mask(true).mask_to_norm(p), 0.0, true);
            let (mean, max) = alpha_diff(&old, &on, &s);
            let (naive_mean, naive_max) = alpha_diff(&old, &naive, &s);
            // (the warp isn't linear across the whole gradient: a little is left at its far edge)
            assert!(mean < 0.01, "{shape:?}: mean {mean}, max {max}");
            assert!(naive_mean > 1.5 * mean && naive_max > 2.0 * max, "{shape:?}: points alone {naive_mean} / {naive_max} vs {mean} / {max}");
        }
    }

    /// Where a zoomed-in pass's new box reaches past what it covered (a turned box's corners), it
    /// takes the whole-photo selection: neither a copy of its own edge (a negative whole-photo
    /// selection) nor a fixed "unselected" (a positive one) (review of storytold/lightcraft#3550).
    #[test]
    fn a_moved_detail_pass_takes_the_whole_selection_past_its_own_area() {
        const SIDE: usize = 16;
        let mut s = DevelopSettings::default();
        s.geometry.rotate = 20.0;
        let frame = Frame::new(300, 200, &s, false);
        for whole in [8.0f32, -8.0] {
            let detail = SegMask::from_logits_in(SIDE, &[8.0; SIDE * SIDE], [0.4, 0.4, 0.6, 0.6]);
            let coarse = SegMask::from_logits(SIDE, &[whole; SIDE * SIDE]);
            let mut m =
                mask(MaskShape::Object { hint: vec![Point::new(0.5, 0.5)], exclude: Vec::new(), seg: Some(coarse), detail: vec![detail], edge: 0.0 });
            m.on_photo = false;
            attach(&mut m, &frame);
            let MaskShape::Object { detail, .. } = &m.components[0].shape else { panic!() };
            let g = SegGrid::new(&detail[0]).unwrap();
            let b = detail[0].bounds().unwrap();
            // a corner of the new box is outside the turned patch: the whole-photo selection's sign
            let inset = |t: f64| Point::new(b[0] + t * (b[2] - b[0]), b[1] + t * (b[3] - b[1]));
            let corner = g.logit(inset(0.03));
            assert!((corner > 0.0) == (whole > 0.0), "whole {whole}: corner {corner}");
            assert!(g.logit(inset(0.5)) > 0.0, "whole {whole}: the patch itself stays selected");
        }
    }

    /// The box around a mapped rectangle holds the whole bent edge, also off the centre lines
    /// (review of storytold/lightcraft#3550).
    #[test]
    fn a_mapped_box_holds_its_bent_edges() {
        let mut s = DevelopSettings::default();
        s.optics.distortion = -80.0;
        let f = Frame::new(300, 200, &s, false).for_mask(true).into_owned();
        let fwd = |p: Point| f.norm_to_mask(p);
        let r = [0.2, 0.1, 0.9, 0.4];
        let b = map_rect(r, &fwd);
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for i in 0..=2000 {
            let t = i as f64 / 2000.0;
            let (x, y) = (r[0] + t * (r[2] - r[0]), r[1] + t * (r[3] - r[1]));
            for q in [(x, r[1]), (x, r[3]), (r[0], y), (r[2], y)] {
                let p = fwd(Point::new(q.0, q.1));
                (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
            }
        }
        let want = [x0.clamp(0.0, 1.0), y0.clamp(0.0, 1.0), x1.clamp(0.0, 1.0), y1.clamp(0.0, 1.0)];
        for k in 0..4 {
            assert!((b[k] - want[k]).abs() < 5e-4, "{b:?} vs {want:?}");
        }
    }
}
