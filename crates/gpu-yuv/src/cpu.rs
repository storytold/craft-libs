//! The conversion on the CPU, sample for sample the rule the GPU passes follow. Tests compare the
//! two, and a caller without a GPU can use it directly.
//!
//! Each output texel (luma at the output size, chroma at half of it) takes the area of the source
//! under it: up to 4 × 4 bilinear taps spread over the texel's footprint, 1 per axis while the
//! source is at most twice the plane's size, one more for each further 2:1 of reduction. The
//! average of the taps goes through the matrix. At the same size the chroma tap lands between
//! four source pixels, so each chroma sample is the average of the 2 × 2 pixels it covers.

use crate::layout::FrameLayout;
use crate::spec::{Coefficients, Conversion, Error};

/// The largest output side [`convert`] takes.
pub const MAX_SIDE: u32 = 16384;

/// The taps along one axis for a reduction of `r` (source size over plane size).
pub(crate) fn taps(r: f32) -> u32 {
    ((r * 0.5 - 0.001).ceil() as i32).clamp(1, 4) as u32
}

struct Image<'a> {
    rgba: &'a [u8],
    width: usize,
    height: usize,
}

impl Image<'_> {
    fn pixel(&self, x: i64, y: i64) -> [f32; 3] {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        let i = (y * self.width + x) * 4;
        match self.rgba.get(i..i + 3) {
            Some(p) => [f32::from(p[0]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[2]) / 255.0],
            None => [0.0; 3],
        }
    }

    /// The linear filter at `(x, y)` in pixels (pixel centers at .5), clamped to the edges.
    fn bilinear(&self, x: f32, y: f32) -> [f32; 3] {
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (xi, yi) = (x0 as i64, y0 as i64);
        let [a, b, c, d] = [self.pixel(xi, yi), self.pixel(xi + 1, yi), self.pixel(xi, yi + 1), self.pixel(xi + 1, yi + 1)];
        let mut out = [0.0; 3];
        for k in 0..3 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            out[k] = top + (bottom - top) * fy;
        }
        out
    }

    /// The average color under the texel centered at `pos` of a plane of `size` texels.
    fn area(&self, pos: [f32; 2], size: [f32; 2]) -> [f32; 3] {
        let s = [self.width as f32, self.height as f32];
        let r = [s[0] / size[0], s[1] / size[1]];
        let n = [taps(r[0]), taps(r[1])];
        let nf = [n[0] as f32, n[1] as f32];
        let step = [r[0] / nf[0], r[1] / nf[1]];
        let center = [pos[0] / size[0] * s[0], pos[1] / size[1] * s[1]];
        let mut sum = [0.0f32; 3];
        for j in 0..n[1] {
            for i in 0..n[0] {
                let px = center[0] + (i as f32 + 0.5 - nf[0] * 0.5) * step[0];
                let py = center[1] + (j as f32 + 0.5 - nf[1] * 0.5) * step[1];
                let c = self.bilinear(px, py);
                for k in 0..3 {
                    sum[k] += c[k];
                }
            }
        }
        let count = nf[0] * nf[1];
        [sum[0] / count, sum[1] / count, sum[2] / count]
    }
}

/// `rgba` (`src_width × src_height`, 4 bytes per pixel; alpha is ignored, so premultiplied
/// pixels convert as if over black) scaled to `width × height` and converted, as one tightly
/// packed frame in [`FrameLayout`] order.
pub fn convert(rgba: &[u8], src_width: u32, src_height: u32, conversion: Conversion, width: u32, height: u32) -> Result<Vec<u8>, Error> {
    let bytes = (src_width as usize).checked_mul(src_height as usize).and_then(|n| n.checked_mul(4));
    if src_width == 0 || src_height == 0 || bytes != Some(rgba.len()) {
        return Err(Error::Source { width: src_width, height: src_height, bytes: rgba.len() });
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(Error::TooLarge { width, height, limit: MAX_SIDE });
    }
    let layout = FrameLayout::new(conversion.format, width, height)?;
    let image = Image { rgba, width: src_width as usize, height: src_height as usize };
    let c = Coefficients::of(conversion);
    let (shift, wide) = (conversion.format.shift(), conversion.format.sample_bytes() == 2);
    let mut out = Vec::with_capacity(layout.frame_bytes());
    let mut put = |v: u16| {
        let v = v << shift;
        if wide {
            out.extend_from_slice(&v.to_le_bytes());
        } else {
            out.push(v as u8);
        }
    };
    let plane = |pw: u32, ph: u32, pick: &mut dyn FnMut([u16; 3])| {
        let size = [pw as f32, ph as f32];
        for j in 0..ph {
            for i in 0..pw {
                pick(c.codes(image.area([i as f32 + 0.5, j as f32 + 0.5], size)));
            }
        }
    };
    let (cw, ch) = (width / 2, height / 2);
    plane(width, height, &mut |v| put(v[0]));
    match conversion.format {
        crate::Format::Nv12 | crate::Format::P010 => plane(cw, ch, &mut |v| {
            put(v[1]);
            put(v[2]);
        }),
        crate::Format::I420 => {
            let mut cr = Vec::with_capacity(cw as usize * ch as usize);
            plane(cw, ch, &mut |v| {
                put(v[1]);
                cr.push(v[2]);
            });
            for v in cr {
                put(v);
            }
        }
    }
    Ok(out)
}
