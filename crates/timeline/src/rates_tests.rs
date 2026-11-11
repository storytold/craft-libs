//! Frame rates, sample rates and pixel aspect ratios.

use crate::rates::FrameRate;
use crate::time::{TICKS_PER_SECOND, Tick};
use craft_ui::testing::trial;

#[trial]
fn frame_boundaries_round_trip_at_every_rate() {
    for rate in FrameRate::COMMON.into_iter().chain([FrameRate::new(37_516, 1000)]) {
        for f in [-1000, -1, 0, 1, 2, 29, 1799, 1_000_003] {
            assert_eq!(rate.frame_at(rate.tick_of(f)), f, "{rate:?} frame {f}");
            assert_eq!(rate.frame_at(rate.tick_of(f) - Tick(1)), f - 1, "{rate:?} before frame {f}");
        }
    }
    assert_eq!(FrameRate::FPS_25.frame_duration(), Tick(TICKS_PER_SECOND / 25));
    assert_eq!(FrameRate::new(48, 2), FrameRate::new(24, 1));
}
