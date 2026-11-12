//! Timelines: exact media time, frame rates and timecode, the playback clock, a frame cache and a
//! frame server, clip edit rules and markers, and the [`TimelineModel`] an app implements on its
//! sequence. With the `ui` feature, [`ui`] has the timeline widgets.
//!
//! Time is an integer [`Tick`] count. Frames, samples and timecode are displays of it, so edits,
//! frame steps and audio alignment never drift.
//!
//! Editorial interchange reads and writes whole edits on the same time:
//!
//! - [`interchange`]: a neutral timeline (bins, media, sequences, tracks, clips, transitions,
//!   markers, effects with keyframes) for the formats that describe a whole edit, with
//!   [`interchange::Catalog`] for the application's effects, and the [`Format`]s, options and
//!   errors every reader and writer shares. [`fx`] has the effects the formats name themselves;
//!   [`build`] builds an [`interchange::Project`] while reading.
//! - [`report`]: what an import or export could not represent exactly.
//! - [`path`]: media references (`file://` URLs, absolute and relative paths), lexically.
//! - [`rate`]: frame rates as documents write them (`timebase` + `ntsc`, floating rates).
//! - [`ale`]: Avid Log Exchange clip logs.
//! - [`edl`]: CMX 3600 edit decision lists.
//! - `fcp7` (feature `xml`): Final Cut Pro 7 XML (`xmeml`).
//! - `fcpxml` (feature `xml`): FCPXML 1.9–1.11.
//! - `otio` (feature `otio`): OpenTimelineIO JSON.
//!
//! The interchange modules do no file I/O: documents are text or bytes, and media references are
//! strings. Each format module gives the document as the format writes it; mapping it onto an
//! application's own timeline is the caller's.

pub mod ale;
pub mod build;
pub mod cache;
pub mod clips;
pub mod clock;
pub mod edl;
#[cfg(feature = "xml")]
pub mod fcp7;
#[cfg(feature = "xml")]
pub mod fcpxml;
pub mod frames;
pub mod fx;
pub mod interchange;
pub mod markers;
pub mod model;
#[cfg(feature = "otio")]
pub mod otio;
pub mod path;
pub mod rate;
pub mod rates;
pub mod report;
pub mod time;
pub mod timecode;
#[cfg(feature = "xml")]
mod xml;

#[cfg(feature = "ui")]
pub mod ui;

pub use build::Imported;
pub use cache::{FrameCache, FrameCacheKey, FramePlayhead, FrameSize};
pub use clips::{ClipEdge, ClipEdit, EdgeHit, EdgeZones, SnapTargets, TrimKind, edge_at, selection_trim, snap_time};
pub use clock::{ClockEvent, Follow, FollowMode, PlaybackClock, PlaybackState};
pub use frames::{CancelToken, FrameRenderer, FrameServer, FrameStats, PlaybackRequest, playback_plan, prefetch_depth};
pub use interchange::{Error, ExportOptions, FcpxmlVersion, Format, ImportOptions, Result, decode_text, detect_text};
pub use markers::{Marker, MarkerColor, MarkerId};
pub use model::{ClipId, ClipInfo, TimelineModel, TrackId, TrackInfo, TrackKind};
pub use rates::{FrameRate, PixelAspect, SampleRate};
pub use report::{Level, Report, ReportEntry};
pub use time::{TICKS_PER_SECOND, Tick, TimeRange, TimeView};
pub use timecode::{TimeDisplay, TimeFormat, TimecodeError, TimecodeFields, format_time, parse_frames, parse_time};

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
