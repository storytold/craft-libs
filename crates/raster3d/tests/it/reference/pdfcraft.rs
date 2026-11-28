//! PdfCraft's camera fit at 4f23ee1c1 (`crates/prc-render/src/render.rs` and `src/vec3.rs`), for
//! the differential tests. Changes: the parts that read PRC meshes (`Bounds::of`, `vertices`,
//! `invisible`, `Camera::fit_meshes`) are left out and `fit_points` frames a point list as
//! `fit_meshes` frames a mesh's vertices; `RenderError::Camera` is a `&'static str`; `at(v, i)`
//! is `v[i]`; the unused vector helpers are left out; comments removed.

pub type RenderError = &'static str;

const FIT_MARGIN: f64 = 1.1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    Perspective { fov_y: f64 },
    Orthographic { height: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub up: [f64; 3],
    pub projection: Projection,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    pub fn of(points: &[[f64; 3]]) -> Option<Bounds> {
        let mut out: Option<Bounds> = None;
        for &p in points {
            let b = out.get_or_insert(Bounds { min: p, max: p });
            for ((lo, hi), c) in b.min.iter_mut().zip(b.max.iter_mut()).zip(p) {
                *lo = lo.min(c);
                *hi = hi.max(c);
            }
        }
        out
    }

    pub fn centre(&self) -> [f64; 3] {
        [0, 1, 2].map(|i| self.min[i] / 2.0 + self.max[i] / 2.0)
    }

    pub fn radius(&self) -> f64 {
        length(sub(self.max, self.min)) / 2.0
    }
}

impl Camera {
    pub fn fit(bounds: &Bounds, direction: [f64; 3], up: [f64; 3], perspective: bool, aspect: f64) -> Result<Camera, RenderError> {
        let corners = (0..8).map(|k| [0, 1, 2].map(|i| if k >> i & 1 == 0 { bounds.min[i] } else { bounds.max[i] }));
        Camera::frame(bounds, corners, direction, up, (perspective, aspect))
    }

    pub fn fit_points(points: &[[f64; 3]], direction: [f64; 3], up: [f64; 3], perspective: bool, aspect: f64) -> Result<Camera, RenderError> {
        let bounds = Bounds::of(points).ok_or("the model has no vertex")?;
        Camera::frame(&bounds, points.iter().copied(), direction, up, (perspective, aspect))
    }

    fn frame(
        bounds: &Bounds,
        points: impl Iterator<Item = [f64; 3]> + Clone,
        direction: [f64; 3],
        up: [f64; 3],
        (perspective, aspect): (bool, f64),
    ) -> Result<Camera, RenderError> {
        if !(aspect.is_finite() && aspect > 0.0) {
            return Err("the aspect ratio is not positive");
        }
        let dir = normalize(direction).ok_or("the view direction is zero")?;
        let centre = bounds.centre();
        let radius = match bounds.radius() {
            r if r.is_finite() && r > 0.0 => r,
            _ => 1.0,
        };
        let [right, upward, forward] = axes(dir, up)?;
        let view = |p: [f64; 3]| {
            let r = sub(p, centre);
            [dot(r, right), dot(r, upward), dot(r, forward)]
        };
        let ((x0, x1), (y0, y1)) = points
            .clone()
            .map(view)
            .fold(((f64::INFINITY, f64::NEG_INFINITY), (f64::INFINITY, f64::NEG_INFINITY)), |((a0, a1), (b0, b1)), [x, y, _]| {
                ((a0.min(x), a1.max(x)), (b0.min(y), b1.max(y)))
            });
        if !(x0 <= x1 && y0 <= y1) {
            return Err("the model has no vertex");
        }
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let target = [0, 1, 2].map(|i| centre[i] + mx * right[i] + my * upward[i]);
        let floor = radius * 1e-3;
        let (projection, distance) = if perspective {
            let fov_y: f64 = 30.0;
            let ty = (fov_y.to_radians() / 2.0).tan() / FIT_MARGIN;
            let tx = ty * aspect;
            let d = points.map(view).fold(floor, |d, [x, y, f]| {
                let need = ((x - mx).abs() / tx).max((y - my).abs() / ty);
                d.max(need - f).max(floor - f)
            });
            (Projection::Perspective { fov_y }, d)
        } else {
            let height = ((y1 - y0).max((x1 - x0) / aspect) * FIT_MARGIN).max(floor);
            (Projection::Orthographic { height }, 2.0 * radius)
        };
        let camera = Camera { eye: sub(target, scale(dir, distance)), target, up, projection };
        camera.basis()?;
        Ok(camera)
    }

    pub fn basis(&self) -> Result<[[f64; 3]; 3], RenderError> {
        let forward = normalize(sub(self.target, self.eye)).ok_or("the eye and the target coincide")?;
        axes(forward, self.up)
    }
}

fn axes(forward: [f64; 3], up: [f64; 3]) -> Result<[[f64; 3]; 3], RenderError> {
    let right = normalize(cross(forward, up)).ok_or("the up direction is zero or parallel to the view direction")?;
    Ok([right, cross(right, forward), forward])
}

fn sub([ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3]) -> [f64; 3] {
    [ax - bx, ay - by, az - bz]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|c| c * s)
}

fn dot([ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3]) -> f64 {
    ax * bx + ay * by + az * bz
}

fn cross([ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3]) -> [f64; 3] {
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn length(a: [f64; 3]) -> f64 {
    let big = largest(a);
    if !(big.is_finite() && big > 0.0) {
        return dot(a, a).sqrt();
    }
    let a = a.map(|c| c / big);
    big * dot(a, a).sqrt()
}

fn largest(a: [f64; 3]) -> f64 {
    a.iter().fold(0.0_f64, |m, c| m.max(c.abs()))
}

fn normalize(a: [f64; 3]) -> Option<[f64; 3]> {
    let big = largest(a);
    if !(big.is_finite() && big > 0.0) {
        return None;
    }
    let a = a.map(|c| c / big);
    let l = dot(a, a).sqrt();
    (l.is_finite() && l > 0.0).then(|| scale(a, 1.0 / l))
}
