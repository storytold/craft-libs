//! The H.264 encoder against ffmpeg / ffprobe as external oracles, pinned decisions and rate control.

// FilmCraft allowed these workspace-wide, where these tests come from.
#![allow(clippy::type_complexity, clippy::needless_range_loop)]

mod common;
mod crf_level_and_note;
mod ffmpeg_oracle;
mod high10_oracle;
mod pinned_decisions;
mod thread_count;
mod vbr_overshoot;
