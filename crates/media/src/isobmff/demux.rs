//! Demuxer: parses box structure lazily (headers + `moov`/`moof` only) and builds per-track sample tables.

use crate::isobmff::bytes::{BoxHeader, Cur, FourCc, boxes, find};
use crate::isobmff::codec::{CodecConfig, SampleEntry, parse_sample_entry};
use crate::isobmff::error::{Error, Result, invalid};
use crate::source::ByteSource;
use std::collections::HashMap;

/// Hard cap on the sample count of the whole file, all tracks and fragments (after PCM chunk merging).
const MAX_SAMPLES: usize = 1 << 26;
/// Bytes of file each sample needs beyond the first `MIN_SAMPLE_ALLOWANCE` samples. A [`Sample`]
/// takes 40 bytes of memory and a constant-size `stsz` or a `trun` without per-sample fields
/// describes one in a byte or none, so without this a file could make the sample tables about
/// 40 times its own size. A table-backed sample has a 4-byte `stsz` or `trun` entry.
const FILE_BYTES_PER_SAMPLE: u64 = 4;
/// Samples any file may hold, however short: 40 MiB of sample tables.
const MIN_SAMPLE_ALLOWANCE: usize = 1 << 20;
/// Hard cap on the size of an in-memory `moov`/`moof` box.
const MAX_HEADER_BOX: u64 = 1 << 30;
/// Most text all metadata entries may hold together; entries past it are dropped. One `keys`
/// name can be named by many `ilst` items, each a copy, so the file's size does not bound this.
const MAX_METADATA_BYTES: usize = 4 << 20;
/// What an entry costs beyond its text, so that empty entries are bounded too.
const METADATA_ENTRY_COST: usize = 64;

/// Largest sample built from a PCM chunk; larger chunks are split. Writers that put a whole
/// recording in one chunk would otherwise give samples past `u32::MAX` bytes.
const PCM_MAX_SAMPLE_BYTES: u64 = 1 << 24;

/// Frames in one piece of a split PCM chunk: whole units, at most `PCM_MAX_SAMPLE_BYTES`.
fn pcm_piece_frames(unit: u64, unit_bytes: u64) -> u64 {
    (PCM_MAX_SAMPLE_BYTES / unit_bytes.max(1)).max(1) * unit
}

/// What is left of the file-wide limits while tracks and fragments are parsed.
struct SampleBudget {
    /// Samples (what the file's length allows, at most `MAX_SAMPLES`, minus those already built).
    samples: usize,
    /// Bytes of file for samples whose count no table entry backs (constant-size `stsz`, `trun`
    /// without per-sample fields). Each such sample claims at least one byte, so a file can't
    /// describe more of them than it has bytes, however many tracks or fragments repeat the claim.
    implied_bytes: u64,
}

impl SampleBudget {
    fn new(file_len: u64) -> Self {
        let by_len = usize::try_from(file_len / FILE_BYTES_PER_SAMPLE).unwrap_or(usize::MAX);
        Self { samples: by_len.clamp(MIN_SAMPLE_ALLOWANCE, MAX_SAMPLES), implied_bytes: file_len }
    }

    fn take(&mut self, n: usize) -> Result<()> {
        self.samples = self.samples.checked_sub(n).ok_or(Error::TooLarge("sample count"))?;
        Ok(())
    }
}

/// One sample (access unit) of a track, in the track's media timescale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sample {
    /// Absolute byte offset in the file.
    pub offset: u64,
    /// Sample size in bytes.
    pub size: u32,
    /// Decode timestamp (raw media time, edit lists not applied).
    pub dts: i64,
    /// Presentation timestamp = dts + composition offset (raw media time).
    pub pts: i64,
    /// Duration in the media timescale.
    pub duration: u32,
    #[allow(missing_docs)]
    pub is_sync: bool,
    /// 1-based index into [`Track::entries`].
    pub description_index: u32,
}

/// One edit list entry (`elst`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Duration of this edit in the movie timescale.
    pub segment_duration: u64,
    /// Start time within the media (track timescale); `-1` marks an empty edit.
    pub media_time: i64,
    /// Media rate as 16.16 fixed point (0x0001_0000 = normal speed, 0 = dwell).
    pub media_rate: i32,
}

/// Broad classification of a track by its handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    #[allow(missing_docs)]
    Video,
    #[allow(missing_docs)]
    Audio,
    #[allow(missing_docs)]
    Timecode,
    #[allow(missing_docs)]
    Subtitle,
    /// Any other handler type.
    Other,
}

/// A sample-to-group mapping (`sbgp`) with the matching descriptions (`sgpd`) kept raw.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SampleGroup {
    /// Four-character grouping type from `sbgp`/`sgpd` (for example `roll`).
    pub grouping_type: FourCc,
    /// (sample_count, group_description_index) runs.
    pub runs: Vec<(u32, u32)>,
    /// Raw description entries from `sgpd`.
    pub descriptions: Vec<Vec<u8>>,
}

/// A demuxed track.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// Track ID from `tkhd`, unique within the file.
    pub id: u32,
    #[allow(missing_docs)]
    pub kind: TrackKind,
    /// Handler type from `hdlr` (for example `vide`, `soun`).
    pub handler: FourCc,
    /// Handler name string from `hdlr`.
    pub handler_name: String,
    /// Media timescale (`mdhd`).
    pub timescale: u32,
    /// Media duration in the media timescale (`mdhd`).
    pub duration: u64,
    /// Track duration in the movie timescale (`tkhd`).
    pub tkhd_duration: u64,
    /// ISO 639-2/T language code (e.g. `und`, `eng`).
    pub language: String,
    /// `tkhd` presentation size (16.16 fixed point converted to integer pixels, rounded down).
    pub width: u32,
    /// `tkhd` presentation height in integer pixels (same conversion as `width`).
    pub height: u32,
    /// `tkhd` enabled flag.
    pub enabled: bool,
    /// `tkhd` alternate group; tracks that share a non-zero value are alternatives of each other.
    pub alternate_group: u16,
    /// `tkhd` volume (8.8 fixed point).
    pub volume: i16,
    /// `tkhd` transformation matrix.
    pub matrix: [i32; 9],
    /// Sample entries from `stsd`. [`Sample::description_index`] is 1-based into this list.
    pub entries: Vec<SampleEntry>,
    /// All samples of the track in decode order.
    pub samples: Vec<Sample>,
    /// Edit list entries from `elst`; empty when the track has none.
    pub edits: Vec<Edit>,
    /// Offset to add to raw pts to get presentation time (track timescale), derived from the
    /// leading empty edits and the first non-empty edit's `media_time`.
    pub edit_offset: i64,
    /// Track references (`tref`): (type, track ids).
    pub references: Vec<(FourCc, Vec<u32>)>,
    /// Raw `sdtp` bytes (one per sample) if present.
    pub sdtp: Vec<u8>,
    /// Sample groups (`sbgp` with matching `sgpd`) of the track.
    pub sample_groups: Vec<SampleGroup>,
    /// True if uncompressed audio samples were merged into one [`Sample`] per chunk.
    pub pcm_chunked: bool,
    sync_index: Vec<u32>,
    pts_order: Vec<u32>,
    /// Per entry: the MPEG audio codec its first sample's frame header names, for the
    /// [`CodecConfig::Mp3`] entries, which name no layer themselves.
    mpeg_audio: Vec<Option<crate::CodecId>>,
}

impl Track {
    /// The first sample entry's codec configuration.
    pub fn codec(&self) -> Option<&CodecConfig> {
        self.entries.first().map(|e| &e.codec)
    }

    /// What a decoder needs for this track, in the container-neutral form the codec crates take:
    /// [`Track::codec_params_for`] the first sample's description (the first entry for a track
    /// without samples, or when that index names no entry). A track whose samples use more than
    /// one description ([`Track::has_mixed_descriptions`]) needs `codec_params_for` per sample.
    pub fn codec_params(&self) -> crate::CodecParams {
        let first = self.samples.first().map(|s| s.description_index);
        self.codec_params_for(first.filter(|&i| i >= 1 && i as usize <= self.entries.len()).unwrap_or(1))
    }

    /// What a decoder needs for the samples whose [`Sample::description_index`] is
    /// `description_index` (1-based, into [`Track::entries`]); [`crate::CodecId::Unknown`] when
    /// there is no such entry.
    pub fn codec_params_for(&self, description_index: u32) -> crate::CodecParams {
        let Some(entry) = description_index.checked_sub(1).and_then(|i| self.entries.get(i as usize)) else {
            return crate::CodecParams::new(crate::CodecId::Unknown);
        };
        let codec = &entry.codec;
        let id = self.mpeg_audio.get(description_index as usize - 1).copied().flatten().unwrap_or(codec.codec_id());
        let mut p = crate::CodecParams::new(id).with_extradata(codec.extradata()).with_tag(entry.format.0);
        if let Some(v) = &entry.video {
            p = p.with_size(u32::from(v.width), u32::from(v.height));
        }
        if let Some(a) = &entry.audio {
            p = p.with_sample_rate(a.sample_rate.round() as u32).with_channels(u16::try_from(a.channels).unwrap_or(0));
        }
        match codec {
            CodecConfig::Pcm(c) => {
                p = p.with_pcm(if c.float {
                    crate::PcmFormat::float(c.bits, c.big_endian)
                } else {
                    crate::PcmFormat::int(c.bits, c.signed, c.big_endian)
                });
                p = p.with_sample_rate(c.sample_rate.round() as u32).with_channels(u16::try_from(c.channels).unwrap_or(0));
            }
            CodecConfig::Opus(o) => p = p.with_sample_rate(48_000).with_channels(u16::from(o.output_channels)),
            _ => {}
        }
        p
    }

    /// True when the samples use more than one sample description, so no single
    /// [`Track::codec_params`] describes them all.
    pub fn has_mixed_descriptions(&self) -> bool {
        self.samples.windows(2).any(|w| w[0].description_index != w[1].description_index)
    }
    /// Video parameters of the track's first sample description, or `None` for other track kinds.
    pub fn video(&self) -> Option<&crate::isobmff::codec::VideoParams> {
        self.entries.first().and_then(|e| e.video.as_ref())
    }
    /// Clockwise quarter turns (0–3) the `tkhd` matrix applies for display, or None when it is not
    /// a pure 0/90/180/270° rotation (mirrored, scaled or sheared — left to the caller).
    ///
    /// ISO/IEC 14496-12 §8.3.2: a sample point (p, q) is displayed at p′ = a·p + c·q + x,
    /// q′ = b·p + d·q + y for the matrix {a, b, u, c, d, v, x, y, w}, with y pointing down. So
    /// {0, 1, −1, 0} sends the top row to the right-hand column (90° clockwise — iPhone portrait
    /// video), {−1, 0, 0, −1} is 180° and {0, −1, 1, 0} is 270°. Only the signs matter: a·d − b·c
    /// carries the scale, which the presentation size already reflects.
    pub fn display_rotation(&self) -> Option<u8> {
        let [a, b, _, c, d, _, _, _, _] = self.matrix;
        match (a.signum(), b.signum(), c.signum(), d.signum()) {
            (1, 0, 0, 1) => Some(0),
            (0, 1, -1, 0) => Some(1),
            (-1, 0, 0, -1) => Some(2),
            (0, -1, 1, 0) => Some(3),
            _ => None,
        }
    }
    /// The first sample entry's audio parameters, or `None` for non-audio tracks.
    pub fn audio(&self) -> Option<&crate::isobmff::codec::AudioParams> {
        self.entries.first().and_then(|e| e.audio.as_ref())
    }
    /// Presentation (edit-adjusted) pts of sample `i`.
    pub fn presentation_pts(&self, i: usize) -> Option<i64> {
        self.samples.get(i).map(|s| s.pts.saturating_add(self.edit_offset))
    }
    /// Index of the sample being presented at raw media time `pts` (the last sample, in
    /// presentation order, whose pts is ≤ `pts`; the first sample if `pts` precedes all).
    pub fn sample_at_pts(&self, pts: i64) -> Option<usize> {
        if self.pts_order.is_empty() {
            return None;
        }
        let k = self.pts_order.partition_point(|&i| self.samples[i as usize].pts <= pts);
        Some(self.pts_order[k.saturating_sub(1)] as usize)
    }
    /// Like [`sample_at_pts`](Self::sample_at_pts) but for edit-adjusted presentation time.
    pub fn sample_at_presentation_time(&self, t: i64) -> Option<usize> {
        self.sample_at_pts(t.saturating_sub(self.edit_offset))
    }
    /// The closest sync sample at or before `index` (decode order); 0 if there is none.
    pub fn sync_sample_before(&self, index: usize) -> usize {
        let k = self.sync_index.partition_point(|&i| i as usize <= index);
        if k == 0 { 0 } else { self.sync_index[k - 1] as usize }
    }
    /// Indices of all sync samples.
    pub fn sync_samples(&self) -> &[u32] {
        &self.sync_index
    }

    fn finish_indexes(&mut self) {
        self.sync_index = self.samples.iter().enumerate().filter(|(_, s)| s.is_sync).map(|(i, _)| i as u32).collect();
        let mut order: Vec<u32> = (0..self.samples.len() as u32).collect();
        order.sort_by_key(|&i| (self.samples[i as usize].pts, i));
        self.pts_order = order;
    }
}

/// Text metadata from `udta` (QuickTime `©xxx` atoms and iTunes-style `meta/ilst`, incl. `keys`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Metadata {
    /// (key, value) pairs in file order. Keys are `©xxx` atom names or `keys` item names.
    pub entries: Vec<(String, String)>,
}

impl Metadata {
    /// The value of the first entry whose key equals `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
    /// Title from `©nam` or the `com.apple.quicktime.title` key.
    pub fn title(&self) -> Option<&str> {
        self.get("©nam").or_else(|| self.get("com.apple.quicktime.title")).or_else(|| self.get("title"))
    }
}

/// A parsed MP4/MOV file (headers and sample tables; media data stays in the source).
#[derive(Clone, Debug, PartialEq)]
pub struct Mp4File {
    /// `ftyp` major brand.
    pub major_brand: FourCc,
    /// `ftyp` minor version.
    pub minor_version: u32,
    /// `ftyp` compatible brands.
    pub compatible_brands: Vec<FourCc>,
    /// QuickTime file (major brand `qt  ` or no `ftyp`).
    pub is_quicktime: bool,
    /// Movie timescale (`mvhd`).
    pub timescale: u32,
    /// Movie duration in the movie timescale (`mvhd`; for fragmented files, the longest track).
    pub duration: u64,
    /// Tracks in `trak` order. The track indices taken by other methods refer to this list.
    pub tracks: Vec<Track>,
    /// Text metadata from `udta`.
    pub metadata: Metadata,
    /// File uses movie fragments (`mvex` present).
    pub fragmented: bool,
    /// Byte offset of the `moov` box.
    pub moov_offset: u64,
    /// Number of `moof` boxes found.
    pub fragment_count: usize,
    /// Nero chapter list (`udta/chpl`), starts in the movie timescale. QuickTime chapter tracks
    /// live in media data: read them with [`Mp4File::read_chapters`].
    pub chpl_chapters: Vec<crate::isobmff::Chapter>,
}

impl Mp4File {
    /// Read the bytes of sample `index` of track `track` (index into [`tracks`](Self::tracks)).
    pub fn read_sample<S: ByteSource + ?Sized>(&self, src: &S, track: usize, index: usize) -> Result<Vec<u8>> {
        let t = self.tracks.get(track).ok_or(Error::NoSuchTrack(track))?;
        let s = t.samples.get(index).ok_or(Error::NoSuchSample(index))?;
        read_range(src, s.offset, s.size as u64)
    }
    /// Index of the sample of track `track` being presented at raw media time `pts`; see [`Track::sample_at_pts`]. `None` if `track` is out of range or has no samples.
    pub fn sample_at_pts(&self, track: usize, pts: i64) -> Option<usize> {
        self.tracks.get(track)?.sample_at_pts(pts)
    }
    /// The closest sync sample at or before `index` in track `track`, in decode order; see [`Track::sync_sample_before`]. `None` if `track` is out of range.
    pub fn sync_sample_before(&self, track: usize, index: usize) -> Option<usize> {
        Some(self.tracks.get(track)?.sync_sample_before(index))
    }
    /// The chapter list: the QuickTime chapter track (the text track an enabled track's `tref/chap`
    /// points at; titles read from `src`, starts in the movie timescale, edit-adjusted) when there is
    /// one, else the Nero `chpl` list. Empty when the file has neither.
    ///
    /// Chapters outside the track's first non-empty edit are left out, and one spanning that edit's
    /// start begins where the edit does; later edits are ignored. Titles are cut to
    /// [`MAX_CHAPTER_TITLE_BYTES`](crate::isobmff::MAX_CHAPTER_TITLE_BYTES).
    pub fn read_chapters<S: ByteSource + ?Sized>(&self, src: &S) -> Result<Vec<crate::isobmff::Chapter>> {
        let chap_id = self
            .tracks
            .iter()
            .filter(|t| t.enabled)
            .flat_map(|t| &t.references)
            .find(|(k, _)| k == b"chap")
            .and_then(|(_, ids)| ids.first().copied());
        let Some(t) = chap_id.and_then(|id| self.tracks.iter().find(|t| t.id == id)) else { return Ok(self.chpl_chapters.clone()) };
        let edit = t.edits.iter().find(|e| e.media_time != -1);
        let media_start = edit.map_or(i64::MIN, |e| e.media_time);
        let media_end = edit
            .filter(|e| e.segment_duration > 0)
            .map_or(i64::MAX, |e| media_start.saturating_add(rescale(i128::from(e.segment_duration), self.timescale, t.timescale)));
        let mut out = Vec::new();
        for s in t.samples.iter().take(crate::isobmff::MAX_CHAPTERS.saturating_mul(16)) {
            if s.pts >= media_end || (s.pts.saturating_add(i64::from(s.duration)) <= media_start && s.pts < media_start) {
                continue;
            }
            let data = read_range(src, s.offset, u64::from(s.size).min(MAX_CHAPTER_SAMPLE_BYTES))?;
            let title = if u64::from(s.size) > MAX_CHAPTER_SAMPLE_BYTES {
                crate::isobmff::chapters::parse_text_sample_prefix(&data)
            } else {
                crate::isobmff::chapters::parse_text_sample(&data)
            };
            let title = title.unwrap_or_default();
            let title = crate::isobmff::chapters::truncate_utf8(&title, crate::isobmff::MAX_CHAPTER_TITLE_BYTES).to_string();
            let pts = s.pts.max(media_start).saturating_add(t.edit_offset).max(0) as u64;
            let start = if t.timescale == 0 {
                0
            } else {
                (u128::from(pts) * u128::from(self.timescale) / u128::from(t.timescale)).min(u128::from(u64::MAX)) as u64
            };
            out.push(crate::isobmff::Chapter { start, title });
        }
        Ok(out)
    }
    /// First track of the given kind.
    pub fn track_of_kind(&self, kind: TrackKind) -> Option<usize> {
        self.tracks.iter().position(|t| t.kind == kind)
    }
}

/// Most of a chapter sample read for its title: its 16-bit length and far more title than is kept.
/// A longer title is decoded from what this holds of it.
const MAX_CHAPTER_SAMPLE_BYTES: u64 = 4098;

fn read_range<S: ByteSource + ?Sized>(src: &S, offset: u64, size: u64) -> Result<Vec<u8>> {
    let end = offset.checked_add(size).ok_or(Error::Truncated("sample"))?;
    if end > src.len() {
        return Err(Error::Truncated("sample data"));
    }
    let mut v = vec![0u8; size as usize];
    src.read_at(offset, &mut v)?;
    Ok(v)
}

/// Parse an MP4/MOV file. Only box headers, `ftyp`, `moov` and `moof` boxes are read.
pub fn open<S: ByteSource>(src: S) -> Result<Mp4File> {
    let len = src.len();
    let mut offset = 0u64;
    let mut ftyp: Option<Vec<u8>> = None;
    let mut moov: Option<(u64, Vec<u8>)> = None;
    let mut moofs: Vec<(u64, Vec<u8>)> = Vec::new();
    while offset + 8 <= len {
        let n = (len - offset).min(32) as usize;
        let mut hb = [0u8; 32];
        src.read_at(offset, &mut hb[..n])?;
        let h = BoxHeader::parse(&hb[..n])?;
        let size = h.size.unwrap_or(len - offset);
        let end = offset.saturating_add(size).min(len);
        let body_len = end.saturating_sub(offset + h.header_len);
        let read_body = |max: u64| -> Result<Vec<u8>> {
            if body_len > max {
                return Err(Error::TooLarge("header box"));
            }
            read_range(&src, offset + h.header_len, body_len)
        };
        match &h.kind.0 {
            b"ftyp" if ftyp.is_none() => ftyp = Some(read_body(1 << 16)?),
            b"moov" if moov.is_none() => moov = Some((offset, read_body(MAX_HEADER_BOX)?)),
            b"moof" => moofs.push((offset, read_body(MAX_HEADER_BOX)?)),
            _ => {}
        }
        if h.size.is_none() {
            break;
        }
        // A box that claims to run past the end of the file ends the scan.
        offset = end;
    }
    let (moov_offset, moov) = moov.ok_or(Error::NoMoov)?;
    let mut file = Mp4File {
        major_brand: FourCc::default(),
        minor_version: 0,
        compatible_brands: Vec::new(),
        is_quicktime: true,
        timescale: 0,
        duration: 0,
        tracks: Vec::new(),
        metadata: Metadata::default(),
        fragmented: false,
        moov_offset,
        fragment_count: moofs.len(),
        chpl_chapters: Vec::new(),
    };
    if let Some(f) = &ftyp {
        let mut c = Cur::labeled(f, "ftyp");
        file.major_brand = c.fourcc()?;
        file.minor_version = c.u32().unwrap_or(0);
        while c.remaining() >= 4 {
            file.compatible_brands.push(c.fourcc()?);
        }
        file.is_quicktime = file.major_brand == b"qt  ";
    }
    let mut trex: HashMap<u32, Trex> = HashMap::new();
    let mut trak_payloads = Vec::new();
    let mut meta_left = MAX_METADATA_BYTES;
    let mut chpl = None;
    for b in boxes(&moov) {
        let b = b?;
        match &b.kind.0 {
            b"cmov" => return Err(Error::Unsupported("compressed movie header (cmov)".into())),
            b"mvhd" => {
                let mut c = Cur::labeled(b.payload, "mvhd");
                let (v, _) = c.full_header()?;
                if v == 1 {
                    c.skip(16)?;
                    file.timescale = c.u32()?;
                    file.duration = c.u64()?;
                } else {
                    c.skip(8)?;
                    file.timescale = c.u32()?;
                    file.duration = c.u32()? as u64;
                }
            }
            b"trak" => trak_payloads.push(b.payload),
            b"mvex" => {
                file.fragmented = true;
                for m in boxes(b.payload).map_while(|b| b.ok()) {
                    if m.kind == b"trex" {
                        let mut c = Cur::labeled(m.payload, "trex");
                        c.full_header()?;
                        let id = c.u32()?;
                        trex.insert(id, Trex { description_index: c.u32()?, duration: c.u32()?, size: c.u32()?, flags: c.u32()? });
                    }
                }
            }
            b"udta" => {
                parse_udta(b.payload, &mut file.metadata, &mut meta_left);
                chpl = find(b.payload, b"chpl");
            }
            b"meta" => parse_meta(b.payload, &mut file.metadata, &mut meta_left),
            _ => {}
        }
    }
    if let Some(p) = chpl {
        file.chpl_chapters = crate::isobmff::chapters::parse_chpl(p, file.timescale);
    }
    let mut budget = SampleBudget::new(len);
    for p in trak_payloads {
        let t = parse_trak(p, &file, len, &mut budget)?;
        file.tracks.push(t);
    }
    // Movie fragments.
    let mut next_dts: Vec<i64> =
        file.tracks.iter().map(|t| t.samples.last().map(|s| s.dts.saturating_add(i64::from(s.duration))).unwrap_or(0)).collect();
    // A duplicate track ID resolves to its first track.
    let mut track_index = HashMap::new();
    for (i, t) in file.tracks.iter().enumerate() {
        track_index.entry(t.id).or_insert(i);
    }
    for (moof_off, moof) in &moofs {
        parse_moof(*moof_off, moof, &mut file.tracks, &track_index, &trex, &mut next_dts, &mut budget)?;
    }
    for t in &mut file.tracks {
        if file.fragmented && !moofs.is_empty() {
            let d: u64 = t.samples.iter().map(|s| s.duration as u64).sum();
            t.duration = t.duration.max(d);
            if t.timescale > 0 && file.timescale > 0 {
                let md = (t.duration as u128 * file.timescale as u128 / t.timescale as u128) as u64;
                file.duration = file.duration.max(md);
            }
        }
        t.finish_indexes();
        // Timecode start frame from the first sample.
        if let Some(CodecConfig::Timecode(tc)) = t.entries.first_mut().map(|e| &mut e.codec)
            && let Some(s) = t.samples.first()
            && s.size >= 4
        {
            let mut b = [0u8; 4];
            if src.read_at(s.offset, &mut b).is_ok() {
                tc.start_frame = Some(u32::from_be_bytes(b));
            }
        }
        t.mpeg_audio = mpeg_audio_layers(t, &src);
    }
    Ok(file)
}

/// For each [`CodecConfig::Mp3`] entry, the layer its first sample's frame header names (these
/// entries name none: ffmpeg stores MP2 as `mp4a` + 0x6B), else what the sample entry says
/// (QuickTime `.mp2`, `.mp3`).
fn mpeg_audio_layers<S: ByteSource>(t: &Track, src: &S) -> Vec<Option<crate::CodecId>> {
    use crate::CodecId;
    let from_header = |index: u32| {
        let s = t.samples.iter().find(|s| s.description_index == index && s.size >= 4)?;
        let mut b = [0u8; 4];
        src.read_at(s.offset, &mut b).ok()?;
        if b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
            return None;
        }
        match (b[1] >> 1) & 3 {
            3 => Some(CodecId::Mp1),
            2 => Some(CodecId::Mp2),
            1 => Some(CodecId::Mp3),
            _ => None,
        }
    };
    (1..)
        .zip(&t.entries)
        .map(|(index, e)| match e.codec {
            CodecConfig::Mp3 => from_header(index).or(Some(if e.format == b".mp2" { CodecId::Mp2 } else { CodecId::Mp3 })),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default)]
struct Trex {
    description_index: u32,
    duration: u32,
    size: u32,
    flags: u32,
}

fn parse_language(code: u16) -> String {
    if code < 0x400 {
        // QuickTime Macintosh language code.
        return if code == 0 { "eng".into() } else { "und".into() };
    }
    let c = |s: u16| (((code >> s) & 0x1F) as u8 + 0x60) as char;
    let s: String = [c(10), c(5), c(0)].into_iter().collect();
    if s.chars().all(|ch| ch.is_ascii_lowercase()) { s } else { "und".into() }
}

fn rescale(v: i128, from: u32, to: u32) -> i64 {
    if from == 0 {
        return 0;
    }
    (v * to as i128 / from as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn parse_trak(p: &[u8], file: &Mp4File, file_len: u64, budget: &mut SampleBudget) -> Result<Track> {
    let mut t = Track {
        id: 0,
        kind: TrackKind::Other,
        handler: FourCc::default(),
        handler_name: String::new(),
        timescale: 0,
        duration: 0,
        tkhd_duration: 0,
        language: "und".into(),
        width: 0,
        height: 0,
        enabled: true,
        alternate_group: 0,
        volume: 0,
        matrix: [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000],
        entries: Vec::new(),
        samples: Vec::new(),
        edits: Vec::new(),
        edit_offset: 0,
        references: Vec::new(),
        sdtp: Vec::new(),
        sample_groups: Vec::new(),
        pcm_chunked: false,
        sync_index: Vec::new(),
        pts_order: Vec::new(),
        mpeg_audio: Vec::new(),
    };
    let mut mdia = None;
    for b in boxes(p) {
        let b = b?;
        match &b.kind.0 {
            b"tkhd" => {
                let mut c = Cur::labeled(b.payload, "tkhd");
                let (v, flags) = c.full_header()?;
                t.enabled = flags & 1 != 0;
                if v == 1 {
                    c.skip(16)?;
                    t.id = c.u32()?;
                    c.skip(4)?;
                    t.tkhd_duration = c.u64()?;
                } else {
                    c.skip(8)?;
                    t.id = c.u32()?;
                    c.skip(4)?;
                    t.tkhd_duration = c.u32()? as u64;
                }
                c.skip(8)?;
                let _layer = c.i16()?;
                t.alternate_group = c.u16()?;
                t.volume = c.i16()?;
                c.skip(2)?;
                for m in t.matrix.iter_mut() {
                    *m = c.i32()?;
                }
                t.width = c.u32()? >> 16;
                t.height = c.u32()? >> 16;
            }
            b"edts" => {
                if let Some(e) = find(b.payload, b"elst") {
                    let mut c = Cur::labeled(e, "elst");
                    let (v, _) = c.full_header()?;
                    let n = c.count(if v == 1 { 20 } else { 12 })?;
                    for _ in 0..n {
                        let (segment_duration, media_time) = if v == 1 { (c.u64()?, c.i64()?) } else { (c.u32()? as u64, c.i32()? as i64) };
                        t.edits.push(Edit { segment_duration, media_time, media_rate: c.i32()? });
                    }
                }
            }
            b"tref" => {
                for r in boxes(b.payload).map_while(|b| b.ok()) {
                    let ids = r.payload.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
                    t.references.push((r.kind, ids));
                }
            }
            b"mdia" => mdia = Some(b.payload),
            _ => {}
        }
    }
    let mdia = mdia.ok_or_else(|| Error::Invalid("trak without mdia".into()))?;
    let mut minf = None;
    for b in boxes(mdia) {
        let b = b?;
        match &b.kind.0 {
            b"mdhd" => {
                let mut c = Cur::labeled(b.payload, "mdhd");
                let (v, _) = c.full_header()?;
                if v == 1 {
                    c.skip(16)?;
                    t.timescale = c.u32()?;
                    t.duration = c.u64()?;
                } else {
                    c.skip(8)?;
                    t.timescale = c.u32()?;
                    t.duration = c.u32()? as u64;
                }
                t.language = parse_language(c.u16()?);
            }
            b"hdlr" => {
                let mut c = Cur::labeled(b.payload, "hdlr");
                c.full_header()?;
                let _pre = c.u32()?;
                t.handler = c.fourcc()?;
                c.skip(12)?;
                let name = c.rest();
                // ISO: null-terminated UTF-8; QuickTime: Pascal string.
                let s = if !name.is_empty() && name[0] as usize == name.len() - 1 && name.len() > 1 {
                    &name[1..]
                } else {
                    name.split(|&b| b == 0).next().unwrap_or(&[])
                };
                t.handler_name = String::from_utf8_lossy(s).into_owned();
            }
            b"minf" => minf = Some(b.payload),
            _ => {}
        }
    }
    t.kind = match &t.handler.0 {
        b"vide" => TrackKind::Video,
        b"soun" => TrackKind::Audio,
        b"tmcd" => TrackKind::Timecode,
        b"text" | b"sbtl" | b"subt" | b"clcp" => TrackKind::Subtitle,
        _ => TrackKind::Other,
    };
    // Edit offset in media timescale.
    let mut empty = 0i128;
    for e in &t.edits {
        if e.media_time == -1 {
            empty += e.segment_duration as i128;
        } else {
            t.edit_offset = rescale(empty, file.timescale, t.timescale).saturating_sub(e.media_time);
            break;
        }
    }
    if let Some(stbl) = minf.and_then(|m| find(m, b"stbl")) {
        parse_stbl(stbl, &mut t, file.is_quicktime, file_len, budget)?;
    }
    Ok(t)
}

mod metadata;
mod moof;
mod stbl;

use metadata::{parse_meta, parse_udta};
use moof::parse_moof;
use stbl::parse_stbl;
