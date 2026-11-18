//! Hostile-input hammer: truncation, bit flips and extreme field values must never panic or hang.

use super::*;
use crate::matroska::mux::{LacingMode, MkvWriter, MuxOptions, TrackSpec};
use crate::matroska::{MkvFile, TrackKind};
use std::io::Cursor;

fn muxed(lacing: LacingMode) -> Vec<u8> {
    let mut v = TrackSpec::new(TrackKind::Video, "V_MJPEG");
    v.video_size = Some((16, 16));
    v.default_duration_ns = Some(40_000_000);
    let mut a = TrackSpec::new(TrackKind::Audio, "A_PCM/INT/LIT");
    a.audio = Some((48000.0, 1, Some(16)));
    let mut s = TrackSpec::new(TrackKind::Subtitle, "S_TEXT/UTF8");
    s.language = Some("fre".into());
    let opts = MuxOptions { lacing, cluster_duration_ns: 100_000_000, ..Default::default() };
    let mut m = MkvWriter::new(Cursor::new(Vec::new()), vec![v, a, s], opts).unwrap();
    for i in 0..12i64 {
        m.write_frame(0, i * 40_000_000, i % 4 == 0, &vec![i as u8; 30 + i as usize], None).unwrap();
        for k in 0..3 {
            m.write_frame(1, (i * 40 + k * 10) * 1_000_000, true, &[k as u8; 48], None).unwrap();
        }
        if i % 5 == 0 {
            m.write_frame(2, i * 40_000_000, true, b"hello", Some(500_000_000)).unwrap();
        }
    }
    m.finish().unwrap().into_inner()
}

fn walk(f: &MkvFile, d: &[u8]) {
    for t in 0..f.tracks.len() {
        let n = f.tracks[t].samples.len();
        for k in [0, n / 2, n.saturating_sub(1), n, usize::MAX] {
            let _ = f.read_sample(d, t, k);
            let _ = f.read_sample_addition(d, t, k);
        }
        let _ = f.keyframe_before(t, i64::MIN);
        let _ = f.keyframe_before(t, i64::MAX);
    }
    let _ = f.duration_ns();
}

fn exercise(d: &[u8]) {
    for opts in
        [OpenOptions { index: true, verify_crc: true, ..Default::default() }, OpenOptions { index: false, verify_crc: false, ..Default::default() }]
    {
        if let Ok(f) = open_with(d, &opts) {
            walk(&f, d);
        }
        if let Ok(mut dm) = Demuxer::with_options(d, opts) {
            let mut n = 0;
            while let Ok(Some(_)) = dm.next_packet() {
                n += 1;
                if n > 100_000 {
                    break;
                }
            }
            for t in 0..dm.tracks().len() {
                let _ = dm.seek(t, 20_000_000);
                let _ = dm.seek(t, i64::MAX);
                let _ = dm.seek(t, i64::MIN);
                let _ = dm.keyframe_index(t);
            }
            let _ = dm.build_index();
        }
    }
}

#[test]
fn hostile_input_never_panics() {
    let seeds = [synthetic(), muxed(LacingMode::None), muxed(LacingMode::Xiph), muxed(LacingMode::Ebml), muxed(LacingMode::FixedOrEbml)];
    for (i, s) in seeds.iter().enumerate() {
        craft_media_testkit::hostile::hammer(&format!("matroska seed {i}"), s, 20_000, exercise);
    }
}
