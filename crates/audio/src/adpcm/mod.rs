//! ADPCM: a decoder and an encoder for four families, each on whole blocks or streams of `i16`
//! samples interleaved by channel (feature `adpcm`).
//!
//! - [`ima`]: IMA / DVI ADPCM in WAV blocks (format tag 0x0011): 4 bits per sample, a 4-byte header
//!   per channel and groups of eight samples per channel.
//! - [`ms`]: Microsoft ADPCM in WAV blocks (format tag 0x0002): 4 bits per sample, adaptive
//!   two-tap prediction with the seven standard coefficient pairs.
//! - [`yamaha`]: Yamaha ADPCM (the YM2608 / AICA family): a raw stream of 4-bit codes.
//! - [`swf`]: the ADPCM of Flash (SWF) audio: 2 to 5 bits per sample, in blocks of 4096 samples.
//!
//! Written from the public descriptions of the algorithms: the IMA Digital Audio Focus and
//! Technical Working Groups' *Recommended Practices for Enhancing Digital Audio Compatibility*
//! (IMA ADPCM), Microsoft's *Multimedia Standards Update* (the WAVE format tags and the block
//! layout of Microsoft ADPCM), the SWF file format specification (the ADPCM section) and the
//! Yamaha ADPCM-B description. ffmpeg is only run as a black-box oracle in the tests.
//!
//! Decoders return errors, never panic, on damaged blocks. Encoders are greedy: they choose each
//! code to bring the decoder's output closest to the sample.

pub mod ima;
pub mod ms;
pub mod swf;
pub mod yamaha;

/// What went wrong decoding or encoding ADPCM.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The data is damaged, truncated or does not fit the stated block layout.
    #[error("malformed ADPCM: {0}")]
    Malformed(&'static str),
    /// The settings or samples given to an encoder cannot be written.
    #[error("ADPCM cannot encode: {0}")]
    InvalidInput(&'static str),
}

/// Result with this module's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// IMA ADPCM step sizes (the 89-entry table of the IMA recommended practices).
pub(crate) const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190,
    209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499,
    2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350,
    22385, 24623, 27086, 29794, 32767,
];

/// Clamp to the 16-bit sample range.
pub(crate) fn clip16(v: i32) -> i32 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX))
}

/// The step size at `index` (clamped to the table).
pub(crate) fn step_at(index: i32) -> i32 {
    STEP_TABLE.get(index.clamp(0, 88) as usize).copied().unwrap_or(7)
}
