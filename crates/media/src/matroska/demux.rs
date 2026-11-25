//! Matroska / WebM demuxer: EBML header, Segment walk, cluster scanning, packet reading, seeking.

use crate::matroska::error::{Error, Result};
use crate::matroska::meta::{Attachment, ClusterInfo, CuePoint, Edition, SeekEntry, SegmentInfo, Tag};
use crate::matroska::track::Track;
use std::collections::HashMap;

const WIN: usize = 64 * 1024;
/// Source bytes damage recovery may read per file (the same bound as the Ogg demuxer's).
const RECOVERY_BUDGET: u64 = 64 << 20;
/// Largest EBML header element read. A real one is a few dozen bytes.
const MAX_EBML_HEADER: u64 = 4096;
/// Largest laced block body read into memory to split its frames. Laced blocks hold small frames
/// (audio); a bigger one is skipped as damage rather than read whole.
const MAX_LACED_BODY: u64 = 16 << 20;
/// Warnings kept per file; the rest are counted in one last entry.
const MAX_WARNINGS: usize = 256;
/// Upper bound for level-1 metadata elements read into memory (Info, Tracks, Cues, Tags…).
const MAX_META: u64 = 256 << 20;
/// Cap on the frames indexed from one file, all tracks together.
const MAX_FRAMES: usize = 1 << 26;
/// Bytes of file each indexed frame needs beyond the first `MIN_FRAME_ALLOWANCE` frames. An
/// indexed frame takes about 76 bytes of memory (its [`Sample`] and keyframe entry) and fixed
/// lacing can describe one in no bytes at all, so without this a file could make the index
/// about 76 times its own size. Real frames carry a block header or lace size and their data,
/// well over 8 bytes each.
const FILE_BYTES_PER_FRAME: u64 = 8;
/// Frames any file may index, however short: about 80 MiB of index.
const MIN_FRAME_ALLOWANCE: usize = 1 << 20;
/// Cap on the TrackEntries of a file. Each kept entry needs only 5 bytes of file but becomes a
/// [`Track`] of several hundred bytes.
pub(super) const MAX_TRACKS: usize = 1 << 16;

/// Record a warning, keeping at most `MAX_WARNINGS` and one entry saying the rest were dropped.
fn warn(warnings: &mut Vec<String>, msg: String) {
    match warnings.len().cmp(&MAX_WARNINGS) {
        std::cmp::Ordering::Less => warnings.push(msg),
        std::cmp::Ordering::Equal => warnings.push("further warnings omitted".into()),
        std::cmp::Ordering::Greater => {}
    }
}

/// What is left of the file-wide limits while clusters are scanned.
struct FrameBudget {
    /// Frames (what the file's length allows, at most `MAX_FRAMES`, minus those already indexed).
    frames: usize,
    /// Bytes of file not yet claimed by a frame. Each frame claims its stored size and at least
    /// one byte, because fixed-size lacing describes up to 256 empty frames in a 4-byte block.
    bytes: u64,
}

impl FrameBudget {
    fn new(file_len: u64) -> Self {
        let by_len = usize::try_from(file_len / FILE_BYTES_PER_FRAME).unwrap_or(usize::MAX);
        Self { frames: by_len.clamp(MIN_FRAME_ALLOWANCE, MAX_FRAMES), bytes: file_len }
    }

    /// The budget left for a file whose `tracks` already hold the frames of an earlier scan.
    fn resumed(file_len: u64, tracks: &[Track]) -> Self {
        let mut b = Self::new(file_len);
        let (frames, bytes) = tracks
            .iter()
            .flat_map(|t| t.samples.iter())
            .fold((0usize, 0u64), |(n, b), s| (n.saturating_add(1), b.saturating_add(u64::from(s.size.max(1)))));
        b.frames = b.frames.saturating_sub(frames);
        b.bytes = b.bytes.saturating_sub(bytes);
        b
    }

    fn take(&mut self, frames: &[(u64, u32)]) -> Result<()> {
        self.frames = self.frames.checked_sub(frames.len()).ok_or(Error::TooLarge("frame count"))?;
        let bytes = frames.iter().map(|&(_, n)| u64::from(n.max(1))).sum::<u64>();
        self.bytes = self.bytes.checked_sub(bytes).ok_or(Error::TooLarge("frame count"))?;
        Ok(())
    }
}

/// Options for [`open_with`] / [`Demuxer::with_options`].
#[derive(Clone, Debug)]
pub struct OpenOptions {
    /// Scan every cluster at open to build per-track sample tables and keyframe indices
    /// (reads only element and block headers, never frame payloads). Default `true`.
    ///
    /// With `false`, open stops at the first Cluster and uses the SeekHead to find Cues, Tags,
    /// Chapters… placed after the clusters; [`Demuxer::seek`] then uses Cues (or builds the index
    /// on first use if there are none).
    pub index: bool,
    /// Verify CRC-32 elements of level-1 elements (and clusters while indexing). Mismatches are
    /// recorded in [`MkvFile::crc_errors`]; parsing continues. Default `false`.
    pub verify_crc: bool,
    /// Index only the front of the file: stop the cluster scan at this absolute byte offset.
    /// Default `None` (scan every cluster).
    ///
    /// The first Cluster is always scanned (a file needs at least one to be useful). After it, an
    /// element of known size (a Cluster, or Cues or Tags between Clusters) is read only if it lies
    /// entirely below the limit, so a byte source that has loaded only the front of the file is
    /// enough. Two things can still read past the limit: the first Cluster, and a Cluster of
    /// unknown size (live streams), whose end is only found by scanning it. Info and Tracks are
    /// read as usual; Cues, Tags and the other elements after the scanned part are not read
    /// until the index is completed.
    ///
    /// The sample tables then cover everything before [`MkvFile::index_end`], which is `Some`
    /// until the index is complete ([`MkvFile::indexed`]). Continue with [`MkvFile::extend_index`]
    /// without rescanning. Ignored when `index` is `false`.
    pub index_limit: Option<u64>,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self { index: true, verify_crc: false, index_limit: None }
    }
}

/// A parsed Matroska / WebM file (first Segment).
#[derive(Clone, Debug)]
pub struct MkvFile {
    /// EBML `DocType`: `matroska` or `webm`.
    pub doc_type: String,
    /// EBML `DocTypeVersion`.
    pub doc_type_version: u64,
    /// EBML `DocTypeReadVersion`: the lowest reader version that can parse the file.
    pub doc_type_read_version: u64,
    /// Absolute offset of the Segment's data (base for SeekHead and Cues positions).
    pub segment_data_start: u64,
    /// Absolute end of the Segment (file end for unknown-size / live segments).
    pub segment_end: u64,
    /// Segment size field was "unknown" (live / streamed output).
    pub live: bool,
    /// Segment `Info` element.
    pub info: SegmentInfo,
    /// Parsed `Tracks` entries, in file order.
    pub tracks: Vec<Track>,
    /// `SeekHead` entries.
    pub seek_head: Vec<SeekEntry>,
    /// Cue points, sorted by time.
    pub cues: Vec<CuePoint>,
    /// Clusters in file order (complete only when [`MkvFile::indexed`]).
    pub clusters: Vec<ClusterInfo>,
    /// Chapter editions from `Chapters`.
    pub chapters: Vec<Edition>,
    /// `Tags` elements.
    pub tags: Vec<Tag>,
    /// Attached files from `Attachments`.
    pub attachments: Vec<Attachment>,
    /// Absolute offset of the first Cluster.
    pub first_cluster: Option<u64>,
    /// Sample tables have been built by scanning all clusters.
    pub indexed: bool,
    /// Set while an open limited by [`OpenOptions::index_limit`] left the index incomplete: the
    /// absolute offset where the cluster scan stopped and where [`MkvFile::extend_index`]
    /// continues. The tables cover every frame before it and none after. `None` once the index
    /// is complete, and for files opened without an index.
    pub index_end: Option<u64>,
    /// Walker state to continue the scan from [`MkvFile::index_end`].
    resume: Option<Resume>,
    /// Number of CRC-32 elements verified (with `verify_crc`).
    pub crc_checked: u32,
    /// Absolute offsets of elements whose CRC-32 did not match.
    pub crc_errors: Vec<u64>,
    /// Non-fatal problems met while parsing (damage, resyncs, truncation).
    pub warnings: Vec<String>,
}

/// What a scan stopped at [`MkvFile::index_end`] needs to continue: the level-1 elements already
/// read (so a later pass does not read them twice) and the metadata byte budget left.
#[derive(Clone, Debug, Default)]
struct Resume {
    parsed: HashMap<u64, u64>,
    meta_left: u64,
}

/// One demuxed frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    /// Index into [`MkvFile::tracks`].
    pub track: usize,
    /// Matroska `TrackNumber`.
    pub track_number: u64,
    /// Presentation timestamp in timestamp ticks (the track timebase, [`Track::timebase`]).
    pub pts: i64,
    /// Presentation timestamp in nanoseconds.
    pub pts_ns: i64,
    /// Duration in ticks (0 if unknown).
    pub duration: u64,
    /// Duration in nanoseconds (0 if unknown).
    pub duration_ns: u64,
    #[allow(missing_docs)]
    pub keyframe: bool,
    #[allow(missing_docs)]
    pub discardable: bool,
    #[allow(missing_docs)]
    pub invisible: bool,
    /// `DiscardPadding` (ns) from the enclosing BlockGroup.
    pub discard_padding_ns: Option<i64>,
    /// Absolute offset of the stored frame data.
    pub offset: u64,
    /// Index of the frame within a laced block (0 when not laced). Laced frames after the first
    /// only have exact timestamps when the block or track carries a duration; otherwise they
    /// repeat the block timestamp and `duration` is 0.
    pub lace: u16,
    /// Frame bytes with header stripping undone. Frames of tracks with other content encodings
    /// (zlib, encryption) are returned as stored; see [`Track::frames_readable`].
    pub data: Vec<u8>,
}

/// Result of [`Demuxer::seek`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeekPoint {
    /// Index into [`MkvFile::tracks`].
    pub track: usize,
    /// Keyframe timestamp in ticks and ns.
    pub pts: i64,
    /// Keyframe timestamp in nanoseconds.
    pub pts_ns: i64,
    /// Sample index of the keyframe when the file is indexed.
    pub sample: Option<usize>,
    /// Absolute offset of the Cluster holding the keyframe.
    pub cluster_offset: u64,
    /// Absolute offset of the Block holding the keyframe.
    pub block_offset: u64,
}

/// A keyframe index entry ([`Demuxer::keyframe_index`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keyframe {
    /// Index into the track's sample table.
    pub sample: usize,
    /// Keyframe timestamp in ticks of the track timebase.
    pub pts: i64,
    /// Keyframe timestamp in nanoseconds.
    pub pts_ns: i64,
    /// Absolute offset of the Block holding the keyframe.
    pub block_offset: u64,
}

mod block;
mod io;
mod open;
mod stream;

pub use open::{open, open_with};
pub use stream::Demuxer;
