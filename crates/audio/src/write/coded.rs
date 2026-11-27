//! The coded formats of [`FileEncoder`](crate::FileEncoder): AAC-LC and ALAC in M4A (through
//! craft-media's MP4 muxer) and MP3, AC-3 and E-AC-3 as raw streams. [`Writer`] drives a
//! [`PacketEncoder`], which maps the caller's WAV-order channels onto the codec's order, and for
//! M4A records the encoder's priming and the exact length in the edit list.

// Each codec is its own feature, and so are the two sinks: with only some of them built, the
// other halves of the shared code are unused.
#![cfg_attr(
    not(all(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode")),
    allow(dead_code, unused_imports, unused_variables)
)]

use super::failed_write_error;
use crate::packet::{EncodedPacket, PacketEncoder};
use crate::{AudioBuffer, AudioError, CodecId, EncodeFormat, EncodeOptions, Result};
use std::io::{Cursor, Seek, Write};

/// A muxer's error as this crate's: a failed write stays an I/O error.
#[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
fn mux_error(e: craft_media::isobmff::Error) -> AudioError {
    match e {
        craft_media::isobmff::Error::Io(e) => AudioError::Io(e),
        e => AudioError::Encode(format!("M4A: {e}")),
    }
}

/// The MP4 sample entry of the stream as encoded so far (`bytes` in packets of `frames` frames in
/// all): the configuration a decoder needs, with the average bit rate.
#[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
fn sample_entry(enc: &PacketEncoder, bytes: u64, frames: u64) -> Option<craft_media::isobmff::SampleEntry> {
    use craft_media::isobmff::{CodecConfig, FourCc, SampleEntry};
    let (rate, channels) = (enc.sample_rate(), u32::from(enc.channels()));
    match enc.codec() {
        CodecId::Aac => {
            let mut entry = SampleEntry::aac(enc.extradata(), channels, rate);
            if let CodecConfig::Aac(config) = &mut entry.codec
                && frames > 0
            {
                let bps = u128::from(bytes) * 8 * u128::from(rate) / u128::from(frames);
                config.avg_bitrate = u32::try_from(bps).unwrap_or(u32::MAX);
            }
            Some(entry)
        }
        CodecId::Alac => {
            let bits = u16::from(enc.bit_depth().unwrap_or(16));
            Some(SampleEntry::audio(FourCc(*b"alac"), CodecConfig::Alac { cookie: enc.extradata() }, channels, f64::from(rate), bits))
        }
        _ => None,
    }
}

/// An M4A file being written: one audio track whose samples are the packets.
///
/// The last packet is held back until the length is known: its duration is cut to the audio's
/// last frame, so the sample durations add up to the priming plus the audio, and the edit that
/// skips the priming ends where the audio does. The movie timescale is the sample rate, which
/// keeps that edit exact.
#[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
struct M4a<W: Write + Seek> {
    mux: craft_media::isobmff::Mp4Writer<W>,
    /// The track, added with the first packet: a file with no audio gets a track without an edit.
    track: Option<usize>,
    entry: craft_media::isobmff::SampleEntry,
    sample_rate: u32,
    priming: u32,
    held: Option<EncodedPacket>,
    /// Frames and bytes of the packets written, the held one not counted.
    frames: u64,
    bytes: u64,
}

#[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
impl<W: Write + Seek> M4a<W> {
    fn new(out: W, enc: &PacketEncoder) -> Result<Self> {
        use craft_media::isobmff::{Brand, Mp4Writer, WriterOptions};
        let entry = sample_entry(enc, 0, 0).ok_or_else(|| AudioError::Unsupported("M4A holds AAC and ALAC".into()))?;
        let sample_rate = enc.sample_rate();
        let mut options = WriterOptions::new(Brand::Mp4);
        options.movie_timescale = sample_rate;
        let mux = Mp4Writer::new(out, options).map_err(mux_error)?;
        Ok(M4a { mux, track: None, entry, sample_rate, priming: enc.priming(), held: None, frames: 0, bytes: 0 })
    }

    fn track(&mut self, has_audio: bool) -> Result<usize> {
        if let Some(track) = self.track {
            return Ok(track);
        }
        let mut config = craft_media::isobmff::TrackConfig::new(self.entry.clone(), self.sample_rate);
        if has_audio && self.priming > 0 {
            config.media_start = Some(i64::from(self.priming));
            // AAC's overlap: a packet decodes right only after the one before it. Apple's
            // decoders take the priming from the edit only when the track says so.
            config.roll_distance = Some(-1);
        }
        let track = self.mux.add_track(config).map_err(mux_error)?;
        self.track = Some(track);
        Ok(track)
    }

    fn sample(&mut self, packet: &EncodedPacket, duration: u32) -> Result<()> {
        let track = self.track(true)?;
        let sample = craft_media::isobmff::WriteSample { data: &packet.data, duration, composition_offset: 0, is_sync: true };
        self.mux.write_sample(track, sample).map_err(mux_error)?;
        self.frames += u64::from(duration);
        self.bytes += packet.data.len() as u64;
        Ok(())
    }

    fn write(&mut self, packet: EncodedPacket) -> Result<()> {
        if let Some(held) = self.held.replace(packet) {
            self.sample(&held, held.duration)?;
        }
        Ok(())
    }

    /// Writes the last packet, cut to the end of the audio the encoder took, and the `moov`.
    fn finish(mut self, enc: &PacketEncoder) -> Result<W> {
        if let Some(held) = self.held.take() {
            let rest = (u64::from(self.priming) + enc.frames()).saturating_sub(self.frames);
            self.sample(&held, rest.min(u64::from(held.duration)) as u32)?;
        }
        let track = self.track(false)?;
        // The configuration is complete only now: the bit rate, and ALAC's largest packet.
        if let Some(entry) = sample_entry(enc, self.bytes, self.frames) {
            self.mux.set_sample_entry(track, entry).map_err(mux_error)?;
        }
        self.mux.finish().map_err(mux_error)
    }
}

/// Where the packets go.
enum Sink<W: Write + Seek> {
    #[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
    M4a(Box<M4a<W>>),
    /// The packets in a row are the file.
    Raw(W),
}

/// A file of one of the coded formats written block by block.
pub(crate) struct Writer<W: Write + Seek> {
    enc: PacketEncoder,
    sink: Sink<W>,
    /// Set when a write has failed: the encoder and the sink no longer agree, so the writer
    /// refuses every later call.
    failed: bool,
}

impl<W: Write + Seek> Writer<W> {
    pub(crate) fn new(out: W, opts: &EncodeOptions, sample_rate: u32, channels: usize) -> Result<Self> {
        let channels = u16::try_from(channels).map_err(|_| AudioError::Encode("too many channels".into()))?;
        let enc = PacketEncoder::new(opts.format.codec(), opts, sample_rate, channels)?;
        let sink = match opts.format {
            #[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
            EncodeFormat::Aac | EncodeFormat::Alac => Sink::M4a(Box::new(M4a::new(out, &enc)?)),
            _ => Sink::Raw(out),
        };
        Ok(Writer { enc, sink, failed: false })
    }

    pub(crate) fn bitrate(&self) -> Option<u32> {
        self.enc.bitrate()
    }

    /// Hands `packets` to the sink.
    fn drain(&mut self, packets: Vec<EncodedPacket>) -> Result<()> {
        for packet in packets {
            match &mut self.sink {
                #[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
                Sink::M4a(m4a) => m4a.write(packet)?,
                Sink::Raw(out) => out.write_all(&packet.data)?,
            }
        }
        Ok(())
    }

    /// Appends `frames` frames of `block`, which holds every channel at least that long.
    pub(crate) fn write<C: AsRef<[f32]>>(&mut self, block: &[C], frames: usize) -> Result<()> {
        if self.failed {
            return Err(failed_write_error());
        }
        // A block at a time, so the packets waiting for the sink stay few however long it is.
        for at in (0..frames).step_by(crate::packet::CHUNK) {
            let end = frames.min(at + crate::packet::CHUNK);
            let part: Vec<&[f32]> = block.iter().map(|c| c.as_ref().get(at..end).unwrap_or_default()).collect();
            let written = self.enc.encode(&part, end - at).and_then(|packets| self.drain(packets));
            if let Err(e) = written {
                self.failed = true;
                return Err(e);
            }
        }
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<W> {
        if self.failed {
            return Err(failed_write_error());
        }
        let last = self.enc.finish()?;
        self.drain(last)?;
        match self.sink {
            #[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
            Sink::M4a(m4a) => m4a.finish(&self.enc),
            Sink::Raw(mut out) => {
                out.flush()?;
                Ok(out)
            }
        }
    }
}

pub(crate) fn encode(buf: &AudioBuffer, opts: &EncodeOptions) -> Result<Vec<u8>> {
    let mut w = Writer::new(Cursor::new(Vec::new()), opts, buf.sample_rate, buf.channels.len())?;
    w.write(&buf.channels, buf.frames())?;
    Ok(w.finish()?.into_inner())
}
