//! Visual ordering of shaped clusters after line bidi levels are resolved.
use std::ops::Range;
use unicode_bidi::{BidiInfo, Level};

/// A glyph's source byte, source length and original resolved level.
/// Zero-length glyphs represent list markers.
#[derive(Clone, Copy)]
pub struct VisualGlyph {
    /// Byte offset in the original text.
    pub byte: usize,
    /// Source length in bytes, or zero for a list marker.
    pub len: usize,
    /// Original level, also used when the line levels lack this byte.
    pub level: Level,
}

/// Order glyph indices by cluster using caller-resolved line levels (UAX #9 L1–L2).
/// `levels` is indexed relative to `paragraph_start`. Adjacent marker glyphs at the
/// same byte form their own cluster and reverse internally at a right-to-left level.
pub fn visual_order(glyphs: impl IntoIterator<Item = VisualGlyph>, paragraph_start: usize, levels: &[Level]) -> Vec<usize> {
    let mut clusters: Vec<(Range<usize>, VisualGlyph)> = Vec::new();
    for (i, glyph) in glyphs.into_iter().enumerate() {
        if let Some((range, _)) = clusters.last_mut().filter(|(_, prev)| prev.byte == glyph.byte && (prev.len == 0) == (glyph.len == 0)) {
            range.end = i + 1;
        } else {
            clusters.push((i..i + 1, glyph));
        }
    }
    let cluster_levels: Vec<_> = clusters
        .iter()
        .map(|(_, g)| if g.len == 0 { g.level } else { levels.get(g.byte.saturating_sub(paragraph_start)).copied().unwrap_or(g.level) })
        .collect();
    BidiInfo::reorder_visual(&cluster_levels)
        .into_iter()
        .filter_map(|i| clusters.get(i))
        .flat_map(|(range, glyph)| {
            let mut indices: Vec<_> = range.clone().collect();
            if glyph.len == 0 && glyph.level.is_rtl() {
                indices.reverse();
            }
            indices
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clusters_and_markers_keep_their_distinct_order() {
        let l = Level::ltr();
        let r = Level::rtl();
        let glyphs = [
            VisualGlyph { byte: 5, len: 0, level: r },
            VisualGlyph { byte: 5, len: 0, level: r },
            VisualGlyph { byte: 5, len: 2, level: r },
            VisualGlyph { byte: 5, len: 2, level: r },
            VisualGlyph { byte: 7, len: 1, level: l },
        ];
        assert_eq!(visual_order(glyphs, 5, &[r, r, l]), vec![2, 3, 1, 0, 4]);
        assert!(visual_order([], 0, &[]).is_empty());
    }
    #[test]
    fn missing_levels_fall_back_to_glyph_level() {
        let r = Level::rtl();
        let glyphs = [VisualGlyph { byte: 2, len: 1, level: r }, VisualGlyph { byte: 3, len: 1, level: r }];
        assert_eq!(visual_order(glyphs, 0, &[]), vec![1, 0]);
    }
}
