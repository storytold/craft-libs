//! The Whisper mel filterbank.
//!
//! OpenAI's Whisper takes an 80-bin log-mel spectrogram built with the Slaney mel scale
//! (librosa's `mel` defaults: `htk=False`, `norm="slaney"`, `fmin=0`, `fmax=sr/2`) from a 25 ms
//! Hann window every 10 ms. The filters are a fixed function of the sample rate and FFT size, so
//! they are computed here rather than shipped as an asset.

/// The number of FFT bins a 400-point transform has (0 Hz through Nyquist).
pub const FFT_BINS: usize = 400 / 2 + 1;

/// The largest mel filter table this build will allocate. Whisper uses 80 (tiny/base) or 128
/// (large-v3); a config asking for more is refused rather than allocated.
pub const MAX_MEL_BINS: usize = 128;

/// The Slaney mel of a frequency in Hz.
pub fn hz_to_mel(hz: f64) -> f64 {
    const F_SP: f64 = 200.0 / 3.0;
    const MIN_LOG_HZ: f64 = 1000.0;
    const MIN_LOG_MEL: f64 = MIN_LOG_HZ / F_SP;
    // ln(6.4) / 27, the step of the logarithmic part of the Slaney scale.
    const LOGSTEP: f64 = 0.068_751_777_420_949_1;
    if hz < MIN_LOG_HZ { hz / F_SP } else { MIN_LOG_MEL + (hz / MIN_LOG_HZ).ln() / LOGSTEP }
}

/// The frequency in Hz of a Slaney mel.
pub fn mel_to_hz(mel: f64) -> f64 {
    const F_SP: f64 = 200.0 / 3.0;
    const MIN_LOG_HZ: f64 = 1000.0;
    const MIN_LOG_MEL: f64 = MIN_LOG_HZ / F_SP;
    // ln(6.4) / 27, the step of the logarithmic part of the Slaney scale.
    const LOGSTEP: f64 = 0.068_751_777_420_949_1;
    if mel < MIN_LOG_MEL { mel * F_SP } else { MIN_LOG_HZ * (LOGSTEP * (mel - MIN_LOG_MEL)).exp() }
}

/// The `n_mel × FFT_BINS` Slaney-normalised triangular filters, row-major as
/// `whisper::audio::pcm_to_mel` expects. A count outside `1..=MAX_MEL_BINS` yields nothing.
pub fn mel_filters(n_mel: usize) -> Vec<f32> {
    let sample_rate = 16_000.0;
    let bins = FFT_BINS;
    if n_mel == 0 || n_mel > MAX_MEL_BINS {
        return Vec::new();
    }
    let Some(len) = n_mel.checked_mul(bins) else { return Vec::new() };
    let freqs: Vec<f64> = (0..=bins).map(|index| index as f64 * sample_rate / 400.0).collect();
    let mel_min = hz_to_mel(0.0);
    let mel_max = hz_to_mel(sample_rate / 2.0);
    let points: Vec<f64> = (0..n_mel + 2).map(|index| mel_to_hz(mel_min + (mel_max - mel_min) * index as f64 / (n_mel + 1) as f64)).collect();
    let mut out = vec![0.0f32; len];
    for mel in 0..n_mel {
        let (low, center, high) = (points[mel], points[mel + 1], points[mel + 2]);
        let enorm = 2.0 / (high - low).max(f64::EPSILON);
        for (bin, frequency) in freqs.iter().take(bins).enumerate() {
            let lower = (frequency - low) / (center - low).max(f64::EPSILON);
            let upper = (high - frequency) / (high - center).max(f64::EPSILON);
            let weight = lower.min(upper).max(0.0) * enorm;
            if let Some(slot) = out.get_mut(mel * bins + bin) {
                *slot = weight as f32;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mel_scale_round_trips() {
        for hz in [0.0, 100.0, 999.9, 1000.0, 2000.0, 8000.0] {
            let back = mel_to_hz(hz_to_mel(hz));
            assert!((back - hz).abs() < 1e-6, "{hz} -> {back}");
        }
    }

    #[test]
    fn eighty_filters_cover_the_spectrum_and_a_hostile_count_allocates_nothing() {
        let filters = mel_filters(80);
        assert_eq!(filters.len(), 80 * FFT_BINS);
        assert!(filters.iter().all(|value| value.is_finite() && *value >= 0.0));
        for row in filters.chunks(FFT_BINS) {
            assert!(row.iter().any(|value| *value > 0.0), "every filter has a peak");
        }
        let first_peak = filters[..FFT_BINS].iter().position(|value| *value > 0.0).unwrap_or(usize::MAX);
        let last_start = 79 * FFT_BINS;
        let last_peak = filters[last_start..].iter().rposition(|value| *value > 0.0).unwrap_or(0);
        assert!(first_peak < 5, "{first_peak}");
        assert!(last_peak > 150, "{last_peak}");
        assert_eq!(mel_filters(0), Vec::<f32>::new());
        assert_eq!(mel_filters(MAX_MEL_BINS + 1), Vec::<f32>::new(), "an oversized table is refused");
        assert_eq!(mel_filters(usize::MAX), Vec::<f32>::new(), "a hostile count allocates nothing");
        assert_eq!(mel_filters(128).len(), 128 * FFT_BINS);
    }
}
