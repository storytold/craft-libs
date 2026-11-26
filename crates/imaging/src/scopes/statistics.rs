//! [`histogram_statistics`]: the Histogram panel's Mean, Std Dev, Median, Pixels, Level, Count
//! and Percentile.

/// The statistics of one channel's histogram. Mean, standard deviation, median and pixels describe
/// the whole histogram; level, count and percentile the inspected level or range.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistogramStatistics {
    /// The mean level.
    pub mean: f64,
    /// The standard deviation of the level (Photoshop's Std Dev), over the whole population.
    pub standard_deviation: f64,
    /// The first level at which the running count reaches half the pixels.
    pub median: u8,
    /// The pixels counted.
    pub pixels: u64,
    /// The inspected levels `lo..=hi` (one level when equal), in order; `None` when nothing is
    /// inspected.
    pub level: Option<[u8; 2]>,
    /// Pixels at the inspected levels.
    pub count: Option<u64>,
    /// For one level, percent of the pixels at or below it; for a range, percent in the range.
    pub percentile: Option<f64>,
    /// Percent of the pixels at level 0 and at the last level: shadow and highlight clipping.
    pub clipped: [f64; 2],
}

/// The statistics of `bins` (one count per level, usually 256), inspecting `levels` (a level or a
/// range, either order; clamped to the bins). `None` when nothing was counted.
///
/// Rules (PhotoCraft's `stats`): mean `Σ i·n_i / N`; variance `Σ (i − mean)²·n_i / N`; median the
/// first `i` where the running count × 2 `>= N`; count of `lo..=hi` the sum of those bins;
/// percentile `100 × sum(0..=hi) / N` when `lo == hi`, else `100 × sum(lo..=hi) / N`; clipped
/// `100 × n_0 / N` and `100 × n_last / N`.
pub fn histogram_statistics(bins: &[u32], levels: Option<[u8; 2]>) -> Option<HistogramStatistics> {
    let total: u64 = bins.iter().map(|count| u64::from(*count)).sum();
    if total == 0 {
        return None;
    }
    let total_f = total as f64;
    let mean = bins.iter().enumerate().map(|(level, count)| level as f64 * f64::from(*count)).sum::<f64>() / total_f;
    let variance = bins.iter().enumerate().map(|(level, count)| (level as f64 - mean).powi(2) * f64::from(*count)).sum::<f64>() / total_f;
    let mut running = 0u64;
    let median = bins
        .iter()
        .position(|count| {
            running += u64::from(*count);
            running * 2 >= total
        })
        .unwrap_or(0);
    let last = bins.len() - 1;
    let top = u8::try_from(last).unwrap_or(u8::MAX);
    let level = levels.map(|[a, b]| [a.min(b).min(top), a.max(b).min(top)]);
    let sum = |lo: u8, hi: u8| bins[usize::from(lo)..=usize::from(hi)].iter().map(|count| u64::from(*count)).sum::<u64>();
    let count = level.map(|[lo, hi]| sum(lo, hi));
    let percentile = level.map(|[lo, hi]| 100.0 * (if lo == hi { sum(0, hi) } else { sum(lo, hi) }) as f64 / total_f);
    let clipped = [100.0 * f64::from(bins[0]) / total_f, 100.0 * f64::from(bins[last]) / total_f];
    Some(HistogramStatistics {
        mean,
        standard_deviation: variance.sqrt(),
        median: u8::try_from(median).unwrap_or(u8::MAX),
        pixels: total,
        level,
        count,
        percentile,
        clipped,
    })
}
