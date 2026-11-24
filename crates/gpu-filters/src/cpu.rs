//! The filters on the CPU: the reference the compute passes are tested against, step for step the
//! rule the shaders follow, and the path a caller without a device takes.
//!
//! Every image is premultiplied RGBA8, `width × height` pixels, tightly packed, red first. A
//! colour channel above its pixel's alpha is read as the alpha.

use crate::spec::{self, Error, Filter, Lut};

/// `filter` applied to `inputs`: one image, or for [`Filter::Composite`] the backdrop and then
/// the layer.
pub fn apply(filter: &Filter, width: u32, height: u32, inputs: &[&[u8]]) -> Result<Vec<u8>, Error> {
    match (filter, inputs) {
        (Filter::GaussianBlur { sigma }, [src]) => gaussian_blur(src, width, height, *sigma),
        (Filter::Lut(lut), [src]) => lut_apply(src, width, height, lut),
        (Filter::Composite { opacity }, [backdrop, layer]) => composite(backdrop, layer, width, height, *opacity),
        _ => Err(Error::Inputs { expected: filter.inputs(), got: inputs.len() }),
    }
}

/// A premultiplied pixel with each colour channel held to its alpha.
fn pixel(src: &[u8], i: usize) -> [u8; 4] {
    let a = src[i * 4 + 3];
    [src[i * 4].min(a), src[i * 4 + 1].min(a), src[i * 4 + 2].min(a), a]
}

/// `x / 255` rounded to the nearest integer, as `(x + 127) / 255`.
fn div255(x: u32) -> u32 {
    (x + 127) / 255
}

/// A `[0, 1]` value as a 16-bit unorm code, as WGSL `pack2x16unorm` rounds it.
fn unorm16(v: f32) -> u16 {
    (0.5 + 65535.0 * v.clamp(0.0, 1.0)).floor() as u16
}

/// A `[0, 1]` value as an 8-bit unorm code, as WGSL `pack4x8unorm` rounds it.
fn unorm8(v: f32) -> u8 {
    (0.5 + 255.0 * v.clamp(0.0, 1.0)).floor() as u8
}

/// The weighted sum one blur pass makes at position `at` of a line of `len` values, where `get`
/// reads the value at an index and positions outside the line are 0. The sum runs from the centre
/// out, in the order the shader adds.
fn tap(weights: &[f32], at: usize, len: usize, get: impl Fn(usize) -> [f32; 4]) -> [f32; 4] {
    let centre = get(at);
    let mut acc = centre.map(|v| weights[0] * v);
    for (i, w) in weights.iter().enumerate().skip(1) {
        let mut pair = [0.0f32; 4];
        if at >= i {
            let v = get(at - i);
            pair = std::array::from_fn(|c| pair[c] + v[c]);
        }
        if at + i < len {
            let v = get(at + i);
            pair = std::array::from_fn(|c| pair[c] + v[c]);
        }
        acc = std::array::from_fn(|c| acc[c] + w * pair[c]);
    }
    acc
}

/// A separable Gaussian blur of `sigma` pixels (see [`Filter::GaussianBlur`] and
/// [`spec::kernel`]). The horizontal pass keeps 16 bits a channel; the vertical pass rounds to 8
/// bits and holds each colour channel to its alpha.
pub fn gaussian_blur(src: &[u8], width: u32, height: u32, sigma: f32) -> Result<Vec<u8>, Error> {
    let pixels = spec::check(&Filter::GaussianBlur { sigma }, width, height, &[src])? as usize;
    let weights = spec::kernel(sigma)?;
    let (w, h) = (width as usize, height as usize);
    let mut mid: Vec<[u16; 4]> = spec::zeroed(pixels)?;
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let acc = tap(&weights, x, w, |i| pixel(src, row + i).map(|v| f32::from(v) / 255.0));
            mid[row + x] = acc.map(unorm16);
        }
    }
    let mut out: Vec<u8> = spec::zeroed(pixels * 4)?;
    for y in 0..h {
        for x in 0..w {
            let acc = tap(&weights, y, h, |i| mid[i * w + x].map(|v| f32::from(v) / 65535.0));
            let [r, g, b, a] = acc.map(unorm8);
            let o = (y * w + x) * 4;
            out[o..o + 4].copy_from_slice(&[r.min(a), g.min(a), b.min(a), a]);
        }
    }
    Ok(out)
}

/// Per-channel lookup tables on straight colour, re-premultiplied (see [`Lut`]).
pub fn lut_apply(src: &[u8], width: u32, height: u32, lut: &Lut) -> Result<Vec<u8>, Error> {
    let pixels = spec::check(&Filter::Lut(Box::new(lut.clone())), width, height, &[src])? as usize;
    let mut out: Vec<u8> = spec::zeroed(pixels * 4)?;
    for i in 0..pixels {
        let p = pixel(src, i);
        let a = u32::from(p[3]);
        let a2 = u32::from(lut.tables[3][a as usize]);
        for ch in 0..3 {
            let c = u32::from(p[ch]);
            let straight = (c * 255 + a / 2).checked_div(a).unwrap_or(0);
            out[i * 4 + ch] = div255(u32::from(lut.tables[ch][straight as usize]) * a2) as u8;
        }
        out[i * 4 + 3] = a2 as u8;
    }
    Ok(out)
}

/// `layer` over `backdrop`, premultiplied source-over, with the layer scaled by `opacity` (see
/// [`Filter::Composite`]).
pub fn composite(backdrop: &[u8], layer: &[u8], width: u32, height: u32, opacity: f32) -> Result<Vec<u8>, Error> {
    let pixels = spec::check(&Filter::Composite { opacity }, width, height, &[backdrop, layer])? as usize;
    let o = spec::opacity_weight(opacity);
    let mut out: Vec<u8> = spec::zeroed(pixels * 4)?;
    for i in 0..pixels {
        let s = pixel(layer, i).map(|v| div255(u32::from(v) * o));
        let d = pixel(backdrop, i).map(u32::from);
        for ch in 0..4 {
            out[i * 4 + ch] = (s[ch] + div255(d[ch] * (255 - s[3]))) as u8;
        }
    }
    Ok(out)
}
