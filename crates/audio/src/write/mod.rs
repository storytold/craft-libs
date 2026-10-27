//! Encoding: WAV/BWF/RF64, AIFF/AIFF-C and FLAC writers, whole-buffer and streaming.

mod aiff;
mod flac;
mod pcm;
mod wav;

use crate::{AudioBuffer, AudioError, ChannelLayout, EncodeFormat, EncodeOptions, Result};
use std::io::{Seek, Write};

/// Encode `buf` as WAV/BWF, AIFF/AIFF-C or FLAC. Samples are clamped to [-1, 1].
pub fn encode(buf: &AudioBuffer, opts: &EncodeOptions) -> Result<Vec<u8>> {
    match opts.format {
        EncodeFormat::Wav => wav::encode(buf, opts.bit_depth, opts.dither, opts.bwf.as_ref(), false, None),
        EncodeFormat::Aiff => aiff::encode(buf, opts.bit_depth, opts.dither),
        EncodeFormat::Flac => flac::encode(buf, opts.bit_depth, opts.dither),
    }
}

/// Formats [`encode`] can write, as the extensions that name them.
pub const ENCODE_EXTENSIONS: &str = "wav, aif, aiff, flac";

/// The format [`encode`] writes for a file extension or format name (`wav`, `aiff`, `flac`, case-insensitive), or `None` when it cannot write it.
pub fn encode_format_for(name: &str) -> Option<EncodeFormat> {
    match name.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "wav" | "wave" | "bwf" => Some(EncodeFormat::Wav),
        "aif" | "aiff" => Some(EncodeFormat::Aiff),
        "flac" => Some(EncodeFormat::Flac),
        _ => None,
    }
}

/// Encode a WAV file in the RF64 layout regardless of size (normally used only above 4 GiB).
pub fn encode_wav_rf64(buf: &AudioBuffer, opts: &EncodeOptions) -> Result<Vec<u8>> {
    wav::encode(buf, opts.bit_depth, opts.dither, opts.bwf.as_ref(), true, None)
}

/// The WAV speaker mask for `layout` over `channels` channels: its mask, or 0 (no speaker
/// assignment) for an unordered layout. A layout for another channel count is an error.
fn wav_mask(layout: &ChannelLayout, channels: usize) -> Result<u32> {
    if layout.channels() != channels {
        return Err(AudioError::Encode(format!("a {}-channel layout for {channels} channels", layout.channels())));
    }
    Ok(layout.mask().unwrap_or(0))
}

/// Like [`encode`], with the speaker layout to record, for layouts a channel count alone cannot
/// identify (5.1 with side vs back surrounds, 7.0 vs 6.1, Ambisonics as
/// [`ChannelLayout::unordered`]). WAV stores it as `dwChannelMask`; AIFF and FLAC have fixed channel
/// orders and ignore it. `buf` must hold `layout.channels()` channels.
pub fn encode_with_layout(buf: &AudioBuffer, opts: &EncodeOptions, layout: ChannelLayout) -> Result<Vec<u8>> {
    let mask = wav_mask(&layout, buf.channels.len())?;
    match opts.format {
        EncodeFormat::Wav => wav::encode(buf, opts.bit_depth, opts.dither, opts.bwf.as_ref(), false, Some(mask)),
        _ => encode(buf, opts),
    }
}

/// The error a stream encoder returns for every call after one of its writes has failed: the
/// sink may already hold a prefix of the failed block, so the encoder's counters can no longer
/// describe the bytes on the sink, and later writes or `finish` must not claim success.
fn failed_write_error() -> AudioError {
    AudioError::Encode("the stream encoder already failed on an earlier write; its output is incomplete".into())
}

/// Writes WAV/BWF (RF64 above 4 GiB), AIFF / AIFF-C `fl32` or FLAC block by block, producing
/// the same bytes [`encode_with_layout`] would for the whole buffer.
pub struct FileEncoder<W: Write + Seek>(Writer<W>);

enum Writer<W> {
    Wav(wav::Writer<W>),
    Aiff(aiff::Writer<W>),
    Flac(flac::Writer<W>),
}

impl<W: Write + Seek> FileEncoder<W> {
    /// Starts a file at the current position of `out`. `layout` overrides the speaker layout a
    /// WAV records (otherwise the usual one for the channel count); it must have `channels`
    /// channels.
    pub fn new(out: W, opts: &EncodeOptions, channels: usize, sample_rate: u32, layout: Option<ChannelLayout>) -> Result<Self> {
        let (depth, dither) = (opts.bit_depth, opts.dither);
        let mask = layout.map(|l| wav_mask(&l, channels)).transpose()?;
        Ok(FileEncoder(match opts.format {
            EncodeFormat::Wav => Writer::Wav(wav::Writer::new(
                out,
                channels,
                sample_rate,
                depth,
                dither,
                wav::HeaderExtras { bwf: opts.bwf.as_ref(), mask, force_rf64: false },
            )?),
            EncodeFormat::Aiff => Writer::Aiff(aiff::Writer::new(out, channels, sample_rate, depth, dither)?),
            EncodeFormat::Flac => Writer::Flac(flac::Writer::new(out, channels, sample_rate, depth, dither)?),
        }))
    }

    /// Fails, without writing anything, when `frames` more frames would not fit the format (an
    /// AIFF holds 4 GiB, a FLAC 2^36 frames).
    pub fn check_room(&self, frames: u64) -> Result<()> {
        match &self.0 {
            Writer::Wav(w) => w.check_room(frames),
            Writer::Aiff(w) => w.check_room(frames),
            Writer::Flac(w) => w.check_room(frames),
        }
        .map(drop)
    }

    /// Appends the first `frames` frames of `block`, one `Vec` per channel (missing samples are
    /// silence), unless they don't fit (see [`FileEncoder::check_room`]). Once a write has
    /// failed, the encoder is unusable: the sink may hold a prefix of the failed block, so
    /// every later `write` and `finish` fails instead of completing an inconsistent file.
    pub fn write(&mut self, block: &[Vec<f32>], frames: usize) -> Result<()> {
        match &mut self.0 {
            Writer::Wav(w) => w.write(block, frames),
            Writer::Aiff(w) => w.write(block, frames),
            Writer::Flac(w) => w.write(block, frames),
        }
    }

    /// Completes the headers and hands back `out`, positioned at the end of the file.
    pub fn finish(self) -> Result<W> {
        match self.0 {
            Writer::Wav(w) => w.finish(),
            Writer::Aiff(w) => w.finish(),
            Writer::Flac(w) => w.finish(),
        }
    }
}

/// The header of a 32-bit float WAV holding `frames` frames (little-endian samples follow it). Its
/// length doesn't depend on `frames`, so a recorder can rewrite it in place as the file grows; it
/// switches to RF64 above 4 GiB.
pub fn wav_float_header(channels: usize, sample_rate: u32, frames: u64) -> Result<Vec<u8>> {
    if channels == 0 || sample_rate == 0 {
        return Err(AudioError::Encode("a WAV needs channels and a sample rate".into()));
    }
    wav::header(channels, sample_rate, (32, true), None, None, frames, false)
}
