//! The coded formats of [`FileEncoder`](crate::FileEncoder): AAC-LC and ALAC in M4A (through
//! craft-media's MP4 muxer) and MP3, AC-3 and E-AC-3 as raw streams. [`Writer`] maps the caller's
//! WAV-order channels onto the encoder's order, drives the packet encoder, and for M4A records the
//! encoder's priming and the exact length in the edit list.

// Each codec is its own feature, and so are the two sinks: with only some of them built, the
// other halves of the shared code are unused.
#![cfg_attr(
    not(all(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode")),
    allow(dead_code, unused_imports, unused_variables, unused_mut)
)]

use super::failed_write_error;
use super::pcm::validate_layout;
use crate::{AudioBuffer, AudioError, ChannelLayout, EncodeFormat, EncodeOptions, RateControl, Result, Speaker};
use std::io::{Cursor, Seek, SeekFrom, Write};

/// Frames handed to the encoder at a time, so its buffers and the packets waiting to be written
/// stay small however long a block is. Also the frames of an ALAC packet.
const CHUNK: usize = 4096;

/// Bits per second per channel when no rate is asked for: AAC.
const AAC_DEFAULT_BPS_PER_CHANNEL: u32 = 64_000;
/// Kilobits per second per channel when no rate is asked for: MP3, and AC-3 and E-AC-3 (per
/// full-bandwidth channel).
const DEFAULT_KBPS_PER_CHANNEL: u32 = 96;

/// The surround speaker a layout may name in place of `speaker`: formats with one surround pair
/// do not tell side from back, so a pair of either kind feeds it.
fn twin(speaker: Speaker) -> Option<Speaker> {
    Some(match speaker {
        Speaker::SideLeft => Speaker::BackLeft,
        Speaker::SideRight => Speaker::BackRight,
        Speaker::BackLeft => Speaker::SideLeft,
        Speaker::BackRight => Speaker::SideRight,
        _ => return None,
    })
}

/// The speakers of `mask` as labels: `"L R C LFE Ls Rs"`.
fn labels(mask: u32) -> String {
    ChannelLayout::from_mask(mask).speakers().iter().map(|s| s.label()).collect::<Vec<_>>().join(" ")
}

/// For each channel the encoder takes, the channel of the caller's audio that feeds it. `mask` is
/// the layout the format has for the channel count, and `order` the encoder's channel order over
/// it (for each encoder channel, the WAV-order channel of `mask`; `None` when the encoder takes
/// WAV order). `layout` names the speakers of the caller's channels. `None` when `layout` is not
/// the format's layout: it is unordered, or has a speaker the format has not.
fn channel_map(mask: u32, order: Option<&[usize]>, layout: &ChannelLayout) -> Option<Vec<usize>> {
    let speakers = ChannelLayout::from_mask(mask).speakers();
    if speakers.len() != usize::from(layout.channels()) {
        return None;
    }
    let mut map = Vec::with_capacity(speakers.len());
    for j in 0..speakers.len() {
        let speaker = *speakers.get(order.and_then(|o| o.get(j).copied()).unwrap_or(j))?;
        let at = layout.index_of(speaker).or_else(|| twin(speaker).and_then(|t| layout.index_of(t)))?;
        if map.contains(&at) {
            return None;
        }
        map.push(at);
    }
    Some(map)
}

/// [`channel_map`] for a format with one layout per channel count, or why the audio does not fit.
fn fixed_map(name: &str, mask: u32, order: Option<&[usize]>, layout: &ChannelLayout) -> Result<Vec<usize>> {
    if mask == 0 {
        return Err(AudioError::Unsupported(format!("{name} cannot hold {} channels", layout.channels())));
    }
    channel_map(mask, order, layout).ok_or_else(|| {
        let given = match layout.mask() {
            Some(m) => labels(m),
            None => "no speaker assignment".into(),
        };
        AudioError::Unsupported(format!("{name} holds {} channels as {}, not {given}; set `EncodeOptions::layout`", layout.channels(), labels(mask)))
    })
}

/// The WAV mask of the speakers an AAC stream of `channels` channels has, as this crate's encoder
/// writes it and its decoder reports it: channel configurations 1–6, the 6.1 program config
/// element for 7, and configuration 7 (7.1 with a front pair left and right of centre) for 8.
#[cfg(feature = "aac-encode")]
fn aac_mask(channels: usize) -> u32 {
    match channels {
        1 => 0x4,
        2 => 0x3,
        3 => 0x7,
        4 => 0x107,
        5 => 0x607,
        6 => 0x60F,
        7 => 0x70F,
        8 => 0xFF,
        _ => 0,
    }
}

/// The encoder's channel order over [`aac_mask`]: [`crate::aac::aac_order`], and for 8 channels
/// configuration 7's C, Lc Rc, L R, Ls Rs, LFE over the WAV order L R C LFE Ls Rs Lc Rc.
#[cfg(feature = "aac-encode")]
fn aac_order(channels: usize) -> Option<&'static [usize]> {
    if channels == 8 { Some(&[2, 6, 7, 0, 1, 4, 5, 3]) } else { crate::aac::aac_order(channels) }
}

/// The AAC rate control for `rate`: a bit rate is kept within what the stream can carry, a
/// quality of 0–1 spans the encoder's VBR range.
#[cfg(feature = "aac-encode")]
fn aac_mode(rate: Option<RateControl>, sample_rate: u32, channels: usize) -> crate::aac::BitrateMode {
    use crate::aac::BitrateMode;
    let (lo, hi) = crate::aac::cbr_range(sample_rate, channels);
    // An empty range (a rate or channel count the encoder refuses anyway) leaves the rate alone.
    let fit = |bps: u32| if lo <= hi { bps.clamp(lo, hi) } else { bps };
    match rate {
        None => BitrateMode::Cbr(fit(AAC_DEFAULT_BPS_PER_CHANNEL.saturating_mul(channels as u32))),
        Some(RateControl::Bitrate(bps)) => BitrateMode::Cbr(fit(bps)),
        // NaN stays NaN, which the encoder refuses.
        Some(RateControl::Quality(q)) => BitrateMode::Vbr(1.0 + 4.0 * q.clamp(0.0, 1.0)),
    }
}

/// The bit rate in kbit/s for a codec with constant bit rates only.
fn constant_kbps(name: &str, rate: Option<RateControl>, default: u32) -> Result<u32> {
    match rate {
        None => Ok(default),
        Some(RateControl::Bitrate(bps)) => Ok(bps / 1000),
        Some(RateControl::Quality(_)) => {
            Err(AudioError::Unsupported(format!("{name} is encoded at a constant bit rate; set a bit rate, not a quality")))
        }
    }
}

/// The AC-3 or E-AC-3 settings for audio with `layout`, and the channel map onto them: the coding
/// mode whose speakers the layout names, at the nearest bit rate the stream can carry.
#[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
fn ac3_config(
    codec: crate::ac3::encoder::Codec,
    sample_rate: u32,
    layout: &ChannelLayout,
    rate: Option<RateControl>,
) -> Result<(crate::ac3::encoder::Config, Vec<usize>)> {
    use crate::ac3::encoder::{Codec, CodingMode, Config};
    use CodingMode::*;
    let name = if codec == Codec::Ac3 { "AC-3" } else { "E-AC-3" };
    let modes = [Mono, Stereo, ThreeFront, StereoSurround, ThreeFrontSurround, StereoTwoSurround, ThreeFrontTwoSurround, Surround71];
    let found = modes.iter().flat_map(|&mode| [false, true].map(|lfe| (mode, lfe))).find_map(|(mode, lfe)| {
        // 7.1 is an E-AC-3 programme with an LFE; the encoder has no other.
        if mode == Surround71 && (codec != Codec::Eac3 || !lfe) {
            return None;
        }
        let config = Config { codec, sample_rate, bitrate_kbps: 0, mode, lfe };
        let map = channel_map(config.channel_layout().mask()?, None, layout)?;
        Some((config, map))
    });
    let Some((mut config, map)) = found else {
        let given = match layout.mask() {
            Some(m) => labels(m),
            None => "no speaker assignment".into(),
        };
        return Err(AudioError::Unsupported(format!("{name} has no layout of {} channels as {given}", layout.channels())));
    };
    let full = config.mode.channels() as u32;
    let want = constant_kbps(name, rate, DEFAULT_KBPS_PER_CHANNEL * full)?;
    // What `Config::validate` asks of the rate: enough for the channels, and for AC-3 an entry of
    // the frame size table, for E-AC-3 a syncframe its 11-bit word count can hold.
    let least = if config.mode == Surround71 { 384 } else { 32 * full };
    config.bitrate_kbps = match codec {
        Codec::Ac3 => {
            let floor = crate::ac3::BITRATES.iter().copied().find(|&r| r >= least).unwrap_or(least);
            crate::ac3::BITRATES.iter().rev().copied().find(|&r| r <= want).filter(|&r| r >= floor).unwrap_or(floor)
        }
        Codec::Eac3 => {
            let most = if config.mode == Surround71 { 1024 } else { (2048 * u64::from(sample_rate) / 96_000).min(1024) as u32 };
            want.clamp(least, most.max(least))
        }
    };
    Ok((config, map))
}

/// One encoded packet and the frames it decodes to.
struct Packet {
    data: Vec<u8>,
    frames: u32,
}

/// A packet encoder of any of the codecs, taking planar `f32` in its own channel order.
enum Codec {
    #[cfg(feature = "aac-encode")]
    Aac(Box<crate::aac::Encoder>),
    #[cfg(feature = "alac-encode")]
    Alac {
        enc: Box<crate::alac::Encoder>,
        quantizers: Vec<super::pcm::Quantizer>,
        /// Quantized samples of the packet being filled, per channel.
        pending: Vec<Vec<i32>>,
    },
    #[cfg(feature = "mp3-encode")]
    Mp3(Box<crate::mp3::Encoder>),
    #[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
    Ac3 {
        enc: Box<crate::ac3::encoder::Encoder>,
        /// The block with every sample finite and within ±1, as the encoder requires.
        planes: Vec<Vec<f32>>,
    },
}

impl Codec {
    /// Encodes `planes` (one slice per encoder channel, equally long, at most [`CHUNK`] frames)
    /// and appends the packets completed.
    fn push(&mut self, planes: &[&[f32]], out: &mut Vec<Packet>) -> Result<()> {
        match self {
            #[cfg(feature = "aac-encode")]
            Codec::Aac(enc) => out.extend(enc.encode(planes).into_iter().map(|data| Packet { data, frames: crate::aac::FRAME_LEN as u32 })),
            #[cfg(feature = "alac-encode")]
            Codec::Alac { enc, quantizers, pending } => {
                let frames = planes.first().map_or(0, |p| p.len());
                let mut at = 0;
                while at < frames {
                    let filled = pending.first().map_or(0, Vec::len);
                    let end = frames.min(at + CHUNK.saturating_sub(filled).max(1));
                    for ((dst, q), src) in pending.iter_mut().zip(quantizers.iter_mut()).zip(planes) {
                        dst.extend(src.get(at..end).unwrap_or_default().iter().map(|&s| q.quantize(s)));
                    }
                    at = end;
                    if pending.first().is_some_and(|p| p.len() >= CHUNK) {
                        alac_packet(enc, pending, out)?;
                    }
                }
            }
            #[cfg(feature = "mp3-encode")]
            Codec::Mp3(enc) => {
                let data = enc.encode(planes);
                if !data.is_empty() {
                    out.push(Packet { data, frames: 0 });
                }
            }
            #[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
            Codec::Ac3 { enc, planes: clean } => {
                for (dst, src) in clean.iter_mut().zip(planes) {
                    dst.clear();
                    dst.extend(src.iter().map(|&s| if s.is_nan() { 0.0 } else { s.clamp(-1.0, 1.0) }));
                }
                let refs: Vec<&[f32]> = clean.iter().map(Vec::as_slice).collect();
                out.extend(enc.push(&refs)?.into_iter().map(|p| Packet { data: p.data, frames: p.samples }));
            }
        }
        Ok(())
    }

    /// Encodes what the encoder still holds and appends the last packets.
    fn finish(&mut self, out: &mut Vec<Packet>) -> Result<()> {
        match self {
            #[cfg(feature = "aac-encode")]
            Codec::Aac(enc) => out.extend(enc.flush().into_iter().map(|data| Packet { data, frames: crate::aac::FRAME_LEN as u32 })),
            #[cfg(feature = "alac-encode")]
            Codec::Alac { enc, pending, .. } => {
                if pending.first().is_some_and(|p| !p.is_empty()) {
                    alac_packet(enc, pending, out)?;
                }
            }
            #[cfg(feature = "mp3-encode")]
            Codec::Mp3(enc) => {
                let data = enc.flush();
                if !data.is_empty() {
                    out.push(Packet { data, frames: 0 });
                }
            }
            #[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
            Codec::Ac3 { enc, .. } => out.extend(enc.finish()?.packets.into_iter().map(|p| Packet { data: p.data, frames: p.samples })),
        }
        Ok(())
    }

    /// Decoded frames before the first frame of the audio.
    fn priming(&self) -> u32 {
        match self {
            #[cfg(feature = "aac-encode")]
            Codec::Aac(enc) => enc.priming_samples(),
            #[allow(unreachable_patterns)]
            _ => 0,
        }
    }

    /// The MP4 sample entry of the stream as encoded so far (`bytes` in packets of `frames`
    /// frames in all): the configuration a decoder needs, with the average bit rate.
    #[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
    fn sample_entry(&self, sample_rate: u32, channels: u32, bytes: u64, frames: u64) -> Option<craft_media::isobmff::SampleEntry> {
        use craft_media::isobmff::{CodecConfig, FourCc, SampleEntry};
        match self {
            #[cfg(feature = "aac-encode")]
            Codec::Aac(enc) => {
                let mut entry = SampleEntry::aac(enc.audio_specific_config(), channels, sample_rate);
                if let CodecConfig::Aac(config) = &mut entry.codec
                    && frames > 0
                {
                    let bps = u128::from(bytes) * 8 * u128::from(sample_rate) / u128::from(frames);
                    config.avg_bitrate = u32::try_from(bps).unwrap_or(u32::MAX);
                }
                Some(entry)
            }
            #[cfg(feature = "alac-encode")]
            Codec::Alac { enc, .. } => {
                let bits = u16::from(enc.encoder_config().bit_depth);
                Some(SampleEntry::audio(FourCc(*b"alac"), CodecConfig::Alac { cookie: enc.cookie() }, channels, f64::from(sample_rate), bits))
            }
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }
}

/// Encodes the pending samples as one ALAC packet and empties them.
#[cfg(feature = "alac-encode")]
fn alac_packet(enc: &mut crate::alac::Encoder, pending: &mut [Vec<i32>], out: &mut Vec<Packet>) -> Result<()> {
    let refs: Vec<&[i32]> = pending.iter().map(Vec::as_slice).collect();
    let frames = refs.first().map_or(0, |p| p.len()) as u32;
    let data = enc.encode(&refs)?;
    pending.iter_mut().for_each(Vec::clear);
    out.push(Packet { data, frames });
    Ok(())
}

/// A muxer's error as this crate's: a failed write stays an I/O error.
#[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
fn mux_error(e: craft_media::isobmff::Error) -> AudioError {
    match e {
        craft_media::isobmff::Error::Io(e) => AudioError::Io(e),
        e => AudioError::Encode(format!("M4A: {e}")),
    }
}

/// A sink seen from where the file starts: position 0 is the sink's position when the file was
/// begun. The MP4 muxer writes chunk offsets as stream positions, and they count from the first
/// byte of the file, which need not be the first byte of the sink.
#[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
struct FromStart<W> {
    out: W,
    start: u64,
}

#[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
impl<W: Write> Write for FromStart<W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.out.write(data)
    }

    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.out.write_all(data)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

#[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
impl<W: Seek> Seek for FromStart<W> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let to = match to {
            SeekFrom::Start(at) => SeekFrom::Start(
                self.start.checked_add(at).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek past the end of the sink"))?,
            ),
            relative => relative,
        };
        let at = self.out.seek(to)?;
        at.checked_sub(self.start).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek to before the start of the file"))
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
    mux: craft_media::isobmff::Mp4Writer<FromStart<W>>,
    /// The track, added with the first packet: a file with no audio gets a track without an edit.
    track: Option<usize>,
    entry: craft_media::isobmff::SampleEntry,
    sample_rate: u32,
    channels: u32,
    priming: u32,
    held: Option<Packet>,
    /// Frames and bytes of the packets written, the held one not counted.
    frames: u64,
    bytes: u64,
}

#[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
impl<W: Write + Seek> M4a<W> {
    fn new(mut out: W, codec: &Codec, sample_rate: u32, channels: u32) -> Result<Self> {
        use craft_media::isobmff::{Brand, Mp4Writer, WriterOptions};
        let start = out.stream_position()?;
        let out = FromStart { out, start };
        let entry = codec.sample_entry(sample_rate, channels, 0, 0).ok_or_else(|| AudioError::Unsupported("M4A holds AAC and ALAC".into()))?;
        let mut options = WriterOptions::new(Brand::Mp4);
        options.movie_timescale = sample_rate;
        let mux = Mp4Writer::new(out, options).map_err(mux_error)?;
        Ok(M4a { mux, track: None, entry, sample_rate, channels, priming: codec.priming(), held: None, frames: 0, bytes: 0 })
    }

    fn track(&mut self, has_audio: bool) -> Result<usize> {
        if let Some(track) = self.track {
            return Ok(track);
        }
        let mut config = craft_media::isobmff::TrackConfig::new(self.entry.clone(), self.sample_rate);
        if has_audio && self.priming > 0 {
            config.media_start = Some(i64::from(self.priming));
        }
        let track = self.mux.add_track(config).map_err(mux_error)?;
        self.track = Some(track);
        Ok(track)
    }

    fn sample(&mut self, packet: &Packet, duration: u32) -> Result<()> {
        let track = self.track(true)?;
        let sample = craft_media::isobmff::WriteSample { data: &packet.data, duration, composition_offset: 0, is_sync: true };
        self.mux.write_sample(track, sample).map_err(mux_error)?;
        self.frames += u64::from(duration);
        self.bytes += packet.data.len() as u64;
        Ok(())
    }

    fn write(&mut self, packet: Packet) -> Result<()> {
        if let Some(held) = self.held.replace(packet) {
            self.sample(&held, held.frames)?;
        }
        Ok(())
    }

    /// Writes the last packet, cut to the end of `audio_frames` frames of audio, and the `moov`.
    fn finish(mut self, codec: &Codec, audio_frames: u64) -> Result<W> {
        if let Some(held) = self.held.take() {
            let rest = (u64::from(self.priming) + audio_frames).saturating_sub(self.frames);
            self.sample(&held, rest.min(u64::from(held.frames)) as u32)?;
        }
        let track = self.track(false)?;
        // The configuration is complete only now: the bit rate, and ALAC's largest packet.
        if let Some(entry) = codec.sample_entry(self.sample_rate, self.channels, self.bytes, self.frames) {
            self.mux.set_sample_entry(track, entry).map_err(mux_error)?;
        }
        Ok(self.mux.finish().map_err(mux_error)?.out)
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
    codec: Codec,
    sink: Sink<W>,
    /// For each encoder channel, the caller's channel that feeds it.
    map: Vec<usize>,
    frames: u64,
    packets: Vec<Packet>,
    /// Set when a write has failed: the encoder and the sink no longer agree, so the writer
    /// refuses every later call.
    failed: bool,
}

impl<W: Write + Seek> Writer<W> {
    pub(crate) fn new(out: W, opts: &EncodeOptions, sample_rate: u32, channels: usize) -> Result<Self> {
        validate_layout(channels, sample_rate)?;
        let layout = opts.layout.unwrap_or_else(|| ChannelLayout::default_for(channels as u16));
        let (codec, map) = match opts.format {
            #[cfg(feature = "aac-encode")]
            EncodeFormat::Aac => {
                let map = fixed_map("AAC", aac_mask(channels), aac_order(channels), &layout)?;
                let config = crate::aac::EncoderConfig {
                    mode: aac_mode(opts.rate_control, sample_rate, channels),
                    ..crate::aac::EncoderConfig::cbr(sample_rate, channels, 0)
                };
                (Codec::Aac(Box::new(crate::aac::Encoder::new(config)?)), map)
            }
            #[cfg(feature = "alac-encode")]
            EncodeFormat::Alac => {
                let count = u8::try_from(channels).unwrap_or(u8::MAX);
                let map = fixed_map("ALAC", crate::alac::channel_mask(count), crate::alac::alac_order(count), &layout)?;
                let bits = match opts.bit_depth {
                    crate::BitDepth::Int16 => 16,
                    crate::BitDepth::Int24 => 24,
                    crate::BitDepth::Int32 => 32,
                    crate::BitDepth::Float32 => return Err(AudioError::Unsupported("ALAC cannot store 32-bit float samples".into())),
                };
                let enc = crate::alac::Encoder::new(crate::alac::EncoderConfig::new(sample_rate, count, bits).with_frame_length(CHUNK as u32))?;
                let quantizers = super::pcm::Quantizer::per_channel(u32::from(bits), opts.dither, channels);
                (Codec::Alac { enc: Box::new(enc), quantizers, pending: vec![Vec::with_capacity(CHUNK); channels] }, map)
            }
            #[cfg(feature = "mp3-encode")]
            EncodeFormat::Mp3 => {
                let mask = match channels {
                    1 => 0x4,
                    2 => 0x3,
                    _ => 0,
                };
                let map = fixed_map("MP3", mask, None, &layout)?;
                let want = constant_kbps("MP3", opts.rate_control, DEFAULT_KBPS_PER_CHANNEL * channels as u32)?;
                // A sample rate MP3 has not is the encoder's to refuse.
                let kbps = crate::mp3::nearest_bitrate(sample_rate, want).unwrap_or(want);
                (Codec::Mp3(Box::new(crate::mp3::Encoder::new(crate::mp3::Config::cbr(sample_rate, channels, kbps))?)), map)
            }
            #[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
            EncodeFormat::Ac3 | EncodeFormat::Eac3 => {
                let codec = if opts.format == EncodeFormat::Ac3 { crate::ac3::encoder::Codec::Ac3 } else { crate::ac3::encoder::Codec::Eac3 };
                let (config, map) = ac3_config(codec, sample_rate, &layout, opts.rate_control)?;
                (Codec::Ac3 { enc: Box::new(crate::ac3::encoder::Encoder::new(config)?), planes: vec![Vec::with_capacity(CHUNK); channels] }, map)
            }
            #[allow(unreachable_patterns)]
            f => return Err(AudioError::Unsupported(format!("writing {f:?}"))),
        };
        let sink = match opts.format {
            #[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
            EncodeFormat::Aac | EncodeFormat::Alac => Sink::M4a(Box::new(M4a::new(out, &codec, sample_rate, channels as u32)?)),
            _ => Sink::Raw(out),
        };
        Ok(Writer { codec, sink, map, frames: 0, packets: Vec::new(), failed: false })
    }

    /// Hands the packets waiting to the sink.
    fn drain(&mut self) -> Result<()> {
        for packet in self.packets.drain(..) {
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
        for at in (0..frames).step_by(CHUNK) {
            let end = frames.min(at + CHUNK);
            let planes: Vec<&[f32]> = self.map.iter().map(|&c| block.get(c).and_then(|p| p.as_ref().get(at..end)).unwrap_or_default()).collect();
            let written = self.codec.push(&planes, &mut self.packets).and_then(|()| self.drain());
            if let Err(e) = written {
                self.failed = true;
                return Err(e);
            }
        }
        self.frames = self.frames.saturating_add(frames as u64);
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<W> {
        if self.failed {
            return Err(failed_write_error());
        }
        self.codec.finish(&mut self.packets)?;
        self.drain()?;
        match self.sink {
            #[cfg(any(feature = "aac-encode", feature = "alac-encode"))]
            Sink::M4a(m4a) => m4a.finish(&self.codec, self.frames),
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

#[cfg(test)]
mod tests;
