//! MIDI file formats.
//!
//! - [`Note`], [`CtrlEvent`] and [`Sequence`]: the neutral model, the notes and channel events of
//!   one track, in ticks at [`PPQ`] (960) ticks per quarter note.
//! - [`read_smf`] / [`write_smf`]: Standard MIDI File reading (format 0/1, format 2 best-effort)
//!   and writing (format 0 or 1, 960 PPQ). [`read_smf_lenient`] reads what makes sense of a
//!   damaged file and says what it could not. The reader treats its input as hostile: every length is
//!   checked, variable-length quantities are bounded and track and event counts are capped.
//!
//! Written from the public Standard MIDI Files 1.0 specification.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod sequence;
mod smf;

pub use sequence::{CtrlEvent, CtrlKind, Note, Sequence};
pub use smf::{LenientRead, MAX_EVENTS, MAX_TRACKS, MAX_WARNINGS, Smf, SmfTrack, read_smf, read_smf_lenient, write_smf};

/// Ticks per quarter note used throughout the model.
pub const PPQ: u32 = 960;

/// Errors from reading or writing MIDI data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MidiError {
    #[error("not a Standard MIDI File (missing MThd header)")]
    NotSmf,
    #[error("unexpected end of data at byte {0}")]
    Truncated(usize),
    #[error("variable-length quantity longer than 4 bytes at byte {0}")]
    VlqTooLong(usize),
    #[error("chunk length {len} at byte {offset} exceeds the remaining data")]
    BadChunkLength { offset: usize, len: u64 },
    #[error("invalid MIDI data at byte {offset}: {reason}")]
    Invalid { offset: usize, reason: &'static str },
    #[error("unsupported MIDI file: {0}")]
    Unsupported(&'static str),
    #[error("too many events (limit {0})")]
    TooManyEvents(usize),
    #[error("too many tracks (limit {0})")]
    TooManyTracks(usize),
    #[error("cannot write MIDI file: {0}")]
    Write(String),
}
