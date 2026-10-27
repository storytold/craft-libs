//! Clean-room pure-Rust Opus (RFC 6716, updated by RFC 8251; RFC 7845 `OpusHead`).
//!
//! - Decoding (feature `decode`): [`Decoder`] (multistream, surround), [`StreamDecoder`] (one
//!   elementary stream), [`OpusHead`] and the packet parser [`packet`]. From FilmCraft's
//!   `filmcraft-opus` (storytold/filmcraft@8fda2f2).
//! - Encoding (feature `encode`): [`encoder::OpusEncoder`] (SILK, hybrid and CELT modes, 48 kHz,
//!   20 ms, mono or stereo). From EffectCraft's `effectcraft-opusenc` (storytold/effectcraft@c618429).
//!
//! Public items keep their names, so FilmCraft and EffectCraft can re-export this module. The two
//! halves come from separate crates and do not share code yet (each has its own range coder, MDCT
//! and band tables); the encoder's `Mode` and `Bandwidth` live in [`encoder`] beside the decoder's.

// Index loops and long argument lists follow the specification's pseudo-code (FilmCraft and
// EffectCraft allow these lints workspace-wide).
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

#[cfg(feature = "opus-decode")]
mod decode;
#[cfg(feature = "opus-encode")]
pub mod encoder;

#[cfg(feature = "opus-decode")]
pub use decode::{Bandwidth, Decoder, Error, Mode, OpusHead, Packet, Result, SAMPLE_RATES, StreamDecoder, Toc, packet};

/// Channel order for Vorbis-ordered surround (mapping family 1, RFC 7845 §5.1.1.2): output channel
/// → decoded channel, giving the WAV/SMPTE order (L R C LFE back… side…) the rest of craft-audio
/// uses. From FilmCraft's `filmcraft-codecs`.
#[cfg(feature = "opus-decode")]
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
