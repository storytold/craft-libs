//! Orbit camera (texture painting navigation) and picking rays.

use glam::{Mat4, Vec2, Vec3, Vec4};
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

    pub fn proj(&self, aspect: f32) -> Mat4 {
        let r = self.scene_radius.max(1e-3);
        let near = (self.distance - r * 4.0).max(r * 0.01).max(1e-4);
        let far = self.distance + r * 50.0;
        let aspect = if aspect.is_finite() && aspect > 1e-4 { aspect } else { 1.0 };
        if self.ortho {
            let h = self.distance * (self.fov.to_radians() * 0.5).tan();
            Mat4::orthographic_rh(-h * aspect, h * aspect, -h, h, near.min(0.01), far)
        } else {
            Mat4::perspective_rh(self.fov.clamp(5.0, 150.0).to_radians(), aspect, near, far)
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    /// Frame a bounding box, keeping the current angles.
    pub fn frame(&mut self, lo: Vec3, hi: Vec3) {
        self.target = (lo + hi) * 0.5;
        let r = ((hi - lo).length() * 0.5).max(1e-3);
        self.scene_radius = r;
        self.distance = r / (self.fov.clamp(5.0, 150.0).to_radians() * 0.5).sin() * 1.1;
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
        let scale = self.distance * (self.fov.to_radians() * 0.5).tan() * 2.0;
        self.target += (-right * dx + up * dy) * scale;
    }

    /// Dolly by a factor exponent (positive zooms in).
    pub fn zoom(&mut self, amount: f32) {
        let r = self.scene_radius.max(1e-3);
        self.distance = (self.distance * (-amount).exp()).clamp(r * 0.05, r * 200.0);
    }

    /// Ray through a viewport point given in normalized device coordinates (-1..1, y up).
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> Ray {
        let inv = self.view_proj(aspect).inverse();
        let a = inv * Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
        let b = inv * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
        let (a, b) = (a.truncate() / a.w, b.truncate() / b.w);
        Ray { origin: a, dir: (b - a).try_normalize().unwrap_or(self.forward()) }
    }

    /// Project a world point to NDC (x, y in -1..1, y up; z depth). None behind the camera.
    pub fn project(&self, p: Vec3, aspect: f32) -> Option<Vec3> {
        let c = self.view_proj(aspect) * p.extend(1.0);
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
}
