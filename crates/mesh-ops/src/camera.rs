//! Orbit camera (texture painting navigation) and picking rays.

use glam::{DVec4, Mat4, Vec2, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct OrbitCamera {
    pub target: Vec3,
    /// Radians around Y.
    pub yaw: f32,
    /// Radians above the horizon, clamped to ±89°.
    pub pitch: f32,
    pub distance: f32,
    /// Vertical field of view, degrees.
    pub fov: f32,
    /// Scene radius (sets the clip planes).
    pub scene_radius: f32,
    /// Orthographic projection instead of perspective.
    #[cfg_attr(feature = "serde", serde(default))]
    pub ortho: bool,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera { target: Vec3::ZERO, yaw: 0.0, pitch: 0.12, distance: 5.0, fov: 40.0, scene_radius: 1.0, ortho: false }
    }
}

/// A ray in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl OrbitCamera {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(cp * sy, sp, cp * cy) * self.distance
    }

    pub fn forward(&self) -> Vec3 {
        (self.target - self.eye()).try_normalize().unwrap_or(Vec3::NEG_Z)
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    /// The vertical field of view in radians, with `fov` clamped to 5° to 150°. Perspective and
    /// orthographic projection, [`OrbitCamera::pan`] and [`OrbitCamera::frame`] all use it.
    fn fov_radians(&self) -> f32 {
        self.fov.clamp(5.0, 150.0).to_radians()
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        let r = self.scene_radius.max(1e-3);
        let near = (self.distance - r * 4.0).max(r * 0.01).max(1e-4);
        let far = self.distance + r * 50.0;
        let aspect = if aspect.is_finite() && aspect > 1e-4 { aspect } else { 1.0 };
        if self.ortho {
            let h = self.distance * (self.fov_radians() * 0.5).tan();
            Mat4::orthographic_rh(-h * aspect, h * aspect, -h, h, near.min(0.01), far)
        } else {
            Mat4::perspective_rh(self.fov_radians(), aspect, near, far)
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    /// Frame a bounding box, keeping the current angles. A box whose center or framing distance
    /// is not finite, or whose framed view-projection at aspect 1 is not finite (an eye that
    /// rounds to the target, or terms past the `f32` range), leaves the camera unchanged.
    pub fn frame(&mut self, lo: Vec3, hi: Vec3) {
        let target = lo * 0.5 + hi * 0.5;
        let r = ((hi - lo).length() * 0.5).max(1e-3);
        let distance = r / (self.fov_radians() * 0.5).sin() * 1.1;
        if !(target.is_finite() && distance.is_finite()) {
            return;
        }
        let framed = OrbitCamera { target, scene_radius: r, distance, ..*self };
        if framed.view_proj(1.0).is_finite() {
            *self = framed;
        }
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx;
        self.pitch = (self.pitch + dy).clamp(-89f32.to_radians(), 89f32.to_radians());
    }

    /// Pan by a screen fraction (dx, dy in viewport heights).
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let f = self.forward();
        let right = f.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(f);
        let scale = self.distance * (self.fov_radians() * 0.5).tan() * 2.0;
        self.target += (-right * dx + up * dy) * scale;
    }

    /// Dolly by a factor exponent (positive zooms in). A distance whose view-projection at aspect 1
    /// is not finite (an eye that rounds to a target far from the origin) leaves the camera
    /// unchanged.
    pub fn zoom(&mut self, amount: f32) {
        let r = self.scene_radius.max(1e-3);
        let distance = (self.distance * (-amount).exp()).clamp(r * 0.05, r * 200.0);
        let zoomed = OrbitCamera { distance, ..*self };
        if zoomed.eye() != zoomed.target && zoomed.view_proj(1.0).is_finite() {
            *self = zoomed;
        }
    }

    /// Ray through a viewport point given in normalized device coordinates (-1..1, y up). It
    /// starts on the near plane: [`OrbitCamera::proj`] has a depth range of 0 to 1. The
    /// view-projection is inverted and the points unprojected in `f64`, where the determinant of
    /// a large scene's matrix and the squared length of the ray do not leave the range.
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> Ray {
        let inv = self.view_proj(aspect).as_dmat4().inverse();
        let (x, y) = (f64::from(ndc.x), f64::from(ndc.y));
        let a = inv * DVec4::new(x, y, 0.0, 1.0);
        let b = inv * DVec4::new(x, y, 1.0, 1.0);
        let (a, b) = (a.truncate() / a.w, b.truncate() / b.w);
        let origin = a.as_vec3();
        let dir = (b - a).try_normalize().map(|d| d.as_vec3()).filter(|d| d.is_finite()).unwrap_or(self.forward());
        Ray { origin, dir }
    }

    /// Project a world point to NDC (x, y in -1..1, y up; z depth). None behind the camera, in
    /// both projections: the point's depth in front of the eye must be over 1e-6.
    pub fn project(&self, p: Vec3, aspect: f32) -> Option<Vec3> {
        let in_view = self.view() * p.extend(1.0);
        // The view looks down -Z.
        if -in_view.z <= 1e-6 {
            return None;
        }
        let c = self.proj(aspect) * in_view;
        (c.w > 1e-6).then(|| c.truncate() / c.w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_ray_points_at_target_and_projection_inverts() {
        let mut c = OrbitCamera::default();
        c.frame(Vec3::splat(-1.0), Vec3::splat(1.0));
        let r = c.ray(Vec2::ZERO, 1.5);
        assert!((r.dir - c.forward()).length() < 1e-4);
        let p = Vec3::new(0.3, -0.2, 0.1);
        let ndc = c.project(p, 1.5).unwrap();
        let r = c.ray(ndc.truncate(), 1.5);
        let to_p = (p - r.origin).normalize();
        assert!(to_p.dot(r.dir) > 0.9999);
    }

    #[test]
    fn pitch_is_clamped_and_zoom_bounded() {
        let mut c = OrbitCamera::default();
        c.orbit(0.0, 10.0);
        assert!(c.pitch <= 89f32.to_radians() + 1e-6);
        for _ in 0..1000 {
            c.zoom(1.0);
        }
        assert!(c.distance > 0.0 && c.distance.is_finite());
        let _ = c.proj(f32::NAN);
    }

    #[test]
    fn orthographic_projection_finds_nothing_behind_the_camera() {
        for ortho in [false, true] {
            let c = OrbitCamera { ortho, ..OrbitCamera::default() };
            assert!(c.project(c.eye() - c.forward(), 1.5).is_none(), "ortho {ortho}");
            assert!(c.project(c.eye() - c.forward() * 1e-3, 1.5).is_none(), "ortho {ortho}");
            assert!(c.project(c.target, 1.5).is_some(), "ortho {ortho}");
        }
    }

    #[test]
    fn field_of_view_is_clamped_the_same_way_in_every_mode() {
        for fov in [0.0, -10.0, 1.0, 179.0, 500.0] {
            let c = OrbitCamera { fov, ortho: true, ..OrbitCamera::default() };
            let clamped = OrbitCamera { fov: fov.clamp(5.0, 150.0), ..c };
            assert!(c.proj(1.5).to_cols_array().iter().all(|v| v.is_finite()), "fov {fov}");
            assert_eq!(c.proj(1.5), clamped.proj(1.5), "fov {fov}");
            let (mut panned, mut expected) = (c, clamped);
            panned.pan(0.25, -0.5);
            expected.pan(0.25, -0.5);
            assert_eq!(panned.target, expected.target, "fov {fov}");
            assert_ne!(panned.target, c.target, "fov {fov}");
        }
    }

    #[test]
    fn frame_leaves_the_camera_unchanged_for_a_box_it_cannot_frame() {
        let mut c = OrbitCamera::default();
        c.frame(Vec3::splat(-1.0), Vec3::splat(1.0));
        let framed = c;
        for (lo, hi) in [
            (Vec3::new(f32::NAN, 0.0, 0.0), Vec3::ONE),
            (Vec3::ZERO, Vec3::new(1.0, f32::INFINITY, 1.0)),
            (Vec3::NEG_INFINITY, Vec3::INFINITY),
            (Vec3::splat(-3e38), Vec3::splat(3e38)),
        ] {
            c.frame(lo, hi);
            assert_eq!(c, framed, "{lo} {hi}");
        }
    }

    #[test]
    fn frame_keeps_the_view_finite() {
        let mut c = OrbitCamera::default();
        c.frame(Vec3::splat(-1e15), Vec3::splat(1e15));
        assert!(c.view_proj(1.0).is_finite());
        assert!((c.scene_radius / (1e15 * 3f32.sqrt()) - 1.0).abs() < 1e-4, "{}", c.scene_radius);
        let framed = c;
        for (lo, hi) in [
            // A finite center and distance, with an eye whose distance from the target squares
            // past the f32 range.
            (Vec3::splat(-5e18), Vec3::splat(5e18)),
            // An eye that rounds to a target this far out.
            (Vec3::splat(1e30), Vec3::splat(1e30)),
            // Corners whose sum overflows f32.
            (Vec3::new(1.7e38, -1.0, -1.0), Vec3::new(1.8e38, 1.0, 1.0)),
        ] {
            c.frame(lo, hi);
            assert_eq!(c, framed, "{lo} {hi}");
            assert!(c.view_proj(1.0).is_finite(), "{lo} {hi}");
        }
    }

    #[test]
    fn zoom_keeps_the_view_finite_near_a_far_target() {
        // At the closest zoom the eye is 0.69 from the target, where coordinates of 1e8 are 8 apart.
        let mut c = OrbitCamera::default();
        c.frame(Vec3::splat(1e8 - 8.0), Vec3::splat(1e8 + 8.0));
        assert!(c.view().is_finite());
        c.zoom(10.0);
        assert!(c.view().is_finite(), "distance {}", c.distance);
        assert_ne!(c.eye(), c.target);
        // Zooming out from there works as before.
        let before = c.distance;
        c.zoom(-1.0);
        assert!(c.distance > before);
    }

    #[test]
    fn orthographic_rays_of_a_large_scene_are_finite() {
        // The view-projection of this scene has a determinant that underflows f32.
        let mut c = OrbitCamera { ortho: true, ..OrbitCamera::default() };
        c.frame(Vec3::splat(-1e15), Vec3::splat(1e15));
        let r = c.ray(Vec2::ZERO, 1.0);
        assert!(r.origin.is_finite() && r.dir.is_finite(), "{r:?}");
        assert!((r.dir - c.forward()).length() < 1e-4, "{r:?}");
        let p = Vec3::new(3e14, -2e14, 1e14);
        let ndc = c.project(p, 1.0).unwrap();
        let r = c.ray(ndc.truncate(), 1.0);
        // The ray passes the point within a millionth of the scene's size.
        let to_p = p - r.origin;
        assert!((to_p - r.dir * to_p.dot(r.dir)).length() < 1e9, "{r:?}");
    }

    #[test]
    fn off_center_rays_of_a_large_scene_leave_the_center() {
        // The near and far points are finite, while the square of their distance overflows f32.
        let mut c = OrbitCamera { pitch: 0.0, ..OrbitCamera::default() };
        c.frame(Vec3::splat(-1e18), Vec3::splat(1e18));
        let r = c.ray(Vec2::new(0.5, 0.0), 1.0);
        assert!(r.origin.is_finite() && r.dir.is_finite(), "{r:?}");
        assert!((r.dir.length() - 1.0).abs() < 1e-5, "{r:?}");
        assert!(r.dir.dot(c.forward()) < 0.999, "{r:?}");
        let ndc = c.project(r.origin + r.dir * c.distance, 1.0).unwrap();
        assert!((ndc.x - 0.5).abs() < 1e-3 && ndc.y.abs() < 1e-3, "{ndc}");
    }
}
