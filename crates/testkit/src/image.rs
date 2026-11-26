//! Image comparison for tests: checked RGBA8 and gray views, difference statistics, PSNR, a
//! tolerance with an optional 3×3 neighborhood, a difference image, PNG reading and writing with
//! size limits, and golden reference checks.
//!
//! Every function returns a [`Result`]. `assert_similar!` is the one form that fails a test, and it
//! panics in the calling test.
//!
//! Statistics compare raw 8-bit sample values. PNG decoding does not apply gamma, ICC or sRGB
//! chunks, so a reference compares as the bytes it stores.
//!
//! The views, statistics, PSNR, tolerance, difference image and in-memory PNG functions build for
//! `wasm32`. The file functions, the bless variable and the golden check are native only.

use std::fmt;
use std::io::Cursor;
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

/// The bless variable `GoldenOptions::new` uses.
pub const DEFAULT_BLESS_VAR: &str = "CRAFT_BLESS";

/// The most failure image pairs [`check_golden`] writes for one name in one folder.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FAILURE_PAIRS: usize = 1000;

/// Why an image could not be compared, read or written.
#[derive(Debug)]
#[non_exhaustive]
pub enum ImageError {
    /// A side is 0, or the stated size does not fit in memory.
    InvalidDimensions { width: u32, height: u32 },
    /// The pixel buffer is not exactly `width × height × channels` bytes.
    InvalidBufferLength { expected: usize, actual: usize },
    /// The two images have different sizes.
    SizeMismatch { a: (u32, u32), b: (u32, u32) },
    /// The two PSNR planes have different sample counts.
    PlaneLengthMismatch { a: usize, b: usize },
    /// A PSNR plane has no samples.
    EmptyPlane,
    /// A tolerance value is NaN or out of range.
    InvalidTolerance(&'static str),
    /// The PSNR peak is not finite or not greater than 0.
    InvalidPeak(f64),
    /// An input or output is larger than its [`PngLimits`] value.
    LimitExceeded(String),
    /// A failure image name or bless variable name is not allowed.
    InvalidName(String),
    /// The reference file does not exist and the bless variable is not `1`.
    MissingReference { path: PathBuf, bless_var: String },
    /// The image is outside the tolerance; the actual and difference images were written.
    GoldenMismatch(Box<GoldenMismatch>),
    /// The image is outside the tolerance, and the actual and difference images could not be
    /// encoded or written.
    GoldenMismatchUnsaved(Box<UnsavedMismatch>),
    /// The PNG stream is malformed, truncated or uses an unsupported form.
    Decode(String),
    /// The PNG encoder failed.
    Encode(String),
    /// A file could not be read or written.
    Io { path: PathBuf, source: std::io::Error },
}

/// The details of a failed golden check.
#[derive(Clone, Debug, PartialEq)]
pub struct GoldenMismatch {
    /// The reference image.
    pub reference: PathBuf,
    /// The comparison that failed.
    pub verdict: Verdict,
    /// The actual image, written inside the failure folder.
    pub actual: PathBuf,
    /// The difference image, written inside the failure folder.
    pub diff: PathBuf,
}

/// The details of a failed golden check whose failure images could not be saved.
#[derive(Debug)]
pub struct UnsavedMismatch {
    /// The reference image.
    pub reference: PathBuf,
    /// The comparison that failed.
    pub verdict: Verdict,
    /// Why the actual or difference image could not be encoded or written.
    pub error: ImageError,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions { width, height } => {
                write!(f, "a {width}x{height} image is not valid: each side must be at least 1 and the buffer size must fit in memory")
            }
            Self::InvalidBufferLength { expected, actual } => {
                write!(f, "the pixel buffer holds {actual} bytes, but the stated size needs {expected}")
            }
            Self::SizeMismatch { a, b } => write!(f, "cannot compare a {}x{} image with a {}x{} one", a.0, a.1, b.0, b.1),
            Self::PlaneLengthMismatch { a, b } => write!(f, "cannot compare a plane of {a} samples with one of {b}"),
            Self::EmptyPlane => write!(f, "PSNR needs at least one sample"),
            Self::InvalidTolerance(why) => write!(f, "invalid tolerance: {why}"),
            Self::InvalidPeak(peak) => write!(f, "invalid PSNR peak {peak}: it must be finite and greater than 0"),
            Self::LimitExceeded(what) | Self::InvalidName(what) => f.write_str(what),
            Self::MissingReference { path, bless_var } => {
                write!(f, "{}: no reference image; run with {bless_var}=1 to write it, and look at it before committing", path.display())
            }
            Self::GoldenMismatch(m) => {
                write!(
                    f,
                    "{}: {}; actual image written to {}, difference image to {}",
                    m.reference.display(),
                    m.verdict,
                    m.actual.display(),
                    m.diff.display()
                )
            }
            Self::GoldenMismatchUnsaved(m) => {
                write!(f, "{}: {}; the actual and difference images could not be written: {}", m.reference.display(), m.verdict, m.error)
            }
            Self::Decode(e) => write!(f, "PNG decoding failed: {e}"),
            Self::Encode(e) => write!(f, "PNG encoding failed: {e}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::GoldenMismatchUnsaved(m) => Some(&m.error),
            _ => None,
        }
    }
}

/// Checks a view's size and returns its pixel count.
fn checked_len(width: u32, height: u32, channels: usize, actual: usize) -> Result<usize, ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::InvalidDimensions { width, height });
    }
    let pixels = usize::try_from(width).ok().zip(usize::try_from(height).ok()).and_then(|(w, h)| w.checked_mul(h));
    let expected = pixels.and_then(|n| n.checked_mul(channels)).ok_or(ImageError::InvalidDimensions { width, height })?;
    if expected != actual {
        return Err(ImageError::InvalidBufferLength { expected, actual });
    }
    Ok(expected / channels)
}

/// A borrowed 8-bit RGBA image, row-major, straight alpha, no row padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba8<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

impl<'a> Rgba8<'a> {
    /// Borrows `pixels` as a `width` × `height` image. Returns an error when a side is 0 or the
    /// buffer is not exactly `width × height × 4` bytes.
    pub fn new(width: u32, height: u32, pixels: &'a [u8]) -> Result<Self, ImageError> {
        checked_len(width, height, 4, pixels.len())?;
        Ok(Self { width, height, pixels })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixel bytes, four per pixel.
    pub fn pixels(&self) -> &'a [u8] {
        self.pixels
    }

    fn plane(self) -> Plane<'a> {
        Plane { width: self.width, height: self.height, channels: 4, pixels: self.pixels }
    }
}

/// A borrowed 8-bit grayscale image, row-major, no row padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gray8<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

impl<'a> Gray8<'a> {
    /// Borrows `pixels` as a `width` × `height` image. Returns an error when a side is 0 or the
    /// buffer is not exactly `width × height` bytes.
    pub fn new(width: u32, height: u32, pixels: &'a [u8]) -> Result<Self, ImageError> {
        checked_len(width, height, 1, pixels.len())?;
        Ok(Self { width, height, pixels })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixel bytes, one per pixel.
    pub fn pixels(&self) -> &'a [u8] {
        self.pixels
    }

    fn plane(self) -> Plane<'a> {
        Plane { width: self.width, height: self.height, channels: 1, pixels: self.pixels }
    }
}

/// A checked view with its channel count.
#[derive(Clone, Copy)]
struct Plane<'a> {
    width: u32,
    height: u32,
    channels: usize,
    pixels: &'a [u8],
}

fn same_size(a: Plane<'_>, b: Plane<'_>) -> Result<(), ImageError> {
    if (a.width, a.height) != (b.width, b.height) {
        return Err(ImageError::SizeMismatch { a: (a.width, a.height), b: (b.width, b.height) });
    }
    Ok(())
}

/// Difference statistics over raw 8-bit samples. RGBA counts alpha as a fourth sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffStats {
    /// Pixels with any sample different.
    pub differing_pixels: usize,
    /// Largest absolute sample difference.
    pub max_abs: u8,
    /// Mean absolute sample difference over every sample.
    pub mean_abs: f64,
    /// The nearest-rank 99th percentile of each pixel's largest sample difference: the value at
    /// one-based position `ceil(0.99 × pixels)` in ascending order.
    pub p99: u8,
    /// PSNR in dB with a peak of 255 over every sample; `f64::INFINITY` when the images are equal.
    pub psnr: f64,
    /// Pixels in each image.
    pub total_pixels: usize,
}

impl fmt::Display for DiffStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PSNR {:.2} dB, max {}, p99 {}, mean {:.3}, {} of {} pixels differ",
            self.psnr, self.max_abs, self.p99, self.mean_abs, self.differing_pixels, self.total_pixels
        )
    }
}

fn max_delta(p: &[u8], q: &[u8]) -> u8 {
    p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

/// The value at one-based position `ceil(count × percent / 100)` of the histogram's samples.
fn nearest_rank(histogram: &[usize; 256], count: usize, percent: u128) -> u8 {
    let rank = (count as u128 * percent).div_ceil(100).max(1);
    let mut seen = 0u128;
    for (value, &n) in histogram.iter().enumerate() {
        seen += n as u128;
        if seen >= rank {
            return u8::try_from(value).unwrap_or(u8::MAX);
        }
    }
    u8::MAX
}

fn psnr_from(squares: u128, samples: usize, peak: f64) -> f64 {
    if squares == 0 {
        return f64::INFINITY;
    }
    let mse = squares as f64 / samples as f64;
    20.0 * peak.log10() - 10.0 * mse.log10()
}

fn stats(a: Plane<'_>, b: Plane<'_>) -> Result<DiffStats, ImageError> {
    same_size(a, b)?;
    let mut histogram = [0usize; 256];
    let (mut sum, mut squares) = (0u128, 0u128);
    let (mut max_abs, mut differing_pixels, mut total_pixels) = (0u8, 0usize, 0usize);
    for (p, q) in a.pixels.chunks_exact(a.channels).zip(b.pixels.chunks_exact(b.channels)) {
        let mut largest = 0u8;
        for (x, y) in p.iter().zip(q) {
            let d = x.abs_diff(*y);
            sum += u128::from(d);
            squares += u128::from(d) * u128::from(d);
            largest = largest.max(d);
        }
        if let Some(n) = histogram.get_mut(usize::from(largest)) {
            *n += 1;
        }
        max_abs = max_abs.max(largest);
        differing_pixels += usize::from(largest > 0);
        total_pixels += 1;
    }
    let samples = a.pixels.len();
    Ok(DiffStats {
        differing_pixels,
        max_abs,
        mean_abs: sum as f64 / samples as f64,
        p99: nearest_rank(&histogram, total_pixels, 99),
        psnr: psnr_from(squares, samples, 255.0),
        total_pixels,
    })
}

/// Statistics for two RGBA images of the same size.
pub fn diff_rgba(a: Rgba8<'_>, b: Rgba8<'_>) -> Result<DiffStats, ImageError> {
    stats(a.plane(), b.plane())
}

/// Statistics for two gray images of the same size.
pub fn diff_gray(a: Gray8<'_>, b: Gray8<'_>) -> Result<DiffStats, ImageError> {
    stats(a.plane(), b.plane())
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
}

/// A sample type [`psnr`] accepts: `u8` or `u16`.
pub trait Sample: Copy + sealed::Sealed {
    /// The sample as an unsigned value.
    fn value(self) -> u32;
}

impl Sample for u8 {
    fn value(self) -> u32 {
        u32::from(self)
    }
}

impl Sample for u16 {
    fn value(self) -> u32 {
        u32::from(self)
    }
}

/// PSNR in dB of two planes: `10 log10(peak² / MSE)`, with `MSE = Σ(a − b)² / samples`.
///
/// The caller supplies `peak`, for example 255 for 8-bit samples, 1023 for 10-bit, 4095 for
/// 12-bit or 65535 for 16-bit. Equal planes return `f64::INFINITY`. Returns an error for planes
/// of different lengths, empty planes, and a peak that is not finite or not greater than 0.
pub fn psnr<T: Sample>(a: &[T], b: &[T], peak: f64) -> Result<f64, ImageError> {
    if !(peak.is_finite() && peak > 0.0) {
        return Err(ImageError::InvalidPeak(peak));
    }
    if a.len() != b.len() {
        return Err(ImageError::PlaneLengthMismatch { a: a.len(), b: b.len() });
    }
    if a.is_empty() {
        return Err(ImageError::EmptyPlane);
    }
    let squares: u128 = a
        .iter()
        .zip(b)
        .map(|(x, y)| {
            let d = u128::from(x.value().abs_diff(y.value()));
            d * d
        })
        .sum();
    Ok(psnr_from(squares, a.len(), peak))
}

/// Acceptance gates for a comparison. Each gate that is set must hold, and every bound is
/// inclusive.
///
/// A pixel is bad when its largest sample difference is greater than `channel`. With
/// `neighborhood`, a pixel is bad only when no pixel in the other image's 3×3 window around it
/// (clipped at the edges) is within `channel`; the check runs in both directions and the larger
/// count is used. `max_abs`, `max_p99` and `min_psnr` always read the aligned statistics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// Largest allowed sample difference.
    pub max_abs: Option<u8>,
    /// Largest allowed [`DiffStats::p99`].
    pub max_p99: Option<u8>,
    /// Smallest allowed PSNR in dB.
    pub min_psnr: Option<f64>,
    /// The sample difference a pixel may have and still not count as bad.
    pub channel: u8,
    /// Largest allowed share of bad pixels, in `0.0..=1.0`; the allowed count is the largest
    /// count whose share of the pixels is at most this value.
    pub max_bad_fraction: Option<f64>,
    /// Match each pixel against the other image's 3×3 window instead of the aligned pixel.
    pub neighborhood: bool,
}

impl Tolerance {
    /// Every sample must be equal.
    pub const EXACT: Tolerance =
        Tolerance { max_abs: Some(0), max_p99: None, min_psnr: None, channel: 0, max_bad_fraction: None, neighborhood: false };

    /// Returns an error when `min_psnr` is NaN or `max_bad_fraction` is NaN or outside
    /// `0.0..=1.0`.
    pub fn validate(&self) -> Result<(), ImageError> {
        if self.min_psnr.is_some_and(f64::is_nan) {
            return Err(ImageError::InvalidTolerance("min_psnr is NaN"));
        }
        if self.max_bad_fraction.is_some_and(|f| !(0.0..=1.0).contains(&f)) {
            return Err(ImageError::InvalidTolerance("max_bad_fraction must be in 0.0..=1.0"));
        }
        Ok(())
    }
}

/// The result of comparing two images under a [`Tolerance`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Verdict {
    /// Aligned statistics.
    pub stats: DiffStats,
    /// Bad pixels under the tolerance's `channel` and `neighborhood` rules.
    pub bad_pixels: usize,
    /// The bad pixels allowed, when the tolerance sets `max_bad_fraction`.
    pub allowed_bad_pixels: Option<usize>,
    /// The tolerance the comparison used.
    pub tolerance: Tolerance,
}

impl Verdict {
    /// Whether every gate of the tolerance holds.
    pub fn passed(&self) -> bool {
        let t = &self.tolerance;
        t.max_abs.is_none_or(|m| self.stats.max_abs <= m)
            && t.max_p99.is_none_or(|m| self.stats.p99 <= m)
            && t.min_psnr.is_none_or(|m| self.stats.psnr >= m)
            && self.allowed_bad_pixels.is_none_or(|n| self.bad_pixels <= n)
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let t = &self.tolerance;
        write!(f, "{}, {} pixels differ by more than {}", self.stats, self.bad_pixels, t.channel)?;
        if t.neighborhood {
            f.write_str(" within a 3x3 neighborhood")?;
        }
        let mut gates = Vec::new();
        if let Some(m) = t.min_psnr {
            gates.push(format!("PSNR >= {m} dB"));
        }
        if let Some(m) = t.max_abs {
            gates.push(format!("max <= {m}"));
        }
        if let Some(m) = t.max_p99 {
            gates.push(format!("p99 <= {m}"));
        }
        if let Some(n) = self.allowed_bad_pixels {
            gates.push(format!("at most {n} such pixels"));
        }
        if !gates.is_empty() {
            write!(f, " (tolerance: {})", gates.join(", "))?;
        }
        Ok(())
    }
}

/// Pixels of `a` with no pixel within `channel` in `b`'s 3×3 window around the same position.
fn unmatched(a: Plane<'_>, b: Plane<'_>, channel: u8) -> usize {
    let (Ok(w), Ok(h)) = (usize::try_from(a.width), usize::try_from(a.height)) else { return 0 };
    let ch = a.channels;
    a.pixels
        .chunks_exact(ch)
        .enumerate()
        .filter(|&(i, p)| {
            let (x, y) = (i % w, i / w);
            let mut rows = y.saturating_sub(1)..=(y + 1).min(h.saturating_sub(1));
            !rows.any(|ny| {
                (x.saturating_sub(1)..=(x + 1).min(w.saturating_sub(1))).any(|nx| {
                    let j = (ny * w + nx) * ch;
                    b.pixels.get(j..j + ch).is_some_and(|q| max_delta(p, q) <= channel)
                })
            })
        })
        .count()
}

/// The largest bad pixel count `n` with `n / total <= fraction`, computed as `f64` division so a
/// share that equals `fraction` is allowed.
fn allowed_bad(fraction: f64, total: usize) -> usize {
    let n = total as f64;
    let mut allowed = ((fraction * n).floor() as usize).min(total);
    while allowed > 0 && allowed as f64 / n > fraction {
        allowed -= 1;
    }
    while allowed < total && (allowed + 1) as f64 / n <= fraction {
        allowed += 1;
    }
    allowed
}

fn compare(a: Plane<'_>, b: Plane<'_>, tolerance: Tolerance) -> Result<Verdict, ImageError> {
    tolerance.validate()?;
    let stats = stats(a, b)?;
    let bad_pixels = if tolerance.neighborhood {
        unmatched(a, b, tolerance.channel).max(unmatched(b, a, tolerance.channel))
    } else {
        a.pixels.chunks_exact(a.channels).zip(b.pixels.chunks_exact(b.channels)).filter(|(p, q)| max_delta(p, q) > tolerance.channel).count()
    };
    let allowed_bad_pixels = tolerance.max_bad_fraction.map(|f| allowed_bad(f, stats.total_pixels));
    Ok(Verdict { stats, bad_pixels, allowed_bad_pixels, tolerance })
}

/// Compares two RGBA images of the same size under `tolerance`.
pub fn compare_rgba(a: Rgba8<'_>, b: Rgba8<'_>, tolerance: Tolerance) -> Result<Verdict, ImageError> {
    compare(a.plane(), b.plane(), tolerance)
}

/// Compares two gray images of the same size under `tolerance`.
pub fn compare_gray(a: Gray8<'_>, b: Gray8<'_>, tolerance: Tolerance) -> Result<Verdict, ImageError> {
    compare(a.plane(), b.plane(), tolerance)
}

/// An opaque RGBA image of the same size showing `|a − b|` × 8 per color channel (at most 255),
/// with each pixel's alpha difference raised into all three color channels.
pub fn diff_image(a: Rgba8<'_>, b: Rgba8<'_>) -> Result<Vec<u8>, ImageError> {
    same_size(a.plane(), b.plane())?;
    let scaled = |x: u8, y: u8| u8::try_from((u16::from(x.abs_diff(y)) * 8).min(255)).unwrap_or(u8::MAX);
    Ok(a.pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .flat_map(|(p, q)| {
            let alpha = scaled(p[3], q[3]);
            [scaled(p[0], q[0]).max(alpha), scaled(p[1], q[1]).max(alpha), scaled(p[2], q[2]).max(alpha), 255]
        })
        .collect())
}

/// Size limits for PNG reading and writing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PngLimits {
    /// Largest width in pixels.
    pub max_width: u32,
    /// Largest height in pixels.
    pub max_height: u32,
    /// Largest encoded PNG, read or written, in bytes.
    pub max_input_bytes: usize,
    /// Largest decoded output in bytes, after conversion to 8-bit gray or RGBA. The returned pixels
    /// never exceed it. The decoder also takes it as a budget for its own working memory, but
    /// applies that budget on a best-effort basis: it allocates small fixed buffers, such as one for
    /// chunk data, before it checks the budget.
    pub max_decoded_bytes: usize,
}

impl PngLimits {
    /// 16,384 × 16,384 pixels, 64 MiB encoded and 256 MiB decoded.
    pub const DEFAULT: PngLimits = PngLimits { max_width: 16_384, max_height: 16_384, max_input_bytes: 64 << 20, max_decoded_bytes: 256 << 20 };
}

impl Default for PngLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A decoded PNG with 8-bit samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodedPng {
    /// Color, gray with alpha, palette, or gray with a `tRNS` chunk, as straight RGBA.
    Rgba { width: u32, height: u32, pixels: Vec<u8> },
    /// Gray without transparency.
    Gray { width: u32, height: u32, pixels: Vec<u8> },
}

impl DecodedPng {
    pub fn width(&self) -> u32 {
        match self {
            Self::Rgba { width, .. } | Self::Gray { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Self::Rgba { height, .. } | Self::Gray { height, .. } => *height,
        }
    }

    /// The pixels as RGBA; gray becomes opaque RGBA with equal color channels.
    pub fn into_rgba(self) -> Vec<u8> {
        match self {
            Self::Rgba { pixels, .. } => pixels,
            Self::Gray { pixels, .. } => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        }
    }
}

fn limit(what: &str, value: impl Into<u128>, max: impl Into<u128>) -> Result<(), ImageError> {
    let (value, max) = (value.into(), max.into());
    if value > max {
        return Err(ImageError::LimitExceeded(format!("{what} {value} exceeds the limit of {max}")));
    }
    Ok(())
}

fn decoding_error(e: png::DecodingError) -> ImageError {
    match e {
        png::DecodingError::LimitsExceeded => ImageError::LimitExceeded("the PNG decoder's working memory exceeds max_decoded_bytes".into()),
        e => ImageError::Decode(e.to_string()),
    }
}

fn to_usize(n: u32) -> Result<usize, ImageError> {
    usize::try_from(n).map_err(|_| ImageError::LimitExceeded(format!("{n} does not fit in memory")))
}

/// Decodes a PNG to 8-bit gray or RGBA.
///
/// Palette images and `tRNS` chunks are expanded, samples under 8 bits are scaled to 8 bits, and
/// 16-bit samples keep their high byte. Interlaced images are returned whole. An animated PNG
/// returns its default image. The chunks after the image, through `IEND`, are read, and the CRC of
/// every chunk, ancillary chunks included, is checked. The input size, the header's width and
/// height, and the decoded size, both as the decoder returns it and after conversion to gray or
/// RGBA, are each checked against `limits` and against the largest buffer the platform can
/// allocate before the buffer for them is allocated.
pub fn decode_png(bytes: &[u8], limits: PngLimits) -> Result<DecodedPng, ImageError> {
    limit("PNG input bytes", bytes.len() as u64, limits.max_input_bytes as u64)?;
    let mut options = png::DecodeOptions::default();
    options.set_skip_ancillary_crc_failures(false);
    let mut decoder = png::Decoder::new_with_options(Cursor::new(bytes), options);
    decoder.set_limits(png::Limits { bytes: limits.max_decoded_bytes });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let (width, height) = {
        let info = decoder.read_header_info().map_err(decoding_error)?;
        (info.width, info.height)
    };
    limit("PNG width", width, limits.max_width)?;
    limit("PNG height", height, limits.max_height)?;
    let mut reader = decoder.read_info().map_err(decoding_error)?;
    let pixels = to_usize(width)?.checked_mul(to_usize(height)?).ok_or(ImageError::InvalidDimensions { width, height })?;
    let channels = match reader.output_color_type().0 {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(ImageError::Decode("the palette was not expanded".into())),
    };
    let size = reader.output_buffer_size().ok_or_else(|| ImageError::LimitExceeded("the decoded PNG does not fit in memory".into()))?;
    limit("decoded PNG bytes", size as u64, limits.max_decoded_bytes as u64)?;
    limit("decoded PNG buffer bytes", size as u64, isize::MAX as u64)?;
    let output = if channels == 1 { pixels as u128 } else { pixels as u128 * 4 };
    limit("decoded PNG bytes", output, limits.max_decoded_bytes as u64)?;
    limit("decoded PNG buffer bytes", output, isize::MAX as u64)?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(decoding_error)?;
    reader.finish().map_err(decoding_error)?;
    if info.bit_depth != png::BitDepth::Eight || (info.width, info.height) != (width, height) {
        return Err(ImageError::Decode(format!("unexpected {:?}-bit {}x{} output", info.bit_depth, info.width, info.height)));
    }
    if info.color_type != reader.output_color_type().0 {
        return Err(ImageError::Decode(format!("unexpected {:?} output", info.color_type)));
    }
    let samples = pixels.checked_mul(channels).filter(|&n| n <= buf.len());
    let Some(samples) = samples else {
        return Err(ImageError::Decode("the decoded frame is shorter than its size".into()));
    };
    if channels == 1 || channels == 4 {
        // The frame is already in its final form: keep the buffer and drop any row padding.
        buf.truncate(samples);
        return Ok(if channels == 1 { DecodedPng::Gray { width, height, pixels: buf } } else { DecodedPng::Rgba { width, height, pixels: buf } });
    }
    let data = buf.get(..samples).unwrap_or_default();
    Ok(match channels {
        2 => DecodedPng::Rgba { width, height, pixels: data.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect() },
        _ => DecodedPng::Rgba { width, height, pixels: data.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect() },
    })
}

/// An output buffer that refuses to grow past `max` bytes.
struct BoundedSink {
    out: Vec<u8>,
    max: usize,
    exceeded: bool,
}

impl std::io::Write for BoundedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.out.len().saturating_add(buf.len()) > self.max {
            self.exceeded = true;
            return Err(std::io::Error::other("the encoded PNG exceeds max_input_bytes"));
        }
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Checks the size of an image to be encoded: its sides, and its pixel bytes as the decoder
/// would return them, so that a written file can be read back under the same limits.
fn check_encode_limits(width: u32, height: u32, decoded_bytes: usize, limits: PngLimits) -> Result<(), ImageError> {
    limit("PNG width", width, limits.max_width)?;
    limit("PNG height", height, limits.max_height)?;
    limit("decoded PNG bytes", decoded_bytes as u64, limits.max_decoded_bytes as u64)
}

fn write_stream(sink: &mut BoundedSink, width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Result<(), png::EncodingError> {
    let mut encoder = png::Encoder::new(sink, width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header()?;
    // The stream writer hands each compressed block to the sink as it fills, so a sink that refuses
    // more bytes stops compression early.
    let mut stream = writer.stream_writer()?;
    std::io::Write::write_all(&mut stream, data)?;
    stream.finish()?;
    writer.finish()
}

/// Encodes `data`, stopping as soon as the output would pass `limits.max_input_bytes`.
fn encode(width: u32, height: u32, color: png::ColorType, data: &[u8], limits: PngLimits) -> Result<Vec<u8>, ImageError> {
    let mut sink = BoundedSink { out: Vec::new(), max: limits.max_input_bytes, exceeded: false };
    match write_stream(&mut sink, width, height, color, data) {
        Ok(()) => Ok(sink.out),
        Err(_) if sink.exceeded => Err(ImageError::LimitExceeded(format!("the encoded PNG exceeds the limit of {} bytes", limits.max_input_bytes))),
        Err(e) => Err(ImageError::Encode(e.to_string())),
    }
}

/// Encodes an RGBA image as an 8-bit PNG: RGB when every pixel is opaque, otherwise RGBA. Returns
/// an error when the image's sides, its pixel bytes or the encoded file are over `limits`.
pub fn encode_png(image: Rgba8<'_>, limits: PngLimits) -> Result<Vec<u8>, ImageError> {
    check_encode_limits(image.width, image.height, image.pixels.len(), limits)?;
    let px = image.pixels.as_chunks::<4>().0;
    if px.iter().all(|p| p[3] == 255) {
        let rgb: Vec<u8> = px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        encode(image.width, image.height, png::ColorType::Rgb, &rgb, limits)
    } else {
        encode(image.width, image.height, png::ColorType::Rgba, image.pixels, limits)
    }
}

/// Encodes a gray image as an 8-bit grayscale PNG. Returns an error when the image's sides, its
/// pixel bytes or the encoded file are over `limits`.
pub fn encode_gray_png(image: Gray8<'_>, limits: PngLimits) -> Result<Vec<u8>, ImageError> {
    check_encode_limits(image.width, image.height, image.pixels.len(), limits)?;
    encode(image.width, image.height, png::ColorType::Grayscale, image.pixels, limits)
}

#[cfg(not(target_arch = "wasm32"))]
fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> ImageError + '_ {
    move |source| ImageError::Io { path: path.to_path_buf(), source }
}

/// Reads and decodes the PNG at `path`. The file size is checked against
/// `limits.max_input_bytes` before it is read.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_png(path: &Path, limits: PngLimits) -> Result<DecodedPng, ImageError> {
    let file = fs::File::open(path).map_err(io_error(path))?;
    let len = file.metadata().map_err(io_error(path))?.len();
    limit("PNG file bytes", len, limits.max_input_bytes as u64)?;
    let mut bytes = Vec::new();
    file.take((limits.max_input_bytes as u64).saturating_add(1)).read_to_end(&mut bytes).map_err(io_error(path))?;
    decode_png(&bytes, limits)
}

/// Writes `bytes` to a new file next to `path`, then renames it over `path`, so a failed write
/// leaves an existing file as it was.
#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), ImageError> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(io_error(dir))?;
    let file_name = path.file_name().ok_or_else(|| ImageError::InvalidName(format!("{} does not name a file", path.display())))?;
    loop {
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(".{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let temp = dir.join(temp_name);
        match create_new(&temp, bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(ImageError::Io { path: temp, source }),
        }
        return fs::rename(&temp, path).map_err(|source| {
            let _ = fs::remove_file(&temp);
            ImageError::Io { path: path.to_path_buf(), source }
        });
    }
}

/// Encodes `image` with [`encode_png`] and writes it to `path`, creating the parent folders.
/// Nothing is written when encoding fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_png(path: &Path, image: Rgba8<'_>, limits: PngLimits) -> Result<(), ImageError> {
    write_file(path, &encode_png(image, limits)?)
}

/// Encodes `image` with [`encode_gray_png`] and writes it to `path`, creating the parent folders.
/// Nothing is written when encoding fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_gray_png(path: &Path, image: Gray8<'_>, limits: PngLimits) -> Result<(), ImageError> {
    write_file(path, &encode_gray_png(image, limits)?)
}

/// Settings for [`check_golden`].
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug)]
pub struct GoldenOptions<'a> {
    /// The variable that blesses references when its value is exactly `1`.
    pub bless_var: &'a str,
    /// The folder that receives the actual and difference images of a failed check.
    pub failures_dir: &'a Path,
    /// The file stem of the failure images: 1 to 100 ASCII letters, digits, `-`, `_` or `.`,
    /// starting with a letter, digit or `_`.
    pub name: &'a str,
    /// Limits for the reference and the written images.
    pub limits: PngLimits,
}

#[cfg(not(target_arch = "wasm32"))]
impl<'a> GoldenOptions<'a> {
    /// Options with [`DEFAULT_BLESS_VAR`] and [`PngLimits::DEFAULT`].
    pub fn new(failures_dir: &'a Path, name: &'a str) -> Self {
        Self { bless_var: DEFAULT_BLESS_VAR, failures_dir, name, limits: PngLimits::DEFAULT }
    }
}

/// Whether a bless variable's value asks to write references: only the exact value `1` does.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_bless_value(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_names(options: &GoldenOptions<'_>) -> Result<(), ImageError> {
    let var = options.bless_var;
    if var.is_empty() || var.contains(['=', '\0']) {
        return Err(ImageError::InvalidName(format!("bless variable name {var:?} must not be empty or contain '=' or NUL")));
    }
    let name = options.name;
    let allowed = name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    let first = name.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let numbered =
        stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes().get(3).is_some_and(|b| (b'1'..=b'9').contains(b));
    let reserved = numbered || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL");
    if name.len() > 100 || !allowed || !first || reserved || name.ends_with('.') {
        return Err(ImageError::InvalidName(format!(
            "failure image name {name:?} must be 1 to 100 ASCII letters, digits, '-', '_' or '.', start with a letter, digit or '_', and not be a device name"
        )));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn create_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    let written = file.write_all(bytes);
    if written.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    written
}

/// Writes the actual and difference images under new names in the failure folder. An existing
/// file is never replaced: `<name>.actual.png`, then `<name>-1.actual.png` and so on.
#[cfg(not(target_arch = "wasm32"))]
fn write_failure(options: &GoldenOptions<'_>, actual: &[u8], diff: &[u8]) -> Result<(PathBuf, PathBuf), ImageError> {
    let dir = options.failures_dir;
    fs::create_dir_all(dir).map_err(io_error(dir))?;
    for n in 0..MAX_FAILURE_PAIRS {
        let stem = if n == 0 { options.name.to_string() } else { format!("{}-{n}", options.name) };
        let actual_path = dir.join(format!("{stem}.actual.png"));
        match create_new(&actual_path, actual) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(ImageError::Io { path: actual_path, source }),
        }
        let diff_path = dir.join(format!("{stem}.diff.png"));
        match create_new(&diff_path, diff) {
            Ok(()) => return Ok((actual_path, diff_path)),
            Err(e) => {
                let _ = fs::remove_file(&actual_path);
                if e.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(ImageError::Io { path: diff_path, source: e });
                }
            }
        }
    }
    Err(ImageError::LimitExceeded(format!("{} already holds {MAX_FAILURE_PAIRS} failure images named {}", dir.display(), options.name)))
}

/// Compares `actual` with the reference PNG at `reference`, or writes the reference when the
/// variable `options.bless_var` is exactly `1`.
///
/// A missing reference returns [`ImageError::MissingReference`] unless blessing. When the
/// comparison fails, the actual and difference images are written as new files inside
/// `options.failures_dir`, and [`ImageError::GoldenMismatch`] names them. When they cannot be
/// encoded or written, [`ImageError::GoldenMismatchUnsaved`] carries the verdict together with
/// that error. A reference that would
/// be larger than `options.limits.max_input_bytes` is not written.
#[cfg(not(target_arch = "wasm32"))]
pub fn check_golden(reference: &Path, actual: Rgba8<'_>, tolerance: Tolerance, options: GoldenOptions<'_>) -> Result<Verdict, ImageError> {
    validate_names(&options)?;
    let bless = is_bless_value(std::env::var_os(options.bless_var).as_deref());
    check_golden_with(reference, actual, tolerance, options, bless)
}

#[cfg(not(target_arch = "wasm32"))]
fn check_golden_with(
    reference: &Path,
    actual: Rgba8<'_>,
    tolerance: Tolerance,
    options: GoldenOptions<'_>,
    bless: bool,
) -> Result<Verdict, ImageError> {
    validate_names(&options)?;
    tolerance.validate()?;
    if bless {
        write_png(reference, actual, options.limits)?;
        return compare_rgba(actual, actual, tolerance);
    }
    let decoded = match read_png(reference, options.limits) {
        Ok(decoded) => decoded,
        Err(ImageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(ImageError::MissingReference { path: reference.to_path_buf(), bless_var: options.bless_var.to_string() });
        }
        Err(e) => return Err(e),
    };
    let (width, height) = (decoded.width(), decoded.height());
    if (width, height) != (actual.width, actual.height) {
        return Err(ImageError::SizeMismatch { a: (width, height), b: (actual.width, actual.height) });
    }
    limit("decoded PNG bytes", actual.pixels.len() as u64, options.limits.max_decoded_bytes as u64)?;
    let pixels = decoded.into_rgba();
    let expected = Rgba8::new(width, height, &pixels)?;
    let verdict = compare_rgba(expected, actual, tolerance)?;
    if verdict.passed() {
        return Ok(verdict);
    }
    let saved = diff_image(expected, actual).and_then(|diff| {
        let actual_png = encode_png(actual, options.limits)?;
        let diff_png = encode_png(Rgba8::new(width, height, &diff)?, options.limits)?;
        write_failure(&options, &actual_png, &diff_png)
    });
    let reference = reference.to_path_buf();
    Err(match saved {
        Ok((actual, diff)) => ImageError::GoldenMismatch(Box::new(GoldenMismatch { reference, verdict, actual, diff })),
        Err(error) => ImageError::GoldenMismatchUnsaved(Box::new(UnsavedMismatch { reference, verdict, error })),
    })
}

/// Fails the test that called [`assert_similar!`](crate::assert_similar); the panic names the
/// caller's location.
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
#[track_caller]
#[allow(clippy::panic)]
pub fn fail_similar(error: ImageError) -> ! {
    panic!("{error}")
}

/// Runs [`check_golden`](crate::image::check_golden) and fails the calling test when it returns
/// an error. Evaluates to the [`Verdict`](crate::image::Verdict).
///
/// ```ignore
/// let options = craft_testkit::image::GoldenOptions::new(&failures, "title_card");
/// craft_testkit::assert_similar!(&reference, actual, tolerance, options);
/// ```
#[cfg(not(target_arch = "wasm32"))]
#[macro_export]
macro_rules! assert_similar {
    ($reference:expr, $actual:expr, $tolerance:expr, $options:expr $(,)?) => {
        match $crate::image::check_golden($reference, $actual, $tolerance, $options) {
            ::core::result::Result::Ok(verdict) => verdict,
            ::core::result::Result::Err(error) => $crate::image::fail_similar(error),
        }
    };
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;
