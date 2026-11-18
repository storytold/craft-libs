//! Checks against sources outside this crate: code values computed here in `f64` from the Kr and
//! Kb values and the quantization each ITU-R recommendation states, the `yuv` crate's conversion
//! of the same pictures, and the plane layouts the formats' public definitions give. None of these
//! checks reads `spec::Coefficients`.
//!
//! - BT.601-7 section 2.5, <https://www.itu.int/rec/R-REC-BT.601>: Kr 0.299, Kb 0.114.
//! - BT.709-6 items 3.2 to 3.5, <https://www.itu.int/rec/R-REC-BT.709>: Kr 0.2126, Kb 0.0722;
//!   limited range `INT[(219 E'Y + 16) 2^(n-8)]` and `INT[(224 E'C + 128) 2^(n-8)]`, where INT
//!   rounds a fraction of 0.5 up.
//! - BT.2020-2 tables 4 and 5, <https://www.itu.int/rec/R-REC-BT.2020>: Kr 0.2627, Kb 0.0593,
//!   quantized as in BT.709.
//! - BT.2100-3 table 9, <https://www.itu.int/rec/R-REC-BT.2100>: full range
//!   `Round[(2^n - 1) E'Y]` and `Round[(2^n - 1) E'C + 2^(n-1)]`, with
//!   `Round(x) = Sign(x) Floor(|x| + 0.5)`.
//! - NV12 and I420: the W3C WebCodecs `VideoPixelFormat` definitions,
//!   <https://www.w3.org/TR/webcodecs/#enumdef-videopixelformat>.
//! - NV12 and P010: the Linux media API's semi-planar YUV formats and their sample 4×4 images,
//!   <https://docs.kernel.org/userspace-api/media/v4l/pixfmt-yuv-planar.html>.

use super::*;

const ALL: [Format; 3] = [Format::Nv12, Format::P010, Format::I420];
const MATRICES: [Matrix; 3] = [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020];
const RANGES: [Range; 2] = [Range::Limited, Range::Full];

/// Kr and Kb as each recommendation states them.
fn kr_kb(m: Matrix) -> (f64, f64) {
    match m {
        Matrix::Bt601 => (0.299, 0.114),
        Matrix::Bt709 => (0.2126, 0.0722),
        Matrix::Bt2020 => (0.2627, 0.0593),
    }
}

/// The unrounded luma, Cb and Cr code values of an 8-bit RGB color.
fn exact(rgb: [f64; 3], c: Conversion) -> [f64; 3] {
    let (kr, kb) = kr_kb(c.matrix);
    let [r, g, b] = rgb.map(|v| v / 255.0);
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let pb = (b - y) / (2.0 * (1.0 - kb));
    let pr = (r - y) / (2.0 * (1.0 - kr));
    let n = c.format.bits() as i32;
    match c.range {
        Range::Limited => {
            let s = 2f64.powi(n - 8);
            [(219.0 * y + 16.0) * s, (224.0 * pb + 128.0) * s, (224.0 * pr + 128.0) * s]
        }
        Range::Full => {
            let m = 2f64.powi(n) - 1.0;
            let half = 2f64.powi(n - 1);
            [m * y, m * pb + half, m * pr + half]
        }
    }
}

/// `Round(x) = Sign(x) Floor(|x| + 0.5)`, clamped to the codes of `bits`.
fn round(x: f64, bits: u32) -> u16 {
    (x.signum() * (x.abs() + 0.5).floor()).clamp(0.0, f64::from((1u32 << bits) - 1)) as u16
}

/// Whether `got` is the rounded `exact` value. A value within 0.001 of a half may round either
/// way in `f32`, so either neighbor passes there.
fn rounds_to(got: u16, exact: f64, bits: u32) -> bool {
    let want = round(exact, bits);
    let near_half = ((exact - exact.floor()) - 0.5).abs() < 1e-3;
    got == want || near_half && got.abs_diff(want) <= 1
}

/// Every sample of a frame as its value (P010 words shifted down by 6).
fn samples(frame: &[u8], format: Format) -> Vec<u16> {
    match format {
        Format::P010 => frame.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b) >> 6).collect(),
        Format::Nv12 | Format::I420 => frame.iter().map(|&b| u16::from(b)).collect(),
    }
}

/// The luma sample of pixel `(x, y)` and the Cb and Cr samples of the chroma texel over it, in
/// a packed frame of `w × h`.
fn at(s: &[u16], format: Format, w: usize, h: usize, x: usize, y: usize) -> [u16; 3] {
    let luma = w * h;
    let (cw, ch) = (w / 2, h / 2);
    let c = (y / 2) * cw + x / 2;
    match format {
        Format::Nv12 | Format::P010 => [s[y * w + x], s[luma + 2 * c], s[luma + 2 * c + 1]],
        Format::I420 => [s[y * w + x], s[luma + c], s[luma + cw * ch + c]],
    }
}

/// Channel values that include black, white, the 75% level (191), the limited-range nominal
/// levels and values one step from the ends.
const LEVELS: [u8; 9] = [0, 1, 16, 64, 128, 191, 235, 254, 255];

/// A picture of every color whose channels are in [`LEVELS`], each as a 2 × 2 block, so that no
/// sample depends on how chroma is subsampled. Returns the pixels, the side and the colors.
fn blocks() -> (Vec<u8>, u32, Vec<[u8; 3]>) {
    let colors: Vec<[u8; 3]> = LEVELS.iter().flat_map(|&r| LEVELS.iter().flat_map(move |&g| LEVELS.iter().map(move |&b| [r, g, b]))).collect();
    let per_row = 27usize;
    let side = per_row * 2;
    let mut px = vec![0u8; side * side * 4];
    for (k, rgb) in colors.iter().enumerate() {
        let (bx, by) = (k % per_row, k / per_row);
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let i = ((by * 2 + dy) * side + bx * 2 + dx) * 4;
            px[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    (px, side as u32, colors)
}

/// Every block of `frame` against the equations. `slack` is 0 for the CPU and 1 for the GPU.
fn check_blocks(frame: &[u8], c: Conversion, side: u32, colors: &[[u8; 3]], slack: u16, what: &str) {
    let s = samples(frame, c.format);
    let side = side as usize;
    for (k, rgb) in colors.iter().enumerate() {
        let (x, y) = ((k % 27) * 2, (k / 27) * 2);
        let got = at(&s, c.format, side, side, x, y);
        let want = exact(rgb.map(f64::from), c);
        for i in 0..3 {
            let ok = rounds_to(got[i], want[i], c.format.bits()) || got[i].abs_diff(round(want[i], c.format.bits())) <= slack;
            assert!(ok, "{what} {c:?} {rgb:?}: component {i} is {} where the equations give {:.4}", got[i], want[i]);
        }
    }
}

#[test]
fn the_cpu_gives_the_codes_of_the_published_equations() {
    let (px, side, colors) = blocks();
    for format in ALL {
        for matrix in MATRICES {
            for range in RANGES {
                let c = Conversion::new(format, matrix, range);
                let frame = cpu::convert(&px, side, side, c, side, side).unwrap();
                check_blocks(&frame, c, side, &colors, 0, "CPU");
            }
        }
    }
}

#[test]
fn nominal_levels_have_the_codes_the_recommendations_state() {
    // White, black and neutral chroma are 235, 16 and 128 at limited range, 255, 0 and 128 at
    // full range, times 4 (and 1023 for full white) at 10 bits. Chroma reaches 240 (960) for blue's
    // Cb and red's Cr, and 16 (64) for yellow's Cb and cyan's Cr. At full range those are
    // Round(255.5) and Round(0.5), clamped to 255 (1023) and 1.
    for format in ALL {
        let ten = format.bits() == 10;
        for matrix in MATRICES {
            for range in RANGES {
                let c = Conversion::new(format, matrix, range);
                let (white, black, mid, high, low) = match (range, ten) {
                    (Range::Limited, false) => (235, 16, 128, 240, 16),
                    (Range::Limited, true) => (940, 64, 512, 960, 64),
                    (Range::Full, false) => (255, 0, 128, 255, 1),
                    (Range::Full, true) => (1023, 0, 512, 1023, 1),
                };
                let code = |rgb: [u8; 3]| {
                    let frame = cpu::convert(&(0..4).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect::<Vec<_>>(), 2, 2, c, 2, 2).unwrap();
                    at(&samples(&frame, format), format, 2, 2, 0, 0)
                };
                assert_eq!(code([255, 255, 255]), [white, mid, mid], "{c:?} white");
                assert_eq!(code([0, 0, 0]), [black, mid, mid], "{c:?} black");
                assert_eq!(code([0, 0, 255])[1], high, "{c:?} blue Cb");
                assert_eq!(code([255, 0, 0])[2], high, "{c:?} red Cr");
                assert_eq!(code([255, 255, 0])[1], low, "{c:?} yellow Cb");
                assert_eq!(code([0, 255, 255])[2], low, "{c:?} cyan Cr");
            }
        }
    }
}

fn yuv_range(r: Range) -> yuv::YuvRange {
    match r {
        Range::Limited => yuv::YuvRange::Limited,
        Range::Full => yuv::YuvRange::Full,
    }
}

fn yuv_matrix(m: Matrix) -> yuv::YuvStandardMatrix {
    match m {
        Matrix::Bt601 => yuv::YuvStandardMatrix::Bt601,
        Matrix::Bt709 => yuv::YuvStandardMatrix::Bt709,
        Matrix::Bt2020 => yuv::YuvStandardMatrix::Bt2020,
    }
}

/// The `yuv` crate's conversion of `px` (`w × h`) as one packed frame in this crate's order.
/// P010 takes 10-bit RGB there, so each channel is scaled to 0..=1023 and rounded first.
fn yuv_convert(px: &[u8], w: u32, h: u32, c: Conversion) -> Vec<u8> {
    use yuv::{YuvBiPlanarImageMut, YuvChromaSubsampling, YuvConversionMode, YuvPlanarImageMut};
    let (range, matrix, mode) = (yuv_range(c.range), yuv_matrix(c.matrix), YuvConversionMode::Professional);
    match c.format {
        Format::Nv12 => {
            let mut img = YuvBiPlanarImageMut::<u8>::alloc(w, h, YuvChromaSubsampling::Yuv420);
            yuv::rgba_to_yuv_nv12(&mut img, px, w * 4, range, matrix, mode).unwrap();
            [img.y_plane.borrow(), img.uv_plane.borrow()].concat()
        }
        Format::I420 => {
            let mut img = YuvPlanarImageMut::<u8>::alloc(w, h, YuvChromaSubsampling::Yuv420);
            yuv::rgba_to_yuv420(&mut img, px, w * 4, range, matrix, mode).unwrap();
            [img.y_plane.borrow(), img.u_plane.borrow(), img.v_plane.borrow()].concat()
        }
        Format::P010 => {
            let wide: Vec<u16> = px.iter().map(|&v| (f64::from(v) * 1023.0 / 255.0).round() as u16).collect();
            let mut img = YuvBiPlanarImageMut::<u16>::alloc(w, h, YuvChromaSubsampling::Yuv420);
            yuv::rgba10_to_p010(&mut img, &wide, w * 4, range, matrix).unwrap();
            [img.y_plane.borrow(), img.uv_plane.borrow()].concat().iter().flat_map(|v| v.to_le_bytes()).collect()
        }
    }
}

/// The largest difference allowed from the `yuv` crate: 1 code value at 8 bits, and 2 at 10 bits,
/// where its input was rounded to 10 bits first.
fn yuv_tolerance(format: Format) -> u16 {
    if format.bits() == 10 { 2 } else { 1 }
}

#[test]
fn the_cpu_matches_the_yuv_crate() {
    let (px, side, _) = blocks();
    for format in ALL {
        for matrix in MATRICES {
            for range in RANGES {
                let c = Conversion::new(format, matrix, range);
                let ours = samples(&cpu::convert(&px, side, side, c, side, side).unwrap(), format);
                let theirs = samples(&yuv_convert(&px, side, side, c), format);
                assert_eq!(ours.len(), theirs.len(), "{c:?}");
                for (i, (a, b)) in ours.iter().zip(&theirs).enumerate() {
                    assert!(a.abs_diff(*b) <= yuv_tolerance(format), "{c:?}: sample {i} is {a} here and {b} in the yuv crate");
                }
            }
        }
    }
}

#[test]
fn packed_planes_have_the_sizes_the_yuv_crate_allocates() {
    use yuv::{YuvBiPlanarImageMut, YuvChromaSubsampling, YuvPlanarImageMut};
    for (w, h) in [(2, 2), (4, 4), (16, 10), (1920, 1080), (3840, 2160)] {
        for format in ALL {
            let l = FrameLayout::new(format, w, h).unwrap();
            let s = format.sample_bytes() as usize;
            let packed: Vec<(usize, usize)> = l.planes.iter().map(|p| (p.row_bytes() as usize, p.row_bytes() as usize * p.height as usize)).collect();
            let theirs: Vec<(usize, usize)> = match format {
                Format::Nv12 | Format::P010 => {
                    let img = YuvBiPlanarImageMut::<u16>::alloc(w, h, YuvChromaSubsampling::Yuv420);
                    vec![(img.y_stride as usize * s, img.y_plane.borrow().len() * s), (img.uv_stride as usize * s, img.uv_plane.borrow().len() * s)]
                }
                Format::I420 => {
                    let img = YuvPlanarImageMut::<u8>::alloc(w, h, YuvChromaSubsampling::Yuv420);
                    vec![
                        (img.y_stride as usize, img.y_plane.borrow().len()),
                        (img.u_stride as usize, img.u_plane.borrow().len()),
                        (img.v_stride as usize, img.v_plane.borrow().len()),
                    ]
                }
            };
            assert_eq!(packed, theirs, "{format} {w}×{h}: row bytes and plane bytes");
            assert_eq!(l.frame_bytes(), theirs.iter().map(|p| p.1).sum::<usize>(), "{format} {w}×{h}");
        }
    }
}

/// Sixteen different colors for a 4 × 4 picture.
fn sixteen() -> Vec<[u8; 3]> {
    (0..16u32).map(|k| [(k * 17) as u8, ((k * 53 + 30) % 256) as u8, ((k * 91 + 60) % 256) as u8]).collect()
}

/// The byte offset of luma sample `(x, y)`.
type LumaAt = fn(usize, usize) -> usize;
/// The byte offsets of the Cb and Cr samples of chroma texel `(x, y)`.
type ChromaAt = fn(usize, usize) -> (usize, usize);

/// Where the public definitions put each sample of a 4 × 4 frame.
fn documented_offsets(format: Format) -> (LumaAt, ChromaAt) {
    match format {
        // "start + 0: Y'00 Y'01 Y'02 Y'03 ... start + 16: Cb00 Cr00 Cb01 Cr01, start + 20: Cb10 Cr10 Cb11 Cr11"
        Format::Nv12 => (|x, y| y * 4 + x, |x, y| (16 + y * 4 + x * 2, 17 + y * 4 + x * 2)),
        // Rows of 8 bytes: "start + 8: Y'10 ... start + 32: Cb00 Cr00 Cb01 Cr01, start + 40: Cb10 ..."
        Format::P010 => (|x, y| y * 8 + x * 2, |x, y| (32 + y * 8 + x * 4, 34 + y * 8 + x * 4)),
        // Y, then U, then V; each chroma plane has 2 rows of 2 samples.
        Format::I420 => (|x, y| y * 4 + x, |x, y| (16 + y * 2 + x, 20 + y * 2 + x)),
    }
}

#[test]
fn a_4x4_frame_puts_each_sample_where_the_format_definitions_do() {
    let colors = sixteen();
    let px: Vec<u8> = colors.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect();
    for format in ALL {
        let c = Conversion::new(format, Matrix::Bt709, Range::Limited);
        let frame = cpu::convert(&px, 4, 4, c, 4, 4).unwrap();
        assert_eq!(frame.len(), 4 * 4 * 3 / 2 * format.sample_bytes() as usize, "{format}");
        let read = |offset: usize| match format {
            Format::P010 => {
                let word = u16::from_le_bytes([frame[offset], frame[offset + 1]]);
                assert_eq!(word & 0x3f, 0, "{format}: the 6 low bits are zero");
                word >> 6
            }
            Format::Nv12 | Format::I420 => u16::from(frame[offset]),
        };
        let (luma, chroma) = documented_offsets(format);
        for y in 0..4 {
            for x in 0..4 {
                let want = exact(colors[y * 4 + x].map(f64::from), c)[0];
                assert!(rounds_to(read(luma(x, y)), want, format.bits()), "{format}: luma {x},{y}");
            }
        }
        for y in 0..2 {
            for x in 0..2 {
                // Each chroma sample is the code of the average of the 2 × 2 pixels under it.
                let quad = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| colors[(y * 2 + dy) * 4 + x * 2 + dx]);
                let mean = [0, 1, 2].map(|i| quad.iter().map(|p| f64::from(p[i])).sum::<f64>() / 4.0);
                let want = exact(mean, c);
                let (cb, cr) = chroma(x, y);
                assert!(rounds_to(read(cb), want[1], format.bits()), "{format}: Cb {x},{y}");
                assert!(rounds_to(read(cr), want[2], format.bits()), "{format}: Cr {x},{y}");
            }
        }
    }
}

#[test]
fn the_gpu_gives_the_codes_of_the_published_equations() {
    let Some((device, queue)) = tests::device() else { return };
    let (px, side, colors) = blocks();
    for format in ALL {
        for matrix in MATRICES {
            for range in RANGES {
                let c = Conversion::new(format, matrix, range);
                let frame = tests::gpu_convert(&device, &queue, (&px, side, side), c, (side, side));
                check_blocks(&frame, c, side, &colors, 1, "GPU");
            }
        }
    }
}
