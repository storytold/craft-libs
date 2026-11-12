//! Blemish detection on faces: small, round spots that are darker or redder than the skin around
//! them. Classical, in the spirit of [`crate::dust`]: a blurred copy gives the surrounding skin,
//! a spot has to stand out from both the surroundings and the local texture, and each candidate
//! blob must be small and roughly round. The face boxes and landmarks come from a face detector
//! (YuNet) so eyes, brows, nostrils and lips are never taken for blemishes.
//!
//! Deliberately conservative: a face with many candidates has stubble, freckles or pores, not
//! blemishes, and gets none; the rest are capped per face. Removing freckles is a different
//! feature and never a default.

use crate::raster::{Plane, Rgba8, blur::gaussian};

/// A face in fractions of the image (0..1, y down): its box and the five landmarks a YuNet
/// detector gives, in its order (right eye, left eye, nose tip, right and left mouth corner,
/// from the subject's point of view).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub landmarks: [(f64, f64); 5],
}

/// A found blemish: which face, centre (0..1 of the image's width / height), radius (fraction
/// of the long edge) and how much it stands out (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blemish {
    pub face: usize,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub strength: f32,
}

/// Faces narrower than this (in pixels of the analysed image) are left alone: a blemish is not
/// resolved below it, and what is found there is lash shadow and nostril edges.
pub const MIN_FACE_PX: f64 = 160.0;
/// More candidates than this on one face means texture (stubble, freckles), not blemishes.
pub const MAX_CANDIDATES_PER_FACE: usize = 12;
/// Most blemishes kept per face, strongest first.
pub const MAX_PER_FACE: usize = 6;

/// Pixels of `img` inside the box that are skin: an ellipse over the face, gated by skin colour,
/// with the eyes, brows, nostrils and mouth cut out around the landmarks.
fn skin_mask(img: &Rgba8, f: &FaceBox, x0: usize, y0: usize, cw: usize, ch: usize) -> Vec<bool> {
    let (w, h) = (img.width as f64, img.height as f64);
    let (fx, fy, fw, fh) = (f.x0 * w, f.y0 * h, (f.x1 - f.x0) * w, (f.y1 - f.y0) * h);
    let (cx, cy) = (fx + fw / 2.0, fy + fh * 0.55);
    let (rx, ry) = (fw * 0.55, fh * 0.70);
    let lm = |i: usize| (f.landmarks[i].0 * w, f.landmarks[i].1 * h);
    let (re, le, nose, mr, ml) = (lm(0), lm(1), lm(2), lm(3), lm(4));
    let mouth = ((mr.0 + ml.0) / 2.0, (mr.1 + ml.1) / 2.0);
    let eye_y = (re.1 + le.1) / 2.0;
    let mut out = vec![false; cw * ch];
    for yy in 0..ch {
        for xx in 0..cw {
            let (px, py) = ((x0 + xx) as f64 + 0.5, (y0 + yy) as f64 + 0.5);
            let in_ellipse = ((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2) <= 1.0;
            if !in_ellipse {
                continue;
            }
            // brows and eyes: a band from above the brows to just below the eyes
            if py > eye_y - fh * 0.32 && py < eye_y + fh * 0.05 {
                continue;
            }
            let near = |(lx, ly): (f64, f64), r: f64| (px - lx).powi(2) + (py - ly).powi(2) <= r * r;
            if near(re, fw * 0.12) || near(le, fw * 0.12) || near(mouth, fw * 0.16) {
                continue;
            }
            // nostrils: an ellipse at and below the nose tip
            if ((px - nose.0) / (fw * 0.20)).powi(2) + ((py - nose.1 - fw * 0.02) / (fw * 0.16)).powi(2) <= 1.0 {
                continue;
            }
            let Some(p) = img.data.get((y0 + yy) * img.width + x0 + xx) else { continue };
            let (r, g, b) = (f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
            let cr = 128.0 + 0.5 * r - 0.4187 * g - 0.0813 * b;
            let cb = 128.0 - 0.1687 * r - 0.3313 * g + 0.5 * b;
            if let Some(o) = out.get_mut(yy * cw + xx) {
                *o = (133.0..180.0).contains(&cr) && (77.0..130.0).contains(&cb);
            }
        }
    }
    out
}

/// Blemishes on the given faces of a rendered image; `sensitivity` 0..100 (higher finds fainter
/// spots). Faces narrower than [`MIN_FACE_PX`] get none; a face with more than
/// [`MAX_CANDIDATES_PER_FACE`] candidates gets none; at most [`MAX_PER_FACE`] per face.
pub fn detect(img: &Rgba8, faces: &[FaceBox], sensitivity: f32) -> Vec<Blemish> {
    let (w, h) = (img.width, img.height);
    if w < 32 || h < 32 {
        return Vec::new();
    }
    let long = w.max(h) as f64;
    let thr = 0.09 - 0.05 * (sensitivity / 100.0).clamp(0.0, 1.0);
    let mut out = Vec::new();
    for (fi, f) in faces.iter().enumerate() {
        let fw = (f.x1 - f.x0) * w as f64;
        let fh = (f.y1 - f.y0) * h as f64;
        if !(fw.is_finite() && fh.is_finite()) || fw < MIN_FACE_PX || fh <= 0.0 {
            continue;
        }
        // the face with a margin, clipped to the image
        let clamp = |v: f64, hi: usize| (v.max(0.0) as usize).min(hi);
        let x0 = clamp(f.x0 * w as f64 - fw * 0.15, w);
        let y0 = clamp(f.y0 * h as f64 - fh * 0.15, h);
        let x1 = clamp(f.x1 * w as f64 + fw * 0.15, w);
        let y1 = clamp(f.y1 * h as f64 + fh * 0.15, h);
        let (cw, ch) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        if cw < 16 || ch < 16 {
            continue;
        }
        let skin = skin_mask(img, f, x0, y0, cw, ch);
        let px = |xx: usize, yy: usize| img.data.get((y0 + yy) * w + x0 + xx).copied().unwrap_or([0; 4]);
        let lum = Plane::from_fn(cw, ch, |xx, yy| {
            let p = px(xx, yy);
            (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])) / 255.0
        });
        // redness: how much red exceeds green, which a pimple does and even skin does not
        let red = Plane::from_fn(cw, ch, |xx, yy| {
            let p = px(xx, yy);
            (f32::from(p[0]) - f32::from(p[1])) / 255.0
        });
        let sigma = (0.035 * fw as f32).max(2.0);
        let around_l = gaussian(&lum, sigma);
        let around_r = gaussian(&red, sigma);
        let detail = Plane::from_fn(cw, ch, |xx, yy| {
            let i = yy * cw + xx;
            (lum.data.get(i).copied().unwrap_or(0.0) - around_l.data.get(i).copied().unwrap_or(0.0)).abs()
        });
        let texture = gaussian(&detail, (0.06 * fw as f32).max(3.0));
        let resp: Vec<f32> = (0..cw * ch)
            .map(|i| {
                let dark = around_l.data.get(i).copied().unwrap_or(0.0) - lum.data.get(i).copied().unwrap_or(0.0);
                let redder = red.data.get(i).copied().unwrap_or(0.0) - around_r.data.get(i).copied().unwrap_or(0.0);
                dark.max(redder).max(0.0)
            })
            .collect();
        let candidate = |i: usize| {
            skin.get(i).copied().unwrap_or(false)
                && resp.get(i).copied().unwrap_or(0.0) > thr
                && resp.get(i).copied().unwrap_or(0.0) > 2.5 * texture.data.get(i).copied().unwrap_or(0.0)
        };
        // blobs of candidate pixels (4-connected), small and roundish
        let mut seen = vec![false; cw * ch];
        let mut found: Vec<Blemish> = Vec::new();
        let (rmin, rmax) = ((0.004 * fw).max(1.5), 0.05 * fw);
        for start in 0..cw * ch {
            if seen.get(start).copied().unwrap_or(true) || !candidate(start) {
                continue;
            }
            let mut stack = vec![start];
            if let Some(s) = seen.get_mut(start) {
                *s = true;
            }
            let (mut n, mut sx, mut sy, mut peak) = (0usize, 0.0f64, 0.0f64, 0.0f32);
            let (mut bx0, mut bx1, mut by0, mut by1) = (cw, 0, ch, 0);
            while let Some(i) = stack.pop() {
                let (xx, yy) = (i % cw, i / cw);
                n += 1;
                sx += xx as f64;
                sy += yy as f64;
                peak = peak.max(resp.get(i).copied().unwrap_or(0.0));
                (bx0, bx1, by0, by1) = (bx0.min(xx), bx1.max(xx), by0.min(yy), by1.max(yy));
                if n as f64 > rmax * rmax * 4.0 {
                    break; // too big to be a blemish: a shadow or a feature
                }
                for j in [i.wrapping_sub(1), i + 1, i.wrapping_sub(cw), i + cw] {
                    let ok = j < cw * ch && !seen.get(j).copied().unwrap_or(true) && (j % cw).abs_diff(xx) <= 1;
                    if ok && candidate(j) {
                        if let Some(s) = seen.get_mut(j) {
                            *s = true;
                        }
                        stack.push(j);
                    }
                }
            }
            let (bw, bh) = ((bx1 - bx0 + 1) as f64, (by1 - by0 + 1) as f64);
            let r = bw.max(bh) / 2.0;
            let round = bw.max(bh) / bw.min(bh) < 3.0 && n as f64 >= 0.35 * bw * bh && n >= 3;
            if r >= rmin && r <= rmax && round {
                found.push(Blemish {
                    face: fi,
                    x: (x0 as f64 + sx / n as f64 + 0.5) / w as f64,
                    y: (y0 as f64 + sy / n as f64 + 0.5) / h as f64,
                    radius: (r / long * 1.6).max(fw * 0.012 / long),
                    strength: peak,
                });
            }
        }
        if found.len() > MAX_CANDIDATES_PER_FACE {
            continue; // texture, not blemishes
        }
        found.sort_by(|a, b| b.strength.total_cmp(&a.strength));
        found.truncate(MAX_PER_FACE);
        out.extend(found);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grey frame with one skin-coloured face ellipse `fw` wide at the centre, landmarks where
    /// a face has them, and `dots` dark spots on the cheeks.
    fn scene(w: usize, h: usize, fw: f64, dots: &[(f64, f64)]) -> (Rgba8, FaceBox) {
        let (cx, cy, fh) = (w as f64 / 2.0, h as f64 / 2.0, fw * 1.3);
        let mut img = Rgba8::filled(w, h, [120, 120, 120, 255]);
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                if ((px - cx) / (fw * 0.5)).powi(2) + ((py - cy) / (fh * 0.5)).powi(2) <= 1.0 {
                    img.data[y * w + x] = [222, 180, 160, 255]; // skin
                }
            }
        }
        for &(dx, dy) in dots {
            let (sx, sy) = (cx + dx * fw, cy + dy * fh);
            for y in 0..h {
                for x in 0..w {
                    if (x as f64 + 0.5 - sx).powi(2) + (y as f64 + 0.5 - sy).powi(2) <= (0.012 * fw).max(1.5).powi(2) {
                        img.data[y * w + x] = [150, 90, 85, 255]; // a dark red spot
                    }
                }
            }
        }
        let n = |x: f64, y: f64| (x / w as f64, y / h as f64);
        let face = FaceBox {
            x0: (cx - fw / 2.0) / w as f64,
            y0: (cy - fh / 2.0) / h as f64,
            x1: (cx + fw / 2.0) / w as f64,
            y1: (cy + fh / 2.0) / h as f64,
            landmarks: [
                n(cx - fw * 0.2, cy - fh * 0.12),
                n(cx + fw * 0.2, cy - fh * 0.12),
                n(cx, cy + fh * 0.08),
                n(cx - fw * 0.15, cy + fh * 0.28),
                n(cx + fw * 0.15, cy + fh * 0.28),
            ],
        };
        (img, face)
    }

    #[test]
    fn finds_spots_on_the_cheeks_and_nothing_else() {
        let dots = [(-0.28, 0.08), (0.3, 0.12), (-0.2, 0.3)];
        let (img, face) = scene(800, 600, 300.0, &dots);
        let found = detect(&img, &[face], 50.0);
        assert_eq!(found.len(), 3, "{found:?}");
        for (dx, dy) in dots {
            let (ex, ey) = (0.5 + dx * 300.0 / 800.0, 0.5 + dy * 390.0 / 600.0);
            assert!(found.iter().any(|b| (b.x - ex).abs() < 0.01 && (b.y - ey).abs() < 0.01), "{found:?} lacks {ex},{ey}");
        }
        assert!(found.iter().all(|b| b.face == 0 && b.radius > 0.0 && b.radius < 0.02 && b.strength > 0.0));
        let (clean, face) = scene(800, 600, 300.0, &[]);
        assert!(detect(&clean, &[face], 50.0).is_empty(), "a clean face has no blemishes");
    }

    #[test]
    fn features_and_small_faces_are_left_alone() {
        // spots on the eyes, the nose tip and the mouth: all inside the cut-outs
        let (img, face) = scene(800, 600, 300.0, &[(-0.2, -0.12), (0.2, -0.12), (0.0, 0.1), (0.0, 0.28)]);
        assert!(detect(&img, &[face], 80.0).is_empty(), "{:?}", detect(&img, &[face], 80.0));
        // the same cheek spots on a face below the size floor
        let (img, face) = scene(800, 600, 120.0, &[(-0.28, 0.08), (0.3, 0.12)]);
        assert!(detect(&img, &[face], 50.0).is_empty());
    }

    #[test]
    fn a_speckled_face_is_texture_not_blemishes() {
        // 24 spots on the cheeks, all outside the eye, nose and mouth cut-outs
        let dots: Vec<(f64, f64)> =
            [-0.42, -0.32, -0.22, 0.22, 0.32, 0.42].iter().flat_map(|&dx| (0..4).map(move |j| (dx, 0.0 + 0.08 * j as f64))).collect();
        let (img, face) = scene(800, 600, 300.0, &dots);
        assert!(detect(&img, &[face], 50.0).is_empty(), "more than {MAX_CANDIDATES_PER_FACE} candidates means stubble or freckles");
    }

    #[test]
    fn hostile_faces_do_not_panic() {
        let (img, _) = scene(64, 48, 20.0, &[]);
        let weird = FaceBox { x0: -2.0, y0: 0.5, x1: 9.0, y1: f64::NAN, landmarks: [(f64::INFINITY, 0.0); 5] };
        let inverted = FaceBox { x0: 0.9, y0: 0.9, x1: 0.1, y1: 0.1, landmarks: [(0.5, 0.5); 5] };
        assert!(detect(&img, &[weird, inverted], 100.0).is_empty());
        assert!(detect(&Rgba8::new(0, 0), &[inverted], 50.0).is_empty());
    }
}
