//! Compositor media sources and a pluggable opener registry.
//!
//! This crate is the EffectCraft / FilmCraft opener hub: a [`MediaSource`] yields compositor
//! [`VideoFrame`]s and [`AudioBuffer`]s, and [`open_bytes`] / [`open_reader`] try caller-supplied
//! openers in order. It lives in its own crate rather than `craft-media` so it can depend on
//! `craft-audio` without a cycle (`craft-audio` already depends on `craft-media`).
//!
//! Built-in still-image, WAV, and generator openers stay in the apps. [`open_bytes`] only runs the
//! `extra` openers; [`open_reader`] tries `reader_openers` and otherwise reads the file whole
//! (up to `max_whole`) and falls back to [`open_bytes`]. Container sources register themselves
//! from later PRs.
//!
//! [`SharedSource`] is `Arc<dyn MediaSource>`. It is not `craft_audio::SharedSource`, which is a
//! `ByteSource` marker used by the audio decoder.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::io;
use std::path::Path;
use std::sync::Arc;

use craft_audio::AudioBuffer;
use craft_color::{ColorInfo, HdrMetadata};
use craft_frame::VideoFrame;
use craft_media::ByteSource;
use craft_time::{FrameRate, Tick};
use serde::{Deserialize, Serialize};

pub mod cancel;
#[cfg(feature = "y4m")]
pub mod y4m;

/// Errors from opening or reading a media source.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum MediaError {
    #[error("unsupported media: {0}")]
    Unsupported(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("no {0} stream")]
    NoStream(&'static str),
    #[error("media offline: {0}")]
    Offline(String),
    /// The request was cancelled.
    #[error("cancelled")]
    Cancelled,
}

impl From<io::Error> for MediaError {
    fn from(e: io::Error) -> Self {
        MediaError::Io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, MediaError>;

/// Kind of media a source reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    /// Video, possibly with audio.
    Movie,
    AudioOnly,
    Still,
    ImageSequence,
    Synthetic,
}

/// Video stream metadata a source reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoStreamInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub par: (u32, u32),
    pub codec: String,
    pub pixel_format: String,
    pub color: ColorInfo,
    pub has_alpha: bool,
    /// Bitrate in bits/s when known.
    pub bitrate: Option<u64>,
    /// HDR static metadata when the file carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<HdrMetadata>,
}

/// Most audio streams a source reports (the count comes from the file: never trusted).
pub const MAX_AUDIO_STREAMS: usize = 64;

/// Audio stream metadata a source reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioStreamInfo {
    pub sample_rate: u32,
    pub channels: u32,
    pub codec: String,
    pub bits_per_sample: Option<u32>,
}

/// Identity and streams of an open source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub name: String,
    pub kind: MediaKind,
    /// Media duration. Stills typically report a default duration chosen by the app.
    pub duration: Tick,
    pub video: Option<VideoStreamInfo>,
    /// Every audio stream the container carries, in file order. Stream 0 is the one
    /// [`MediaSource::audio`] reads; the others are read with [`MediaSource::audio_stream`].
    /// Project files written before multi-stream support hold a single `audio` object (or
    /// `null`): both still load.
    #[serde(default, alias = "audio", deserialize_with = "deserialize_audio_streams")]
    pub audio_streams: Vec<AudioStreamInfo>,
    pub container: String,
    /// Timecode of the first frame (in frames at `video.frame_rate`), if the file carries one.
    pub start_timecode: Option<i64>,
    pub file_size: Option<u64>,
}

/// Reads `audio_streams` from a list, or from the single optional stream older project files wrote.
fn deserialize_audio_streams<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<AudioStreamInfo>, D::Error> {
    struct StreamsVisitor;
    struct RejectExtra;
    impl<'de> serde::de::DeserializeSeed<'de> for RejectExtra {
        type Value = ();
        fn deserialize<D: serde::Deserializer<'de>>(self, _d: D) -> std::result::Result<(), D::Error> {
            Err(serde::de::Error::custom("audio stream count exceeds MAX_AUDIO_STREAMS"))
        }
    }
    impl<'de> serde::de::Visitor<'de> for StreamsVisitor {
        type Value = Vec<AudioStreamInfo>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 64 audio streams, a legacy audio object, or null")
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> std::result::Result<Self::Value, A::Error> {
            AudioStreamInfo::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(|stream| vec![stream])
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error> {
            let mut streams = Vec::new();
            // Do not reserve from an untrusted size hint or buffer an untagged sequence.
            while streams.len() < MAX_AUDIO_STREAMS {
                let Some(stream) = seq.next_element()? else { return Ok(streams) };
                streams.push(stream);
            }
            // Detect an excess entry without deserializing or allocating its contents.
            seq.next_element_seed(RejectExtra)?;
            Ok(streams)
        }
    }
    d.deserialize_any(StreamsVisitor)
}

impl MediaInfo {
    pub fn frame_rate(&self) -> FrameRate {
        self.video.as_ref().map(|v| v.frame_rate).unwrap_or_default()
    }
    pub fn has_video(&self) -> bool {
        self.video.is_some()
    }
    pub fn has_audio(&self) -> bool {
        !self.audio_streams.is_empty()
    }
    /// The primary audio stream (stream 0), the one [`MediaSource::audio`] reads.
    pub fn audio(&self) -> Option<&AudioStreamInfo> {
        self.audio_streams.first()
    }
}

/// A request for a video frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameRequest {
    /// Media time.
    pub time: Tick,
    /// Desired scale relative to full resolution (1.0 = full). Sources may return a larger frame.
    pub scale: f32,
}

impl FrameRequest {
    pub fn full(time: Tick) -> Self {
        Self { time, scale: 1.0 }
    }
}

/// A decoded or generated media source that can yield compositor frames and audio.
pub trait MediaSource: Send + Sync {
    fn info(&self) -> &MediaInfo;
    /// The frame displayed at media time `req.time`.
    fn video_frame(&self, req: FrameRequest) -> Result<Arc<VideoFrame>>;
    /// `frames` audio frames starting at sample index `start` (at `sample_rate`, media time).
    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer>;
    /// Like [`audio`](Self::audio) for audio stream `stream` (an index into
    /// [`MediaInfo::audio_streams`]). Sources with a single audio stream need not override this:
    /// stream 0 is `audio`, any other index is [`MediaError::NoStream`].
    fn audio_stream(&self, stream: usize, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        if stream == 0 { self.audio(start, frames, sample_rate) } else { Err(MediaError::NoStream("audio")) }
    }
}

/// Shared compositor source. Distinct from [`craft_audio::SharedSource`], a `ByteSource` marker.
pub type SharedSource = Arc<dyn MediaSource>;

/// Random-access bytes used by [`open_reader`].
pub type SharedBytes = Arc<dyn ByteSource + Send + Sync>;

/// File extensions recognised at import (the media browser filters on these).
pub const STILL_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp", "tga", "dpx", "exr", "hdr", "dng"];
pub const AUDIO_EXTENSIONS: &[&str] = &["wav", "wave", "bwf", "aif", "aiff", "mp3", "mp2", "m4a", "aac", "flac", "ogg", "opus"];
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mxf", "mts", "m2ts", "m2t", "ts", "mpg", "mpeg", "vob", "mod", "tod", "m2v", "m1v", "mpv", "3gp",
    "y4m", "apv",
];

/// True when `path` has a still, audio, or video extension we recognise, ignoring case.
pub fn is_importable(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    STILL_EXTENSIONS.contains(&ext.as_str()) || AUDIO_EXTENSIONS.contains(&ext.as_str()) || VIDEO_EXTENSIONS.contains(&ext.as_str())
}

/// A pluggable in-memory opener. Returns `None` when the format is not this opener's.
pub type Opener = fn(name: &str, bytes: Arc<[u8]>) -> Option<Result<SharedSource>>;

/// Opens a container from a reader: `head` is the first bytes of the file (for sniffing).
/// Returns `None` when the format is not this opener's.
pub type ReaderOpener = fn(name: &str, head: &[u8], reader: &SharedBytes) -> Option<Result<SharedSource>>;

/// Bytes sniffed by [`open_reader`].
pub const HEAD_LEN: usize = 64 * 1024;

/// Default ceiling for a whole-file fallback or an offline output buffer (256 MiB).
/// Streaming openers are tried first. Use open_reader_within for an explicit different file budget.
pub const DEFAULT_MAX_MEDIA_BYTES: usize = 256 << 20;

fn allocate<T: Default + Clone>(len: usize, max_bytes: usize) -> io::Result<Vec<T>> {
    if len.checked_mul(std::mem::size_of::<T>()).is_none_or(|bytes| bytes > max_bytes || bytes > isize::MAX as usize) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "media buffer exceeds allocation limit"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|e| io::Error::other(format!("cannot allocate media buffer: {e}")))?;
    out.resize(len, T::default());
    Ok(out)
}

fn io_named(name: &str, e: io::Error) -> MediaError {
    MediaError::Io(format!("{name}: {e}"))
}

/// Read `len` bytes at `offset` (fewer at the end of the source).
pub fn read_range(r: &dyn ByteSource, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    read_range_within(r, offset, len, DEFAULT_MAX_MEDIA_BYTES)
}

fn read_range_within(r: &dyn ByteSource, offset: u64, len: usize, max_bytes: usize) -> io::Result<Vec<u8>> {
    let n = usize::try_from(r.len().saturating_sub(offset).min(len as u64))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "media length is not representable"))?;
    let mut v = allocate(n, max_bytes)?;
    r.read_at(offset, &mut v)?;
    Ok(v)
}

/// Open media from bytes by trying `extra` openers in order. There is no built-in fallback:
/// stills, WAV, and generators stay in the apps and register here.
pub fn open_bytes(name: &str, bytes: Arc<[u8]>, extra: &[Opener]) -> Result<SharedSource> {
    for o in extra {
        if let Some(r) = o(name, bytes.clone()) {
            return r;
        }
    }
    Err(MediaError::Unsupported(format!("{name}: unrecognised format")))
}

/// Open media from a reader: `reader_openers` first; otherwise read the whole file (if it fits in
/// `max_whole`) and use [`open_bytes`] with `extra`.
pub fn open_reader(name: &str, reader: SharedBytes, reader_openers: &[ReaderOpener], extra: &[Opener]) -> Result<SharedSource> {
    open_reader_within(name, reader, reader_openers, extra, DEFAULT_MAX_MEDIA_BYTES as u64)
}

/// Like [`open_reader`], but a file no reader opener takes is read whole only up to `max_whole`
/// bytes; a larger one is refused instead. For looking at files rather than importing them:
/// reading a multi-gigabyte file into memory to show its duration, or to find out it isn't
/// supported, thrashes the disk.
pub fn open_reader_within(
    name: &str,
    reader: SharedBytes,
    reader_openers: &[ReaderOpener],
    extra: &[Opener],
    max_whole: u64,
) -> Result<SharedSource> {
    let head = read_range(&*reader, 0, HEAD_LEN).map_err(|e| io_named(name, e))?;
    for o in reader_openers {
        if let Some(r) = o(name, &head, &reader) {
            return r;
        }
    }
    if extra.is_empty() {
        return Err(MediaError::Unsupported(format!("{name}: unrecognised format")));
    }
    if reader.len() > max_whole {
        return Err(MediaError::Unsupported(format!("{name}: no streaming reader for this format, and it is too large to read whole here")));
    }
    let len = usize::try_from(reader.len()).map_err(|_| MediaError::Unsupported(format!("{name}: media length is not representable")))?;
    let budget = usize::try_from(max_whole).unwrap_or(usize::MAX);
    let all = read_range_within(&*reader, 0, len, budget).map_err(|e| io_named(name, e))?;
    open_bytes(name, all.into(), extra)
}

/// Open a regular file through the shared pooled FileReader. Non-files (including FIFOs)
/// are refused without a blocking open, using the same contract as other media readers.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_path(path: &Path, reader_openers: &[ReaderOpener], extra: &[Opener]) -> Result<SharedSource> {
    let file = craft_media::FileReader::open(path).map_err(|e| MediaError::Io(format!("{}: {e}", path.display())))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    open_reader(&name, Arc::new(file), reader_openers, extra)
}

fn audio_channels(info: &MediaInfo) -> u16 {
    info.audio().map_or(2, |a| u16::try_from(a.channels).unwrap_or(u16::MAX).max(1))
}

/// A source that renders a solid slate (used for offline media).
pub struct OfflineSource {
    pub info: MediaInfo,
}

impl MediaSource for OfflineSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, req: FrameRequest) -> Result<Arc<VideoFrame>> {
        let v = self.info.video.as_ref().ok_or(MediaError::NoStream("video"))?;
        if !req.scale.is_finite() || req.scale <= 0.0 || v.width == 0 || v.height == 0 {
            return Err(MediaError::Decode("invalid offline frame dimensions or scale".into()));
        }
        let s = req.scale.clamp(0.02, 1.0);
        let (w, h) = (((v.width as f32 * s) as u32).max(2), ((v.height as f32 * s) as u32).max(2));
        let samples = usize::try_from(w)
            .ok()
            .and_then(|w| usize::try_from(h).ok().and_then(|h| w.checked_mul(h)))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| MediaError::Decode("offline frame dimensions overflow".into()))?;
        let mut px = allocate::<u8>(samples, DEFAULT_MAX_MEDIA_BYTES)?;
        for p in px.as_chunks_mut::<4>().0 {
            p.copy_from_slice(&[140, 16, 16, 255]);
        }
        Ok(Arc::new(VideoFrame::rgba8(w, h, px).map_err(|e| MediaError::Decode(e.to_string()))?.with_pts(req.time)))
    }
    fn audio(&self, _start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        let channels = usize::from(audio_channels(&self.info));
        let bytes = frames
            .checked_mul(channels)
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| MediaError::Decode("offline audio dimensions overflow".into()))?;
        if bytes > DEFAULT_MAX_MEDIA_BYTES {
            return Err(MediaError::Decode("offline audio exceeds allocation limit".into()));
        }
        let mut planes = Vec::new();
        planes.try_reserve_exact(channels).map_err(|e| MediaError::Io(e.to_string()))?;
        for _ in 0..channels {
            planes.push(allocate::<f32>(frames, DEFAULT_MAX_MEDIA_BYTES)?);
        }
        Ok(AudioBuffer { sample_rate, channels: planes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent(name: &str, kind: MediaKind) -> OfflineSource {
        OfflineSource {
            info: MediaInfo {
                name: name.into(),
                kind,
                duration: Tick::from_seconds_f64(1.0),
                video: Some(VideoStreamInfo {
                    width: 16,
                    height: 8,
                    frame_rate: FrameRate::FPS_24,
                    par: (1, 1),
                    codec: "slate".into(),
                    pixel_format: "rgba8".into(),
                    color: ColorInfo::default(),
                    has_alpha: false,
                    bitrate: None,
                    hdr: None,
                }),
                audio_streams: vec![AudioStreamInfo { sample_rate: 48000, channels: 2, codec: "pcm".into(), bits_per_sample: Some(32) }],
                container: "none".into(),
                start_timecode: None,
                file_size: None,
            },
        }
    }

    fn accept_ok(name: &str, _bytes: Arc<[u8]>) -> Option<Result<SharedSource>> {
        if name.ends_with(".ok") { Some(Ok(Arc::new(silent(name, MediaKind::Synthetic)))) } else { None }
    }

    fn reject_all(_name: &str, _bytes: Arc<[u8]>) -> Option<Result<SharedSource>> {
        None
    }

    fn boom(name: &str, _bytes: Arc<[u8]>) -> Option<Result<SharedSource>> {
        Some(Err(MediaError::Decode(format!("{name}: boom"))))
    }

    fn reader_ok(name: &str, head: &[u8], _reader: &SharedBytes) -> Option<Result<SharedSource>> {
        if head.starts_with(b"OK") { Some(Ok(Arc::new(silent(name, MediaKind::Movie)))) } else { None }
    }

    struct HugeSource;
    impl ByteSource for HugeSource {
        fn len(&self) -> u64 {
            u64::MAX
        }
        fn read_at(&self, _offset: u64, buf: &mut [u8]) -> io::Result<()> {
            assert!(buf.len() <= HEAD_LEN, "must reject whole-file allocation before reading");
            buf.fill(0);
            Ok(())
        }
    }

    #[test]
    fn whole_file_fallback_and_explicit_ranges_are_bounded() {
        let reader: SharedBytes = Arc::new(HugeSource);
        assert!(open_reader("large.ok", reader.clone(), &[], &[accept_ok]).is_err());
        assert!(open_reader_within("large.ok", reader.clone(), &[], &[accept_ok], u64::MAX).is_err());
        assert!(read_range(&HugeSource, 0, usize::MAX).is_err());
        assert!(read_range_within(&HugeSource, 0, 10, 9).is_err());
        assert_eq!(read_range_within(&HugeSource, 0, 10, 10).unwrap(), vec![0; 10]);
    }

    struct SniffOnlySource(std::sync::atomic::AtomicUsize);
    impl ByteSource for SniffOnlySource {
        fn len(&self) -> u64 {
            u64::MAX
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            assert_eq!(offset, 0);
            assert_eq!(buf.len(), HEAD_LEN);
            assert_eq!(self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst), 0, "must only sniff once");
            buf.fill(0);
            buf[..2].copy_from_slice(b"OK");
            Ok(())
        }
    }

    #[test]
    fn no_fallback_stops_after_sniff_and_streaming_opener_precedes_size_cap() {
        let reader = Arc::new(SniffOnlySource(std::sync::atomic::AtomicUsize::new(0)));
        let error = open_reader("unknown", reader.clone(), &[], &[]).err().unwrap();
        assert!(matches!(error, MediaError::Unsupported(_)));
        assert_eq!(reader.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let reader = Arc::new(SniffOnlySource(std::sync::atomic::AtomicUsize::new(0)));
        let source = open_reader_within("large.mov", reader.clone(), &[reader_ok], &[accept_ok], 0).unwrap();
        assert_eq!(source.info().kind, MediaKind::Movie);
        assert_eq!(reader.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn hostile_offline_metadata_and_requests_are_errors() {
        let mut source = silent("bad", MediaKind::Synthetic);
        source.info.video.as_mut().unwrap().width = u32::MAX;
        source.info.video.as_mut().unwrap().height = u32::MAX;
        assert!(source.video_frame(FrameRequest::full(Tick::ZERO)).is_err());
        assert!(source.audio(0, usize::MAX, 48000).is_err());
        let source = silent("bad-scale", MediaKind::Synthetic);
        for scale in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert!(source.video_frame(FrameRequest { time: Tick::ZERO, scale }).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn open_path_refuses_a_fifo_without_waiting_for_a_writer() {
        let root = std::env::temp_dir().join(format!("craft-media-source-fifo-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("fifo");
        if std::process::Command::new("mkfifo").arg(&path).status().is_ok_and(|s| s.success()) {
            assert!(open_path(&path, &[], &[]).is_err());
            std::fs::remove_file(&path).unwrap();
        }
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn importable_extensions_match_regardless_of_case() {
        for name in ["clip.mov", "clip.MOV", "clip.Mov", "shot.MP4", "IMG_0001.JPG", "sound.WAV", "take.M2TS"] {
            assert!(is_importable(Path::new(name)), "{name}");
        }
        assert!(!is_importable(Path::new("notes.txt")));
    }

    #[test]
    fn open_bytes_tries_extra_in_order_and_otherwise_is_unsupported() {
        let bytes: Arc<[u8]> = Arc::from(&b"xxxx"[..]);
        let err = open_bytes("a.bin", bytes.clone(), &[]).err().expect("expected error");
        assert!(matches!(err, MediaError::Unsupported(_)));
        let err = open_bytes("a.bin", bytes.clone(), &[reject_all, accept_ok]).err().expect("expected error");
        assert!(matches!(err, MediaError::Unsupported(_)));
        let src = open_bytes("clip.ok", bytes.clone(), &[reject_all, accept_ok]).unwrap();
        assert_eq!(src.info().name, "clip.ok");
        let err = open_bytes("clip.ok", bytes, &[boom, accept_ok]).err().expect("expected error");
        assert!(matches!(err, MediaError::Decode(_)));
    }

    #[test]
    fn open_reader_uses_reader_openers_then_falls_back_within_max_whole() {
        let small: SharedBytes = Arc::new(b"HELLO".to_vec());
        let err = open_reader("a.bin", small.clone(), &[], &[]).err().expect("expected error");
        assert!(matches!(err, MediaError::Unsupported(_)));

        let ok: SharedBytes = Arc::new(b"OK-file".to_vec());
        let src = open_reader("clip.mov", ok, &[reader_ok], &[]).unwrap();
        assert_eq!(src.info().kind, MediaKind::Movie);

        let tagged: SharedBytes = Arc::new(b"xxxx".to_vec());
        let src = open_reader("clip.ok", tagged.clone(), &[], &[accept_ok]).unwrap();
        assert_eq!(src.info().name, "clip.ok");

        let big = vec![0u8; HEAD_LEN + 8];
        let r: SharedBytes = Arc::new(big);
        let e = open_reader_within("a.avi", r, &[], &[accept_ok], HEAD_LEN as u64).err().expect("expected error");
        assert!(matches!(e, MediaError::Unsupported(s) if s.contains("too large")));
    }

    #[test]
    fn audio_streams_serde_accepts_legacy_single_object() {
        let stream = r#"{"sample_rate":48000,"channels":2,"codec":"pcm","bits_per_sample":null}"#;
        let prefix = r#"{"name":"a","kind":"Movie","duration":0,"video":null,"audio_streams":["#;
        let suffix = r#"],"container":"wav","start_timecode":null,"file_size":null}"#;
        let full = std::iter::repeat_n(stream, MAX_AUDIO_STREAMS).collect::<Vec<_>>().join(",");
        let info: MediaInfo = serde_json::from_str(&format!("{prefix}{full}{suffix}")).unwrap();
        assert_eq!(info.audio_streams.len(), MAX_AUDIO_STREAMS);
        for extra in [stream, r#"{"malformed":"not an audio stream"}"#] {
            let error = serde_json::from_str::<MediaInfo>(&format!("{prefix}{full},{extra}{suffix}")).unwrap_err();
            assert!(error.to_string().contains("MAX_AUDIO_STREAMS"), "{error}");
        }
        let many = r#"{"name":"a","kind":"Movie","duration":0,"video":null,"audio_streams":[{"sample_rate":48000,"channels":2,"codec":"pcm","bits_per_sample":null}],"container":"wav","start_timecode":null,"file_size":null}"#;
        let info: MediaInfo = serde_json::from_str(many).unwrap();
        assert_eq!(info.audio_streams.len(), 1);
        let one = r#"{"name":"a","kind":"AudioOnly","duration":0,"video":null,"audio":{"sample_rate":44100,"channels":1,"codec":"pcm","bits_per_sample":16},"container":"wav","start_timecode":null,"file_size":null}"#;
        let info: MediaInfo = serde_json::from_str(one).unwrap();
        assert_eq!(info.audio().map(|a| a.sample_rate), Some(44100));
        let none = r#"{"name":"a","kind":"Still","duration":0,"video":null,"audio":null,"container":"png","start_timecode":null,"file_size":null}"#;
        let info: MediaInfo = serde_json::from_str(none).unwrap();
        assert!(info.audio_streams.is_empty());
    }

    #[test]
    fn offline_source_returns_slate_and_silence() {
        let src = silent("off", MediaKind::Synthetic);
        let frame = src.video_frame(FrameRequest::full(Tick::ZERO)).unwrap();
        assert_eq!((frame.width, frame.height), (16, 8));
        assert_eq!(frame.pts, Tick::ZERO);
        let audio = src.audio(0, 8, 48000).unwrap();
        assert_eq!(audio.sample_rate, 48000);
        assert_eq!(audio.num_channels(), 2);
        assert_eq!(audio.frames(), 8);
        assert!(src.audio_stream(1, 0, 8, 48000).is_err());
    }
}
