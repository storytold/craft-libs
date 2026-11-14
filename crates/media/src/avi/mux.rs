//! Clean-room AVI muxer, written from Microsoft's published AVI RIFF file reference and the
//! OpenDML AVI File Format Extensions 1.02.
//!
//! [`AviWriter`] writes one `RIFF AVI ` with the `hdrl` header list, a `movi` list of chunks in
//! the order they are given, and the AVI 1.0 `idx1` index. A file that grows past
//! [`MuxOptions::riff_limit`] (1 GiB) continues in `RIFF AVIX` extensions. Every RIFF ends its
//! `movi` list with an OpenDML standard index (`ix##`) per stream, and each stream's `strl`
//! holds a super index (`indx`) of them, so readers find the chunks past the first RIFF; `idx1`
//! covers the first RIFF only, as OpenDML asks.
//!
//! The header list is written at the start with room reserved for the super indexes and
//! rewritten with the final counts by [`AviWriter::finish`]: the output must be `Write + Seek`.

use std::io::{Seek, SeekFrom, Write};

use super::{BitmapInfo, Error, Result, WaveFormat};
use crate::{CodecId, CodecParams};

/// `AVIF_HASINDEX`: the file has an `idx1`.
const AVIF_HASINDEX: u32 = 0x10;
/// `AVIF_ISINTERLEAVED`: the chunks of the streams are interleaved.
const AVIF_ISINTERLEAVED: u32 = 0x100;
/// `AVIIF_KEYFRAME` in an `idx1` entry.
const AVIIF_KEYFRAME: u32 = 0x10;
/// Bit 31 of a standard index entry's size: not a key frame.
const NOT_KEY: u32 = 0x8000_0000;
/// `dmlh` is 248 bytes: `dwTotalFrames` and reserved space.
const DMLH_SIZE: usize = 248;
/// Chunk ids carry a two-digit decimal stream number.
const MAX_STREAMS: usize = 100;
/// The smallest RIFF size the writer accepts.
const MIN_RIFF_LIMIT: u64 = 16 << 10;
/// The largest: RIFF and chunk sizes are 32-bit, and OpenDML entries use bit 31 as a flag.
const MAX_RIFF_LIMIT: u64 = 1 << 31;

/// How the chunks of an audio stream measure time (`strh` `dwScale`, `dwRate`, `dwSampleSize`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioTiming {
    /// Chunks are runs of fixed-size sample frames of `block_align` bytes (PCM, A-law, constant
    /// bitrate MP3 written as a byte stream): the stream is timed by its bytes.
    Bytes,
    /// Each chunk is one packet of `samples_per_packet` sample frames (1152 for MPEG audio Layer
    /// II/III at 32 kHz and above, 1024 for AAC, 1536 for AC-3).
    Packets {
        /// Sample frames in a packet.
        samples_per_packet: u32,
    },
}

/// A video stream for [`AviWriter`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoSpec {
    /// The `BITMAPINFOHEADER` written as `strf`. `compression` is the FourCC (`XVID`, `MJPG`,
    /// `H264`…), and `extra` follows the 40-byte header.
    pub format: BitmapInfo,
    /// `fccHandler`; the compression FourCC when `None`.
    pub handler: Option<[u8; 4]>,
    /// Frames per second is `rate / scale`.
    pub rate: u32,
    /// See `rate`.
    pub scale: u32,
    /// `strn`.
    pub name: Option<String>,
}

impl VideoSpec {
    /// A compressed stream of `fourcc` pictures, 24 bits per pixel in the header, at `rate / scale`
    /// frames per second.
    pub fn new(fourcc: [u8; 4], width: u32, height: u32, rate: u32, scale: u32) -> VideoSpec {
        VideoSpec {
            format: BitmapInfo {
                width: i32::try_from(width).unwrap_or(i32::MAX),
                height: i32::try_from(height).unwrap_or(i32::MAX),
                bit_count: 24,
                compression: fourcc,
                size_image: width.saturating_mul(height).saturating_mul(3),
                extra: Vec::new(),
            },
            handler: None,
            rate,
            scale,
            name: None,
        }
    }

    /// A stream for an encoder's track description: the usual AVI FourCC of its codec (`FMP4` for
    /// MPEG-4 Part 2, `H264`, `HEVC`, `MJPG`, `MPG2`, `VP80`, `VP90`, `AV01`), its picture size
    /// and its extradata after the header. MPEG-4 Part 2 gets `FMP4` rather than `XVID` or `DIVX`
    /// because decoders read those as naming the encoder: ffmpeg, for one, decodes an `XVID`
    /// stream with Xvid's own IDCT. Set `format.compression` for a player that wants another. `None` for a codec without a FourCC here or a
    /// description without a size. H.264 and HEVC chunks must then be Annex B with an Annex B or
    /// empty extradata, as AVI readers expect.
    pub fn for_codec(params: &CodecParams, rate: u32, scale: u32) -> Option<VideoSpec> {
        let fourcc = match params.codec {
            CodecId::Mpeg4Video => *b"FMP4",
            CodecId::H264 => *b"H264",
            CodecId::Hevc => *b"HEVC",
            CodecId::Mjpeg => *b"MJPG",
            CodecId::Mpeg2Video => *b"MPG2",
            CodecId::Vp8 => *b"VP80",
            CodecId::Vp9 => *b"VP90",
            CodecId::Av1 => *b"AV01",
            _ => return None,
        };
        let mut v = VideoSpec::new(fourcc, params.width?, params.height?, rate, scale);
        v.format.extra = params.extradata.clone();
        Some(v)
    }
}

/// An audio stream for [`AviWriter`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSpec {
    /// The `WAVEFORMATEX` written as `strf`.
    pub format: WaveFormat,
    /// How its chunks measure time.
    pub timing: AudioTiming,
    /// `strn`.
    pub name: Option<String>,
}

impl AudioSpec {
    /// Interleaved little-endian integer PCM (`bits` 8, 16, 24 or 32), timed by its bytes.
    pub fn pcm(sample_rate: u32, channels: u16, bits: u16) -> AudioSpec {
        let block_align = channels.saturating_mul(bits.div_ceil(8));
        AudioSpec {
            format: WaveFormat {
                format_tag: 1,
                channels,
                sample_rate,
                avg_bytes_per_sec: sample_rate.saturating_mul(u32::from(block_align)),
                block_align,
                bits_per_sample: bits,
                extra: Vec::new(),
            },
            timing: AudioTiming::Bytes,
            name: None,
        }
    }
}

/// One stream of an AVI file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamSpec {
    /// `vids`.
    Video(VideoSpec),
    /// `auds`.
    Audio(AudioSpec),
}

/// Options for [`AviWriter`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MuxOptions {
    /// Largest size of one RIFF (the first, then each `AVIX` extension), 16 KiB to 2 GiB.
    /// 1 GiB by default, which OpenDML recommends for the first RIFF so AVI 1.0 readers can use it.
    pub riff_limit: u64,
    /// RIFFs the super indexes have room for (each takes 16 bytes a stream in the header). 256 by
    /// default: 256 GiB at the default `riff_limit`.
    pub max_riffs: u32,
}

impl Default for MuxOptions {
    fn default() -> Self {
        MuxOptions { riff_limit: 1 << 30, max_riffs: 256 }
    }
}

/// A chunk's entry in the standard index of its RIFF.
#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Offset of the data from the `movi` list's type id (the index's base).
    offset: u32,
    size: u32,
    key: bool,
}

/// One `ix##` written: its position, its size and the stream time it covers.
#[derive(Clone, Copy, Debug)]
struct SuperEntry {
    at: u64,
    size: u32,
    duration: u32,
}

/// The RIFF being written.
#[derive(Clone, Copy, Debug)]
struct Riff {
    /// Position of `RIFF`.
    start: u64,
    /// Position of the `movi` list's `LIST`.
    movi: u64,
}

#[derive(Debug)]
struct StreamState {
    spec: StreamSpec,
    /// `00dc`, `01wb`…
    id: [u8; 4],
    chunks: u64,
    bytes: u64,
    largest: u32,
    /// The chunks of the current RIFF.
    entries: Vec<Entry>,
    /// Stream time of the current RIFF's chunks.
    duration: u64,
    supers: Vec<SuperEntry>,
}

impl StreamState {
    /// Stream time (`dwScale / dwRate` units) of a chunk of `len` bytes.
    fn units(&self, len: u64) -> u64 {
        match &self.spec {
            StreamSpec::Audio(a) if a.timing == AudioTiming::Bytes => len / u64::from(a.format.block_align.max(1)),
            _ => 1,
        }
    }

    /// `(dwScale, dwRate, dwSampleSize)`.
    fn time_base(&self) -> (u32, u32, u32) {
        match &self.spec {
            StreamSpec::Video(v) => (v.scale, v.rate, 0),
            StreamSpec::Audio(a) => match a.timing {
                AudioTiming::Bytes => (u32::from(a.format.block_align), a.format.avg_bytes_per_sec, u32::from(a.format.block_align)),
                AudioTiming::Packets { samples_per_packet } => (samples_per_packet, a.format.sample_rate, 0),
            },
        }
    }

    /// The stream's length in its own units.
    fn length(&self) -> u64 {
        match &self.spec {
            StreamSpec::Audio(a) if a.timing == AudioTiming::Bytes => self.bytes / u64::from(a.format.block_align.max(1)),
            _ => self.chunks,
        }
    }

    /// The stream's duration in seconds.
    fn seconds(&self) -> f64 {
        let (scale, rate, _) = self.time_base();
        self.length() as f64 * f64::from(scale) / f64::from(rate.max(1))
    }
}

fn le16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn le32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn sat32(x: u64) -> u32 {
    u32::try_from(x).unwrap_or(u32::MAX)
}

/// A chunk: id, size, data, a pad byte to an even size.
fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(data.len() + 9);
    v.extend_from_slice(id);
    le32(&mut v, sat32(data.len() as u64));
    v.extend_from_slice(data);
    if data.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn list(ltype: &[u8; 4], body: &[u8]) -> Vec<u8> {
    chunk(b"LIST", &[ltype.as_slice(), body].concat())
}

fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

/// The `hdrl` list. Its length depends only on the stream descriptions and `max_riffs`, so the
/// final one ([`AviWriter::finish`]) replaces the first in place.
fn hdrl(streams: &[StreamState], main: Option<usize>, max_riffs: u32, first_frames: u32) -> Vec<u8> {
    let video = main.and_then(|i| streams.get(i)).and_then(|s| match &s.spec {
        StreamSpec::Video(v) => Some((v, s)),
        StreamSpec::Audio(_) => None,
    });
    let seconds = streams.iter().map(StreamState::seconds).fold(0.0, f64::max);
    let total: u64 = streams.iter().map(|s| s.bytes).sum();
    let largest = streams.iter().map(|s| s.largest).max().unwrap_or(0);

    let mut avih = Vec::with_capacity(56);
    le32(&mut avih, video.map_or(0, |(v, _)| (1e6 * f64::from(v.scale) / f64::from(v.rate)).round() as u32));
    le32(&mut avih, if seconds > 0.0 { (total as f64 / seconds).ceil() as u32 } else { 0 });
    le32(&mut avih, 0); // dwPaddingGranularity
    le32(&mut avih, AVIF_HASINDEX | AVIF_ISINTERLEAVED);
    le32(&mut avih, first_frames);
    le32(&mut avih, 0); // dwInitialFrames
    le32(&mut avih, streams.len() as u32);
    le32(&mut avih, largest.saturating_add(8));
    le32(&mut avih, video.map_or(0, |(v, _)| v.format.width.unsigned_abs()));
    le32(&mut avih, video.map_or(0, |(v, _)| v.format.height.unsigned_abs()));
    avih.extend_from_slice(&[0; 16]);

    let mut body = chunk(b"avih", &avih);
    for s in streams {
        let (scale, rate, sample_size) = s.time_base();
        let mut strh = Vec::with_capacity(56);
        let (fcc, handler, frame) = match &s.spec {
            StreamSpec::Video(v) => (*b"vids", v.handler.unwrap_or(v.format.compression), (v.format.width, v.format.height.abs())),
            StreamSpec::Audio(_) => (*b"auds", [0; 4], (0, 0)),
        };
        strh.extend_from_slice(&fcc);
        strh.extend_from_slice(&handler);
        le32(&mut strh, 0); // dwFlags
        le16(&mut strh, 0); // wPriority
        le16(&mut strh, 0); // wLanguage
        le32(&mut strh, 0); // dwInitialFrames
        le32(&mut strh, scale);
        le32(&mut strh, rate);
        le32(&mut strh, 0); // dwStart
        le32(&mut strh, sat32(s.length()));
        le32(&mut strh, s.largest);
        le32(&mut strh, u32::MAX); // dwQuality: default
        le32(&mut strh, sample_size);
        for v in [0, 0, frame.0, frame.1] {
            le16(&mut strh, i16::try_from(v).unwrap_or(i16::MAX) as u16);
        }

        let mut strf = Vec::new();
        match &s.spec {
            StreamSpec::Video(v) => {
                let f = &v.format;
                le32(&mut strf, sat32(40 + f.extra.len() as u64));
                le32(&mut strf, f.width as u32);
                le32(&mut strf, f.height as u32);
                le16(&mut strf, 1); // biPlanes
                le16(&mut strf, f.bit_count);
                strf.extend_from_slice(&f.compression);
                le32(&mut strf, f.size_image);
                strf.extend_from_slice(&[0; 16]); // pels per metre, colours used and important
                strf.extend_from_slice(&f.extra);
            }
            StreamSpec::Audio(a) => {
                let f = &a.format;
                le16(&mut strf, f.format_tag);
                le16(&mut strf, f.channels);
                le32(&mut strf, f.sample_rate);
                le32(&mut strf, f.avg_bytes_per_sec);
                le16(&mut strf, f.block_align);
                le16(&mut strf, f.bits_per_sample);
                le16(&mut strf, u16::try_from(f.extra.len()).unwrap_or(u16::MAX));
                strf.extend_from_slice(&f.extra[..f.extra.len().min(usize::from(u16::MAX))]);
            }
        }

        // indx: AVI_INDEX_OF_INDEXES, room for `max_riffs` entries
        let mut indx = Vec::with_capacity(24 + 16 * max_riffs as usize);
        le16(&mut indx, 4); // wLongsPerEntry
        indx.push(0); // bIndexSubType
        indx.push(0); // bIndexType: AVI_INDEX_OF_INDEXES
        le32(&mut indx, s.supers.len() as u32);
        indx.extend_from_slice(&s.id);
        indx.extend_from_slice(&[0; 12]);
        for e in &s.supers {
            indx.extend_from_slice(&e.at.to_le_bytes());
            le32(&mut indx, e.size);
            le32(&mut indx, e.duration);
        }
        indx.resize(24 + 16 * max_riffs as usize, 0);

        let mut strl = chunk(b"strh", &strh);
        strl.extend(chunk(b"strf", &strf));
        let name = match &s.spec {
            StreamSpec::Video(v) => v.name.as_deref(),
            StreamSpec::Audio(a) => a.name.as_deref(),
        };
        if let Some(n) = name {
            strl.extend(chunk(b"strn", &[n.as_bytes(), &[0]].concat()));
        }
        strl.extend(chunk(b"indx", &indx));
        body.extend(list(b"strl", &strl));
    }
    let mut dmlh = vec![0u8; DMLH_SIZE];
    let frames = video.map_or(0, |(_, s)| sat32(s.chunks));
    dmlh[..4].copy_from_slice(&frames.to_le_bytes());
    body.extend(list(b"odml", &chunk(b"dmlh", &dmlh)));
    list(b"hdrl", &body)
}

/// Writes an AVI file chunk by chunk; see the [module documentation](self).
#[derive(Debug)]
pub struct AviWriter<W: Write + Seek> {
    w: W,
    /// Where the file starts in `w`.
    base: u64,
    opts: MuxOptions,
    streams: Vec<StreamState>,
    /// The stream whose frame period and size the main header gives (the first video stream).
    main: Option<usize>,
    /// The next byte to write, from the start of the file.
    pos: u64,
    /// Bytes of the `hdrl` list, fixed so it can be rewritten in place.
    hdrl_len: usize,
    riff: Riff,
    /// RIFFs started so far.
    riffs: u32,
    /// `idx1` of the first RIFF (16 bytes an entry), while it is open.
    idx1: Vec<[u8; 16]>,
    /// Video frames in the first RIFF (`avih` `dwTotalFrames`).
    first_frames: u32,
}

impl<W: Write + Seek> AviWriter<W> {
    /// Start a file with `streams` (1 to 100) at the current position of `w`.
    pub fn new(mut w: W, streams: Vec<StreamSpec>, opts: MuxOptions) -> Result<Self> {
        if streams.is_empty() || streams.len() > MAX_STREAMS {
            return Err(invalid(format!("{} streams (1 to {MAX_STREAMS})", streams.len())));
        }
        if !(MIN_RIFF_LIMIT..=MAX_RIFF_LIMIT).contains(&opts.riff_limit) {
            return Err(invalid(format!("riff_limit {} is outside {MIN_RIFF_LIMIT}..={MAX_RIFF_LIMIT}", opts.riff_limit)));
        }
        if opts.max_riffs == 0 || opts.max_riffs > 1 << 16 {
            return Err(invalid("max_riffs must be 1 to 65536"));
        }
        for (i, s) in streams.iter().enumerate() {
            match s {
                StreamSpec::Video(v) if v.rate == 0 || v.scale == 0 => return Err(invalid(format!("stream {i}: zero frame rate"))),
                StreamSpec::Audio(a) if a.format.sample_rate == 0 => return Err(invalid(format!("stream {i}: zero sample rate"))),
                StreamSpec::Audio(a) if a.timing == AudioTiming::Bytes && (a.format.block_align == 0 || a.format.avg_bytes_per_sec == 0) => {
                    return Err(invalid(format!("stream {i}: byte-timed audio needs a block align and a byte rate")));
                }
                StreamSpec::Audio(AudioSpec { timing: AudioTiming::Packets { samples_per_packet: 0 }, .. }) => {
                    return Err(invalid(format!("stream {i}: zero samples a packet")));
                }
                _ => {}
            }
        }
        let main = streams.iter().position(|s| matches!(s, StreamSpec::Video(_)));
        let streams: Vec<StreamState> = streams
            .into_iter()
            .enumerate()
            .map(|(i, spec)| {
                let kind = if matches!(spec, StreamSpec::Video(_)) { b"dc" } else { b"wb" };
                StreamState {
                    spec,
                    id: [b'0' + (i / 10) as u8, b'0' + (i % 10) as u8, kind[0], kind[1]],
                    chunks: 0,
                    bytes: 0,
                    largest: 0,
                    entries: Vec::new(),
                    duration: 0,
                    supers: Vec::new(),
                }
            })
            .collect();
        let header = hdrl(&streams, main, opts.max_riffs, 0);
        if (header.len() + 24) as u64 > opts.riff_limit / 2 {
            return Err(invalid("the header list takes more than half of riff_limit"));
        }
        let base = w.stream_position()?;
        let mut head = Vec::with_capacity(header.len() + 24);
        head.extend_from_slice(b"RIFF");
        le32(&mut head, 0);
        head.extend_from_slice(b"AVI ");
        head.extend_from_slice(&header);
        let movi = head.len() as u64;
        head.extend_from_slice(b"LIST");
        le32(&mut head, 0);
        head.extend_from_slice(b"movi");
        w.write_all(&head)?;
        Ok(AviWriter {
            w,
            base,
            opts,
            streams,
            main,
            pos: head.len() as u64,
            hdrl_len: header.len(),
            riff: Riff { start: 0, movi },
            riffs: 1,
            idx1: Vec::new(),
            first_frames: 0,
        })
    }

    /// Bytes the current RIFF would take if it were closed after one more chunk of `chunk_len`
    /// bytes for `stream`: its chunks, the standard indexes and (first RIFF) `idx1`.
    fn riff_size_with(&self, stream: usize, chunk_len: u64) -> u64 {
        let ix: u64 = self.streams.iter().enumerate().map(|(i, s)| 32 + 8 * (s.entries.len() as u64 + u64::from(i == stream))).sum();
        let idx1 = if self.riffs == 1 { 8 + 16 * (self.idx1.len() as u64 + 1) } else { 0 };
        self.pos - self.riff.start + chunk_len + ix + idx1
    }

    /// Write one chunk of stream `stream`: a video frame (`key` for a key frame; an empty chunk
    /// repeats the previous picture) or a run of audio. Chunks go into the file in call order,
    /// so the caller interleaves the streams.
    pub fn write_chunk(&mut self, stream: usize, data: &[u8], key: bool) -> Result<()> {
        if stream >= self.streams.len() {
            return Err(invalid(format!("no stream {stream}")));
        }
        let len = data.len() as u64;
        let chunk_len = 8 + len + (len & 1);
        if self.riff_size_with(stream, chunk_len) > self.opts.riff_limit && self.streams.iter().any(|s| !s.entries.is_empty()) {
            self.next_riff()?;
        }
        if self.riff_size_with(stream, chunk_len) > self.opts.riff_limit {
            return Err(invalid(format!("a {len}-byte chunk does not fit in a RIFF of riff_limit {} bytes", self.opts.riff_limit)));
        }
        let index_base = self.riff.movi + 8;
        let s = &mut self.streams[stream];
        let at = self.pos;
        let mut head = Vec::with_capacity(8);
        head.extend_from_slice(&s.id);
        le32(&mut head, len as u32);
        self.w.write_all(&head)?;
        self.w.write_all(data)?;
        if len & 1 == 1 {
            self.w.write_all(&[0])?;
        }
        self.pos += chunk_len;
        // riff_limit ≤ 2 GiB keeps every offset in the RIFF below 2^31
        s.entries.push(Entry { offset: (at + 8 - index_base) as u32, size: len as u32, key });
        s.duration += s.units(len);
        s.chunks += 1;
        s.bytes += len;
        s.largest = s.largest.max(len as u32);
        if self.riffs == 1 {
            let mut e = [0u8; 16];
            e[..4].copy_from_slice(&s.id);
            e[4..8].copy_from_slice(&(if key { AVIIF_KEYFRAME } else { 0 }).to_le_bytes());
            e[8..12].copy_from_slice(&((at - index_base) as u32).to_le_bytes());
            e[12..].copy_from_slice(&(len as u32).to_le_bytes());
            self.idx1.push(e);
            if Some(stream) == self.main {
                self.first_frames += 1;
            }
        }
        Ok(())
    }

    fn patch(&mut self, at: u64, bytes: &[u8]) -> Result<()> {
        self.w.seek(SeekFrom::Start(self.base + at))?;
        self.w.write_all(bytes)?;
        self.w.seek(SeekFrom::Start(self.base + self.pos))?;
        Ok(())
    }

    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.w.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    /// Close the current RIFF: the standard indexes at the end of its `movi`, the sizes, and
    /// (first RIFF) `idx1`.
    fn end_riff(&mut self) -> Result<()> {
        let index_base = self.riff.movi + 8;
        for i in 0..self.streams.len() {
            let s = &mut self.streams[i];
            let mut ix = Vec::with_capacity(24 + 8 * s.entries.len());
            le16(&mut ix, 2); // wLongsPerEntry
            ix.push(0); // bIndexSubType
            ix.push(1); // bIndexType: AVI_INDEX_OF_CHUNKS
            le32(&mut ix, s.entries.len() as u32);
            ix.extend_from_slice(&s.id);
            ix.extend_from_slice(&index_base.to_le_bytes());
            le32(&mut ix, 0);
            for e in s.entries.drain(..) {
                le32(&mut ix, e.offset);
                le32(&mut ix, e.size | if e.key { 0 } else { NOT_KEY });
            }
            let at = self.pos;
            let id = [b'i', b'x', s.id[0], s.id[1]];
            let c = chunk(&id, &ix);
            s.supers.push(SuperEntry { at, size: c.len() as u32, duration: sat32(s.duration) });
            s.duration = 0;
            self.put(&c)?;
        }
        let movi_size = sat32(self.pos - self.riff.movi - 8);
        self.patch(self.riff.movi + 4, &movi_size.to_le_bytes())?;
        if self.riffs == 1 {
            let idx1 = std::mem::take(&mut self.idx1).concat();
            self.put(&chunk(b"idx1", &idx1))?;
        }
        let riff_size = sat32(self.pos - self.riff.start - 8);
        self.patch(self.riff.start + 4, &riff_size.to_le_bytes())
    }

    /// Close the current RIFF and start a `RIFF AVIX` extension.
    fn next_riff(&mut self) -> Result<()> {
        if self.riffs >= self.opts.max_riffs {
            return Err(invalid(format!("the file needs more than max_riffs ({}) RIFFs", self.opts.max_riffs)));
        }
        self.end_riff()?;
        let start = self.pos;
        let mut head = Vec::with_capacity(24);
        head.extend_from_slice(b"RIFF");
        le32(&mut head, 0);
        head.extend_from_slice(b"AVIX");
        head.extend_from_slice(b"LIST");
        le32(&mut head, 0);
        head.extend_from_slice(b"movi");
        self.put(&head)?;
        self.riff = Riff { start, movi: start + 12 };
        self.riffs += 1;
        Ok(())
    }

    /// Close the file: the last RIFF's indexes and sizes, then the header list with the final
    /// counts. Returns the writer, positioned at the end of the file.
    pub fn finish(mut self) -> Result<W> {
        self.end_riff()?;
        let header = hdrl(&self.streams, self.main, self.opts.max_riffs, self.first_frames);
        if header.len() != self.hdrl_len {
            return Err(invalid("the header list changed size"));
        }
        self.patch(12, &header)?;
        self.w.flush()?;
        Ok(self.w)
    }
}
