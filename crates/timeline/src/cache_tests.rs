//! The frame cache.

use craft_ui::testing::trial;
use std::sync::Arc;

use crate::cache::{FrameCache, FrameCacheKey, FramePlayhead, FrameSize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TestKey {
    pub series: u8,
    pub frame: i64,
    pub scale: u8,
}

impl TestKey {
    pub(crate) fn new(series: u8, frame: i64) -> Self {
        Self { series, frame, scale: 1 }
    }
}

impl FrameCacheKey for TestKey {
    fn frame(&self) -> i64 {
        self.frame
    }

    fn at_frame(&self, frame: i64) -> Self {
        Self { frame, ..*self }
    }

    fn same_series(&self, other: &Self) -> bool {
        self.series == other.series && self.scale == other.scale
    }

    fn same_scale(&self, other: &Self) -> bool {
        self.scale == other.scale
    }
}

#[derive(Debug)]
pub(crate) struct TestFrame(pub usize);

impl FrameSize for TestFrame {
    fn bytes(&self) -> usize {
        self.0
    }
}

fn fill(cache: &mut FrameCache<TestKey, TestFrame>, frames: impl IntoIterator<Item = i64>) {
    for f in frames {
        cache.insert(TestKey::new(0, f), Arc::new(TestFrame(10)));
    }
}

fn playhead(current: i64, start: i64, end: i64, looping: bool) -> FramePlayhead<TestKey> {
    FramePlayhead { series: TestKey::new(0, 0), current, start, end, step: 1, looping }
}

// with a playhead, unreached frames go first, then the one reached last.
#[trial]
fn spares_the_frames_the_preview_reaches_next() {
    let mut cache = FrameCache::new(50);
    cache.set_playhead(Some(playhead(5, 5, 20, false)));
    fill(&mut cache, [0, 1, 5, 6, 7, 8]);
    assert!(!cache.contains(&TestKey::new(0, 0)) && !cache.contains(&TestKey::new(0, 1)));
    assert!((5..=8).all(|f| cache.contains(&TestKey::new(0, f))));
    fill(&mut cache, [9, 10]);
    assert_eq!(cache.cached_frames(&TestKey::new(0, 0)), [5, 6, 7, 8]);
}

// nearest finds the exact frame, else the nearest earlier one within max_back.
