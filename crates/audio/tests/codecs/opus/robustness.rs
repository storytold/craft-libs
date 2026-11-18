//! Garbage, truncated and bit-flipped packets must never panic; concealment must produce
//! well-formed output.

use std::path::PathBuf;

use craft_audio::opus::{Decoder, OpusHead, StreamDecoder};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

fn check_finite(out: &[f32]) {
    assert!(out.iter().all(|v| v.is_finite()), "non-finite output");
}

#[test]
fn random_garbage_never_panics() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    for &rate in &[48000u32, 24000, 16000, 12000, 8000] {
        for channels in 1..=2 {
            let mut dec = StreamDecoder::new(rate, channels).unwrap();
            let mut out = Vec::new();
            for i in 0..3000 {
                let len = (rng.next() % 400) as usize;
                let mut pkt: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
                if let Some(t) = pkt.first_mut() {
                    // Cycle through every TOC configuration.
                    *t = ((i % 32) as u8) << 3 | (*t & 7);
                }
                out.clear();
                let _ = dec.decode_interleaved(Some(&pkt), &mut out);
                check_finite(&out);
                if i % 17 == 0 {
                    out.clear();
                    let _ = dec.decode_interleaved(None, &mut out);
                    check_finite(&out);
                }
            }
        }
    }
}

#[test]
fn multistream_garbage_never_panics() {
    let mut h = b"OpusHead".to_vec();
    h.extend([1, 6, 0x38, 0x01, 0x80, 0xBB, 0, 0, 0, 0, 1, 4, 2, 0, 4, 1, 2, 3, 5]);
    let mut dec = Decoder::new(&h).unwrap();
    let mut rng = Rng(42);
    for _ in 0..2000 {
        let len = (rng.next() % 600) as usize;
        let pkt: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if let Ok(out) = dec.decode(Some(&pkt)) {
            assert_eq!(out.len(), 6);
            for c in &out {
                check_finite(c);
            }
        }
    }
    let out = dec.decode(None).unwrap();
    assert_eq!(out.len(), 6);
}

#[test]
fn concealment_before_and_after_packets() {
    let mut dec = Decoder::from_head(OpusHead::simple(2).unwrap(), 48000).unwrap();
    // Loss before any packet: silence of a default duration.
    let out = dec.decode(None).unwrap();
    assert_eq!(out.len(), 2);
    assert!(out[0].iter().all(|&v| v == 0.0));
}

/// The RFC 8251 vectors, or `None` after printing `SKIPPED` (a failure with
/// `CRAFT_MEDIA_REQUIRE_ORACLES=1`). `scripts/fetch-opus-vectors.sh` fetches them.
fn vectors_dir() -> Option<PathBuf> {
    let d = crate::common::fixture_subdir("opus").join("vectors");
    if d.join("testvector01.bit").exists() {
        return Some(d);
    }
    crate::common::skip(&format!("the Opus RFC 8251 vectors are not in {} (scripts/fetch-opus-vectors.sh fetches them)", d.display()))
}

/// Real packets (from the conformance vectors) truncated and bit-flipped, interleaved with losses.
#[test]
fn corrupted_real_packets() {
    let Some(dir) = vectors_dir() else {
        return;
    };
    let mut rng = Rng(7);
    for v in [1, 2, 5, 12] {
        let data = std::fs::read(dir.join(format!("testvector{v:02}.bit"))).unwrap();
        let mut dec = StreamDecoder::new(48000, 2).unwrap();
        let mut out = Vec::new();
        let mut p = 0;
        let mut n = 0;
        while p + 8 <= data.len() && n < 1500 {
            let len = u32::from_be_bytes(data[p..p + 4].try_into().unwrap()) as usize;
            p += 8;
            let mut pkt = data[p..(p + len).min(data.len())].to_vec();
            p += len;
            n += 1;
            match rng.next() % 4 {
                0 => {
                    let keep = (rng.next() as usize) % (pkt.len() + 1);
                    pkt.truncate(keep);
                }
                1 if !pkt.is_empty() => {
                    for _ in 0..3 {
                        let i = rng.next() as usize % pkt.len();
                        pkt[i] ^= 1 << (rng.next() % 8);
                    }
                }
                2 => pkt.clear(),
                _ => {}
            }
            out.clear();
            let _ = dec.decode_interleaved(if pkt.is_empty() { None } else { Some(&pkt) }, &mut out);
            check_finite(&out);
        }
    }
}

/// An Ogg page holding `packet` whole (RFC 3533), with its CRC.
fn ogg_page(packet: &[u8], flags: u8, granule: u64, seq: u32) -> Vec<u8> {
    let mut p = b"OggS\0".to_vec();
    p.push(flags);
    p.extend_from_slice(&granule.to_le_bytes());
    p.extend_from_slice(&1u32.to_le_bytes());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(&[0; 4]);
    let mut lacing = vec![255u8; packet.len() / 255];
    lacing.push((packet.len() % 255) as u8);
    p.push(lacing.len() as u8);
    p.extend_from_slice(&lacing);
    p.extend_from_slice(packet);
    let mut crc = 0u32;
    for &b in &p {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    p[22..26].copy_from_slice(&crc.to_le_bytes());
    p
}

#[test]
fn ogg_with_a_malformed_opus_head_reports_it() {
    // Mapping family 0 with three channels: RFC 7845 §5.1.1 allows at most two.
    let mut head = b"OpusHead\x01\x03".to_vec();
    head.extend_from_slice(&312u16.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&[0, 0, 0]);
    let mut file = ogg_page(&head, 2, 0, 0);
    file.extend(ogg_page(b"OpusTags\0\0\0\0\0\0\0\0", 0, 0, 1));
    // One 20 ms silent CELT frame.
    file.extend(ogg_page(&[0xF8, 0xFF, 0xFE], 4, 960, 2));
    match craft_audio::decode(&file, Some("opus")) {
        Err(craft_audio::AudioError::Malformed(m)) => assert!(m.contains("family 0"), "{m}"),
        other => panic!("expected a malformed-header error, got {:?}", other.map(|(info, _)| info)),
    }
}
