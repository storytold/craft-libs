//! [`TrackDecoder`]: one track's packets in, planar `f32` in WAV channel order out, whatever the
//! codec. The file-level [`crate::decode`] and [`crate::Decoder`] use it for every native codec;
//! an app with its own demuxer (MXF, MPEG-TS, a network stream) uses it directly.

use crate::pcm::{PcmKind, deinterleave};
use crate::{AudioBuffer, AudioError, ChannelLayout, CodecId, CodecParams, Result, SampleFormat};

/// How many leading packets [`TrackDecoder::new`] looks at.
pub const LOOKAHEAD_PACKETS: usize = 8;

/// The codec-specific decoder behind a [`TrackDecoder`].
enum Inner {
    Pcm { kind: PcmKind, channels: usize },
}

/// Decodes the packets of one audio track to planar `f32`, in WAV channel order when
/// [`TrackDecoder::layout`] is ordered.
///
/// It takes the track's [`CodecParams`] (from `isobmff::Track::codec_params`,
/// `matroska::Track::codec_params`, `ogg::Stream::codec_params`, or built by hand for another
/// container) and decodes PCM. Container timing (priming, end trimming) is the caller's: every
/// decoded sample is returned.
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
    /// (up to [`LOOKAHEAD_PACKETS`]; empty when they are not at hand), for codecs that find their
    /// configuration in the stream; PCM needs none. Missing configuration is
    /// [`AudioError::Malformed`].
    ///
    /// Codecs without a native packet decoder yet (AAC, MP3, FLAC, ALAC, Vorbis, Opus) are
    /// [`AudioError::Unsupported`]; `craft_audio::decode` decodes them through symphonia when the
    /// `symphonia` feature is on.
    pub fn new(params: &CodecParams, lookahead: &[&[u8]]) -> Result<TrackDecoder> {
        let rate = params.sample_rate.unwrap_or(0);
        let channels = usize::from(params.channels.unwrap_or(0));
        // PCM needs no lookahead; the codec decoders take their configuration from it.
        let _ = lookahead;
        let (inner, codec, sample_rate, layout, sample_format) = match params.codec {
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

    /// The codec.
    pub fn codec(&self) -> CodecId {
        self.codec
    }

    /// The rate of the decoded audio.
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
    /// silence as long as the packet would have been (from its size, or the last packet). Empty
    /// when the length is unknown.
    pub fn conceal(&mut self, packet: Option<&[u8]>) -> AudioBuffer {
        let data = packet.unwrap_or_default();
        let frames = match &mut self.inner {
            Inner::Pcm { kind, channels } => Some(data.len() / (kind.bytes() * *channels).max(1)),
        };
        let frames = frames.filter(|&n| n > 0).unwrap_or(self.last_frames);
        AudioBuffer { sample_rate: self.sample_rate, channels: vec![vec![0.0; frames]; usize::from(self.layout.channels())] }
    }

    /// The frames every packet decodes to, for codecs whose packets all have one length; `None`
    /// for PCM.
    pub fn frames_per_packet(&self) -> Option<usize> {
        match &self.inner {
            Inner::Pcm { .. } => None,
        }
    }

    /// Frames to decode before a seek target and drop, so the decoder has settled when the target
    /// is reached; none for PCM.
    pub fn preroll(&self) -> usize {
        match &self.inner {
            Inner::Pcm { .. } => 0,
        }
    }

    /// Forget the decoder state, for a seek: the next packet decodes as if it were the first
    /// (codecs with overlap need [`TrackDecoder::preroll`] frames to settle).
    pub fn reset(&mut self) {
        match &mut self.inner {
            Inner::Pcm { .. } => {}
        }
    }
}

fn unsupported(codec: CodecId) -> AudioError {
    if cfg!(feature = "symphonia") {
        AudioError::Unsupported(format!("no native {} packet decoder yet (craft_audio::decode reads it through symphonia)", codec.name()))
    } else {
        AudioError::Unsupported(format!("no native {} packet decoder yet", codec.name()))
    }
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
        for codec in [CodecId::Aac, CodecId::Flac, CodecId::Vorbis, CodecId::Alac, CodecId::Mp3, CodecId::Opus, CodecId::H264] {
            assert!(matches!(TrackDecoder::new(&CodecParams::new(codec), &[]), Err(AudioError::Unsupported(_))), "{codec:?}");
        }
        // PCM needs its sample format.
        assert!(TrackDecoder::new(&CodecParams::new(CodecId::Pcm).with_channels(2).with_sample_rate(48_000), &[]).is_err());
    }
}
