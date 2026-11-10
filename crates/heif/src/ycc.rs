//! YCbCr → RGB for decoded HEVC pictures, done here rather than in heic-rs so the result matches
//! libheif (the reference every HEIF tool is checked against) to within one code value:
//!
//! - **Which matrix and range.** The item's `colr` `nclx` box when there is one; otherwise the
//!   HEVC VUI ([`crate::vui`]); otherwise the H.265 defaults (limited range). An unspecified matrix
//!   is BT.601, as libheif reads it. heic-rs 0.1.1 ignores the VUI and assumes BT.709 limited
//!   range for every file without `nclx`, which shifts every iPhone photo (full range, ICC only)
//!   by about 8 code values.
//! - **Chroma upsampling.** Nearest neighbour (each chroma sample covers its 2×2 or 2×1 luma
//!   block), as libheif does by default. libheif switches to a centred bilinear filter (weights
//!   9/3/3/1 sixteenths, in integers) when a `clap` crop starts at an odd column or row; so does
//!   [`Upsampling::Bilinear`].
//!
//! The arithmetic is floating point and the result 16-bit, whatever the coded depth, so the
//! develop pipeline gets every bit the conversion produces.

use heic_rs::grid::Mosaic;
use heic_rs::hevc::{ChromaFormat, Frame};

/// The coded pictures one image is made of.
pub enum Planes<'a> {
    Picture(&'a Frame),
    Grid(Mosaic<'a>),
}

impl Planes<'_> {
    pub fn size(&self) -> (usize, usize) {
        match self {
            Planes::Picture(f) => (f.width as usize, f.height as usize),
            Planes::Grid(m) => (m.width() as usize, m.height() as usize),
        }
    }

    pub fn bit_depth(&self) -> u8 {
        match self {
            Planes::Picture(f) => f.bit_depth,
            Planes::Grid(m) => m.bit_depth(),
        }
    }

    pub fn chroma(&self) -> ChromaFormat {
        match self {
            Planes::Picture(f) => f.chroma,
            Planes::Grid(m) => m.chroma(),
        }
    }

    fn chroma_width(&self) -> usize {
        let (w, h) = self.size();
        self.chroma().chroma_size(w as u32, h as u32).0 as usize
    }

    fn luma_row(&self, y: usize, dst: &mut [u16]) -> Option<()> {
        match self {
            Planes::Picture(f) => copy_row(&f.y, f.y_stride as usize, y, dst),
            Planes::Grid(m) => m.luma_row(y, dst),
        }
    }

    fn chroma_row(&self, cr: bool, r: usize, dst: &mut [u16]) -> Option<()> {
        match self {
            Planes::Picture(f) => copy_row(if cr { &f.cr } else { &f.cb }, f.c_stride as usize, r, dst),
            Planes::Grid(m) => m.chroma_row(cr, r, dst),
        }
    }
}

fn copy_row(plane: &[u16], stride: usize, y: usize, dst: &mut [u16]) -> Option<()> {
    let start = y.checked_mul(stride)?;
    dst.copy_from_slice(plane.get(start..start.checked_add(dst.len())?)?);
    Some(())
}

/// How the three coded channels map to R, G, B (H.273 `MatrixCoefficients`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Matrix {
    /// 0: the channels are G, B, R.
    Identity,
    /// 8: YCgCo.
    YCgCo,
    /// Every Kr/Kb matrix: R = Y + `cr_r`·Cr, G = Y − `cb_g`·Cb − `cr_g`·Cr, B = Y + `cb_b`·Cb.
    Kr { cr_r: f32, cb_g: f32, cr_g: f32, cb_b: f32 },
}

impl Matrix {
    /// Unknown, reserved and unspecified codes are BT.601, like libheif.
    fn from_code(code: u16) -> Matrix {
        let (kr, kb) = match code {
            0 => return Matrix::Identity,
            8 => return Matrix::YCgCo,
            1 => (0.2126, 0.0722),
            4 => (0.30, 0.11),
            7 => (0.212, 0.087),
            // 10 (BT.2020 constant luminance) is approximated by its non-constant matrix.
            9 | 10 => (0.2627, 0.0593),
            _ => (0.299, 0.114),
        };
        let kg = 1.0 - kr - kb;
        Matrix::Kr { cr_r: 2.0 * (1.0 - kr), cb_g: 2.0 * kb * (1.0 - kb) / kg, cr_g: 2.0 * kr * (1.0 - kr) / kg, cb_b: 2.0 * (1.0 - kb) }
    }
}

/// How the samples are to be read: the matrix and the range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signal {
    pub matrix: u16,
    pub full_range: bool,
}

/// How subsampled chroma is brought to the luma resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsampling {
    Nearest,
    /// Centred bilinear: chroma sits between its two luma columns (and rows for 4:2:0).
    Bilinear,
}

/// One chroma row brought to the full width `w` of luma row `y`.
fn chroma_at(planes: &Planes<'_>, cr: bool, y: usize, w: usize, mode: Upsampling, buf: &mut [u16], next: &mut [u16]) -> Option<Vec<u16>> {
    let chroma = planes.chroma();
    let (xs, ys) = (chroma.x_shift(), chroma.y_shift());
    let (wf, hf) = planes.size();
    let rows = chroma.chroma_size(wf as u32, hf as u32).1 as usize;
    let cy = y >> ys;
    planes.chroma_row(cr, cy, buf)?;
    let last = buf.len().checked_sub(1)?;
    if mode == Upsampling::Nearest || xs == 0 {
        return (0..w).map(|x| buf.get(x >> xs).copied()).collect();
    }
    // The nearer neighbour on each axis takes a quarter of the weight.
    let ny = if ys == 0 {
        cy
    } else if y.is_multiple_of(2) {
        cy.saturating_sub(1)
    } else {
        (cy + 1).min(rows.checked_sub(1)?)
    };
    planes.chroma_row(cr, ny, next)?;
    (0..w)
        .map(|x| {
            let cx = x >> 1;
            let nx = if x % 2 == 0 { cx.saturating_sub(1) } else { (cx + 1).min(last) };
            let (a, an) = (u32::from(*buf.get(cx)?), u32::from(*buf.get(nx)?));
            let v = if ys == 0 {
                (3 * a + an + 2) >> 2
            } else {
                let (b, bn) = (u32::from(*next.get(cx)?), u32::from(*next.get(nx)?));
                (9 * a + 3 * b + 3 * an + bn + 8) >> 4
            };
            u16::try_from(v).ok()
        })
        .collect()
}

/// Converts `planes` to interleaved 16-bit RGB, or RGBA with room left for alpha (filled with
/// 65535 here). `None` when the planes can't supply a row they promised (a malformed grid).
pub fn to_rgb16(planes: &Planes<'_>, signal: Signal, alpha: bool, mode: Upsampling) -> Option<Vec<u16>> {
    let (w, h) = planes.size();
    let ch = if alpha { 4 } else { 3 };
    let bd = u32::from(planes.bit_depth());
    if !(8..=16).contains(&bd) || w == 0 || h == 0 {
        return None;
    }
    // The default caller limit is 2²8 pixels, at most four channels per pixel, so the largest
    // legitimate `w·h·ch` is 2²30 samples. Anything more is a malformed or hostile canvas the
    // decode path would have refused before calling here (CodeRabbit #48): refuse rather than
    // ask for a multi-gigabuffer (or worse, over usize).
    if w.checked_mul(h)?.checked_mul(ch)? > 1 << 30 {
        return None;
    }
    let max = ((1u32 << bd) - 1) as f32;
    let half = (1u32 << (bd - 1)) as f32;
    let scale = (1u32 << (bd - 8)) as f32;
    // Normalised Y = (v − y_off)·y_mul, Cb/Cr = (v − half)·c_mul.
    let (y_off, y_mul, c_mul) =
        if signal.full_range { (0.0, 1.0 / max, 1.0 / max) } else { (16.0 * scale, 1.0 / (219.0 * scale), 1.0 / (224.0 * scale)) };
    let matrix = Matrix::from_code(signal.matrix);
    let mono = planes.chroma() == ChromaFormat::Monochrome;
    let cw = planes.chroma_width();

    let mut out = vec![0u16; w.checked_mul(h)?.checked_mul(ch)?];
    let row = |y: usize, dst: &mut [u16]| -> Option<()> {
        let mut luma = vec![0u16; w];
        planes.luma_row(y, &mut luma)?;
        let (cb, cr) = if mono {
            (Vec::new(), Vec::new())
        } else {
            let (mut buf, mut next) = (vec![0u16; cw], vec![0u16; cw]);
            (chroma_at(planes, false, y, w, mode, &mut buf, &mut next)?, chroma_at(planes, true, y, w, mode, &mut buf, &mut next)?)
        };
        for (x, (px, &yv)) in dst.chunks_exact_mut(ch).zip(&luma).enumerate() {
            let rgb = if mono {
                let v = (f32::from(yv) - y_off) * y_mul;
                [v, v, v]
            } else {
                let (u, v) = (f32::from(*cb.get(x)?), f32::from(*cr.get(x)?));
                match matrix {
                    // G, B, R all read like luma.
                    Matrix::Identity => [(v - y_off) * y_mul, (f32::from(yv) - y_off) * y_mul, (u - y_off) * y_mul],
                    Matrix::YCgCo => {
                        let (yn, cg, co) = ((f32::from(yv) - y_off) * y_mul, (u - half) * c_mul, (v - half) * c_mul);
                        let t = yn - cg;
                        [t + co, yn + cg, t - co]
                    }
                    Matrix::Kr { cr_r, cb_g, cr_g, cb_b } => {
                        let (yn, u, v) = ((f32::from(yv) - y_off) * y_mul, (u - half) * c_mul, (v - half) * c_mul);
                        [yn + cr_r * v, yn - cb_g * u - cr_g * v, yn + cb_b * u]
                    }
                }
            };
            for (o, c) in px.iter_mut().zip(rgb) {
                *o = (c.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
            }
            if alpha && let Some(a) = px.get_mut(3) {
                *a = u16::MAX;
            }
        }
        Some(())
    };
    let row_len = w * ch;
    #[cfg(not(target_arch = "wasm32"))]
    let ok = {
        use rayon::prelude::*;
        out.par_chunks_mut(row_len).enumerate().all(|(y, dst)| row(y, dst).is_some())
    };
    #[cfg(target_arch = "wasm32")]
    let ok = out.chunks_mut(row_len).enumerate().all(|(y, dst)| row(y, dst).is_some());
    ok.then_some(out)
}

/// Writes an alpha picture's luma, full range, into channel 3 of `rgba`.
pub fn put_alpha(planes: &Planes<'_>, rgba: &mut [u16]) -> Option<()> {
    let (w, h) = planes.size();
    let max = ((1u32 << u32::from(planes.bit_depth()).clamp(1, 16)) - 1) as f32;
    let mut luma = vec![0u16; w];
    for (y, row) in rgba.chunks_exact_mut(w * 4).enumerate().take(h) {
        planes.luma_row(y, &mut luma)?;
        for (px, &a) in row.as_chunks_mut::<4>().0.iter_mut().zip(&luma) {
            let v = (f32::from(a) / max).clamp(0.0, 1.0);
            px[3] = (v * 65535.0 + 0.5) as u16;
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, bd: u8, chroma: ChromaFormat, y: u16, cb: u16, cr: u16) -> Frame {
        let (cw, chh) = chroma.chroma_size(w, h);
        let n = (cw * chh) as usize;
        Frame {
            width: w,
            height: h,
            bit_depth: bd,
            chroma,
            y: vec![y; (w * h) as usize],
            cb: vec![cb; n],
            cr: vec![cr; n],
            y_stride: w,
            c_stride: cw,
        }
    }

    fn rgb(f: &Frame, matrix: u16, full_range: bool) -> [u16; 3] {
        let v = to_rgb16(&Planes::Picture(f), Signal { matrix, full_range }, false, Upsampling::Nearest).unwrap();
        [v[0], v[1], v[2]]
    }

    #[test]
    fn neutral_and_primaries_convert_exactly() {
        // Full-range grey stays grey; limited-range black and white reach the ends.
        assert_eq!(rgb(&frame(4, 4, 8, ChromaFormat::Yuv420, 128, 128, 128), 6, true), [128 * 257; 3]);
        assert_eq!(rgb(&frame(4, 4, 8, ChromaFormat::Yuv420, 16, 128, 128), 1, false), [0; 3]);
        assert_eq!(rgb(&frame(4, 4, 8, ChromaFormat::Yuv420, 235, 128, 128), 1, false), [65535; 3]);
        assert_eq!(rgb(&frame(4, 4, 10, ChromaFormat::Yuv420, 940, 512, 512), 9, false), [65535; 3]);
        // BT.601 full-range pure red: Y = 76.245, Cb = 84.97, Cr = 255.5 → (255, 0, 0) within rounding.
        let red = rgb(&frame(2, 2, 8, ChromaFormat::Yuv444, 76, 85, 255), 6, true);
        assert!(red[0] > 65000 && red[1] < 300 && red[2] < 300, "{red:?}");
        // Identity: the channels are G, B, R.
        assert_eq!(rgb(&frame(2, 2, 8, ChromaFormat::Yuv444, 10, 20, 30), 0, true), [30 * 257, 10 * 257, 20 * 257]);
        // Monochrome.
        assert_eq!(rgb(&frame(2, 2, 8, ChromaFormat::Monochrome, 51, 0, 0), 6, true), [51 * 257; 3]);
    }

    #[test]
    fn chroma_is_nearest_neighbour() {
        // 4:2:0, 4×2 luma, chroma 2×1: the left chroma sample covers columns 0–1 only.
        let mut f = frame(4, 2, 8, ChromaFormat::Yuv420, 128, 128, 128);
        f.cr = vec![200, 128];
        let v = to_rgb16(&Planes::Picture(&f), Signal { matrix: 6, full_range: true }, false, Upsampling::Nearest).unwrap();
        let red: Vec<u16> = v.chunks(3).map(|p| p[0]).collect();
        assert!(red[0] == red[1] && red[1] == red[4] && red[0] > red[2] && red[2] == 128 * 257, "{red:?}");
    }

    #[test]
    fn bilinear_chroma_is_centred_9_3_3_1() {
        // 4:2:0, 4×4 luma, chroma 2×2 with one bright corner sample.
        let mut f = frame(4, 4, 8, ChromaFormat::Yuv420, 128, 128, 128);
        f.cr = vec![200, 128, 128, 128];
        let p = Planes::Picture(&f);
        let (mut buf, mut next) = (vec![0; 2], vec![0; 2]);
        let rows: Vec<Vec<u16>> = (0..4).map(|y| chroma_at(&p, true, y, 4, Upsampling::Bilinear, &mut buf, &mut next).unwrap()).collect();
        // Row 0 sits nearest chroma row 0 (and clamps upward); row 1 blends a quarter of row 1.
        assert_eq!(rows[0], [200, 182, 146, 128]);
        assert_eq!(rows[1], [182, 169, 142, 128]);
        assert_eq!(rows[2], [146, 142, 133, 128]);
        assert_eq!(rows[3], [128; 4]);
        assert_eq!(chroma_at(&p, true, 1, 4, Upsampling::Nearest, &mut buf, &mut next).unwrap(), [200, 200, 128, 128]);
    }

    #[test]
    fn alpha_and_bad_planes() {
        let f = frame(2, 2, 8, ChromaFormat::Yuv420, 128, 128, 128);
        let mut v = to_rgb16(&Planes::Picture(&f), Signal { matrix: 6, full_range: true }, true, Upsampling::Nearest).unwrap();
        assert_eq!(v[3], 65535);
        put_alpha(&Planes::Picture(&frame(2, 2, 8, ChromaFormat::Monochrome, 51, 0, 0)), &mut v).unwrap();
        assert_eq!(v[3], 51 * 257);
        let mut short = frame(4, 4, 8, ChromaFormat::Yuv420, 128, 128, 128);
        short.y.truncate(5);
        assert!(to_rgb16(&Planes::Picture(&short), Signal { matrix: 6, full_range: true }, false, Upsampling::Bilinear).is_none());
        assert!(
            to_rgb16(
                &Planes::Picture(&frame(2, 2, 7, ChromaFormat::Yuv420, 0, 0, 0)),
                Signal { matrix: 6, full_range: true },
                false,
                Upsampling::Nearest
            )
            .is_none()
        );
    }

    /// CodeRabbit #48: a plane claiming 500 000 × 500 000 pixels must be refused without ever
    /// allocating for it (before the cap this vec was never built; upper-bounding allocation
    /// would have been a 7.5-gibabuffer or a panic instead of an error).
    #[test]
    fn an_oversized_canvas_is_refused_without_allocation() {
        let f = Frame {
            width: 500_000,
            height: 500_000,
            bit_depth: 8,
            chroma: ChromaFormat::Yuv420,
            y: vec![128],
            cb: vec![128; 4],
            cr: vec![128; 4],
            y_stride: 0,
            c_stride: 0,
        };
        assert!(to_rgb16(&Planes::Picture(&f), Signal { matrix: 6, full_range: true }, false, Upsampling::Nearest).is_none());
    }
}
