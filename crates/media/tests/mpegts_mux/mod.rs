//! Frames for the read-back tests, and a packet-level check of what the muxer wrote.

#![allow(dead_code)]

use std::collections::HashMap;

use craft_media::mpegts::mux::{Codec, Frame, Muxer, PACKET_SIZE};

/// A frame queued for the muxer, with the bytes the demuxer should give back for it.
pub struct Item {
    pub stream: usize,
    pub data: Vec<u8>,
    pub pts: i64,
    pub dts: i64,
    pub key: bool,
    pub expect: Vec<u8>,
}

/// Writes `items` in DTS order (stable, so equal DTS keep their order).
pub fn write_all(m: &mut Muxer<Vec<u8>>, items: &mut [Item]) {
    items.sort_by_key(|i| i.dts);
    for i in items.iter() {
        let dts = (i.dts != i.pts).then_some(i.dts);
        m.write_frame(i.stream, &Frame { data: &i.data, pts: i.pts, dts, key: i.key }).unwrap();
    }
}

pub fn h264(key: bool, disposable: bool, size: usize, seed: u8) -> Vec<u8> {
    let mut v = if key {
        vec![0, 0, 0, 1, 0x67, 0x42, 0xC0, 0x1E, 0xDA, 0, 0, 0, 1, 0x68, 0xCE, 0x3C, 0x80, 0, 0, 1, 0x65, 0x88, 0x84]
    } else if disposable {
        vec![0, 0, 0, 1, 0x01, 0x9E]
    } else {
        vec![0, 0, 0, 1, 0x41, 0x9A]
    };
    fill(&mut v, size, seed);
    v
}

pub fn hevc(key: bool, size: usize, seed: u8) -> Vec<u8> {
    let mut v = if key {
        vec![0, 0, 0, 1, 0x40, 0x01, 0x0C, 0, 0, 0, 1, 0x42, 0x01, 0x01, 0, 0, 0, 1, 0x44, 0x01, 0xC1, 0, 0, 1, 0x26, 0x01, 0xAF]
    } else {
        vec![0, 0, 0, 1, 0x02, 0x01, 0xD0]
    };
    fill(&mut v, size, seed);
    v
}

/// An MPEG-2 picture (I with a sequence and GOP header, or P), `size` bytes.
pub fn mpeg2(key: bool, temporal_reference: u16, size: usize, seed: u8) -> Vec<u8> {
    let mut v = Vec::new();
    if key {
        v.extend_from_slice(&[0, 0, 1, 0xB3, 0x14, 0x00, 0xF0, 0x13, 0xFF, 0xFF, 0xE0, 0x18]);
        v.extend_from_slice(&[0, 0, 1, 0xB8, 0x00, 0x08, 0x00, 0x00]);
    }
    let pct: u8 = if key { 1 } else { 2 };
    let tr = temporal_reference & 0x3FF;
    v.extend_from_slice(&[0, 0, 1, 0x00, (tr >> 2) as u8, ((tr & 3) as u8) << 6 | pct << 3 | 0x07, 0xFF, 0xF8]);
    v.extend_from_slice(&[0, 0, 1, 0x01, 0x12]);
    fill(&mut v, size, seed);
    v
}

/// Pads with bytes that never form a start code.
pub fn fill(v: &mut Vec<u8>, size: usize, seed: u8) {
    let mut x = seed as u32 | 1;
    while v.len() < size {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        v.push(((x >> 16) as u8).max(1));
    }
}

/// An ADTS frame: AAC LC, 48 kHz, 2 channels, `payload` bytes after the header.
pub fn adts(payload: usize, seed: u8) -> Vec<u8> {
    let len = payload + 7;
    let mut v = vec![0xFF, 0xF1, 0x4C, 0x80 | (len >> 11) as u8, (len >> 3) as u8, ((len & 7) as u8) << 5 | 0x1F, 0xFC];
    fill(&mut v, len, seed);
    v
}

/// An AC-3 sync frame: 48 kHz, 64 kbit/s, 256 bytes.
pub fn ac3(seed: u8) -> Vec<u8> {
    let mut v = vec![0x0B, 0x77, 0, 0, 0x08, 0x40, 0x40];
    fill(&mut v, 256, seed);
    v
}

/// An MPEG-1 Layer III frame (48 kHz, 128 kbit/s, 384 bytes), or with `low` an MPEG-2 one
/// (24 kHz, 64 kbit/s, 192 bytes).
pub fn mp3(low: bool, seed: u8) -> Vec<u8> {
    let (mut v, size) = if low { (vec![0xFF, 0xF3, 0x84, 0x44], 192) } else { (vec![0xFF, 0xFB, 0x94, 0x44], 384) };
    fill(&mut v, size, seed);
    v
}

/// An E-AC-3 sync frame: independent substream 0, 256 bytes, 48 kHz, six blocks, 2/0, bsid 16.
pub fn eac3(seed: u8) -> Vec<u8> {
    let mut v = vec![0x0B, 0x77, 0x00, 0x7F, 0x34, 0x80];
    fill(&mut v, 256, seed);
    v
}

/// An MPEG-4 Part 2 frame: a visual object sequence, visual object, video object layer and an
/// I-VOP, or a P-VOP alone.
pub fn mpeg4(key: bool, size: usize, seed: u8) -> Vec<u8> {
    let mut v = if key {
        vec![0, 0, 1, 0xB0, 0x01, 0, 0, 1, 0xB5, 0x09, 0, 0, 1, 0x00, 0, 0, 1, 0x20, 0x00, 0x84, 0x40, 0, 0, 1, 0xB6, 0x10]
    } else {
        vec![0, 0, 1, 0xB6, 0x50]
    };
    fill(&mut v, size, seed);
    v
}

pub fn leb128(mut n: usize, v: &mut Vec<u8>) {
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            v.push(b);
            return;
        }
        v.push(b | 0x80);
    }
}

/// An AV1 temporal unit in the low overhead format: a temporal delimiter, a sequence header on
/// keyframes, and a frame OBU whose payload holds sequences that need emulation prevention.
pub fn av1(key: bool, size: usize, seed: u8) -> Vec<u8> {
    let mut v = vec![0x12, 0x00];
    if key {
        v.extend_from_slice(&[0x0A, 0x05, 0x00, 0x00, 0x00, 0x02, 0x7F]);
    }
    let mut payload = vec![0, 0, 1, 0, 0, 3, 0, 0, 0];
    fill(&mut payload, size, seed);
    v.push(0x32);
    leb128(payload.len(), &mut v);
    v.extend_from_slice(&payload);
    v
}

/// The OBUs of a temporal unit without its temporal delimiters.
pub fn av1_without_delimiters(tu: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < tu.len() {
        let h = tu[at];
        let mut i = at + 1 + usize::from(h & 4 != 0);
        let (mut size, mut shift) = (0usize, 0);
        loop {
            let b = tu[i];
            i += 1;
            size |= usize::from(b & 0x7F) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        if (h >> 3) & 0x0F != 2 {
            out.extend_from_slice(&tu[at..i + size]);
        }
        at = i + size;
    }
    out
}

/// Takes the start codes and emulation prevention bytes out of an AV1 PES payload, checking
/// that no 0x000000, 0x000001 or 0x000002 appears inside an OBU and that every 0x000003 is
/// followed by 0x00 to 0x03.
pub fn av1_unescape(es: &[u8]) -> Vec<u8> {
    assert!(es.starts_with(&[0, 0, 1]));
    let mut out = Vec::new();
    let (mut zeros, mut i) = (0, 0);
    while i < es.len() {
        if zeros == 0 && es[i..].starts_with(&[0, 0, 1]) {
            i += 3;
            continue;
        }
        let b = es[i];
        i += 1;
        if zeros >= 2 {
            assert!(b >= 3, "0x0000{b:02X} inside an OBU");
            zeros = 0;
            if b == 3 {
                assert!(es.get(i).is_some_and(|&n| n <= 3), "0x000003 followed by a byte that needs no escape");
                continue;
            }
            out.push(b);
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

/// The Opus control header and packet the demuxer gives back for an Opus frame.
pub fn opus_unit(packet: &[u8]) -> Vec<u8> {
    let mut v = vec![0x7F, 0xE0];
    v.extend(std::iter::repeat_n(0xFF, packet.len() / 255));
    v.push((packet.len() % 255) as u8);
    v.extend_from_slice(packet);
    v
}

/// What the demuxer gives back for a video frame: an access unit delimiter first.
pub fn with_aud(codec: Codec, data: &[u8]) -> Vec<u8> {
    let mut v = match codec {
        Codec::H264 => vec![0, 0, 0, 1, 0x09, 0xF0],
        Codec::Hevc => vec![0, 0, 0, 1, 0x46, 0x01, 0x50],
        _ => Vec::new(),
    };
    v.extend_from_slice(data);
    v
}

/// Checks the packet layer: sync bytes, continuity counters, adaptation fields, and the PCR on
/// `pcr_pid` never more than `limit` (27 MHz) apart. Returns the PCRs.
pub fn check_packets(ts: &[u8], pcr_pid: u16, limit: i64) -> Vec<i64> {
    assert_eq!(ts.len() % PACKET_SIZE, 0);
    let mut cc: HashMap<u16, u8> = HashMap::new();
    let mut pcrs = Vec::new();
    for p in ts.chunks(PACKET_SIZE) {
        assert_eq!(p[0], 0x47);
        assert_eq!(p[1] & 0x80, 0, "transport error indicator");
        assert_eq!(p[3] & 0xC0, 0, "scrambling");
        let pid = ((p[1] & 0x1F) as u16) << 8 | p[2] as u16;
        let afc = p[3] >> 4 & 3;
        assert_ne!(afc, 0);
        let payload = afc & 1 != 0;
        if let Some(&c) = cc.get(&pid) {
            assert_eq!(p[3] & 0x0F, if payload { (c + 1) & 0x0F } else { c }, "continuity on PID {pid:#x}");
        }
        cc.insert(pid, p[3] & 0x0F);
        if afc & 2 != 0 {
            let len = p[4] as usize;
            assert!(if payload { len <= 182 } else { len == 183 });
            if len > 0 && p[5] & 0x10 != 0 {
                assert!(len >= 7, "PCR flag in an adaptation field of {len} byte{}", if len == 1 { "" } else { "s" });
                assert_eq!(pid, pcr_pid, "a PCR off the PCR PID");
                let b = &p[6..12];
                assert_eq!(b[4] & 0x7E, 0x7E, "reserved bits");
                let base = (b[0] as i64) << 25 | (b[1] as i64) << 17 | (b[2] as i64) << 9 | (b[3] as i64) << 1 | (b[4] >> 7) as i64;
                let ext = ((b[4] & 1) as i64) << 8 | b[5] as i64;
                assert!(ext < 300);
                pcrs.push(base * 300 + ext);
            }
            if len > 1 {
                assert!(p[6 + if p[5] & 0x10 != 0 { 6 } else { 0 }..5 + len].iter().all(|&b| b == 0xFF), "stuffing bytes");
            }
        }
    }
    let wrap = (1i64 << 33) * 300;
    for w in pcrs.windows(2) {
        let d = (w[1] - w[0]).rem_euclid(wrap);
        assert!(d > 0 && d <= limit, "PCRs {} apart (limit {limit})", d);
    }
    pcrs
}
