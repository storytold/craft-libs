//! Boundaries between runs that need separate shaping.

/// Tracks the properties of the current shaping segment while walking grapheme clusters.
/// The caller resolves font faces, bidi levels and script classification; marks stay with the
/// preceding segment even when their face, level or case differs.
pub struct SegmentBoundary<F, L, S> {
    face: F,
    level: L,
    small_caps: bool,
    strong_script: Option<S>,
}

impl<F: Copy + Eq, L: Copy + Eq, S: Copy + Eq> SegmentBoundary<F, L, S> {
    /// Begin at the first segment's face and bidi level.
    pub fn new(face: F, level: L) -> Self {
        Self { face, level, small_caps: false, strong_script: None }
    }

    /// Observe a cluster and return whether it begins a new shaping segment. Pass `None` for
    /// common or inherited script and `true` for marks, joiners and variation selectors.
    pub fn advance(&mut self, face: F, level: L, small_caps: bool, strong_script: Option<S>, mark: bool) -> bool {
        let script_change = matches!((self.strong_script, strong_script), (Some(a), Some(b)) if a != b);
        let split = (self.face != face || self.level != level || self.small_caps != small_caps || script_change) && !mark;
        if split {
            self.face = face;
            self.level = level;
            self.small_caps = small_caps;
        }
        if let Some(script) = strong_script {
            self.strong_script = Some(script);
        }
        split
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_marks_and_splits_on_face_direction_case_and_script() {
        let mut state = SegmentBoundary::<u8, u8, u8>::new(1, 0);
        assert!(!state.advance(1, 0, false, Some(1), false));
        assert!(!state.advance(2, 1, true, None, true));
        assert!(state.advance(2, 1, true, Some(2), false));
        assert!(!state.advance(2, 1, true, None, false));
        assert!(state.advance(2, 1, true, Some(1), false));
    }
    #[test]
    fn strong_script_on_a_mark_updates_the_following_boundary() {
        let mut state = SegmentBoundary::<u8, u8, u8>::new(1, 0);
        assert!(!state.advance(1, 0, false, Some(1), false));
        assert!(!state.advance(1, 0, false, Some(2), true));
        assert!(!state.advance(1, 0, false, Some(2), false));
    }
}
