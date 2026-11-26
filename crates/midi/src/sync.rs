//! MIDI synchronization wire messages, separate from transport scheduling.
//!
//! Extracted from LoopCraft `crates/midi/src/sync.rs` at
//! `18c68c6e18b13d90db3001dd9f95462cdc4b05a8` (source history credits bflatastic).
//! This module preserves its prefix-parser, clamped song-position and non-drop time-code
//! behavior. It does not parse a byte stream or schedule clocks; hosts retain those policies.
//!
//! Only 24, 25 and 30 non-drop frames per second are represented, not 29.97 drop-frame.

/// MIDI clock: 24 to the quarter note.
pub const CLOCKS_PER_QUARTER: u32 = 24;
pub const CLOCK: u8 = 0xF8;
pub const START: u8 = 0xFA;
pub const CONTINUE: u8 = 0xFB;
pub const STOP: u8 = 0xFC;
pub const SONG_POSITION: u8 = 0xF2;
pub const QUARTER_FRAME: u8 = 0xF1;
/// The furthest Song Position Pointer reaches, in sixteenth notes (fourteen bits).
pub const MAX_POSITION: u16 = 0x3FFF;

/// What a device that leads the time sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMessage {
    Clock,
    Start,
    Continue,
    Stop,
    /// Song Position Pointer: sixteenth notes from the start of the song.
    Position(u16),
}

impl SyncMessage {
    /// Parse a status-first message prefix; trailing bytes are ignored for compatibility.
    /// Returns `None` for an unknown status, truncated song position, or non-seven-bit data.
    /// This is not a stream parser and does not remove interleaved real-time messages.
    pub fn parse(message: &[u8]) -> Option<SyncMessage> {
        Some(match *message {
            [CLOCK, ..] => SyncMessage::Clock,
            [START, ..] => SyncMessage::Start,
            [CONTINUE, ..] => SyncMessage::Continue,
            [STOP, ..] => SyncMessage::Stop,
            [SONG_POSITION, low, high, ..] if low < 0x80 && high < 0x80 => SyncMessage::Position(u16::from(high) << 7 | u16::from(low)),
            _ => return None,
        })
    }
}

/// Song Position Pointer at `sixteenths` from the start, the low seven bits first.
pub fn song_position(sixteenths: u16) -> [u8; 3] {
    let at = sixteenths.min(MAX_POSITION);
    [SONG_POSITION, (at & 0x7F) as u8, (at >> 7) as u8]
}

/// The frame rates time code is sent at (non-drop).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rate {
    Fps24,
    #[default]
    Fps25,
    Fps30,
}

impl Rate {
    /// 24 and 30 frames a second are themselves; anything else is 25.
    pub fn of(fps: u8) -> Rate {
        match fps {
            24 => Rate::Fps24,
            30 => Rate::Fps30,
            _ => Rate::Fps25,
        }
    }

    pub fn fps(self) -> u8 {
        match self {
            Rate::Fps24 => 24,
            Rate::Fps25 => 25,
            Rate::Fps30 => 30,
        }
    }

    /// As the hours byte carries it: 0 for 24, 1 for 25, 3 for 30 non-drop.
    fn code(self) -> u8 {
        match self {
            Rate::Fps24 => 0,
            Rate::Fps25 => 1,
            Rate::Fps30 => 3,
        }
    }
}

/// Hours, minutes, seconds and frames at `secs`; the hours wrap at a day.
fn timecode(secs: f64, rate: Rate) -> [u8; 4] {
    let fps = u64::from(rate.fps());
    // `as` saturates: before the start, and NaN, is the start. The nudge keeps a frame's own
    // start in it when the sum lands a hair short.
    let frames = (secs * fps as f64 + 1e-6) as u64;
    let s = frames / fps;
    [(s / 3600 % 24) as u8, (s / 60 % 60) as u8, (s % 60) as u8, (frames % fps) as u8]
}

/// The full-frame message (universal real-time SysEx, all devices) locating to `secs`.
///
/// Compatibility conversion: negative values and NaN locate to zero; positive overflow
/// saturates the frame count at `u64::MAX`. Hours wrap at 24. No drop-frame conversion.
pub fn full_frame(secs: f64, rate: Rate) -> [u8; 10] {
    let [h, m, s, f] = timecode(secs, rate);
    [0xF0, 0x7F, 0x7F, 0x01, 0x01, rate.code() << 5 | h, m, s, f, 0xF7]
}

/// Quarter frame `piece` (0..8) of `time` (hours, minutes, seconds, frames): eight of them, a
/// quarter of a frame apart, carry the whole time, the frames' low nibble first.
fn quarter_frame(time: [u8; 4], rate: Rate, piece: u8) -> [u8; 2] {
    let [h, m, s, f] = time;
    let piece = piece & 7;
    let nibble = match piece {
        0 => f,
        1 => f >> 4,
        2 => s,
        3 => s >> 4,
        4 => m,
        5 => m >> 4,
        6 => h,
        _ => h >> 4 | rate.code() << 1,
    };
    [QUARTER_FRAME, piece << 4 | nibble & 0x0F]
}

/// All eight quarter-frame wire messages for a single time-code snapshot, in wire order.
///
/// This is serialization only: the host must send each piece at the appropriate time and
/// account for transport direction/latency. Seconds use the same conversion as [`full_frame`].
pub fn quarter_frames(secs: f64, rate: Rate) -> [[u8; 2]; 8] {
    let time = timecode(secs, rate);
    [0, 1, 2, 3, 4, 5, 6, 7].map(|piece| quarter_frame(time, rate, piece))
}
