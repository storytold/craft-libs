//! Map ligature caret fractions to grapheme boundaries.
use unicode_segmentation::UnicodeSegmentation;

/// Return logical source-byte caret stops inside a shaped cluster. `font_carets` receives the
/// number of stops and may return their normalized positions along the glyph's logical width.
/// Missing or incorrectly sized font data falls back to equal divisions of the width.
pub fn ligature_caret_stops(text: &str, byte: usize, len: usize, font_carets: impl FnOnce(usize) -> Option<Vec<f64>>) -> Vec<(usize, f64)> {
    let Some(s) = byte.checked_add(len).and_then(|end| text.get(byte..end)).filter(|_| len > 1) else { return Vec::new() };
    let starts: Vec<usize> = s.grapheme_indices(true).skip(1).map(|(i, _)| byte + i).collect();
    if starts.is_empty() {
        return Vec::new();
    }
    let parts = starts.len() + 1;
    let mut at =
        font_carets(starts.len()).filter(|c| c.len() == starts.len()).unwrap_or_else(|| (1..parts).map(|j| j as f64 / parts as f64).collect());
    at.sort_by(f64::total_cmp);
    starts.into_iter().zip(at).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_graphemes_and_sorts_font_positions() {
        assert_eq!(ligature_caret_stops("ábc", 0, 5, |_| Some(vec![0.8, 0.3])), vec![(3, 0.3), (4, 0.8)]);
    }
    #[test]
    fn falls_back_and_rejects_invalid_ranges() {
        assert_eq!(ligature_caret_stops("abc", 0, 3, |_| Some(vec![0.5])), vec![(1, 1.0 / 3.0), (2, 2.0 / 3.0)]);
        assert!(ligature_caret_stops("abc", usize::MAX, 2, |_| None).is_empty());
        assert!(ligature_caret_stops("é", 1, 1, |_| None).is_empty());
    }
}
