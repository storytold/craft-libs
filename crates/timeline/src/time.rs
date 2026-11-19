//! Compatibility re-exports for timeline time primitives.
//!
//! The implementation lives in `craft-time`; this module keeps the established
//! `craft_timeline::time::*` paths available to downstream users.

pub use craft_time::{
    FrameRate, ParseError, SAMPLE_RATES, TICKS_PER_SECOND, Tick, TimeDisplay, TimeRange, fields_to_frames, format_timecode_frames, frames_to_fields,
    parse_time_display_relative as parse_time_display, parse_timecode_relative as parse_timecode,
};

/// Format a timeline frame count as film feet + frames, retaining its established compact display.
pub fn format_feet_frames(frame: i64, frames_per_foot: i64) -> String {
    let per_foot = frames_per_foot.max(1).unsigned_abs();
    let frames = frame.unsigned_abs();
    format!("{}{}+{:02}", if frame < 0 { "-" } else { "" }, frames / per_foot, frames % per_foot)
}

/// Format time using the timeline's historical compact Feet + Frames display.
pub fn format_time(tick: Tick, rate: FrameRate, drop_frame: bool, display: TimeDisplay, sample_rate: i64) -> String {
    if let TimeDisplay::Feet35 | TimeDisplay::Feet16 = display {
        return format_feet_frames(rate.frame_at(tick), display.frames_per_foot().unwrap_or(16));
    }
    craft_time::format_time(tick, rate, drop_frame, display, sample_rate)
}

#[cfg(test)]
mod tests;
