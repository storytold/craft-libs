//! Rotation about the image centre at the image's own bit depth.

use crate::image::{Image, background};

/// Rotate the content `degrees` counter-clockwise as displayed about the image centre, into a
/// buffer of the same layout and size. Corners the rotation uncovers take the most common
/// border pixel, so a scan on tinted paper keeps its tint. Images of 8 bits or more are
/// resampled bilinearly; 1-, 2- and 4-bit and indexed images by nearest neighbour, because a
/// blend of two bilevel samples or two palette indices is not a sample of the same kind.
#[must_use]
pub fn rotate(image: &Image<'_>, degrees: f64) -> Vec<u8> {
    let fill = background(image);
    let mut out = vec![0u8; image.byte_len()];
    let (sin, cos) = if degrees.is_finite() { (-degrees).to_radians().sin_cos() } else { (0.0, 1.0) };
    let (cx, cy) = (f64::from(image.width()) / 2.0, f64::from(image.height()) / 2.0);
    for y in 0..image.height() {
        for x in 0..image.width() {
            // The source of an output pixel centre, rotated back.
            let u = f64::from(x) + 0.5 - cx;
            let v = f64::from(y) + 0.5 - cy;
            let sx = u * cos + v * sin + cx - 0.5;
            let sy = -u * sin + v * cos + cy - 0.5;
            for (c, &paper) in (0..image.components()).zip(&fill) {
                image.put(&mut out, x, y, c, sample_at(image, sx, sy, c, paper));
            }
        }
    }
    out
}

/// Component `c` at source position `(sx, sy)` in pixels, `paper` outside the image.
fn sample_at(image: &Image<'_>, sx: f64, sy: f64, c: u32, paper: u32) -> u32 {
    let (w, h) = (i64::from(image.width()), i64::from(image.height()));
    let at =
        |x: i64, y: i64| -> f64 { if x < 0 || y < 0 || x >= w || y >= h { f64::from(paper) } else { f64::from(image.get(x as u32, y as u32, c)) } };
    if !image.blends() {
        return at(sx.round() as i64, sy.round() as i64) as u32;
    }
    let (x0, y0) = (sx.floor(), sy.floor());
    let (fx, fy) = (sx - x0, sy - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
    let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
    (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, f64::from(image.max())) as u32
}
