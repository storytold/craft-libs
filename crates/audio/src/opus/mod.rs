//! Clean-room pure-Rust Opus (RFC 6716, updated by RFC 8251; RFC 7845 `OpusHead`).
//!
//! Decoding (feature `opus-decode`): [`Decoder`] (multistream, surround), [`StreamDecoder`] (one
//! elementary stream), [`OpusHead`] and the packet parser [`packet`]. From FilmCraft's
//! `filmcraft-opus` (storytold/filmcraft@8fda2f2).
//!
//! Public items keep their names, so FilmCraft can re-export this module.

// Index loops and long argument lists follow the specification's pseudo-code (FilmCraft allows
// these lints workspace-wide).
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

mod decode;

pub use decode::{Decoder, Error, Mode, OpusHead, Packet, Result, SAMPLE_RATES, StreamDecoder, Toc, packet};

/// Coded audio bandwidth (RFC 6716 §2, Table 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bandwidth {
    /// 4 kHz (8 kHz sampling).
    Narrowband,
    /// 6 kHz (12 kHz sampling).
    Mediumband,
    /// 8 kHz (16 kHz sampling).
    Wideband,
    /// 12 kHz (24 kHz sampling).
    SuperWideband,
    /// 20 kHz (48 kHz sampling).
    Fullband,
}

/// Channel order for Vorbis-ordered surround (mapping family 1, RFC 7845 §5.1.1.2): output channel
/// → decoded channel, giving the WAV/SMPTE order (L R C LFE back… side…) the rest of craft-audio
/// uses. From FilmCraft's `filmcraft-codecs`.
pub fn wav_order(channels: usize) -> Option<&'static [usize]> {
    Some(match channels {
        3 => &[0, 2, 1],
        5 => &[0, 2, 1, 3, 4],
        6 => &[0, 2, 1, 5, 3, 4],
        7 => &[0, 2, 1, 6, 5, 3, 4],
        8 => &[0, 2, 1, 7, 5, 6, 3, 4],
        _ => return None,
    })
}
