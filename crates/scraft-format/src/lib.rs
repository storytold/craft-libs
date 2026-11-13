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
    read_session_data(name, bytes.to_vec(), limit)
}

/// Turn already-read `.scraft` bytes into text without making another copy.
pub fn read_session_data(name: &str, bytes: Vec<u8>, limit: u64) -> Result<String, BundleError> {
    check_session_size(name, u64::try_from(bytes.len()).unwrap_or(u64::MAX), limit, false)?;
    String::from_utf8(bytes).map_err(|_| BundleError::Session(format!("{name}: not a SoundCraft session")))
}

/// The size of pretty JSON for `session`, measured without retaining the serialized document.
pub fn session_json_len(session: &Session) -> Result<u64, BundleError> {
    struct Count(u64);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer_pretty(&mut count, session).map_err(|error| BundleError::Session(error.to_string()))?;
    Ok(count.0)
}

/// Serialize a `.scraft` session, refusing output larger than the caller's read limit while
/// keeping the temporary allocation within that limit.
pub fn write_session(session: &Session, limit: u64) -> Result<String, BundleError> {
    write_session_named(session, "the session", limit)
}

/// [`write_session`] with the path or label used for the session-size error.
pub fn write_session_named(session: &Session, name: &str, limit: u64) -> Result<String, BundleError> {
    struct Bounded(Vec<u8>, u64, bool);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let remaining = self.1.saturating_sub(u64::try_from(self.0.len()).unwrap_or(u64::MAX));
            let count = usize::try_from(remaining).unwrap_or(usize::MAX).min(bytes.len());
            self.0.extend_from_slice(&bytes[..count]);
            self.2 |= count < bytes.len();
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let capacity = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut output = Bounded(Vec::with_capacity(capacity.min(1024 * 1024)), limit, false);
    let serialized = serde_json::to_writer_pretty(&mut output, session);
    if output.2 {
        check_session_size(name, limit.saturating_add(1), limit, true)?;
    }
    serialized.map_err(|error| BundleError::Session(error.to_string()))?;
    String::from_utf8(output.0).map_err(|error| BundleError::Session(error.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod session_io_tests {
    use super::*;

    #[test]
    fn bounded_writer_matches_canonical_session_json_and_refuses_one_byte_under() {
        let session = Session::default();
        let canonical = session.to_json().unwrap();
        let len = u64::try_from(canonical.len()).unwrap();
        assert_eq!(session_json_len(&session).unwrap(), len);
        assert_eq!(write_session(&session, len).unwrap(), canonical);
        assert!(write_session(&session, len - 1).is_err());
    }

    #[test]
    fn bare_session_reader_checks_size_and_utf8() {
        assert!(read_session_bytes("song.scraft", b"{}", 1).is_err());
        assert!(read_session_bytes("song.scraft", &[0xff], 10).is_err());
        assert_eq!(read_session_bytes("song.scraft", b"{}", 2).unwrap(), "{}");
    }
}
