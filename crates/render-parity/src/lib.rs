//! Render parity: how far two renders of the same photo are apart, and whether that is within a
//! stated [`Tolerance`].
//!
//! The shared develop pipeline (craft-libs#587) has to give LightCraft and PhotoCraft the same
//! picture. A stage moved from an app, or an app switched to the shared crate, is checked by
//! rendering the same fixture both ways and comparing them here. The [`Report`] names the worst
//! pixel, so a failure points at where to look.
//!
//! Renders are scene-linear RGB with Rec. 709 / sRGB primaries and a D65 white, interleaved, with
//! 1.0 as diffuse white. Colour difference is CIEDE2000 in CIELAB relative to D65 (from
//! [`craft_cms::math`]); values above 1.0 are allowed and compared as they are.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

use craft_cms::math::{delta_e2000, xyz_to_lab};

/// Linear Rec. 709 RGB → CIE XYZ (D65), IEC 61966-2-1.
const RGB_TO_XYZ: [[f64; 3]; 3] = [[0.4124, 0.3576, 0.1805], [0.2126, 0.7152, 0.0722], [0.0193, 0.1192, 0.9505]];
/// The D65 white in XYZ, from the matrix above applied to RGB (1, 1, 1).
const D65: [f64; 3] = [0.9505, 1.0, 1.089];

/// A render to compare: `width` × `height` interleaved RGB samples, row by row.
#[derive(Clone, Copy, Debug)]
pub struct Image<'a> {
    width: u32,
    height: u32,
    rgb: &'a [f32],
}

impl<'a> Image<'a> {
    /// A render of `width` × `height` pixels whose `rgb` holds exactly three samples per pixel.
    pub fn new(width: u32, height: u32, rgb: &'a [f32]) -> Result<Self, ParityError> {
        let expected = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(3));
        if expected != Some(rgb.len()) {
            return Err(ParityError::Length { width, height, samples: rgb.len() });
        }
        Ok(Self { width, height, rgb })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

/// Why two renders could not be compared.
#[derive(Clone, Debug, PartialEq)]
pub enum ParityError {
    /// The sample count is not three per pixel.
    Length { width: u32, height: u32, samples: usize },
    /// The renders differ in size, so a pixel of one has no counterpart in the other.
    Size { expected: (u32, u32), actual: (u32, u32) },
    /// A sample is NaN or infinite: no difference can be stated for it.
    NotFinite { x: u32, y: u32 },
}

impl fmt::Display for ParityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { width, height, samples } => write!(f, "{samples} samples is not RGB for {width} × {height} pixels"),
            Self::Size { expected, actual } => write!(f, "render is {} × {}, expected {} × {}", actual.0, actual.1, expected.0, expected.1),
            Self::NotFinite { x, y } => write!(f, "pixel ({x}, {y}) is not a finite colour"),
        }
    }
}

impl std::error::Error for ParityError {}

/// How far two renders may be apart and still count as the same picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// The largest CIEDE2000 difference at any pixel.
    pub max_delta_e: f64,
    /// The largest mean CIEDE2000 difference over the picture.
    pub mean_delta_e: f64,
    /// The largest difference in any one linear channel sample.
    pub max_channel: f32,
}

impl Tolerance {
    /// The baseline parity of craft-libs#587: well below a visible difference (CIEDE2000 1.0), and
    /// within a 10-bit code value in any channel.
    pub const BASELINE: Self = Self { max_delta_e: 0.5, mean_delta_e: 0.05, max_channel: 1.0 / 1024.0 };
}

impl Default for Tolerance {
    fn default() -> Self {
        Self::BASELINE
    }
}

/// The difference between two renders of the same size.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// Pixels compared.
    pub pixels: u64,
    /// The largest CIEDE2000 difference, at [`Self::worst`].
    pub max_delta_e: f64,
    /// The mean CIEDE2000 difference.
    pub mean_delta_e: f64,
    /// The largest absolute difference in any channel sample.
    pub max_channel: f32,
    /// The mean absolute difference per channel (R, G, B).
    pub mean_channel: [f64; 3],
    /// The first pixel with the largest CIEDE2000 difference, when any pixel differs.
    pub worst: Option<(u32, u32)>,
}

impl Report {
    /// Whether every measure is within `tolerance`. A NaN limit is never met.
    pub fn within(&self, tolerance: &Tolerance) -> bool {
        self.exceeded(tolerance).is_empty()
    }

    /// The measures over `tolerance`, each with its value and limit, for a test failure message.
    pub fn exceeded(&self, tolerance: &Tolerance) -> Vec<String> {
        let mut over = Vec::new();
        if exceeds(self.max_delta_e, tolerance.max_delta_e) {
            over.push(format!("max ΔE2000 {:.4} > {}", self.max_delta_e, tolerance.max_delta_e));
        }
        if exceeds(self.mean_delta_e, tolerance.mean_delta_e) {
            over.push(format!("mean ΔE2000 {:.4} > {}", self.mean_delta_e, tolerance.mean_delta_e));
        }
        if exceeds(f64::from(self.max_channel), f64::from(tolerance.max_channel)) {
            over.push(format!("max channel difference {:.6} > {}", self.max_channel, tolerance.max_channel));
        }
        over
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} px: ΔE2000 max {:.4} mean {:.4}, channel max {:.6}", self.pixels, self.max_delta_e, self.mean_delta_e, self.max_channel)?;
        match self.worst {
            Some((x, y)) => write!(f, ", worst at ({x}, {y})"),
            None => write!(f, ", identical"),
        }
    }
}

/// Compare `actual` with `expected`, pixel by pixel.
pub fn compare(expected: &Image<'_>, actual: &Image<'_>) -> Result<Report, ParityError> {
    if (expected.width, expected.height) != (actual.width, actual.height) {
        return Err(ParityError::Size { expected: (expected.width, expected.height), actual: (actual.width, actual.height) });
    }
    let width = expected.width.max(1) as usize;
    let mut report = Report { pixels: 0, max_delta_e: 0.0, mean_delta_e: 0.0, max_channel: 0.0, mean_channel: [0.0; 3], worst: None };
    let mut sum_delta_e = 0.0;
    for (i, (e, a)) in expected.rgb.as_chunks::<3>().0.iter().zip(actual.rgb.as_chunks::<3>().0).enumerate() {
        let (x, y) = ((i % width) as u32, (i / width) as u32);
        let (Some(e), Some(a)) = (finite(*e), finite(*a)) else { return Err(ParityError::NotFinite { x, y }) };
        for c in 0..3 {
            let d = (e[c] - a[c]).abs();
            report.max_channel = report.max_channel.max(d);
            report.mean_channel[c] += f64::from(d);
        }
        let delta_e = delta_e2000(lab(e), lab(a));
        sum_delta_e += delta_e;
        if delta_e > report.max_delta_e {
            report.max_delta_e = delta_e;
            report.worst = Some((x, y));
        }
        report.pixels += 1;
    }
    if report.pixels > 0 {
        let n = report.pixels as f64;
        report.mean_delta_e = sum_delta_e / n;
        report.mean_channel = report.mean_channel.map(|s| s / n);
    }
    if report.worst.is_none() && report.max_channel > 0.0 {
        // channel differences too small to move CIELAB still name a pixel
        report.worst = first_difference(expected, actual);
    }
    Ok(report)
}

/// Whether `value` exceeds `limit`. A NaN limit (or value) exceeds it: an unordered comparison never
/// passes, so an invalid tolerance cannot switch a check off.
fn exceeds(value: f64, limit: f64) -> bool {
    !matches!(value.partial_cmp(&limit), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal))
}

fn finite(px: [f32; 3]) -> Option<[f32; 3]> {
    px.iter().all(|v| v.is_finite()).then_some(px)
}

fn first_difference(expected: &Image<'_>, actual: &Image<'_>) -> Option<(u32, u32)> {
    let width = expected.width.max(1) as usize;
    let i = expected.rgb.as_chunks::<3>().0.iter().zip(actual.rgb.as_chunks::<3>().0).position(|(e, a)| e != a)?;
    Some(((i % width) as u32, (i / width) as u32))
}

fn lab(rgb: [f32; 3]) -> [f64; 3] {
    let rgb = rgb.map(f64::from);
    let xyz = RGB_TO_XYZ.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2]);
    xyz_to_lab(xyz, D65)
}

#[cfg(test)]
mod tests;
