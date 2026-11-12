//! Diagnostic views drawn over a finished render ([`Overlay`]), shared by the CPU and GPU renderers
//! (they run on the 8-bit result, after the histogram is taken).
//!
//! - **Point Color range:** the photo is rendered without that sample's own adjustment; pixels
//!   inside the sample's range keep their colour, the rest turn grey (weighted by the range).
//! - **Visualize Spots** (Remove tool): a high-pass of the luminance thresholded to black/white,
//!   so dust spots and specks stand out as white dots. The scale is relative to the image (the
//!   same spots show at any preview size); the threshold slider (0..100) raises sensitivity.
//! - **Mask overlay** (Masking): the evaluated alpha of one mask, drawn as a colour tint, a colour
//!   tint on a black-and-white image, the image on black / white, or the alpha as white on black
//!   ([`MaskView`]). Both renderers hand the same alpha plane (the one the render used) to [`apply`].
//! - **Alt/Option-drag previews** (Lightroom's temporary views while a slider is dragged with the
//!   modifier held; [`Overlay::for_control`] picks one per control): shadow / highlight clipping
//!   (white or black, with the clipped channels shown), greyscale (Sharpening Amount), the fine
//!   detail band sharpening works on (Radius / Detail) and the sharpening mask (Masking). The
//!   sharpening views read the render's own detail planes ([`sharpen_plane`]).

use std::borrow::Cow;

use crate::color::perceptual::{lab_to_lch, oklab_from_2020};
use crate::color::transfer::srgb_to_linear;
use crate::color::{REC2020, SRGB};
use crate::raster::{Plane, Rgba8};
use craft_develop_model::DevelopSettings;

use crate::colorops::PointK;
use crate::{Plan, for_rows};

/// A diagnostic view of a render.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Overlay {
    #[default]
    None,
    /// Point Color sample `i`: its range in colour, everything else grey.
    PointColorRange(u8),
    /// Visualize Spots with a threshold 0..100 (higher = more sensitive).
    Spots(u8),
    /// The evaluated alpha of mask `id` (a [`craft_develop_model::Mask`] id), drawn as `view` in
    /// `color` at `opacity` (0..100; the colour views only).
    Mask { id: u16, view: MaskView, color: [u8; 3], opacity: u8 },
    /// The evaluated alpha of every visible mask that has components, drawn as one `view` in
    /// `color` at `opacity` (0..100; the colour views only) — every mask's region at once.
    AllMasks { view: MaskView, color: [u8; 3], opacity: u8 },
    /// Visualize HDR (photos edited in HDR): the SDR image dimmed to grey, with every pixel the HDR
    /// rendition puts above SDR white painted by how far above it is ([`HDR_BANDS`]).
    HdrRange,
    /// Shadow clipping (Alt-drag on Blacks / Shadows): white, a channel at 0 drops out of the
    /// colour, so pixels clipped in every channel are black and partly clipped ones are coloured.
    ClipShadows,
    /// Highlight clipping (Alt-drag on Exposure / Highlights / Whites): black, a channel at 255
    /// shows in the colour, so pixels clipped in every channel are white.
    ClipHighlights,
    /// The photo in greyscale (Alt-drag on Sharpening Amount).
    Grayscale,
    /// The fine detail band sharpening amplifies, as grey ± detail (Alt-drag on Sharpening Radius
    /// / Detail).
    SharpenDetail,
    /// The sharpening mask, white where sharpening applies (Alt-drag on Sharpening Masking).
    SharpenMask,
    /// Clipping warnings (J): the photo with every pixel that has a channel at 255 painted
    /// [`CLIP_HIGH`] and every other pixel with a channel at 0 painted [`CLIP_LOW`].
    ClipWarnings,
}

/// [`Overlay::ClipWarnings`] colour of clipped highlights.
pub const CLIP_HIGH: [u8; 3] = [255, 0, 0];
/// [`Overlay::ClipWarnings`] colour of clipped shadows.
pub const CLIP_LOW: [u8; 3] = [0, 0, 255];

/// [`Overlay::HdrRange`] colours (sRGB) for 0–1, 1–2, 2–3 and ≥ 3 stops above SDR white.
pub const HDR_BANDS: [[u8; 3]; 4] = [[255, 214, 92], [255, 150, 40], [235, 64, 52], [214, 60, 210]];

/// How [`Overlay::Mask`] draws the mask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaskView {
    /// The photo with the mask tinted in the overlay colour.
    #[default]
    Color,
    /// A black-and-white photo with the mask tinted in the overlay colour.
    ColorOnBw,
    /// The photo where the mask is, black elsewhere.
    ImageOnBlack,
    /// The photo where the mask is, white elsewhere.
    ImageOnWhite,
    /// The mask itself: white on black.
    WhiteOnBlack,
}

impl MaskView {
    pub const ALL: [MaskView; 5] = [MaskView::Color, MaskView::ColorOnBw, MaskView::ImageOnBlack, MaskView::ImageOnWhite, MaskView::WhiteOnBlack];

    /// Stable name (control channel, settings).
    pub fn name(self) -> &'static str {
        match self {
            MaskView::Color => "color",
            MaskView::ColorOnBw => "colorOnBw",
            MaskView::ImageOnBlack => "imageOnBlack",
            MaskView::ImageOnWhite => "imageOnWhite",
            MaskView::WhiteOnBlack => "whiteOnBlack",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MaskView::Color => "Color Overlay",
            MaskView::ColorOnBw => "Color Overlay on B&W",
            MaskView::ImageOnBlack => "Image on Black",
            MaskView::ImageOnWhite => "Image on White",
            MaskView::WhiteOnBlack => "White on Black",
        }
    }

    pub fn parse(s: &str) -> Option<MaskView> {
        MaskView::ALL.into_iter().find(|v| v.name() == s)
    }

    /// The next view (Shift+O cycles them).
    pub fn next(self) -> MaskView {
        let i = MaskView::ALL.iter().position(|v| *v == self).unwrap_or(0);
        MaskView::ALL[(i + 1) % MaskView::ALL.len()]
    }

    fn index(self) -> u64 {
        MaskView::ALL.iter().position(|v| *v == self).unwrap_or(0) as u64
    }
}

/// [`Overlay::Mask`]'s fields in 51 bits (exact in an `f64`): id 16, view 4, colour 24, opacity 7.
fn pack_mask(id: u16, view: MaskView, color: [u8; 3], opacity: u8) -> u64 {
    let rgb = (color[0] as u64) << 16 | (color[1] as u64) << 8 | color[2] as u64;
    id as u64 | view.index() << 16 | rgb << 20 | (opacity.min(100) as u64) << 44
}

fn unpack_mask(v: u64) -> Overlay {
    let rgb = (v >> 20) & 0xff_ffff;
    Overlay::Mask {
        id: (v & 0xffff) as u16,
        view: MaskView::ALL.get(((v >> 16) & 0xf) as usize).copied().unwrap_or_default(),
        color: [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8],
        opacity: ((v >> 44) & 0x7f).min(100) as u8,
    }
}

impl Overlay {
    /// A plain (kind, value) pair, e.g. for sending a request to a web worker.
    pub fn to_parts(self) -> (u8, f64) {
        match self {
            Overlay::None => (0, 0.0),
            Overlay::PointColorRange(i) => (1, i as f64),
            Overlay::Spots(t) => (2, t as f64),
            Overlay::Mask { id, view, color, opacity } => (3, pack_mask(id, view, color, opacity) as f64),
            Overlay::AllMasks { view, color, opacity } => (5, pack_mask(0, view, color, opacity) as f64),
            Overlay::HdrRange => (4, 0.0),
            Overlay::ClipShadows => (6, 0.0),
            Overlay::ClipHighlights => (7, 0.0),
            Overlay::Grayscale => (8, 0.0),
            Overlay::SharpenDetail => (9, 0.0),
            Overlay::SharpenMask => (10, 0.0),
            Overlay::ClipWarnings => (11, 0.0),
        }
    }

    /// Inverse of [`Overlay::to_parts`] (unknown kinds: no overlay).
    pub fn from_parts(kind: u8, v: f64) -> Overlay {
        match kind {
            1 => Overlay::PointColorRange(v as u8),
            2 => Overlay::Spots(v.clamp(0.0, 100.0) as u8),
            3 if v.is_finite() && v >= 0.0 => unpack_mask(v as u64),
            4 => Overlay::HdrRange,
            5 if v.is_finite() && v >= 0.0 => {
                let rgb = (v as u64 >> 20) & 0xff_ffff;
                Overlay::AllMasks {
                    view: MaskView::ALL.get(((v as u64 >> 16) & 0xf) as usize).copied().unwrap_or_default(),
                    color: [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8],
                    opacity: ((v as u64 >> 44) & 0x7f).min(100) as u8,
                }
            }
            6 => Overlay::ClipShadows,
            7 => Overlay::ClipHighlights,
            8 => Overlay::Grayscale,
            9 => Overlay::SharpenDetail,
            10 => Overlay::SharpenMask,
            11 => Overlay::ClipWarnings,
            _ => Overlay::None,
        }
    }

    /// A stable value for render-cache keys (0 = no overlay).
    pub fn key(self) -> u64 {
        match self {
            Overlay::None => 0,
            Overlay::PointColorRange(i) => 0x1000 + i as u64,
            Overlay::Spots(t) => 0x2000 + t as u64,
            Overlay::Mask { id, view, color, opacity } => 3 << 60 | pack_mask(id, view, color, opacity),
            Overlay::AllMasks { view, color, opacity } => 5 << 60 | pack_mask(0, view, color, opacity),
            Overlay::HdrRange => 0x4000,
            Overlay::ClipShadows => 0x5000,
            Overlay::ClipHighlights => 0x5001,
            Overlay::Grayscale => 0x5002,
            Overlay::SharpenDetail => 0x5003,
            Overlay::SharpenMask => 0x5004,
            Overlay::ClipWarnings => 0x5005,
        }
    }

    /// The temporary preview Lightroom shows while develop control `id` is dragged with Alt
    /// (Option on macOS) held, if it has one.
    pub fn for_control(id: &str) -> Option<Overlay> {
        match id {
            "light.exposure" | "light.highlights" | "light.whites" => Some(Overlay::ClipHighlights),
            "light.shadows" | "light.blacks" => Some(Overlay::ClipShadows),
            "detail.sharpenAmount" | "detail.nrLuminance" | "detail.nrDetail" | "detail.nrContrast" => Some(Overlay::Grayscale),
            "detail.sharpenRadius" | "detail.sharpenDetail" => Some(Overlay::SharpenDetail),
            "detail.sharpenMasking" => Some(Overlay::SharpenMask),
            _ => None,
        }
    }

    /// The clipping previews, which test the sRGB values the histogram counts (a render for a
    /// display profile draws them in sRGB, then converts).
    pub fn is_clipping(self) -> bool {
        matches!(self, Overlay::ClipShadows | Overlay::ClipHighlights | Overlay::ClipWarnings)
    }

    /// Whether the overlay draws from the render's detail planes ([`sharpen_plane`]).
    pub fn reads_detail(self) -> bool {
        matches!(self, Overlay::SharpenDetail | Overlay::SharpenMask)
    }

    /// Whether the overlay works pixel by pixel on the finished image, so a window render shows
    /// the same as the whole frame would.
    pub fn per_pixel(self) -> bool {
        matches!(
            self,
            Overlay::None | Overlay::Mask { .. } | Overlay::ClipShadows | Overlay::ClipHighlights | Overlay::ClipWarnings | Overlay::Grayscale
        )
    }

    /// The mask an [`Overlay::Mask`] shows (if it exists in `s` and has components).
    pub fn mask(self, s: &DevelopSettings) -> Option<&craft_develop_model::Mask> {
        match self {
            Overlay::Mask { id, .. } => s.masks.iter().find(|m| m.id == id as u32 && !m.components.is_empty()),
            _ => None,
        }
    }
}

/// Settings changes an overlay needs before rendering (e.g. the visualized sample's adjustment
/// is left out, so the range is shown on the colours it selects).
pub fn adjust_settings(o: Overlay, s: &mut Cow<'_, DevelopSettings>) {
    if let Overlay::PointColorRange(i) = o
        && let Some(p) = s.point_colors.get(i as usize)
        && !p.is_neutral()
    {
        let p = &mut s.to_mut().point_colors[i as usize];
        (p.hue_shift, p.sat_shift, p.lum_shift, p.variance) = (0.0, 0.0, 0.0, 0.0);
    }
}

/// The plane a sharpening view draws ([`Overlay::reads_detail`]), from the render's log luminance
/// `log_l` and its process-specific sharpening blur (`None` when the render didn't need it:
/// blurred here with the same kernel and sigma). [`Overlay::SharpenDetail`]: the detail band (log luminance minus its blur, in EV);
/// [`Overlay::SharpenMask`]: the sharpening weight under the photo's Masking. Both renderers call
/// this with their own planes.
pub fn sharpen_plane(o: Overlay, s: &DevelopSettings, log_l: &Plane, band: Option<&Plane>, px_per_long: f64, native_long: f64) -> Option<Plane> {
    if !o.reads_detail() {
        return None;
    }
    let own;
    let pixel = crate::local::pixel_sharpen(s);
    let blur = match band.filter(|b| (b.width, b.height) == (log_l.width, log_l.height)) {
        Some(b) => b,
        None => {
            own = if pixel {
                let sigma = crate::local::sharpen_sigma(crate::local::sharpen_sigma_raw(s, px_per_long, native_long));
                crate::raster::blur::gaussian_taps(log_l, &crate::raster::blur::gauss_taps(sigma))
            } else {
                crate::raster::blur::gaussian(log_l, crate::local::texture_sigma(px_per_long))
            };
            &own
        }
    };
    let masking = (s.detail.sharpen_masking / 100.0).clamp(0.0, 1.0) as f32;
    let mask = o == Overlay::SharpenMask;
    let data = log_l
        .data
        .iter()
        .zip(&blur.data)
        .map(|(l, b)| {
            let det = l - b;
            if !mask {
                det
            } else if pixel {
                crate::finish::pixel_sharpen_mask_weight(masking, det)
            } else {
                crate::finish::sharpen_mask_weight(masking, det)
            }
        })
        .collect();
    Some(Plane { width: log_l.width, height: log_l.height, data })
}

/// [`Overlay::SharpenDetail`]: encoded grey per EV of detail (±0.4 EV spans black to white).
const DETAIL_GAIN: f32 = 320.0;

/// Draw overlay `o` over `img` (rendered with `plan`). `plane` is what the overlay shows, at the
/// image's size: the evaluated alpha of the mask an [`Overlay::Mask`] shows (see
/// [`Overlay::mask`]) or a sharpening view's [`sharpen_plane`]; `masks` is the set of alphas an
/// [`Overlay::AllMasks`] shows, one per visible mask with components.
pub fn apply(img: &mut Rgba8, o: Overlay, plan: &Plan<'_>, plane: Option<&Plane>, masks: Option<&[Plane]>) {
    match o {
        // drawn by the renderer from the HDR rendition ([`hdr_range`])
        Overlay::None | Overlay::HdrRange => {}
        Overlay::PointColorRange(i) => {
            if let Some(p) = plan.settings.point_colors.get(i as usize) {
                point_color_range(img, &PointK::new(p));
            }
        }
        Overlay::Spots(t) => spots(img, t, plan.px_per_long),
        Overlay::Mask { view, color, opacity, .. } => {
            if let Some(a) = plane.filter(|a| (a.width, a.height) == (img.width, img.height)) {
                mask_view(img, a, view, color, opacity);
            }
        }
        Overlay::ClipShadows => clipping(img, false),
        Overlay::ClipHighlights => clipping(img, true),
        Overlay::ClipWarnings => clip_warnings(img),
        Overlay::Grayscale => grayscale(img),
        Overlay::SharpenDetail | Overlay::SharpenMask => {
            if let Some(p) = plane.filter(|a| (a.width, a.height) == (img.width, img.height)) {
                let detail = o == Overlay::SharpenDetail;
                let w = img.width;
                for_rows(&mut img.data, w, |y, row| {
                    for (x, px) in row.iter_mut().enumerate() {
                        let v = p.data[y * w + x];
                        let g = if detail { 128.0 + v * DETAIL_GAIN } else { 255.0 * v };
                        let g = g.round().clamp(0.0, 255.0) as u8;
                        *px = [g, g, g, 255];
                    }
                });
            }
        }
        Overlay::AllMasks { view, color, opacity } => {
            // one view of the union of the masks: painting the planes one after another
            // overwrites (White on Black, Image on Black…) or compounds (Color) the earlier ones
            let mut union: Option<Plane> = None;
            for a in masks.unwrap_or_default().iter().filter(|a| (a.width, a.height) == (img.width, img.height)) {
                match &mut union {
                    None => union = Some(a.clone()),
                    Some(u) => u.data.iter_mut().zip(&a.data).for_each(|(u, a)| *u = u.max(*a)),
                }
            }
            if let Some(u) = &union {
                mask_view(img, u, view, color, opacity);
            }
        }
    }
}

/// [`Overlay::ClipHighlights`] (`high`) or [`Overlay::ClipShadows`]: each channel shows only
/// whether it is clipped, on black (highlights) or white (shadows).
pub fn clipping(img: &mut Rgba8, high: bool) {
    let w = img.width;
    for_rows(&mut img.data, w, |_, row| {
        for px in row.iter_mut() {
            for c in &mut px[..3] {
                *c = if high {
                    if *c == 255 { 255 } else { 0 }
                } else if *c == 0 {
                    0
                } else {
                    255
                };
            }
            px[3] = 255;
        }
    });
}

/// [`Overlay::ClipWarnings`]: pixels with a channel at 255 in [`CLIP_HIGH`], the rest with a
/// channel at 0 in [`CLIP_LOW`], everything else unchanged.
pub fn clip_warnings(img: &mut Rgba8) {
    let w = img.width;
    for_rows(&mut img.data, w, |_, row| {
        for px in row.iter_mut() {
            let c = &px[..3];
            let paint = if c.contains(&255) {
                CLIP_HIGH
            } else if c.contains(&0) {
                CLIP_LOW
            } else {
                continue;
            };
            *px = [paint[0], paint[1], paint[2], 255];
        }
    });
}

fn grayscale(img: &mut Rgba8) {
    let w = img.width;
    for_rows(&mut img.data, w, |_, row| {
        for px in row.iter_mut() {
            let g = (0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32).round().clamp(0.0, 255.0) as u8;
            *px = [g, g, g, 255];
        }
    });
}

/// Draw mask alpha `a` over `img` as `view`.
pub fn mask_view(img: &mut Rgba8, a: &Plane, view: MaskView, color: [u8; 3], opacity: u8) {
    let op = opacity.min(100) as f32 / 100.0;
    let col = color.map(|c| c as f32);
    let w = img.width;
    for_rows(&mut img.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let a = a.data[y * w + x].clamp(0.0, 1.0);
            let p = [px[0] as f32, px[1] as f32, px[2] as f32];
            let grey = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
            let out: [f32; 3] = match view {
                MaskView::Color => std::array::from_fn(|c| p[c] + (col[c] - p[c]) * a * op),
                MaskView::ColorOnBw => std::array::from_fn(|c| grey + (col[c] - grey) * a * op),
                MaskView::ImageOnBlack => p.map(|v| v * a),
                MaskView::ImageOnWhite => p.map(|v| v * a + 255.0 * (1.0 - a)),
                MaskView::WhiteOnBlack => [255.0 * a; 3],
            };
            let o = out.map(|v| v.round().clamp(0.0, 255.0) as u8);
            *px = [o[0], o[1], o[2], 255];
        }
    });
}

/// Blur radius (Gaussian sigma) of the spot view, as a fraction of the long edge.
const SPOT_SIGMA: f64 = 0.006;

/// High-pass threshold (encoded luminance) for slider value `t` (0..100): 100 shows the faintest
/// specks, 0 only strong ones.
pub fn spot_threshold(t: u8) -> f32 {
    let k = 1.0 - t.min(100) as f32 / 100.0;
    0.01 + 0.2 * k * k
}

fn spots(img: &mut Rgba8, t: u8, ppl: f64) {
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 {
        return;
    }
    let l = crate::raster::Plane::from_fn(w, h, |x, y| {
        let p = img.data[y * w + x];
        (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0
    });
    let b = crate::raster::blur::gaussian(&l, (SPOT_SIGMA * ppl).max(0.8) as f32);
    let thr = spot_threshold(t);
    for (i, px) in img.data.iter_mut().enumerate() {
        let v = if (l.data[i] - b.data[i]).abs() > thr { 255 } else { 0 };
        *px = [v, v, v, 255];
    }
}

#[allow(clippy::needless_range_loop)] // the index addresses several arrays at once
fn point_color_range(img: &mut Rgba8, k: &PointK) {
    let m = SRGB.to_space(&REC2020).to_f32();
    let lut: Vec<f32> = (0..256).map(|v| srgb_to_linear(v as f32 / 255.0)).collect();
    let w = img.width;
    for_rows(&mut img.data, w, |_, row| {
        for px in row.iter_mut() {
            let lin = [lut[px[0] as usize], lut[px[1] as usize], lut[px[2] as usize]];
            let c = std::array::from_fn(|r| m[r][0] * lin[0] + m[r][1] * lin[1] + m[r][2] * lin[2]);
            let [l, ch, h] = lab_to_lch(oklab_from_2020(c));
            let a = k.weight(l, ch, h);
            let grey = 0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32;
            for c in 0..3 {
                px[c] = (grey + (px[c] as f32 - grey) * a).round().clamp(0.0, 255.0) as u8;
            }
        }
    });
}

/// Draw [`Overlay::HdrRange`] on `img` (the SDR render) from `hdr` (the HDR render of the same
/// request, float linear with luminance weights `luma`): grey below SDR white, band colours above.
pub fn hdr_range(img: &mut Rgba8, hdr: &crate::DeepImage, luma: [f32; 3]) {
    let crate::DeepSamples::F32(v) = &hdr.samples else { return };
    if (hdr.width, hdr.height) != (img.width, img.height) {
        return;
    }
    let w = img.width;
    for_rows(&mut img.data, w, |y, row| {
        for (x, p) in row.iter_mut().enumerate() {
            let i = (y * w + x) * 3;
            let Some(c) = v.get(i..i + 3) else { continue };
            let lum = luma[0] * c[0] + luma[1] * c[1] + luma[2] * c[2];
            let grey = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) * 0.55;
            if lum > 1.0 {
                let stops = lum.log2();
                let band = HDR_BANDS[(stops as usize).min(HDR_BANDS.len() - 1)];
                for k in 0..3 {
                    p[k] = (grey * 0.25 + band[k] as f32 * 0.75).round().clamp(0.0, 255.0) as u8;
                }
            } else {
                let g = grey.round().clamp(0.0, 255.0) as u8;
                (p[0], p[1], p[2]) = (g, g, g);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Rgb32f;
    use crate::{RenderRequest, SourceInfo, render};
    use craft_develop_model::PointColor;

    #[test]
    fn spots_view_marks_specks_at_any_size() {
        // a smooth gradient with two small dark specks
        let src = Rgb32f::from_fn(400, 300, |x, y| {
            let d = |cx: f32, cy: f32| ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let base = 0.1 + 0.3 * x as f32 / 400.0;
            if d(100.0, 100.0) < 2.5 || d(300.0, 200.0) < 2.5 { [base * 0.5; 3] } else { [base; 3] }
        });
        let info = SourceInfo::default();
        let s = DevelopSettings::default();
        for size in [400, 200] {
            let req = RenderRequest { overlay: Overlay::Spots(50), ..RenderRequest::fit(size, size) };
            let v = render(&src, &info, &s, &req).image;
            let at = |x: f64, y: f64| v.data[(y * v.height as f64) as usize * v.width + (x * v.width as f64) as usize];
            assert_eq!(at(0.25, 1.0 / 3.0), [255, 255, 255, 255], "speck at size {size}");
            assert_eq!(at(0.75, 2.0 / 3.0), [255, 255, 255, 255]);
            assert_eq!(at(0.5, 0.5), [0, 0, 0, 255], "smooth gradient is black");
            let white = v.data.iter().filter(|p| p[0] == 255).count() as f64 / v.data.len() as f64;
            assert!(white < 0.01, "{white}");
        }
        assert!(spot_threshold(0) > spot_threshold(50) && spot_threshold(50) > spot_threshold(100));
        let (k, v) = Overlay::Spots(37).to_parts();
        assert_eq!(Overlay::from_parts(k, v), Overlay::Spots(37));
    }

    #[test]
    fn point_color_range_keeps_the_selected_colour_only() {
        // left half orange, right half blue
        let src = Rgb32f::from_fn(32, 16, |x, _| if x < 16 { [0.4, 0.15, 0.04] } else { [0.03, 0.06, 0.35] });
        let info = SourceInfo::default();
        let plain = render(&src, &info, &DevelopSettings::default(), &RenderRequest::fit(32, 16)).image;
        // sample the orange as it renders
        let px = plain.data[8 * 32 + 4];
        let lin = [px[0], px[1], px[2]].map(|v| srgb_to_linear(v as f32 / 255.0));
        let c = SRGB.to_space(&REC2020).apply_f32(lin);
        let [l, ch, h] = lab_to_lch(oklab_from_2020(c));
        let mut s = DevelopSettings::default();
        s.point_colors.push(PointColor { lum: l as f64, chroma: ch as f64, hue: (h as f64).to_degrees(), hue_shift: 80.0, ..Default::default() });
        let req = RenderRequest { overlay: Overlay::PointColorRange(0), ..RenderRequest::fit(32, 16) };
        let v = render(&src, &info, &s, &req).image;
        let (o, b) = (v.data[8 * 32 + 4], v.data[8 * 32 + 28]);
        // the orange stays orange (and unshifted: the overlay shows the selection, not the edit)
        assert!(o[0].abs_diff(px[0]) <= 2 && o[2].abs_diff(px[2]) <= 2, "{o:?} vs {px:?}");
        // the blue turns grey
        assert!(b[0].abs_diff(b[2]) <= 1, "{b:?}");
        // without the overlay the edit shows
        let e = render(&src, &info, &s, &RenderRequest::fit(32, 16)).image.data[8 * 32 + 4];
        assert!(e[1].abs_diff(px[1]) > 10, "{e:?} vs {px:?}");
    }

    #[test]
    fn mask_overlay_shows_the_evaluated_alpha() {
        use craft_develop_model::{Mask, MaskComponent, MaskOp, MaskShape};
        use craft_geom::Point;
        let src = Rgb32f::from_fn(80, 40, |x, _| if x < 40 { [0.18, 0.18, 0.18] } else { [0.05, 0.1, 0.3] });
        let info = SourceInfo::default();
        let shape = MaskShape::Radial { center: Point::new(0.25, 0.5), rx: 0.15, ry: 0.15, angle: 0.0, feather: 10.0, invert: false };
        let mut s = DevelopSettings::default();
        s.masks.push(Mask { id: 7, components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape }], ..Default::default() });
        let plain = render(&src, &info, &s, &RenderRequest::fit(80, 40)).image;
        let shot = |s: &DevelopSettings, view: MaskView| {
            let o = Overlay::Mask { id: 7, view, color: [255, 0, 0], opacity: 100 };
            render(&src, &info, s, &RenderRequest { overlay: o, ..RenderRequest::fit(80, 40) }).image
        };
        let (inside, outside) = (20 * 80 + 20, 20 * 80 + 70);
        let c = shot(&s, MaskView::Color);
        assert_eq!(c.data[inside], [255, 0, 0, 255], "full tint inside the mask");
        assert_eq!(c.data[outside], plain.data[outside], "untouched outside");
        let bw = shot(&s, MaskView::WhiteOnBlack);
        assert_eq!((bw.data[inside], bw.data[outside]), ([255; 4], [0, 0, 0, 255]));
        let ib = shot(&s, MaskView::ImageOnBlack);
        assert_eq!((ib.data[inside], ib.data[outside]), (plain.data[inside], [0, 0, 0, 255]));
        let iw = shot(&s, MaskView::ImageOnWhite);
        assert_eq!(iw.data[outside], [255; 4]);
        let g = shot(&s, MaskView::ColorOnBw).data[outside];
        assert!(g[0] == g[1] && g[1] == g[2], "grey outside: {g:?}");
        // a hidden mask still shows (it's the one being edited)
        s.masks[0].visible = false;
        assert_eq!(shot(&s, MaskView::WhiteOnBlack).data[inside], [255; 4]);
        // an unknown mask draws nothing
        let o = Overlay::Mask { id: 9, view: MaskView::WhiteOnBlack, color: [0; 3], opacity: 50 };
        assert_eq!(render(&src, &info, &s, &RenderRequest { overlay: o, ..RenderRequest::fit(80, 40) }).image, plain);
        // parts round trip (web worker requests) and distinct cache keys
        let o = Overlay::Mask { id: 513, view: MaskView::ImageOnWhite, color: [12, 200, 255], opacity: 65 };
        let (k, v) = o.to_parts();
        assert_eq!(Overlay::from_parts(k, v), o);
        assert_ne!(o.key(), Overlay::Mask { id: 513, view: MaskView::Color, color: [12, 200, 255], opacity: 65 }.key());
        assert_eq!(MaskView::WhiteOnBlack.next(), MaskView::Color);
        assert_eq!(MaskView::parse("colorOnBw"), Some(MaskView::ColorOnBw));
    }

    #[test]
    fn clipping_views_show_the_clipped_channels() {
        let mut img = Rgba8 {
            width: 5,
            height: 1,
            data: vec![[0, 0, 0, 255], [255, 255, 255, 255], [255, 40, 0, 255], [0, 128, 200, 255], [120, 130, 140, 255]],
        };
        let mut hi = img.clone();
        clipping(&mut hi, true);
        // black background, white where every channel clips, the clipped channels' colour otherwise
        assert_eq!(hi.data, vec![[0, 0, 0, 255], [255; 4], [255, 0, 0, 255], [0, 0, 0, 255], [0, 0, 0, 255]]);
        clipping(&mut img, false);
        // white background, black where every channel clips, the unclipped channels' colour otherwise
        assert_eq!(img.data, vec![[0, 0, 0, 255], [255; 4], [255, 255, 0, 255], [0, 255, 255, 255], [255; 4]]);
    }

    #[test]
    fn clipping_previews_follow_the_rendered_clipping() {
        // left: deep shadow, middle: grey, right: bright highlight
        let src = Rgb32f::from_fn(60, 20, |x, _| match x / 20 {
            0 => [0.0; 3],
            1 => [0.18; 3],
            _ => [3.0; 3],
        });
        let info = SourceInfo::default();
        let mut s = DevelopSettings::default();
        s.light.blacks = -100.0;
        s.light.whites = 100.0;
        let req = |o| RenderRequest { overlay: o, ..RenderRequest::fit(60, 20) };
        let plain = render(&src, &info, &s, &req(Overlay::None));
        let hi = render(&src, &info, &s, &req(Overlay::ClipHighlights));
        let lo = render(&src, &info, &s, &req(Overlay::ClipShadows));
        let at = |i: &Rgba8, x: usize| i.data[10 * 60 + x];
        assert_eq!(at(&plain.image, 55), [255; 4], "the highlight clips");
        assert_eq!(at(&plain.image, 5), [0, 0, 0, 255], "the shadow clips");
        assert_eq!((at(&hi.image, 55), at(&hi.image, 30), at(&hi.image, 5)), ([255; 4], [0, 0, 0, 255], [0, 0, 0, 255]));
        assert_eq!((at(&lo.image, 5), at(&lo.image, 30), at(&lo.image, 55)), ([0, 0, 0, 255], [255; 4], [255; 4]));
        // a preview only: the histogram is the photo's
        assert_eq!(hi.histogram, plain.histogram);
        assert_eq!(render(&src, &info, &s, &req(Overlay::None)).image, plain.image);
    }

    #[test]
    fn sharpening_preview_follows_the_process_radius_and_render_planes() {
        use crate::raster::blur::{gaussian, gaussian_taps};
        use craft_develop_model::Process;
        let l = Plane { width: 80, height: 16, data: (0..1280).map(|i| ((i % 80) as f32 * 0.73).sin() * 0.3).collect() };
        let mut s = DevelopSettings::default();
        let mut bands = Vec::new();
        for process in Process::ALL {
            s.process = process.version();
            s.detail.sharpen_radius = 3.0;
            s.detail.sharpen_masking = 70.0;
            let pixel = crate::local::pixel_sharpen(&s);
            let blur = if pixel {
                let sigma = crate::local::sharpen_sigma(crate::local::sharpen_sigma_raw(&s, 80.0, 160.0));
                gaussian_taps(&l, &crate::raster::blur::gauss_taps(sigma))
            } else {
                gaussian(&l, crate::local::texture_sigma(80.0))
            };
            for o in [Overlay::SharpenDetail, Overlay::SharpenMask] {
                let cached = sharpen_plane(o, &s, &l, Some(&blur), 80.0, 160.0).unwrap();
                let fresh = sharpen_plane(o, &s, &l, None, 80.0, 160.0).unwrap();
                assert_eq!(cached.data, fresh.data, "{process:?} {o:?}");
                for ((&actual, &log), &base) in fresh.data.iter().zip(&l.data).zip(&blur.data) {
                    let det = log - base;
                    let expected = if o == Overlay::SharpenDetail {
                        det
                    } else if pixel {
                        crate::finish::pixel_sharpen_mask_weight(0.7, det)
                    } else {
                        crate::finish::sharpen_mask_weight(0.7, det)
                    };
                    assert_eq!(actual, expected);
                }
            }
            bands.push(sharpen_plane(Overlay::SharpenDetail, &s, &l, None, 80.0, 160.0).unwrap().data);
        }
        assert_eq!(bands[0], bands[1], "the legacy previews stay unchanged");
        assert_ne!(bands[1], bands[2], "V3 previews use the pixel-scale band");
        s.detail.sharpen_radius = 1.0;
        assert_ne!(bands[2], sharpen_plane(Overlay::SharpenDetail, &s, &l, None, 80.0, 160.0).unwrap().data);
    }

    #[test]
    fn sharpening_previews_show_the_mask_and_detail() {
        // a smooth ramp (left) and a hard edge (at x = 60)
        let src = Rgb32f::from_fn(80, 40, |x, _| if x < 60 { [0.05 + 0.002 * x as f32; 3] } else { [0.6; 3] });
        let info = SourceInfo::default();
        let mut s = DevelopSettings::default();
        s.detail.sharpen_amount = 60.0;
        let shot = |s: &DevelopSettings, o| render(&src, &info, s, &RenderRequest { overlay: o, ..RenderRequest::fit(80, 40) }).image;
        let (flat, edge) = (20 * 80 + 20, 20 * 80 + 60);
        // Masking 0 sharpens everything: an all-white mask
        let m = shot(&s, Overlay::SharpenMask);
        assert!(m.data.iter().all(|p| *p == [255; 4]));
        // Masking 100 keeps the edge, protects the ramp
        s.detail.sharpen_masking = 100.0;
        let m = shot(&s, Overlay::SharpenMask);
        assert_eq!((m.data[edge], m.data[flat]), ([255; 4], [0, 0, 0, 255]));
        // the same without sharpening on (the render has no texture plane then)
        s.detail.sharpen_amount = 0.0;
        assert_eq!(shot(&s, Overlay::SharpenMask), m);
        // detail: mid grey on the ramp, light / dark either side of the edge
        let d = shot(&s, Overlay::SharpenDetail);
        assert!(d.data[flat][0].abs_diff(128) <= 3, "{:?}", d.data[flat]);
        assert!(d.data[edge][0] > 200 && d.data[edge - 1][0] < 60, "{:?} {:?}", d.data[edge], d.data[edge - 1]);
        // greyscale
        let src = Rgb32f::from_fn(8, 8, |_, _| [0.4, 0.1, 0.05]);
        let g = render(&src, &info, &s, &RenderRequest { overlay: Overlay::Grayscale, ..RenderRequest::fit(8, 8) }).image;
        assert!(g.data.iter().all(|p| p[0] == p[1] && p[1] == p[2] && p[0] > 0));
    }

    #[test]
    fn alt_previews_map_to_controls_and_round_trip() {
        assert_eq!(Overlay::for_control("light.blacks"), Some(Overlay::ClipShadows));
        assert_eq!(Overlay::for_control("light.shadows"), Some(Overlay::ClipShadows));
        assert_eq!(Overlay::for_control("light.whites"), Some(Overlay::ClipHighlights));
        assert_eq!(Overlay::for_control("light.exposure"), Some(Overlay::ClipHighlights));
        assert_eq!(Overlay::for_control("light.highlights"), Some(Overlay::ClipHighlights));
        assert_eq!(Overlay::for_control("detail.sharpenAmount"), Some(Overlay::Grayscale));
        assert_eq!(Overlay::for_control("detail.sharpenDetail"), Some(Overlay::SharpenDetail));
        assert_eq!(Overlay::for_control("detail.sharpenMasking"), Some(Overlay::SharpenMask));
        for c in ["detail.nrLuminance", "detail.nrDetail", "detail.nrContrast"] {
            assert_eq!(Overlay::for_control(c), Some(Overlay::Grayscale), "{c}");
        }
        assert_eq!(Overlay::for_control("detail.nrColor"), None);
        assert_eq!(Overlay::for_control("light.contrast"), None);
        // every mapped id is a real control (a rename would silently drop its preview)
        for c in [
            "light.exposure",
            "light.highlights",
            "light.whites",
            "light.shadows",
            "light.blacks",
            "detail.sharpenAmount",
            "detail.sharpenRadius",
            "detail.sharpenDetail",
            "detail.sharpenMasking",
            "detail.nrLuminance",
            "detail.nrDetail",
            "detail.nrContrast",
        ] {
            assert!(craft_develop_model::controls::find(c).is_some(), "{c}");
            assert!(Overlay::for_control(c).is_some(), "{c}");
        }
        assert!(Overlay::ClipShadows.is_clipping() && Overlay::ClipHighlights.is_clipping() && !Overlay::Grayscale.is_clipping());
        let all =
            [Overlay::ClipShadows, Overlay::ClipHighlights, Overlay::Grayscale, Overlay::SharpenDetail, Overlay::SharpenMask, Overlay::ClipWarnings];
        for o in all {
            let (k, v) = o.to_parts();
            assert_eq!(Overlay::from_parts(k, v), o);
            assert_eq!(all.iter().filter(|p| p.key() == o.key()).count(), 1);
            assert_ne!(o.key(), Overlay::HdrRange.key());
        }
        assert!(Overlay::ClipShadows.per_pixel() && !Overlay::SharpenDetail.per_pixel());
        assert!(Overlay::ClipWarnings.is_clipping() && Overlay::ClipWarnings.per_pixel());
    }

    #[test]
    fn clip_warnings_paint_clipped_pixels_only() {
        let data = vec![[255, 255, 255, 255], [0, 0, 0, 255], [128, 128, 128, 255], [255, 40, 0, 255], [10, 0, 30, 255]];
        let mut img = Rgba8 { width: 5, height: 1, data };
        super::clip_warnings(&mut img);
        let rgb: Vec<[u8; 3]> = img.data.iter().map(|p| [p[0], p[1], p[2]]).collect();
        assert_eq!(rgb, [super::CLIP_HIGH, super::CLIP_LOW, [128, 128, 128], super::CLIP_HIGH, super::CLIP_LOW]);
    }

    #[test]
    fn all_masks_overlay_shows_every_visible_mask_at_once() {
        use craft_develop_model::{Mask, MaskComponent, MaskOp, MaskShape};
        use craft_geom::Point;
        let src = Rgb32f::from_fn(80, 40, |_, _| [0.1, 0.1, 0.1]);
        let info = SourceInfo::default();
        let left = MaskShape::Radial { center: Point::new(0.25, 0.5), rx: 0.15, ry: 0.15, angle: 0.0, feather: 10.0, invert: false };
        let right = MaskShape::Radial { center: Point::new(0.75, 0.5), rx: 0.15, ry: 0.15, angle: 0.0, feather: 10.0, invert: false };
        let mut s = DevelopSettings::default();
        s.masks.push(Mask {
            id: 1,
            visible: true,
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: left }],
            ..Default::default()
        });
        s.masks.push(Mask {
            id: 2,
            visible: true,
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: right }],
            ..Default::default()
        });
        let o = Overlay::AllMasks { view: MaskView::WhiteOnBlack, color: [255, 0, 0], opacity: 100 };
        let img = render(&src, &info, &s, &RenderRequest { overlay: o, ..RenderRequest::fit(80, 40) }).image;
        let (in_l, in_r, mid) = (20 * 80 + 12, 20 * 80 + 60, 20 * 80 + 40);
        assert_eq!(img.data[in_l], [255; 4], "left mask region painted");
        assert_eq!(img.data[in_r], [255; 4], "right mask region painted");
        assert_eq!(img.data[mid], [0, 0, 0, 255], "between the masks: black");
        // a hidden mask is skipped
        s.masks[1].visible = false;
        let img2 = render(&src, &info, &s, &RenderRequest { overlay: o, ..RenderRequest::fit(80, 40) }).image;
        assert_eq!(img2.data[in_r], [0, 0, 0, 255], "hidden right mask not painted: black like everything outside");
        assert_eq!(img2.data[in_l], [255; 4], "visible left mask still painted");
        // parts round trip and a stable, distinct cache key
        let (k, v) = o.to_parts();
        assert_eq!(Overlay::from_parts(k, v), o);
        assert_ne!(o.key(), Overlay::Mask { id: 1, view: MaskView::WhiteOnBlack, color: [255, 0, 0], opacity: 100 }.key());
    }
}
