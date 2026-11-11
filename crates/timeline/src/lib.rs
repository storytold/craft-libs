//! Timelines: exact media time, frame rates and timecode, the playback clock, a frame cache and a
//! frame server, clip edit rules and markers, and the [`TimelineModel`] an app implements on its
//! sequence; [`interchange`], the neutral timeline document the interchange formats read and write. With the
//! `ui` feature, [`ui`] has the timeline widgets.
//!
//! Time is an integer [`Tick`] count. Frames, samples and timecode are displays of it, so edits,
//! frame steps and audio alignment never drift.

pub mod cache;
pub mod clips;
pub mod clock;
pub mod frames;
pub mod interchange;
pub mod markers;
pub mod model;
pub mod rates;
pub mod time;
pub mod timecode;

#[cfg(feature = "ui")]
pub mod ui;

pub use cache::{FrameCache, FrameCacheKey, FramePlayhead, FrameSize};
pub use clips::{ClipEdge, ClipEdit, EdgeHit, EdgeZones, SnapTargets, TrimKind, edge_at, selection_trim, snap_time};
pub use clock::{ClockEvent, Follow, FollowMode, PlaybackClock, PlaybackState};
pub use frames::{CancelToken, FrameRenderer, FrameServer, FrameStats, PlaybackRequest, playback_plan, prefetch_depth};
pub use markers::{Marker, MarkerColor, MarkerId};
pub use model::{ClipId, ClipInfo, TimelineModel, TrackId, TrackInfo, TrackKind};
pub use rates::{FrameRate, PixelAspect, SampleRate};
pub use time::{TICKS_PER_SECOND, Tick, TimeRange, TimeView};
pub use timecode::{TimeDisplay, TimeFormat, TimecodeError, TimecodeFields, format_time, parse_time};

#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod clips_tests;
#[cfg(test)]
mod clock_tests;
#[cfg(test)]
mod frames_tests;
#[cfg(test)]
mod rates_tests;
#[cfg(test)]
mod time_tests;
#[cfg(test)]
mod timecode_tests;

/// The stories and trials this crate registers.
#[cfg(feature = "screen-testing")]
pub mod testing {
    craft_ui::testing_registered!();
}

#[cfg(all(test, feature = "screen-testing"))]
extern crate self as craft_timeline;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}
