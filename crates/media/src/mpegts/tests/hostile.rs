//! Hostile-input hammer: truncation, bit flips and extreme field values must never panic or hang.

use super::*;

fn exercise(d: &[u8]) {
    let data = d.to_vec();
    let _ = sniff(d);
    let Ok(f) = open(&data) else { return };
    for (s, st) in f.streams.iter().enumerate() {
        let _ = (st.kind(), st.pts_range(), st.codec.name());
        let n = st.units.len();
        for i in [0, n / 2, n.saturating_sub(1), n, usize::MAX] {
            let _ = f.read_unit(&data, s, i);
        }
    }
    let _ = f.find(Kind::Video, |_| true);
}

fn ts_seed(m2ts: bool) -> Vec<u8> {
    let mut mux = TsMux::new(m2ts);
    mux.psi(&[(0x02, 0x1011), (0x0F, 0x1100), (0x1B, 0x1200), (0x81, 0x1300), (0x24, 0x1400), (0x03, 0x1500)]);
    for k in 0..6u16 {
        mux.pes(
            0x1011,
            &pes(0xE0, Some(3600 * k as i64), Some(3600 * k as i64 - 7200), &mpeg2_frame(if k == 0 { 1 } else { 3 }, k, k % 2 == 1, 400), false),
        );
        mux.pes(0x1100, &pes(0xC0, Some(1920 * k as i64), None, &adts_frame(k as u8), true));
        mux.pes(0x1200, &pes(0xE1, Some(3600 * k as i64), None, &[0, 0, 0, 1, 0x09, 0xF0, 0, 0, 1, 0x65, 0x88, 1, 2, 3], false));
        let mut ac3 = vec![0x0B, 0x77, 0, 0, 30, 0x40];
        ac3.extend(std::iter::repeat_n(k as u8, 1792 - 6));
        mux.pes(0x1300, &pes(0xBD, Some(2880 * k as i64), None, &ac3, true));
        mux.pes(0x1400, &pes(0xE2, Some(3600 * k as i64), None, &[0, 0, 0, 1, 0x46, 0x01, 0x50, 0, 0, 0, 1, 0x26, 0x01, 0xAF, 1, 2], false));
        mux.pes(0x1500, &pes(0xC1, Some(2880 * k as i64), None, &mp2_frame(k as u8), true));
    }
    mux.out
}

fn ps_seed() -> Vec<u8> {
    let frames: Vec<Vec<u8>> = (0..5).map(|k| mpeg2_frame(if k == 0 { 1 } else { 2 }, k as u16, k == 3, 900 + 100 * k)).collect();
    let es: Vec<u8> = frames.concat();
    let mut out = Vec::new();
    let mut a = 0u8;
    for (i, chunk) in es.chunks(700).enumerate() {
        out.extend([0, 0, 1, 0xBA, 0x44, 0, 4, 0, 4, 1, 1, 0x89, 0xC3, 0xF8]);
        out.extend(pes(0xE0, Some(3600 * i as i64), None, chunk, true));
        let mut ac3 = vec![0x80, 1, 0, 1, 0x0B, 0x77, 0, 0, 30, 0x40];
        ac3.extend(std::iter::repeat_n(a, 1792 - 6));
        a += 1;
        out.extend(pes(0xBD, Some(1000 + 2880 * a as i64), None, &ac3, true));
        // LPCM in private stream 1, system header and PSM-ish noise between packs
        out.extend([0, 0, 1, 0xBB, 0, 6, 0x80, 0, 1, 4, 0xE1, 0xFF]);
        out.extend(pes(0xBD, Some(5000 + 100 * i as i64), None, &[0xA0, 1, 0, 4, 0, 0x01, 0x81, 1, 2, 3, 4, 5, 6, 7, 8], true));
    }
    out.extend([0, 0, 1, 0xB9]);
    out
}

#[test]
fn hostile_input_never_panics() {
    let mut seeds = vec![ts_seed(false), ts_seed(true), ps_seed()];
    seeds.push(work_tests::big_program_stream(false, 40).0);
    seeds.push(work_tests::big_program_stream(true, 40).0);
    // fewer rounds than FilmCraft's 40 000: tests here build without optimisation, and the two
    // large program streams cost a megabyte of window refill per unit read
    for (i, s) in seeds.iter().enumerate() {
        craft_media_testkit::hostile::hammer(&format!("mpegts seed {i}"), s, if i < 3 { 4_000 } else { 150 }, exercise);
    }
}

#[test]
fn elementary_stream_frame_parsers_never_panic() {
    let codecs = [
        Codec::Mpeg1Video,
        Codec::Mpeg2Video,
        Codec::H264,
        Codec::Hevc,
        Codec::MpegAudio,
        Codec::AacAdts,
        Codec::AacLatm,
        Codec::Ac3,
        Codec::Eac3,
        Codec::Dts,
        Codec::TrueHd,
        Codec::LpcmBluray,
        Codec::LpcmDvd,
        Codec::Unknown(0x55),
    ];
    let mut ac3 = vec![0x0B, 0x77, 0, 0, 30, 0x40];
    ac3.extend([1u8; 60]);
    let seeds = [adts_frame(3), mp2_frame(1), ac3, mpeg2_frame(1, 0, false, 64), vec![0x56, 0xE0, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80]];
    for (i, s) in seeds.iter().enumerate() {
        craft_media_testkit::hostile::hammer(&format!("es seed {i}"), s, 40_000, |d| {
            for c in &codecs {
                let _ = frame_bytes(c, d);
                let _ = frame_info(c, d);
            }
        });
    }
}
