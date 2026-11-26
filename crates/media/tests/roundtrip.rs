//! Property tests: write with our muxers, read back, compare sample tables and bytes.

use craft_media::isobmff::*;
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::io::{Cursor, Seek, SeekFrom, Write};

#[derive(Clone, Debug)]
struct S {
    size: usize,
    duration: u32,
    cto: i32,
    sync: bool,
}

fn sample_strategy() -> impl Strategy<Value = S> {
    (0usize..300, 1u32..3000, -200i32..2000, any::<bool>()).prop_map(|(size, duration, cto, sync)| S { size, duration, cto, sync })
}

fn payload(track: usize, i: usize, n: usize) -> Vec<u8> {
    (0..n).map(|k| (track * 31 + i * 7 + k) as u8).collect()
}

fn entry(kind: u8) -> SampleEntry {
    match kind {
        0 => SampleEntry::avc(AvcConfig::new(vec![vec![0x67, 0x64, 0, 0x28, 1, 2]], vec![vec![0x68, 3]], 4), 320, 240),
        1 => SampleEntry::prores(FourCc(*b"apcn"), 1920, 1080),
        2 => SampleEntry::aac(vec![0x11, 0x90], 2, 48000),
        _ => SampleEntry::jpeg(64, 64),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn progressive_roundtrip(
        tracks in prop::collection::vec((0u8..4, prop::collection::vec(sample_strategy(), 0..40)), 1..4),
        order in prop::collection::vec(0usize..4, 0..200),
        mov in any::<bool>(),
        fast in any::<bool>(),
        with_pcm in any::<bool>(),
        start in prop::option::of(0i64..500),
    ) {
        let brand = if mov { Brand::Mov } else { Brand::Mp4 };
        let mut w = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(brand)).unwrap();
        for (k, (kind, _)) in tracks.iter().enumerate() {
            let mut cfg = TrackConfig::new(entry(*kind), 1000 + k as u32 * 11);
            cfg.media_start = start;
            w.add_track(cfg).unwrap();
        }
        let pcm_track = if with_pcm {
            let p = PcmConfig { bits: 24, float: false, big_endian: false, signed: true, channels: 2, sample_rate: 48000.0 };
            Some(w.add_track(TrackConfig::new(SampleEntry::pcm(p), 48000)).unwrap())
        } else { None };
        // Interleave: follow `order`, then flush the rest.
        let mut next = vec![0usize; tracks.len()];
        let mut pcm_chunks = Vec::new();
        let emit = |w: &mut Mp4Writer<Cursor<Vec<u8>>>, t: usize, next: &mut Vec<usize>| {
            let i = next[t];
            let s = &tracks[t].1[i];
            let d = payload(t, i, s.size);
            w.write_sample(t, WriteSample { data: &d, duration: s.duration, composition_offset: s.cto, is_sync: s.sync }).unwrap();
            next[t] += 1;
        };
        for (step, &o) in order.iter().enumerate() {
            if let Some(p) = pcm_track && step % 5 == 0 {
                let d = payload(9, step, 6 * (1 + step % 7));
                w.write_sample(p, WriteSample { data: &d, duration: 0, composition_offset: 0, is_sync: true }).unwrap();
                pcm_chunks.push(d);
            }
            let t = o % tracks.len();
            if next[t] < tracks[t].1.len() {
                emit(&mut w, t, &mut next);
            }
        }
        for t in 0..tracks.len() {
            while next[t] < tracks[t].1.len() {
                emit(&mut w, t, &mut next);
            }
        }
        let out = if fast { w.finish_faststart() } else { w.finish() }.unwrap().into_inner();
        let f = open(out.as_slice()).unwrap();
        prop_assert_eq!(f.is_quicktime, mov);
        prop_assert_eq!(f.tracks.len(), tracks.len() + pcm_track.is_some() as usize);
        for (k, (kind, samples)) in tracks.iter().enumerate() {
            let t = &f.tracks[k];
            prop_assert_eq!(t.codec(), Some(&entry(*kind).codec));
            prop_assert_eq!(t.samples.len(), samples.len());
            let mut dts = 0i64;
            for (i, (s, r)) in samples.iter().zip(&t.samples).enumerate() {
                prop_assert_eq!(r.size as usize, s.size);
                prop_assert_eq!(r.duration, s.duration);
                prop_assert_eq!(r.dts, dts);
                prop_assert_eq!(r.pts, dts + s.cto as i64);
                prop_assert_eq!(r.is_sync, s.sync);
                prop_assert_eq!(f.read_sample(out.as_slice(), k, i).unwrap(), payload(k, i, s.size));
                dts += s.duration as i64;
            }
            prop_assert_eq!(t.duration, dts as u64);
            match start {
                Some(st) if !samples.is_empty() || st == 0 => prop_assert_eq!(t.edit_offset, -st),
                _ => {}
            }
        }
        if let Some(p) = pcm_track {
            let t = &f.tracks[p];
            let got: Vec<u8> = (0..t.samples.len()).flat_map(|i| f.read_sample(out.as_slice(), p, i).unwrap()).collect();
            prop_assert_eq!(got, pcm_chunks.concat());
            let frames: u64 = t.samples.iter().map(|s| s.duration as u64).sum();
            prop_assert_eq!(frames, pcm_chunks.iter().map(|c| c.len() as u64 / 6).sum::<u64>());
        }
    }

    #[test]
    fn fragmented_roundtrip(
        samples in prop::collection::vec(sample_strategy(), 1..60),
        cuts in prop::collection::vec(any::<bool>(), 60),
    ) {
        let mut w = FragmentedWriter::new(Vec::new(), WriterOptions::new(Brand::Mp4));
        let v = w.add_track(TrackConfig::new(entry(0), 90000)).unwrap();
        let a = w.add_track(TrackConfig::new(entry(2), 48000)).unwrap();
        for (i, s) in samples.iter().enumerate() {
            w.write_sample(v, WriteSample { data: &payload(0, i, s.size), duration: s.duration, composition_offset: s.cto, is_sync: s.sync }).unwrap();
            w.write_sample(a, WriteSample { data: &payload(1, i, s.size / 2), duration: 1024, composition_offset: 0, is_sync: true }).unwrap();
            if cuts[i] { w.flush_fragment().unwrap(); }
        }
        let out = w.finish().unwrap();
        let f = open(out.as_slice()).unwrap();
        prop_assert!(f.fragmented);
        let mut dts = 0i64;
        for (i, s) in samples.iter().enumerate() {
            let r = f.tracks[0].samples[i];
            prop_assert_eq!((r.size as usize, r.dts, r.pts, r.is_sync), (s.size, dts, dts + s.cto as i64, s.sync));
            prop_assert_eq!(f.read_sample(out.as_slice(), 0, i).unwrap(), payload(0, i, s.size));
            prop_assert_eq!(f.read_sample(out.as_slice(), 1, i).unwrap(), payload(1, i, s.size / 2));
            prop_assert_eq!(f.tracks[1].samples[i].dts, 1024 * i as i64);
            dts += s.duration as i64;
        }
    }
}

/// An output that keeps only writes under 1 MiB and reads everything else as zeros, so tests can
/// write files past 4 GiB from a reused zero buffer.
#[derive(Default)]
struct Sparse {
    pos: u64,
    len: u64,
    kept: BTreeMap<u64, Vec<u8>>,
}

impl Write for Sparse {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let end = self.pos + b.len() as u64;
        self.kept.retain(|&at, _| at < self.pos || at >= end);
        if b.len() < 1 << 20 {
            self.kept.insert(self.pos, b.to_vec());
        }
        self.pos += b.len() as u64;
        self.len = self.len.max(self.pos);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for Sparse {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::End(d) => self.len.checked_add_signed(d).unwrap(),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).unwrap(),
        };
        Ok(self.pos)
    }
}

impl ByteSource for Sparse {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        buf.fill(0);
        let end = offset + buf.len() as u64;
        for (&at, data) in &self.kept {
            let (lo, hi) = (at.max(offset), (at + data.len() as u64).min(end));
            if lo < hi {
                buf[(lo - offset) as usize..(hi - offset) as usize].copy_from_slice(&data[(lo - at) as usize..(hi - at) as usize]);
            }
        }
        Ok(())
    }
}

const MIB: usize = 1 << 20;

#[test]
fn writer_uses_co64_and_largesize_mdat_past_4_gib() {
    let zeros = vec![0u8; MIB];
    let mut w = Mp4Writer::new(Sparse::default(), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = w.add_track(TrackConfig::new(entry(3), 30)).unwrap();
    let n = 4 * 1024 + 1;
    for _ in 0..n {
        w.write_sample(t, WriteSample { data: &zeros, duration: 1, composition_offset: 0, is_sync: true }).unwrap();
    }
    w.write_sample(t, WriteSample { data: b"abc", duration: 1, composition_offset: 0, is_sync: true }).unwrap();
    let out = w.finish().unwrap();
    let f = open(&out).unwrap();
    let s = &f.tracks[0].samples;
    assert_eq!(s.len(), n + 1);
    assert!(s[n].offset > u64::from(u32::MAX));
    assert_eq!(f.read_sample(&out, 0, n).unwrap(), b"abc");
}
