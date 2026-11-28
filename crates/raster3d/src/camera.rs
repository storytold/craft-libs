//! Cameras: fitting one to a box or to a scene's drawn vertices, and the clip and view
//! transforms a camera gives a [`Scene`].
//!
//! Camera space has x to the right of the image, y up the image and z along the view direction.
//! A fitted camera frames the projected points rather than a sphere around them: its target is
//! the middle of their projection, and the tighter image axis keeps 5% clear on each side.

use crate::RenderError;
use crate::scene::Scene;

/// The vertical field of view of a fitted perspective camera, in degrees.
pub const FIT_FOV_Y: f64 = 30.0;

/// The fitted extent over the points' projected extent: 5% clear on each side.
const FIT_MARGIN: f64 = 1.1;

/// The perspective near plane, as a fraction of the eye-to-target distance.
const NEAR_FRACTION: f64 = 1e-4;

/// Why a camera could not be fitted or could not form a scene's transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CameraError {
    /// A bound is not finite, or a minimum is above its maximum.
    #[error("the bounds are not finite or a minimum is above its maximum")]
    Bounds,
    /// The aspect ratio is not positive and finite.
    #[error("the aspect ratio is not positive and finite")]
    Aspect,
    /// The view direction is zero or not finite.
    #[error("the view direction is zero or not finite")]
    Direction,
    /// The up direction is zero, not finite or parallel to the view direction.
    #[error("the up direction is zero, not finite or parallel to the view direction")]
    Up,
    /// The field of view is not inside 0 to 180 degrees or leaves no finite focal length, or
    /// the orthographic height is not positive and finite.
    #[error("the field of view or the orthographic height is out of range")]
    Projection,
    /// The scene has no drawn vertex to frame.
    #[error("the scene has no drawn vertex")]
    Empty,
    /// The camera's eye, target or distances are not finite, the eye and the target coincide, its
    /// transforms or near plane do not fit `f32`, an image axis collapses in `f32`, or a fitted
    /// camera does not hold its points.
    #[error("the camera is outside the range the transforms can hold")]
    OutOfRange,
}

impl From<CameraError> for RenderError {
    fn from(e: CameraError) -> Self {
        RenderError::BadCamera(e)
    }
}

/// How a camera projects the scene onto the image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    /// A pinhole camera with this vertical field of view, in degrees (exclusive range 0 to 180).
    Perspective {
        /// Vertical field of view, degrees.
        fov_y: f64,
    },
    /// A parallel projection showing this much of the scene vertically, in world units.
    Orthographic {
        /// Visible height, world units.
        height: f64,
    },
}

/// Where a camera is and where it looks, in world coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// The camera position.
    pub eye: [f64; 3],
    /// The point at the image center.
    pub target: [f64; 3],
    /// The direction that appears upward; it must not be parallel to the view direction.
    pub up: [f64; 3],
    /// The projection.
    pub projection: Projection,
}

/// An axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// The smallest x, y and z.
    pub min: [f64; 3],
    /// The largest x, y and z.
    pub max: [f64; 3],
}

impl Bounds {
    /// The box around every vertex of the scene's drawn triangles, or `None` when there is none.
    /// A drawn triangle has three vertices that exist with finite positions, and its first
    /// vertex names a material that exists and is visible, as in [`crate::try_render`].
    #[must_use]
    pub fn of_scene(s: &Scene) -> Option<Bounds> {
        let mut out: Option<Bounds> = None;
        for p in drawn(s) {
            let b = out.get_or_insert(Bounds { min: p, max: p });
            for ((lo, hi), c) in b.min.iter_mut().zip(b.max.iter_mut()).zip(p) {
                *lo = lo.min(c);
                *hi = hi.max(c);
            }
        }
        out
    }

    /// The box's center, finite for any finite box.
    #[must_use]
    pub fn center(&self) -> [f64; 3] {
        [0, 1, 2].map(|i| self.min[i] / 2.0 + self.max[i] / 2.0)
    }

    /// Half the box's diagonal, the radius of a sphere holding it: finite for any finite box.
    #[must_use]
    pub fn radius(&self) -> f64 {
        length([0, 1, 2].map(|i| self.max[i] / 2.0 - self.min[i] / 2.0))
    }

    fn check(&self) -> Result<(), CameraError> {
        let ok = (0..3).all(|i| self.min[i].is_finite() && self.max[i].is_finite() && self.min[i] <= self.max[i]);
        if ok { Ok(()) } else { Err(CameraError::Bounds) }
    }
}

/// The positions of the vertices of the scene's drawn triangles (see [`Bounds::of_scene`]).
fn drawn(s: &Scene) -> impl Iterator<Item = [f64; 3]> + Clone + '_ {
    s.indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|t| {
            let v = t.map(|i| s.vertices.get(i as usize));
            let [Some(a), Some(b), Some(c)] = v else {
                return None;
            };
            let visible = s.materials.get(a.material as usize).is_some_and(|m| m.visible);
            let corners = [a.pos, b.pos, c.pos];
            (visible && corners.iter().flatten().all(|x| x.is_finite())).then_some(corners)
        })
        .flatten()
        .map(|p| p.map(f64::from))
}

impl Camera {
    /// A camera looking along `direction` that frames the eight corners of `bounds` in an image
    /// of the given aspect ratio (width / height), with a [`FIT_FOV_Y`] perspective or an
    /// orthographic projection, the tighter image axis keeping 5% clear on each side. For bounds of
    /// zero size the calculations fall back to a radius of 1: the smallest distance and image
    /// height are 0.001 and an orthographic eye stands 2 from the target, while the framed corners
    /// still coincide.
    ///
    /// The points are checked against the basis of the returned camera, whose eye is rounded to
    /// `f64`: an orthographic camera that the rounding leaves short of a point is widened around
    /// the same target, keeping the 5% margin.
    ///
    /// # Errors
    /// [`RenderError::BadCamera`] when `bounds` are not finite or a minimum is above its maximum,
    /// `aspect` is not positive and finite, `direction` is zero or not finite, `up` is zero, not
    /// finite or parallel to `direction`, the camera's eye or extent is outside `f64`'s range or
    /// would coincide with its target, a perspective camera leaves a point outside its view or
    /// short of its near plane in `f64`, or, where the point and the camera's clip rows fit `f32`,
    /// the `f32` transform [`Camera::apply`] writes for an image of this aspect ratio would clip a
    /// point.
    pub fn fit(bounds: &Bounds, direction: [f64; 3], up: [f64; 3], perspective: bool, aspect: f64) -> Result<Camera, RenderError> {
        bounds.check()?;
        let corners = (0..8).map(|k| [0, 1, 2].map(|i| if k >> i & 1 == 0 { bounds.min[i] } else { bounds.max[i] }));
        Camera::frame(bounds, corners, direction, up, (perspective, aspect)).map_err(RenderError::from)
    }

    /// As [`Camera::fit`], framing every vertex of the scene's drawn triangles (see
    /// [`Bounds::of_scene`]) rather than their box, so an oblique view is not padded by empty box
    /// corners.
    ///
    /// # Errors
    /// As [`Camera::fit`], and [`CameraError::Empty`] when the scene has no drawn vertex.
    pub fn fit_scene(s: &Scene, direction: [f64; 3], up: [f64; 3], perspective: bool, aspect: f64) -> Result<Camera, RenderError> {
        let bounds = Bounds::of_scene(s).ok_or(CameraError::Empty)?;
        Camera::frame(&bounds, drawn(s), direction, up, (perspective, aspect)).map_err(RenderError::from)
    }

    /// Frames `points` (which `bounds` holds) from `direction`, aimed at the middle of their
    /// projection.
    fn frame(
        bounds: &Bounds,
        points: impl Iterator<Item = [f64; 3]> + Clone,
        direction: [f64; 3],
        up: [f64; 3],
        (perspective, aspect): (bool, f64),
    ) -> Result<Camera, CameraError> {
        if !(aspect.is_finite() && aspect > 0.0) {
            return Err(CameraError::Aspect);
        }
        let dir = normalize(direction).ok_or(CameraError::Direction)?;
        let center = bounds.center();
        let radius = match bounds.radius() {
            r if r.is_finite() && r > 0.0 => r,
            _ => 1.0,
        };
        let [right, upward, forward] = axes(dir, up)?;
        let view = |p: [f64; 3]| {
            let r = sub(p, center);
            [dot(r, right), dot(r, upward), dot(r, forward)]
        };
        let ((x0, x1), (y0, y1)) = points
            .clone()
            .map(view)
            .fold(((f64::INFINITY, f64::NEG_INFINITY), (f64::INFINITY, f64::NEG_INFINITY)), |((a0, a1), (b0, b1)), [x, y, _]| {
                ((a0.min(x), a1.max(x)), (b0.min(y), b1.max(y)))
            });
        if !(x0 <= x1 && y0 <= y1) {
            return Err(CameraError::Empty);
        }
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let target = [0, 1, 2].map(|i| center[i] + mx * right[i] + my * upward[i]);
        let floor = radius * 1e-3;
        let (projection, distance) = if perspective {
            let ty = (FIT_FOV_Y.to_radians() / 2.0).tan() / FIT_MARGIN;
            let tx = ty * aspect;
            // A point at lateral (x, y) from the target and depth f past it is in view when
            // |x| <= (d + f)·tx and |y| <= (d + f)·ty.
            let d = points.clone().map(view).fold(floor, |d, [x, y, f]| {
                let need = ((x - mx).abs() / tx).max((y - my).abs() / ty);
                d.max(need - f).max(floor - f)
            });
            (Projection::Perspective { fov_y: FIT_FOV_Y }, d)
        } else {
            let height = ((y1 - y0).max((x1 - x0) / aspect) * FIT_MARGIN).max(floor);
            if !(height.is_finite() && height > 0.0) {
                return Err(CameraError::OutOfRange);
            }
            (Projection::Orthographic { height }, 2.0 * radius)
        };
        let mut camera = Camera { eye: sub(target, scale(dir, distance)), target, up, projection };
        if !camera.eye.iter().chain(&camera.target).all(|c| c.is_finite()) {
            return Err(CameraError::OutOfRange);
        }
        // Rounding the eye to f64 can turn the view, so the points are checked against the
        // returned camera's own basis.
        let basis = camera.basis()?;
        let [right, upward, forward] = basis;
        match camera.projection {
            Projection::Orthographic { height } => {
                let mut need = 0.0_f64;
                for p in points.clone() {
                    let r = sub(p, camera.target);
                    need = need.max(dot(r, upward).abs()).max(dot(r, right).abs() / aspect);
                }
                if !need.is_finite() {
                    return Err(CameraError::OutOfRange);
                }
                // A fit that lost points to rounding is widened around the same target.
                if 2.0 * need > height {
                    let wider = 2.0 * need * FIT_MARGIN;
                    if !wider.is_finite() {
                        return Err(CameraError::OutOfRange);
                    }
                    camera.projection = Projection::Orthographic { height: wider };
                }
            }
            Projection::Perspective { fov_y } => {
                let ty = (fov_y.to_radians() / 2.0).tan();
                let tx = ty * aspect;
                let near = camera.near();
                for p in points.clone() {
                    let r = sub(p, camera.eye);
                    let (x, y, z) = (dot(r, right), dot(r, upward), dot(r, forward));
                    if !(z > near && x.abs() <= z * tx && y.abs() <= z * ty) {
                        return Err(CameraError::OutOfRange);
                    }
                }
            }
        }
        // The f32 clip rows `apply` writes for this aspect ratio, where they fit f32: the
        // rasterizer clips a point outside them, or short of the perspective near plane.
        let ortho = matches!(camera.projection, Projection::Orthographic { .. });
        let [rx, ry] = camera.lateral(basis, aspect)?.map(|r| r.map(|c| c as f32));
        let rw = view_row(forward, camera.eye).map(|c| c as f32);
        let near = camera.near() as f32;
        let fits = rx.iter().chain(&ry).all(|c| c.is_finite()) && (ortho || (rw.iter().all(|c| c.is_finite()) && near.is_normal()));
        if fits {
            let row = |m: [f32; 4], q: [f32; 3]| m[0] * q[0] + m[1] * q[1] + m[2] * q[2] + m[3];
            for q in points.map(|p| p.map(|c| c as f32)).filter(|q| q.iter().all(|c| c.is_finite())) {
                let (x, y) = (row(rx, q), row(ry, q));
                let w = if ortho { 1.0 } else { row(rw, q) };
                if !(w.is_finite() && (ortho || w > near) && x.abs() <= w && y.abs() <= w) {
                    return Err(CameraError::OutOfRange);
                }
            }
        }
        Ok(camera)
    }

    /// The clip rows of the image's x and y axes for an image of aspect ratio `aspect`, from the
    /// camera's `basis`.
    ///
    /// # Errors
    /// [`CameraError::Projection`] for a field of view outside 0 to 180 degrees or too small for
    /// a finite focal length, or an orthographic height that is not positive and finite.
    fn lateral(&self, basis: [[f64; 3]; 3], aspect: f64) -> Result<[[f64; 4]; 2], CameraError> {
        let (kx, ky) = match self.projection {
            Projection::Perspective { fov_y } => {
                if !(fov_y > 0.0 && fov_y < 180.0) {
                    return Err(CameraError::Projection);
                }
                let focal = 1.0 / (fov_y.to_radians() / 2.0).tan();
                if !focal.is_finite() {
                    return Err(CameraError::Projection);
                }
                (focal / aspect, focal)
            }
            Projection::Orthographic { height } => {
                if !(height.is_finite() && height > 0.0) {
                    return Err(CameraError::Projection);
                }
                let half = height / 2.0;
                (1.0 / (half * aspect), 1.0 / half)
            }
        };
        let [vr, vu] = [basis[0], basis[1]].map(|a| view_row(a, self.eye));
        Ok([vr.map(|c| c * kx), vu.map(|c| c * ky)])
    }

    /// The perspective near plane: [`NEAR_FRACTION`] of the eye-to-target distance.
    fn near(&self) -> f64 {
        (length(span(self.eye, self.target)) * NEAR_FRACTION).max(f64::MIN_POSITIVE)
    }

    /// Right, up and forward unit vectors.
    ///
    /// # Errors
    /// [`CameraError::OutOfRange`] when the eye or the target is not finite or they coincide, and
    /// [`CameraError::Up`] when `up` is zero, not finite or parallel to the view direction.
    pub fn basis(&self) -> Result<[[f64; 3]; 3], CameraError> {
        if !self.eye.iter().chain(&self.target).all(|c| c.is_finite()) {
            return Err(CameraError::OutOfRange);
        }
        let forward = normalize(span(self.eye, self.target)).ok_or(CameraError::OutOfRange)?;
        axes(forward, self.up)
    }

    /// Writes the camera into `s`: [`Scene::view`] (rows right, up and forward, so the third row
    /// gives the depth along the view direction from the eye), [`Scene::clip`], [`Scene::eye`],
    /// [`Scene::cam_fwd`] and [`Scene::ortho`], for an image of the scene's raster aspect ratio.
    ///
    /// A perspective camera clips at a near plane 1/10,000 of the way from the eye to the target
    /// and has no far plane. An orthographic camera's depth range holds every vertex of the
    /// scene's drawn triangles (see [`Bounds::of_scene`]), with room on each side for rounding,
    /// so the camera must be applied again after the drawn set changes: a moved vertex, a changed
    /// index or a material made visible.
    ///
    /// The scene's index order is left as it is. The rasterizer blends the transparent range in
    /// that order, so a caller that sorts transparent triangles by depth sorts them again for the
    /// new view.
    ///
    /// # Errors
    /// [`RenderError::BadSize`] for a raster side of zero, and [`RenderError::BadCamera`] for an
    /// eye or target that is not finite or coincides, an up direction that is zero, not finite or
    /// parallel to the view, a field of view outside 0 to 180 degrees or too small for a finite
    /// focal length, an orthographic height that is not positive and finite, transforms or a
    /// near plane that do not fit `f32`, or an image axis whose clip row rounds to zero in `f32`.
    /// On an error the scene is left as it was.
    pub fn apply(&self, s: &mut Scene) -> Result<(), RenderError> {
        if s.width == 0 || s.height == 0 {
            return Err(RenderError::BadSize(s.width, s.height));
        }
        let aspect = f64::from(s.width) / f64::from(s.height);
        let basis = self.basis()?;
        let view: [[f64; 4]; 3] = basis.map(|a| view_row(a, self.eye));
        let [vr, vu, vf] = view;
        let [cx, cy] = self.lateral(basis, aspect)?;
        let (clip, ortho) = match self.projection {
            Projection::Perspective { .. } => {
                let near = self.near();
                // Reversed depth: normalized device z = near / camera z, 1 at the near plane.
                if !((near as f32).is_normal() && near.is_finite()) {
                    return Err(CameraError::OutOfRange.into());
                }
                ([cx, cy, [0.0, 0.0, 0.0, near], vf], false)
            }
            Projection::Orthographic { .. } => {
                let (near, far) = ortho_depth(s, vf)?;
                // Reversed depth: normalized device z runs from 1 at `near` to 0 at `far`.
                let k = 1.0 / (far - near);
                if !(k as f32).is_normal() {
                    return Err(CameraError::OutOfRange.into());
                }
                let mut z = vf.map(|c| c * -k);
                z[3] += far * k;
                ([cx, cy, z, [0.0, 0.0, 0.0, 1.0]], true)
            }
        };
        let narrow = |m: [[f64; 4]; 4]| m.map(|r| r.map(|c| c as f32));
        let clip = narrow(clip);
        let view = narrow([vr, vu, vf, [0.0, 0.0, 0.0, 1.0]]);
        let eye = self.eye.map(|c| c as f32);
        let fwd = basis[2].map(|c| c as f32);
        if !clip.iter().chain(&view).flatten().chain(&eye).chain(&fwd).all(|c| c.is_finite()) {
            return Err(CameraError::OutOfRange.into());
        }
        // A lateral row that rounds to zero in f32 collapses that image axis.
        if clip[..2].iter().any(|r| r[..3].iter().all(|&c| c == 0.0)) {
            return Err(CameraError::OutOfRange.into());
        }
        // In f32, as the rasterizer computes clip w: the target must stay past the near plane, or
        // an eye that rounds onto it clips the scene.
        if !ortho {
            let t = self.target.map(|c| c as f32);
            let w = clip[3][0] * t[0] + clip[3][1] * t[1] + clip[3][2] * t[2] + clip[3][3];
            if !(w.is_finite() && w > clip[2][3]) {
                return Err(CameraError::OutOfRange.into());
            }
        }
        (s.clip, s.view, s.eye, s.cam_fwd, s.ortho) = (clip, view, eye, fwd, ortho);
        Ok(())
    }
}

/// The near and far camera depths of an orthographic camera whose depth row is `vf`: the depth
/// range of the scene's drawn vertices, widened on each side by half its extent and by 2^-16 of
/// the largest term of the depth sum, which covers the rounding of the `f32` clip transform.
/// A scene with no drawn vertex gets the range 0 to 1.
fn ortho_depth(s: &Scene, vf: [f64; 4]) -> Result<(f64, f64), CameraError> {
    let mut range: Option<(f64, f64, f64)> = None;
    for p in drawn(s) {
        let z = vf[0] * p[0] + vf[1] * p[1] + vf[2] * p[2] + vf[3];
        let terms = (vf[0] * p[0]).abs() + (vf[1] * p[1]).abs() + (vf[2] * p[2]).abs() + vf[3].abs();
        let r = range.get_or_insert((z, z, terms));
        *r = (r.0.min(z), r.1.max(z), r.2.max(terms));
    }
    let (lo, hi, terms) = range.unwrap_or((0.0, 1.0, 1.0));
    let pad = (hi / 2.0 - lo / 2.0) + terms * f64::powi(2.0, -16) + f64::from(f32::MIN_POSITIVE);
    let (near, far) = (lo - pad, hi + pad);
    let ok = near.is_finite() && far.is_finite() && (far - near).is_finite() && far > near;
    if ok { Ok((near, far)) } else { Err(CameraError::OutOfRange) }
}

/// The view-transform row of unit axis `a` for a camera at `eye`: the distance along `a` from
/// the eye.
fn view_row(a: [f64; 3], eye: [f64; 3]) -> [f64; 4] {
    [a[0], a[1], a[2], -dot(a, eye)]
}

/// Right, up and `forward` (a unit vector) for a camera whose up direction is `up`.
fn axes(forward: [f64; 3], up: [f64; 3]) -> Result<[[f64; 3]; 3], CameraError> {
    if !up.iter().all(|c| c.is_finite()) {
        return Err(CameraError::Up);
    }
    // An up direction whose cross product with `forward` overflows is normalized first.
    let right = normalize(cross(forward, up)).or_else(|| normalize(cross(forward, normalize(up)?))).ok_or(CameraError::Up)?;
    Ok([right, cross(right, forward), forward])
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The direction from `from` to `to`: `to - from`, halved when the full difference overflows.
fn span(from: [f64; 3], to: [f64; 3]) -> [f64; 3] {
    let d = sub(to, from);
    if d.iter().all(|c| c.is_finite()) { d } else { sub(scale(to, 0.5), scale(from, 0.5)) }
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|c| c * s)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The right-handed cross product `a × b`.
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The largest component of `a` by magnitude; NaN components are passed over.
fn largest(a: [f64; 3]) -> f64 {
    a.iter().fold(0.0_f64, |m, c| m.max(c.abs()))
}

/// The Euclidean length of `a`, taken with `a` divided by its largest component so squaring
/// cannot overflow or underflow; infinite or NaN when a component is.
fn length(a: [f64; 3]) -> f64 {
    let big = largest(a);
    if !(big.is_finite() && big > 0.0) {
        return dot(a, a).sqrt();
    }
    let a = a.map(|c| c / big);
    big * dot(a, a).sqrt()
}

/// `a` scaled to unit length; `None` when its length is zero or not finite. `a` is first divided
/// by its largest component, so squaring cannot overflow or underflow.
fn normalize(a: [f64; 3]) -> Option<[f64; 3]> {
    let big = largest(a);
    if !(big.is_finite() && big > 0.0) {
        return None;
    }
    let a = a.map(|c| c / big);
    let l = dot(a, a).sqrt();
    (l.is_finite() && l > 0.0).then(|| scale(a, 1.0 / l))
}
