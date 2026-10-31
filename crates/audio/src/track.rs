//! [`TrackDecoder`]: one track's packets in, planar `f32` in WAV channel order out, whatever the
//! codec. The file-level [`crate::decode`] and [`crate::Decoder`] use it for every native codec;
//! an app with its own demuxer (MXF, MPEG-TS, a network stream) uses it directly.

#[cfg(feature = "aac-decode")]
use crate::aac;
use crate::pcm::{PcmKind, deinterleave};
use crate::{AudioBuffer, AudioError, ChannelLayout, CodecId, CodecParams, Result, SampleFormat};

/// How many leading packets [`TrackDecoder::new`] looks at.
pub const LOOKAHEAD_PACKETS: usize = 8;

/// The codec-specific decoder behind a [`TrackDecoder`].
enum Inner {
    #[cfg(feature = "aac-decode")]
    Aac {
        dec: Box<aac::Decoder>,
        up: Option<aac::Upsample2x>,
        order: Option<&'static [usize]>,
    },
    Pcm {
        kind: PcmKind,
        channels: usize,
    },
}

/// Decodes the packets of one audio track to planar `f32`, in WAV channel order when
/// [`TrackDecoder::layout`] is ordered.
///
/// It takes the track's [`CodecParams`] (from `isobmff::Track::codec_params`,
/// `matroska::Track::codec_params`, `ogg::Stream::codec_params`, or built by hand for another
/// container) and decodes AAC (LC; HE-AAC as its upsampled LC core) and PCM.
/// Container timing (AAC priming, end trimming) is the caller's: every decoded sample is
/// returned.
///
/// ```no_run
/// # fn demo(params: &craft_audio::CodecParams, packets: &[Vec<u8>]) -> craft_audio::Result<()> {
/// use craft_audio::TrackDecoder;
///
/// let lookahead: Vec<&[u8]> = packets.iter().take(8).map(Vec::as_slice).collect();
/// let mut dec = TrackDecoder::new(params, &lookahead)?;
/// println!("{} at {} Hz, {}", dec.codec().name(), dec.sample_rate(), dec.layout());
/// for p in packets {
///     let block = match dec.decode(p) {
///         Ok(block) => block,
///         Err(_) => dec.conceal(Some(p)), // keep the timing across a damaged packet
///     };
///     // block.channels: one Vec<f32> per channel
/// }
/// # Ok(()) }
/// ```
pub struct TrackDecoder {
    inner: Inner,
    codec: CodecId,
    sample_rate: u32,
    layout: ChannelLayout,
    sample_format: SampleFormat,
    /// Frames of the last decoded packet, the length of a concealed one when the codec does not
    /// say.
    last_frames: usize,
}

impl std::fmt::Debug for TrackDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackDecoder")
            .field("codec", &self.codec)
            .field("sample_rate", &self.sample_rate)
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl TrackDecoder {
    /// A decoder for the track `params` describes. `lookahead` holds the track's first packets
    /// (up to [`LOOKAHEAD_PACKETS`]; empty when they are not at hand): HE-AAC signalled only
    /// inside the stream is found in them.
    ///
    /// Streams without codec configuration are accepted where the packets carry it: AAC with
    /// empty `extradata` takes its configuration from the first packet's ADTS header (MPEG-TS,
    /// raw `.aac`), and ADTS headers are stripped from every packet. Other missing configuration is
    /// [`AudioError::Malformed`].
    ///
    /// Codecs without a native packet decoder yet (MP3, FLAC, ALAC, Vorbis, Opus) are
    /// [`AudioError::Unsupported`]; `craft_audio::decode` decodes them through symphonia when the
    /// `symphonia` feature is on.
    pub fn new(params: &CodecParams, lookahead: &[&[u8]]) -> Result<TrackDecoder> {
        let rate = params.sample_rate.unwrap_or(0);
        let channels = usize::from(params.channels.unwrap_or(0));
        let (inner, codec, sample_rate, layout, sample_format) = match params.codec {
            #[cfg(feature = "aac-decode")]
            CodecId::Aac | CodecId::HeAac => {
                // ADTS-framed packets (MPEG-TS, raw `.aac`) carry their configuration in each
                // header: without extradata, the first header gives it.
                let payloads: Vec<&[u8]> =
                    lookahead.iter().map(|p| adts_payloads(p).and_then(|r| r.ok()).and_then(|v| v.first().copied()).unwrap_or(p)).collect();
                let asc = if params.extradata.is_empty() {
                    let h = lookahead.first().and_then(|p| aac::AdtsHeader::parse(p).ok()).ok_or_else(|| {
                        AudioError::Malformed("AAC track without an AudioSpecificConfig (extradata), and its packets are not ADTS".into())
                    })?;
                    h.to_asc().to_bytes()
                } else {
                    params.extradata.clone()
                };
                let asc = asc.as_slice();
                let dec = aac::Decoder::new(asc)?;
                let out = aac::output_rate(asc, payloads.iter().copied().take(LOOKAHEAD_PACKETS)).unwrap_or(dec.sample_rate());
                let up = (dec.sample_rate().checked_mul(2) == Some(out)).then(|| aac::Upsample2x::new(dec.channels()));
                let codec = if up.is_some() { CodecId::HeAac } else { CodecId::Aac };
                let cfg = dec.config().channel_config;
                let layout = aac_layout(cfg, dec.channels());
                let order = layout.is_ordered().then(|| aac::wav_order(cfg)).flatten();
                (Inner::Aac { dec: Box::new(dec), up, order }, codec, out, layout, SampleFormat::Compressed)
            }
            CodecId::Pcm => {
                let pcm = params.pcm.ok_or_else(|| AudioError::Malformed("PCM track without a sample format".into()))?;
                let (kind, sample_format) = pcm_kind(pcm.bits, pcm.float, pcm.big_endian, !pcm.signed)
                    .ok_or_else(|| AudioError::Unsupported(format!("{}-bit PCM", pcm.bits)))?;
                if channels == 0 || rate == 0 {
                    return Err(AudioError::Malformed("PCM track without channels or a sample rate".into()));
                }
                (Inner::Pcm { kind, channels }, CodecId::Pcm, rate, ChannelLayout::default_for(crate::layout::count(channels)), sample_format)
            }
            other => return Err(unsupported(other)),
        };
        Ok(TrackDecoder { inner, codec, sample_rate, layout, sample_format, last_frames: 0 })
    }

    /// The codec, refined from the stream: [`CodecId::HeAac`] for AAC with spectral band
    /// replication.
    pub fn codec(&self) -> CodecId {
        self.codec
    }

    /// The rate of the decoded audio (for HE-AAC, twice the AAC-LC core rate).
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The speakers of the decoded channels; ordered layouts come in WAV order.
    pub fn layout(&self) -> ChannelLayout {
        self.layout
    }

    /// The number of decoded channels.
    pub fn channels(&self) -> u16 {
        self.layout.channels()
    }

    /// The stored sample format: the PCM depth, or [`SampleFormat::Compressed`].
    pub fn sample_format(&self) -> SampleFormat {
        self.sample_format
    }

    /// Decode one packet. A packet that fails is an error for every codec: call [`TrackDecoder::conceal`] for audio of its length, so what follows keeps
    /// its timing.
    pub fn decode(&mut self, packet: &[u8]) -> Result<AudioBuffer> {
        let channels = match &mut self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { dec, up, order } => {
                let mut out: Vec<Vec<f32>> = Vec::new();
                let units = match adts_payloads(packet) {
                    Some(units) => units?,
                    None => vec![packet],
                };
                for unit in units {
                    let mut block = dec.decode(unit)?;
                    if let Some(up) = up {
                        block = up.process(block);
                    }
                    append(&mut out, block);
                }
                reorder(out, *order)
            }
            Inner::Pcm { kind, channels } => {
                let frames = (packet.len() / (kind.bytes() * *channels).max(1)) as u64;
                deinterleave(packet, *kind, *channels, frames)?
            }
        };
        if let Some(n) = channels.iter().map(Vec::len).min().filter(|&n| n > 0) {
            self.last_frames = n;
            if channels.len() != usize::from(self.layout.channels()) {
                self.layout = ChannelLayout::unordered(crate::layout::count(channels.len()));
            }
        }
        Ok(AudioBuffer { sample_rate: self.sample_rate, channels })
    }

    /// What stands in for a lost or undecodable packet (`packet` when its bytes are at hand):
    /// silence as long as the packet would have been (from its frame
    /// header, the codec's frame length, or the last packet). Empty when the length is unknown.
    pub fn conceal(&mut self, packet: Option<&[u8]>) -> AudioBuffer {
        let data = packet.unwrap_or_default();
        let frames = match &mut self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => Some(aac::FRAME_LEN * if up.is_some() { 2 } else { 1 }),
            Inner::Pcm { kind, channels } => Some(data.len() / (kind.bytes() * *channels).max(1)),
        };
        let frames = frames.filter(|&n| n > 0).unwrap_or(self.last_frames);
        AudioBuffer { sample_rate: self.sample_rate, channels: vec![vec![0.0; frames]; usize::from(self.layout.channels())] }
    }

    /// The frames every packet decodes to, for codecs whose packets all have one length: AAC
    /// (1024, 2048 for HE-AAC); `None` for PCM.
    pub fn frames_per_packet(&self) -> Option<usize> {
        match &self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => Some(aac::FRAME_LEN * if up.is_some() { 2 } else { 1 }),
            Inner::Pcm { .. } => None,
        }
    }

    /// Frames to decode before a seek target and drop, so the decoder has settled when the target
    /// is reached: two AAC frames, none for PCM.
    pub fn preroll(&self) -> usize {
        match &self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => 2 * aac::FRAME_LEN * if up.is_some() { 2 } else { 1 },
            Inner::Pcm { .. } => 0,
        }
    }

    /// Forget the decoder state, for a seek: the next packet decodes as if it were the first
    /// (codecs with overlap, such as AAC, need [`TrackDecoder::preroll`] frames to
    /// settle).
    pub fn reset(&mut self) {
        match &mut self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { dec, up, .. } => {
                dec.reset();
                if let Some(up) = up {
                    up.reset();
                }
            }
            Inner::Pcm { .. } => {}
        }
    }
}

fn unsupported(codec: CodecId) -> AudioError {
    let feature = match codec {
        CodecId::Aac | CodecId::HeAac => Some("aac-decode"),
        _ => None,
    };
    match feature {
        // A codec this crate decodes, in a build without its feature.
        Some(f) => AudioError::Unsupported(format!("decoding {} needs the `{f}` feature", codec.name())),
        None if cfg!(feature = "symphonia") => {
            AudioError::Unsupported(format!("no native {} packet decoder yet (craft_audio::decode reads it through symphonia)", codec.name()))
        }
        None => AudioError::Unsupported(format!("no native {} packet decoder yet", codec.name())),
    }
}

/// `channels` reordered by `order` (output channel → decoded channel); unchanged when the counts
/// differ.
#[cfg(feature = "aac-decode")]
fn reorder(mut channels: Vec<Vec<f32>>, order: Option<&[usize]>) -> Vec<Vec<f32>> {
    match order {
        Some(order) if order.len() == channels.len() => order.iter().map(|&c| channels.get_mut(c).map(std::mem::take).unwrap_or_default()).collect(),
        _ => channels,
    }
}

/// Append `more` to `out` channel by channel; a block whose channel count differs is dropped.
#[cfg(feature = "aac-decode")]
fn append(out: &mut Vec<Vec<f32>>, more: Vec<Vec<f32>>) {
    if out.is_empty() {
        *out = more;
    } else if out.len() == more.len() {
        for (o, c) in out.iter_mut().zip(more) {
            o.extend(c);
        }
    }
}

/// The raw access units of an ADTS-framed packet (one or more whole frames, nothing else), or
/// `None` when the packet is not ADTS. Frames with several raw data blocks are unsupported.
#[cfg(feature = "aac-decode")]
fn adts_payloads(packet: &[u8]) -> Option<Result<Vec<&[u8]>>> {
    if packet.len() < 7 || packet[0] != 0xFF || packet[1] & 0xF6 != 0xF0 {
        return None;
    }
    let frames = aac::split_adts(packet).ok()?;
    let whole: usize = frames.iter().map(|(h, _)| h.frame_length).sum();
    if whole != packet.len() {
        return None;
    }
    if frames.iter().any(|(h, _)| h.raw_data_blocks > 1) {
        return Some(Err(AudioError::Unsupported("ADTS frames with several raw data blocks".into())));
    }
    Some(Ok(frames.into_iter().map(|(_, p)| p).collect()))
}

/// The sample layout and stored format of PCM samples, `None` for a depth we do not read.
pub(crate) fn pcm_kind(bits: u16, float: bool, big_endian: bool, unsigned: bool) -> Option<(PcmKind, SampleFormat)> {
    let bytes = usize::from(bits.div_ceil(8));
    match (float, bits) {
        (true, 32) => Some((PcmKind::Float { bytes: 4, big_endian }, SampleFormat::Float32)),
        (true, 64) => Some((PcmKind::Float { bytes: 8, big_endian }, SampleFormat::Float64)),
        (false, 1..=32) => {
            let sf = match bits {
                1..=8 => SampleFormat::Int8,
                9..=16 => SampleFormat::Int16,
                17..=24 => SampleFormat::Int24,
                _ => SampleFormat::Int32,
            };
            Some((PcmKind::Int { bytes, big_endian, unsigned }, sf))
        }
        _ => None,
    }
}

#[cfg(feature = "aac-decode")]
/// The speakers of an AAC channel configuration, in the WAV order [`aac::wav_order`] gives;
/// configuration 7 and program-config layouts keep the decoded order and are unordered.
pub(crate) fn aac_layout(channel_config: u8, channels: usize) -> ChannelLayout {
    match channel_config {
        1 => ChannelLayout::MONO,
        2 => ChannelLayout::STEREO,
        3 => ChannelLayout::SURROUND_3_0,
        4 => ChannelLayout::from_mask(0x107),
        5 => ChannelLayout::from_mask(0x607),
        6 => ChannelLayout::SURROUND_5_1_SIDE,
        11 => ChannelLayout::SURROUND_6_1,
        12 => ChannelLayout::SURROUND_7_1,
        _ => ChannelLayout::unordered(crate::layout::count(channels)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PcmFormat;

    #[test]
    fn pcm_tracks_decode_in_wav_order() {
        let params = CodecParams::new(CodecId::Pcm).with_sample_rate(48_000).with_channels(2).with_pcm(PcmFormat::int(16, true, false));
        let mut dec = TrackDecoder::new(&params, &[]).unwrap();
        assert_eq!((dec.codec(), dec.sample_rate(), dec.layout()), (CodecId::Pcm, 48_000, ChannelLayout::STEREO));
        let block = dec.decode(&[0x00, 0x40, 0x00, 0xC0]).unwrap();
        assert_eq!(block.channels, vec![vec![0.5], vec![-0.5]]);
        assert_eq!(dec.conceal(None).frames(), 1);
    }

    #[test]
    fn codecs_without_a_packet_decoder_are_unsupported() {
        for codec in [CodecId::Flac, CodecId::Vorbis, CodecId::Alac, CodecId::Mp3, CodecId::Opus, CodecId::H264] {
            assert!(matches!(TrackDecoder::new(&CodecParams::new(codec), &[]), Err(AudioError::Unsupported(_))), "{codec:?}");
        }
        // A codec whose feature is off names the feature.
        #[cfg(not(feature = "aac-decode"))]
        assert!(matches!(TrackDecoder::new(&CodecParams::new(CodecId::Aac), &[]), Err(AudioError::Unsupported(m)) if m.contains("aac-decode")));
        // PCM needs its sample format.
        assert!(TrackDecoder::new(&CodecParams::new(CodecId::Pcm).with_channels(2).with_sample_rate(48_000), &[]).is_err());
    }
}
