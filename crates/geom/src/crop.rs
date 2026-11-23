//! The crop model.
//!
//! The image (w × h pixels after orientation) is rotated by `angle` about its centre. The crop is an
//! axis-aligned rectangle in that rotated frame, stored *normalized* to the image size
//! (`x / w`, `y / h`), so it is independent of preview resolution. The crop must stay inside the
//! rotated image (Lightroom's "Constrain to image").

use serde::{Deserialize, Serialize};

use crate::{Affine, MIN_CROP_FRACTION, Point, Rect, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropGeometry {
    /// Normalized crop rectangle in the rotated frame (0..1 when angle = 0).
    pub rect: Rect,
    /// Straighten angle in degrees, clockwise positive, −45..45.
    pub angle: f64,
}

impl Default for CropGeometry {
    fn default() -> Self {
        Self { rect: Rect::UNIT, angle: 0.0 }
    }
}

impl CropGeometry {
    pub fn is_identity(&self) -> bool {
        self.angle == 0.0 && self.rect == Rect::UNIT
    }

    /// Crop rectangle in pixels of the rotated frame for an image of `w × h`.
    pub fn rect_px(&self, w: f64, h: f64) -> Rect {
        self.rect.scale(w, h)
    }

    /// Transform from *output* (cropped) pixel coordinates to *source* (oriented image) pixel
    /// coordinates, for an output of `out_w × out_h` pixels rendered from a `w × h` source.
    pub fn output_to_source(&self, w: f64, h: f64, out_w: f64, out_h: f64) -> Affine {
        let r = self.rect_px(w, h);
        // output -> crop rect in rotated frame
        let to_rot = Affine::translate(Vec2::new(r.x0, r.y0)) * Affine::scale(r.width() / out_w, r.height() / out_h);
        // rotated frame -> source: inverse rotation about the image centre
        let c = Point::new(w / 2.0, h / 2.0);
        Affine::rotate_about(-self.angle.to_radians(), c) * to_rot
    }

    /// This crop with values no crop tool can produce taken out, for settings read from a file or an
    /// agent (issue storytold/lightcraft#2555): the angle within ±45° (not finite: 0), and the rect clipped to the
    /// bounding box of the image turned by that angle, which holds every crop that stays on the
    /// photo, so a real crop is unchanged while one reaching far past the photo can no longer make
    /// a full-size output of billions of pixels. A rect with nothing left (reversed, or not finite)
    /// is the whole frame.
    pub fn bounded(&self, w: f64, h: f64) -> CropGeometry {
        let angle = if self.angle.is_finite() { self.angle.clamp(-45.0, 45.0) } else { 0.0 };
        let (sin, cos) = angle.to_radians().sin_cos();
        let (bw, bh) = (w * cos.abs() + h * sin.abs(), w * sin.abs() + h * cos.abs());
        let bbox = Rect::from_center(Point::new(w / 2.0, h / 2.0), bw, bh);
        let r = self.rect_px(w, h);
        let finite = [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite());
        let inside = r.x0 >= bbox.x0 && r.y0 >= bbox.y0 && r.x1 <= bbox.x1 && r.y1 <= bbox.y1;
        // a reversed rect (right edge left of its left edge, or bottom above top) has a negative size
        let ordered = r.x0 <= r.x1 && r.y0 <= r.y1;
        if finite && inside && ordered {
            // left exactly as stored: a round trip through pixels would change its last bits
            return CropGeometry { rect: self.rect, angle };
        }
        let rect = if finite { r.intersect(&bbox) } else { Rect::new(0.0, 0.0, 0.0, 0.0) };
        let rect = if rect.is_empty() || w <= 0.0 || h <= 0.0 { Rect::UNIT } else { rect.scale(1.0 / w, 1.0 / h) };
        CropGeometry { rect, angle }
    }

    /// Whether every crop corner lies within the rotated image.
    pub fn is_within_image(&self, w: f64, h: f64) -> bool {
        let r = self.rect_px(w, h);
        let back = Affine::rotate_about(-self.angle.to_radians(), Point::new(w / 2.0, h / 2.0));
        r.corners().iter().all(|p| {
            let q = back.apply(*p);
            q.x >= -1e-6 * w && q.y >= -1e-6 * h && q.x <= w * (1.0 + 1e-6) && q.y <= h * (1.0 + 1e-6)
        })
    }

    /// Shrink the crop (about its centre, keeping aspect) and then shift it until it fits inside the
    /// rotated image.
    pub fn constrained(&self, w: f64, h: f64) -> CropGeometry {
        if self.is_within_image(w, h) {
            return *self;
        }
        let r = self.rect_px(w, h);
        // Try moving towards the image centre first.
        let c = Point::new(w / 2.0, h / 2.0);
        let mut best = *self;
        for i in 1..=32 {
            let t = i as f64 / 32.0;
            let nc = r.center().lerp(c, t);
            let cand = CropGeometry { rect: Rect::from_center(nc, r.width(), r.height()).scale(1.0 / w, 1.0 / h), angle: self.angle };
            if cand.is_within_image(w, h) {
                return cand;
            }
            best = cand;
        }
        // Then shrink around the centre.
        let s = max_inscribed_scale(w, h, self.angle, r.width(), r.height());
        let nr = Rect::from_center(c, r.width() * s, r.height() * s);
        let _ = best;
        let out = CropGeometry { rect: nr.scale(1.0 / w, 1.0 / h), angle: self.angle };
        if [out.rect.x0, out.rect.y0, out.rect.x1, out.rect.y1].iter().all(|v| v.is_finite()) {
            return out;
        }
        // A rect too large for pixel maths (`inf * 0` above): the largest crop of its aspect that
        // fits, else of the image's own.
        let aspect = Some(self.rect.width() / self.rect.height() * (w / h)).filter(|a| a.is_finite() && *a > 0.0);
        crop_fit_angle(w, h, self.angle, aspect)
    }

    /// This crop turned to `angle_deg` on a `w × h` image: kept as it is when it still fits inside
    /// the rotated image, else shrunk about its own centre, keeping its aspect, until it fits
    /// (Lightroom's "Constrain to image"). Every way of setting the angle goes through here (the
    /// rotate handle, the Straighten slider and Angle field, `crop.straighten`, `develop.set` on
    /// `crop.angle`), so they all give the same crop (issue storytold/lightcraft#742). A crop that is the largest of its
    /// aspect at its own angle (the full photo, or one an earlier turn shrank to fit) stays the
    /// largest at the new angle, so it grows back as the angle returns towards 0° (issue storytold/lightcraft#1388).
    /// Any other crop never grows: a crop dragged in from a corner still touches two edges of the
    /// photo, and turning it used to swap it for the largest crop of its aspect (issue storytold/lightcraft#1794).
    pub fn with_angle(&self, angle_deg: f64, w: f64, h: f64) -> CropGeometry {
        let r = self.rect_px(w, h);
        // a degenerate rect has no aspect to keep: fall back to the image's own
        let aspect = Some(r.width() / r.height()).filter(|a| a.is_finite() && *a > 0.0);
        let largest = crop_fit_angle(w, h, self.angle, aspect).rect;
        let is_largest = [(self.rect.x0, largest.x0), (self.rect.y0, largest.y0), (self.rect.x1, largest.x1), (self.rect.y1, largest.y1)]
            .iter()
            .all(|(a, b)| (a - b).abs() < 1e-9);
        let cand = CropGeometry { rect: self.rect, angle: angle_deg };
        if !is_largest && cand.is_within_image(w, h) {
            return cand;
        }
        let fitted = crop_fit_angle(w, h, angle_deg, aspect);
        if is_largest || aspect.is_none() {
            return fitted;
        }
        // shrunk in place, as long as that leaves a crop the drag model would allow
        let min_side = MIN_CROP_FRACTION * w.min(h);
        if let Some(s) = cand.fit_scale_about_centre(w, h).filter(|s| r.width().min(r.height()) * s >= min_side) {
            return CropGeometry { rect: Rect::from_center(r.center(), r.width() * s, r.height() * s).scale(1.0 / w, 1.0 / h), angle: angle_deg };
        }
        // its centre is off the turned photo (a sliver in a corner): centred, at its own size at most
        let f = fitted.rect_px(w, h);
        let Some(s) = Some(r.width() / f.width()).filter(|s| s.is_finite() && *s < 1.0) else {
            return fitted;
        };
        CropGeometry { rect: Rect::from_center(f.center(), f.width() * s, f.height() * s).scale(1.0 / w, 1.0 / h), angle: angle_deg }
    }

    /// The largest scale, at most 1, at which this crop shrunk about its own centre lies inside the
    /// rotated image; `None` when its centre is outside it.
    fn fit_scale_about_centre(&self, w: f64, h: f64) -> Option<f64> {
        let r = self.rect_px(w, h);
        let back = Affine::rotate_about(-self.angle.to_radians(), Point::new(w / 2.0, h / 2.0));
        let c = back.apply(r.center());
        if !(0.0..=w).contains(&c.x) || !(0.0..=h).contains(&c.y) {
            return None;
        }
        // each corner, in the image frame, is c + s·d: keep both coordinates in 0..w and 0..h
        let mut s: f64 = 1.0;
        for corner in r.corners() {
            let d = back.apply(corner) - c;
            for (q, dq, max) in [(c.x, d.x, w), (c.y, d.y, h)] {
                if dq > 0.0 {
                    s = s.min((max - q) / dq);
                } else if dq < 0.0 {
                    s = s.min(q / -dq);
                }
            }
        }
        Some(s).filter(|s| s.is_finite() && *s > 0.0)
    }
}

/// Largest scale `s` such that a centred `cw·s × ch·s` rectangle fits inside a `w × h` image rotated
/// by `angle_deg` about its centre.
pub fn max_inscribed_scale(w: f64, h: f64, angle_deg: f64, cw: f64, ch: f64) -> f64 {
    let a = angle_deg.to_radians().abs();
    let (s, c) = a.sin_cos();
    // Half extents of the crop must satisfy, in the image frame (rotated back):
    // |x| c + |y| s <= w/2 and |x| s + |y| c <= h/2 for the crop corner (x, y) = (cw/2, ch/2)·scale.
    let (hx, hy) = (cw / 2.0, ch / 2.0);
    let s1 = (w / 2.0) / (hx * c + hy * s);
    let s2 = (h / 2.0) / (hx * s + hy * c);
    s1.min(s2)
}

/// The biggest crop of the image's own aspect (or `aspect` if given) that fits at `angle_deg`.
pub fn crop_fit_angle(w: f64, h: f64, angle_deg: f64, aspect: Option<f64>) -> CropGeometry {
    let a = aspect.unwrap_or(w / h);
    let base = Rect::new(0.0, 0.0, w, h).fit_aspect(a);
    let s = max_inscribed_scale(w, h, angle_deg, base.width(), base.height()).min(1.0);
    let r = Rect::from_center(Point::new(w / 2.0, h / 2.0), base.width() * s, base.height() * s);
    CropGeometry { rect: r.scale(1.0 / w, 1.0 / h), angle: angle_deg }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue storytold/lightcraft#2555: `bounded` leaves every crop that stays on the photo exactly as it was, and
    /// clips one reaching far past it to the turned photo's bounding box.
    #[test]
    fn bounded_keeps_real_crops_and_clips_the_rest() {
        for (w, h) in [(6000.0, 4000.0), (4000.0, 6000.0), (9000.0, 1000.0)] {
            for angle in [-45.0, -12.5, 0.0, 0.1, 7.0, 45.0] {
                for aspect in [None, Some(1.0), Some(3.0), Some(0.25)] {
                    let real = crop_fit_angle(w, h, angle, aspect);
                    assert_eq!(real.bounded(w, h), real, "{w}x{h} {angle} {aspect:?}");
                }
            }
            assert_eq!(CropGeometry::default().bounded(w, h), CropGeometry::default());
        }
        let (w, h) = (6000.0, 4000.0);
        let wild = CropGeometry { rect: Rect::new(0.0, 0.0, 1000.0, 1000.0), angle: 5.0 };
        let b = wild.bounded(w, h).rect_px(w, h);
        assert!(b.width() <= w * 1.5 && b.height() <= h * 1.5, "{b:?}");
        // angle beyond the tools' range, or not a number; a rect that is not a number
        assert_eq!(CropGeometry { rect: Rect::UNIT, angle: 400.0 }.bounded(w, h).angle, 45.0);
        assert_eq!(CropGeometry { rect: Rect::UNIT, angle: f64::NAN }.bounded(w, h).angle, 0.0);
        let nan = CropGeometry { rect: Rect::new(f64::NAN, 0.0, 1.0, 1.0), angle: 3.0 }.bounded(w, h);
        assert_eq!(nan.rect, Rect::UNIT);
        // a crop entirely off the turned photo has nothing left: the whole frame
        let off = CropGeometry { rect: Rect::new(50.0, 50.0, 60.0, 60.0), angle: 10.0 }.bounded(w, h);
        assert_eq!(off.rect, Rect::UNIT);
        // a reversed rect inside the photo is the whole frame, never a negative size
        let reversed = CropGeometry { rect: Rect::new(0.8, 0.2, 0.2, 0.8), angle: 0.0 }.bounded(w, h);
        assert_eq!(reversed.rect, Rect::UNIT);
        let flipped = CropGeometry { rect: Rect::new(0.2, 0.8, 0.8, 0.2), angle: 3.0 }.bounded(w, h);
        assert_eq!(flipped.rect, Rect::UNIT);
        // a zero-size crop is left as it was
        let line = CropGeometry { rect: Rect::new(0.5, 0.2, 0.5, 0.8), angle: 0.0 };
        assert_eq!(line.bounded(w, h), line);
    }

    #[test]
    fn identity_maps_pixels() {
        let c = CropGeometry::default();
        let t = c.output_to_source(400.0, 300.0, 400.0, 300.0);
        let p = t.apply(Point::new(10.0, 20.0));
        assert!((p.x - 10.0).abs() < 1e-9 && (p.y - 20.0).abs() < 1e-9);
    }

    #[test]
    fn rotated_fit_is_inside() {
        for ang in [-45.0, -10.0, 0.0, 3.5, 30.0, 45.0] {
            let c = crop_fit_angle(600.0, 400.0, ang, None);
            assert!(c.is_within_image(600.0, 400.0), "{ang}");
            // and maximal: 1% bigger does not fit (except angle 0 where it equals the image)
            let big = CropGeometry { rect: Rect::from_center(c.rect.center(), c.rect.width() * 1.01, c.rect.height() * 1.01), angle: ang };
            assert!(!big.is_within_image(600.0, 400.0), "{ang}");
        }
    }

    #[test]
    fn constrain_pulls_crop_inside() {
        let c = CropGeometry { rect: Rect::new(0.5, 0.5, 1.4, 1.2), angle: 12.0 };
        let k = c.constrained(300.0, 200.0);
        assert!(k.is_within_image(300.0, 200.0));
    }

    #[test]
    fn with_angle_keeps_a_crop_that_still_fits() {
        // a small centred crop fits at a few degrees: the angle changes, the rect does not
        let c = CropGeometry { rect: Rect::new(0.3, 0.3, 0.7, 0.7), angle: 0.0 };
        for ang in [-3.0, 0.5, 2.0, 5.0] {
            let t = c.with_angle(ang, 600.0, 400.0);
            assert_eq!(t.rect, c.rect, "{ang}°");
            assert_eq!(t.angle, ang);
        }
    }

    #[test]
    fn with_angle_shrinks_a_crop_that_no_longer_fits_keeping_its_aspect() {
        for (rect, ang) in [(Rect::UNIT, 15.0), (Rect::new(0.0, 0.1, 0.8, 0.9), -20.0), (Rect::new(0.2, 0.0, 0.8, 1.0), 45.0)] {
            let c = CropGeometry { rect, angle: 0.0 };
            let t = c.with_angle(ang, 600.0, 400.0);
            assert!(t.is_within_image(600.0, 400.0), "{rect:?} at {ang}°");
            let (a, b) = (c.rect_px(600.0, 400.0), t.rect_px(600.0, 400.0));
            assert!((a.aspect() - b.aspect()).abs() < 1e-9, "aspect kept: {} vs {}", a.aspect(), b.aspect());
            assert_eq!(t.angle, ang);
        }
    }

    #[test]
    fn with_angle_grows_a_fitted_crop_back_as_the_angle_returns() {
        // issue storytold/lightcraft#1388: the crop a turn shrank to fit is the largest of its aspect; turning back
        // gives the largest at the new angle, not the shrunk one again
        for aspect in [None, Some(1.0), Some(16.0 / 9.0)] {
            let mut c = crop_fit_angle(600.0, 400.0, 0.0, aspect);
            for ang in [10.0, 25.0, -25.0, 7.0, 0.0] {
                c = c.with_angle(ang, 600.0, 400.0);
                let want = crop_fit_angle(600.0, 400.0, ang, aspect);
                assert!(
                    (c.rect.width() - want.rect.width()).abs() < 1e-9 && (c.rect.height() - want.rect.height()).abs() < 1e-9,
                    "{aspect:?} at {ang}°: {c:?} vs {want:?}"
                );
            }
        }
    }

    #[test]
    fn with_angle_shrinks_a_crop_against_the_edge_about_its_own_centre() {
        // issue storytold/lightcraft#1794: a crop dragged in from one corner keeps the opposite corner on the photo's,
        // so it stops fitting at the first fraction of a degree; it used to become the largest crop
        // of its aspect, nearly the whole photo
        let (w, h) = (600.0, 400.0);
        let c = CropGeometry { rect: Rect::new(0.0, 0.0, 0.6, 0.6), angle: 0.0 };
        let a = c.rect_px(w, h);
        for ang in [-10.0, -1.0, 0.5, 3.0, 20.0] {
            let t = c.with_angle(ang, w, h);
            let b = t.rect_px(w, h);
            assert!(t.is_within_image(w, h), "{ang}°");
            assert_eq!(t.angle, ang);
            assert!(b.width() <= a.width() + 1e-9, "{ang}°: never larger than the crop made, {} vs {}", b.width(), a.width());
            assert!((a.aspect() - b.aspect()).abs() < 1e-9, "{ang}°: aspect kept");
            assert!(b.center().dist(a.center()) < 1e-9, "{ang}°: shrunk about its own centre");
            // as large as fits there: 1 % bigger about the same centre does not
            let big = CropGeometry { rect: Rect::from_center(t.rect.center(), t.rect.width() * 1.01, t.rect.height() * 1.01), angle: ang };
            assert!(!big.is_within_image(w, h), "{ang}°: the largest about its centre");
        }
        // a small turn is a small change, not a jump
        let t = c.with_angle(0.5, w, h).rect_px(w, h);
        assert!(t.width() > 0.97 * a.width(), "0.5° keeps nearly all of the crop: {} of {}", t.width(), a.width());
    }

    #[test]
    fn with_angle_never_grows_a_crop_whose_centre_leaves_the_photo() {
        // a sliver in the corner: at 45° its centre is outside the turned photo, so it can't shrink
        // in place; it moves to the centre at its own size, not at the largest of its aspect
        let (w, h) = (600.0, 400.0);
        let c = CropGeometry { rect: Rect::new(0.0, 0.0, 0.05, 0.05), angle: 0.0 };
        let a = c.rect_px(w, h);
        let t = c.with_angle(45.0, w, h);
        let b = t.rect_px(w, h);
        assert!(t.is_within_image(w, h));
        assert!(b.width() <= a.width() + 1e-9 && (a.aspect() - b.aspect()).abs() < 1e-9, "{b:?} from {a:?}");
        assert!(b.center().dist(Point::new(w / 2.0, h / 2.0)) < 1e-9, "centred: {b:?}");
    }

    #[test]
    fn with_angle_survives_a_degenerate_crop() {
        // zero height: no aspect to keep (width / height = ∞)
        let c = CropGeometry { rect: Rect::new(0.0, 0.5, 1.0, 0.5), angle: 0.0 };
        let t = c.with_angle(30.0, 600.0, 400.0);
        assert!(t.is_within_image(600.0, 400.0));
        assert!([t.rect.x0, t.rect.y0, t.rect.x1, t.rect.y1].iter().all(|v| v.is_finite()));
    }

    #[test]
    fn half_crop_maps_to_centre_quadrant() {
        let c = CropGeometry { rect: Rect::new(0.25, 0.25, 0.75, 0.75), angle: 0.0 };
        let t = c.output_to_source(100.0, 100.0, 50.0, 50.0);
        let p = t.apply(Point::new(0.0, 0.0));
        assert!((p.x - 25.0).abs() < 1e-9 && (p.y - 25.0).abs() < 1e-9);
    }

    #[test]
    fn constrained_huge_rect_is_finite_and_inside() {
        for rect in [Rect::new(0.0, 0.0, 1e308, 1e308), Rect::new(-1e308, -1e308, 1e308, 1e308), Rect::new(0.0, 0.0, 1e308, 0.5)] {
            for angle in [0.0, 30.0] {
                let c = CropGeometry { rect, angle }.constrained(400.0, 300.0);
                let r = c.rect;
                assert!([r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()), "{rect:?} {angle}: {r:?}");
                assert!(c.is_within_image(400.0, 300.0), "{rect:?} {angle}: {r:?}");
            }
        }
    }
}
