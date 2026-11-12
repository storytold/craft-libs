//! Limited indexing ([`OpenOptions::index_limit`]) and [`MkvFile::extend_index`], on files written
//! with [`MkvWriter`] (one Cluster per second, Cues at the end).

use std::cell::Cell;
use std::io::{Cursor, ErrorKind};

use crate::matroska::{ByteSource, Demuxer, MkvFile, MkvWriter, MuxOptions, OpenOptions, TrackKind, TrackSpec, open_with};

/// A video track of `frames` 100 ms frames (every fifth a keyframe): ten frames per Cluster.
fn file(frames: i64) -> Vec<u8> {
    let mut v = TrackSpec::new(TrackKind::Video, "V_MJPEG");
    v.video_size = Some((16, 16));
    v.default_duration_ns = Some(100_000_000);
    let mut w = MkvWriter::new(Cursor::new(Vec::new()), vec![v], MuxOptions::default()).expect("writer");
    for k in 0..frames {
        w.write_frame(0, k * 100_000_000, k % 5 == 0, &[k as u8; 200], Some(100_000_000)).expect("frame");
    }
    w.finish().expect("finish").into_inner()
}

fn limited(bytes: &[u8], limit: Option<u64>) -> MkvFile {
    open_with(bytes, &OpenOptions { index_limit: limit, ..OpenOptions::default() }).expect("open")
}

fn unlimited() -> OpenOptions {
    OpenOptions::default()
}

/// The sample tables, cluster list and cues of two files are equal.
fn assert_same(a: &MkvFile, b: &MkvFile) {
    assert_eq!(a.tracks.len(), b.tracks.len());
    for (x, y) in a.tracks.iter().zip(&b.tracks) {
        assert_eq!(x.samples, y.samples);
        assert_eq!(x.sync, y.sync);
    }
    assert_eq!(a.clusters, b.clusters);
    assert_eq!(a.cues, b.cues);
    assert_eq!((a.indexed, a.index_end), (b.indexed, b.index_end));
}

#[test]
fn default_options_are_unchanged() {
    let bytes = file(45);
    let f = open_with(&bytes[..], &OpenOptions::default()).expect("open");
    assert!(f.indexed && f.index_end.is_none());
    assert_eq!(f.clusters.len(), 5);
    assert_eq!(f.tracks[0].samples.len(), 45);
    assert!(!f.cues.is_empty());
}

#[test]
fn limit_stops_before_a_cluster_that_does_not_fit() {
    let bytes = file(45);
    let full = limited(&bytes, None);
    let c = &full.clusters;
    // the limit lies in the middle of the third cluster: two are indexed, the third waits
    let f = limited(&bytes, Some(c[2].offset + 40));
    assert_eq!(f.clusters, c[..2]);
    assert_eq!(f.tracks[0].samples, full.tracks[0].samples[..20]);
    assert!(!f.indexed);
    assert_eq!(f.index_end, Some(c[2].offset));
    // Info and Tracks are there; the Cues at the end are not read yet
    assert_eq!(f.info.duration, full.info.duration);
    assert_eq!(f.duration_ns(), full.duration_ns());
    assert!(f.cues.is_empty());
    // a limit exactly at the end of a cluster keeps that cluster
    let g = limited(&bytes, Some(c[1].end));
    assert_eq!(g.clusters.len(), 2);
    assert_eq!(g.index_end, Some(c[1].end));
}

#[test]
fn limit_before_the_first_cluster_still_scans_it() {
    let bytes = file(45);
    let full = limited(&bytes, None);
    for limit in [0, 1, full.first_cluster.expect("cluster") / 2] {
        let f = limited(&bytes, Some(limit));
        assert_eq!(f.clusters.len(), 1, "limit {limit}");
        assert_eq!(f.tracks[0].samples.len(), 10);
        assert_eq!(f.index_end, Some(full.clusters[0].end));
        assert!(!f.indexed);
    }
}

#[test]
fn limit_past_the_end_is_a_full_index() {
    let bytes = file(45);
    let full = limited(&bytes, None);
    for limit in [bytes.len() as u64, bytes.len() as u64 + 1, u64::MAX] {
        assert_same(&limited(&bytes, Some(limit)), &full);
    }
}

#[test]
fn limit_without_an_index_is_ignored() {
    let bytes = file(45);
    let f = open_with(&bytes[..], &OpenOptions { index: false, verify_crc: false, index_limit: Some(0) }).expect("open");
    assert!(!f.indexed && f.index_end.is_none() && f.clusters.is_empty());
    assert!(!f.cues.is_empty(), "the SeekHead still leads to the Cues");
}

#[test]
fn extending_in_steps_equals_a_full_open() {
    let bytes = file(45);
    let full = limited(&bytes, None);
    let mut f = limited(&bytes, Some(0));
    let mut seen = vec![f.clusters.len()];
    for c in &full.clusters {
        f.extend_index(&bytes[..], &OpenOptions { index_limit: Some(c.end), ..unlimited() }).expect("extend");
        seen.push(f.clusters.len());
    }
    assert_eq!(seen, [1, 1, 2, 3, 4, 5]);
    // the Cues after the last cluster wait for the rest of the index
    assert!(!f.indexed && f.index_end.is_some() && f.cues.is_empty());
    f.extend_index(&bytes[..], &unlimited()).expect("complete");
    assert_same(&f, &full);
}

#[test]
fn extending_twice_with_the_same_limit_changes_nothing() {
    let bytes = file(45);
    let mut f = limited(&bytes, Some(0));
    let limit = f.clusters[0].end + 5_000;
    let opts = OpenOptions { index_limit: Some(limit), ..unlimited() };
    f.extend_index(&bytes[..], &opts).expect("extend");
    let (samples, end) = (f.tracks[0].samples.clone(), f.index_end);
    f.extend_index(&bytes[..], &opts).expect("extend again");
    assert_eq!((f.tracks[0].samples.clone(), f.index_end), (samples, end));
    // a lower limit than what is indexed is a no-op too
    f.extend_index(&bytes[..], &OpenOptions { index_limit: Some(0), ..unlimited() }).expect("extend lower");
    assert_eq!(f.index_end, end);
    // to the end, then once more on a complete index
    f.extend_index(&bytes[..], &unlimited()).expect("complete");
    assert!(f.indexed);
    f.extend_index(&bytes[..], &unlimited()).expect("complete again");
    assert_same(&f, &limited(&bytes, None));
}

/// A source that has loaded only a prefix: reads beyond it fail like the web build's pending reads.
struct Loaded<'a> {
    data: &'a [u8],
    loaded: Cell<u64>,
}

impl ByteSource for Loaded<'_> {
    fn len(&self) -> u64 {
        self.data.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        if offset + buf.len() as u64 > self.loaded.get() {
            return Err(std::io::Error::new(ErrorKind::WouldBlock, "not loaded yet"));
        }
        self.data.read_at(offset, buf)
    }
}

#[test]
fn a_growing_prefix_indexes_without_reading_ahead() {
    let bytes = file(45);
    let full = limited(&bytes, None);
    let src = Loaded { data: &bytes, loaded: Cell::new(0) };
    // nothing loaded: the open fails with WouldBlock instead of panicking
    let opts = |limit: u64| OpenOptions { index_limit: Some(limit), ..unlimited() };
    let err = open_with(&src, &opts(0)).expect_err("nothing loaded");
    assert!(err.to_string().contains("not loaded"), "{err}");
    // the head up to the end of the first cluster is enough to open, however much more is asked for
    src.loaded.set(full.clusters[0].end);
    let mut f = open_with(&src, &opts(src.loaded.get())).expect("first cluster loaded");
    assert_eq!(f.clusters.len(), 1);
    // each chunk that arrives indexes the clusters it holds completely, and nothing past them
    for (n, c) in full.clusters.iter().enumerate().skip(1) {
        src.loaded.set(c.end - 1);
        f.extend_index(&src, &opts(src.loaded.get())).expect("extend");
        assert_eq!(f.clusters.len(), n, "a cluster missing its last byte waits");
        src.loaded.set(c.end);
        f.extend_index(&src, &opts(src.loaded.get())).expect("extend");
        assert_eq!(f.clusters.len(), n + 1);
    }
    // the tail (Cues) arrives: the index completes
    src.loaded.set(bytes.len() as u64);
    f.extend_index(&src, &unlimited()).expect("complete");
    assert_same(&f, &full);
}

/// A source whose read number `fail_at` fails once with `WouldBlock`, whatever it asks for.
struct Flaky<'a> {
    data: &'a [u8],
    calls: Cell<usize>,
    fail_at: usize,
}

impl ByteSource for Flaky<'_> {
    fn len(&self) -> u64 {
        self.data.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == self.fail_at {
            return Err(std::io::Error::new(ErrorKind::WouldBlock, "not loaded yet"));
        }
        self.data.read_at(offset, buf)
    }
}

#[test]
fn a_failed_extension_resumes_without_duplicates() {
    // Tags between the clusters and Cues after them make the elements read twice visible
    let bytes = file(45);
    let full = limited(&bytes, None);
    let mut failed = 0;
    for fail_at in 1..40 {
        let mut f = limited(&bytes, Some(0));
        let src = Flaky { data: &bytes, calls: Cell::new(0), fail_at };
        // a read that fails anywhere in the scan leaves the file consistent: the retry finishes it
        if f.extend_index(&src, &unlimited()).is_err() {
            failed += 1;
            assert!(f.index_end.is_some() && !f.indexed, "fail_at {fail_at}");
            assert_eq!(f.clusters.len(), f.tracks[0].samples.len() / 10, "whole clusters only, fail_at {fail_at}");
            f.extend_index(&bytes[..], &unlimited()).expect("resume");
        }
        assert_same(&f, &full);
    }
    assert!(failed > 0, "the failing read was reached");
}

#[test]
fn truncated_files_never_panic() {
    let bytes = file(25);
    for len in 0..bytes.len() {
        for limit in [Some(0), Some(len as u64 / 2), None] {
            let cut = &bytes[..len];
            let Ok(mut f) = open_with(cut, &OpenOptions { index_limit: limit, ..unlimited() }) else { continue };
            let _ = f.extend_index(cut, &unlimited());
            let _ = f.extend_index(cut, &OpenOptions { index_limit: Some(1), ..unlimited() });
            assert!(f.index_end.is_none() || !f.indexed);
        }
    }
}

#[test]
fn demuxer_completes_a_limited_index_and_seeks() {
    let bytes = file(45);
    let full = limited(&bytes, None);
    let mut d = Demuxer::with_options(&bytes[..], OpenOptions { index_limit: Some(0), ..unlimited() }).expect("open");
    assert_eq!(d.file().clusters.len(), 1);
    // packets stream from the first Cluster on, whatever the index covers
    assert_eq!(d.next_packet().expect("packet").expect("some").pts, 0);
    let keys = d.keyframe_index(0).expect("keyframes");
    assert!(d.file().indexed);
    assert_eq!(keys.len(), full.tracks[0].sync.len());
    let at = d.seek(0, 3_500_000_000).expect("seek");
    assert_eq!(at.pts_ns, 3_500_000_000);
}

#[test]
fn a_tiny_last_block_needs_no_bytes_past_its_cluster() {
    let mut v = TrackSpec::new(TrackKind::Video, "V_MJPEG");
    v.video_size = Some((16, 16));
    let mut w = MkvWriter::new(Cursor::new(Vec::new()), vec![v], MuxOptions::default()).expect("writer");
    for k in 0..30 {
        w.write_frame(0, k * 100_000_000, true, &[k as u8], Some(100_000_000)).expect("frame");
    }
    let bytes = w.finish().expect("finish").into_inner();
    let full = limited(&bytes, None);
    // a 7-byte block ends each cluster: its header read must not reach into the next cluster
    let src = Loaded { data: &bytes, loaded: Cell::new(full.clusters[0].end) };
    let f = open_with(&src, &OpenOptions { index_limit: Some(src.loaded.get()), ..unlimited() }).expect("open");
    assert_eq!(f.clusters.len(), 1);
}
