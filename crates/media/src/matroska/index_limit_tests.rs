//! Limited indexing ([`OpenOptions::index_limit`]) and [`MkvFile::extend_index`], on files written
//! with [`MkvWriter`] (one Cluster per second, Cues at the end).

use std::cell::Cell;
use std::io::{Cursor, ErrorKind};

use crate::matroska::{ByteSource, MkvFile, MkvWriter, MuxOptions, OpenOptions, TrackKind, TrackSpec, open_with};

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
