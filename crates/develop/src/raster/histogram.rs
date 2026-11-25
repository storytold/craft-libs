//! Histograms of display-encoded images (what the Histogram panel shows).
//!
//! An HDR histogram ([`Histogram::of_hdr`]) keeps the same 256 bins: the first
//! `Histogram::hdr_from` cover SDR black to SDR white (sRGB-encoded, as an SDR histogram), the
//! rest cover the HDR range above SDR white in equal stops.

use serde::Serialize;

use super::Rgba8;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Histogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    pub luma: Vec<u32>,
    pub total: u32,
    /// HDR histograms: the first bin above SDR white, and the stops above SDR white the last bin
    /// reaches. `None` for SDR histograms (every bin is SDR).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hdr: Option<HdrBins>,
}

/// The HDR part of a [`Histogram`]: bins `from..256` cover `0..stops` stops above SDR white.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HdrBins {
    pub from: usize,
    pub stops: f32,
}

impl HdrBins {
    /// The bin of a linear value (1 = SDR white; `NaN` and negatives count as black).
    pub fn bin(self, v: f32) -> usize {
        let last = Histogram::BINS - 1;
        // a split outside 2..=last would leave one side without bins (or underflow)
        let from = self.from.clamp(2, last);
        if v.is_nan() || v <= 0.0 {
            return 0;
        }
        if v <= 1.0 {
            let e = craft_color::linear_to_srgb(v);
            return ((e * (from - 1) as f32 + 0.5) as usize).min(from - 1);
        }
        let stops = if self.stops.is_finite() { self.stops.max(1e-3) } else { 1e-3 };
        let t = (v.log2() / stops).clamp(0.0, 1.0);
        (from + (t * (last - from) as f32 + 0.5) as usize).min(last)
    }
}

impl Histogram {
    pub const BINS: usize = 256;
    /// HDR histograms: the first bin above SDR white (SDR takes the left half).
    pub const HDR_FROM: usize = Self::BINS / 2;

    fn empty(hdr: Option<HdrBins>) -> Histogram {
        Histogram { r: vec![0; 256], g: vec![0; 256], b: vec![0; 256], luma: vec![0; 256], total: 0, hdr }
    }

    /// Histogram of linear RGB samples (1 = SDR white) reaching `stops` stops above SDR white.
    /// `samples` are visited at most ~1 MP of them (evenly strided).
    pub fn of_hdr(samples: &[[f32; 3]], stops: f32) -> Histogram {
        let bins = HdrBins { from: Self::HDR_FROM, stops };
        let mut h = Self::empty(Some(bins));
        let step = (samples.len() / 1_000_000).max(1);
        for c in samples.iter().step_by(step) {
            h.r[bins.bin(c[0])] += 1;
            h.g[bins.bin(c[1])] += 1;
            h.b[bins.bin(c[2])] += 1;
            h.luma[bins.bin(0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2])] += 1;
            h.total += 1;
        }
        h
    }

    /// HDR histogram of per-pixel bins already computed with `bins` (`[r, g, b, luma]` each),
    /// visiting at most ~1 MP of them.
    pub fn of_bins(px: impl ExactSizeIterator<Item = [u8; 4]>, bins: HdrBins) -> Histogram {
        let mut h = Self::empty(Some(bins));
        let step = (px.len() / 1_000_000).max(1);
        for [r, g, b, l] in px.step_by(step) {
            h.r[r as usize] += 1;
            h.g[g as usize] += 1;
            h.b[b as usize] += 1;
            h.luma[l as usize] += 1;
            h.total += 1;
        }
        h
    }

    /// The histogram as flat words (to cross a worker boundary): r, g, b, luma (256 each), the
    /// total, then the first HDR bin (0: SDR) and the HDR stops as `f32` bits.
    pub fn to_words(&self) -> Vec<u32> {
        let mut w = Vec::with_capacity(4 * Self::BINS + 3);
        for ch in [&self.r, &self.g, &self.b, &self.luma] {
            w.extend(ch.iter().copied().chain(std::iter::repeat(0)).take(Self::BINS));
        }
        w.push(self.total);
        let (from, stops) = self.hdr.map_or((0, 0.0), |b| (b.from as u32, b.stops));
        w.extend([from, stops.to_bits()]);
        w
    }

    /// [`Histogram::to_words`] back; also takes the older layout without the HDR words. `None`
    /// for anything malformed.
    pub fn from_words(w: &[u32]) -> Option<Histogram> {
        let n = Self::BINS;
        if w.len() != 4 * n + 1 && w.len() != 4 * n + 3 {
            return None;
        }
        let ch = |k: usize| w.get(k * n..(k + 1) * n).map(<[u32]>::to_vec);
        let hdr = match (w.get(4 * n + 1), w.get(4 * n + 2)) {
            (Some(&from), Some(&bits)) if from > 1 && (from as usize) < n => {
                let stops = f32::from_bits(bits);
                Some(HdrBins { from: from as usize, stops: if stops.is_finite() && stops > 0.0 { stops } else { 1.0 } })
            }
            _ => None,
        };
        Some(Histogram { r: ch(0)?, g: ch(1)?, b: ch(2)?, luma: ch(3)?, total: *w.get(4 * n)?, hdr })
    }

    /// Fraction of pixels above SDR white in any channel (0 for SDR histograms).
    pub fn above_sdr(&self) -> f32 {
        let Some(b) = self.hdr else { return 0.0 };
        if self.total == 0 {
            return 0.0;
        }
        let over = |v: &[u32]| v.iter().skip(b.from).sum::<u32>();
        over(&self.r).max(over(&self.g)).max(over(&self.b)) as f32 / self.total as f32
    }

    pub fn of_srgb8(img: &Rgba8) -> Histogram {
        let mut h = Self::empty(None);
        // Sample at most ~1 MP for speed.
        let step = ((img.len() as f64 / 1_000_000.0).sqrt().ceil() as usize).max(1);
        for y in (0..img.height).step_by(step) {
            for x in (0..img.width).step_by(step) {
                let p = img.get(x, y);
                h.r[p[0] as usize] += 1;
                h.g[p[1] as usize] += 1;
                h.b[p[2] as usize] += 1;
                let l = (p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) >> 8;
                h.luma[l.min(255) as usize] += 1;
                h.total += 1;
            }
        }
        h
    }

    /// Fraction of pixels clipped at black / white in any channel (approximate: per channel max).
    pub fn clipping(&self) -> (f32, f32) {
        if self.total == 0 {
            return (0.0, 0.0);
        }
        let t = self.total as f32;
        let lo = self.r[0].max(self.g[0]).max(self.b[0]) as f32 / t;
        let hi = self.r[255].max(self.g[255]).max(self.b[255]) as f32 / t;
        (lo, hi)
    }

    /// Mean luma in 0..1.
    pub fn mean_luma(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        self.luma.iter().enumerate().map(|(i, &c)| i as f32 * c as f32).sum::<f32>() / (255.0 * self.total as f32)
    }

    /// Luma value (0..1) below which `q` of the pixels fall.
    pub fn percentile(&self, q: f32) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let target = ((q.clamp(0.0, 1.0) * self.total as f32) as u32).max(1);
        let mut acc = 0;
        for (i, &c) in self.luma.iter().enumerate() {
            acc += c;
            if acc >= target {
                return i as f32 / 255.0;
            }
        }
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_stats() {
        let img = Rgba8::from_fn(10, 10, |x, _| if x < 5 { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
        let h = Histogram::of_srgb8(&img);
        assert_eq!(h.total, 100);
        assert_eq!(h.r[0], 50);
        assert_eq!(h.luma[255], 50);
        let (lo, hi) = h.clipping();
        assert!((lo - 0.5).abs() < 1e-6 && (hi - 0.5).abs() < 1e-6);
        assert!((h.mean_luma() - 0.5).abs() < 1e-3);
        assert_eq!(h.percentile(0.25), 0.0);
        assert_eq!(h.percentile(0.75), 1.0);
        assert_eq!(h.above_sdr(), 0.0);
    }

    #[test]
    fn percentile_zero_skips_empty_bins() {
        let img = Rgba8::from_fn(4, 4, |_, _| [200, 200, 200, 255]);
        let h = Histogram::of_srgb8(&img);
        // every pixel is in one bin, so every percentile is that bin's luma, never black
        assert!(h.percentile(1.0) > 0.0);
        assert_eq!(h.percentile(0.0), h.percentile(1.0));
        assert_eq!(h.percentile(0.01), h.percentile(1.0));
    }

    #[test]
    fn percentile_of_empty_histogram_is_zero() {
        let h = Histogram::of_hdr(&[], 4.0);
        assert_eq!(h.percentile(0.0), 0.0);
        assert_eq!(h.percentile(0.5), 0.0);
    }

    #[test]
    fn hdr_bins_split_sdr_and_stops() {
        let px = [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [4.0, 4.0, 4.0], [16.0, 16.0, 16.0], [f32::NAN, -1.0, f32::INFINITY]];
        let h = Histogram::of_hdr(&px, 4.0);
        let b = h.hdr.unwrap();
        assert_eq!(h.total, 5);
        assert_eq!(b.bin(1.0), b.from - 1, "SDR white is the last SDR bin");
        assert_eq!(b.bin(16.0), 255);
        assert_eq!(b.bin(1e9), 255);
        let two = b.bin(4.0);
        assert!(two > b.from && two < 255, "{two}");
        assert_eq!(h.r[0], 2, "black and NaN");
        assert_eq!(h.b[255], 2, "16 and +inf");
        assert!((h.above_sdr() - 0.6).abs() < 1e-6, "{}", h.above_sdr());
        assert_eq!(Histogram::of_hdr(&[], 4.0).above_sdr(), 0.0);
    }

    #[test]
    fn words_round_trip_and_reject_garbage() {
        let img = Rgba8::from_fn(8, 8, |x, y| [(x * 30) as u8, (y * 30) as u8, 9, 255]);
        let sdr = Histogram::of_srgb8(&img);
        assert_eq!(Histogram::from_words(&sdr.to_words()), Some(sdr.clone()));
        let hdr = Histogram::of_hdr(&[[0.5, 2.0, 9.0]], 4.0);
        assert_eq!(Histogram::from_words(&hdr.to_words()), Some(hdr));
        // the older layout (no HDR words) is an SDR histogram
        assert_eq!(Histogram::from_words(&sdr.to_words()[..1025]), Some(sdr.clone()));
        assert_eq!(Histogram::from_words(&[1, 2, 3]), None);
        let mut bad = sdr.to_words();
        bad[1025] = 9999; // first HDR bin out of range: SDR
        bad[1026] = f32::NAN.to_bits();
        assert_eq!(Histogram::from_words(&bad).unwrap().hdr, None);
        bad[1025] = 100; // NaN stops fall back to something drawable
        assert_eq!(Histogram::from_words(&bad).unwrap().hdr.unwrap().stops, 1.0);
    }
    /// A hand-built split outside the bins (any `from`, any `stops`) never panics and stays in range.
    #[test]
    fn hdr_bins_with_any_split_stay_in_range() {
        for from in [0, 1, 2, 128, 255, 256, usize::MAX] {
            for stops in [0.0, 3.0, f32::NAN, f32::INFINITY] {
                let b = HdrBins { from, stops };
                for v in [f32::NAN, -1.0, 0.0, 0.5, 1.0, 2.0, 1e9] {
                    assert!(b.bin(v) < Histogram::BINS, "{from} {stops} {v}");
                }
            }
        }
    }
}
