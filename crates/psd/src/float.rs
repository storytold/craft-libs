//! Float pixels: the merged image and single layers as straight RGBA `f32` in 0..1, for
//! readers that composite in floating point.
//!
//! Every colour mode and depth comes out display-encoded RGB: CMYK is converted naively
//! (`channel × black`, without a profile), indexed colour goes through the palette, Lab through
//! D50 to sRGB, and 32-bit documents, which are linear, are encoded to sRGB. A layer's raster
//! mask is applied to its alpha, and a solid colour fill layer without pixels is filled.
//!
//! Sizes read from a file are untrusted. One image holds at most [`MAX_DECODE_PIXELS`], and a
//! decode works out everything it will hold at once (the result, the CMYK black plane, the
//! decoded planes, the mask) and refuses with [`PsdError::LimitExceeded`] before allocating
//! when that is over its budget ([`DECODE_BUDGET`], or the limit passed to the `_within`
//! functions). Came from EffectCraft's `effectcraft-psd`.

use crate::error::{PsdError, Result};
use crate::file::PsdFile;
use crate::header::{ColorMode, row_bytes};
use crate::layer::{CHANNEL_TRANSPARENCY, CHANNEL_USER_MASK, LayerMask, LayerRecord, Rect};
use crate::smart::{MAX_BAKE_PIXELS, SmartObject, size_for, square_to_quad};

/// The most pixels one decoded image (the composite, a layer or a mask) may have: 2^28, for
/// example 16 384 × 16 384, 4 GiB as float RGBA.
pub const MAX_DECODE_PIXELS: u64 = 1 << 28;

/// What one decode may hold in memory at once: 4 GiB, 1 GiB where pointers are 32 bits wide.
pub const DECODE_BUDGET: u64 = if cfg!(target_pointer_width = "32") { 1 << 30 } else { 4 << 30 };

/// The bake's rasterising budget, in bounding-box pixels visited over all mesh triangles, as a
/// multiple of the output pixel count. A sane warp visits about 2× the output; a hostile mesh
/// that makes every triangle cover the whole output would visit thousands of times more.
const BAKE_WORK_FACTOR: u64 = 8;
/// The smallest budget, so tiny outputs are never refused for rounding at the triangle edges.
const MIN_BAKE_WORK: u64 = 1 << 20;

/// Decoded pixels: straight (unpremultiplied) RGBA in 0..1, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct FloatImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width × height` pixels.
    pub data: Vec<[f32; 4]>,
}

/// `w × h` as a pixel count, or an error when it exceeds [`MAX_DECODE_PIXELS`].
fn pixel_count(w: usize, h: usize) -> Result<usize> {
    match (w as u64).checked_mul(h as u64) {
        Some(n) if n <= MAX_DECODE_PIXELS => Ok(n as usize),
        _ => Err(PsdError::LimitExceeded("image has more pixels than MAX_DECODE_PIXELS")),
    }
}

/// Refuses a decode whose buffers would together hold more than `limit` bytes.
fn within(limit: u64, needed: u64) -> Result<()> {
    if needed <= limit { Ok(()) } else { Err(PsdError::LimitExceeded("decoding needs more memory than the decode budget")) }
}

/// `n` copies of `value`, or an error when there is no memory for them.
fn filled<T: Clone>(n: usize, value: T) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| PsdError::OutOfMemory("not enough memory for the decoded pixels"))?;
    v.resize(n, value);
    Ok(v)
}

/// A 32-bit float sample made safe to use: NaN reads as 0, +inf as white (1.0, like the integer
/// depths' maximum), -inf as 0.
fn finite_sample(v: f32) -> f32 {
    if v.is_nan() || v == f32::NEG_INFINITY {
        0.0
    } else if v == f32::INFINITY {
        1.0
    } else {
        v
    }
}

/// Hands every sample of the plane `raw` (`w`×`h`, row-major, in order) to `f` as a float.
/// Samples a short `raw` does not have are not reported.
fn for_each_sample(depth: u16, raw: &[u8], w: usize, h: usize, mut f: impl FnMut(usize, f32)) {
    let n = w.saturating_mul(h);
    match depth {
        1 => {
            let rowb = w.div_ceil(8);
            for y in 0..h {
                for x in 0..w {
                    let Some(b) = raw.get(y * rowb + x / 8) else { return };
                    // 1 = black.
                    f(y * w + x, if b & (0x80 >> (x % 8)) != 0 { 0.0 } else { 1.0 });
                }
            }
        }
        16 => raw.as_chunks::<2>().0.iter().take(n).enumerate().for_each(|(i, c)| f(i, f32::from(u16::from_be_bytes(*c)) / 65535.0)),
        32 => raw.as_chunks::<4>().0.iter().take(n).enumerate().for_each(|(i, c)| f(i, finite_sample(f32::from_be_bytes(*c)))),
        _ => raw.iter().take(n).enumerate().for_each(|(i, &b)| f(i, f32::from(b) / 255.0)),
    }
}

/// The sRGB encoding of a linear value.
pub fn srgb_encode(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// CIE L*a*b* (D50; L* 0..100, a* and b* about −128..127) to sRGB (D65 through the
/// Bradford-adapted matrix), encoded and clamped to 0..1.
pub fn lab_to_srgb(l: f32, a: f32, b: f32) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let f = |t: f32| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0) };
    let (x, y, z) = (0.964_22 * f(fx), f(fy), 0.825_21 * f(fz));
    // XYZ (D50) to linear sRGB.
    let r = 3.133_856 * x - 1.616_867 * y - 0.490_615 * z;
    let g = -0.978_768 * x + 1.916_142 * y + 0.033_454 * z;
    let bb = 0.071_945 * x - 0.228_991 * y + 1.405_243 * z;
    [r, g, bb].map(|v| srgb_encode(v.clamp(0.0, 1.0)))
}

/// What turns a document's channel values into RGB.
struct ToRgb<'a> {
    mode: ColorMode,
    depth: u16,
    palette: &'a [u8],
}

impl ToRgb<'_> {
    fn of(file: &PsdFile) -> ToRgb<'_> {
        ToRgb { mode: file.header.color_mode, depth: file.header.depth, palette: &file.color_mode_data }
    }

    /// Colour channels before alpha; a mode without a fixed count shows its first channel.
    fn color_channels(&self) -> usize {
        self.mode.color_channels().map_or(1, usize::from)
    }

    /// Colour values (the mode's channels in 0..1, missing ones 0) to RGB.
    fn rgb(&self, v: [f32; 4]) -> [f32; 3] {
        let rgb = match self.mode {
            ColorMode::Rgb => [v[0], v[1], v[2]],
            // Stored inverted (1 = no ink).
            ColorMode::Cmyk => [v[0] * v[3], v[1] * v[3], v[2] * v[3]],
            ColorMode::Indexed if self.palette.len() >= 768 => {
                let idx = (v[0] * 255.0).round() as usize & 255;
                let p = |k: usize| self.palette.get(k * 256 + idx).map_or(0.0, |&c| f32::from(c) / 255.0);
                [p(0), p(1), p(2)]
            }
            ColorMode::Lab => lab_to_srgb(v[0] * 100.0, v[1] * 255.0 - 128.0, v[2] * 255.0 - 128.0),
            _ => [v[0]; 3],
        };
        if self.depth == 32 && self.mode != ColorMode::Lab { rgb.map(srgb_encode) } else { rgb }
    }
}

/// Where the source rows or columns `s0..s0+n` land in an output starting at `o0` with `on`
/// entries: the source range that is inside, and the offset to subtract (64-bit: file bounds can
/// span the whole `i32` range).
fn span(s0: i32, n: usize, o0: i32, on: usize) -> (std::ops::Range<usize>, i64) {
    let off = i64::from(o0) - i64::from(s0);
    let lo = off.clamp(0, n as i64);
    let hi = (off + on as i64).clamp(lo, n as i64);
    (lo as usize..hi as usize, off)
}

/// The geometry of one layer decode: the output rectangle and the source rectangle inside it.
struct Placement {
    ow: usize,
    oh: usize,
    out_rect: Rect,
    src: Rect,
    sw: usize,
    sh: usize,
}

impl Placement {
    /// Calls `f(output index, source index)` for every source pixel that lands in the output.
    fn for_each(&self, mut f: impl FnMut(usize, usize)) {
        let (ys, oy) = span(self.src.top, self.sh, self.out_rect.top, self.oh);
        let (xs, ox) = span(self.src.left, self.sw, self.out_rect.left, self.ow);
        for y in ys {
            for x in xs.clone() {
                f((y as i64 - oy) as usize * self.ow + (x as i64 - ox) as usize, y * self.sw + x);
            }
        }
    }
}

fn size_of(r: Rect) -> Result<(usize, usize)> {
    if r.is_empty() { Ok((0, 0)) } else { r.size() }
}

impl PsdFile {
    /// The merged (flattened) image. Alpha is the first extra channel when
    /// [`PsdFile::merged_has_alpha`]; an RGB composite is stored matted against white and has
    /// the matte removed, as [`PsdFile::composite_rgba8`] does.
    pub fn composite_f32(&self) -> Result<FloatImage> {
        self.composite_f32_within(DECODE_BUDGET)
    }

    /// [`Self::composite_f32`] holding at most `limit` bytes at once: the result (16 bytes a
    /// pixel), the CMYK black plane and the decoded planes of every channel.
    pub fn composite_f32_within(&self, limit: u64) -> Result<FloatImage> {
        let h = &self.header;
        h.validate()?;
        let (w, hh) = (h.width as usize, h.height as usize);
        let n = pixel_count(w, hh)?;
        let to_rgb = ToRgb::of(self);
        let cc = to_rgb.color_channels();
        let nch = usize::from(h.channels);
        let alpha = self.merged_has_alpha();
        let want = (cc + usize::from(alpha)).min(nch);
        let plane = h.row_bytes().checked_mul(hh).ok_or(PsdError::LimitExceeded("plane size overflow"))?;
        let cmyk = h.color_mode == ColorMode::Cmyk && want > 3;
        let black_bytes = if cmyk { 4 * n as u64 } else { 0 };
        within(limit, (16 * n as u64).saturating_add(black_bytes).saturating_add((plane as u64).saturating_mul(nch as u64)))?;
        let all = self.decode_merged()?;
        // Opaque black until a channel says otherwise.
        let mut data = filled(n, [0.0f32, 0.0, 0.0, 1.0])?;
        let mut black = if cmyk { filled(n, 0.0f32)? } else { Vec::new() };
        for c in 0..want {
            let Some(raw) = all.get(c * plane..(c + 1) * plane) else { break };
            let slot = if c < cc.min(3) {
                c
            } else if c == cc && alpha {
                3
            } else {
                // The fourth CMYK channel is the black plate, not a slot of the result.
                for_each_sample(h.depth, raw, w, hh, |i, v| {
                    if let Some(k) = black.get_mut(i) {
                        *k = v;
                    }
                });
                continue;
            };
            for_each_sample(h.depth, raw, w, hh, |i, v| {
                if let Some(p) = data.get_mut(i) {
                    p[slot] = v;
                }
            });
        }
        let unmatte = alpha && h.color_mode == ColorMode::Rgb;
        for (i, p) in data.iter_mut().enumerate() {
            let a = p[3];
            if unmatte && a > 0.0 && a < 1.0 {
                for c in p.iter_mut().take(3) {
                    *c = ((*c + a - 1.0) / a).clamp(0.0, 1.0);
                }
            }
            let [r, g, b] = to_rgb.rgb([p[0], p[1], p[2], black.get(i).copied().unwrap_or(0.0)]);
            *p = [r, g, b, a];
        }
        Ok(FloatImage { width: h.width, height: h.height, data })
    }

    /// One layer's pixels with its raster mask applied to alpha. A solid colour fill layer
    /// without pixels is filled over the canvas (or over its mask's rectangle when the mask
    /// hides everything outside it). `canvas`: document-sized with the layer in place;
    /// otherwise the layer's own bounds (1×1 for an empty layer).
    pub fn layer_f32(&self, index: usize, canvas: bool) -> Result<FloatImage> {
        self.layer_f32_within(index, canvas, DECODE_BUDGET)
    }

    /// [`Self::layer_f32`] holding at most `limit` bytes at once: the result (16 bytes a pixel),
    /// then either the channel being decoded (its bytes and its samples as floats) with the CMYK
    /// black plane, or the mask.
    pub fn layer_f32_within(&self, index: usize, canvas: bool, limit: u64) -> Result<FloatImage> {
        let l = self.layers().get(index).ok_or_else(|| PsdError::invalid(format!("no layer {index}")))?;
        let h = &self.header;
        let doc = Rect::from_xywh(0, 0, h.width, h.height);
        let has_pixels = !l.rect.is_empty() && l.channels.iter().any(|c| c.id >= 0);
        let fill = l.solid_color().filter(|_| !has_pixels);
        let mask = l.layer_mask().filter(|m| !m.disabled());
        // Fill layers often have no pixel data: they cover the canvas (or the mask).
        let src = if fill.is_some() { l.layer_mask().filter(|m| !m.rect.is_empty() && m.default_color == 0).map_or(doc, |m| m.rect) } else { l.rect };
        let out_rect = if canvas {
            doc
        } else if src.is_empty() {
            Rect::from_xywh(0, 0, 1, 1)
        } else {
            src
        };
        let (ow, oh) = out_rect.size()?;
        let n = pixel_count(ow, oh)?;
        let (sw, sh) = size_of(src)?;
        let place = Placement { ow, oh, out_rect, src, sw, sh };
        let cmyk = h.color_mode == ColorMode::Cmyk;
        // The peak: the result, then either the CMYK black plane with the channel being decoded,
        // or the mask. Checked before anything is allocated.
        let plane = |w: usize, hh: usize| (row_bytes(w, h.depth) as u64).saturating_mul(hh as u64);
        let pixels_peak = if has_pixels {
            plane(sw, sh).saturating_add(4 * (sw as u64) * (sh as u64)).saturating_add(if cmyk { 4 * n as u64 } else { 0 })
        } else {
            0
        };
        let mask_peak = match mask {
            Some(m) if l.channel(CHANNEL_USER_MASK).is_some() => size_of(m.rect).map(|(mw, mh)| plane(mw, mh))?,
            _ => 0,
        };
        within(limit, (16 * n as u64).saturating_add(pixels_peak.max(mask_peak)))?;
        pixel_count(sw, sh)?;
        let mut out = filled(n, [0.0f32; 4])?;
        match fill {
            Some(f) => place.for_each(|d, _| {
                if let Some(p) = out.get_mut(d) {
                    *p = [f[0] as f32, f[1] as f32, f[2] as f32, 1.0];
                }
            }),
            None => self.layer_channels(l, &place, cmyk, &mut out)?,
        }
        if let Some(m) = mask {
            self.apply_layer_mask(l, m, out_rect, &mut out)?;
        }
        Ok(FloatImage { width: ow as u32, height: oh as u32, data: out })
    }

    /// Decodes the layer's colour and transparency channels into `out` and converts to RGB.
    fn layer_channels(&self, l: &LayerRecord, place: &Placement, cmyk: bool, out: &mut [[f32; 4]]) -> Result<()> {
        let h = &self.header;
        let to_rgb = ToRgb::of(self);
        let cc = to_rgb.color_channels();
        // Opaque unless there is a transparency channel.
        place.for_each(|d, _| {
            if let Some(p) = out.get_mut(d) {
                p[3] = 1.0;
            }
        });
        let mut black: Vec<f32> = Vec::new();
        if place.sw > 0 && place.sh > 0 {
            for c in &l.channels {
                // The fourth CMYK channel is the black plate, not a slot of the result.
                let to_black = cmyk && c.id == 3;
                let slot = match c.id {
                    CHANNEL_TRANSPARENCY => 3,
                    id if id >= 0 && (id as usize) < cc.min(3) => id as usize,
                    _ if to_black => 0,
                    _ => continue,
                };
                let raw = c.decode(place.sw, place.sh, h.depth, h.version)?;
                if to_black && black.is_empty() {
                    black = filled(out.len(), 0.0f32)?;
                }
                let mut plane = filled(place.sw * place.sh, 0.0f32)?;
                for_each_sample(h.depth, &raw, place.sw, place.sh, |i, v| {
                    if let Some(o) = plane.get_mut(i) {
                        *o = v;
                    }
                });
                drop(raw);
                place.for_each(|d, s| {
                    let Some(&v) = plane.get(s) else { return };
                    if to_black {
                        if let Some(k) = black.get_mut(d) {
                            *k = v;
                        }
                    } else if let Some(p) = out.get_mut(d) {
                        p[slot] = v;
                    }
                });
            }
        }
        place.for_each(|d, _| {
            if let Some(p) = out.get_mut(d) {
                let [r, g, b] = to_rgb.rgb([p[0], p[1], p[2], black.get(d).copied().unwrap_or(0.0)]);
                *p = [r, g, b, p[3]];
            }
        });
        Ok(())
    }

    /// Multiplies alpha by the raster mask, including its default outside the mask bounds.
    fn apply_layer_mask(&self, l: &LayerRecord, m: &LayerMask, out_rect: Rect, out: &mut [[f32; 4]]) -> Result<()> {
        let h = &self.header;
        let (ow, oh) = out_rect.size()?;
        let (mw, mh) = size_of(m.rect)?;
        let inverted = m.flags & LayerMask::FLAG_INVERT != 0;
        let def = f32::from(m.default_color) / 255.0;
        // The factor outside the mask rectangle, or everywhere when there is no mask data.
        let outside = if inverted { 1.0 - def } else { def };
        let scale = |px: &mut [[f32; 4]], v: f32| {
            if v != 1.0 {
                for p in px {
                    p[3] *= v;
                }
            }
        };
        let mask = match l.channel(CHANNEL_USER_MASK) {
            Some(c) if mw > 0 && mh > 0 => {
                pixel_count(mw, mh)?;
                c.decode(mw, mh, h.depth, h.version)?
            }
            _ => {
                scale(out, outside);
                return Ok(());
            }
        };
        let mut values = filled(mw, def)?;
        // The columns of the output that fall inside the mask rectangle (the same for every row).
        let lo = (i64::from(m.rect.left) - i64::from(out_rect.left)).clamp(0, ow as i64);
        let hi = (i64::from(m.rect.right) - i64::from(out_rect.left)).clamp(lo, ow as i64);
        let (lo, hi) = (lo as usize, hi as usize);
        let first = (i64::from(out_rect.left) + lo as i64 - i64::from(m.rect.left)) as usize;
        let rowb = row_bytes(mw, h.depth);
        for (y, row) in out.chunks_exact_mut(ow.max(1)).take(oh).enumerate() {
            let py = i64::from(out_rect.top) + y as i64;
            if py < i64::from(m.rect.top) || py >= i64::from(m.rect.bottom) {
                scale(row, outside);
                continue;
            }
            let my = (py - i64::from(m.rect.top)) as usize;
            values.fill(def);
            if let Some(raw) = mask.get(my * rowb..(my + 1) * rowb) {
                for_each_sample(h.depth, raw, mw, 1, |i, v| {
                    if let Some(o) = values.get_mut(i) {
                        *o = v;
                    }
                });
            }
            let (left, rest) = row.split_at_mut(lo);
            let (mid, right) = rest.split_at_mut(hi - lo);
            scale(left, outside);
            scale(right, outside);
            for (i, p) in mid.iter_mut().enumerate() {
                let v = values.get(first + i).copied().unwrap_or(def);
                p[3] *= if inverted { 1.0 - v } else { v };
            }
        }
        Ok(())
    }
}

impl SmartObject {
    /// Bakes the content (straight RGBA) as placed: warped and mapped by the quad, rendered at
    /// `k` pixels per document pixel (into `size` pixels when given; see
    /// [`SmartObject::placed_size`] and [`SmartObject::bake_scale`]). Returns the pixels and
    /// their document-space origin. `None` when the content is empty, the baked image would
    /// exceed [`MAX_BAKE_PIXELS`], or the mesh needs more rasterising work than eight times the
    /// output allows (a hostile mesh whose triangles each cover the whole output).
    pub fn render_placed(&self, content: &FloatImage, k: f64, size: Option<(u32, u32)>) -> Option<(FloatImage, [f64; 2])> {
        let (cw, ch) = (f64::from(content.width), f64::from(content.height));
        if content.width == 0 || content.height == 0 {
            return None;
        }
        // The quad is solved once for the bounds, the size and the whole grid.
        let m = square_to_quad(self.quad);
        let bb = self.bounds_with(&m, cw, ch);
        let (ow, oh) = match size {
            Some((w, h)) => (w.max(1), h.max(1)),
            None => size_for(bb, k),
        };
        let (ow, oh) = (ow as usize, oh as usize);
        let count = ow.checked_mul(oh).filter(|&c| c as u64 <= MAX_BAKE_PIXELS)?;
        // A forward grid, filled triangle by triangle with inverse barycentric sampling.
        let n = 48usize;
        let mut dst = Vec::with_capacity((n + 1) * (n + 1));
        let mut src = Vec::with_capacity((n + 1) * (n + 1));
        for j in 0..=n {
            for i in 0..=n {
                let s = [i as f64 / n as f64 * cw, j as f64 / n as f64 * ch];
                let d = self.map_with(&m, s, cw, ch);
                dst.push([(d[0] - bb[0]) * k, (d[1] - bb[1]) * k]);
                src.push(s);
            }
        }
        let mut out: Vec<[f32; 4]> = Vec::new();
        out.try_reserve_exact(count).ok()?;
        out.resize(count, [0.0; 4]);
        let sampler = Sampler { content, cw, ch };
        let idx = |i: usize, j: usize| j * (n + 1) + i;
        // A sane mesh's triangles each span about one grid cell (ow/n by oh/n), rounded out to
        // whole pixels. A thin output (16384×1) stacks every grid row onto the same pixel row, so
        // allow each of the 2n² triangles about two cells' extent along each axis, with margin.
        let rounding = (4 * n as u64).saturating_mul((ow as u64).saturating_add(oh as u64).saturating_add(n as u64));
        let budget = (count as u64).saturating_mul(BAKE_WORK_FACTOR).max(MIN_BAKE_WORK).saturating_add(rounding);
        let mut work = 0u64;
        for j in 0..n {
            for i in 0..n {
                for tri in [[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)], [idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]] {
                    let corners = |v: &[[f64; 2]]| tri.map(|t| v.get(t).copied().unwrap_or([0.0; 2]));
                    work = work.saturating_add(fill_triangle(corners(&dst), corners(&src), (ow, oh), &sampler, &mut out));
                    if work > budget {
                        return None;
                    }
                }
            }
        }
        // Back to straight alpha.
        for p in &mut out {
            let a = p[3];
            if a > 0.0 {
                for c in p.iter_mut().take(3) {
                    *c /= a;
                }
            }
        }
        Some((FloatImage { width: ow as u32, height: oh as u32, data: out }, [bb[0], bb[1]]))
    }
}

/// Bilinear, premultiplied sampling of the content at content-pixel coordinates.
struct Sampler<'a> {
    content: &'a FloatImage,
    cw: f64,
    ch: f64,
}

impl Sampler<'_> {
    fn at(&self, x: usize, y: usize) -> [f32; 4] {
        // `data` is a public field: a short one reads as transparent.
        let p = self.content.data.get(y * self.content.width as usize + x).copied().unwrap_or([0.0; 4]);
        [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]
    }

    fn sample(&self, x: f64, y: f64) -> [f32; 4] {
        let (fx, fy) = ((x - 0.5).clamp(0.0, self.cw - 1.0), (y - 0.5).clamp(0.0, self.ch - 1.0));
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.content.width as usize - 1), (y0 + 1).min(self.content.height as usize - 1));
        let (a, b) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let (p00, p10, p01, p11) = (self.at(x0, y0), self.at(x1, y0), self.at(x0, y1), self.at(x1, y1));
        std::array::from_fn(|c| (p00[c] * (1.0 - a) + p10[c] * a) * (1.0 - b) + (p01[c] * (1.0 - a) + p11[c] * a) * b)
    }
}

/// Fills the output pixels inside the triangle `[a, b, c]` (output pixels) with the content
/// sampled at the matching point of the source triangle `s`. Returns the bounding-box pixels
/// visited.
fn fill_triangle(
    [a, b, c]: [[f64; 2]; 3],
    [sa, sb, sc]: [[f64; 2]; 3],
    (ow, oh): (usize, usize),
    sampler: &Sampler<'_>,
    out: &mut [[f32; 4]],
) -> u64 {
    let den = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if den.abs() < 1e-12 {
        return 0;
    }
    let (l1y, l1x) = (b[1] - c[1], c[0] - b[0]);
    let (l2y, l2x) = (c[1] - a[1], a[0] - c[0]);
    let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
    let x1 = (a[0].max(b[0]).max(c[0]).ceil() as usize).min(ow);
    let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
    let y1 = (a[1].max(b[1]).max(c[1]).ceil() as usize).min(oh);
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let l1 = (l1y * (px - c[0]) + l1x * (py - c[1])) / den;
            let l2 = (l2y * (px - c[0]) + l2x * (py - c[1])) / den;
            let l3 = 1.0 - l1 - l2;
            if l1 < -1e-9 || l2 < -1e-9 || l3 < -1e-9 {
                continue;
            }
            if let Some(o) = out.get_mut(y * ow + x) {
                *o = sampler.sample(sa[0] * l1 + sb[0] * l2 + sc[0] * l3, sa[1] * l1 + sb[1] * l2 + sc[1] * l3);
            }
        }
    }
    (x1.saturating_sub(x0) as u64).saturating_mul(y1.saturating_sub(y0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{LayerSpec, MaskSpec, PixelData, PsdBuilder};
    use crate::descriptor::{Descriptor, Value};
    use crate::io::WriteExt;
    use crate::pixels::f32_to_bytes;
    use crate::tagged::TaggedBlock;
    use crate::testgen::{merged_only, mode_depths};
    use crate::{Compression, ImageData, Version};

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// A flattened file whose merged image is `planes` (one per channel, concatenated).
    fn flat(mode: ColorMode, depth: u16, w: u32, h: u32, planes: &[u8]) -> PsdFile {
        let mut file = merged_only(Version::Psd, mode, depth, Compression::Raw, w, h);
        file.image_data = ImageData::encode(Compression::Raw, planes, &file.header).unwrap();
        file
    }

    fn reopen(b: &PsdBuilder) -> PsdFile {
        PsdFile::from_bytes(&b.to_bytes().unwrap()).unwrap()
    }

    /// The float composite agrees with the 8-bit one (within a step) in every mode both read,
    /// at every depth below 32, which the 8-bit path leaves linear.
    #[test]
    fn the_float_composite_matches_the_8_bit_one() {
        for mode in [ColorMode::Bitmap, ColorMode::Grayscale, ColorMode::Indexed, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Duotone]
        {
            for &depth in mode_depths(mode).iter().filter(|&&d| d != 32) {
                for compression in Compression::ALL {
                    let file = merged_only(Version::Psd, mode, depth, compression, 7, 5);
                    let f = file.composite_f32().unwrap();
                    let b = file.composite_rgba8().unwrap();
                    assert_eq!((f.width, f.height), (b.width, b.height));
                    // The 8-bit path quantizes 16-bit Lab before its non-linear conversion.
                    let tolerance = if mode == ColorMode::Lab && depth == 16 { 2.01 } else { 1.01 };
                    for (p, q) in f.data.iter().zip(b.data.as_chunks::<4>().0) {
                        for c in 0..4 {
                            assert!((p[c] * 255.0 - f32::from(q[c])).abs() <= tolerance, "{mode:?} {depth} {compression:?}: {p:?} vs {q:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn lab_converts_through_d50_to_srgb_in_both_paths() {
        // White, black, and a saturated red-ish Lab colour.
        let file = flat(ColorMode::Lab, 8, 3, 1, &[255, 0, 135, 128, 128, 208, 128, 128, 195]);
        let f = file.composite_f32().unwrap();
        assert!(f.data[0][..3].iter().all(|&v| v > 0.99), "{:?}", f.data[0]);
        assert!(f.data[1][..3].iter().all(|&v| v < 0.01), "{:?}", f.data[1]);
        assert!(f.data[2][0] > 0.8 && f.data[2][1] < 0.3 && f.data[2][2] < 0.3, "{:?}", f.data[2]);
        let b = file.composite_rgba8().unwrap();
        assert_eq!(&b.data[..8], &[255, 255, 255, 255, 0, 0, 0, 255]);
        assert!(b.data[8] > 200 && b.data[9] < 80 && b.data[11] == 255, "{:?}", &b.data[8..]);
    }

    #[test]
    fn a_32_bit_composite_is_encoded_to_srgb_and_non_finite_samples_are_tamed() {
        let planes = [[0.5f32, f32::NAN], [0.5, f32::INFINITY], [0.5, f32::NEG_INFINITY]].iter().flat_map(|p| f32_to_bytes(p)).collect::<Vec<u8>>();
        let f = flat(ColorMode::Rgb, 32, 2, 1, &planes).composite_f32().unwrap();
        assert!(f.data[0][..3].iter().all(|&v| close(v, srgb_encode(0.5))), "{:?}", f.data[0]);
        assert_eq!([f.data[1][0], f.data[1][2], f.data[1][3]], [0.0, 0.0, 1.0]);
        assert!(close(f.data[1][1], 1.0));
    }

    #[test]
    fn a_transparent_rgb_composite_loses_its_white_matte() {
        // Half-transparent red stored over white: (255, 128, 128) at alpha 128.
        let mut file = flat(ColorMode::Rgb, 8, 1, 1, &[255, 128, 128]);
        file.header.channels = 4;
        file.image_data = ImageData::encode(Compression::Raw, &[255, 128, 128, 128], &file.header).unwrap();
        assert!(file.merged_has_alpha());
        let p = file.composite_f32().unwrap().data[0];
        assert!(close(p[3], 128.0 / 255.0) && close(p[0], 1.0) && p[1] < 0.01 && p[2] < 0.01, "{p:?}");
    }

    #[test]
    fn layers_decode_in_their_bounds_or_on_the_canvas() {
        let mut b = PsdBuilder::new(4, 2).depth(16).compression(Compression::ZipPrediction);
        let data = vec![65535, 0, 0, 65535, 0, 32768, 0, 32768];
        b.push_layer(LayerSpec::new("two", 1, 1, 2, 1, PixelData::Rgba16(data)));
        let file = reopen(&b);
        let own = file.layer_f32(0, false).unwrap();
        assert_eq!((own.width, own.height), (2, 1));
        assert_eq!(own.data[0], [1.0, 0.0, 0.0, 1.0]);
        assert!(close(own.data[1][1], 32768.0 / 65535.0) && close(own.data[1][3], 32768.0 / 65535.0));
        let canvas = file.layer_f32(0, true).unwrap();
        assert_eq!((canvas.width, canvas.height), (4, 2));
        assert_eq!(canvas.data[5], own.data[0]);
        assert_eq!(canvas.data[6], own.data[1]);
        assert!(canvas.data.iter().enumerate().filter(|&(i, _)| i != 5 && i != 6).all(|(_, p)| p[3] == 0.0));
        assert!(file.layer_f32(9, false).is_err());
    }

    #[test]
    fn a_layer_mask_scales_alpha_inside_and_outside_its_rectangle() {
        let mut b = PsdBuilder::new(4, 1);
        let mut spec = LayerSpec::new("masked", 0, 0, 4, 1, PixelData::Rgba8([255, 0, 0, 255].repeat(4)));
        spec.mask = Some(MaskSpec { rect: Rect::from_xywh(1, 0, 2, 1), data: vec![255, 51], default_color: 0, disabled: false });
        b.push_layer(spec.clone());
        let alpha = |file: &PsdFile| file.layer_f32(0, true).unwrap().data.iter().map(|p| p[3]).collect::<Vec<f32>>();
        assert_eq!(alpha(&reopen(&b)), [0.0, 1.0, 0.2, 0.0]);
        spec.mask = Some(MaskSpec { rect: Rect::from_xywh(1, 0, 2, 1), data: vec![255, 51], default_color: 255, disabled: true });
        let mut b = PsdBuilder::new(4, 1);
        b.push_layer(spec);
        assert_eq!(alpha(&reopen(&b)), [1.0; 4]);
    }

    #[test]
    fn cmyk_layers_and_composites_convert_ink_to_rgb() {
        let mut b = PsdBuilder::new(2, 1).color_mode(ColorMode::Cmyk);
        let ink = PixelData::Cmyka8(vec![0, 0, 0, 0, 255, 0, 0, 0, 255, 255]);
        b.push_layer(LayerSpec::new("ink", 0, 0, 2, 1, ink.clone()));
        b.composite(ink);
        let file = reopen(&b);
        for img in [file.layer_f32(0, false).unwrap(), file.composite_f32().unwrap()] {
            assert_eq!(img.data[0], [1.0, 1.0, 1.0, 1.0]);
            assert_eq!(&img.data[1][..3], &[0.0, 0.0, 0.0]);
        }
    }

    #[test]
    fn a_fill_layer_without_pixels_fills_the_canvas_or_its_mask() {
        let mut file = reopen(&PsdBuilder::new(3, 2));
        let rgb = Descriptor::new("RGBC").with("Rd  ", Value::Double(255.0)).with("Grn ", Value::Double(127.5)).with("Bl  ", Value::Double(0.0));
        let mut soco = Vec::new();
        soco.put_u32(16);
        Descriptor::new("null").with("Clr ", Value::Descriptor(rgb)).write(&mut soco);
        file.layers_mut().push(LayerRecord { blocks: vec![TaggedBlock::new(*b"SoCo", soco)], ..Default::default() });
        let i = file.layers().len() - 1;
        let img = file.layer_f32(i, true).unwrap();
        assert!(img.data.iter().all(|p| *p == [1.0, 0.5, 0.0, 1.0]), "{:?}", img.data);
        // A mask that hides everything outside its rectangle bounds the fill.
        let layer = &mut file.layers_mut()[i];
        layer.mask = crate::layer::MaskData::Mask(LayerMask::new(Rect::from_xywh(1, 1, 2, 1), 0, 0));
        layer.channels.push(crate::layer::ChannelData::encode(CHANNEL_USER_MASK, Compression::Raw, &[255, 255], 2, 1, 8, Version::Psd).unwrap());
        let own = file.layer_f32(i, false).unwrap();
        assert_eq!((own.width, own.height), (2, 1));
        let canvas = file.layer_f32(i, true).unwrap();
        assert_eq!(canvas.data.iter().map(|p| p[3]).collect::<Vec<_>>(), [0.0, 0.0, 0.0, 0.0, 1.0, 1.0]);
    }

    /// Sizes are untrusted: an image over the pixel cap, or a decode over its memory budget, is
    /// refused before anything is allocated for it.
    #[test]
    fn the_pixel_cap_and_the_memory_budget_refuse_before_allocating() {
        let mut big = flat(ColorMode::Rgb, 8, 1, 1, &[1, 2, 3]);
        big.header.version = Version::Psb;
        (big.header.width, big.header.height) = (300_000, 300_000);
        assert!(matches!(big.composite_f32(), Err(PsdError::LimitExceeded(_))));
        let mut file = reopen(&PsdBuilder::new(2, 2));
        file.layers_mut().push(LayerRecord { rect: Rect::from_xywh(0, 0, 300_000, 300_000), ..Default::default() });
        let last = file.layers().len() - 1;
        assert!(matches!(file.layer_f32(last, false), Err(PsdError::LimitExceeded(_))));

        let mut b = PsdBuilder::new(64, 64);
        b.push_layer(LayerSpec::new("l", 0, 0, 64, 64, PixelData::Rgba8(vec![9; 64 * 64 * 4])));
        let file = reopen(&b);
        let n = 64 * 64u64;
        // The composite: the result, plus the decoded planes of every channel.
        let peak = 16 * n + u64::from(file.header.channels) * n;
        assert!(file.composite_f32_within(peak).is_ok());
        assert!(matches!(file.composite_f32_within(peak - 1), Err(PsdError::LimitExceeded(_))));
        // The layer: the result, plus one channel's bytes and its samples as floats.
        assert!(file.layer_f32_within(0, true, 16 * n + 5 * n).is_ok());
        assert!(matches!(file.layer_f32_within(0, true, 16 * n + 5 * n - 1), Err(PsdError::LimitExceeded(_))));
        assert_eq!(file.layer_f32_within(0, true, 16 * n + 5 * n).unwrap(), file.layer_f32(0, true).unwrap());
    }

    fn so(quad: [[f64; 2]; 4], warp: Option<crate::smart::Warp>) -> SmartObject {
        SmartObject { uuid: "x".into(), quad, size: None, warp }
    }

    #[test]
    fn render_placed_bakes_a_perspective_quad() {
        // 2×1 content: red | green, pinned to a trapezoid.
        let content = FloatImage { width: 2, height: 1, data: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]] };
        let (px, origin) = so([[0.0, 0.0], [40.0, 5.0], [40.0, 15.0], [0.0, 20.0]], None).render_placed(&content, 1.0, None).unwrap();
        assert_eq!(origin, [0.0, 0.0]);
        assert_eq!((px.width, px.height), (40, 20));
        let at = |x: usize, y: usize| px.data[y * 40 + x];
        assert!(at(5, 10)[0] > 0.9 && at(5, 10)[3] > 0.99, "{:?}", at(5, 10));
        assert!(at(35, 10)[1] > 0.9, "{:?}", at(35, 10));
        assert_eq!(at(38, 1)[3], 0.0, "outside the trapezoid");
        // Empty content, or a short `data`, bakes nothing or reads as transparent.
        assert!(so([[0.0; 2]; 4], None).render_placed(&FloatImage { width: 0, height: 0, data: vec![] }, 1.0, None).is_none());
        let short = FloatImage { width: 4, height: 4, data: vec![[1.0; 4]; 3] };
        assert!(so([[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]], None).render_placed(&short, 1.0, None).is_some());
    }

    #[test]
    fn render_placed_refuses_an_output_over_the_pixel_cap() {
        let content = FloatImage { width: 2, height: 1, data: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]] };
        let s = so([[0.0, 0.0], [40.0, 5.0], [40.0, 15.0], [0.0, 20.0]], None);
        assert!(s.render_placed(&content, 1.0, Some((16384, 16384))).is_none());
        assert!(s.render_placed(&content, 1.0, Some((u32::MAX, u32::MAX))).is_none());
    }

    #[test]
    fn a_mesh_that_makes_every_triangle_cover_the_output_is_refused() {
        // A 48×48 custom mesh whose patch corners scatter over the four corners of the
        // content, so most of the 4608 triangles span the whole 2048² output.
        let (cols, rows) = (48usize, 48usize);
        let stride = cols * 3 + 1;
        let mut mesh = vec![[0.0, 0.0]; stride * (rows * 3 + 1)];
        for j in 0..=rows {
            for i in 0..=cols {
                if let Some(p) = mesh.get_mut(j * 3 * stride + i * 3) {
                    let h = (i * 7 + j * 13 + i * j * 5) % 4;
                    *p = [if h & 1 == 0 { 0.0 } else { 16.0 }, if h & 2 == 0 { 0.0 } else { 16.0 }];
                }
            }
        }
        let warp = crate::smart::Warp { style: "warpCustom".into(), bounds: [0.0, 0.0, 16.0, 16.0], cols, rows, mesh, ..Default::default() };
        let s = so([[0.0, 0.0], [2048.0, 0.0], [2048.0, 2048.0], [0.0, 2048.0]], Some(warp));
        let content = FloatImage { width: 16, height: 16, data: vec![[1.0, 0.0, 0.0, 1.0]; 256] };
        assert!(s.render_placed(&content, 1.0, Some((2048, 2048))).is_none());
    }

    #[test]
    fn a_thin_output_is_not_refused_for_rounding() {
        let warp = crate::smart::Warp { style: "warpArc".into(), bounds: [0.0, 0.0, 16.0, 16.0], bend: 20.0, ..Default::default() };
        let s = so([[0.0, 0.0], [16384.0, 0.0], [16384.0, 1.0], [0.0, 1.0]], Some(warp));
        let content = FloatImage { width: 16, height: 16, data: vec![[1.0, 0.0, 0.0, 1.0]; 256] };
        assert!(s.render_placed(&content, 1.0, Some((16384, 1))).is_some());
    }
}
