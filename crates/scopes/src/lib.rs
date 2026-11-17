//! Video scope math (no UI): the waveform, parade, histogram and the YUV and HLS vectorscopes of
//! a frame, as count grids ("how many samples land on this cell"), plus numeric summaries
//! ([`summary`]) and the density-to-pixels mapping a UI draws ([`paint`]).
//!
//! The input is a [`Signal`]: the frame's R'G'B' code values (display-encoded, 0..1 nominal;
//! values outside it are super-whites and sub-blacks of float or HDR frames), decimated to at most
//! [`MAX_W`] × [`MAX_H`] samples by nearest-sample picking (no averaging, so flat colors and test
//! patterns land on exact cells). [`Signal::fill`] reads a borrowed RGBA8 or NV12 [`Frame`] with
//! any row stride. Y'CbCr comes from the scope's matrix (BT.601, BT.709 or BT.2020 NCL;
//! `craft_color::rgb_to_ycbcr`).
//!
//! | Scope | Grid | Columns × rows |
//! |---|---|---|
//! | Waveform RGB / luma / luma and chroma / luma and chroma with the chroma hidden | per trace: R, G, B / Y / Y and C (Y ± \|CbCr\|) / Y | signal width × `rows` |
//! | Parade RGB / YUV / RGB and luma | per trace: R, G, B / Y, Cb + ½, Cr + ½ / R, G, B, Y | signal width × `rows` (drawn side by side) |
//! | Histogram | 256 bins per channel R, G, B, Y | — |
//! | YUV vectorscope | Cb horizontal, Cr up, ±[`VECTOR_EXTENT`] | `vector_size`² |
//! | HLS vectorscope | hue as the angle (red at the YUV red angle), HLS saturation as the radius (1 = 0.5) | `vector_size`² |
//!
//! [`compute`] picks the waveform and parade traces from [`Params::waveform`] and
//! [`Params::parade`].
//!
//! Waveform rows cover `[lo, hi]` = `[0, 1]` with clamping on (values clamped first) and
//! `[-0.1, 1.1]` without (values outside are dropped). With 256 rows over `[0, 1]`, an 8-bit code
//! value `c` lands exactly on row `c`.
//!
//! Every `*_into` function writes into a value the caller keeps between frames. Once that value
//! and the [`Signal`] have grown to the frame's size, the call does not allocate.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod frame;
pub mod paint;
pub mod summary;
#[cfg(test)]
mod tests;

pub use craft_color::{Matrix, Range};
use craft_color::{normalize_c, normalize_y, rgb_to_ycbcr, ycbcr_to_rgb};
pub use frame::{Frame, FrameError};
use serde::{Deserialize, Serialize};

/// Largest decimated signal (a 1080p frame is sampled every 4th pixel both ways).
pub const MAX_W: usize = 480;
pub const MAX_H: usize = 270;

/// Largest signal [`Signal::fill_max`] and [`Signal::fill_rgba_f32_with`] make, whatever size they
/// are asked for, and the most columns a [`Grid`] holds.
pub const MAX_SIGNAL_W: usize = 1920;
pub const MAX_SIGNAL_H: usize = 1080;

/// Most waveform and parade levels a [`Params`] or a [`Grid`] can hold; larger values are clamped.
pub const MAX_ROWS: usize = 4096;
/// Largest vectorscope grid a [`Params`] can ask for; larger values are clamped.
pub const MAX_VECTOR_SIZE: usize = 1023;

/// Half-width of the vectorscope plane in Cb/Cr units (the 100 % targets of every matrix fit).
pub const VECTOR_EXTENT: f32 = 0.6;

/// The skin-tone line (the "I" axis of NTSC): 123° counter-clockwise from +Cb.
pub const SKIN_TONE_DEG: f32 = 123.0;

// ------------------------------------------------------------------------------------ settings

/// A scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ScopeKind {
    VectorscopeYuv,
    VectorscopeHls,
    Histogram,
    Parade,
    #[default]
    Waveform,
}

impl ScopeKind {
    pub const ALL: [ScopeKind; 5] =
        [ScopeKind::VectorscopeYuv, ScopeKind::VectorscopeHls, ScopeKind::Histogram, ScopeKind::Parade, ScopeKind::Waveform];
    /// Stable id (`"vectorscopeYuv"`…), as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            ScopeKind::VectorscopeYuv => "vectorscopeYuv",
            ScopeKind::VectorscopeHls => "vectorscopeHls",
            ScopeKind::Histogram => "histogram",
            ScopeKind::Parade => "parade",
            ScopeKind::Waveform => "waveform",
        }
    }
    /// English label; an app translates it before showing it.
    pub fn label(self) -> &'static str {
        match self {
            ScopeKind::VectorscopeYuv => "YUV vectorscope",
            ScopeKind::VectorscopeHls => "HLS vectorscope",
            ScopeKind::Histogram => "Histogram",
            ScopeKind::Parade => "Parade",
            ScopeKind::Waveform => "Waveform",
        }
    }
    /// By id or label, ignoring case, spaces and dashes (`"vectorscope"` = the YUV one).
    pub fn from_name(s: &str) -> Option<ScopeKind> {
        let n = norm(s);
        if n == "vectorscope" || n == "yuv" {
            return Some(ScopeKind::VectorscopeYuv);
        }
        if n == "hls" {
            return Some(ScopeKind::VectorscopeHls);
        }
        Self::ALL.into_iter().find(|k| norm(k.name()) == n || norm(k.label()) == n)
    }
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// Waveform type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WaveformType {
    #[default]
    Rgb,
    Luma,
    Yc,
    YcNoChroma,
}

impl WaveformType {
    pub const ALL: [WaveformType; 4] = [WaveformType::Rgb, WaveformType::Luma, WaveformType::Yc, WaveformType::YcNoChroma];
    pub fn label(self) -> &'static str {
        match self {
            WaveformType::Rgb => "RGB",
            WaveformType::Luma => "Luma",
            WaveformType::Yc => "Luma and chroma",
            WaveformType::YcNoChroma => "Luma and chroma, chroma hidden",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        let n = norm(s);
        Self::ALL.into_iter().find(|k| norm(k.label()) == n || norm(&format!("{k:?}")) == n)
    }
}

/// Parade type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParadeType {
    #[default]
    Rgb,
    Yuv,
    RgbWhite,
}

impl ParadeType {
    pub const ALL: [ParadeType; 3] = [ParadeType::Rgb, ParadeType::Yuv, ParadeType::RgbWhite];
    pub fn label(self) -> &'static str {
        match self {
            ParadeType::Rgb => "RGB",
            ParadeType::Yuv => "YUV",
            ParadeType::RgbWhite => "RGB and luma",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        let n = norm(s);
        Self::ALL.into_iter().find(|k| norm(k.label()) == n || norm(&format!("{k:?}")) == n)
    }
}

/// The scopes' color space (the Y'CbCr matrix; Rec. 2100 also means an HDR signal).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorSpace {
    /// Follow the source: Rec. 2100 for an HDR working space, else Rec. 709.
    #[default]
    Auto,
    Rec601,
    Rec709,
    Rec2100,
}

impl ColorSpace {
    pub const ALL: [ColorSpace; 4] = [ColorSpace::Auto, ColorSpace::Rec601, ColorSpace::Rec709, ColorSpace::Rec2100];
    pub fn label(self) -> &'static str {
        match self {
            ColorSpace::Auto => "Automatic",
            ColorSpace::Rec601 => "Rec. 601",
            ColorSpace::Rec709 => "Rec. 709",
            ColorSpace::Rec2100 => "Rec. 2100",
        }
    }
    /// `Auto` resolved for a source (`hdr`: its working space is PQ or HLG).
    pub fn resolve(self, hdr: bool) -> ColorSpace {
        match self {
            ColorSpace::Auto if hdr => ColorSpace::Rec2100,
            ColorSpace::Auto => ColorSpace::Rec709,
            c => c,
        }
    }
    pub fn matrix(self) -> Matrix {
        match self {
            ColorSpace::Rec601 => Matrix::Bt601,
            ColorSpace::Rec2100 => Matrix::Bt2020Ncl,
            _ => Matrix::Bt709,
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        let n = norm(s);
        Self::ALL.into_iter().find(|k| norm(k.label()) == n || norm(&format!("{k:?}")) == n || norm(k.label()).trim_start_matches("rec") == n)
    }
}

/// Signal scale (the bit-depth choice): the labels of the levels axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scale {
    /// 0–255 code values.
    #[default]
    Bits8,
    /// 0.0–1.0.
    Float,
    /// cd/m² on a PQ axis (0–10 000).
    Hdr,
}

impl Scale {
    pub const ALL: [Scale; 3] = [Scale::Bits8, Scale::Float, Scale::Hdr];
    pub fn label(self) -> &'static str {
        match self {
            Scale::Bits8 => "8-bit",
            Scale::Float => "Float",
            Scale::Hdr => "HDR",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        let n = norm(s);
        Self::ALL.into_iter().find(|k| norm(k.label()) == n || norm(&format!("{k:?}")) == n)
    }
}

/// Trace brightness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Brightness {
    Dimmed,
    #[default]
    Normal,
    Bright,
}

impl Brightness {
    pub const ALL: [Brightness; 3] = [Brightness::Dimmed, Brightness::Normal, Brightness::Bright];
    pub fn label(self) -> &'static str {
        match self {
            Brightness::Dimmed => "Dimmed",
            Brightness::Normal => "Normal",
            Brightness::Bright => "Bright",
        }
    }
    /// Multiplier of the trace intensity.
    pub fn gain(self) -> f32 {
        match self {
            Brightness::Dimmed => 0.55,
            Brightness::Normal => 1.0,
            Brightness::Bright => 1.6,
        }
    }
}

/// Which color-bar targets the YUV vectorscope draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Targets {
    #[default]
    #[serde(rename = "75")]
    Percent75,
    #[serde(rename = "100")]
    Percent100,
}

impl Targets {
    pub fn amplitude(self) -> f32 {
        match self {
            Targets::Percent75 => 0.75,
            Targets::Percent100 => 1.0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Targets::Percent75 => "75%",
            Targets::Percent100 => "100%",
        }
    }
}

/// How scopes are computed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub matrix: Matrix,
    /// Clamping: values are clamped to 0..1 before they are plotted.
    pub clamp: bool,
    /// Waveform and parade levels (2..=[`MAX_ROWS`]).
    pub rows: usize,
    /// Vectorscope grid size (square; 3..=[`MAX_VECTOR_SIZE`]). An even size is rounded up to the
    /// next odd one, so the center is a cell.
    pub vector_size: usize,
    /// The traces [`compute`] draws for [`ScopeKind::Waveform`].
    pub waveform: WaveformType,
    /// The traces [`compute`] draws for [`ScopeKind::Parade`].
    pub parade: ParadeType,
}

impl Default for Params {
    fn default() -> Self {
        Params { matrix: Matrix::Bt709, clamp: true, rows: 256, vector_size: 255, waveform: WaveformType::Rgb, parade: ParadeType::Rgb }
    }
}

impl Params {
    /// The plotted value range of waveform rows.
    pub fn range(&self) -> (f32, f32) {
        if self.clamp { (0.0, 1.0) } else { (-0.1, 1.1) }
    }
    fn rows(&self) -> usize {
        self.rows.clamp(2, MAX_ROWS)
    }
    fn vector_size(&self) -> usize {
        (self.vector_size.clamp(3, MAX_VECTOR_SIZE) | 1).min(MAX_VECTOR_SIZE)
    }
}

// ------------------------------------------------------------------------------------ signal

/// A frame's R'G'B' samples (decimated), row-major: `rgb[y * w + x]`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Signal {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<[f32; 3]>,
}

/// Decimation steps for a `w × h` frame read into at most `max_w × max_h` samples (each capped to
/// [`MAX_SIGNAL_W`] × [`MAX_SIGNAL_H`]).
fn steps(w: usize, h: usize, max_w: usize, max_h: usize) -> (usize, usize) {
    let (max_w, max_h) = (max_w.clamp(1, MAX_SIGNAL_W), max_h.clamp(1, MAX_SIGNAL_H));
    (w.div_ceil(max_w).max(1), h.div_ceil(max_h).max(1))
}

impl Signal {
    /// From tightly packed RGBA8 (code / 255), decimated to [`MAX_W`] × [`MAX_H`]. A buffer
    /// shorter than `w * h * 4` gives an empty signal.
    pub fn from_rgba8(w: usize, h: usize, px: &[u8]) -> Signal {
        Self::from_rgba8_max(w, h, px, MAX_W, MAX_H)
    }

    pub fn from_rgba8_max(w: usize, h: usize, px: &[u8], max_w: usize, max_h: usize) -> Signal {
        let mut s = Signal::default();
        let Some(stride) = w.checked_mul(4) else { return s };
        if s.fill_max(&Frame::Rgba8 { width: w, height: h, stride, data: px }, max_w, max_h).is_err() {
            s.clear();
        }
        s
    }

    /// Reads `frame` into this signal, decimated to [`MAX_W`] × [`MAX_H`], reusing its buffer.
    pub fn fill(&mut self, frame: &Frame<'_>) -> Result<(), FrameError> {
        self.fill_max(frame, MAX_W, MAX_H)
    }

    /// Reads `frame` into this signal, decimated to at most `max_w` × `max_h` samples (never more
    /// than [`MAX_SIGNAL_W`] × [`MAX_SIGNAL_H`]). The buffer is reused, so the call does not
    /// allocate once it has held a frame this size. On an error the signal is left empty.
    pub fn fill_max(&mut self, frame: &Frame<'_>, max_w: usize, max_h: usize) -> Result<(), FrameError> {
        self.clear();
        frame.validate()?;
        let (w, h) = (frame.width(), frame.height());
        if w == 0 || h == 0 {
            return Ok(());
        }
        let (sx, sy) = steps(w, h, max_w, max_h);
        let (ow, oh) = (w.div_ceil(sx), h.div_ceil(sy));
        self.rgb.reserve(ow.saturating_mul(oh));
        match *frame {
            Frame::Rgba8 { stride, data, .. } => {
                let lut: [f32; 256] = std::array::from_fn(|i| i as f32 / 255.0);
                for y in (0..h).step_by(sy) {
                    let row = data.get(y * stride..).unwrap_or(&[]);
                    for x in (0..w).step_by(sx) {
                        let c = match row.get(x * 4..x * 4 + 3) {
                            Some(&[r, g, b]) => [lut[r as usize], lut[g as usize], lut[b as usize]],
                            _ => [0.0; 3],
                        };
                        self.rgb.push(c);
                    }
                }
            }
            Frame::Nv12 { y: yp, y_stride, uv, uv_stride, matrix, range, .. } => {
                let ylut: [f32; 256] = std::array::from_fn(|i| normalize_y(i as u32, 8, range).unwrap_or(0.0));
                let clut: [f32; 256] = std::array::from_fn(|i| normalize_c(i as u32, 8, range).unwrap_or(0.0));
                for y in (0..h).step_by(sy) {
                    let yrow = yp.get(y * y_stride..).unwrap_or(&[]);
                    let crow = uv.get((y / 2) * uv_stride..).unwrap_or(&[]);
                    for x in (0..w).step_by(sx) {
                        let c = match (yrow.get(x), crow.get((x / 2) * 2..(x / 2) * 2 + 2)) {
                            (Some(&l), Some(&[cb, cr])) => ycbcr_to_rgb(ylut[l as usize], clut[cb as usize], clut[cr as usize], matrix),
                            _ => [0.0; 3],
                        };
                        self.rgb.push(c);
                    }
                }
            }
        }
        self.w = ow;
        self.h = oh;
        Ok(())
    }

    /// From tightly packed RGBA f32 pixels through `map` (for example linear light to PQ code
    /// values), decimated. A buffer shorter than `w * h * 4` gives an empty signal.
    pub fn from_rgba_f32_with(w: usize, h: usize, px: &[f32], max_w: usize, max_h: usize, map: impl Fn([f32; 4]) -> [f32; 3]) -> Signal {
        let mut s = Signal::default();
        s.fill_rgba_f32_with(w, h, px, max_w, max_h, map);
        s
    }

    /// [`Signal::from_rgba_f32_with`] into this signal, reusing its buffer.
    pub fn fill_rgba_f32_with(&mut self, w: usize, h: usize, px: &[f32], max_w: usize, max_h: usize, map: impl Fn([f32; 4]) -> [f32; 3]) {
        self.clear();
        let needed = w.checked_mul(h).and_then(|n| n.checked_mul(4));
        if w == 0 || h == 0 || needed.is_none_or(|n| px.len() < n) {
            return;
        }
        let (sx, sy) = steps(w, h, max_w, max_h);
        let (ow, oh) = (w.div_ceil(sx), h.div_ceil(sy));
        self.rgb.reserve(ow.saturating_mul(oh));
        for y in (0..h).step_by(sy) {
            for x in (0..w).step_by(sx) {
                let i = (y * w + x) * 4;
                let c = match px.get(i..i + 4) {
                    Some(&[r, g, b, a]) => map([r, g, b, a]),
                    _ => [0.0; 3],
                };
                self.rgb.push(c);
            }
        }
        self.w = ow;
        self.h = oh;
    }

    /// A synthetic signal (tests, generated patterns), at most [`MAX_SIGNAL_W`] × [`MAX_SIGNAL_H`]:
    /// a larger size is cut to that.
    pub fn from_fn(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 3]) -> Signal {
        let (w, h) = (w.min(MAX_SIGNAL_W), h.min(MAX_SIGNAL_H));
        let mut rgb = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                rgb.push(f(x, y));
            }
        }
        Signal { w, h, rgb }
    }

    /// Every sample through `f` (for example SDR code values to PQ for the HDR scale).
    pub fn map(&self, f: impl Fn([f32; 3]) -> [f32; 3]) -> Signal {
        Signal { w: self.w, h: self.h, rgb: self.rgb.iter().map(|c| f(*c)).collect() }
    }

    /// Empties the signal and keeps its buffer.
    pub fn clear(&mut self) {
        self.w = 0;
        self.h = 0;
        self.rgb.clear();
    }

    pub fn len(&self) -> usize {
        self.rgb.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rgb.is_empty()
    }

    /// The rows of samples, each `w` long. A signal whose `w` and `rgb` disagree yields only
    /// whole rows.
    fn rows(&self) -> std::slice::ChunksExact<'_, [f32; 3]> {
        self.rgb.chunks_exact(self.w.max(1))
    }

    /// Columns the grids of this signal have: `w`, or 0 when there are no samples.
    fn cols(&self) -> usize {
        if self.rgb.is_empty() { 0 } else { self.w.min(self.rgb.len()) }
    }
}

/// Linear light (1.0 = reference white = 203 cd/m², premultiplied RGBA) to PQ code values, for
/// HDR scopes: 0..1 on the axis is 0..10 000 cd/m².
pub fn linear_to_pq([r, g, b, a]: [f32; 4]) -> [f32; 3] {
    let a = if a > 1e-6 { a } else { 1.0 };
    let white = craft_color::REFERENCE_WHITE_NITS as f32;
    let f = |v: f32| craft_color::pq_inverse_eotf(((v / a).max(0.0) * white / 10_000.0).clamp(0.0, 1.0));
    [f(r), f(g), f(b)]
}

/// An SDR code value (BT.1886, 100 cd/m² white) on the PQ axis (the HDR scale of an SDR frame).
pub fn sdr_to_pq(v: f32) -> f32 {
    craft_color::pq_inverse_eotf((v.clamp(0.0, 1.0).powf(2.4) * 100.0 / 10_000.0).clamp(0.0, 1.0))
}

/// cd/m² to height on the PQ axis (0..1).
pub fn nits_to_pq(nits: f32) -> f32 {
    craft_color::pq_inverse_eotf((nits / 10_000.0).clamp(0.0, 1.0))
}

// ------------------------------------------------------------------------------------ grids

/// A count grid: `data[row * cols + col]`, row 0 = the lowest level.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Grid {
    pub name: &'static str,
    pub cols: usize,
    pub rows: usize,
    pub data: Vec<u32>,
}

impl Grid {
    pub fn new(name: &'static str, cols: usize, rows: usize) -> Grid {
        let mut g = Grid::default();
        g.reset(name, cols, rows);
        g
    }
    /// Zeroes the grid at a new size (at most [`MAX_SIGNAL_W`] columns and [`MAX_ROWS`] rows),
    /// reusing its buffer.
    pub fn reset(&mut self, name: &'static str, cols: usize, rows: usize) {
        self.name = name;
        self.cols = cols.min(MAX_SIGNAL_W);
        self.rows = rows.min(MAX_ROWS);
        self.data.clear();
        self.data.resize(self.cols * self.rows, 0);
    }
    /// The count of a cell (0 outside the grid).
    pub fn at(&self, col: usize, row: usize) -> u32 {
        if col >= self.cols || row >= self.rows {
            return 0;
        }
        row.checked_mul(self.cols).and_then(|i| i.checked_add(col)).and_then(|i| self.data.get(i)).copied().unwrap_or(0)
    }
    pub fn total(&self) -> u64 {
        self.data.iter().map(|&c| c as u64).sum()
    }
    pub fn max(&self) -> u32 {
        self.data.iter().copied().max().unwrap_or(0)
    }
    /// The rows hit in a column, lowest first.
    pub fn rows_in(&self, col: usize) -> Vec<usize> {
        (0..self.rows).filter(|&r| self.at(col, r) > 0).collect()
    }
    #[inline]
    fn hit(&mut self, col: usize, row: usize) {
        if col >= self.cols || row >= self.rows {
            return;
        }
        if let Some(c) = self.data.get_mut(row * self.cols + col) {
            *c = c.saturating_add(1);
        }
    }
}

/// Waveform or parade: one grid per trace, columns = the signal's width.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Waveform {
    pub lo: f32,
    pub hi: f32,
    /// Levels of every trace.
    pub rows: usize,
    /// Samples per column (the signal's height).
    pub per_column: usize,
    pub traces: Vec<Grid>,
}

impl Waveform {
    pub fn trace(&self, name: &str) -> Option<&Grid> {
        self.traces.iter().find(|t| t.name == name)
    }
    /// The value at the center of a row.
    pub fn level(&self, row: usize) -> f32 {
        self.lo + (self.hi - self.lo) * row as f32 / (self.rows.max(2) - 1) as f32
    }
}

/// Row of value `v` in `[lo, hi]` over `rows` levels (None outside, for NaN, or with fewer than
/// two rows or an empty range).
#[inline]
pub fn row_of(v: f32, lo: f32, hi: f32, rows: usize) -> Option<usize> {
    if rows < 2 || hi <= lo {
        return None;
    }
    let t = (v - lo) / (hi - lo);
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(((t * (rows - 1) as f32).round() as usize).min(rows - 1))
}

/// Up to 4 (trace, value) pairs per sample.
type Hits = ([(u8, f32); 4], usize);

fn build_into(s: &Signal, p: &Params, names: &[&'static str], out: &mut Waveform, sample: impl Fn([f32; 3]) -> Hits) {
    let (lo, hi) = p.range();
    let rows = p.rows();
    let cols = s.cols();
    out.traces.truncate(names.len());
    while out.traces.len() < names.len() {
        out.traces.push(Grid::default());
    }
    for (g, &name) in out.traces.iter_mut().zip(names) {
        g.reset(name, cols, rows);
    }
    let mut per_column = 0;
    if cols > 0 {
        for row in s.rows() {
            per_column += 1;
            for (x, &c) in row.iter().enumerate() {
                let c = if p.clamp { c.map(|v| v.clamp(0.0, 1.0)) } else { c };
                let (hits, n) = sample(c);
                for &(t, v) in hits.get(..n).unwrap_or(&[]) {
                    let v = if p.clamp { v.clamp(0.0, 1.0) } else { v };
                    if let (Some(r), Some(g)) = (row_of(v, lo, hi, rows), out.traces.get_mut(t as usize)) {
                        g.hit(x, r);
                    }
                }
            }
        }
    }
    out.lo = lo;
    out.hi = hi;
    out.rows = rows;
    out.per_column = per_column;
}

/// The waveform of a signal.
pub fn waveform(s: &Signal, kind: WaveformType, p: &Params) -> Waveform {
    let mut w = Waveform::default();
    waveform_into(s, kind, p, &mut w);
    w
}

/// [`waveform`] into `out`, reusing its grids.
pub fn waveform_into(s: &Signal, kind: WaveformType, p: &Params, out: &mut Waveform) {
    let m = p.matrix;
    let z = (0u8, 0.0f32);
    match kind {
        WaveformType::Rgb => build_into(s, p, &["R", "G", "B"], out, |[r, g, b]| ([(0, r), (1, g), (2, b), z], 3)),
        WaveformType::Luma | WaveformType::YcNoChroma => build_into(s, p, &["Y"], out, |[r, g, b]| ([(0, rgb_to_ycbcr(r, g, b, m)[0]), z, z, z], 1)),
        WaveformType::Yc => build_into(s, p, &["Y", "C"], out, |[r, g, b]| {
            let [y, cb, cr] = rgb_to_ycbcr(r, g, b, m);
            let c = (cb * cb + cr * cr).sqrt();
            ([(0, y), (1, y + c), (1, y - c), z], if c > 1e-6 { 3 } else { 2 })
        }),
    }
}

/// The parade of a signal (the traces are drawn side by side).
pub fn parade(s: &Signal, kind: ParadeType, p: &Params) -> Waveform {
    let mut w = Waveform::default();
    parade_into(s, kind, p, &mut w);
    w
}

/// [`parade`] into `out`, reusing its grids.
pub fn parade_into(s: &Signal, kind: ParadeType, p: &Params, out: &mut Waveform) {
    let m = p.matrix;
    let z = (0u8, 0.0f32);
    match kind {
        ParadeType::Rgb => build_into(s, p, &["R", "G", "B"], out, |[r, g, b]| ([(0, r), (1, g), (2, b), z], 3)),
        ParadeType::RgbWhite => {
            build_into(s, p, &["R", "G", "B", "Y"], out, |[r, g, b]| ([(0, r), (1, g), (2, b), (3, rgb_to_ycbcr(r, g, b, m)[0])], 4))
        }
        ParadeType::Yuv => build_into(s, p, &["Y", "Cb", "Cr"], out, |[r, g, b]| {
            let [y, cb, cr] = rgb_to_ycbcr(r, g, b, m);
            ([(0, y), (1, cb + 0.5), (2, cr + 0.5), z], 3)
        }),
    }
}

// ------------------------------------------------------------------------------------ histogram

/// 256-bin histograms of R', G', B' and Y'.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Histogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    pub y: Vec<u32>,
    pub samples: u32,
    /// Samples below 0 or above the top bin per channel (R, G, B, Y). With clamping they are
    /// also counted in the first or last bin of an SDR histogram.
    pub below: [u32; 4],
    pub above: [u32; 4],
    /// How an HDR histogram splits its bins; `None` for an SDR histogram.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hdr: Option<HdrBins>,
    /// HDR histograms: samples with any of R, G and B above SDR white (0 for SDR histograms).
    pub above_sdr_samples: u32,
}

pub const BINS: usize = 256;

impl Histogram {
    fn reset(&mut self, hdr: Option<HdrBins>) {
        for v in [&mut self.r, &mut self.g, &mut self.b, &mut self.y] {
            v.clear();
            v.resize(BINS, 0);
        }
        self.samples = 0;
        self.below = [0; 4];
        self.above = [0; 4];
        self.hdr = hdr;
        self.above_sdr_samples = 0;
    }
    fn channel(&mut self, i: usize) -> &mut Vec<u32> {
        match i {
            0 => &mut self.r,
            1 => &mut self.g,
            2 => &mut self.b,
            _ => &mut self.y,
        }
    }
    fn count(&mut self, i: usize, bin: usize) {
        if let Some(c) = self.channel(i).get_mut(bin) {
            *c = c.saturating_add(1);
        }
    }
}

/// Bin of a value (None when outside 0..1).
#[inline]
pub fn bin_of(v: f32) -> Option<usize> {
    if (0.0..=1.0 + 1e-6).contains(&v) { Some(((v * 255.0).round() as usize).min(BINS - 1)) } else { None }
}

pub fn histogram(s: &Signal, p: &Params) -> Histogram {
    let mut h = Histogram::default();
    histogram_into(s, p, &mut h);
    h
}

/// [`histogram`] into `out`, reusing its bins.
pub fn histogram_into(s: &Signal, p: &Params, out: &mut Histogram) {
    out.reset(None);
    out.samples = u32::try_from(s.len()).unwrap_or(u32::MAX);
    for &[r, g, b] in &s.rgb {
        let y = rgb_to_ycbcr(r, g, b, p.matrix)[0];
        // With clamping, the luma bin comes from the clamped R'G'B', as on the waveform; `below`
        // and `above` count the luma of the sample as it is.
        let y_bin = if p.clamp {
            let [cr, cg, cb] = [r, g, b].map(|v| v.clamp(0.0, 1.0));
            rgb_to_ycbcr(cr, cg, cb, p.matrix)[0]
        } else {
            y
        };
        for (i, (v, at)) in [(r, r), (g, g), (b, b), (y, y_bin)].into_iter().enumerate() {
            if !v.is_nan() && bin_of(v).is_none() {
                let side = if v < 0.0 { &mut out.below[i] } else { &mut out.above[i] };
                *side = side.saturating_add(1);
            }
            match bin_of(at) {
                Some(k) => out.count(i, k),
                None if at.is_nan() || !p.clamp => {}
                None => out.count(i, if at < 0.0 { 0 } else { BINS - 1 }),
            }
        }
    }
}

/// The split of an HDR histogram: bins below `from` cover black to SDR white (sRGB-encoded, as an
/// SDR histogram), and bins `from..256` cover `0..stops` stops above SDR white in equal steps.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HdrBins {
    pub from: usize,
    pub stops: f32,
}

impl HdrBins {
    /// SDR takes the left half of the bins.
    pub const DEFAULT_FROM: usize = BINS / 2;

    /// The default split reaching `stops` stops above SDR white.
    pub fn new(stops: f32) -> HdrBins {
        HdrBins { from: Self::DEFAULT_FROM, stops }
    }

    /// The bin of a linear value (1 = SDR white; NaN and negatives count as black). Any `from`
    /// and any `stops` give a bin inside the histogram.
    pub fn bin(self, v: f32) -> usize {
        let last = BINS - 1;
        let from = self.from.clamp(2, last);
        if v.is_nan() || v <= 0.0 {
            return 0;
        }
        if v <= 1.0 {
            let e = craft_color::linear_to_srgb(v);
            return ((e * (from - 1) as f32 + 0.5) as usize).min(from - 1);
        }
        let t = (v.log2() / self.stops()).clamp(0.0, 1.0);
        (from + (t * (last - from) as f32 + 0.5) as usize).min(last)
    }

    fn stops(self) -> f32 {
        if self.stops.is_finite() { self.stops.max(1e-3) } else { 1e-3 }
    }

    /// The linear value the last bin reaches.
    pub fn top(self) -> f32 {
        self.stops().exp2()
    }
}

/// The HDR histogram of a signal of linear values (1 = SDR white): R, G, B and the luma of
/// `matrix`, split by `bins`. Negative values count in `below`, values past [`HdrBins::top`] in
/// `above`; both also land in the first or last bin.
pub fn histogram_hdr_into(linear: &Signal, matrix: Matrix, bins: HdrBins, out: &mut Histogram) {
    out.reset(Some(bins));
    out.samples = u32::try_from(linear.len()).unwrap_or(u32::MAX);
    let (kr, kb) = matrix.kr_kb();
    let top = bins.top();
    for &[r, g, b] in &linear.rgb {
        if r > 1.0 || g > 1.0 || b > 1.0 {
            out.above_sdr_samples = out.above_sdr_samples.saturating_add(1);
        }
        let y = kr * r + (1.0 - kr - kb) * g + kb * b;
        for (i, v) in [r, g, b, y].into_iter().enumerate() {
            out.count(i, bins.bin(v));
            if v < 0.0 {
                out.below[i] = out.below[i].saturating_add(1);
            } else if v > top {
                out.above[i] = out.above[i].saturating_add(1);
            }
        }
    }
}

/// Fraction of samples with any of R, G and B above SDR white (0 for SDR histograms).
pub fn above_sdr(h: &Histogram) -> f32 {
    if h.hdr.is_none() || h.samples == 0 {
        return 0.0;
    }
    (h.above_sdr_samples as f32 / h.samples as f32).min(1.0)
}

// ------------------------------------------------------------------------------------ vectorscopes

/// A vectorscope: `data[y * size + x]`, x = Cb (right = +), y = Cr (row 0 = top = +Cr).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Vectorscope {
    pub size: usize,
    pub extent: f32,
    pub data: Vec<u32>,
    pub samples: u32,
    /// Mean (Cb, Cr) of the plotted points.
    pub mean: [f32; 2],
    /// The matrix the points were plotted with; [`paint::vectorscope`] colors them with it.
    pub matrix: Matrix,
    /// Whether the points are HLS (hue as the angle from the matrix's red, saturation as the
    /// radius) instead of (Cb, Cr).
    pub hls: bool,
}

/// Cell of a (Cb, Cr) point on a `size`² plane spanning ±`extent` (None outside, for NaN, or for a
/// plane under 2 cells or with no extent).
#[inline]
pub fn cell_of(cb: f32, cr: f32, size: usize, extent: f32) -> Option<(usize, usize)> {
    if size < 2 || extent.is_nan() || extent <= 0.0 {
        return None;
    }
    let n = (size - 1) as f32;
    let fx = (cb / extent * 0.5 + 0.5) * n;
    let fy = (0.5 - cr / extent * 0.5) * n;
    if !(0.0..=n).contains(&fx) || !(0.0..=n).contains(&fy) {
        return None;
    }
    Some((fx.round() as usize, fy.round() as usize))
}

impl Vectorscope {
    fn reset(&mut self, size: usize, matrix: Matrix, hls: bool) {
        self.size = size;
        self.matrix = matrix;
        self.hls = hls;
        self.extent = VECTOR_EXTENT;
        self.data.clear();
        self.data.resize(size.saturating_mul(size), 0);
        self.samples = 0;
        self.mean = [0.0; 2];
    }
    /// The count of a cell (0 outside the plane).
    pub fn at(&self, x: usize, y: usize) -> u32 {
        if x >= self.size || y >= self.size {
            return 0;
        }
        y.checked_mul(self.size).and_then(|i| i.checked_add(x)).and_then(|i| self.data.get(i)).copied().unwrap_or(0)
    }
    pub fn cell(&self, cb: f32, cr: f32) -> Option<(usize, usize)> {
        cell_of(cb, cr, self.size, self.extent)
    }
    /// (Cb, Cr) at the center of a cell.
    pub fn point(&self, x: usize, y: usize) -> [f32; 2] {
        let n = self.size.saturating_sub(1).max(1) as f32;
        [(x as f32 / n - 0.5) * 2.0 * self.extent, (0.5 - y as f32 / n) * 2.0 * self.extent]
    }
    #[inline]
    fn plot(&mut self, cb: f32, cr: f32, sum: &mut [f64; 2]) {
        if let Some((x, y)) = self.cell(cb, cr)
            && let Some(c) = self.data.get_mut(y * self.size + x)
        {
            *c = c.saturating_add(1);
            self.samples = self.samples.saturating_add(1);
            sum[0] += cb as f64;
            sum[1] += cr as f64;
        }
    }
    fn finish(&mut self, sum: [f64; 2]) {
        if self.samples > 0 {
            self.mean = [(sum[0] / self.samples as f64) as f32, (sum[1] / self.samples as f64) as f32];
        }
    }
}

/// The YUV vectorscope (Cb, Cr of each sample).
pub fn vectorscope_yuv(s: &Signal, p: &Params) -> Vectorscope {
    let mut v = Vectorscope::default();
    vectorscope_yuv_into(s, p, &mut v);
    v
}

/// [`vectorscope_yuv`] into `out`, reusing its plane.
pub fn vectorscope_yuv_into(s: &Signal, p: &Params, out: &mut Vectorscope) {
    out.reset(p.vector_size(), p.matrix, false);
    let mut sum = [0f64; 2];
    for &c in &s.rgb {
        let [r, g, b] = if p.clamp { c.map(|x| x.clamp(0.0, 1.0)) } else { c };
        let [_, cb, cr] = rgb_to_ycbcr(r, g, b, p.matrix);
        out.plot(cb, cr, &mut sum);
    }
    out.finish(sum);
}

/// Angle (degrees, counter-clockwise from +Cb, 0..360) of a chroma point.
pub fn angle_deg(cb: f32, cr: f32) -> f32 {
    cr.atan2(cb).to_degrees().rem_euclid(360.0)
}

/// The angle of pure red on a matrix's vectorscope (BT.709 ≈ 103°): the HLS scope puts hue 0 there.
pub fn red_angle(m: Matrix) -> f32 {
    let [_, cb, cr] = rgb_to_ycbcr(1.0, 0.0, 0.0, m);
    angle_deg(cb, cr)
}

/// A sample's point on the HLS vectorscope: hue as the angle from red (counter-clockwise, red,
/// yellow, green, cyan, blue, magenta), HLS saturation as the radius (1 = 0.5).
pub fn hls_point([r, g, b]: [f32; 3], m: Matrix) -> [f32; 2] {
    let [h, s, _] = craft_color::rgb_to_hsl(r, g, b);
    let a = (red_angle(m) + h * 360.0).to_radians();
    let rad = s.clamp(0.0, 1.0) * 0.5;
    [rad * a.cos(), rad * a.sin()]
}

/// Hue steps of the HLS vectorscope's direction table (0.1°).
const HUE_STEPS: usize = 3600;

/// The HLS vectorscope.
pub fn vectorscope_hls(s: &Signal, p: &Params) -> Vectorscope {
    let mut v = Vectorscope::default();
    vectorscope_hls_into(s, p, &mut v);
    v
}

/// [`vectorscope_hls`] into `out`, reusing its plane.
pub fn vectorscope_hls_into(s: &Signal, p: &Params, out: &mut Vectorscope) {
    out.reset(p.vector_size(), p.matrix, true);
    let mut sum = [0f64; 2];
    // hue to unit direction, tabulated at 0.1° on the stack (trigonometry per sample is the slow part)
    let red = red_angle(p.matrix);
    let dirs: [[f32; 2]; HUE_STEPS + 1] = std::array::from_fn(|i| {
        let a = (red + i as f32 / 10.0).to_radians();
        [a.cos(), a.sin()]
    });
    for &c in &s.rgb {
        let [r, g, b] = if p.clamp { c.map(|x| x.clamp(0.0, 1.0)) } else { c };
        let [h, sat, _] = craft_color::rgb_to_hsl(r, g, b);
        let [dx, dy] = dirs.get(((h * HUE_STEPS as f32).round() as usize).min(HUE_STEPS)).copied().unwrap_or([1.0, 0.0]);
        let rad = sat.clamp(0.0, 1.0) * 0.5;
        out.plot(rad * dx, rad * dy, &mut sum);
    }
    out.finish(sum);
}

/// A color-bar target of the YUV vectorscope.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Target {
    pub name: &'static str,
    pub rgb: [f32; 3],
    pub cb: f32,
    pub cr: f32,
}

/// The six color-bar targets (R, Mg, B, Cy, G, Yl) at `amplitude` (0.75 or 1.0).
pub fn targets(m: Matrix, amplitude: f32) -> [Target; 6] {
    let a = amplitude;
    let t = |name, rgb: [f32; 3]| {
        let [_, cb, cr] = rgb_to_ycbcr(rgb[0], rgb[1], rgb[2], m);
        Target { name, rgb, cb, cr }
    };
    [t("R", [a, 0.0, 0.0]), t("Mg", [a, 0.0, a]), t("B", [0.0, 0.0, a]), t("Cy", [0.0, a, a]), t("G", [0.0, a, 0.0]), t("Yl", [a, a, 0.0])]
}

/// Hue labels of the HLS vectorscope at radius 0.5: (name, Cb, Cr).
pub fn hls_targets(m: Matrix) -> [(&'static str, f32, f32); 6] {
    let red = red_angle(m);
    let at = |name, h: f32| {
        let a = (red + h).to_radians();
        (name, 0.5 * a.cos(), 0.5 * a.sin())
    };
    [at("R", 0.0), at("Yl", 60.0), at("G", 120.0), at("Cy", 180.0), at("B", 240.0), at("Mg", 300.0)]
}

// ------------------------------------------------------------------------------------ all scopes

/// One computed scope.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Computed {
    Waveform(Waveform),
    Parade(Waveform),
    Histogram(Histogram),
    Vectorscope(Vectorscope),
}

impl Default for Computed {
    fn default() -> Self {
        Computed::Waveform(Waveform::default())
    }
}

/// Compute one scope.
pub fn compute(kind: ScopeKind, s: &Signal, p: &Params) -> Computed {
    let mut c = Computed::default();
    compute_into(kind, s, p, &mut c);
    c
}

/// [`compute`] into `out`. While `kind` stays the same, the buffers of the previous result are
/// reused.
pub fn compute_into(kind: ScopeKind, s: &Signal, p: &Params, out: &mut Computed) {
    match (kind, &mut *out) {
        (ScopeKind::Waveform, Computed::Waveform(w)) => waveform_into(s, p.waveform, p, w),
        (ScopeKind::Parade, Computed::Parade(w)) => parade_into(s, p.parade, p, w),
        (ScopeKind::Histogram, Computed::Histogram(h)) => histogram_into(s, p, h),
        (ScopeKind::VectorscopeYuv, Computed::Vectorscope(v)) => vectorscope_yuv_into(s, p, v),
        (ScopeKind::VectorscopeHls, Computed::Vectorscope(v)) => vectorscope_hls_into(s, p, v),
        _ => {
            // a different kind: move the old grids over where the shapes match
            let old = std::mem::take(out);
            *out = match (kind, old) {
                (ScopeKind::Waveform, Computed::Parade(w)) => Computed::Waveform(w),
                (ScopeKind::Parade, Computed::Waveform(w)) => Computed::Parade(w),
                (ScopeKind::Waveform, _) => Computed::Waveform(Waveform::default()),
                (ScopeKind::Parade, _) => Computed::Parade(Waveform::default()),
                (ScopeKind::Histogram, _) => Computed::Histogram(Histogram::default()),
                (ScopeKind::VectorscopeYuv | ScopeKind::VectorscopeHls, Computed::Vectorscope(v)) => Computed::Vectorscope(v),
                (ScopeKind::VectorscopeYuv | ScopeKind::VectorscopeHls, _) => Computed::Vectorscope(Vectorscope::default()),
            };
            compute_into(kind, s, p, out);
        }
    }
}
