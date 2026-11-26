//! Mapping from symphonia codec, layout and track types to the crate's own.

use crate::{AudioError, AudioInfo, ChannelLayout, CodecId, Container, Result, SampleFormat};
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_ADPCM_G722, CODEC_ID_ADPCM_G726, CODEC_ID_ADPCM_G726LE, CODEC_ID_ADPCM_IMA_QT, CODEC_ID_ADPCM_IMA_WAV, CODEC_ID_ADPCM_MS,
    CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_MP1, CODEC_ID_MP2, CODEC_ID_MP3, CODEC_ID_OPUS, CODEC_ID_PCM_ALAW, CODEC_ID_PCM_F32BE, CODEC_ID_PCM_F32LE,
    CODEC_ID_PCM_F64BE, CODEC_ID_PCM_F64BE_PLANAR, CODEC_ID_PCM_F64LE, CODEC_ID_PCM_MULAW, CODEC_ID_PCM_S32LE, CODEC_ID_VORBIS,
};
use symphonia::core::codecs::audio::{AudioCodecId, AudioCodecParameters, CODEC_ID_NULL_AUDIO};
use symphonia::core::formats::{FormatReader, Track};

/// The codec symphonia reports, as a [`CodecId`].
pub(super) fn codec_id_of(c: AudioCodecId) -> CodecId {
    match c {
        c if (CODEC_ID_PCM_S32LE..=CODEC_ID_PCM_F64BE_PLANAR).contains(&c) => CodecId::Pcm,
        c if c == CODEC_ID_PCM_ALAW => CodecId::ALaw,
        c if c == CODEC_ID_PCM_MULAW => CodecId::MuLaw,
        c if [CODEC_ID_ADPCM_G722, CODEC_ID_ADPCM_G726, CODEC_ID_ADPCM_G726LE, CODEC_ID_ADPCM_IMA_QT, CODEC_ID_ADPCM_IMA_WAV, CODEC_ID_ADPCM_MS]
            .contains(&c) =>
        {
            CodecId::Adpcm
        }
        c if c == CODEC_ID_FLAC => CodecId::Flac,
        c if c == CODEC_ID_ALAC => CodecId::Alac,
        c if c == CODEC_ID_AAC => CodecId::Aac,
        c if c == CODEC_ID_MP1 => CodecId::Mp1,
        c if c == CODEC_ID_MP2 => CodecId::Mp2,
        c if c == CODEC_ID_MP3 => CodecId::Mp3,
        c if c == CODEC_ID_OPUS => CodecId::Opus,
        c if c == CODEC_ID_VORBIS => CodecId::Vorbis,
        _ => CodecId::Unknown,
    }
}

/// The container a symphonia reader stands for, refined by the detection (which tells MP4 from
/// QuickTime and Matroska from WebM).
pub(super) fn container_of(short_name: &str, detected: Option<Container>) -> Container {
    match short_name {
        "wave" => Container::Wav,
        "aiff" => Container::Aiff,
        "flac" => Container::Flac,
        "caf" => Container::Caf,
        "ogg" => Container::Ogg,
        "isomp4" if detected == Some(Container::QuickTime) => Container::QuickTime,
        "isomp4" => Container::Mp4,
        "matroska" if detected == Some(Container::WebM) => Container::WebM,
        "matroska" => Container::Matroska,
        "mp1" | "mp2" | "mp3" => Container::MpegAudio,
        "aac" | "adts" => Container::Adts,
        _ => detected.unwrap_or(Container::MpegAudio),
    }
}

/// The speakers of symphonia's channel description; its first 18 position bits are the WAV mask
/// bits, and its decoded planes follow them.
fn layout_of(channels: Option<&Channels>) -> ChannelLayout {
    match channels {
        Some(Channels::Positioned(p)) => {
            let bits = p.bits();
            match u32::try_from(bits) {
                Ok(mask) if mask != 0 => ChannelLayout::from_mask(mask),
                _ => ChannelLayout::unordered(bits.count_ones() as u16),
            }
        }
        Some(c) => ChannelLayout::unordered(crate::layout::count(c.count())),
        None => ChannelLayout::unordered(0),
    }
}

/// The stored sample format, using the codec and its declared bit depth. Lossy and unusual
/// codecs decode to `Compressed`.
pub(super) fn sample_format_of(p: &AudioCodecParameters) -> SampleFormat {
    let c = p.codec;
    if c == CODEC_ID_PCM_F32LE || c == CODEC_ID_PCM_F32BE {
        return SampleFormat::Float32;
    }
    if c == CODEC_ID_PCM_F64LE || c == CODEC_ID_PCM_F64BE {
        return SampleFormat::Float64;
    }
    let lossy = [
        CODEC_ID_MP1,
        CODEC_ID_MP2,
        CODEC_ID_MP3,
        CODEC_ID_AAC,
        CODEC_ID_VORBIS,
        CODEC_ID_OPUS,
        CODEC_ID_ADPCM_MS,
        CODEC_ID_ADPCM_IMA_WAV,
        CODEC_ID_ADPCM_IMA_QT,
        CODEC_ID_ADPCM_G722,
        CODEC_ID_ADPCM_G726,
        CODEC_ID_ADPCM_G726LE,
        CODEC_ID_PCM_ALAW,
        CODEC_ID_PCM_MULAW,
    ];
    let lossless = !lossy.contains(&c);
    match (lossless, p.bits_per_sample) {
        (true, Some(1..=8)) => SampleFormat::Int8,
        (true, Some(9..=16)) => SampleFormat::Int16,
        (true, Some(17..=24)) => SampleFormat::Int24,
        (true, Some(25..=32)) => SampleFormat::Int32,
        _ => SampleFormat::Compressed,
    }
}

/// The audio parameters of `track`, if it carries any.
pub(super) fn audio_params(track: &Track) -> Option<&AudioCodecParameters> {
    track.codec_params.as_ref()?.audio()
}

/// The audio tracks with a decodable codec.
pub(super) fn audio_tracks(reader: &dyn FormatReader) -> impl Iterator<Item = &Track> {
    reader.tracks().iter().filter(|t| audio_params(t).is_some_and(|p| p.codec != CODEC_ID_NULL_AUDIO))
}

/// The `n`-th audio track (the first when `None`), counting an Ogg file's `streams` in file order
/// when they are known. With no `n`, the first stream symphonia has a track for: it has none for
/// some of the codecs `tracks` lists.
fn pick_track<'a>(reader: &'a dyn FormatReader, streams: &[(u32, Option<u32>)], n: Option<usize>) -> Option<&'a Track> {
    let with_serial = |serial: u32| audio_tracks(reader).find(|t| t.id == serial);
    match n {
        _ if streams.is_empty() => audio_tracks(reader).nth(n.unwrap_or(0)),
        Some(n) => with_serial(streams.get(n)?.0),
        None => streams.iter().find_map(|s| with_serial(s.0)),
    }
}

/// The `n`-th audio track (see [`pick_track`]), with the stream information the UI shows.
pub(super) fn info_of(
    reader: &dyn FormatReader,
    detected: Option<Container>,
    ogg_streams: &[(u32, Option<u32>)],
    n: Option<usize>,
) -> Result<(u32, AudioInfo)> {
    let container = container_of(reader.format_info().short_name, detected);
    let track = pick_track(reader, ogg_streams, n).ok_or_else(|| AudioError::Unsupported("no decodable audio track".into()))?;
    let p = audio_params(track).ok_or_else(|| AudioError::Unsupported("no decodable audio track".into()))?;
    let sample_rate = p.sample_rate.filter(|&r| r > 0).ok_or_else(|| AudioError::Malformed("unknown sample rate".into()))?;
    let layout = layout_of(p.channels.as_ref());
    let frames = track.num_frames.unwrap_or(0);
    Ok((track.id, AudioInfo::new(container, codec_id_of(p.codec), sample_format_of(p), sample_rate, layout, frames)))
}
