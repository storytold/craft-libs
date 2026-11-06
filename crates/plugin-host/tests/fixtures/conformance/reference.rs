//! Plain Rust versions of the `kernel-posterize` and `kernel-blur` plugins, and a test image.
//! The conformance tests compare engine output to these, and the engine benchmark example checks its timed
//! runs against them. They are written from the kernel description (see the WAT headers), not from
//! the WAT code, so a mistake in a kernel shows up as a mismatch.
//!
//! Include with `#[path = "fixtures/conformance/reference.rs"] mod reference;` from a test file.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// A u8 RGBA image. Rows may carry padding after `w * 4` bytes.
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub pixels: Vec<u8>,
}

/// Deterministic noise with a forced first pixel, rows padded by `pad` bytes.
pub fn noise_image(w: u32, h: u32, pad: u32, seed: u64) -> Image {
    let stride = w * 4 + pad;
    let mut state = seed | 1;
    let mut pixels = vec![0u8; (stride * h) as usize];
    for y in 0..h {
        for x in 0..w * 4 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            pixels[(y * stride + x) as usize] = (state >> 24) as u8;
        }
    }
    if let Some(first) = pixels.get_mut(0..4) {
        first.copy_from_slice(&[0, 255, 128, 255]);
    }
    Image { w, h, stride, pixels }
}

/// Each color byte becomes `((v * levels) >> 8) * 255 / (levels - 1)`. Alpha is copied. Padding stays 0.
pub fn posterize_reference(img: &Image, levels: u32) -> Vec<u8> {
    let mut out = vec![0u8; img.pixels.len()];
    let lut: Vec<u8> = (0..256u32).map(|v| (((v * levels) >> 8) * 255 / (levels - 1)) as u8).collect();
    for y in 0..img.h {
        for x in 0..img.w {
            let i = (y * img.stride + x * 4) as usize;
            for c in 0..3 {
                out[i + c] = lut[img.pixels[i + c] as usize];
            }
            out[i + 3] = img.pixels[i + 3];
        }
    }
    out
}

/// A 9x9 box blur as two 9 tap passes with edge clamping and integer division by 9 after each pass.
/// Direct window sums, no sliding window.
pub fn blur_reference(img: &Image) -> Vec<u8> {
    let (w, h) = (img.w as i64, img.h as i64);
    let at = |x: i64, y: i64, c: usize| img.pixels[(y * i64::from(img.stride) + x * 4) as usize + c] as u32;
    let mut tmp = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let sum: u32 = (-4..=4).map(|k| at((x + k).clamp(0, w - 1), y, c)).sum();
                tmp[((y * w + x) * 4) as usize + c] = (sum / 9) as u8;
            }
        }
    }
    let mut out = vec![0u8; img.pixels.len()];
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let sum: u32 = (-4..=4).map(|k| u32::from(tmp[(((y + k).clamp(0, h - 1) * w + x) * 4) as usize + c])).sum();
                out[(y * i64::from(img.stride) + x * 4) as usize + c] = (sum / 9) as u8;
            }
        }
    }
    out
}
