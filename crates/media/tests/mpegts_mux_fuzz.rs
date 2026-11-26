//! Random programs, frames and timelines: the packet layer stays valid, and every frame reads back
//! with the demuxer, across random idle gaps and output swaps. Random bytes for every codec never
//! panic.
// Test code: a failed unwrap is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::mpegts_mux;

use craft_media::mpegts;
use craft_media::mpegts::mux::{Av1Config, Codec, Config, Error, Frame, MAX_PCR_DELAY, MAX_PCR_INTERVAL, Muxer, OpusMapping, OpusTrim, Stream};
use mpegts_mux::*;
use proptest::prelude::*;

/// One step of a plan: (stream, size, DTS step, PTS offset, key, idle time before it, cut before
/// it: 0 none, 1 a new segment, 2 a continuing swap).
type Step = (usize, usize, i64, i64, bool, Option<i64>, u8);

#[derive(Debug, Clone)]
struct Plan {
    pcr_interval: u32,
    pcr_own: bool,
    pcr_delay: u32,
    psi_interval: u32,
    psi_before_keyframes: bool,
    batch_packets: u16,
    hold_packets: bool,
    frames: Vec<Step>,
}

/// The streams of the read-back property, in the order of `Step`'s stream index.
const STREAMS: usize = 8;

fn plan() -> impl Strategy<Value = Plan> {
    let step = (0usize..STREAMS, 1usize..4000, 1i64..12_000, 0i64..9000, any::<bool>(), prop::option::weighted(0.2, 0i64..40_000), 0u8..3);
    (
        1u32..=MAX_PCR_INTERVAL,
        any::<bool>(),
        1u32..=MAX_PCR_DELAY,
        1u32..200_000,
        any::<bool>(),
        1u16..400,
        any::<bool>(),
        prop::collection::vec(step, 5..60),
    )
        .prop_map(|(pcr_interval, pcr_own, pcr_delay, psi_interval, psi_before_keyframes, batch_packets, hold_packets, frames)| Plan {
            pcr_interval,
            pcr_own,
            pcr_delay,
            psi_interval,
            psi_before_keyframes,
            batch_packets,
            hold_packets,
            frames,
        })
}

/// A frame of stream `s` and the bytes the demuxer gives back for it.
fn frame(s: usize, size: usize, key: bool, n: usize) -> (Vec<u8>, Vec<u8>) {
    let seed = n as u8;
    match s {
        0 => {
            let d = h264(key, false, size.max(30), seed);
            let e = with_aud(Codec::H264, &d);
            (d, e)
        }
        1 => {
            let mut d = vec![0xFC];
            fill(&mut d, size.min(1500), seed);
            let e = opus_unit(&d);
            (d, e)
        }
        2 => {
            let mut d = Vec::new();
            fill(&mut d, size.min(2000), seed);
            let len = d.len() + 7;
            let mut e = vec![0xFF, 0xF1, 0x50, 0x40 | (len >> 11) as u8, (len >> 3) as u8, ((len & 7) as u8) << 5 | 0x1F, 0xFC];
            e.extend_from_slice(&d);
            (d, e)
        }
        3 => {
            let d = hevc(key, size.max(40), seed);
            let e = with_aud(Codec::Hevc, &d);
            (d, e)
        }
        4 => (ac3(seed), ac3(seed)),
        5 => (eac3(seed), eac3(seed)),
        6 => (mp3(false, seed), mp3(false, seed)),
        _ => {
            let d = mpeg2(key, (n % 1024) as u16, size.max(40), seed);
            (d.clone(), d)
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn random_programs_read_back(p in plan()) {
        let streams = [
            Stream::new(0x100, Codec::H264),
            Stream::new(0x101, Codec::Opus { channels: 2 }),
            Stream::new(0x102, Codec::Aac { object_type: 2, sample_rate_index: 4, channels: 1 }),
            Stream::new(0x103, Codec::Hevc),
            Stream::new(0x104, Codec::Ac3),
            Stream::new(0x105, Codec::Eac3),
            Stream::new(0x106, Codec::MpegAudio { low_sample_rate: false }),
            Stream::new(0x107, Codec::Mpeg2Video),
        ];
        let cfg = Config {
            pcr_interval: p.pcr_interval,
            pcr_pid: p.pcr_own.then_some(0x1F0),
            pcr_delay: p.pcr_delay,
            psi_interval: p.psi_interval,
            psi_before_keyframes: p.psi_before_keyframes,
            batch_packets: p.batch_packets,
            hold_packets: p.hold_packets,
            ..Config::default()
        };
        let mut m = Muxer::new(Vec::new(), cfg, &streams).unwrap();
        let mut t = 100_000i64;
        let mut want: [Vec<(Vec<u8>, i64, i64)>; STREAMS] = Default::default();
        let mut segments = Vec::new();
        let mut restarted = false;
        for (n, &(s, size, step, offset, key, idle, cut)) in p.frames.iter().enumerate() {
            if let Some(idle) = idle {
                // an idle gap that ends no later than the next frame's DTS
                m.advance(t + idle.min(step)).unwrap();
            }
            t += step;
            match cut {
                1 => {
                    segments.push((true, m.replace_writer(Vec::new())));
                    restarted = true;
                }
                2 => segments.push((false, m.replace_writer_continuing(Vec::new()))),
                _ => {}
            }
            let (data, expect) = frame(s, size, key, n);
            let pts = if matches!(s, 0 | 3 | 7) { t + offset } else { t };
            m.write_frame(s, &Frame { data: &data, pts, dts: Some(t), key }).unwrap();
            want[s].push((expect, pts, t));
        }
        let pcr_pid = m.pcr_pid();
        m.flush().unwrap();
        let stats = m.stats();
        let last = m.finish().unwrap();
        // an output that follows `replace_writer` starts with the PAT
        let mut parts: Vec<Vec<u8>> = Vec::new();
        let mut fresh = false;
        for (restart, seg) in segments {
            prop_assert!(seg.error.is_none());
            prop_assert_eq!(seg.writer.len() as u64, seg.packets * 188);
            if fresh && !seg.writer.is_empty() {
                prop_assert_eq!((seg.writer[1] & 0x1F, seg.writer[2]), (0, 0));
            }
            fresh = restart;
            parts.push(seg.writer);
        }
        if fresh && !last.is_empty() {
            prop_assert_eq!((last[1] & 0x1F, last[2]), (0, 0));
        }
        parts.push(last);
        let ts = parts.concat();
        prop_assert_eq!(ts.len() as u64, stats.bytes());
        if stats.pcr_gaps == 0 && !restarted {
            check_packets(&ts, pcr_pid, i64::from(p.pcr_interval) * 300);
        }
        let file = mpegts::open(&ts).unwrap();
        prop_assert!(file.warnings.is_empty(), "{:?}", file.warnings);
        for (k, w) in want.iter().enumerate() {
            let units = &file.streams[k].units;
            prop_assert_eq!(units.len(), w.len(), "stream {}", k);
            for (n, (u, (bytes, pts, dts))) in units.iter().zip(w).enumerate() {
                prop_assert_eq!(u.pts, Some(*pts));
                prop_assert_eq!(u.dts.or(u.pts), Some(*dts));
                prop_assert_eq!(&file.read_unit(&ts, k, n).unwrap(), bytes);
            }
        }
    }

    #[test]
    fn random_bytes_never_panic(
        codec in 0usize..14,
        frames in prop::collection::vec((prop::collection::vec(any::<u8>(), 0..3000), any::<i64>(), prop::option::of(any::<i64>()), any::<bool>(), any::<u16>(), any::<u16>()), 1..30),
        interval in 0u32..20_000,
    ) {
        let av1 = Av1Config::from_av1c(&[0x81, 0x08, 0x0C, 0x00]).unwrap();
        let mapped = OpusMapping { channels: 6, family: 1, streams: 4, coupled: 2, mapping: [0, 4, 1, 2, 3, 5, 0, 0] };
        let codec = [
            Codec::H264,
            Codec::Hevc,
            Codec::Mpeg2Video,
            Codec::AacAdts,
            Codec::Aac { object_type: 2, sample_rate_index: 3, channels: 2 },
            Codec::Opus { channels: 2 },
            Codec::Ac3,
            Codec::MpegAudio { low_sample_rate: false },
            Codec::MpegAudio { low_sample_rate: true },
            Codec::Mpeg1Video,
            Codec::Mpeg4Part2,
            Codec::Av1(av1),
            Codec::Eac3,
            Codec::OpusMapped(mapped),
        ][codec];
        let cfg = Config { pcr_interval: interval, ..Config::default() };
        let Ok(mut m) = Muxer::new(Vec::new(), cfg, &[Stream::new(0x100, codec)]) else {
            prop_assert!(interval == 0 || interval > MAX_PCR_INTERVAL);
            return Ok(());
        };
        for (n, (data, pts, dts, key, start, end)) in frames.iter().enumerate() {
            let trim = if n % 3 == 0 { OpusTrim { start: *start, end: *end } } else { OpusTrim::default() };
            match m.write_frame_with_trim(0, &Frame { data, pts: *pts, dts: *dts, key: *key }, trim) {
                Ok(()) | Err(Error::BadFrame(_) | Error::EmptyFrame | Error::Timestamp(_) | Error::FrameTooLarge { .. }) => {}
                Err(e) => prop_assert!(false, "{e}"),
            }
            let _ = m.advance(*pts);
            if n % 7 == 6 {
                m.discontinuity();
            }
        }
        let ts = m.finish().unwrap();
        prop_assert_eq!(ts.len() % 188, 0);
        // the demuxer recognizes a transport stream by five sync bytes in a row
        if ts.len() >= 5 * 188 {
            prop_assert!(mpegts::open(&ts).is_ok());
        }
    }
}
