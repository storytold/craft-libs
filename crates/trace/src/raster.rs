/// Errors decoding a raster.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("could not decode image: {0}")]
    Decode(String),
    #[error("image is empty")]
    Empty,
    #[error("the image is {width} × {height} pixels, more than Image Trace takes ({max} megapixels)")]
    TooLarge { width: u32, height: u32, max: u64 },
    #[error("this isn't a flat-colour logo ({:.0}% of its solid areas match a few flat colours), so Flat Logo mode would turn its gradients into patches: use Color mode instead", (.fraction * 100.0).floor())]
    NotFlat { fraction: f64 },
    #[error("\"{0}\" is not a colour: write it as #rrggbb")]
    BadPalette(String),
    #[error(
        "this trace would make more than {max} anchor points, more than the document can show smoothly: raise Noise, lower Paths or Colors, or pick a lower-fidelity preset"
    )]
    TooComplex { max: usize },
    /// The caller of [`crate::trace_watched`] stopped the trace.
    #[error("the trace was cancelled")]
    Cancelled,
}

/// The most pixels an image to trace may have (64 megapixels: 256 MB decoded); larger images are
/// refused instead of running out of memory.
pub const MAX_PIXELS: u64 = 64 << 20;

/// The most pixels an image may have for [`Raster::decode_to_fit`] to decode it and scale it down
/// to [`MAX_PIXELS`] (192 megapixels: 768 MB decoded); larger images are refused.
pub const MAX_SOURCE_PIXELS: u64 = 192 << 20;

/// Header facts for tracing, without reserving a PNG's full decoded pixel buffer.
/// The shared allocation probe runs before the decoder's plausibility checks so the caller's
/// size refusal wins even for a large header-only file. The streaming reader still checks the
/// PNG structure and plausibility, and reports the expanded layout (including tRNS alpha).
pub(crate) fn trace_header(bytes: &[u8], max_pixels: u64, max_alloc: u64) -> Result<Option<craft_image_codecs::RasterInspection>, TraceError> {
    use craft_image_codecs::{ChannelLayout, Format, Limits, PngDecodeRows, RasterColorType, RasterInspection};
    let decode_err = |e: craft_image_codecs::CodecError| TraceError::Decode(e.to_string());
    // Refuse a GIF frame larger than its logical screen before inspection's
    // plausibility check can turn the caller's size refusal into a decode error.
    if gif_first_frame_pixels(bytes).is_some_and(|pixels| pixels > max_pixels) {
        return Err(TraceError::TooLarge { width: 0, height: 0, max: max_pixels >> 20 });
    }
    if craft_image_codecs::detect(bytes) != Some(Format::Png) {
        return craft_image_codecs::inspect_raster(bytes).map_err(decode_err);
    }
    let (width, height) = craft_image_codecs::probe_allocation_size(bytes).map_err(decode_err)?;
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(TraceError::TooLarge { width, height, max: max_pixels >> 20 });
    }
    // The allocation probe validates the IHDR prefix. For explicit alpha layouts, its
    // sample width also gives a lower bound on decoded bytes before plausibility checks.
    // Preserve the alpha caller's TooLarge refusal for header-only RGBA16/GrayA images;
    // the shared reader still validates CRCs, transparency expansion, and all other layouts.
    let alpha_bytes_per_pixel = match bytes.get(24..26) {
        Some([8, 4]) => 2,
        Some([16, 4]) | Some([8, 6]) => 4,
        Some([16, 6]) => 8,
        _ => 0,
    };
    if (u64::from(width) * u64::from(height)).saturating_mul(alpha_bytes_per_pixel) > max_alloc {
        return Err(TraceError::TooLarge { width, height, max: max_pixels >> 20 });
    }
    let limits = Limits { max_pixels, max_alloc, ..Limits::default() };
    let rows = PngDecodeRows::new_streaming(bytes, &limits).map_err(decode_err)?;
    let h = rows.header();
    let color_type = match h.layout {
        ChannelLayout::Gray => RasterColorType::Gray,
        ChannelLayout::GrayA => RasterColorType::GrayAlpha,
        ChannelLayout::Rgb => RasterColorType::Rgb,
        ChannelLayout::Rgba => RasterColorType::Rgba,
        _ => RasterColorType::Other,
    };
    Ok(Some(RasterInspection {
        width: h.width,
        height: h.height,
        channels: h.layout.channels() as u8,
        bits_per_sample: (h.sample.bytes() * 8) as u8,
        color_type,
        palette: None,
        transparent_index: None,
    }))
}

/// Reuse an already-normalized RGBA8 decode instead of allocating a second full pixel buffer.
fn into_rgba8(img: craft_image_codecs::Image) -> Vec<u8> {
    if img.layout() == craft_image_codecs::ChannelLayout::Rgba && img.sample_type() == craft_image_codecs::SampleType::U8 {
        img.into_data()
    } else {
        img.to_rgba8()
    }
}

/// The largest size with `w : h`'s aspect of at most `max` pixels (each side at least 1).
pub(crate) fn fit_size(w: u32, h: u32, max: u64) -> (u32, u32) {
    let pixels = u64::from(w) * u64::from(h);
    if pixels <= max || pixels == 0 {
        return (w, h);
    }
    let k = (max as f64 / pixels as f64).sqrt();
    let side = |v: u32| ((f64::from(v) * k).floor() as u32).clamp(1, v);
    let (mut nw, mut nh) = (side(w), side(h));
    // A side held at 1 pixel leaves the other too long: cut it to what fits.
    if u64::from(nw) * u64::from(nh) > max {
        if nw >= nh {
            nw = u32::try_from(max / u64::from(nh)).unwrap_or(nw).clamp(1, nw);
        } else {
            nh = u32::try_from(max / u64::from(nw)).unwrap_or(nh).clamp(1, nh);
        }
    }
    (nw, nh)
}

/// The `w × h` RGBA8 `rgba` reduced to `nw × nh` (at most `w × h`): each pixel covers a block of
/// whole source pixels, and every source pixel falls in exactly one block. A pixel's alpha is the
/// mean of its block's alphas, and its color is the block's colors weighted by their alpha, so a
/// clear pixel next to an opaque one does not darken or tint it. A wholly clear block (every alpha
/// 0) keeps the plain mean of its colors.
pub(crate) fn box_reduced(rgba: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let span = |i: u32, n: u32, of: u32| {
        let at = |i: u32| usize::try_from(u64::from(i) * u64::from(of) / u64::from(n.max(1))).unwrap_or(usize::MAX);
        let start = at(i);
        start..at(i + 1).max(start + 1)
    };
    let cols: Vec<_> = (0..nw).map(|x| span(x, nw, w)).collect();
    let mut out = Vec::with_capacity(nw as usize * nh as usize * 4);
    // Per output column: the alpha-weighted color sums, the alpha sum and the plain color sums.
    let mut sums = vec![[0u64; 7]; cols.len()];
    let row_len = w as usize * 4;
    for ty in 0..nh {
        let rows = span(ty, nh, h);
        sums.fill([0; 7]);
        for y in rows.clone() {
            let Some(row) = rgba.get(y * row_len..(y + 1) * row_len) else { break };
            for (s, c) in sums.iter_mut().zip(&cols) {
                let px = row.get(c.start * 4..c.end * 4).unwrap_or_default();
                for &[r, g, b, a] in px.as_chunks::<4>().0 {
                    let a64 = u64::from(a);
                    s[0] += u64::from(r) * a64;
                    s[1] += u64::from(g) * a64;
                    s[2] += u64::from(b) * a64;
                    s[3] += a64;
                    s[4] += u64::from(r);
                    s[5] += u64::from(g);
                    s[6] += u64::from(b);
                }
            }
        }
        for (s, c) in sums.iter().zip(&cols) {
            let count = (rows.len() * c.len()).max(1) as u64;
            let mean = |v: u64, n: u64| u8::try_from((v + n / 2) / n).unwrap_or(u8::MAX);
            if s[3] == 0 {
                out.extend_from_slice(&[mean(s[4], count), mean(s[5], count), mean(s[6], count), 0]);
            } else {
                out.extend_from_slice(&[mean(s[0], s[3]), mean(s[1], s[3]), mean(s[2], s[3]), mean(s[3], count)]);
            }
        }
    }
    out
}

/// An RGBA8 raster (row-major, top row first).
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The pixels the first frame of GIF `bytes` declares in its image descriptor, which the decoder
/// allocates for before it crops the frame to the logical screen the dimension check reads. `None`
/// when the bytes aren't a GIF or hold no frame header.
pub(crate) fn gif_first_frame_pixels(bytes: &[u8]) -> Option<u64> {
    if !bytes.starts_with(b"GIF8") {
        return None;
    }
    let flags = *bytes.get(10)?;
    let mut at = 13 + if flags & 0x80 != 0 { 3 << ((flags & 7) + 1) } else { 0 };
    loop {
        match *bytes.get(at)? {
            // An extension: its data sub-blocks end at a zero length.
            0x21 => {
                at += 2;
                loop {
                    let len = usize::from(*bytes.get(at)?);
                    at += 1 + len;
                    if len == 0 {
                        break;
                    }
                }
            }
            0x2C => {
                let d = bytes.get(at + 5..at + 9)?;
                return Some(u64::from(u16::from_le_bytes([d[0], d[1]])) * u64::from(u16::from_le_bytes([d[2], d[3]])));
            }
            _ => return None,
        }
    }
}

impl Raster {
    /// A raster from RGBA bytes, which must be exactly `width * height * 4` of them: any other
    /// length (bytes from a file, a plug-in or a message) is a [`TraceError::Decode`] error, not
    /// a crash. (That error's text is already translated; its detail names the sizes.)
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, TraceError> {
        let want = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4));
        if want != Some(rgba.len()) {
            return Err(TraceError::Decode(format!("{} bytes of RGBA data for a {width} × {height} image", rgba.len())));
        }
        Ok(Self { width, height, rgba })
    }
    /// A raster whose pixel `(x, y)` is `f(x, y)`.
    pub fn from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Self {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Self { width, height, rgba }
    }
    /// Decode PNG / JPEG / WebP / GIF bytes (at most [`MAX_PIXELS`], checked before decoding).
    pub fn decode(bytes: &[u8]) -> Result<Self, TraceError> {
        if let Some(info) = trace_header(bytes, MAX_PIXELS, craft_image_codecs::Limits::default().max_alloc)? {
            if info.width == 0 || info.height == 0 {
                return Err(TraceError::Empty);
            }
            if u64::from(info.width) * u64::from(info.height) > MAX_PIXELS || gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_PIXELS) {
                return Err(TraceError::TooLarge { width: info.width, height: info.height, max: MAX_PIXELS >> 20 });
            }
        } else if gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_PIXELS) {
            return Err(TraceError::TooLarge { width: 0, height: 0, max: MAX_PIXELS >> 20 });
        }
        let opts = craft_image_codecs::DecodeOptions {
            // Match rendering and trace placement: coordinates refer to stored pixels.
            keep_orientation: true,
            limits: craft_image_codecs::Limits { max_pixels: MAX_PIXELS, ..craft_image_codecs::Limits::default() },
        };
        let img = craft_image_codecs::decode_with(bytes, &opts).map_err(|e| TraceError::Decode(e.to_string()))?;
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        Ok(Self { width: w, height: h, rgba: into_rgba8(img) })
    }
    /// Decode as [`Raster::decode`], but an image of more than [`MAX_PIXELS`] (up to
    /// [`MAX_SOURCE_PIXELS`]) is scaled down, aspect kept, to the largest size within
    /// [`MAX_PIXELS`] instead of being refused: a 300 ppi scan of a poster traces at slightly
    /// less than its own resolution. The caller maps the result back with `width` / `height`.
    pub fn decode_to_fit(bytes: &[u8]) -> Result<Self, TraceError> {
        if let Some(info) = trace_header(bytes, MAX_SOURCE_PIXELS, if cfg!(target_arch = "wasm32") { 512 << 20 } else { MAX_SOURCE_PIXELS * 4 })? {
            if info.width == 0 || info.height == 0 {
                return Err(TraceError::Empty);
            }
            let pixels = u64::from(info.width) * u64::from(info.height);
            if pixels <= MAX_PIXELS && !gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_PIXELS) {
                return Self::decode(bytes);
            }
            if pixels > MAX_SOURCE_PIXELS {
                return Err(TraceError::TooLarge { width: info.width, height: info.height, max: MAX_SOURCE_PIXELS >> 20 });
            }
            if gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_SOURCE_PIXELS) {
                return Err(TraceError::TooLarge { width: info.width, height: info.height, max: MAX_SOURCE_PIXELS >> 20 });
            }
        } else if gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_SOURCE_PIXELS) {
            return Err(TraceError::TooLarge { width: 0, height: 0, max: MAX_SOURCE_PIXELS >> 20 });
        }
        // Decoded within the allocation the source limit needs; the web keeps file import's 512 MB.
        // A deeper image (16 bits a channel) that needs more fails with a decode error.
        let opts = craft_image_codecs::DecodeOptions {
            // Match rendering and trace placement: coordinates refer to stored pixels.
            keep_orientation: true,
            limits: craft_image_codecs::Limits {
                max_pixels: MAX_SOURCE_PIXELS,
                max_alloc: if cfg!(target_arch = "wasm32") { 512 << 20 } else { MAX_SOURCE_PIXELS * 4 },
                ..craft_image_codecs::Limits::default()
            },
        };
        let img = craft_image_codecs::decode_with(bytes, &opts).map_err(|e| TraceError::Decode(e.to_string()))?;
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        let rgba = into_rgba8(img);
        if u64::from(w) * u64::from(h) <= MAX_PIXELS {
            return Ok(Self { width: w, height: h, rgba });
        }
        Ok(Self::fitted(&Self { width: w, height: h, rgba }, MAX_PIXELS))
    }
    /// `img` scaled down, aspect kept, to the largest size of at most `max` pixels.
    pub(crate) fn fitted(img: &Self, max: u64) -> Self {
        let (nw, nh) = fit_size(img.width, img.height, max);
        // Box-filtered: every source pixel counts, so thin lines and anti-aliased edges survive.
        Self { width: nw, height: nh, rgba: box_reduced(&img.rgba, img.width, img.height, nw, nh) }
    }
    /// Encode as PNG.
    pub fn encode_png(&self) -> Vec<u8> {
        let Ok(img) = craft_image_codecs::Image::from_u8(self.width, self.height, craft_image_codecs::ChannelLayout::Rgba, self.rgba.clone()) else {
            return Vec::new();
        };
        craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &Default::default()).unwrap_or_default()
    }
    /// Pixel `(x, y)`; transparent black outside the raster (or past the end of a raster whose
    /// fields were set to a size its bytes don't have).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        match self.rgba.get(i..i + 4) {
            Some(&[r, g, b, a]) => [r, g, b, a],
            _ => [0; 4],
        }
    }
}
