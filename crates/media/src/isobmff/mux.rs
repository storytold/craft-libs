//! Muxers: progressive MP4/MOV ([`Mp4Writer`], optional faststart) and fragmented MP4 ([`FragmentedWriter`]).

use crate::isobmff::bytes::BoxBuf;
use crate::isobmff::chapters::{Chapter, normalize_chapters, text_sample, text_sample_entry_payload, write_chpl};
use crate::isobmff::codec::{CodecConfig, SampleEntry, TimecodeConfig};
use crate::isobmff::demux::Edit;
use crate::isobmff::error::{Error, Result};
use std::io::{Read, Seek, SeekFrom, Write};

/// Output flavor: determines brands and some sample-entry encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brand {
    /// ISO MP4: `isom` major brand, compatible `isom iso2 avc1 mp41 mp42`.
    Mp4,
    /// QuickTime MOV: `qt  ` major brand.
    Mov,
}

/// Movie-level writer options.
#[derive(Clone, Debug)]
pub struct WriterOptions {
    /// File type brand written to `ftyp`.
    pub brand: Brand,
    /// Movie timescale (`mvhd`); default 1000.
    pub movie_timescale: u32,
    /// Text metadata written to `moov/udta` (keys like `©nam`, `©ART`, `©cmt`, `©too`).
    pub metadata: Vec<(String, String)>,
    /// Chapter marks (starts in `movie_timescale`). [`Mp4Writer`] writes them both as a QuickTime
    /// chapter track (`tref/chap` → disabled text track) and as a Nero `udta/chpl` list;
    /// [`FragmentedWriter`] writes only `chpl`. See [`crate::isobmff::chapters`] for the formats and limits.
    pub chapters: Vec<Chapter>,
}

impl WriterOptions {
    /// Options for `brand` with a 1000 ticks-per-second movie timescale and no metadata.
    pub fn new(brand: Brand) -> Self {
        WriterOptions { brand, movie_timescale: 1000, metadata: Vec::new(), chapters: Vec::new() }
    }
}

/// Per-track configuration.
#[derive(Clone, Debug)]
pub struct TrackConfig {
    /// Sample entry (codec configuration) for the track's `stsd` box.
    pub entry: SampleEntry,
    /// Media timescale. For PCM it must equal the sample rate (one tick per audio frame).
    pub timescale: u32,
    /// ISO 639-2/T language (`und` by default).
    pub language: String,
    /// Explicit edit list (segment durations in the movie timescale). Takes precedence over `media_start`.
    pub edits: Vec<Edit>,
    /// Convenience: create one edit starting at this media time (e.g. AAC encoder delay / priming,
    /// or the first pts of a B-frame stream) spanning the rest of the media.
    pub media_start: Option<i64>,
    /// `hdlr` name; a sensible default is used when `None`.
    pub handler_name: Option<String>,
}

impl TrackConfig {
    /// Configuration for `entry` and a media `timescale` in ticks per second, with language `und`, no edits and no handler name.
    pub fn new(entry: SampleEntry, timescale: u32) -> Self {
        TrackConfig { entry, timescale, language: "und".into(), edits: Vec::new(), media_start: None, handler_name: None }
    }
}

/// A sample handed to the muxers.
#[derive(Clone, Copy, Debug)]
pub struct WriteSample<'a> {
    /// Encoded sample payload, in the format the track's sample entry declares.
    pub data: &'a [u8],
    /// Duration in the media timescale. Ignored for PCM (each frame lasts one tick).
    pub duration: u32,
    /// pts − dts in the media timescale.
    pub composition_offset: i32,
    /// True for a sync sample (random access point, such as a key frame).
    pub is_sync: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Handler {
    Video,
    Audio,
    Timecode,
    /// QuickTime text track carrying chapter titles.
    Text,
    Other,
}

fn handler_of(e: &SampleEntry) -> Handler {
    if matches!(e.codec, CodecConfig::Timecode(_)) {
        Handler::Timecode
    } else if e.video.is_some() {
        Handler::Video
    } else if e.audio.is_some() {
        Handler::Audio
    } else {
        Handler::Other
    }
}

/// The `CodecConfig` variant, and the fourcc of variants that cover several codecs.
fn same_codec(a: &CodecConfig, b: &CodecConfig) -> bool {
    use CodecConfig::*;
    match (a, b) {
        (ProRes { fourcc: x }, ProRes { fourcc: y })
        | (Jpeg { fourcc: x }, Jpeg { fourcc: y })
        | (Dnx { fourcc: x }, Dnx { fourcc: y })
        | (Unknown { fourcc: x, .. }, Unknown { fourcc: y, .. }) => x == y,
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// Accumulated sample tables for one track.
#[derive(Clone, Debug, Default)]
struct Tables {
    stts: Vec<(u32, u32)>,
    ctts: Vec<(u32, i32)>,
    sizes: Vec<u32>,
    /// Constant sample size (PCM) — then `sizes` is unused.
    const_size: Option<u32>,
    sample_count: u64,
    /// 1-based sync sample numbers.
    sync: Vec<u32>,
    all_sync: bool,
    /// (offset relative to file start before shifting, sample count)
    chunks: Vec<(u64, u32)>,
    media_duration: u64,
    /// End of the last sample on the composition timeline (decode time + composition offset +
    /// duration, maximum over samples).
    comp_end: u64,
}

impl Tables {
    fn new() -> Self {
        Tables { all_sync: true, ..Default::default() }
    }
    fn push_run<T: PartialEq + Copy>(runs: &mut Vec<(u32, T)>, mut count: u64, v: T) {
        if let Some(last) = runs.last_mut()
            && last.1 == v
        {
            let n = count.min(u64::from(u32::MAX - last.0));
            last.0 += n as u32;
            count -= n;
        }
        while count > 0 {
            let n = count.min(u64::from(u32::MAX));
            runs.push((n as u32, v));
            count -= n;
        }
    }

    /// Records `count` samples written at `offset`, `sample_bytes` each, extending the last
    /// chunk when the write follows it. A chunk holds at most `max` samples.
    fn push_chunks(&mut self, mut offset: u64, mut count: u64, sample_bytes: u64, contiguous: bool, max: u32) {
        if contiguous && let Some(c) = self.chunks.last_mut() {
            let n = count.min(u64::from(max.saturating_sub(c.1)));
            c.1 += n as u32;
            count -= n;
            offset += n * sample_bytes;
        }
        while count > 0 {
            let n = count.min(u64::from(max));
            self.chunks.push((offset, n as u32));
            count -= n;
            offset += n * sample_bytes;
        }
    }
}

/// `stsz` counts samples (PCM: frames) in 32 bits.
fn too_many_samples() -> Error {
    Error::Mux("more than 2^32 - 1 samples in one track".into())
}

/// PCM chunk size limit, ffmpeg's: readers that take each PCM chunk as one sample (this crate's
/// demuxer among them) get samples of bounded size.
const PCM_CHUNK_BYTES: u64 = 1 << 20;

/// Frames per PCM chunk: at most [`PCM_CHUNK_BYTES`], at least one frame.
fn pcm_chunk_frames(frame_bytes: u32) -> u32 {
    (PCM_CHUNK_BYTES / u64::from(frame_bytes.max(1))).max(1) as u32
}

struct WTrack {
    cfg: TrackConfig,
    id: u32,
    handler: Handler,
    pcm_frame_bytes: Option<u32>,
    t: Tables,
    /// Track id of a linked timecode track (`tref/tmcd`).
    tmcd_ref: Option<u32>,
    /// For timecode tracks: (video track index, start frame).
    tmcd_source: Option<(usize, u32)>,
    /// Track id of the chapter text track (`tref/chap`).
    chap_ref: Option<u32>,
}

fn pack_language(lang: &str) -> u16 {
    let b = lang.as_bytes();
    if b.len() == 3 && b.iter().all(|c| c.is_ascii_lowercase()) {
        (((b[0] - 0x60) as u16) << 10) | (((b[1] - 0x60) as u16) << 5) | ((b[2] - 0x60) as u16)
    } else {
        0x55C4 // "und"
    }
}

fn rescale(v: u64, from: u32, to: u32) -> u64 {
    if from == 0 {
        return 0;
    }
    (v as u128 * to as u128 / from as u128).min(u64::MAX as u128) as u64
}

fn validate_track(entry: &SampleEntry, timescale: u32) -> Result<Option<u32>> {
    if timescale == 0 {
        return Err(Error::Mux("timescale must be non-zero".into()));
    }
    if let CodecConfig::Pcm(p) = &entry.codec {
        if p.bits == 0 || p.channels == 0 {
            return Err(Error::Mux("PCM needs bits and channels".into()));
        }
        if (p.sample_rate - timescale as f64).abs() > 0.5 {
            return Err(Error::Mux("PCM track timescale must equal the sample rate".into()));
        }
        return Ok(Some(p.bytes_per_frame()));
    }
    Ok(None)
}

mod boxes;
mod fragmented;
mod progressive;

pub use fragmented::FragmentedWriter;
pub use progressive::Mp4Writer;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isobmff::codec::AvcConfig;
    use std::io::Cursor;

    fn avc_entry() -> SampleEntry {
        SampleEntry::avc(AvcConfig::new(vec![vec![0x67, 0x42, 0, 0x1E]], vec![vec![0x68, 0xCE]], 4), 64, 48)
    }

    fn chaptered(brand: Brand, chapters: Vec<Chapter>, fast: bool) -> Vec<u8> {
        let mut opts = WriterOptions::new(brand);
        opts.chapters = chapters;
        let mut w = Mp4Writer::new(Cursor::new(Vec::new()), opts).unwrap();
        let v = w.add_track(TrackConfig::new(avc_entry(), 12800)).unwrap();
        // 100 frames of 512 / 12800 s = 4 s
        for i in 0..100u32 {
            w.write_sample(v, WriteSample { data: &[1, 2, 3], duration: 512, composition_offset: 0, is_sync: i % 25 == 0 }).unwrap();
        }
        if fast { w.finish_faststart() } else { w.finish() }.unwrap().into_inner()
    }

    fn ch(start: u64, title: &str) -> Chapter {
        Chapter { start, title: title.into() }
    }

    #[test]
    fn chapters_are_written_as_quicktime_track_and_chpl() {
        for brand in [Brand::Mp4, Brand::Mov] {
            for fast in [false, true] {
                let out = chaptered(brand, vec![ch(2500, "Zwei"), ch(0, "Eins"), ch(1000, "日本語")], fast);
                let f = crate::isobmff::open(out.as_slice()).unwrap();
                let want = vec![ch(0, "Eins"), ch(1000, "日本語"), ch(2500, "Zwei")];
                assert_eq!(f.chpl_chapters, want, "{brand:?} chpl");
                assert_eq!(f.read_chapters(out.as_slice()).unwrap(), want, "{brand:?} chapter track");
                // the text track: disabled, referenced by the video track's tref/chap, lasting the movie
                assert_eq!(f.tracks.len(), 2);
                let t = &f.tracks[1];
                assert_eq!(t.handler, b"text");
                assert!(!t.enabled);
                assert_eq!(f.tracks[0].references, vec![(crate::isobmff::FourCc(*b"chap"), vec![t.id])]);
                let durs: Vec<u32> = t.samples.iter().map(|s| s.duration).collect();
                assert_eq!(durs, vec![1000, 1500, 1500]);
                assert_eq!(f.duration, 4000);
                // the video still reads back
                assert_eq!(f.tracks[0].samples.len(), 100);
                assert_eq!(f.read_sample(out.as_slice(), 0, 99).unwrap(), vec![1, 2, 3]);
            }
        }
    }

    /// The media time (chapter track timescale) of `movie` ticks.
    fn chapter_media_time(f: &crate::isobmff::Mp4File, movie: u64) -> i64 {
        (u128::from(movie) * u128::from(f.tracks[1].timescale) / u128::from(f.timescale)) as i64
    }

    #[test]
    fn chapters_are_clipped_to_the_chapter_tracks_edit() {
        let out = chaptered(Brand::Mp4, vec![ch(0, "A"), ch(1000, "B"), ch(2000, "C"), ch(3000, "D")], false);
        let mut f = crate::isobmff::open(out.as_slice()).unwrap();
        // the edit starts inside B and ends inside C: A and D are outside, B begins at zero
        let from = chapter_media_time(&f, 1500);
        f.tracks[1].edits = vec![crate::isobmff::Edit { segment_duration: 1000, media_time: from, media_rate: 0x0001_0000 }];
        f.tracks[1].edit_offset = -from;
        assert_eq!(f.read_chapters(out.as_slice()).unwrap(), vec![ch(0, "B"), ch(500, "C")]);
    }

    #[test]
    fn a_hostile_chapter_sample_is_cut_and_read_only_in_part() {
        let mut out = chaptered(Brand::Mp4, vec![ch(0, "A"), ch(1000, "B")], false);
        let mut f = crate::isobmff::open(out.as_slice()).unwrap();
        let offset = out.len() as u64;
        out.extend_from_slice(&3000u16.to_be_bytes());
        out.extend_from_slice(&[b'x'; 3000]);
        // a long title is cut, and a declared size past the end of the file reads only what is needed
        f.tracks[1].samples[0].offset = offset;
        f.tracks[1].samples[0].size = 3002;
        f.tracks[1].samples[1].offset = offset;
        f.tracks[1].samples[1].size = u32::MAX;
        out.extend_from_slice(&[0; 4098]);
        let v = f.read_chapters(out.as_slice()).unwrap();
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|c| c.title.len() == crate::isobmff::MAX_CHAPTER_TITLE_BYTES), "{:?}", v[0].title.len());
    }

    #[test]
    fn hostile_chapters_do_not_break_the_file() {
        let long = "x".repeat(10_000);
        let chapters = vec![ch(0, ""), ch(0, "dup"), ch(4000, "at the end"), ch(u64::MAX, "far"), ch(10, &long), ch(20, "a\0b\nc")];
        let out = chaptered(Brand::Mov, chapters, true);
        let f = crate::isobmff::open(out.as_slice()).unwrap();
        let got = f.read_chapters(out.as_slice()).unwrap();
        assert_eq!(got.iter().map(|c| c.start).collect::<Vec<_>>(), vec![0, 10, 20]);
        assert_eq!(got[0].title, "");
        assert_eq!(got[1].title.len(), crate::isobmff::MAX_CHAPTER_TITLE_BYTES);
        assert_eq!(got[2].title, "a b c");
        assert_eq!(f.chpl_chapters, got);
        // all chapters past the end: no chapter track, no chpl
        let out = chaptered(Brand::Mp4, vec![ch(9000, "gone")], false);
        let f = crate::isobmff::open(out.as_slice()).unwrap();
        assert_eq!(f.tracks.len(), 1);
        assert!(f.chpl_chapters.is_empty() && f.read_chapters(out.as_slice()).unwrap().is_empty());
        // over the cap
        let many: Vec<Chapter> = (0..1000).map(|i| ch(i * 3, "c")).collect();
        let out = chaptered(Brand::Mp4, many, false);
        let f = crate::isobmff::open(out.as_slice()).unwrap();
        assert_eq!(f.read_chapters(out.as_slice()).unwrap().len(), crate::isobmff::MAX_CHAPTERS);
        assert_eq!(f.chpl_chapters.len(), crate::isobmff::MAX_CHAPTERS);
    }
}
