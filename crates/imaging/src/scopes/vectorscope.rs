//! The vectorscope's count grid ([`Vectorscope`], [`HUES`] × [`SATURATIONS`]) and
//! [`hue_saturation`].

/// Hue cells of the vectorscope grid (PhotoCraft's).
pub const HUES: usize = 128;
/// Saturation cells of the vectorscope grid.
pub const SATURATIONS: usize = 64;

/// A bounded hue and saturation distribution: `counts[hue * SATURATIONS + saturation]` pixels in
/// each cell, and the pixels counted. The colours must already be in the app's chosen RGB space;
/// orientation and colour management are the caller's.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Vectorscope {
    /// `HUES × SATURATIONS` counts, hue-major.
    pub counts: Vec<u32>,
    /// Pixels counted (transparent and non-finite ones are skipped).
    pub samples: u64,
}

impl Default for Vectorscope {
    fn default() -> Self {
        Self { counts: vec![0; HUES * SATURATIONS], samples: 0 }
    }
}

/// What a screen reader hears about a vectorscope: the most saturated cell that holds pixels, and
/// the count-weighted circular mean hue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VectorscopeSummary {
    /// The highest saturation with a count, 0..=1.
    pub peak_saturation: f32,
    /// The mean hue in degrees, 0..360 (circular mean weighted by count × saturation), red at 0.
    pub mean_hue: f32,
}

impl Vectorscope {
    /// Counts `pixels` (straight RGBA): transparent (alpha `<= 0`) and non-finite pixels are
    /// skipped; RGB is clamped to 0..=1; the hue cell is `floor(h × HUES)` (at most `HUES − 1`),
    /// the saturation cell `round(s × (SATURATIONS − 1))`; counts saturate. PhotoCraft samples at
    /// most 8192 proxy pixels; the caller picks them.
    pub fn from_rgba(pixels: &[[f32; 4]]) -> Self {
        let mut out = Self::default();
        for pixel in pixels {
            if pixel[3] <= 0.0 || !pixel.iter().all(|v| v.is_finite()) {
                continue;
            }
            let rgb = [pixel[0].clamp(0.0, 1.0), pixel[1].clamp(0.0, 1.0), pixel[2].clamp(0.0, 1.0)];
            let (hue, saturation) = hue_saturation(rgb);
            let hue_cell = ((hue * HUES as f32) as usize).min(HUES - 1);
            let saturation_cell = ((saturation * (SATURATIONS - 1) as f32).round() as usize).min(SATURATIONS - 1);
            if let Some(count) = out.counts.get_mut(hue_cell * SATURATIONS + saturation_cell) {
                *count = count.saturating_add(1);
            }
            out.samples = out.samples.saturating_add(1);
        }
        out
    }

    /// Adds `other`'s counts and samples (saturating), for a scope counted in tiles or jobs.
    pub fn merge(&mut self, other: &Vectorscope) {
        for (count, added) in self.counts.iter_mut().zip(&other.counts) {
            *count = count.saturating_add(*added);
        }
        self.samples = self.samples.saturating_add(other.samples);
    }

    /// The largest count, at least 1: the cloud's alpha is `0.2 + 0.8 × sqrt(count / peak)`.
    pub fn peak(&self) -> u32 {
        self.counts.iter().copied().max().unwrap_or(1).max(1)
    }

    /// The hue (turns, 0..1) and saturation (0..=1) a cell index stands for: `(index /
    /// SATURATIONS) / HUES` and `(index % SATURATIONS) / (SATURATIONS − 1)`.
    pub fn cell(index: usize) -> (f32, f32) {
        ((index / SATURATIONS) as f32 / HUES as f32, (index % SATURATIONS) as f32 / (SATURATIONS - 1) as f32)
    }

    /// The summary for AccessKit; `None` when nothing was counted.
    pub fn summary(&self) -> Option<VectorscopeSummary> {
        let mut peak_saturation: Option<f32> = None;
        let (mut sin, mut cos) = (0.0f64, 0.0f64);
        for (index, count) in self.counts.iter().enumerate().filter(|(_, count)| **count > 0) {
            let (hue, saturation) = Self::cell(index);
            peak_saturation = Some(peak_saturation.map_or(saturation, |peak| peak.max(saturation)));
            let weight = f64::from(*count) * f64::from(saturation);
            let angle = f64::from(hue) * std::f64::consts::TAU;
            sin += weight * angle.sin();
            cos += weight * angle.cos();
        }
        let peak_saturation = peak_saturation?;
        let mean_hue = if sin == 0.0 && cos == 0.0 { 0.0 } else { sin.atan2(cos).to_degrees().rem_euclid(360.0) as f32 };
        Some(VectorscopeSummary { peak_saturation, mean_hue })
    }
}

/// Hue in turns (0..1, red 0, yellow 1/6, green 1/3, blue 2/3) and saturation (0..=1) of an RGB
/// colour; achromatic colours (max − min `<= f32::EPSILON`, or max `<= 0`) are `(0, 0)`.
// pc: crates/algo/src/vectorscope.rs:39 `hue_saturation` (copied; the skeleton writes it, spec "Skeleton parts").
pub fn hue_saturation([r, g, b]: [f32; 3]) -> (f32, f32) {
    let high = r.max(g).max(b);
    let low = r.min(g).min(b);
    let delta = high - low;
    if delta <= f32::EPSILON || high <= 0.0 {
        return (0.0, 0.0);
    }
    let h = if high == r {
        (g - b) / delta
    } else if high == g {
        2.0 + (b - r) / delta
    } else {
        4.0 + (r - g) / delta
    };
    ((h / 6.0).rem_euclid(1.0), delta / high)
}
