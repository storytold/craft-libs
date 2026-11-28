//! Opus, AV1 and MPEG-4 Part 2 streams built by hand from the carriage rules: the Opus transport
//! draft (control headers, the Opus audio descriptor), Carriage of AV1 in MPEG-2 TS (start codes,
//! emulation prevention, the AV1 video descriptor) and stream type 0x10.

use super::super::ts::opus_config;
use super::*;

const OPUS_PID: u16 = 0x1100;
const AV1_PID: u16 = 0x1200;
const MPEG4_PID: u16 = 0x1300;

/// The registration descriptor "Opus" and an Opus audio descriptor with this channel_config_code
/// and the bytes after it.
fn opus_desc(code: &[u8]) -> Vec<u8> {
    let mut d = vec![0x05, 4, b'O', b'p', b'u', b's', 0x7F, 1 + code.len() as u8, 0x80];
    d.extend_from_slice(code);
    d
}

/// An opus_control_header for `size` payload bytes.
fn control(size: usize, start: Option<u16>, end: Option<u16>, ext: Option<&[u8]>) -> Vec<u8> {
    let flags = 0xE0 | if start.is_some() { 0x10 } else { 0 } | if end.is_some() { 0x08 } else { 0 } | if ext.is_some() { 0x04 } else { 0 };
    let mut v = vec![0x7F, flags];
    v.extend(std::iter::repeat_n(0xFF, size / 255));
    v.push((size % 255) as u8);
    for t in [start, end].into_iter().flatten() {
        v.extend_from_slice(&t.to_be_bytes());
    }
    if let Some(e) = ext {
        v.push(e.len() as u8);
        v.extend_from_slice(e);
    }
    v
}

/// `n` bytes of an Opus packet: a TOC byte, then filler.
fn opus_packet(n: usize, seed: u8) -> Vec<u8> {
    let mut v = vec![0xFC];
    v.extend((1..n).map(|i| (i as u8).wrapping_mul(7) ^ seed));
    v
}

fn opus_mux(code: &[u8]) -> TsMux {
    let mut mux = TsMux::new(false);
    mux.psi_full(&[(0x06, OPUS_PID, opus_desc(code))], OPUS_PID);
    mux
}

/// A transport stream packet on `pid` that starts a PES packet and sets the random access
/// indicator; `payload` is at most 182 bytes.
fn random_access_packet(mux: &mut TsMux, pid: u16, payload: &[u8]) {
    let cc = mux.cc.entry(pid).or_insert(0);
    let al = 183 - payload.len();
    let mut p = vec![0x47, 0x40 | (pid >> 8) as u8, pid as u8, 0x30 | *cc, al as u8, 0x40];
    p.extend(std::iter::repeat_n(0xFF, al - 1));
    p.extend_from_slice(payload);
    *cc = (*cc + 1) & 15;
    mux.out.extend(p);
}

/// A transport stream packet on `pid` that sets the unit start indicator and carries only an
/// adaptation field, with the random access indicator set when `random_access` is.
fn adaptation_only_start(mux: &mut TsMux, pid: u16, random_access: bool) {
    let cc = *mux.cc.entry(pid).or_insert(0);
    let mut p = vec![0x47, 0x40 | (pid >> 8) as u8, pid as u8, 0x20 | cc, 183, if random_access { 0x40 } else { 0 }];
    p.extend(std::iter::repeat_n(0xFF, 182));
    mux.out.extend(p);
}

/// Opus, AV1 and MPEG-4 Part 2 streams in one program, for the hostile-input sweep.
pub(super) fn seed() -> Vec<u8> {
    let mut mux = TsMux::new(false);
    let av1 = vec![0x05, 4, b'A', b'V', b'0', b'1', 0x80, 4, 0x81, 0x08, 0x0C, 0xC0];
    mux.psi_full(&[(0x06, OPUS_PID, opus_desc(&[0x02])), (0x06, AV1_PID, av1), (0x10, MPEG4_PID, Vec::new())], OPUS_PID);
    for k in 0..6i64 {
        let mut au = control(40, (k == 0).then_some(312), None, Some(&[1, 2]));
        au.extend(opus_packet(40, k as u8));
        au.extend(control(3, None, None, None));
        au.extend(opus_packet(3, k as u8));
        mux.pes(OPUS_PID, &pes(0xBD, Some(1800 * k), None, &au, true));
        let tu = [0, 0, 1, 0x12, 0x00, 0, 0, 1, 0x0A, 0x04, 0, 0, 3, 0, 0x7F, 0, 0, 1, 0x30, 0x10, 0x20];
        mux.pes(AV1_PID, &pes(0xBD, Some(3600 * k), None, &tu, true));
        let vop = [0, 0, 1, 0xB6, if k == 0 { 0x10 } else { 0x50 }, 1, 2, 3];
        mux.pes(MPEG4_PID, &pes(0xE0, Some(3600 * k), None, &vop, true));
    }
    mux.out
}

#[test]
fn opus_access_units_are_cut_by_their_control_headers() {
    let mut mux = opus_mux(&[0x02]);
    let (a, b, c, d) = (opus_packet(600, 1), opus_packet(5, 2), opus_packet(7, 3), opus_packet(10, 4));
    // one PES packet: an access unit with a start trim and a control extension, a plain one, and
    // one without a control header that runs to the end of the packet
    let mut first = control(a.len(), Some(312), None, Some(&[9, 9, 9]));
    first.extend(&a);
    first.extend(control(b.len(), None, None, None));
    first.extend(&b);
    first.extend(&c);
    mux.pes(OPUS_PID, &pes(0xBD, Some(90_000), None, &first, true));
    let mut second = control(d.len(), None, Some(100), None);
    second.extend(&d);
    mux.pes(OPUS_PID, &pes(0xBD, Some(91_800), None, &second, true));
    let f = open(&mux.out).unwrap();
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    let s = &f.streams[0];
    assert_eq!((s.codec.clone(), s.kind(), s.skipped_bytes), (Codec::Opus, Kind::Audio, 0));
    let units: Vec<Vec<u8>> = (0..s.units.len()).map(|n| f.read_unit(&mux.out, 0, n).unwrap()).collect();
    assert_eq!(units, [a, b, c, d]);
    // the PES timestamps belong to the first access unit of each packet
    assert_eq!(s.units.iter().map(|u| u.pts).collect::<Vec<_>>(), [Some(90_000), None, None, Some(91_800)]);
    assert!(s.units.iter().all(|u| u.key));
    let o = s.opus.as_ref().unwrap();
    assert_eq!(o.trims, [OpusTrim { unit: 0, start: 312, end: 0 }, OpusTrim { unit: 3, start: 0, end: 100 }]);
}

#[test]
fn an_opus_control_header_may_cross_packet_boundaries() {
    // the second header starts at every offset around the end of the first TS packet's payload
    for len in 150..200 {
        let mut mux = opus_mux(&[0x01]);
        let (a, b) = (opus_packet(len, 5), opus_packet(300, 6));
        let mut au = control(a.len(), None, None, None);
        au.extend(&a);
        au.extend(control(b.len(), Some(80), Some(20), None));
        au.extend(&b);
        mux.pes(OPUS_PID, &pes(0xBD, Some(1000), None, &au, true));
        let f = open(&mux.out).unwrap();
        let s = &f.streams[0];
        assert_eq!(s.units.len(), 2, "len {len}");
        assert_eq!((f.read_unit(&mux.out, 0, 0).unwrap(), f.read_unit(&mux.out, 0, 1).unwrap()), (a, b), "len {len}");
        assert_eq!(s.opus.as_ref().unwrap().trims, [OpusTrim { unit: 1, start: 80, end: 20 }], "len {len}");
    }
}

#[test]
fn opus_access_units_past_their_packet_are_dropped() {
    let mut mux = opus_mux(&[0x02]);
    // a payload size of 1000 in a packet that holds 50 bytes of it
    let mut cut = control(1000, None, None, None);
    cut.extend(opus_packet(50, 1));
    mux.pes(OPUS_PID, &pes(0xBD, Some(0), None, &cut, true));
    // a payload size past the largest unit: 0xFF bytes for 130 MiB, then the packet ends
    let mut huge = vec![0x7F, 0xE0];
    huge.extend(std::iter::repeat_n(0xFF, 535_000));
    mux.pes(OPUS_PID, &pes(0xBD, Some(1800), None, &huge, false));
    let good = opus_packet(20, 2);
    let mut au = control(good.len(), None, None, None);
    au.extend(&good);
    mux.pes(OPUS_PID, &pes(0xBD, Some(3600), None, &au, true));
    let f = open(&mux.out).unwrap();
    let s = &f.streams[0];
    assert_eq!(s.units.len(), 1);
    assert_eq!(s.units[0].pts, Some(3600));
    assert_eq!(f.read_unit(&mux.out, 0, 0).unwrap(), good);
    assert!(s.skipped_bytes >= 50 + 535_000, "{} bytes skipped", s.skipped_bytes);
}

#[test]
fn opus_channel_configurations_follow_the_draft_table() {
    let config = |code: &[u8]| opus_config(&opus_desc(code)).map(|o| (o.channels, o.family, o.streams, o.coupled, o.mapping));
    // dual mono as one coupled stream and as two mono streams
    assert_eq!(config(&[0x00]), Some((2, 255, 1, 1, vec![0, 1])));
    assert_eq!(config(&[0x80]), Some((2, 255, 2, 0, vec![0, 1])));
    assert_eq!(config(&[0x03]), Some((3, 1, 2, 1, vec![0, 2, 1])));
    assert_eq!(config(&[0x07]), Some((7, 1, 4, 3, vec![0, 4, 1, 2, 3, 5, 6])));
    // the same orders with a mono stream per channel
    assert_eq!(config(&[0x82]), Some((2, 1, 2, 0, vec![0, 1])));
    assert_eq!(config(&[0x88]), Some((8, 1, 8, 0, vec![0, 1, 2, 3, 4, 5, 6, 7])));
    for reserved in [0x09, 0x7F, 0x89, 0xFF] {
        assert_eq!(config(&[reserved]), None, "{reserved:#x}");
    }
    // explicit: 3 channels, family 1, 2 streams (01), 1 coupled (01), mapping 0 2 1 (00 10 01)
    assert_eq!(config(&[0x81, 3, 1, 0b0101_0010, 0b0100_0000]), Some((3, 1, 2, 1, vec![0, 2, 1])));
    // a silent channel is the all-ones mapping value: 2 streams (1), none coupled (00), mapping
    // silence 0 (11 00)
    assert_eq!(config(&[0x81, 2, 255, 0b1001_1000]), Some((2, 255, 2, 0, vec![255, 0])));
    // no channels, a reserved family, more streams than channels, a mapping past the coded
    // channels, too few bytes
    assert_eq!(config(&[0x81, 0, 0]), None);
    assert_eq!(config(&[0x81, 2, 2, 0]), None);
    assert_eq!(config(&[0x81, 3, 1, 0xFF, 0xFF]), None);
    // two streams, none coupled: mapping values take 2 bits, and 2 names no coded channel
    // (1, 00, then 10 00)
    assert_eq!(config(&[0x81, 2, 255, 0b1001_0000]), None);
    assert_eq!(config(&[0x81, 3, 1]), None);
    // family 1 stops at 8 channels; family 0 at 2
    assert_eq!(config(&[0x81, 9, 1, 0, 0, 0, 0, 0]), None);
    assert_eq!(config(&[0x81, 3, 0]), None);
}

#[test]
fn the_pre_skip_counts_only_the_start_trims_that_open_the_stream() {
    let pre_skip = |trims: &[(usize, u16, u16)]| {
        let mut o = opus_config(&opus_desc(&[0x02])).unwrap();
        o.trims = trims.iter().map(|&(unit, start, end)| OpusTrim { unit, start, end }).collect();
        let head = o.opus_head();
        u16::from_le_bytes([head[10], head[11]])
    };
    assert_eq!(pre_skip(&[]), 0);
    // a lead-in longer than one access unit, then a start trim after a discontinuity
    assert_eq!(pre_skip(&[(0, 960, 0), (1, 960, 0), (2, 312, 0), (40, 500, 0)]), 2232);
    // an end trim on the first unit stops the lead-in
    assert_eq!(pre_skip(&[(0, 0, 100), (1, 312, 0)]), 0);
    assert_eq!(pre_skip(&[(3, 312, 0)]), 0);
    assert_eq!(pre_skip(&[(0, 60_000, 0), (1, 60_000, 0)]), u16::MAX);
}

#[test]
fn av1_units_come_back_as_obus() {
    let mut mux = TsMux::new(false);
    // hdr_wcg_idc 3 (11), a reserved bit (0), initial_presentation_delay_present (1), delay 3
    // (0011)
    let desc = vec![0x05, 4, b'A', b'V', b'0', b'1', 0x80, 4, 0x81, 0x08, 0x0C, 0b1101_0011];
    mux.psi_full(&[(0x06, AV1_PID, desc)], AV1_PID);
    // a temporal delimiter; a sequence header with obu_size 4 whose payload 00 00 00 7F carries
    // an emulation prevention byte, and a stray byte after it; a frame OBU without obu_size
    let key = [0, 0, 1, 0x12, 0x00, 0, 0, 1, 0x0A, 0x04, 0, 0, 3, 0, 0x7F, 0x55, 0, 0, 1, 0x30, 0x10, 0x20, 0x30];
    random_access_packet(&mut mux, AV1_PID, &pes(0xBD, Some(3600), None, &key, true));
    let next = [0, 0, 1, 0x32, 0x02, 0xAA, 0xBB];
    mux.pes(AV1_PID, &pes(0xBD, Some(7200), None, &next, true));
    // null packets, so the file holds the five packets the probe looks for
    mux.pes(0x1FFF, &[0xFF; 184 * 2]);
    let f = open(&mux.out).unwrap();
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    let s = &f.streams[0];
    assert_eq!((s.codec.clone(), s.kind(), s.codec.codec_id()), (Codec::Av1, Kind::Video, crate::CodecId::Av1));
    assert_eq!(s.av1c, Some([0x81, 0x08, 0x0C, 0b0001_0011]));
    assert_eq!(s.codec_params().extradata, [0x81, 0x08, 0x0C, 0b0001_0011]);
    assert_eq!(s.units.iter().map(|u| u.key).collect::<Vec<_>>(), [true, false]);
    assert_eq!(f.read_unit(&mux.out, 0, 0).unwrap(), [0x0A, 0x04, 0, 0, 0, 0x7F, 0x32, 0x03, 0x10, 0x20, 0x30]);
    assert_eq!(f.read_unit(&mux.out, 0, 1).unwrap(), [0x32, 0x02, 0xAA, 0xBB]);
}

#[test]
fn an_adaptation_field_only_unit_start_keeps_the_random_access_indicator_of_the_pes_packet() {
    let tu = [0, 0, 1, 0x12, 0x00, 0, 0, 1, 0x32, 0x02, 0xAA, 0xBB];
    for first_is_key in [true, false] {
        let mut mux = TsMux::new(false);
        let desc = vec![0x05, 4, b'A', b'V', b'0', b'1', 0x80, 4, 0x81, 0x08, 0x0C, 0xC0];
        mux.psi_full(&[(0x06, AV1_PID, desc)], AV1_PID);
        // the PES header ends in the next packet; an adaptation-field-only unit start with the
        // other indicator comes between the two
        let first = pes(0xBD, Some(3600), None, &tu, true);
        if first_is_key {
            random_access_packet(&mut mux, AV1_PID, &first[..5]);
        } else {
            mux.packet(AV1_PID, true, &first[..5]);
        }
        adaptation_only_start(&mut mux, AV1_PID, !first_is_key);
        mux.packet(AV1_PID, false, &first[5..]);
        mux.pes(AV1_PID, &pes(0xBD, Some(7200), None, &tu, true));
        mux.pes(0x1FFF, &[0xFF; 184 * 2]);
        let f = open(&mux.out).unwrap();
        let s = &f.streams[0];
        assert_eq!(s.units.iter().map(|u| u.key).collect::<Vec<_>>(), [first_is_key, false], "first is key: {first_is_key}");
    }
}

#[test]
fn mpeg4_part2_units_take_their_flags_from_the_first_vop() {
    let mut mux = TsMux::new(false);
    mux.psi(&[(0x10, MPEG4_PID)]);
    // visual object sequence and VOL headers, then an I-VOP; a P-VOP whose data goes on in a PES
    // packet without a timestamp; a B-VOP
    let i_vop = [0, 0, 1, 0xB0, 0x01, 0, 0, 1, 0x00, 0, 0, 1, 0x20, 0x00, 0x84, 0, 0, 1, 0xB6, 0x10, 1, 2];
    let p_vop = [0, 0, 1, 0xB6, 0x50, 3, 4];
    let more = [5u8; 300];
    let b_vop = [0, 0, 1, 0xB6, 0x90, 6, 7];
    mux.pes(MPEG4_PID, &pes(0xE0, Some(3600), None, &i_vop, true));
    mux.pes(MPEG4_PID, &pes(0xE0, Some(7200), None, &p_vop, true));
    mux.pes(MPEG4_PID, &pes(0xE0, None, None, &more, true));
    mux.pes(MPEG4_PID, &pes(0xE0, Some(10_800), None, &b_vop, true));
    let f = open(&mux.out).unwrap();
    let s = &f.streams[0];
    assert_eq!((s.codec.clone(), s.kind(), s.codec.codec_id()), (Codec::Mpeg4Part2, Kind::Video, crate::CodecId::Mpeg4Video));
    assert_eq!(s.units.iter().map(|u| (u.key, u.disposable)).collect::<Vec<_>>(), [(true, false), (false, false), (false, true)]);
    assert_eq!(f.read_unit(&mux.out, 0, 1).unwrap(), [&p_vop[..], &more].concat());
}
