//! The container's transformative properties (`clap` crop, `irot` rotation, `imir` mirror),
//! applied to interleaved 16-bit pixels in the order the file associates them.
//!
//! `imir` follows libheif, which writes and reads `axis = 0` as a flip about the horizontal axis
//! (top and bottom exchanged) and `axis = 1` as a left-right flip. heic-rs 0.1.1 names the coded
//! values the other way round (`Mirror::LeftRight` is `axis = 0`), so they are swapped here.

use heic_rs::props::Transform;
use heic_rs::props::simple::{Clap, Mirror, Rotation};

use crate::Error;

/// Interleaved samples, `ch` per pixel, row-major.
pub struct Pixels {
    pub width: usize,
    pub height: usize,
    pub ch: usize,
    pub data: Vec<u16>,
}

impl Pixels {
    fn remap(&self, width: usize, height: usize, src: impl Fn(usize, usize) -> (usize, usize)) -> Pixels {
        let ch = self.ch;
        let mut data = vec![0u16; width * height * ch];
        for (y, row) in data.chunks_exact_mut(width * ch).enumerate() {
            for (x, px) in row.chunks_exact_mut(ch).enumerate() {
                let (sx, sy) = src(x, y);
                let i = (sy * self.width + sx) * ch;
                if let Some(s) = self.data.get(i..i + ch) {
                    px.copy_from_slice(s);
                }
            }
        }
        Pixels { width, height, ch, data }
    }
}

/// The rectangle `(left, top, width, height)` a `clap` selects from a `w` × `h` image. Its centre
/// is `((w - 1) / 2 + horizOff, (h - 1) / 2 + vertOff)`; a corner that falls between two pixels
/// is rounded as libheif does: left down, top up.
pub fn crop_rect(w: usize, h: usize, c: Clap) -> Result<(usize, usize, usize, usize), Error> {
    let bad = || Error::Malformed("clap selects an impossible rectangle".into());
    let size = |(n, d): (u32, u32)| (d != 0).then(|| n / d).ok_or_else(bad);
    let (cw, ch) = (size(c.width)? as usize, size(c.height)? as usize);
    if cw == 0 || ch == 0 || cw > w || ch > h {
        return Err(bad());
    }
    // Twice the corner, so the half pixels of odd sizes stay exact.
    let twice = |full: usize, cropped: usize, (n, d): (i32, u32)| -> Result<i64, Error> {
        if d == 0 {
            return Err(bad());
        }
        Ok((full - cropped) as i64 + (2 * i64::from(n)).div_euclid(i64::from(d)))
    };
    let left = twice(w, cw, c.horiz_off)?.div_euclid(2);
    let top = (twice(h, ch, c.vert_off)? + 1).div_euclid(2);
    let left = usize::try_from(left).map_err(|_| bad())?;
    let top = usize::try_from(top).map_err(|_| bad())?;
    if left + cw > w || top + ch > h {
        return Err(bad());
    }
    Ok((left, top, cw, ch))
}

/// Whether a `clap` among `transforms` (applied to a `w` × `h` image) starts at an odd column
/// or row. libheif crops such images after upsampling their chroma bilinearly instead of with
/// its default nearest neighbour; [`crate::ycc`] follows it so the pixels match.
pub fn odd_crop(mut w: usize, mut h: usize, transforms: &[Transform]) -> Result<bool, Error> {
    for t in transforms {
        match *t {
            Transform::Crop(c) => {
                let (left, top, cw, ch) = crop_rect(w, h, c)?;
                if left % 2 == 1 || top % 2 == 1 {
                    return Ok(true);
                }
                (w, h) = (cw, ch);
            }
            Transform::Rotate(Rotation::Ccw90 | Rotation::Ccw270) => (w, h) = (h, w),
            _ => {}
        }
    }
    Ok(false)
}

pub fn apply(mut img: Pixels, transforms: &[Transform]) -> Result<Pixels, Error> {
    for t in transforms {
        let (w, h) = (img.width, img.height);
        img = match *t {
            Transform::Crop(c) => {
                let (left, top, cw, ch) = crop_rect(w, h, c)?;
                img.remap(cw, ch, |x, y| (x + left, y + top))
            }
            // Counter-clockwise: the right column becomes the top row.
            Transform::Rotate(Rotation::Ccw90) => img.remap(h, w, |x, y| (w - 1 - y, x)),
            Transform::Rotate(Rotation::Ccw180) => img.remap(w, h, |x, y| (w - 1 - x, h - 1 - y)),
            Transform::Rotate(Rotation::Ccw270) => img.remap(h, w, |x, y| (y, h - 1 - x)),
            Transform::Rotate(Rotation::None) => img,
            // `axis = 0` (heic-rs's `LeftRight`): top and bottom exchanged, as libheif reads it.
            Transform::Mirror(Mirror::LeftRight) => img.remap(w, h, |x, y| (x, h - 1 - y)),
            // `axis = 1`: left and right exchanged.
            Transform::Mirror(Mirror::TopBottom) => img.remap(w, h, |x, y| (w - 1 - x, y)),
        };
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3×2, one channel: 1 2 3 / 4 5 6.
    fn img() -> Pixels {
        Pixels { width: 3, height: 2, ch: 1, data: vec![1, 2, 3, 4, 5, 6] }
    }

    fn run(t: Transform) -> (usize, usize, Vec<u16>) {
        let p = apply(img(), &[t]).unwrap();
        (p.width, p.height, p.data)
    }

    #[test]
    fn rotations_are_counter_clockwise() {
        assert_eq!(run(Transform::Rotate(Rotation::Ccw90)), (2, 3, vec![3, 6, 2, 5, 1, 4]));
        assert_eq!(run(Transform::Rotate(Rotation::Ccw180)), (3, 2, vec![6, 5, 4, 3, 2, 1]));
        assert_eq!(run(Transform::Rotate(Rotation::Ccw270)), (2, 3, vec![4, 1, 5, 2, 6, 3]));
        assert_eq!(run(Transform::Rotate(Rotation::None)), (3, 2, vec![1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn mirrors_follow_libheif() {
        // axis 0 (heic-rs `LeftRight`) exchanges top and bottom; axis 1 left and right.
        assert_eq!(run(Transform::Mirror(Mirror::LeftRight)), (3, 2, vec![4, 5, 6, 1, 2, 3]));
        assert_eq!(run(Transform::Mirror(Mirror::TopBottom)), (3, 2, vec![3, 2, 1, 6, 5, 4]));
    }

    fn clap(w: u32, h: u32, ho: i32, vo: i32) -> Clap {
        Clap { width: (w, 1), height: (h, 1), horiz_off: (ho, 1), vert_off: (vo, 1) }
    }

    #[test]
    fn clap_corners_round_like_libheif() {
        // Measured against libheif 1.23 on a 512 × 512 picture.
        assert_eq!(crop_rect(512, 512, clap(301, 201, 0, 0)).unwrap(), (105, 156, 301, 201));
        assert_eq!(crop_rect(512, 512, clap(300, 200, 0, 0)).unwrap(), (106, 156, 300, 200));
        assert_eq!(crop_rect(512, 512, clap(301, 201, 37, -21)).unwrap(), (142, 135, 301, 201));
        assert_eq!(crop_rect(512, 512, clap(301, 201, -3, 5)).unwrap(), (102, 161, 301, 201));
        assert_eq!(crop_rect(512, 512, clap(300, 201, 0, 0)).unwrap(), (106, 156, 300, 201));
        assert_eq!(crop_rect(1500, 1000, clap(1001, 701, 37, -21)).unwrap(), (286, 129, 1001, 701));
        let p = apply(img(), &[Transform::Crop(clap(2, 1, 1, 0))]).unwrap();
        // The half-pixel top rounds down a row.
        assert_eq!((p.width, p.height, p.data), (2, 1, vec![5, 6]));
    }

    #[test]
    fn impossible_claps_are_errors() {
        for c in [
            clap(0, 1, 0, 0),
            clap(4, 1, 0, 0),
            clap(3, 2, 1, 0),
            clap(3, 2, 0, -1),
            Clap { width: (1, 0), ..clap(1, 1, 0, 0) },
            Clap { horiz_off: (0, 0), ..clap(1, 1, 0, 0) },
        ] {
            assert!(crop_rect(3, 2, c).is_err(), "{c:?}");
        }
        assert!(apply(img(), &[Transform::Crop(clap(9, 9, 0, 0))]).is_err());
    }

    #[test]
    fn odd_crops_are_detected_through_rotations() {
        assert!(!odd_crop(512, 512, &[Transform::Crop(clap(300, 200, 0, 0))]).unwrap());
        assert!(odd_crop(512, 512, &[Transform::Crop(clap(301, 200, 0, 0))]).unwrap());
        // After a quarter turn the crop applies to a 200 × 300 image.
        let t = [Transform::Crop(clap(300, 200, 0, 0)), Transform::Rotate(Rotation::Ccw90), Transform::Crop(clap(197, 300, 0, 0))];
        assert!(odd_crop(512, 512, &t).unwrap());
        assert!(!odd_crop(512, 512, &[Transform::Rotate(Rotation::Ccw90)]).unwrap());
    }
}
