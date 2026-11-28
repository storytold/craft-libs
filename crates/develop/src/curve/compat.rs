//! Compatibility operations for projects whose stored adjustments use the Gaussian-region
//! tone curve and unit-slope out-of-range sampling. These are not the newer parametric curve.
//!
//! The formulas came from PhotoCraft's Camera Raw Filter, originally written by Brandon Thomas
//! (storytold/photocraft@aa87012aab6d710b1e1d2ff1654a45e58c09d955).

/// Compile a four-region curve into a monotonically nondecreasing 0…1 table.
///
/// `splits` and `amounts` are percentages. Splits are clamped individually to 5…95, not
/// reordered; the amounts are not clamped. Keeps the legacy expression order for stored edits.
/// Returns an empty table if fewer than two samples are requested.
pub fn gaussian_region_lut(splits: [f32; 3], amounts: [f32; 4], len: usize) -> Vec<f32> {
    if len < 2 {
        return Vec::new();
    }
    let s = splits.map(|v| v.clamp(5.0, 95.0) / 100.0);
    let centres = [s[0] / 2.0, (s[0] + s[1]) / 2.0, (s[1] + s[2]) / 2.0, (s[2] + 1.0) / 2.0];
    let widths = [s[0].max(0.1), (s[1] - s[0]).max(0.1), (s[2] - s[1]).max(0.1), (1.0 - s[2]).max(0.1)];
    let mut lut: Vec<f32> = (0..len)
        .map(|i| {
            let x = i as f32 / (len - 1) as f32;
            let mut y = x;
            for k in 0..4 {
                let d = (x - centres[k]) / (widths[k] * 1.2);
                let bump = (-d * d * 2.0).exp() * x * (1.0 - x) * 4.0;
                y += amounts[k] / 100.0 * 0.25 * bump;
            }
            y.clamp(0.0, 1.0)
        })
        .collect();
    for i in 1..len {
        lut[i] = lut[i].max(lut[i - 1]);
    }
    lut
}

/// Interpolate a table on 0…1 and extend its endpoints with slope one outside that interval.
/// Returns the input unchanged for a table with fewer than two entries.
pub fn sample_lut_linear_tails(lut: &[f32], value: f32) -> f32 {
    if lut.len() < 2 {
        return value;
    }
    if !(0.0..=1.0).contains(&value) {
        return if value < 0.0 { value + lut[0] } else { lut[lut.len() - 1] + (value - 1.0) };
    }
    let x = value * (lut.len() - 1) as f32;
    let i = (x as usize).min(lut.len() - 2);
    let t = x - i as f32;
    lut[i] + (lut[i + 1] - lut[i]) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_region_lut_keeps_legacy_saved_curve_samples() {
        // Synthetic saved sliders and five samples captured with the legacy formula; values are
        // decimal here because the platform exp implementation can round the last bit differently.
        let lut = gaussian_region_lut([90.0, 5.0, 120.0], [-92.0, 64.0, -35.0, 91.0], 5);
        let old = [0.0, 0.030_084_822, 0.330_180_67, 0.543_366_2, 1.0];
        assert_eq!(lut.len(), old.len());
        for (&new, old) in lut.iter().zip(old) {
            assert!((new - old).abs() <= 1.0e-6, "{new} != {old}");
        }
        let crest = gaussian_region_lut([25.0, 50.0, 75.0], [1400.0, 0.0, -1200.0, 0.0], 1024);
        assert!(crest.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!((crest[0], crest[crest.len() - 1]), (0.0, 1.0));
        assert!(gaussian_region_lut([0.0; 3], [0.0; 4], 1).is_empty());
    }

    #[test]
    fn sample_lut_linear_tails_keeps_legacy_signed_and_hdr_values() {
        let lut = [0.125_f32, 0.15625, 0.75];
        for (value, old) in [(-0.5_f32, -0.375_f32), (0.0, 0.125), (0.25, 0.140625), (0.5, 0.15625), (0.75, 0.453125), (1.0, 0.75), (1.5, 1.25)] {
            assert_eq!(sample_lut_linear_tails(&lut, value).to_bits(), old.to_bits());
        }
        assert!(sample_lut_linear_tails(&lut, f32::NAN).is_nan());
        assert_eq!(sample_lut_linear_tails(&[], -0.5), -0.5);
    }
}
