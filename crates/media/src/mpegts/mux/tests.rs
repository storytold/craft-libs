use super::*;

use std::cell::RefCell;
use std::rc::Rc;

const WIDE_OPUS: OpusMapping = OpusMapping { channels: 6, family: 1, streams: 4, coupled: 2, mapping: [0, 4, 1, 2, 3, 5, 0, 0] };

/// One packet's header fields.
#[derive(Debug, Clone, Copy)]
struct Pk {
    pid: u16,
    pusi: bool,
    cc: u8,
    payload: bool,
    rai: bool,
    disc: bool,
    pcr: Option<i64>,
}

fn walk(ts: &[u8]) -> Vec<Pk> {
    assert_eq!(ts.len() % PACKET_SIZE, 0);
    ts.chunks(PACKET_SIZE)
        .map(|p| {
            assert_eq!(p[0], 0x47);
            let afc = p[3] >> 4 & 3;
            let (mut rai, mut disc, mut pcr) = (false, false, None);
            if afc & 2 != 0 {
                let len = p[4] as usize;
                assert!(len <= 183);
                if afc == 2 {
                    assert_eq!(len, 183);
                }
                if len > 0 {
                    rai = p[5] & 0x40 != 0;
                    disc = p[5] & 0x80 != 0;
                    if p[5] & 0x10 != 0 {
                        let b = &p[6..12];
                        let base = (b[0] as i64) << 25 | (b[1] as i64) << 17 | (b[2] as i64) << 9 | (b[3] as i64) << 1 | (b[4] >> 7) as i64;
                        pcr = Some(base * 300 + (((b[4] & 1) as i64) << 8 | b[5] as i64));
                    }
                }
            }
            Pk { pid: ((p[1] & 0x1F) as u16) << 8 | p[2] as u16, pusi: p[1] & 0x40 != 0, cc: p[3] & 0x0F, payload: afc & 1 != 0, rai, disc, pcr }
        })
        .collect()
}

fn h264(key: bool) -> Vec<u8> {
    if key {
        vec![0, 0, 0, 1, 0x67, 0x42, 0, 0x1E, 0, 0, 0, 1, 0x68, 0xCE, 0, 0, 1, 0x65, 0x88, 0x84, 0x21]
    } else {
        vec![0, 0, 0, 1, 0x41, 0x9A, 0x11]
    }
}

fn mux(cfg: Config, streams: &[Stream], f: impl FnOnce(&mut Muxer<Vec<u8>>)) -> Vec<u8> {
    let mut m = Muxer::new(Vec::new(), cfg, streams).unwrap();
    f(&mut m);
    m.finish().unwrap()
}

#[test]
fn starts_with_pat_pmt_and_a_pcr_on_the_first_frame() {
    let ts = mux(Config::default(), &[Stream::new(0x100, Codec::H264)], |m| {
        m.write_frame(0, &Frame { data: &h264(true), pts: 90_000, dts: None, key: true }).unwrap();
    });
    let pk = walk(&ts);
    assert_eq!(pk.len(), 3);
    assert_eq!((pk[0].pid, pk[0].pusi), (0, true));
    assert_eq!((pk[1].pid, pk[1].pusi), (0x1000, true));
    assert_eq!((pk[2].pid, pk[2].pusi, pk[2].rai), (0x100, true, true));
    assert_eq!(pk[2].pcr, Some((90_000 - 63_000) * 300));
    // the payload ends the packet: the access unit delimiter, then the frame
    let frame = h264(true);
    let p = &ts[2 * PACKET_SIZE..];
    assert_eq!(&p[PACKET_SIZE - frame.len()..PACKET_SIZE], &frame[..]);
    assert_eq!(&p[PACKET_SIZE - frame.len() - 6 - 14..PACKET_SIZE - frame.len() - 6][..4], &[0, 0, 1, 0xE0]);
    assert_eq!(&p[PACKET_SIZE - frame.len() - 6..PACKET_SIZE - frame.len()], &[0, 0, 0, 1, 9, 0xF0]);
}

#[test]
fn every_stuffing_length_fills_the_packet() {
    // frames whose PES packet ends at every offset of a packet, with and without a PCR
    for n in 1..=400usize {
        let ts = mux(Config { pcr_pid: Some(0x200), ..Config::default() }, &[Stream::new(0x100, Codec::MpegAudio { low_sample_rate: false })], |m| {
            let mut data = vec![0u8; n];
            data[0] = 0xFF;
            if n > 1 {
                data[1] = 0xFB;
            }
            if n == 1 {
                assert!(matches!(m.write_frame(0, &Frame { data: &data, pts: 90_000, dts: None, key: true }), Err(Error::BadFrame(_))));
                return;
            }
            m.write_frame(0, &Frame { data: &data, pts: 90_000, dts: None, key: true }).unwrap();
            m.write_frame(0, &Frame { data: &data, pts: 92_000, dts: None, key: false }).unwrap();
        });
        if n == 1 {
            continue;
        }
        let pk = walk(&ts);
        let mut payload = 0;
        for (p, raw) in pk.iter().zip(ts.chunks(PACKET_SIZE)) {
            if p.pid == 0x100 {
                let start = if raw[3] & 0x20 != 0 { 5 + raw[4] as usize } else { 4 };
                payload += PACKET_SIZE - start;
            }
        }
        assert_eq!(payload, 2 * (14 + n), "{n}");
    }
}

#[test]
fn continuity_counters_count_payload_packets_per_pid() {
    let ts = mux(Config::default(), &[Stream::new(0x100, Codec::H264), Stream::new(0x101, Codec::Ac3)], |m| {
        let big = {
            let mut v = h264(true);
            v.resize(10_000, 0x55);
            v
        };
        for k in 0..40i64 {
            m.write_frame(0, &Frame { data: &big, pts: 90_000 + k * 3000, dts: None, key: k % 10 == 0 }).unwrap();
            m.write_frame(1, &Frame { data: &[0x0B, 0x77, 1, 2, 3], pts: 90_000 + k * 3000, dts: None, key: true }).unwrap();
        }
    });
    let mut last: std::collections::HashMap<u16, u8> = Default::default();
    for p in walk(&ts) {
        if let Some(&c) = last.get(&p.pid) {
            let want = if p.payload { (c + 1) & 0x0F } else { c };
            assert_eq!(p.cc, want, "PID {:#x}", p.pid);
        }
        last.insert(p.pid, p.cc);
    }
}

#[test]
fn pcr_never_lapses_past_the_interval() {
    for interval in [900u32, 3600, MAX_PCR_INTERVAL] {
        // video on 0x100 carries the PCR; a 2 s gap in the video is filled from the audio clock
        let ts =
            mux(Config { pcr_interval: interval, ..Config::default() }, &[Stream::new(0x100, Codec::H264), Stream::new(0x101, Codec::Ac3)], |m| {
                let mut t = 90_000i64;
                for k in 0..200i64 {
                    if !(50..110).contains(&k) {
                        m.write_frame(0, &Frame { data: &h264(k == 0), pts: t, dts: None, key: k == 0 }).unwrap();
                    }
                    m.write_frame(1, &Frame { data: &[0x0B, 0x77, 0], pts: t, dts: None, key: true }).unwrap();
                    t += 3000;
                }
            });
        let pcrs: Vec<i64> = walk(&ts).iter().filter(|p| p.pid == 0x100).filter_map(|p| p.pcr).collect();
        assert!(pcrs.len() > 1);
        for w in pcrs.windows(2) {
            assert!(w[1] > w[0], "PCR goes forward");
            assert!(w[1] - w[0] <= i64::from(interval) * 300, "interval {interval}: {} apart", w[1] - w[0]);
        }
    }
}

#[test]
fn pcr_on_a_pid_of_its_own_and_through_advance() {
    let mut m = Muxer::new(Vec::new(), Config { pcr_pid: Some(0x1FF), ..Config::default() }, &[Stream::new(0x101, Codec::Ac3)]).unwrap();
    assert_eq!(m.pcr_pid(), 0x1FF);
    m.advance(63_000).unwrap();
    m.advance(63_000 + 90_000).unwrap();
    m.write_frame(0, &Frame { data: &[0x0B, 0x77], pts: 63_000 + 90_000, dts: None, key: true }).unwrap();
    let st = m.stats();
    let ts = m.finish().unwrap();
    let pk = walk(&ts);
    let pcrs: Vec<i64> = pk.iter().filter(|p| p.pid == 0x1FF).map(|p| p.pcr.unwrap()).collect();
    assert_eq!(pcrs.first(), Some(&0));
    assert_eq!(pcrs.last(), Some(&(90_000 * 300)));
    assert!(pcrs.windows(2).all(|w| w[1] - w[0] <= 3600 * 300));
    assert!(pk.iter().filter(|p| p.pid == 0x1FF).all(|p| !p.payload && p.cc == 0));
    assert_eq!(st.pcrs as usize, pcrs.len());
    assert_eq!(pcrs.len(), 26, "one PCR, 24 to fill the second, one at its end");
    // at the start, then every 100 ms of the gap, the last one at its end
    assert_eq!(st.psi, 11);
    let mut last_psi = None;
    let mut pcr = 0;
    for p in &pk {
        if let Some(v) = p.pcr {
            pcr = v;
        }
        if p.pid == 0 {
            if let Some(l) = last_psi {
                assert!(pcr - l <= (9000 + 3600) * 300, "PAT {} after the last", pcr - l);
            }
            last_psi = Some(pcr);
        }
    }
    assert_eq!(st.bytes(), ts.len() as u64);
}

#[test]
fn a_pcr_packet_before_the_first_payload_keeps_the_counter_continuous() {
    // the audio frame comes first, so the video PID carries a PCR-only packet before its first
    // payload packet
    let ts = mux(Config::default(), &[Stream::new(0x100, Codec::H264), Stream::new(0x101, Codec::Ac3)], |m| {
        m.write_frame(1, &Frame { data: &[0x0B, 0x77], pts: 90_000, dts: None, key: true }).unwrap();
        m.write_frame(0, &Frame { data: &h264(true), pts: 90_000, dts: None, key: true }).unwrap();
    });
    let video: Vec<Pk> = walk(&ts).into_iter().filter(|p| p.pid == 0x100).collect();
    assert_eq!(video.len(), 2);
    assert!(!video[0].payload && video[0].pcr.is_some());
    assert!(video[1].payload);
    assert_eq!(video[1].cc, (video[0].cc + 1) & 0x0F);
}

#[test]
fn a_long_gap_is_closed_by_one_pcr() {
    let mut m = Muxer::new(Vec::new(), Config { pcr_interval: 1, ..Config::default() }, &[Stream::new(0x100, Codec::Ac3)]).unwrap();
    m.write_frame(0, &Frame { data: &[0x0B, 0x77], pts: 63_000, dts: None, key: true }).unwrap();
    m.write_frame(0, &Frame { data: &[0x0B, 0x77], pts: 63_000 + 90_000, dts: None, key: true }).unwrap();
    assert_eq!(m.stats().pcr_gaps, 1);
    let pcrs: Vec<i64> = walk(&m.finish().unwrap()).iter().filter_map(|p| p.pcr).collect();
    assert_eq!(pcrs.len() as u32, 2 + MAX_PCR_FILL);
    assert_eq!(pcrs.last(), Some(&(90_000 * 300)));
}

#[test]
fn psi_repeats_on_keyframes_and_new_writers() {
    let mut m = Muxer::new(Vec::new(), Config::default(), &[Stream::new(0x100, Codec::H264)]).unwrap();
    m.write_frame(0, &Frame { data: &h264(true), pts: 90_000, dts: None, key: true }).unwrap();
    m.write_frame(0, &Frame { data: &h264(false), pts: 93_000, dts: None, key: false }).unwrap();
    assert_eq!(m.stats().psi, 1);
    m.write_frame(0, &Frame { data: &h264(true), pts: 96_000, dts: None, key: true }).unwrap();
    assert_eq!(m.stats().psi, 2);
    let first = m.replace_writer(Vec::new()).writer;
    m.write_frame(0, &Frame { data: &h264(false), pts: 99_000, dts: None, key: false }).unwrap();
    let second = m.finish().unwrap();
    let pk = walk(&second);
    assert_eq!((pk[0].pid, pk[1].pid), (0, 0x1000), "a new segment starts with the PAT and PMT");
    let pk_first = walk(&first);
    let last_video = pk_first.iter().rev().find(|p| p.pid == 0x100).unwrap().cc;
    assert_eq!(pk.iter().find(|p| p.pid == 0x100).unwrap().cc, (last_video + 1) & 0x0F, "counters carry on");
}

#[test]
fn a_keyframe_after_a_gap_follows_the_tables_directly() {
    let ts = mux(Config::default(), &[Stream::new(0x100, Codec::H264)], |m| {
        m.write_frame(0, &Frame { data: &h264(true), pts: 90_000, dts: None, key: true }).unwrap();
        // the gap reaches past the table interval and several PCR intervals
        m.write_frame(0, &Frame { data: &h264(true), pts: 110_000, dts: None, key: true }).unwrap();
    });
    let pk = walk(&ts);
    let keys: Vec<usize> = pk.iter().enumerate().filter(|(_, p)| p.pid == 0x100 && p.rai).map(|(i, _)| i).collect();
    assert_eq!(keys.len(), 2);
    for k in keys {
        assert_eq!((pk[k - 2].pid, pk[k - 1].pid), (0, 0x1000), "the PAT and PMT right before the keyframe at packet {k}");
    }
    assert!(pk.iter().any(|p| p.pid == 0x100 && !p.payload), "PCR-only packets fill the gap");
    // a packet the caller writes between the tables and a keyframe brings the tables again
    let mut null = [0xFF; PACKET_SIZE];
    null[..4].copy_from_slice(&[0x47, 0x1F, 0xFF, 0x10]);
    let cfg = Config { pcr_interval: 9_000, psi_interval: 3_600, ..Config::default() };
    let ts = mux(cfg, &[Stream::new(0x100, Codec::H264)], |m| {
        m.advance(90_000).unwrap();
        // the tables fall due here and the PCR does not, so the tables are the last packets
        m.advance(93_600).unwrap();
        m.get_mut().extend_from_slice(&null);
        m.write_frame(0, &Frame { data: &h264(true), pts: 93_600, dts: None, key: true }).unwrap();
    });
    let pk = walk(&ts);
    let k = pk.iter().position(|p| p.pid == 0x100 && p.rai).unwrap();
    assert_eq!((pk[k - 3].pid, pk[k - 2].pid, pk[k - 1].pid), (0x1FFF, 0, 0x1000));
}

#[test]
fn tables_lost_with_a_failed_write_come_again_before_a_keyframe() {
    let cfg = Config { pcr_interval: 9_000, psi_interval: 3_600, ..Config::default() };
    // the second write fails: the one at the end of the second advance
    let mut m = Muxer::new(Flaky { fail_on: Some(2), calls: 0, data: Vec::new() }, cfg, &[Stream::new(0x100, Codec::H264)]).unwrap();
    m.advance(90_000).unwrap();
    // the tables fall due and are dropped with the failed batch
    assert!(matches!(m.advance(93_600), Err(Error::Io(_))));
    m.write_frame(0, &Frame { data: &h264(true), pts: 93_600, dts: None, key: true }).unwrap();
    let pk = walk(&m.finish().unwrap().data);
    let k = pk.iter().position(|p| p.pid == 0x100 && p.rai).unwrap();
    assert_eq!((pk[k - 2].pid, pk[k - 1].pid), (0, 0x1000));
}

#[test]
fn every_new_segment_carries_a_pcr() {
    for pcr_pid in [None, Some(0x1F0)] {
        let mut m = Muxer::new(Vec::new(), Config { pcr_pid, ..Config::default() }, &[Stream::new(0x100, Codec::H264)]).unwrap();
        m.write_frame(0, &Frame { data: &h264(true), pts: 90_000, dts: None, key: true }).unwrap();
        m.replace_writer(Vec::new());
        // a frame within the PCR interval
        m.write_frame(0, &Frame { data: &h264(false), pts: 90_001, dts: None, key: false }).unwrap();
        let segment = m.replace_writer(Vec::new()).writer;
        m.advance(91_000).unwrap();
        let third = m.finish().unwrap();
        let want = pcr_pid.unwrap_or(0x100);
        for (n, ts) in [segment, third].iter().enumerate() {
            let pk = walk(ts);
            assert_eq!((pk[0].pid, pk[1].pid), (0, 0x1000), "{pcr_pid:?} segment {n}");
            assert!(pk.iter().any(|p| p.pid == want && p.pcr.is_some()), "{pcr_pid:?} segment {n} has a PCR");
        }
    }
}

#[test]
fn a_segment_after_an_idle_gap_starts_with_the_tables() {
    let mut m = Muxer::new(Vec::new(), Config::default(), &[Stream::new(0x100, Codec::H264)]).unwrap();
    m.write_frame(0, &Frame { data: &h264(true), pts: 90_000, dts: None, key: true }).unwrap();
    m.replace_writer(Vec::new());
    m.advance(180_000).unwrap();
    let pk = walk(&m.finish().unwrap());
    assert_eq!((pk[0].pid, pk[1].pid), (0, 0x1000));
    assert!(pk.len() > 10, "the gap is filled with PCRs");
}

#[test]
fn short_table_intervals_hold_in_idle_gaps() {
    let mut m = Muxer::new(Vec::new(), Config { psi_interval: 900, ..Config::default() }, &[Stream::new(0x100, Codec::Ac3)]).unwrap();
    m.advance(63_000).unwrap();
    m.advance(63_000 + 90_000).unwrap();
    // every 900 ticks: at the start, 99 times inside the second, and at its end
    assert_eq!(m.stats().psi, 101);
    // a table interval of one tick fills at most MAX_PCR_FILL repeats per gap, and the PCRs still
    // fill the gap
    let mut m = Muxer::new(Vec::new(), Config { psi_interval: 1, ..Config::default() }, &[Stream::new(0x100, Codec::Ac3)]).unwrap();
    m.advance(63_000).unwrap();
    m.advance(63_000 + 90_000).unwrap();
    assert_eq!(m.stats().psi, 2 + u64::from(MAX_PCR_FILL));
    assert_eq!((m.stats().pcrs, m.stats().pcr_gaps), (26, 0));
}

#[test]
fn frames_behind_the_sent_clock_are_refused() {
    let mut m = Muxer::new(Vec::new(), Config::default(), &[Stream::new(0x100, Codec::H264), Stream::new(0x101, Codec::Ac3)]).unwrap();
    m.write_frame(0, &Frame { data: &h264(true), pts: 900_000, dts: None, key: true }).unwrap();
    // the clock is 900 000 - 63 000: audio up to the PCR delay behind the video is fine
    m.write_frame(1, &Frame { data: &[0x0B, 0x77], pts: 900_000 - 63_000, dts: None, key: true }).unwrap();
    let before = m.get_ref().len();
    assert!(matches!(m.write_frame(1, &Frame { data: &[0x0B, 0x77], pts: 900_000 - 63_001, dts: None, key: true }), Err(Error::Timestamp(_))));
    m.advance(2_000_000).unwrap();
    let after = m.get_ref().len();
    assert!(after > before);
    assert!(matches!(m.write_frame(0, &Frame { data: &h264(false), pts: 1_000_000, dts: None, key: false }), Err(Error::Timestamp(_))));
    assert_eq!(m.get_ref().len(), after, "nothing is written for a refused frame");
}

#[test]
fn timestamps_and_dts() {
    let ts = mux(Config::default(), &[Stream::new(0x100, Codec::H264)], |m| {
        m.write_frame(0, &Frame { data: &h264(true), pts: (1 << 33) + 6000, dts: Some((1 << 33) + 3000), key: true }).unwrap();
    });
    let p = &ts[2 * PACKET_SIZE..];
    let start = 5 + p[4] as usize;
    let h = &p[start..];
    assert_eq!(&h[..9], &[0, 0, 1, 0xE0, 0, (3 + 10 + 6 + h264(true).len()) as u8, 0x84, 0xC0, 10]);
    assert_eq!(h[9] >> 4, 3);
    assert_eq!(h[14] >> 4, 1);
}

#[test]
fn bad_frames_and_timestamps_write_nothing() {
    let mut m = Muxer::new(Vec::new(), Config::default(), &[Stream::new(0x100, Codec::H264), Stream::new(0x101, Codec::AacAdts)]).unwrap();
    fn f(data: &[u8], pts: i64, dts: Option<i64>) -> Frame<'_> {
        Frame { data, pts, dts, key: true }
    }
    assert!(matches!(m.write_frame(2, &f(&h264(true), 90_000, None)), Err(Error::NoSuchStream(2))));
    assert!(matches!(m.write_frame(0, &f(&[], 90_000, None)), Err(Error::EmptyFrame)));
    assert!(matches!(m.write_frame(0, &f(&h264(true), 90_000, Some(90_001))), Err(Error::Timestamp(_))));
    assert!(matches!(m.write_frame(0, &f(&[0x65, 0x88], 90_000, None)), Err(Error::BadFrame(_))));
    assert!(matches!(m.write_frame(1, &f(&[0xFF, 0xF1, 0, 0, 0, 0], 90_000, None)), Err(Error::BadFrame(_))));
    // ten ADTS frames of 7000 bytes: each is valid, together they pass the PES length
    let one = {
        let mut v = vec![0xFF, 0xF1, 0x4C, 0x80 | (7000 >> 11) as u8, (7000 >> 3) as u8, ((7000 & 7) as u8) << 5 | 0x1F, 0xFC];
        v.resize(7000, 0x11);
        v
    };
    let long_adts = one.repeat(10);
    assert!(matches!(m.write_frame(1, &f(&long_adts, 90_000, None)), Err(Error::FrameTooLarge { limit: 65_527, .. })));
    assert!(matches!(m.write_frame(0, &f(&h264(true), i64::MIN, None)), Err(Error::Timestamp(_))));
    assert!(m.get_ref().is_empty());
    m.write_frame(0, &f(&h264(true), 90_000, None)).unwrap();
    assert!(matches!(m.write_frame(0, &f(&h264(true), 80_000, None)), Err(Error::Timestamp(_))));
    // video has no PES length limit
    let mut big = h264(true);
    big.resize(200_000, 1);
    m.write_frame(0, &f(&big, 93_000, None)).unwrap();
}

#[test]
fn settings_are_checked() {
    let one = [Stream::new(0x100, Codec::H264)];
    let bad = |cfg: Config, streams: &[Stream]| matches!(Muxer::new(Vec::new(), cfg, streams), Err(Error::Config(_)));
    let err = |cfg: Config, streams: &[Stream]| Muxer::new(Vec::new(), cfg, streams).err();
    let d = Config::default();
    assert!(bad(d, &[]));
    assert!(bad(d, &[Stream::new(0x100, Codec::H264); MAX_STREAMS + 1]));
    assert!(bad(Config { program_number: 0, ..d }, &one));
    assert!(matches!(err(Config { pmt_pid: 0x0F, ..d }, &one), Some(Error::BadPid(0x0F))));
    assert!(matches!(err(Config { pmt_pid: 0x1FFF, ..d }, &one), Some(Error::BadPid(0x1FFF))));
    assert!(bad(Config { pcr_interval: 0, ..d }, &one));
    assert!(bad(Config { pcr_interval: MAX_PCR_INTERVAL + 1, ..d }, &one));
    assert!(bad(Config { psi_interval: 0, ..d }, &one));
    assert!(bad(Config { batch_packets: 0, ..d }, &one));
    assert!(bad(Config { batch_packets: MAX_BATCH_PACKETS + 1, ..d }, &one));
    assert!(matches!(err(Config { pcr_delay: 0, ..d }, &one), Some(Error::PcrDelay(0))));
    assert!(matches!(err(Config { pcr_delay: MAX_PCR_DELAY + 1, ..d }, &one), Some(Error::PcrDelay(90_001))));
    assert!(err(Config { pcr_delay: MAX_PCR_DELAY, ..d }, &one).is_none());
    assert!(err(Config { pcr_delay: 1, ..d }, &one).is_none());
    assert!(matches!(err(Config { pcr_pid: Some(0x1000), ..d }, &one), Some(Error::DuplicatePid(0x1000))));
    assert!(matches!(err(Config { pcr_pid: Some(0x2000), ..d }, &one), Some(Error::BadPid(0x2000))));
    assert!(matches!(err(d, &[Stream::new(0x0005, Codec::H264)]), Some(Error::BadPid(5))));
    assert!(matches!(err(d, &[Stream::new(0x1000, Codec::H264)]), Some(Error::DuplicatePid(0x1000))));
    assert!(matches!(err(d, &[Stream::new(0x100, Codec::H264), Stream::new(0x100, Codec::Ac3)]), Some(Error::DuplicatePid(0x100))));
    assert!(!bad(Config { pcr_pid: Some(0x1FFE), ..d }, &one));
}

#[test]
fn stream_settings_are_checked() {
    let bad = |cfg: Config, streams: &[Stream]| matches!(Muxer::new(Vec::new(), cfg, streams), Err(Error::Config(_)));
    let err = |cfg: Config, streams: &[Stream]| Muxer::new(Vec::new(), cfg, streams).err();
    let d = Config::default();
    // the widest program fits: 32 streams of the longest descriptors
    let wide: Vec<Stream> = (0..MAX_STREAMS as u16).map(|i| Stream::new(0x100 + i, Codec::OpusMapped(WIDE_OPUS)).with_language(*b"eng")).collect();
    assert!(err(d, &wide).is_none());
    assert!(bad(d, &[Stream::new(0x100, Codec::OpusMapped(OpusMapping { streams: 7, ..WIDE_OPUS }))]));
    assert!(bad(d, &[Stream::new(0x100, Codec::OpusMapped(OpusMapping { coupled: 5, ..WIDE_OPUS }))]));
    assert!(bad(d, &[Stream::new(0x100, Codec::OpusMapped(OpusMapping { mapping: [0, 1, 2, 3, 4, 10, 0, 0], ..WIDE_OPUS }))]));
    assert!(bad(d, &[Stream::new(0x100, Codec::OpusMapped(OpusMapping { family: 0, ..WIDE_OPUS }))]));
    assert!(bad(d, &[Stream::new(0x100, Codec::OpusMapped(OpusMapping { family: 2, ..WIDE_OPUS }))]), "a reserved family");
    assert!(!bad(d, &[Stream::new(0x100, Codec::OpusMapped(OpusMapping { family: 255, ..WIDE_OPUS }))]));
    let stereo = OpusMapping { channels: 2, family: 0, streams: 1, coupled: 1, mapping: [0, 1, 0, 0, 0, 0, 0, 0] };
    assert!(!bad(d, &[Stream::new(0x100, Codec::OpusMapped(stereo))]));
    assert!(bad(d, &[Stream::new(0x100, Codec::OpusMapped(OpusMapping { coupled: 255, ..stereo }))]), "no overflow");
    assert!(bad(d, &[Stream::new(0x100, Codec::OpusMapped(OpusMapping { mapping: [1, 0, 0, 0, 0, 0, 0, 0], ..stereo }))]), "swapped");
    assert!(!bad(d, &[Stream::new(0x100, Codec::OpusMapped(OpusMapping { mapping: [0, 1, 2, 3, 4, 255, 0, 0], ..WIDE_OPUS }))]));
    let av1 = Av1Config::from_av1c(&[0x81, 0x08, 0x0C, 0x00]).unwrap();
    assert!(!bad(d, &[Stream::new(0x100, Codec::Av1(av1))]));
    assert!(bad(d, &[Stream::new(0x100, Codec::Av1(Av1Config { seq_profile: 3, ..av1 }))]));
    assert!(Av1Config::from_av1c(&[0x01, 0x08, 0x0C, 0x00]).is_err());
    assert!(Av1Config::from_av1c(&[0x81, 0x08, 0x0C]).is_err());
    assert!(bad(d, &[Stream::new(0x100, Codec::Opus { channels: 0 })]));
    assert!(bad(d, &[Stream::new(0x100, Codec::Opus { channels: 9 })]));
    assert!(bad(d, &[Stream::new(0x100, Codec::Aac { object_type: 0, sample_rate_index: 3, channels: 2 })]));
    assert!(bad(d, &[Stream::new(0x100, Codec::Aac { object_type: 2, sample_rate_index: 13, channels: 2 })]));
    assert!(bad(d, &[Stream::new(0x100, Codec::Aac { object_type: 2, sample_rate_index: 3, channels: 0 })]));
    assert!(bad(d, &[Stream::new(0x100, Codec::Ac3).with_language(*b"e1g")]));
    assert!(bad(d, &[Stream::new(0x100, Codec::Ac3).with_language(*b"ENG")]));
    assert!(!bad(d, &[Stream::new(0x100, Codec::Ac3).with_language(*b"eng")]));
    // the first video stream carries the PCR by default
    let m = Muxer::new(Vec::new(), d, &[Stream::new(0x101, Codec::Ac3), Stream::new(0x100, Codec::H264)]).unwrap();
    assert_eq!(m.pcr_pid(), 0x100);
}

#[test]
fn wide_programs_split_the_pmt_across_packets() {
    let streams: Vec<Stream> = (0..MAX_STREAMS as u16).map(|i| Stream::new(0x100 + i, Codec::Opus { channels: 2 }).with_language(*b"eng")).collect();
    let ts = mux(Config::default(), &streams, |m| {
        m.write_frame(0, &Frame { data: &[0xFC, 1], pts: 90_000, dts: None, key: true }).unwrap();
    });
    let pk = walk(&ts);
    let pmt: Vec<_> = pk.iter().filter(|p| p.pid == 0x1000).collect();
    assert_eq!(pmt.len(), 4, "32 stream entries of 21 bytes");
    assert!(pmt[0].pusi && pmt[1..].iter().all(|p| !p.pusi));
    assert_eq!(pmt.iter().map(|p| p.cc).collect::<Vec<_>>(), [0, 1, 2, 3]);
    let mut section: Vec<u8> =
        ts.chunks(PACKET_SIZE).zip(&pk).filter(|(_, p)| p.pid == 0x1000).flat_map(|(raw, p)| raw[if p.pusi { 5 } else { 4 }..].to_vec()).collect();
    let len = 3 + ((section[1] as usize & 0x0F) << 8 | section[2] as usize);
    section.truncate(len);
    assert_eq!(psi::crc32(&section), 0);
}

/// An output that records the length of every write call.
#[derive(Clone, Default)]
struct Calls(Rc<RefCell<Vec<usize>>>);

impl Write for Calls {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().push(b.len());
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Video and audio for `seconds`, a 2 s GOP at 30 frames a second, in DTS order: (stream, frame).
fn av_timeline(seconds: i64) -> Vec<(usize, Vec<u8>, i64, bool)> {
    let mut v = Vec::new();
    for k in 0..seconds * 30 {
        let key = k % 60 == 0;
        let mut f = h264(key);
        f.resize(if key { 3000 } else { 700 }, 0x55);
        v.push((0, f, 90_000 + k * 3000, key));
    }
    for k in 0..seconds * 1000 / 32 {
        v.push((1, vec![0x0B, 0x77, 0, 0, 0x08, 0x40, 0x40, 1, 2, 3], 90_000 + k * 2880, true));
    }
    v.sort_by_key(|f| f.2);
    v
}

fn av_streams() -> [Stream; 2] {
    [Stream::new(0x100, Codec::H264), Stream::new(0x101, Codec::Ac3)]
}

#[test]
fn packets_go_out_in_batches() {
    for hold in [false, true] {
        let calls = Calls::default();
        let mut m = Muxer::new(calls.clone(), Config { hold_packets: hold, ..Config::default() }, &av_streams()).unwrap();
        for (s, f, t, key) in av_timeline(2) {
            m.write_frame(s, &Frame { data: &f, pts: t, dts: None, key }).unwrap();
        }
        let before = m.stats();
        m.flush().unwrap();
        let st = m.stats();
        // held packets are counted once the output takes them
        assert_eq!(before.packets == st.packets, !hold);
        m.finish().unwrap();
        let lens = calls.0.borrow();
        assert!(lens.iter().all(|&n| n % PACKET_SIZE == 0 && n > 0 && n <= 7 * PACKET_SIZE), "{lens:?}");
        assert_eq!(lens.iter().sum::<usize>() as u64, st.bytes());
        if hold {
            assert!(lens[..lens.len() - 1].iter().all(|&n| n == 7 * PACKET_SIZE), "held: full batches only");
        } else {
            assert!(lens.len() as u64 >= st.frames, "a write at the end of every call");
        }
    }
}

#[test]
fn held_packets_reach_the_old_writer_before_a_swap() {
    let cfg = Config { hold_packets: true, batch_packets: 348, ..Config::default() };
    let mut m = Muxer::new(Vec::new(), cfg, &av_streams()).unwrap();
    let mut segments = Vec::new();
    for (s, f, t, key) in av_timeline(10) {
        if s == 0 && key && t > 90_000 {
            segments.push(m.replace_writer(Vec::new()));
        }
        m.write_frame(s, &Frame { data: &f, pts: t, dts: None, key }).unwrap();
    }
    // held packets count once they reach the output
    let held = m.stats();
    let last = m.replace_writer(Vec::new());
    let st = m.stats();
    assert!(held.packets < st.packets);
    segments.push(last);
    assert_eq!(segments.len(), 5, "a 2 s GOP over 10 s");
    let mut packets = 0;
    let mut frames = 0;
    for (n, seg) in segments.iter().enumerate() {
        assert!(seg.error.is_none());
        assert_eq!(seg.writer.len() as u64, seg.packets * PACKET_SIZE as u64, "segment {n}");
        assert!(seg.starts_with_keyframe, "segment {n}");
        let pk = walk(&seg.writer);
        assert_eq!((pk[0].pid, pk[1].pid), (0, 0x1000), "segment {n}");
        assert!(pk[2].pid == 0x100 && pk[2].pcr.is_some() && pk[2].rai, "segment {n}: the keyframe with a PCR");
        assert_eq!(seg.first_pts, Some(90_000 + n as i64 * 180_000));
        assert!(seg.last_pts.unwrap() < 90_000 + (n as i64 + 1) * 180_000);
        packets += seg.packets;
        frames += seg.frames;
    }
    assert_eq!((packets, frames), (st.packets, st.frames));
    // a segment cut before a frame that is not a keyframe says so
    let mut m = Muxer::new(Vec::new(), Config::default(), &av_streams()).unwrap();
    m.write_frame(0, &Frame { data: &h264(true), pts: 90_000, dts: None, key: true }).unwrap();
    assert!(m.replace_writer(Vec::new()).starts_with_keyframe);
    m.write_frame(0, &Frame { data: &h264(false), pts: 93_000, dts: None, key: false }).unwrap();
    assert!(!m.replace_writer(Vec::new()).starts_with_keyframe);
}

#[test]
fn a_continuing_swap_leaves_the_packets_unchanged() {
    let one = mux(Config::default(), &av_streams(), |m| {
        for (s, f, t, key) in av_timeline(3) {
            m.write_frame(s, &Frame { data: &f, pts: t, dts: None, key }).unwrap();
        }
    });
    let mut m = Muxer::new(Vec::new(), Config::default(), &av_streams()).unwrap();
    let mut parts = Vec::new();
    for (n, (s, f, t, key)) in av_timeline(3).into_iter().enumerate() {
        if n % 7 == 6 {
            parts.push(m.replace_writer_continuing(Vec::new()).writer);
        }
        m.write_frame(s, &Frame { data: &f, pts: t, dts: None, key }).unwrap();
    }
    parts.push(m.finish().unwrap());
    assert!(parts.len() > 10);
    assert_eq!(parts.concat(), one);
}

#[test]
fn a_discontinuity_starts_a_new_time_base() {
    for pcr_pid in [None, Some(0x1F0)] {
        let mut m = Muxer::new(Vec::new(), Config { pcr_pid, ..Config::default() }, &av_streams()).unwrap();
        for (s, f, t, key) in av_timeline(2) {
            m.write_frame(s, &Frame { data: &f, pts: t, dts: None, key }).unwrap();
        }
        let before = m.stats().packets as usize;
        m.discontinuity();
        // the encoder restarts at a smaller timestamp, the audio first this time
        m.write_frame(1, &Frame { data: &[0x0B, 0x77, 9], pts: 100_000, dts: None, key: true }).unwrap();
        m.write_frame(0, &Frame { data: &h264(true), pts: 100_000, dts: None, key: true }).unwrap();
        m.write_frame(0, &Frame { data: &h264(false), pts: 103_000, dts: None, key: false }).unwrap();
        let ts = m.finish().unwrap();
        let pk = walk(&ts);
        assert!(pk[..before].iter().all(|p| !p.disc));
        let after = &pk[before..];
        assert_eq!((after[0].pid, after[1].pid), (0, 0x1000), "{pcr_pid:?}: the tables first");
        let pcr_on = pcr_pid.unwrap_or(0x100);
        let first_pcr = after.iter().find(|p| p.pcr.is_some()).unwrap();
        assert_eq!((first_pcr.pid, first_pcr.disc, first_pcr.pcr), (pcr_on, true, Some((100_000 - 63_000) * 300)), "{pcr_pid:?}");
        for pid in [0x100, 0x101] {
            let first = after.iter().find(|p| p.pid == pid).unwrap();
            assert!(first.disc, "{pcr_pid:?}: PID {pid:#x}");
            assert_eq!(after.iter().filter(|p| p.pid == pid && p.disc).count(), 1, "{pcr_pid:?}: once on PID {pid:#x}");
        }
    }
}

#[test]
fn the_clock_after_advance_is_on_the_dts_scale() {
    let mut m = Muxer::new(Vec::new(), Config::default(), &av_streams()).unwrap();
    m.advance(900_000).unwrap();
    let pcrs: Vec<i64> = walk(m.get_ref()).iter().filter_map(|p| p.pcr).collect();
    assert_eq!(pcrs, [(900_000 - 63_000) * 300]);
    let before = m.get_ref().len();
    assert!(matches!(m.write_frame(1, &Frame { data: &[0x0B, 0x77], pts: 900_000 - 63_001, dts: None, key: true }), Err(Error::Timestamp(_))));
    assert_eq!(m.get_ref().len(), before);
    m.write_frame(0, &Frame { data: &h264(true), pts: 900_000 - 63_000, dts: None, key: true }).unwrap();
}

#[test]
fn streams_may_skew_by_the_pcr_delay() {
    let mut m = Muxer::new(Vec::new(), Config::default(), &av_streams()).unwrap();
    // video 500 ms ahead of the audio
    for k in 0..30i64 {
        m.write_frame(0, &Frame { data: &h264(k == 0), pts: 135_000 + k * 3000, dts: None, key: k == 0 }).unwrap();
        m.write_frame(1, &Frame { data: &[0x0B, 0x77], pts: 90_000 + k * 3000, dts: None, key: true }).unwrap();
    }
    // 800 ms behind the video is refused
    let before = m.get_ref().len();
    m.write_frame(0, &Frame { data: &h264(false), pts: 900_000, dts: None, key: false }).unwrap();
    let mid = m.get_ref().len();
    assert!(mid > before);
    assert!(matches!(m.write_frame(1, &Frame { data: &[0x0B, 0x77], pts: 900_000 - 72_000, dts: None, key: true }), Err(Error::Timestamp(_))));
    assert_eq!(m.get_ref().len(), mid);
}

#[test]
fn video_decoding_times_must_increase() {
    let mut m = Muxer::new(Vec::new(), Config::default(), &av_streams()).unwrap();
    m.write_frame(0, &Frame { data: &h264(true), pts: 93_000, dts: Some(90_000), key: true }).unwrap();
    let before = m.get_ref().len();
    assert!(matches!(m.write_frame(0, &Frame { data: &h264(false), pts: 96_000, dts: Some(90_000), key: false }), Err(Error::Timestamp(_))));
    assert_eq!(m.get_ref().len(), before);
    // audio frames may share a time
    m.write_frame(1, &Frame { data: &[0x0B, 0x77], pts: 90_000, dts: None, key: true }).unwrap();
    m.write_frame(1, &Frame { data: &[0x0B, 0x77], pts: 90_000, dts: None, key: true }).unwrap();
}

#[test]
fn video_frames_past_the_reader_limit_are_refused() {
    let mut m = Muxer::new(Vec::new(), Config::default(), &[Stream::new(0x100, Codec::H264)]).unwrap();
    // zeroed pages are not touched before the size check
    let mut big = vec![0u8; MAX_VIDEO_FRAME + 1];
    big[..5].copy_from_slice(&[0, 0, 0, 1, 0x65]);
    assert!(matches!(
        m.write_frame(0, &Frame { data: &big, pts: 90_000, dts: None, key: true }),
        Err(Error::FrameTooLarge { limit: MAX_VIDEO_FRAME, .. })
    ));
    assert_eq!(m.stats(), Stats::default());
    assert!(m.get_ref().is_empty());
    // an AV1 frame under the limit whose escapes take it over
    let av1 = Av1Config::from_av1c(&[0x81, 0x08, 0x0C, 0x00]).unwrap();
    let mut m = Muxer::new(Vec::new(), Config::default(), &[Stream::new(0x100, Codec::Av1(av1))]).unwrap();
    let mut tu = vec![0x0A, 0x05, 0x00, 0x00, 0x00, 0x02, 0x7F, 0x32];
    let payload = MAX_VIDEO_FRAME - 100;
    let mut n = payload;
    while n >= 0x80 {
        tu.push((n & 0x7F) as u8 | 0x80);
        n >>= 7;
    }
    tu.push(n as u8);
    tu.resize(tu.len() + payload, 0);
    *tu.last_mut().unwrap() = 1;
    assert!(tu.len() <= MAX_VIDEO_FRAME);
    assert!(matches!(
        m.write_frame(0, &Frame { data: &tu, pts: 90_000, dts: None, key: true }),
        Err(Error::FrameTooLarge { limit: MAX_VIDEO_FRAME, size }) if size > MAX_VIDEO_FRAME
    ));
    assert!(m.get_ref().is_empty());
}

#[test]
fn audio_specific_configs_become_adts_fields() {
    let asc = |b: [u8; 2]| Codec::aac_from_asc(&b);
    assert_eq!(asc([0x11, 0x90]).unwrap(), Codec::Aac { object_type: 2, sample_rate_index: 3, channels: 2 });
    assert_eq!(asc([0x12, 0x08]).unwrap(), Codec::Aac { object_type: 2, sample_rate_index: 4, channels: 1 });
    assert_eq!(asc([0x11, 0xB0]).unwrap(), Codec::Aac { object_type: 2, sample_rate_index: 3, channels: 6 });
    assert_eq!(asc([0x11, 0xB8]).unwrap(), Codec::Aac { object_type: 2, sample_rate_index: 3, channels: 7 });
    assert!(asc([0x17, 0x80]).is_err(), "explicit sampling frequency");
    assert!(asc([0x29, 0x90]).is_err(), "SBR");
    assert!(asc([0x11, 0x80]).is_err(), "channel configuration 0");
    assert!(asc([0x11, 0x94]).is_err(), "frameLengthFlag 1");
    assert!(Codec::aac_from_asc(&[0x11]).is_err());
}

#[test]
fn opus_trims_follow_the_draft_rules() {
    let opus = [Stream::new(0x100, Codec::Opus { channels: 2 }), Stream::new(0x101, Codec::Ac3)];
    let mut m = Muxer::new(Vec::new(), Config::default(), &opus).unwrap();
    // 8 CELT frames of 20 ms: 7680 samples
    let au = [0xFF, 8, 0x11, 0x22];
    let f = |k: i64| Frame { data: &au, pts: 90_000 + k * 14_400, dts: None, key: true };
    assert!(matches!(m.write_frame_with_trim(1, &f(0), OpusTrim { start: 1, end: 0 }), Err(Error::BadFrame(_))), "not Opus");
    assert!(matches!(m.write_frame_with_trim(0, &f(0), OpusTrim { start: 7681, end: 0 }), Err(Error::BadFrame(_))), "longer than the unit");
    assert!(matches!(m.write_frame_with_trim(0, &f(0), OpusTrim { start: 8192, end: 0 }), Err(Error::BadFrame(_))), "13 bits");
    assert!(m.get_ref().is_empty());
    // whole units may be discarded at the start, 65 535 samples in all
    for k in 0..8 {
        m.write_frame_with_trim(0, &f(k), OpusTrim { start: 7680, end: 0 }).unwrap();
    }
    assert!(matches!(m.write_frame_with_trim(0, &f(8), OpusTrim { start: 7680, end: 0 }), Err(Error::BadFrame(_))));
    m.write_frame_with_trim(0, &f(8), OpusTrim { start: 312, end: 0 }).unwrap();
    // the lead-in is over
    assert!(matches!(m.write_frame_with_trim(0, &f(9), OpusTrim { start: 1, end: 0 }), Err(Error::BadFrame(_))));
    m.write_frame(0, &f(9)).unwrap();
    m.write_frame_with_trim(0, &f(10), OpusTrim { start: 0, end: 100 }).unwrap();
    // a restarted encoder trims its own start again
    m.discontinuity();
    m.write_frame_with_trim(0, &f(0), OpusTrim { start: 312, end: 0 }).unwrap();
    m.write_frame_with_trim(0, &f(1), OpusTrim { start: 0, end: 100 }).unwrap();
    let ts = m.finish().unwrap();
    // the last unit's control header: end trim flag, size 4, end_trim 100
    let last = ts.chunks(PACKET_SIZE).rev().find(|p| p[1] & 0x40 != 0 && (u16::from(p[1] & 0x1F) << 8 | u16::from(p[2])) == 0x100).unwrap();
    let pes = &last[5 + last[4] as usize..];
    assert_eq!(&pes[14..19], &[0x7F, 0xE8, 4, 0, 100]);
}

/// Each stream PID's PES packets: (random access indicator of the first packet, bytes).
fn pes_of(ts: &[u8], pids: &[u16]) -> std::collections::HashMap<u16, Vec<(bool, Vec<u8>)>> {
    let mut out: std::collections::HashMap<u16, Vec<(bool, Vec<u8>)>> = Default::default();
    for (raw, p) in ts.chunks(PACKET_SIZE).zip(walk(ts)) {
        if !pids.contains(&p.pid) || !p.payload {
            continue;
        }
        let start = if raw[3] & 0x20 != 0 { 5 + raw[4] as usize } else { 4 };
        let list = out.entry(p.pid).or_default();
        if p.pusi {
            list.push((p.rai, Vec::new()));
        }
        list.last_mut().unwrap().1.extend_from_slice(&raw[start..]);
    }
    out
}

/// Reads a timestamp, checking its prefix and marker bits.
fn read_ts(b: &[u8], prefix: u8) -> i64 {
    assert_eq!(b[0] >> 4, prefix);
    assert_eq!([b[0] & 1, b[2] & 1, b[4] & 1], [1, 1, 1], "marker bits");
    (((b[0] >> 1) & 7) as i64) << 30 | (b[1] as i64) << 22 | ((b[2] >> 1) as i64) << 15 | (b[3] as i64) << 7 | (b[4] >> 1) as i64
}

#[test]
fn pes_headers_hold_their_fixed_bits_lengths_and_keyframes() {
    let streams = [
        Stream::new(0x100, Codec::H264),
        Stream::new(0x101, Codec::Ac3),
        Stream::new(0x102, Codec::Aac { object_type: 2, sample_rate_index: 3, channels: 2 }),
    ];
    let mut m = Muxer::new(Vec::new(), Config::default(), &streams).unwrap();
    let mut frames: [Vec<(bool, i64, i64)>; 3] = Default::default();
    let mut outputs = Vec::new();
    for k in 0..120i64 {
        if k % 40 == 0 && k > 0 {
            outputs.push(m.replace_writer(Vec::new()).writer);
        }
        let key = k % 20 == 0;
        let mut v = h264(key);
        v.resize(if key { 30_000 } else { 70_000 + (k as usize * 997) % 9000 }, 0x55);
        let dts = 90_000 + k * 3000;
        let pts = dts + if key { 0 } else { 6000 };
        m.write_frame(0, &Frame { data: &v, pts, dts: Some(dts), key }).unwrap();
        frames[0].push((key, pts, dts));
        m.write_frame(1, &Frame { data: &[0x0B, 0x77, 1, 2, 3], pts: dts, dts: None, key: true }).unwrap();
        frames[1].push((true, dts, dts));
        m.write_frame(2, &Frame { data: &vec![7; 1 + (k as usize * 31) % 1500], pts: dts, dts: None, key: true }).unwrap();
        frames[2].push((true, dts, dts));
    }
    outputs.push(m.finish().unwrap());
    let mut seen = [0usize; 3];
    for ts in &outputs {
        for (pid, list) in pes_of(ts, &[0x100, 0x101, 0x102]) {
            let s = usize::from(pid - 0x100);
            for (rai, pes) in list {
                let (key, pts, dts) = frames[s][seen[s]];
                seen[s] += 1;
                assert_eq!(&pes[..3], &[0, 0, 1]);
                assert_eq!(pes[3], streams[s].codec.stream_id());
                assert_eq!(pes[6], 0x84, "'10', data_alignment_indicator");
                let length = usize::from(pes[4]) << 8 | usize::from(pes[5]);
                if s == 0 && pes.len() - 6 > 0xFFFF {
                    assert_eq!(length, 0);
                } else {
                    assert_eq!(length, pes.len() - 6, "PID {pid:#x}");
                }
                assert_eq!(rai, key, "PID {pid:#x}");
                if pts == dts {
                    assert_eq!((pes[7], pes[8]), (0x80, 5));
                    assert_eq!(read_ts(&pes[9..14], 2), pts);
                } else {
                    assert_eq!((pes[7], pes[8]), (0xC0, 10));
                    assert_eq!((read_ts(&pes[9..14], 3), read_ts(&pes[14..19], 1)), (pts, dts));
                }
            }
        }
    }
    assert_eq!(seen, [120, 120, 120]);
}

/// An output that fails on one write call.
struct Flaky {
    fail_on: Option<usize>,
    calls: usize,
    data: Vec<u8>,
}

impl Write for Flaky {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if Some(self.calls) == self.fail_on {
            return Err(io::Error::other("upload failed"));
        }
        self.data.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_new_writer_recovers_from_an_io_error() {
    let mut m = Muxer::new(Flaky { fail_on: Some(3), calls: 0, data: Vec::new() }, Config::default(), &av_streams()).unwrap();
    let mut failed = false;
    let mut recovered = None;
    for (s, f, t, key) in av_timeline(2) {
        match m.write_frame(s, &Frame { data: &f, pts: t, dts: None, key }) {
            Ok(()) => {}
            Err(Error::Io(_)) if !failed => {
                failed = true;
                let old = m.replace_writer(Flaky { fail_on: None, calls: 0, data: Vec::new() });
                assert_eq!(old.writer.data.len() % PACKET_SIZE, 0);
                recovered = Some(m.stats().packets);
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert!(failed && recovered.is_some());
    let ts = m.finish().unwrap().data;
    let pk = walk(&ts);
    assert_eq!((pk[0].pid, pk[1].pid), (0, 0x1000));
    assert!(pk.iter().take(4).any(|p| p.pcr.is_some()), "a PCR right after the tables");
    // counters run on within the new output
    let mut last: std::collections::HashMap<u16, u8> = Default::default();
    for p in &pk {
        if let Some(&c) = last.get(&p.pid) {
            assert_eq!(p.cc, if p.payload { (c + 1) & 0x0F } else { c });
        }
        last.insert(p.pid, p.cc);
    }
}

/// An output that takes `take` bytes in all and then fails.
struct Short {
    take: usize,
    data: Vec<u8>,
}

impl Write for Short {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self.data.len() >= self.take {
            return Err(io::Error::other("disk full"));
        }
        // at most 100 bytes a call, so a batch goes out in several calls
        let n = b.len().min(100).min(self.take - self.data.len());
        self.data.extend_from_slice(&b[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn packet_counts_hold_only_what_the_output_took() {
    let take = 5 * PACKET_SIZE + 50;
    let mut m = Muxer::new(Short { take, data: Vec::new() }, Config::default(), &av_streams()).unwrap();
    let mut failed = false;
    for (s, f, t, key) in av_timeline(1) {
        if m.write_frame(s, &Frame { data: &f, pts: t, dts: None, key }).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert_eq!(m.stats().packets, 5);
    let seg = m.replace_writer(Short { take: usize::MAX, data: Vec::new() });
    assert_eq!((seg.packets, seg.writer.data.len()), (5, take));
    // the new output takes a partial write in several calls and counts every packet
    m.write_frame(0, &Frame { data: &h264(true), pts: 200_000, dts: None, key: true }).unwrap();
    let st = m.stats();
    let seg = m.replace_writer(Short { take: 0, data: Vec::new() });
    assert_eq!(seg.writer.data.len() as u64, seg.packets * PACKET_SIZE as u64);
    assert_eq!(st.packets, 5 + seg.packets);
}
