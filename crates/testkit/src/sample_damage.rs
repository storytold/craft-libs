//! Stable sampled truncations and corruptions for existing parser regression corpora.
//!
//! Unlike `hostile::mutations`, this preserves the historical TAR/EVM fixture sequence.

/// Deterministic pseudo-random bytes (xorshift) for fuzz-like loops.
pub fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// Every way to damage a sample used by robustness tests: truncations at many lengths and
/// single-byte corruptions spread across the file.
pub fn mutations(sample: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let n = sample.len();
    let step = (n / 64).max(1);
    let mut len = 0;
    while len < n {
        out.push(sample[..len].to_vec());
        len += step;
    }
    for (i, seed) in (0..n).step_by(step).zip(1u64..) {
        let mut m = sample.to_vec();
        let noise = noise(seed, 8);
        for (j, v) in noise.iter().enumerate() {
            if let Some(b) = m.get_mut(i + j) {
                *b = *v;
            }
        }
        out.push(m);
        let mut m = sample.to_vec();
        if let Some(b) = m.get_mut(i) {
            *b = 0xff;
        }
        out.push(m);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{mutations, noise};

    #[test]
    fn fixture_sequence_stays_stable() {
        assert_eq!(noise(1, 8), [173, 118, 54, 116, 236, 121, 207, 234]);
        assert!(noise(1, 0).is_empty());
        assert!(mutations(&[]).is_empty());
        assert_eq!(mutations(&[42]), vec![vec![], vec![173], vec![255]]);
    }

    #[test]
    fn sampling_stride_preserves_truncation_and_corruption_order() {
        let sample = vec![42; 128];
        let cases = mutations(&sample);
        assert_eq!(cases.len(), 192);
        for (index, prefix) in cases.iter().take(64).enumerate() {
            assert_eq!(prefix, &sample[..index * 2]);
        }
        for (index, pair) in cases[64..].as_chunks::<2>().0.iter().enumerate() {
            assert_eq!(pair[0].len(), sample.len());
            let mut expected = sample.clone();
            expected[index * 2] = 255;
            assert_eq!(pair[1], expected);
        }
    }
}
