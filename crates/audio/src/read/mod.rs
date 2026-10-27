//! Format detection, probing and decoding. The native path (craft-media's demuxers, the WAV and
//! AIFF readers, and [`TrackDecoder`]) comes first; symphonia decodes the rest.

mod aiff;
#[cfg(feature = "symphonia")]
mod caf;
#[cfg(feature = "symphonia")]
mod flac;
#[cfg(feature = "symphonia")]
mod mp4;
mod native;
mod pcm;
mod source;
#[cfg(feature = "symphonia")]
mod symph;
mod track;
mod wav;

pub use track::{LOOKAHEAD_PACKETS, TrackDecoder};

use crate::{AudioBuffer, AudioError, AudioInfo, ChannelLayout, CodecId, Container, Result};
use craft_media::ByteSource;
use native::Native;
use source::Src;
use std::sync::Arc;
#[cfg(feature = "symphonia")]
use symph::Symph;

/// The decode limit [`decode`] applies unless [`DecodeOptions::max_samples`] says otherwise:
/// 2^30 samples, 4 GiB of `f32`.
pub const DEFAULT_MAX_SAMPLES: u64 = pcm::MAX_DECODE_SAMPLES;

/// Size of an ID3v2 tag at the start of `b`, if there is one. The 10-byte footer flag (`0x10`)
/// exists only in ID3v2.4; in v2.2/v2.3 that bit is reserved and must not extend the skip.
pub(crate) fn id3v2_len(b: &[u8]) -> Option<usize> {
    if b.get(..3)? != b"ID3" {
        return None;
    }
    let version = *b.get(3)?;
    let flags = *b.get(5)?;
    let size = b.get(6..10)?.iter().fold(0usize, |acc, &x| (acc << 7) | usize::from(x & 0x7F));
    let footer = version == 4 && flags & 0x10 != 0;
    Some(10 + size + if footer { 10 } else { 0 })
}

fn detect_by_magic(b: &[u8], depth: u32) -> Option<Container> {
    let head = |n: usize| b.get(..n);
    if wav::is_wav(b) {
        return Some(Container::Wav);
    }
    if aiff::is_aiff(b) {
        return Some(Container::Aiff);
    }
    match head(4)? {
        b"fLaC" => return Some(Container::Flac),
        b"OggS" => return Some(Container::Ogg),
        b"caff" => return Some(Container::Caf),
        // Matroska and WebM differ only in the EBML header's DocType, within its first bytes.
        [0x1A, 0x45, 0xDF, 0xA3] => {
            let header = b.get(..64.min(b.len())).unwrap_or_default();
            return Some(if header.windows(4).any(|w| w == b"webm") { Container::WebM } else { Container::Matroska });
        }
        _ => {}
    }
    if b.get(4..8) == Some(b"ftyp") {
        return Some(if b.get(8..12) == Some(b"qt  ") { Container::QuickTime } else { Container::Mp4 });
    }
    if let Some(len) = id3v2_len(b) {
        if depth == 0
            && let Some(f) = b.get(len..).and_then(|rest| detect_by_magic(rest, 1))
        {
            return Some(f);
        }
        return Some(Container::MpegAudio);
    }
    let (b0, b1) = (*b.first()?, *b.get(1)?);
    if b0 == 0x0B && b1 == 0x77 {
        return Some(Container::Ac3);
    }
    if b0 == 0xFF && b1 & 0xE0 == 0xE0 {
        let layer = (b1 >> 1) & 0x3;
        if b1 & 0xF0 == 0xF0 && layer == 0 {
            return Some(Container::Adts);
        }
        if layer != 0 {
            return Some(Container::MpegAudio);
        }
    }
    None
}

fn detect_by_ext(ext: &str) -> Option<Container> {
    Some(match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "wav" | "wave" | "bwf" | "rf64" => Container::Wav,
        "aif" | "aiff" | "aifc" => Container::Aiff,
        "flac" => Container::Flac,
        "mp3" | "mp2" | "mp1" | "mpa" => Container::MpegAudio,
        "ogg" | "oga" | "opus" => Container::Ogg,
        "aac" | "adts" => Container::Adts,
        "m4a" | "m4b" | "mp4" => Container::Mp4,
        "mov" => Container::QuickTime,
        "mkv" | "mka" => Container::Matroska,
        "webm" => Container::WebM,
        "ac3" | "eac3" | "ec3" => Container::Ac3,
        "caf" => Container::Caf,
        _ => return None,
    })
}

/// The container of a file, by its magic bytes, falling back to the extension hint (`"wav"`,
/// `".flac"`); `None` when neither says.
pub fn detect_container(bytes: &[u8], ext_hint: Option<&str>) -> Option<Container> {
    detect_by_magic(bytes, 0).or_else(|| ext_hint.and_then(detect_by_ext))
}

/// Options for [`Decoder`], [`decode_with`] and [`probe_with`].
///
/// `#[non_exhaustive]`: start from `DecodeOptions::default()` and use the `with_*` methods.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Which audio track to decode, counted among the file's audio tracks as [`tracks`] lists
    /// them; `None` is the first.
    pub track: Option<usize>,
    /// The most samples (frames × channels) [`decode_with`] returns; a longer file is
    /// [`AudioError::TooLarge`]. The streaming [`Decoder`] has no limit.
    pub max_samples: u64,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        DecodeOptions { track: None, max_samples: DEFAULT_MAX_SAMPLES }
    }
}

impl DecodeOptions {
    /// Decode the audio track at index `track` (see [`DecodeOptions::track`]).
    pub fn with_track(mut self, track: usize) -> DecodeOptions {
        self.track = Some(track);
        self
    }

    /// Set [`DecodeOptions::max_samples`].
    pub fn with_max_samples(mut self, max_samples: u64) -> DecodeOptions {
        self.max_samples = max_samples;
        self
    }
}

/// One audio track of a file, as [`tracks`] lists it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TrackInfo {
    /// The value for [`DecodeOptions::track`].
    pub index: usize,
    /// The codec the container names (an AAC track that turns out to be HE-AAC is `Aac` here).
    pub codec: CodecId,
    /// The sample rate the container declares (0 when it does not say).
    pub sample_rate: u32,
    /// The channel count the container declares (0 when it does not say).
    pub channels: u16,
}

/// The decoder behind a [`Decoder`].
enum Inner<'a> {
    Native(Box<Native<'a>>),
    #[cfg(feature = "symphonia")]
    Symph(Box<Symph>),
}

/// A file's audio, decoded block by block: the streaming counterpart of [`decode`], with seeking
/// and track choice.
///
/// Native formats (WAV, AIFF, AAC, MP1/MP2, AC-3/E-AC-3, Opus and PCM in MP4/MOV, Matroska/WebM,
/// Ogg or raw streams) are read packet by packet from the source, so memory use does not grow with
/// the file. Formats symphonia decodes (MP3, FLAC, ALAC, Vorbis, CAF, ADPCM, A-law, µ-law) read the
/// whole source into memory first; their decoded audio still streams.
///
/// ```no_run
/// # fn demo() -> craft_audio::Result<()> {
/// use craft_audio::{DecodeOptions, Decoder};
///
/// let file = std::fs::File::open("long-recording.m4a")?;
/// let mut dec = Decoder::open(file, Some("m4a"), &DecodeOptions::default())?;
/// println!("{} {} Hz {}", dec.info().codec.name(), dec.info().sample_rate, dec.info().layout);
/// dec.seek(48_000 * 60)?; // one minute in
/// while let Some(block) = dec.next_block()? {
///     // block.channels: planar f32, WAV channel order
/// }
/// # Ok(()) }
/// ```
pub struct Decoder<'a> {
    inner: Inner<'a>,
}

impl Decoder<'static> {
    /// Open a file read from `source` (a `std::fs::File`, a shared buffer, any
    /// [`craft_media::ByteSource`]). `ext_hint` is the file's extension, used only when its bytes
    /// do not say what it is.
    pub fn open(source: impl ByteSource + Send + Sync + 'static, ext_hint: Option<&str>, opts: &DecodeOptions) -> Result<Decoder<'static>> {
        Decoder::from_src(Src::Shared(Arc::new(source)), ext_hint, opts)
    }
}

impl<'a> Decoder<'a> {
    /// Open a file held in memory.
    pub fn from_bytes(bytes: &'a [u8], ext_hint: Option<&str>, opts: &DecodeOptions) -> Result<Decoder<'a>> {
        Decoder::from_src(Src::Slice(bytes), ext_hint, opts)
    }

    fn from_src(src: Src<'a>, ext_hint: Option<&str>, opts: &DecodeOptions) -> Result<Decoder<'a>> {
        let head = src.head();
        let container = detect_container(&head, ext_hint);
        if let Some(n) = Native::open(&src, container, opts.track) {
            return Ok(Decoder { inner: Inner::Native(Box::new(n?)) });
        }
        Decoder::fallback(src, container, opts)
    }

    /// A file the native path does not read: symphonia's.
    #[cfg(feature = "symphonia")]
    fn fallback(src: Src<'a>, container: Option<Container>, opts: &DecodeOptions) -> Result<Decoder<'a>> {
        let mut s = Symph::open(src.whole()?, container, opts.track)?;
        // A WAV with a codec symphonia reads (ADPCM, A-law, ...) keeps its BWF metadata.
        if container == Some(Container::Wav)
            && let Ok(wav::WavParse::Foreign { bwf }) = wav::parse(&src)
        {
            s.set_bwf(bwf);
        }
        Ok(Decoder { inner: Inner::Symph(Box::new(s)) })
    }

    /// A file the native path does not read, without symphonia.
    #[cfg(not(feature = "symphonia"))]
    fn fallback(_src: Src<'a>, container: Option<Container>, _opts: &DecodeOptions) -> Result<Decoder<'a>> {
        let what = container.map_or("this file", Container::name);
        Err(AudioError::Unsupported(format!("{what} needs a decoder this build leaves out (the `decode` feature adds symphonia)")))
    }

    /// What the file holds. `frames` is 0 until decoded when the container does not say how long
    /// the track is ([`Decoder::length_known`]).
    pub fn info(&self) -> &AudioInfo {
        match &self.inner {
            Inner::Native(n) => n.info(),
            #[cfg(feature = "symphonia")]
            Inner::Symph(s) => s.info(),
        }
    }

    /// Whether [`AudioInfo::frames`] came from the file rather than being unknown (an MP3 without
    /// a Xing frame, PCM in Matroska).
    pub fn length_known(&self) -> bool {
        match &self.inner {
            Inner::Native(n) => n.frames_known(),
            #[cfg(feature = "symphonia")]
            Inner::Symph(s) => s.frames_known(),
        }
    }

    /// The next block of decoded audio (planar `f32`, WAV channel order when the layout is
    /// ordered), or `None` at the end. Blocks follow the codec's packets, so their length varies.
    pub fn next_block(&mut self) -> Result<Option<AudioBuffer>> {
        match &mut self.inner {
            Inner::Native(n) => n.next_block(),
            #[cfg(feature = "symphonia")]
            Inner::Symph(s) => s.next_block(),
        }
    }

    /// Continue decoding from frame `frame` (samples per channel from the start); the next block
    /// starts exactly there. Returns the frame decoding resumes at.
    pub fn seek(&mut self, frame: u64) -> Result<u64> {
        match &mut self.inner {
            Inner::Native(n) => n.seek(frame),
            #[cfg(feature = "symphonia")]
            Inner::Symph(s) => s.seek(frame),
        }
    }
}

/// Read a file's container, codec, sample rate, channel layout, length and BWF metadata, without
/// decoding the audio where the headers say enough.
///
/// ```
/// # #[cfg(feature = "lossless-encode")] {
/// use craft_audio::{AudioBuffer, ChannelLayout, CodecId, Container, EncodeOptions, encode, probe};
///
/// let wav = encode(&AudioBuffer::new(48_000, 2, 4800), &EncodeOptions::default()).unwrap();
/// let info = probe(&wav, Some("wav")).unwrap();
/// assert_eq!((info.container, info.codec, info.layout), (Container::Wav, CodecId::Pcm, ChannelLayout::STEREO));
/// assert_eq!((info.sample_rate, info.frames), (48_000, 4800));
/// # }
/// ```
pub fn probe(bytes: &[u8], ext_hint: Option<&str>) -> Result<AudioInfo> {
    probe_with(bytes, ext_hint, &DecodeOptions::default())
}

/// [`probe`] for the track `opts` names.
pub fn probe_with(bytes: &[u8], ext_hint: Option<&str>, opts: &DecodeOptions) -> Result<AudioInfo> {
    let mut d = Decoder::from_bytes(bytes, ext_hint, opts)?;
    if d.length_known() {
        return Ok(d.info().clone());
    }
    // The length is not in the headers: count it by decoding.
    let mut frames = 0u64;
    let mut channels = None;
    while let Some(b) = d.next_block()? {
        frames = frames.saturating_add(b.frames() as u64);
        channels.get_or_insert(b.channels.len());
    }
    let mut info = d.info().clone();
    info.frames = frames;
    fit_channels(&mut info, channels);
    Ok(info)
}

/// Decode the first audio track of a file to planar `f32`, trimmed to its presentation length, in
/// WAV channel order when [`AudioInfo::layout`] is ordered. Use [`Decoder`] to stream a long
/// file instead, and [`decode_with`] for another track or size limit.
///
/// ```
/// # #[cfg(feature = "lossless-encode")] {
/// use craft_audio::{AudioBuffer, EncodeFormat, EncodeOptions, decode, encode};
///
/// let tone: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
/// let flac = encode(&AudioBuffer { sample_rate: 48_000, channels: vec![tone] }, &EncodeOptions { format: EncodeFormat::Flac, ..Default::default() }).unwrap();
/// let (info, buf) = decode(&flac, None).unwrap();
/// assert_eq!((info.channels, buf.frames()), (1, 4800));
/// # }
/// ```
pub fn decode(bytes: &[u8], ext_hint: Option<&str>) -> Result<(AudioInfo, AudioBuffer)> {
    decode_with(bytes, ext_hint, &DecodeOptions::default())
}

/// [`decode`] with options: another track, or another size limit.
pub fn decode_with(bytes: &[u8], ext_hint: Option<&str>, opts: &DecodeOptions) -> Result<(AudioInfo, AudioBuffer)> {
    let mut d = Decoder::from_bytes(bytes, ext_hint, opts)?;
    let declared = d.info().frames.saturating_mul(u64::from(d.info().channels.max(1)));
    if d.length_known() && declared > opts.max_samples {
        return Err(AudioError::TooLarge(format!("{declared} samples exceeds the decode limit")));
    }
    let mut planar: Vec<Vec<f32>> = Vec::new();
    let mut frames = 0u64;
    while let Some(block) = d.next_block()? {
        if planar.is_empty() {
            planar = vec![Vec::new(); block.channels.len()];
        }
        if block.channels.len() != planar.len() {
            continue;
        }
        frames = frames.saturating_add(block.frames() as u64);
        if frames.saturating_mul(planar.len() as u64) > opts.max_samples {
            return Err(AudioError::TooLarge("decoded audio exceeds the decode limit".into()));
        }
        for (dst, src) in planar.iter_mut().zip(block.channels) {
            dst.try_reserve(src.len()).map_err(|_| AudioError::TooLarge("cannot allocate decoded audio".into()))?;
            dst.extend(src);
        }
    }
    let mut info = d.info().clone();
    let channels = (!planar.is_empty()).then_some(planar.len());
    fit_channels(&mut info, channels);
    if planar.is_empty() {
        planar = vec![Vec::new(); usize::from(info.channels)];
    }
    info.frames = planar.iter().map(Vec::len).min().unwrap_or(0) as u64;
    let sample_rate = info.sample_rate;
    Ok((info, AudioBuffer { sample_rate, channels: planar }))
}

/// The decoded channel count is authoritative: a layout that names another count is dropped.
fn fit_channels(info: &mut AudioInfo, decoded: Option<usize>) {
    if let Some(ch) = decoded
        && ch != usize::from(info.channels)
    {
        info.channels = u16::try_from(ch).unwrap_or(u16::MAX);
        info.layout = ChannelLayout::unordered(ch);
    }
}

/// The audio tracks of a file, in the order [`DecodeOptions::track`] counts them.
pub fn tracks(bytes: &[u8], ext_hint: Option<&str>) -> Result<Vec<TrackInfo>> {
    tracks_in(&Src::Slice(bytes), ext_hint)
}

/// [`tracks`] for a file read from a source.
pub fn tracks_of(source: impl ByteSource + Send + Sync + 'static, ext_hint: Option<&str>) -> Result<Vec<TrackInfo>> {
    tracks_in(&Src::Shared(Arc::new(source)), ext_hint)
}

fn tracks_in(src: &Src<'_>, ext_hint: Option<&str>) -> Result<Vec<TrackInfo>> {
    let container = detect_container(&src.head(), ext_hint);
    let list = |v: Vec<(CodecId, u32, u16)>| -> Vec<TrackInfo> {
        v.into_iter().enumerate().map(|(index, (codec, sample_rate, channels))| TrackInfo { index, codec, sample_rate, channels }).collect()
    };
    if let Some(params) = native::tracks(src, container) {
        return Ok(list(params.into_iter().map(|p| (p.codec, p.sample_rate.unwrap_or(0), p.channels.unwrap_or(0))).collect()));
    }
    match Native::open(src, container, None) {
        // Single-track formats: WAV, AIFF, raw streams.
        Some(n) => {
            let n = n?;
            let i = n.info();
            Ok(list(vec![(i.codec, i.sample_rate, i.channels)]))
        }
        #[cfg(feature = "symphonia")]
        None => Ok(list(symph::tracks(src.whole()?, container)?)),
        #[cfg(not(feature = "symphonia"))]
        None => Ok(Vec::new()),
    }
}

#[cfg(all(test, not(feature = "symphonia")))]
mod native_only_tests {
    use super::*;

    /// Without symphonia, a format with no native decoder is unsupported, not malformed.
    #[test]
    fn formats_left_to_symphonia_are_unsupported() {
        let flac_header = [&b"fLaC"[..], &[0x80, 0, 0, 34], &[0; 34]].concat();
        assert!(matches!(decode(&flac_header, None), Err(AudioError::Unsupported(m)) if m.contains("FLAC")));
        assert!(matches!(probe(&flac_header, None), Err(AudioError::Unsupported(_))));
    }
}
