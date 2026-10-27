//! Container and codec identifiers and codec parameters shared by the codec crates (craft-audio,
//! and the video crate to come), so a file's container and codec are named the same way
//! everywhere, and a codec is configured the same way whatever container its track came from.
//!
//! Both enums are `#[non_exhaustive]`: new formats are added as minor changes, so a `match` on them
//! needs a `_` arm.

#![warn(missing_docs)]

/// The file or stream format that holds the codec data.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Container {
    /// RIFF WAVE, including BWF and RF64/BW64.
    Wav,
    /// AIFF and AIFF-C.
    Aiff,
    /// A native FLAC stream (`fLaC`).
    Flac,
    /// ISO base media file format: MP4, M4A, M4B.
    Mp4,
    /// QuickTime (MOV).
    QuickTime,
    /// Matroska (MKV, MKA).
    Matroska,
    /// WebM (the Matroska subset).
    WebM,
    /// Ogg.
    Ogg,
    /// Apple Core Audio Format.
    Caf,
    /// A raw ADTS AAC stream.
    Adts,
    /// A raw MPEG audio stream (MP1, MP2 or MP3 frames, optionally behind ID3v2 tags).
    MpegAudio,
    /// A raw AC-3 or E-AC-3 stream.
    Ac3,
}

impl Container {
    /// A short display name: `"WAV"`, `"MP4"`, `"Matroska"`.
    pub fn name(self) -> &'static str {
        match self {
            Container::Wav => "WAV",
            Container::Aiff => "AIFF",
            Container::Flac => "FLAC",
            Container::Mp4 => "MP4",
            Container::QuickTime => "QuickTime",
            Container::Matroska => "Matroska",
            Container::WebM => "WebM",
            Container::Ogg => "Ogg",
            Container::Caf => "CAF",
            Container::Adts => "ADTS",
            Container::MpegAudio => "MPEG audio",
            Container::Ac3 => "AC-3",
        }
    }
}

/// A codec, audio or video.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CodecId {
    /// Uncompressed PCM, integer or float.
    Pcm,
    /// ADPCM (IMA, Microsoft, G.722, G.726).
    Adpcm,
    /// G.711 A-law.
    ALaw,
    /// G.711 µ-law.
    MuLaw,
    /// FLAC.
    Flac,
    /// Apple Lossless.
    Alac,
    /// AAC (LC). Containers report every AAC stream as `Aac`; a decoder that finds spectral band
    /// replication reports [`CodecId::HeAac`].
    Aac,
    /// HE-AAC (AAC with spectral band replication, v1 or v2).
    HeAac,
    /// MPEG-1/2 Audio Layer I.
    Mp1,
    /// MPEG-1/2 Audio Layer II.
    Mp2,
    /// MPEG-1/2 Audio Layer III.
    Mp3,
    /// AC-3 (Dolby Digital).
    Ac3,
    /// E-AC-3 (Dolby Digital Plus).
    Eac3,
    /// Opus.
    Opus,
    /// Vorbis.
    Vorbis,
    /// Speex.
    Speex,
    /// H.264 / AVC.
    H264,
    /// H.265 / HEVC.
    Hevc,
    /// AV1.
    Av1,
    /// VP8.
    Vp8,
    /// VP9.
    Vp9,
    /// Apple ProRes.
    ProRes,
    /// Motion JPEG.
    Mjpeg,
    /// MPEG-2 video.
    Mpeg2Video,
    /// MPEG-4 Part 2 video.
    Mpeg4Video,
    /// Theora.
    Theora,
    /// Avid DNxHD / DNxHR.
    Dnxhd,
    /// APV (Advanced Professional Video).
    Apv,
    /// A codec this crate has no identifier for.
    Unknown,
}

impl CodecId {
    /// A short display name: `"AAC"`, `"HE-AAC"`, `"E-AC-3"`, `"H.264"`.
    pub fn name(self) -> &'static str {
        match self {
            CodecId::Pcm => "PCM",
            CodecId::Adpcm => "ADPCM",
            CodecId::ALaw => "A-law",
            CodecId::MuLaw => "µ-law",
            CodecId::Flac => "FLAC",
            CodecId::Alac => "ALAC",
            CodecId::Aac => "AAC",
            CodecId::HeAac => "HE-AAC",
            CodecId::Mp1 => "MP1",
            CodecId::Mp2 => "MP2",
            CodecId::Mp3 => "MP3",
            CodecId::Ac3 => "AC-3",
            CodecId::Eac3 => "E-AC-3",
            CodecId::Opus => "Opus",
            CodecId::Vorbis => "Vorbis",
            CodecId::Speex => "Speex",
            CodecId::H264 => "H.264",
            CodecId::Hevc => "HEVC",
            CodecId::Av1 => "AV1",
            CodecId::Vp8 => "VP8",
            CodecId::Vp9 => "VP9",
            CodecId::ProRes => "ProRes",
            CodecId::Mjpeg => "Motion JPEG",
            CodecId::Mpeg2Video => "MPEG-2",
            CodecId::Mpeg4Video => "MPEG-4",
            CodecId::Theora => "Theora",
            CodecId::Dnxhd => "DNxHD",
            CodecId::Apv => "APV",
            CodecId::Unknown => "unknown",
        }
    }

    /// Whether this is an audio codec.
    pub fn is_audio(self) -> bool {
        matches!(
            self,
            CodecId::Pcm
                | CodecId::Adpcm
                | CodecId::ALaw
                | CodecId::MuLaw
                | CodecId::Flac
                | CodecId::Alac
                | CodecId::Aac
                | CodecId::HeAac
                | CodecId::Mp1
                | CodecId::Mp2
                | CodecId::Mp3
                | CodecId::Ac3
                | CodecId::Eac3
                | CodecId::Opus
                | CodecId::Vorbis
                | CodecId::Speex
        )
    }

    /// Whether this is a video codec.
    pub fn is_video(self) -> bool {
        matches!(
            self,
            CodecId::H264
                | CodecId::Hevc
                | CodecId::Av1
                | CodecId::Vp8
                | CodecId::Vp9
                | CodecId::ProRes
                | CodecId::Mjpeg
                | CodecId::Mpeg2Video
                | CodecId::Mpeg4Video
                | CodecId::Theora
                | CodecId::Dnxhd
                | CodecId::Apv
        )
    }
}

/// The layout of uncompressed PCM samples.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PcmFormat {
    /// Bits per sample (8, 16, 24, 32 for integers; 32 or 64 for floats).
    pub bits: u16,
    /// IEEE float samples.
    pub float: bool,
    /// Big-endian byte order.
    pub big_endian: bool,
    /// Signed integers (8-bit WAV and `raw ` QuickTime PCM are unsigned).
    pub signed: bool,
}

impl PcmFormat {
    /// Integer samples of `bits` bits.
    pub fn int(bits: u16, signed: bool, big_endian: bool) -> PcmFormat {
        PcmFormat { bits, float: false, big_endian, signed }
    }

    /// IEEE float samples of `bits` bits (32 or 64).
    pub fn float(bits: u16, big_endian: bool) -> PcmFormat {
        PcmFormat { bits, float: true, big_endian, signed: true }
    }
}

/// What a decoder needs to know about a track, whatever container it came from.
///
/// `extradata` is the codec configuration in Matroska's `CodecPrivate` form, which ffmpeg's
/// extradata also follows:
///
/// | Codec | `extradata` |
/// |---|---|
/// | AAC | `AudioSpecificConfig` |
/// | Opus | the `OpusHead` identification header (RFC 7845 §5.1) |
/// | Vorbis | the three headers, Xiph-laced |
/// | FLAC | `fLaC` followed by the metadata blocks (STREAMINFO first) |
/// | ALAC | the magic cookie (`ALACSpecificConfig`) |
/// | AC-3 / E-AC-3 | the MP4 `dac3` / `dec3` payload, or empty |
/// | MP1/MP2/MP3, PCM | empty |
/// | H.264 / HEVC / AV1 | the `avcC` / `hvcC` / `av1C` record |
/// | VP9 | the MP4 `vpcC` payload, or Matroska's codec feature metadata |
/// | APV | the `apvC` record |
///
/// `sample_rate` and `channels` are what the container declares (for HE-AAC with implicit
/// signalling, usually the core rate); `width` and `height` are the coded picture size of a video
/// track. `tag` is the container's four-character code for the codec when it has one (the MP4
/// sample entry format, Matroska's ProRes or VfW FourCC), which is how a ProRes profile (`apcn`,
/// `ap4h`, …) or a DNx variant is told apart.
///
/// `#[non_exhaustive]`: build it with [`CodecParams::new`] and the `with_*` methods.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CodecParams {
    /// The codec.
    pub codec: CodecId,
    /// The codec configuration (see the table above).
    pub extradata: Vec<u8>,
    /// The container's four-character code for the codec, if it has one.
    pub tag: Option<[u8; 4]>,
    /// Audio: samples per second.
    pub sample_rate: Option<u32>,
    /// Audio: channel count.
    pub channels: Option<u16>,
    /// Audio: the sample layout of uncompressed PCM.
    pub pcm: Option<PcmFormat>,
    /// Video: coded picture width in pixels.
    pub width: Option<u32>,
    /// Video: coded picture height in pixels.
    pub height: Option<u32>,
}

impl CodecParams {
    /// Parameters for `codec`, with nothing else known yet.
    pub fn new(codec: CodecId) -> CodecParams {
        CodecParams { codec, extradata: Vec::new(), tag: None, sample_rate: None, channels: None, pcm: None, width: None, height: None }
    }

    /// Sets `tag`.
    pub fn with_tag(mut self, tag: [u8; 4]) -> CodecParams {
        self.tag = Some(tag);
        self
    }

    /// The coded picture size; a zero dimension leaves both unknown.
    pub fn with_size(mut self, width: u32, height: u32) -> CodecParams {
        (self.width, self.height) = if width > 0 && height > 0 { (Some(width), Some(height)) } else { (None, None) };
        self
    }

    /// Sets `extradata`.
    pub fn with_extradata(mut self, extradata: impl Into<Vec<u8>>) -> CodecParams {
        self.extradata = extradata.into();
        self
    }

    /// Sets `sample_rate`; 0 leaves it unknown.
    pub fn with_sample_rate(mut self, sample_rate: u32) -> CodecParams {
        self.sample_rate = (sample_rate > 0).then_some(sample_rate);
        self
    }

    /// Sets `channels`; 0 leaves it unknown.
    pub fn with_channels(mut self, channels: u16) -> CodecParams {
        self.channels = (channels > 0).then_some(channels);
        self
    }

    /// Sets `pcm`.
    pub fn with_pcm(mut self, pcm: PcmFormat) -> CodecParams {
        self.pcm = Some(pcm);
        self
    }
}

/// Xiph lacing of `packets` (Matroska `CodecPrivate` for Vorbis): the count less one, each
/// length but the last as a run of 255s and a remainder, then the packets. `None` for no packets
/// or more than 256, which the one-byte count cannot hold.
pub fn xiph_lace(packets: &[Vec<u8>]) -> Option<Vec<u8>> {
    let mut out = vec![u8::try_from(packets.len().checked_sub(1)?).ok()?];
    for p in packets.iter().take(packets.len().saturating_sub(1)) {
        let mut n = p.len();
        while n >= 255 {
            out.push(255);
            n -= 255;
        }
        out.push(n as u8);
    }
    for p in packets {
        out.extend_from_slice(p);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_and_video_are_disjoint() {
        for c in [CodecId::Aac, CodecId::Opus, CodecId::Pcm, CodecId::Eac3, CodecId::Speex] {
            assert!(c.is_audio() && !c.is_video(), "{c:?}");
        }
        for c in [CodecId::H264, CodecId::Av1, CodecId::ProRes, CodecId::Theora, CodecId::Dnxhd, CodecId::Apv] {
            assert!(c.is_video() && !c.is_audio(), "{c:?}");
        }
        assert!(!CodecId::Unknown.is_audio() && !CodecId::Unknown.is_video());
    }

    #[test]
    fn params_builder() {
        let p = CodecParams::new(CodecId::Aac).with_extradata([0x12, 0x10]).with_sample_rate(48_000).with_channels(2).with_sample_rate(0);
        assert_eq!((p.codec, p.extradata.as_slice(), p.sample_rate, p.channels), (CodecId::Aac, &[0x12, 0x10][..], None, Some(2)));
        let v = CodecParams::new(CodecId::ProRes).with_tag(*b"apcn").with_size(1920, 1080);
        assert_eq!((v.tag, v.width, v.height), (Some(*b"apcn"), Some(1920), Some(1080)));
        assert_eq!(v.with_size(0, 1080).width, None);
    }

    #[test]
    fn xiph_lacing() {
        let laced = xiph_lace(&[vec![1; 30], vec![2; 300], vec![3; 5]]).unwrap();
        assert_eq!(&laced[..4], &[2, 30, 255, 45]);
        assert_eq!(laced.len(), 4 + 335);
        assert_eq!(xiph_lace(&[]), None);
        assert_eq!(xiph_lace(&vec![vec![0]; 257]), None);
        assert!(xiph_lace(&vec![vec![0]; 256]).is_some());
    }
}
