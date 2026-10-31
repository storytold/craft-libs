//! FLAC writing (RFC 9639): [`Writer`] quantizes planar `f32` and streams it through [`encoder`]
//! (linear prediction, stereo decorrelation, STREAMINFO MD5) into any `Write + Seek`.

mod crc;
mod encoder;
mod frame;
mod md5;
mod predict;

#[cfg(test)]
use craft_media_testkit as common;
#[cfg(test)]
mod oracle_tests;

use super::pcm::{Quantizer, validate_buffer, validate_layout};
use crate::{AudioBuffer, AudioError, BitDepth, Result};
use encoder::{Encoder, EncoderConfig, STREAMINFO_OFFSET};
use std::io::{Cursor, Seek, SeekFrom, Write};

/// Checks that FLAC can hold the layout; returns the bits per sample.
fn check(channels: usize, sample_rate: u32, depth: BitDepth) -> Result<u8> {
    validate_layout(channels, sample_rate)?;
    if channels > 8 {
        return Err(AudioError::Unsupported(format!("FLAC supports at most 8 channels, got {channels}")));
    }
    // STREAMINFO could say up to 1048575 Hz, but a frame header names at most 655350 Hz, and
    // symphonia (our decoder) refuses streams above that.
    if sample_rate > 655_350 {
        return Err(AudioError::Unsupported(format!("FLAC sample rate {sample_rate} exceeds 655350 Hz")));
    }
    match depth {
        BitDepth::Int16 => Ok(16),
        BitDepth::Int24 => Ok(24),
        BitDepth::Int32 => Ok(32),
        BitDepth::Float32 => Err(AudioError::Unsupported("FLAC cannot store 32-bit float samples".into())),
    }
}

fn encoder_error(e: encoder::Error) -> AudioError {
    AudioError::Encode(format!("FLAC: {e}"))
}

/// A FLAC stream written block by block; [`Writer::finish`] fills in the STREAMINFO sizes, sample
/// count and MD5.
pub(crate) struct Writer<W> {
    out: W,
    start: u64,
    encoder: Encoder,
    quantizers: Vec<Quantizer>,
    planar: Vec<Vec<i32>>,
    frames: u64,
    /// Set when a write has failed: the encoder may hold samples the sink never received, so the
    /// writer refuses every later call.
    failed: bool,
}

impl<W: Write + Seek> Writer<W> {
    pub(crate) fn new(mut out: W, channels: usize, sample_rate: u32, depth: BitDepth, dither: bool) -> Result<Self> {
        let bps = check(channels, sample_rate, depth)?;
        // `check` bounds channels to 1..=8.
        let encoder = Encoder::new(EncoderConfig::new(sample_rate, channels as u8, bps)).map_err(encoder_error)?;
        let start = out.stream_position()?;
        out.write_all(&encoder.header())?;
        Ok(Writer {
            out,
            start,
            encoder,
            quantizers: Quantizer::per_channel(u32::from(bps), dither, channels),
            planar: vec![Vec::new(); channels],
            frames: 0,
            failed: false,
        })
    }

    /// The frame count after `frames` more, or an error past what FLAC can count (2^36 frames).
    pub(crate) fn check_room(&self, frames: u64) -> Result<u64> {
        let total = self.frames.saturating_add(frames);
        if total >= 1u64 << 36 {
            return Err(AudioError::TooLarge("too many frames for FLAC".into()));
        }
        Ok(total)
    }

    /// Appends `frames` frames of planar audio (missing samples are silence), unless they don't
    /// fit (see [`Writer::check_room`]).
    pub(crate) fn write<C: AsRef<[f32]>>(&mut self, block: &[C], frames: usize) -> Result<()> {
        if self.failed {
            return Err(super::failed_write_error());
        }
        let total = self.check_room(frames as u64)?;
        for (c, (p, q)) in self.planar.iter_mut().zip(self.quantizers.iter_mut()).enumerate() {
            let src = block.get(c).map(AsRef::as_ref).unwrap_or_default();
            p.clear();
            p.extend((0..frames).map(|i| q.quantize(src.get(i).copied().unwrap_or(0.0))));
        }
        let refs: Vec<&[i32]> = self.planar.iter().map(Vec::as_slice).collect();
        let written = self.encoder.encode(&refs).map_err(encoder_error).and_then(|bytes| Ok(self.out.write_all(&bytes)?));
        if let Err(e) = written {
            self.failed = true;
            return Err(e);
        }
        self.frames = total;
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<W> {
        if self.failed {
            return Err(super::failed_write_error());
        }
        let last = self.encoder.finish();
        self.out.write_all(&last)?;
        self.out.seek(SeekFrom::Start(self.start.saturating_add(STREAMINFO_OFFSET)))?;
        self.out.write_all(&self.encoder.streaminfo())?;
        self.out.seek(SeekFrom::End(0))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

pub(crate) fn encode(buf: &AudioBuffer, depth: BitDepth, dither: bool) -> Result<Vec<u8>> {
    validate_buffer(buf)?;
    let bps = check(buf.channels.len(), buf.sample_rate, depth)?;
    let frames = buf.frames();
    let mut out = Vec::new();
    out.try_reserve(frames.saturating_mul(buf.channels.len()).saturating_mul(usize::from(bps) / 8) / 2)
        .map_err(|_| AudioError::TooLarge("cannot allocate FLAC output".into()))?;
    let mut w = Writer::new(Cursor::new(out), buf.channels.len(), buf.sample_rate, depth, dither)?;
    w.write(&buf.channels, frames)?;
    Ok(w.finish()?.into_inner())
}
