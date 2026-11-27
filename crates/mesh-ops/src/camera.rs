//! Orbit camera (texture painting navigation) and picking rays.

use glam::{DVec4, Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    #[serde(default)]
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

    fn has_finite_basis(&self) -> bool {
        let Some(forward) = (self.target - self.eye()).try_normalize() else { return false };
        let r = self.scene_radius.max(1e-3);
        let near = (self.distance - r * 4.0).max(r * 0.01).max(1e-4);
        let far = self.distance + r * 50.0;
        self.eye().is_finite()
            && forward.cross(Vec3::Y).try_normalize().is_some()
            && self.scene_radius.is_finite()
            && self.fov_radians().is_finite()
            && self.distance > 0.0
            && near.is_finite()
            && far.is_finite()
            && far > if self.ortho { near.min(0.01) } else { near }
    }

    /// Frame a bounding box, keeping the current angles. A box whose center or framing distance
    /// is not finite, or whose framed view-projection at aspect 1 is not finite (an eye that
    /// rounds to the target, or terms past the `f32` range), leaves the camera unchanged.
    pub fn frame(&mut self, lo: Vec3, hi: Vec3) {
        let _ = self.frame_with_aspect(lo, hi, 1.0);
    }

    /// Frames a bounding box at the viewport's width-to-height `aspect`, keeping the angles.
    /// Returns an error and leaves the camera unchanged when `aspect` is not finite or is at
    /// most 0.0001, or when a finite view cannot contain every box corner.
    pub fn frame_with_aspect(&mut self, lo: Vec3, hi: Vec3, aspect: f32) -> crate::Result<()> {
        if !aspect.is_finite() || aspect <= 1e-4 {
            return Err(crate::GeomError::Invalid("camera aspect must be finite and greater than 0.0001".into()));
        }
        let target = ((lo.as_dvec3() + hi.as_dvec3()) * 0.5).as_vec3();
        let reach = (lo.as_dvec3() - target.as_dvec3()).abs().max((hi.as_dvec3() - target.as_dvec3()).abs());
        let r = (reach.length() as f32).max(1e-3);
        let vertical = self.fov_radians() * 0.5;
        let angle = if aspect < 1.0 { (vertical.tan() * aspect).atan() } else { vertical };
        let distance = r / angle.sin() * 1.1;
        if !(target.is_finite() && distance.is_finite()) {
            return Err(crate::GeomError::Invalid("camera framing bounds do not produce a finite view".into()));
        }
        let framed = OrbitCamera { target, scene_radius: r, distance, ..*self };
        if !framed.has_finite_basis() {
            return Err(crate::GeomError::Invalid("camera framing bounds do not produce a finite view".into()));
        }
        let projection = framed.view_proj(aspect);
        let inside = |p: Vec3| p.is_finite() && p.x.abs() <= 1.0 && p.y.abs() <= 1.0 && (0.0..=1.0).contains(&p.z);
        let contains = projection.is_finite()
            && projection.as_dmat4().determinant() != 0.0
            && projection.as_dmat4().inverse().is_finite()
            && [lo.x, hi.x].into_iter().all(|x| {
                [lo.y, hi.y].into_iter().all(|y| {
                    [lo.z, hi.z].into_iter().all(|z| {
                        let clip = projection * Vec3::new(x, y, z).extend(1.0);
                        let p = clip.truncate() / clip.w;
                        let separate = framed.proj(aspect) * (framed.view() * Vec3::new(x, y, z).extend(1.0));
                        clip.w > 0.0 && inside(p) && separate.w > 0.0 && inside(separate.truncate() / separate.w)
                    })
                })
            });
        if contains {
            *self = framed;
            Ok(())
        } else {
            Err(crate::GeomError::Invalid("camera framing bounds do not produce a finite view".into()))
        }
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        let candidate = OrbitCamera { yaw: self.yaw - dx, pitch: (self.pitch + dy).clamp(-89f32.to_radians(), 89f32.to_radians()), ..*self };
        if candidate.has_finite_basis() && candidate.view_proj(1.0).is_finite() {
            *self = candidate;
        }
    }

    /// Pan by a screen fraction (dx, dy in viewport heights). A movement that collapses the eye
    /// onto the target or produces a non-finite view leaves the camera unchanged.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let f = self.forward();
        let right = f.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(f);
        let scale = self.distance * (self.fov_radians() * 0.5).tan() * 2.0;
        let candidate = OrbitCamera { target: self.target + (-right * dx + up * dy) * scale, ..*self };
        if candidate.has_finite_basis() && candidate.view_proj(1.0).is_finite() {
            *self = candidate;
        }
    }

    /// Dolly by a factor exponent (positive zooms in). A distance whose view-projection at aspect 1
    /// is not finite (an eye that rounds to a target far from the origin) leaves the camera
    /// unchanged. The distance cap is computed from the field of view, scene radius, and the
    /// framing aspect threshold of 0.0001.
    /// A non-finite scene radius leaves the camera unchanged.
    pub fn zoom(&mut self, amount: f32) {
        if amount.is_nan() || amount == 0.0 || !self.scene_radius.is_finite() {
            return;
        }
        let r = self.scene_radius.max(1e-3);
        let angle = ((self.fov_radians() * 0.5).tan() * 1e-4).atan();
        let cap = (r / angle.sin() * 1.1).max(r * 200.0).min(f32::MAX);
        let scaled = f64::from(self.distance) * (-f64::from(amount)).exp();
        let distance = scaled.clamp(f64::from(r * 0.05), f64::from(cap)) as f32;
        let zoomed = OrbitCamera { distance, ..*self };
        if zoomed.has_finite_basis() && zoomed.view_proj(1.0).is_finite() {
            *self = zoomed;
        }
    }

    /// Ray through a viewport point given in normalized device coordinates (-1..1, y up). It
    /// starts on the near plane: [`OrbitCamera::proj`] has a depth range of 0 to 1. The
    /// view-projection is inverted and the points unprojected in `f64`, where the determinant of
    /// a large scene's matrix and the squared length of the ray do not leave the range.
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> Ray {
        let combined = self.view_proj(aspect).as_dmat4();
        let (x, y) = (f64::from(ndc.x), f64::from(ndc.y));
        let unproject = |transform: glam::DMat4| {
            let inv = transform.inverse();
            let a = inv * DVec4::new(x, y, 0.0, 1.0);
            let b = inv * DVec4::new(x, y, 1.0, 1.0);
            (a.truncate() / a.w, b.truncate() / b.w)
        };
        let forward = self.forward().as_dvec3();
        let rendered = (combined.determinant() != 0.0).then(|| unproject(combined));
        let (a, b) = rendered
            .filter(|(a, b)| a.is_finite() && b.is_finite() && (*b - *a).dot(forward) > 0.0 && (*a - self.eye().as_dvec3()).dot(forward) > 0.0)
            .unwrap_or_else(|| unproject(self.proj(aspect).as_dmat4() * self.view().as_dmat4()));
        let dir = (b - a).try_normalize().map(|d| d.as_vec3()).filter(|d| d.is_finite()).unwrap_or(self.forward());
        let rounded = a.as_vec3();
        let magnitudes = dir.abs();
        let axis = if magnitudes.x >= magnitudes.y && magnitudes.x >= magnitudes.z {
            0
        } else if magnitudes.y >= magnitudes.z {
            1
        } else {
            2
        };
        let coordinate = if dir[axis] > 0.0 && f64::from(rounded[axis]) > a[axis] {
            rounded[axis].next_down()
        } else if dir[axis] < 0.0 && f64::from(rounded[axis]) < a[axis] {
            rounded[axis].next_up()
        } else {
            rounded[axis]
        };
        let retreat = ((a[axis] - f64::from(coordinate)) / f64::from(dir[axis])).max(0.0);
        let origin = (a - dir.as_dvec3() * retreat).as_vec3();
        Ray { origin, dir }
    }

    /// Project a world point to NDC (x, y in -1..1, y up; z depth). None behind the camera, in
    /// both projections: the point's depth in front of the eye must be over 1e-6.
    pub fn project(&self, p: Vec3, aspect: f32) -> Option<Vec3> {
        let depth = (p.as_dvec3() - self.eye().as_dvec3()).dot(self.forward().as_dvec3());
        if !depth.is_finite() || depth <= 1e-6 {
            return None;
        }
        let c = self.view_proj(aspect) * p.extend(1.0);
        (c.w > 1e-6).then(|| c.truncate() / c.w)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn translated_orthographic_depth_keeps_visible_points() {
        let camera = OrbitCamera { target: Vec3::new(0.0, 0.0, 1_000_000.0), yaw: 0.2, pitch: 0.0, distance: 5.0, ortho: true, ..Default::default() };
        let point = Vec3::new(0.8, 0.0, 1_000_000.0 + 4.875);
        let clip = camera.view_proj(1.0) * point.extend(1.0);
        assert!(clip.x.abs() <= clip.w && (0.0..=clip.w).contains(&clip.z), "{clip:?}");
        assert!(camera.project(point, 1.0).is_some());
    }

    #[test]
    fn translated_ray_origin_does_not_round_onto_a_visible_surface() {
        let camera =
            super::OrbitCamera { target: glam::Vec3::new(0.0, 0.0, 16_777_382.0), yaw: 0.0, pitch: 0.0, distance: 12.0, ..Default::default() };
        let ray = camera.ray(glam::Vec2::ZERO, 1.0);
        assert!(ray.origin.z > 16_777_384.0, "{ray:?}");
        let ray = camera.ray(glam::Vec2::new(0.5, 0.0), 1.0);
        let inverse = camera.view_proj(1.0).as_dmat4().inverse();
        let near = inverse * glam::DVec4::new(0.5, 0.0, 0.0, 1.0);
        let far = inverse * glam::DVec4::new(0.5, 0.0, 1.0, 1.0);
        let near = near.truncate() / near.w;
        let direction = far.truncate() / far.w - near;
        let plane = 16_777_380.0;
        let expected_x = near.x + direction.x * ((plane - near.z) / direction.z);
        let actual_x = f64::from(ray.origin.x) + f64::from(ray.dir.x) * ((plane - f64::from(ray.origin.z)) / f64::from(ray.dir.z));
        assert!((actual_x - expected_x).abs() < 1e-5, "{actual_x} vs {expected_x}: {ray:?}");
        let camera = super::OrbitCamera { ortho: true, pitch: 0.0, yaw: 1e-10, ..Default::default() };
        let ray = camera.ray(glam::Vec2::new(-0.5, 0.0), 1.0);
        let depth = (ray.origin - camera.eye()).dot(camera.forward());
        assert!((0.0..0.02).contains(&depth), "{depth}: {ray:?}");
    }

    #[test]
    fn projection_matches_the_render_matrix_at_large_translations() {
        let camera = super::OrbitCamera { target: glam::Vec3::new(1e8, 0.0, 0.0), yaw: 0.0, pitch: 0.0, distance: 32.0, ..Default::default() };
        let point = camera.target + glam::Vec3::X * 8.0;
        let clip = camera.view_proj(1.0) * point.extend(1.0);
        assert_eq!(camera.project(point, 1.0), Some(clip.truncate() / clip.w));
    }

    #[test]
    fn framing_rejects_a_collapsed_basis_before_matrix_construction() {
        let mut camera = OrbitCamera::default();
        let before = camera;
        let point = Vec3::splat(1e30);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| camera.frame_with_aspect(point, point, 1.0)));
        assert!(result.is_ok(), "framing must report an error before building a singular basis");
        assert!(result.unwrap().is_err());
        assert_eq!(camera, before);
    }

    #[test]
    fn zoom_rejects_nonfinite_scene_radii_without_panicking() {
        for radius in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            let mut camera = OrbitCamera { scene_radius: radius, ..OrbitCamera::default() };
            let before = camera.distance;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| camera.zoom(1.0)));
            assert!(result.is_ok(), "radius {radius}");
            assert_eq!(camera.distance, before);
        }
    }

    #[test]
    fn zoom_reverses_across_the_default_distance_threshold() {
        for ortho in [false, true] {
            let mut camera = OrbitCamera { ortho, scene_radius: 1.0, distance: 201.0, ..OrbitCamera::default() };
            let amount = (201.0_f32 / 199.0).ln();
            camera.zoom(amount);
            assert!(camera.distance < 200.0);
            camera.zoom(-amount);
            assert!((camera.distance - 201.0).abs() < 1e-3, "{ortho}: {}", camera.distance);
            camera.zoom(-100.0);
            let limit = camera.distance;
            camera.zoom(-100.0);
            assert_eq!(camera.distance, limit);
            assert!(limit.is_finite());
        }
    }

    #[test]
    fn framing_rejects_unusable_combined_projection() {
        let mut camera = OrbitCamera { yaw: 0.0, pitch: 0.0, ..OrbitCamera::default() };
        let before = camera;
        assert!(camera.frame_with_aspect(Vec3::new(-0.03125, 0.0, 1_114_112.0), Vec3::new(0.03125, 0.0, 1_114_112.0), 1.0).is_err());
        assert_eq!(camera, before);
    }

    #[test]
    fn framing_rejects_singular_orthographic_projection() {
        let mut camera = OrbitCamera { ortho: true, ..OrbitCamera::default() };
        let before = camera;
        assert!(camera.frame_with_aspect(-Vec3::ONE, Vec3::ONE, 1e38).is_err());
        assert_eq!(camera, before);
    }

    #[test]
    fn framing_rejects_a_rounded_eye_that_clips_the_target() {
        let mut camera = OrbitCamera { yaw: 0.0, pitch: 0.0, fov: 40.0, ..OrbitCamera::default() };
        let before = camera;
        let point = Vec3::new(0.0, 0.0, 67_108_864.0);
        assert!(camera.frame_with_aspect(point, point, 0.0007).is_err());
        assert_eq!(camera, before);
    }

    use super::*;

    #[test]
    fn framing_encloses_a_translated_box_around_the_stored_target() {
        let mut camera = OrbitCamera { yaw: 0.0, pitch: 0.0, ..OrbitCamera::default() };
        let (lo, hi) = (1e8f32, 1e8f32 + 8.0);
        camera.frame_with_aspect(Vec3::splat(lo), Vec3::splat(hi), 1.0).unwrap();
        for x in [lo, hi] {
            for y in [lo, hi] {
                for z in [lo, hi] {
                    let projected = camera.project(Vec3::new(x, y, z), 1.0).unwrap();
                    assert!(projected.x.abs() <= 1.0 && projected.y.abs() <= 1.0, "{projected:?}");
                }
            }
        }
    }

    #[test]
    fn pan_preserves_the_camera_when_rounding_would_collapse_the_eye() {
        let mut camera = OrbitCamera {
            target: Vec3::new(67_108_860.0, 0.0, -67_108_864.0),
            yaw: std::f32::consts::FRAC_PI_4,
            pitch: 0.0,
            distance: 5.0,
            fov: 150.0,
            ..OrbitCamera::default()
        };
        assert!(camera.view().is_finite());
        let before = camera;
        camera.pan(-0.25, 0.0);
        assert_eq!(camera, before);
        assert!(camera.view().is_finite());
    }

    #[test]
    fn framing_keeps_box_corners_inside_a_portrait_viewport() {
        for ortho in [false, true] {
            for aspect in [0.25, 1.0, 4.0] {
                let mut camera = OrbitCamera { yaw: 0.0, pitch: 0.0, ortho, ..OrbitCamera::default() };
                camera.frame_with_aspect(-Vec3::ONE, Vec3::ONE, aspect).unwrap();
                for x in [-1.0, 1.0] {
                    for y in [-1.0, 1.0] {
                        for z in [-1.0, 1.0] {
                            let point = camera.project(Vec3::new(x, y, z), aspect).unwrap();
                            assert!(point.x.abs() <= 1.0 && point.y.abs() <= 1.0, "{ortho} {aspect}: {point:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn portrait_framing_keeps_zoom_continuous_beyond_the_default_distance_limit() {
        for ortho in [false, true] {
            let mut camera = OrbitCamera { ortho, ..OrbitCamera::default() };
            camera.frame_with_aspect(-Vec3::ONE, Vec3::ONE, 0.001).unwrap();
            let distance = camera.distance;
            for amount in [0.0, -0.1, 0.1, f32::NAN] {
                let mut zoomed = camera;
                zoomed.zoom(amount);
                if amount == 0.0 || amount.is_nan() {
                    assert_eq!(zoomed.distance, distance);
                } else if amount < 0.0 {
                    assert!(zoomed.distance > distance);
                } else {
                    assert!(zoomed.distance < distance && zoomed.distance > distance * 0.8);
                }
            }
        }
    }

    #[test]
    fn framing_rejects_invalid_aspects_without_changing_the_camera() {
        let camera = OrbitCamera::default();
        for aspect in [0.0, -1.0, 1e-4, f32::NAN, f32::INFINITY] {
            let mut framed = camera;
            assert!(framed.frame_with_aspect(-Vec3::ONE, Vec3::ONE, aspect).is_err());
            assert_eq!(framed, camera);
        }
    }

    #[test]
    fn orbit_preserves_the_camera_when_the_eye_would_round_to_its_target() {
        let camera = OrbitCamera { target: Vec3::splat(1e8), distance: 5.0, pitch: 0.0, ..OrbitCamera::default() };
        assert!(camera.view().is_finite());
        for (dx, dy) in [(-std::f32::consts::FRAC_PI_4, 0.0), (f32::NAN, 0.0), (0.0, f32::NAN)] {
            let mut moved = camera;
            moved.orbit(dx, dy);
            assert_eq!(moved, camera);
            assert!(moved.view_proj(1.0).is_finite());
        }
    }

    #[test]
    fn translated_camera_rays_keep_a_finite_origin() {
        for target_z in [16_777_210.0, 16_777_212.0] {
            let camera = OrbitCamera { target: Vec3::new(0.0, 0.0, target_z), distance: 4.0, pitch: 0.0, ..OrbitCamera::default() };
            for aspect in [0.5, 1.0, 2.0] {
                for point in [Vec2::ZERO, Vec2::new(0.5, -0.25)] {
                    let ray = camera.ray(point, aspect);
                    assert!(ray.origin.is_finite() && ray.dir.is_finite(), "{ray:?}");
                    assert!((ray.dir.length() - 1.0).abs() < 0.00001, "{ray:?}");
                    assert!(ray.dir.dot(camera.forward()) > 0.0, "{target_z}: {ray:?}");
                }
            }
        }
    }

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
