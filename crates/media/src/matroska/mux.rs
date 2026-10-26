//! Minimal Matroska / WebM muxer: SimpleBlocks (optionally laced), BlockGroups for explicit
//! durations, Cues, SeekHead, Info with Duration. Output must be `Write + Seek` (sizes and the
//! SeekHead are back-patched on [`MkvWriter::finish`]).

use std::io::{Seek, SeekFrom, Write};

use crate::matroska::ebml::{write_id, write_size, write_unknown_size};
use crate::matroska::error::{Error, Result};
use crate::matroska::ids::*;
use crate::matroska::track::TrackKind;

/// Lacing used for audio tracks. Only frames of a track with a DefaultDuration lace, and a lace
/// ends where the timestamps stop advancing by that duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LacingMode {
    None,
    Xiph,
    Ebml,
    /// Fixed-size lacing when consecutive frames have equal sizes, otherwise EBML lacing.
    FixedOrEbml,
}

/// Track description for [`MkvWriter`].
#[derive(Clone, Debug)]
pub struct TrackSpec {
    pub kind: TrackKind,
    pub codec_id: String,
    pub codec_private: Vec<u8>,
    pub default_duration_ns: Option<u64>,
    pub language: Option<String>,
    pub name: Option<String>,
    /// Pixel width/height for video tracks.
    pub video_size: Option<(u32, u32)>,
    /// (sampling frequency, channels, bit depth) for audio tracks.
    pub audio: Option<(f64, u64, Option<u64>)>,
    pub codec_delay_ns: u64,
    pub seek_pre_roll_ns: u64,
}

impl TrackSpec {
    pub fn new(kind: TrackKind, codec_id: impl Into<String>) -> Self {
        Self {
            kind,
            codec_id: codec_id.into(),
            codec_private: Vec::new(),
            default_duration_ns: None,
            language: None,
            name: None,
            video_size: None,
            audio: None,
            codec_delay_ns: 0,
            seek_pre_roll_ns: 0,
        }
    }
}

/// Muxer options.
#[derive(Clone, Debug)]
pub struct MuxOptions {
    /// `webm` or `matroska`.
    pub doc_type: String,
    /// Nanoseconds per tick (default 1 000 000).
    pub timestamp_scale: u64,
    /// Start a new cluster at the next video keyframe (or any frame without video) after this long.
    pub cluster_duration_ns: u64,
    pub lacing: LacingMode,
    /// Maximum frames per laced block. Matroska allows 1 to 256; [`MkvWriter::new`] clamps
    /// other values into that range.
    pub max_lace_frames: usize,
    pub title: Option<String>,
    pub writing_app: String,
}

impl Default for MuxOptions {
    fn default() -> Self {
        Self {
            doc_type: "matroska".into(),
            timestamp_scale: 1_000_000,
            cluster_duration_ns: 1_000_000_000,
            lacing: LacingMode::None,
            max_lace_frames: 8,
            title: None,
            writing_app: "craft-media".into(),
        }
    }
}

pub(crate) fn el(out: &mut Vec<u8>, id: u32, data: &[u8]) {
    write_id(out, id);
    write_size(out, data.len() as u64, 1);
    out.extend_from_slice(data);
}

pub(crate) fn el_uint(out: &mut Vec<u8>, id: u32, v: u64) {
    let n = (8 - v.leading_zeros() as usize / 8).max(1);
    el(out, id, &v.to_be_bytes()[8 - n..]);
}

pub(crate) fn el_float(out: &mut Vec<u8>, id: u32, v: f64) {
    el(out, id, &v.to_be_bytes());
}

pub(crate) fn el_str(out: &mut Vec<u8>, id: u32, s: &str) {
    el(out, id, s.as_bytes());
}

fn void(out: &mut Vec<u8>, total: usize) {
    debug_assert!(total >= 2);
    let sl = if total - 2 <= 126 { 1 } else { 8 };
    write_id(out, VOID);
    write_size(out, (total - 1 - sl) as u64, sl);
    out.resize(out.len() + total - 1 - sl, 0);
}

#[derive(Clone, Debug)]
struct Pending {
    track: usize,
    ts: i64,
    frames: Vec<Vec<u8>>,
}

const SEEK_HEAD_SPACE: usize = 128;

/// Streaming Matroska/WebM writer.
pub struct MkvWriter<W: Write + Seek> {
    w: W,
    opts: MuxOptions,
    tracks: Vec<TrackSpec>,
    segment_size_pos: u64,
    segment_data_start: u64,
    seek_head_pos: u64,
    duration_pos: u64,
    info_pos: u64,
    tracks_pos: u64,
    cluster: Vec<u8>,
    cluster_ts: Option<i64>,
    cluster_start_ns: i64,
    pending: Option<Pending>,
    cues: Vec<(u64, u64, u64)>, // (time ticks, track number, cluster position)
    /// The cluster number in which each track last had a cue. Starting a cluster bumps
    /// `cluster_number` instead of clearing an entry for every declared track.
    track_cued_in: Vec<u64>,
    cluster_number: u64,
    end_ns: i64,
    has_video: bool,
}

impl<W: Write + Seek> MkvWriter<W> {
    pub fn new(mut w: W, tracks: Vec<TrackSpec>, mut opts: MuxOptions) -> Result<Self> {
        if tracks.is_empty() || opts.timestamp_scale == 0 {
            return Err(Error::Invalid("muxer needs at least one track and a non-zero timestamp scale".into()));
        }
        // A block stores its frame count minus one in a byte.
        opts.max_lace_frames = opts.max_lace_frames.clamp(1, 256);
        let mut out = Vec::new();
        let mut h = Vec::new();
        el_uint(&mut h, EBML_VERSION, 1);
        el_uint(&mut h, EBML_READ_VERSION, 1);
        el_uint(&mut h, EBML_MAX_ID_LENGTH, 4);
        el_uint(&mut h, EBML_MAX_SIZE_LENGTH, 8);
        el_str(&mut h, DOC_TYPE, &opts.doc_type);
        el_uint(&mut h, DOC_TYPE_VERSION, 4);
        el_uint(&mut h, DOC_TYPE_READ_VERSION, 2);
        el(&mut out, EBML, &h);
        write_id(&mut out, SEGMENT);
        let base = w.stream_position()?;
        let segment_size_pos = base + out.len() as u64;
        write_unknown_size(&mut out, 8);
        let segment_data_start = base + out.len() as u64;
        let seek_head_pos = segment_data_start;
        void(&mut out, SEEK_HEAD_SPACE);
        // Info
        let info_pos = base + out.len() as u64;
        let mut info = Vec::new();
        el_uint(&mut info, TIMESTAMP_SCALE, opts.timestamp_scale);
        el_str(&mut info, MUXING_APP, "craft-media");
        el_str(&mut info, WRITING_APP, &opts.writing_app);
        if let Some(t) = &opts.title {
            el_str(&mut info, TITLE, t);
        }
        let dur_off_in_info = info.len() + 2 + 1; // ID (2) + size (1)
        el_float(&mut info, DURATION, 0.0);
        write_id(&mut out, INFO);
        write_size(&mut out, info.len() as u64, 1);
        let duration_pos = base + out.len() as u64 + dur_off_in_info as u64;
        out.extend_from_slice(&info);
        // Tracks
        let tracks_pos = base + out.len() as u64;
        let mut tr = Vec::new();
        for (i, t) in tracks.iter().enumerate() {
            let mut e = Vec::new();
            el_uint(&mut e, TRACK_NUMBER, i as u64 + 1);
            el_uint(&mut e, TRACK_UID, 0x1000 + i as u64 + 1);
            let ty = match t.kind {
                TrackKind::Video => 1,
                TrackKind::Audio => 2,
                TrackKind::Subtitle => 0x11,
                TrackKind::Complex => 3,
                TrackKind::Logo => 0x10,
                TrackKind::Buttons => 0x12,
                TrackKind::Control => 0x20,
                TrackKind::Metadata => 0x21,
                TrackKind::Other(n) => n as u64,
            };
            el_uint(&mut e, TRACK_TYPE, ty);
            el_uint(&mut e, FLAG_LACING, (t.kind == TrackKind::Audio && opts.lacing != LacingMode::None) as u64);
            if let Some(l) = &t.language {
                el_str(&mut e, LANGUAGE, l);
            }
            if let Some(n) = &t.name {
                el_str(&mut e, NAME, n);
            }
            el_str(&mut e, CODEC_ID, &t.codec_id);
            if !t.codec_private.is_empty() {
                el(&mut e, CODEC_PRIVATE, &t.codec_private);
            }
            if let Some(d) = t.default_duration_ns {
                el_uint(&mut e, DEFAULT_DURATION, d);
            }
            if t.codec_delay_ns > 0 {
                el_uint(&mut e, CODEC_DELAY, t.codec_delay_ns);
            }
            if t.seek_pre_roll_ns > 0 {
                el_uint(&mut e, SEEK_PRE_ROLL, t.seek_pre_roll_ns);
            }
            if let Some((pw, ph)) = t.video_size {
                let mut v = Vec::new();
                el_uint(&mut v, PIXEL_WIDTH, pw as u64);
                el_uint(&mut v, PIXEL_HEIGHT, ph as u64);
                el(&mut e, VIDEO, &v);
            }
            if let Some((rate, ch, bits)) = t.audio {
                let mut a = Vec::new();
                el_float(&mut a, SAMPLING_FREQUENCY, rate);
                el_uint(&mut a, CHANNELS, ch);
                if let Some(b) = bits {
                    el_uint(&mut a, BIT_DEPTH, b);
                }
                el(&mut e, AUDIO, &a);
            }
            el(&mut tr, TRACK_ENTRY, &e);
        }
        el(&mut out, TRACKS, &tr);
        w.write_all(&out)?;
        let has_video = tracks.iter().any(|t| t.kind == TrackKind::Video);
        let n = tracks.len();
        Ok(Self {
            w,
            opts,
            tracks,
            segment_size_pos,
            segment_data_start,
            seek_head_pos,
            duration_pos,
            info_pos,
            tracks_pos,
            cluster: Vec::new(),
            cluster_ts: None,
            cluster_start_ns: 0,
            pending: None,
            cues: Vec::new(),
            track_cued_in: vec![u64::MAX; n],
            cluster_number: 0,
            end_ns: 0,
            has_video,
        })
    }

    /// Add one frame. `pts_ns` is the presentation time (codec delay is added back for storage);
    /// frames must be supplied in the desired file order. `duration_ns` forces a BlockGroup with a
    /// BlockDuration when it differs from the track's DefaultDuration.
    pub fn write_frame(&mut self, track: usize, pts_ns: i64, keyframe: bool, data: &[u8], duration_ns: Option<u64>) -> Result<()> {
        let spec = self.tracks.get(track).ok_or(Error::NoSuchTrack(track))?;
        // the Matroska ProRes mapping stores frames without the 8-byte `size`+`icpf` header
        let data = if spec.codec_id == "V_PRORES" && data.get(4..8) == Some(b"icpf") { &data[8..] } else { data };
        let scale = self.opts.timestamp_scale as i64;
        let stored_ns = pts_ns + spec.codec_delay_ns as i64;
        if stored_ns < 0 {
            return Err(Error::Invalid("negative timestamp".into()));
        }
        let ts = (stored_ns + scale / 2) / scale;
        let dur_end = pts_ns + duration_ns.or(spec.default_duration_ns).unwrap_or(0) as i64;
        self.end_ns = self.end_ns.max(dur_end).max(pts_ns);
        let is_video = spec.kind == TrackKind::Video;
        let lacing = spec.kind == TrackKind::Audio && self.opts.lacing != LacingMode::None;
        let explicit_dur = duration_ns.filter(|&d| Some(d) != spec.default_duration_ns);
        let lace_duration = spec.default_duration_ns.filter(|_| lacing && explicit_dur.is_none());
        // new cluster?
        let start_new = match self.cluster_ts {
            None => true,
            Some(cts) => {
                let long = stored_ns - self.cluster_start_ns >= self.opts.cluster_duration_ns as i64;
                (long && (!self.has_video || (is_video && keyframe))) || ts - cts > 30_000 || ts < cts
            }
        };
        if start_new {
            self.flush_cluster()?;
            self.cluster_ts = Some(ts);
            self.cluster_start_ns = stored_ns;
            el_uint(&mut self.cluster, TIMESTAMP, ts as u64);
        }
        let cts = self.cluster_ts.unwrap_or(0);
        // cue: video keyframes, or the first frame of each track per cluster when there is no video
        if (is_video && keyframe) || (!self.has_video && self.track_cued_in[track] != self.cluster_number) {
            self.track_cued_in[track] = self.cluster_number;
            let pos = self.w.stream_position()? - self.segment_data_start;
            if self.cues.last().is_none_or(|c| c.2 != pos || c.1 != track as u64 + 1) {
                self.cues.push((ts as u64, track as u64 + 1, pos));
            }
        }
        // A laced frame has no timestamp of its own: readers space the frames of a block by
        // DefaultDuration. So only frames with that duration lace, and a frame joins the pending
        // lace only when the tick it would get there is the tick it has.
        if let Some(dd) = lace_duration {
            let max = self.opts.max_lace_frames;
            let joins = self.pending.as_ref().is_some_and(|p| {
                let n = p.frames.len();
                let implied = ((cts + p.ts) as i128 * scale as i128 + n as i128 * dd as i128 + scale as i128 / 2) / scale as i128;
                p.track == track && n < max && implied == ts as i128
            });
            if !joins {
                self.flush_pending();
            }
            match &mut self.pending {
                Some(p) => p.frames.push(data.to_vec()),
                None => self.pending = Some(Pending { track, ts: ts - cts, frames: vec![data.to_vec()] }),
            }
            return Ok(());
        }
        self.flush_pending();
        let mut block = Vec::new();
        write_size(&mut block, track as u64 + 1, 1);
        block.extend_from_slice(&((ts - cts) as i16).to_be_bytes());
        if let Some(d) = explicit_dur {
            block.push(0);
            block.extend_from_slice(data);
            let mut g = Vec::new();
            el(&mut g, BLOCK, &block);
            el_uint(&mut g, BLOCK_DURATION, ((u128::from(d) + scale as u128 / 2) / scale as u128) as u64);
            if !keyframe {
                el(&mut g, REFERENCE_BLOCK, &[0xFF]);
            }
            el(&mut self.cluster, BLOCK_GROUP, &g);
        } else {
            block.push(if keyframe { 0x80 } else { 0 });
            block.extend_from_slice(data);
            el(&mut self.cluster, SIMPLE_BLOCK, &block);
        }
        Ok(())
    }

    fn flush_pending(&mut self) {
        let Some(p) = self.pending.take() else { return };
        let mut block = Vec::new();
        write_size(&mut block, p.track as u64 + 1, 1);
        block.extend_from_slice(&(p.ts as i16).to_be_bytes());
        let n = p.frames.len();
        let mode = if n == 1 { LacingMode::None } else { self.opts.lacing };
        let fixed = p.frames.iter().all(|f| f.len() == p.frames[0].len());
        match mode {
            LacingMode::None => block.push(0x80),
            LacingMode::Xiph => {
                block.push(0x80 | 0x02);
                block.push((n - 1) as u8);
                for f in &p.frames[..n - 1] {
                    let mut s = f.len();
                    while s >= 255 {
                        block.push(255);
                        s -= 255;
                    }
                    block.push(s as u8);
                }
            }
            LacingMode::FixedOrEbml if fixed => {
                block.push(0x80 | 0x04);
                block.push((n - 1) as u8);
            }
            LacingMode::Ebml | LacingMode::FixedOrEbml => {
                block.push(0x80 | 0x06);
                block.push((n - 1) as u8);
                write_size(&mut block, p.frames[0].len() as u64, 1);
                for i in 1..n - 1 {
                    let d = p.frames[i].len() as i64 - p.frames[i - 1].len() as i64;
                    // signed VINT: smallest length whose range holds d
                    let mut len = 1;
                    while len < 8 && d.unsigned_abs() >= (1u64 << (7 * len - 1)) - 1 {
                        len += 1;
                    }
                    let v = (d + (1i64 << (7 * len - 1)) - 1) as u64 | (1u64 << (7 * len));
                    block.extend_from_slice(&v.to_be_bytes()[8 - len..]);
                }
            }
        }
        for f in &p.frames {
            block.extend_from_slice(f);
        }
        el(&mut self.cluster, SIMPLE_BLOCK, &block);
    }

    fn flush_cluster(&mut self) -> Result<()> {
        self.flush_pending();
        if self.cluster_ts.is_some() {
            let mut out = Vec::new();
            el(&mut out, CLUSTER, &self.cluster);
            self.w.write_all(&out)?;
        }
        self.cluster.clear();
        self.cluster_ts = None;
        self.cluster_number = self.cluster_number.wrapping_add(1);
        Ok(())
    }

    /// Write Cues, back-patch SeekHead / Segment size / Duration, and return the writer.
    pub fn finish(mut self) -> Result<W> {
        self.flush_cluster()?;
        let cues_pos = self.w.stream_position()?;
        let mut cues = Vec::new();
        for &(time, track, pos) in &self.cues {
            let mut ctp = Vec::new();
            el_uint(&mut ctp, CUE_TRACK, track);
            el_uint(&mut ctp, CUE_CLUSTER_POSITION, pos);
            let mut cp = Vec::new();
            el_uint(&mut cp, CUE_TIME, time);
            el(&mut cp, CUE_TRACK_POSITIONS, &ctp);
            el(&mut cues, CUE_POINT, &cp);
        }
        let has_cues = !cues.is_empty();
        if has_cues {
            let mut out = Vec::new();
            el(&mut out, CUES, &cues);
            self.w.write_all(&out)?;
        }
        let end = self.w.stream_position()?;
        // SeekHead
        let mut sh = Vec::new();
        let mut entries = vec![(INFO, self.info_pos), (TRACKS, self.tracks_pos)];
        if has_cues {
            entries.push((CUES, cues_pos));
        }
        for (id, pos) in entries {
            let mut s = Vec::new();
            let mut idb = Vec::new();
            write_id(&mut idb, id);
            el(&mut s, SEEK_ID, &idb);
            el_uint(&mut s, SEEK_POSITION, pos - self.segment_data_start);
            el(&mut sh, SEEK, &s);
        }
        let mut out = Vec::new();
        el(&mut out, SEEK_HEAD, &sh);
        if out.len() + 2 > SEEK_HEAD_SPACE {
            return Err(Error::Invalid("SeekHead does not fit its reserved space".into()));
        }
        let rest = SEEK_HEAD_SPACE - out.len();
        void(&mut out, rest);
        self.w.seek(SeekFrom::Start(self.seek_head_pos))?;
        self.w.write_all(&out)?;
        // Duration
        let dur = self.end_ns as f64 / self.opts.timestamp_scale as f64;
        self.w.seek(SeekFrom::Start(self.duration_pos))?;
        self.w.write_all(&dur.to_be_bytes())?;
        // Segment size
        let mut sz = Vec::new();
        write_size(&mut sz, end - self.segment_data_start, 8);
        self.w.seek(SeekFrom::Start(self.segment_size_pos))?;
        self.w.write_all(&sz)?;
        self.w.seek(SeekFrom::Start(end))?;
        self.w.flush()?;
        Ok(self.w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matroska::{Demuxer, TrackKind};
    use std::io::Cursor;

    fn roundtrip(lacing: LacingMode) {
        let mut v = TrackSpec::new(TrackKind::Video, "V_MJPEG");
        v.video_size = Some((16, 16));
        v.default_duration_ns = Some(40_000_000);
        let mut a = TrackSpec::new(TrackKind::Audio, "A_PCM/INT/LIT");
        a.audio = Some((48000.0, 1, Some(16)));
        a.default_duration_ns = Some(10_000_000);
        let mut s = TrackSpec::new(TrackKind::Subtitle, "S_TEXT/UTF8");
        s.language = Some("fre".into());
        let opts = MuxOptions { lacing, ..Default::default() };
        let mut m = MkvWriter::new(Cursor::new(Vec::new()), vec![v, a, s], opts).unwrap();
        let mut expect = Vec::new();
        for i in 0..50i64 {
            let vf = vec![i as u8; 100 + i as usize];
            m.write_frame(0, i * 40_000_000, i % 10 == 0, &vf, None).unwrap();
            expect.push((0usize, i * 40, vf, i % 10 == 0));
            for k in 0..4 {
                let n = if lacing == LacingMode::FixedOrEbml { 960 } else { 900 + (i as usize * 7 + k * 13) % 300 };
                let af = vec![(i * 4 + k as i64) as u8; n];
                let t = i * 40 + k as i64 * 10;
                m.write_frame(1, t * 1_000_000, true, &af, None).unwrap();
                expect.push((1, t, af, true));
            }
            if i % 20 == 5 {
                m.write_frame(2, i * 40_000_000, true, b"hello", Some(500_000_000)).unwrap();
                expect.push((2, i * 40, b"hello".to_vec(), true));
            }
        }
        let bytes = m.finish().unwrap().into_inner();
        let mut d = Demuxer::from_slice(&bytes).unwrap();
        assert!(d.file().warnings.is_empty(), "{:?}", d.file().warnings);
        assert_eq!(d.tracks()[2].language, "fre");
        assert!(!d.file().cues.is_empty());
        assert_eq!(d.duration_ns(), Some(2_300_000_000));
        let got: Vec<_> = d.by_ref().map(|p| p.unwrap()).collect();
        assert_eq!(got.len(), expect.len());
        for (g, e) in got.iter().zip(&expect) {
            assert_eq!((g.track, g.pts, &g.data, g.keyframe), (e.0, e.1, &e.2, e.3));
        }
        let sub = got.iter().find(|p| p.track == 2).unwrap();
        assert_eq!(sub.duration, 500);
        // the index agrees with the packet stream
        let f = d.file();
        for (ti, t) in f.tracks.iter().enumerate() {
            let pk: Vec<_> = got.iter().filter(|p| p.track == ti).collect();
            assert_eq!(t.samples.len(), pk.len());
            for (i, p) in pk.iter().enumerate() {
                assert_eq!(f.read_sample(&bytes[..], ti, i).unwrap(), p.data);
                assert_eq!(t.samples[i].pts, p.pts);
            }
        }
        // seeking
        let sp = d.seek(0, 1_230_000_000).unwrap();
        assert_eq!(sp.pts, 1200);
        let p = d.next_packet().unwrap().unwrap();
        assert_eq!((p.track, p.pts, p.keyframe), (0, 1200, true));
        let sp = d.seek(1, 1_235_000_000).unwrap();
        assert_eq!(sp.pts, 1230);
        let p = d.next_packet().unwrap().unwrap();
        assert_eq!((p.track, p.pts), (1, 1230));
    }

    #[test]
    fn roundtrip_unlaced() {
        roundtrip(LacingMode::None);
    }
    #[test]
    fn roundtrip_xiph() {
        roundtrip(LacingMode::Xiph);
    }
    #[test]
    fn roundtrip_ebml() {
        roundtrip(LacingMode::Ebml);
    }
    #[test]
    fn roundtrip_fixed() {
        roundtrip(LacingMode::FixedOrEbml);
    }

    /// Laced frames get their timestamps from the block's and DefaultDuration, so a gap in the
    /// input must end the lace, and a track without DefaultDuration must not lace at all.
    #[test]
    fn lacing_keeps_timestamp_gaps() {
        let mut a = TrackSpec::new(TrackKind::Audio, "A_PCM/INT/LIT");
        a.audio = Some((48000.0, 1, Some(16)));
        a.default_duration_ns = Some(10_000_000);
        let mut b = TrackSpec::new(TrackKind::Audio, "A_PCM/INT/LIT");
        b.audio = Some((48000.0, 1, Some(16)));
        let opts = MuxOptions { lacing: LacingMode::Xiph, ..Default::default() };
        let mut m = MkvWriter::new(Cursor::new(Vec::new()), vec![a, b], opts).unwrap();
        for ms in [0i64, 100, 110, 120, 200] {
            m.write_frame(0, ms * 1_000_000, true, &[ms as u8; 64], None).unwrap();
        }
        for ms in [300i64, 310] {
            m.write_frame(1, ms * 1_000_000, true, &[ms as u8; 64], None).unwrap();
        }
        let bytes = m.finish().unwrap().into_inner();
        let got: Vec<_> = Demuxer::from_slice(&bytes).unwrap().map(|p| p.unwrap()).map(|p| (p.track, p.pts, p.lace)).collect();
        assert_eq!(got, [(0, 0, 0), (0, 100, 0), (0, 110, 1), (0, 120, 2), (0, 200, 0), (1, 300, 0), (1, 310, 0)]);
    }

    #[test]
    fn lace_size_is_clamped_to_256_frames() {
        let mut a = TrackSpec::new(TrackKind::Audio, "A_PCM/INT/LIT");
        a.audio = Some((48000.0, 1, Some(16)));
        a.default_duration_ns = Some(1_000_000);
        for lacing in [LacingMode::Xiph, LacingMode::Ebml, LacingMode::FixedOrEbml] {
            let opts = MuxOptions { lacing, max_lace_frames: 300, ..Default::default() };
            let mut m = MkvWriter::new(Cursor::new(Vec::new()), vec![a.clone()], opts).unwrap();
            for i in 0..300i64 {
                m.write_frame(0, i * 1_000_000, true, &[i as u8; 4], None).unwrap();
            }
            let bytes = m.finish().unwrap().into_inner();
            let d = Demuxer::from_slice(&bytes).unwrap();
            assert!(d.file().warnings.is_empty(), "{:?}", d.file().warnings);
            let got: Vec<_> = d.map(|p| p.unwrap()).map(|p| (p.pts, p.lace, p.data)).collect();
            let want: Vec<_> = (0..300i64).map(|i| (i, (i % 256) as u16, vec![i as u8; 4])).collect();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn block_duration_rounds_to_the_nearest_tick() {
        let mut v = TrackSpec::new(TrackKind::Video, "V_MJPEG");
        v.video_size = Some((16, 16));
        let mut m = MkvWriter::new(Cursor::new(Vec::new()), vec![v], MuxOptions::default()).unwrap();
        m.write_frame(0, 0, true, &[0; 4], Some(41_700_000)).unwrap();
        let bytes = m.finish().unwrap().into_inner();
        assert_eq!(Demuxer::from_slice(&bytes).unwrap().next_packet().unwrap().unwrap().duration, 42);
    }

    #[test]
    fn audio_only_files_cue_each_track_once_per_cluster() {
        let mut a = TrackSpec::new(TrackKind::Audio, "A_PCM/INT/LIT");
        a.audio = Some((48000.0, 1, Some(16)));
        let opts = MuxOptions { cluster_duration_ns: 1, lacing: LacingMode::None, ..Default::default() };
        let mut m = MkvWriter::new(Cursor::new(Vec::new()), vec![a.clone(), a], opts).unwrap();
        for i in 0..4i64 {
            m.write_frame(0, i * 10, true, &[1; 4], None).unwrap();
            m.write_frame(1, i * 10, true, &[2; 4], None).unwrap();
            m.write_frame(0, i * 10 + 1, true, &[3; 4], None).unwrap();
        }
        // Clusters hold [t0, t1] then [t0] at each step: one cue per track per cluster.
        let mut seen: Vec<(u64, u64)> = m.cues.iter().map(|c| (c.1, c.2)).collect();
        assert_eq!(seen.len(), 12, "{:?}", m.cues);
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 12, "{:?}", m.cues);
    }
}
