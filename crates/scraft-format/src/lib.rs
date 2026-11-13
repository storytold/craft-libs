//! The SoundCraft session document.
//!
//! A [`Session`] is pure data: tracks with playlists of clips, the mixer (inserts, sends, routing,
//! automation), tempo/meter, memory locations and groups. Audio sample data lives in
//! [`SourcePool`] behind `Arc`s, so cloning a session for undo is cheap.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod automation;
pub mod b64;
pub mod bundle;
mod clip;
mod ids;
mod markers;
mod mixer;
mod session;
mod source;
mod track;

pub use craft_midi as soundcraft_midi;
pub use craft_time::soundcraft as soundcraft_time;

pub use automation::{AutoParam, AutomationLane, AutomationMode, AutomationPoint};
pub use bundle::{BUNDLE_EXTENSION, BundleError, BundleLimits, DEFAULT_BUNDLE_LIMITS, check_session_size, read_bundle, write_bundle};
pub use clip::{Clip, ClipContent, Fade, FadeShape};
pub use ids::{BusId, ClipId, GroupId, MarkerId, SourceId, TrackId};
pub use markers::{Group, MarkerKind, MemoryLocation};
pub use mixer::{
    Bus, ChannelFormat, INSERT_SLOTS, Insert, Mixer, OutputPath, Route, SEND_SLOTS, SendSlot, Speaker, SurroundPan, fader_db_to_pos, fader_pos_to_db,
};
pub use session::{
    AutoName, BitDepthSetting, EditMode, EditState, FEET_FRAMES_START, MAX_COMMENT_CHARS, MAX_NAME_CHARS, SESSION_EXTENSION, Session, SessionError,
    Tool, TrackView, ZoomState, ZoomToggle, truncate_chars,
};
pub use source::{FileFormat, Source, SourceAudio, SourcePool, VideoSource};
pub use track::{Playlist, TRACK_COLORS, Track, TrackHeight, TrackKind};

pub use soundcraft_time as time;

/// Largest `.scraft` session opened or written by SoundCraft.
pub const MAX_SESSION_BYTES: u64 = 256 << 20;

/// Read a bare `.scraft` file after enforcing the caller's byte limit.
pub fn read_session_bytes(name: &str, bytes: &[u8], limit: u64) -> Result<String, BundleError> {
    check_session_size(name, u64::try_from(bytes.len()).unwrap_or(u64::MAX), limit, false)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| BundleError::Session(format!("{name}: not a SoundCraft session")))
}

/// Serialize a `.scraft` session and refuse output larger than the caller's read limit.
pub fn write_session(session: &Session, limit: u64) -> Result<String, BundleError> {
    let text = session.to_json().map_err(|error| BundleError::Session(error.to_string()))?;
    check_session_size("the session", u64::try_from(text.len()).unwrap_or(u64::MAX), limit, true)?;
    Ok(text)
}
