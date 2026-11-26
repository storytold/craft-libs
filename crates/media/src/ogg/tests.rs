//! Synthetic Ogg streams: a small page writer (RFC 3533) and Opus-like packets.

use super::*;

#[test]
fn recovery_charges_only_bytes_read_again() {
    let mut r = Recovery { budget: 10, seen: 0 };
    assert!(r.charge(0, 100));
    assert_eq!(r.budget, 10, "a first read is free");
    assert!(r.charge(95, 105));
    assert_eq!((r.budget, r.seen), (5, 105));
    assert!(!r.charge(0, 6));
    assert_eq!(r.budget, 0);
    // a scan of bytes not read before costs nothing; one over bytes already read is charged and
    // says when the budget ran out
    let source = b"junkOggS".to_vec();
    let mut r = Recovery { budget: 0, seen: 0 };
    assert_eq!(find_capture(&source, 0, 8, &mut r).unwrap(), Capture::At(4));
    let mut r = Recovery { budget: 7, seen: 8 };
    assert_eq!(find_capture(&source, 0, 8, &mut r).unwrap(), Capture::BudgetSpent);
    let mut r = Recovery { budget: 8, seen: 8 };
    assert_eq!(find_capture(&source, 0, 8, &mut r).unwrap(), Capture::At(4));
    let mut r = Recovery::new(16);
    assert_eq!(find_capture(&b"junk, no pattern".to_vec(), 0, 16, &mut r).unwrap(), Capture::End);
}

/// Write one page holding `segments` (lacing already computed by `lace`).
fn page(out: &mut Vec<u8>, flags: u8, granule: i64, serial: u32, seq: u32, lacing: &[u8], body: &[u8]) {
    let start = out.len();
    out.extend_from_slice(b"OggS");
    out.push(0);
    out.push(flags);
    out.extend_from_slice(&granule.to_le_bytes());
    out.extend_from_slice(&serial.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.push(lacing.len() as u8);
    out.extend_from_slice(lacing);
    out.extend_from_slice(body);
    let crc = crc32(&out[start..]);
    out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
}

fn lace(packets: &[&[u8]], last_continues: bool) -> (Vec<u8>, Vec<u8>) {
    let mut l = Vec::new();
    let mut b = Vec::new();
    for (i, p) in packets.iter().enumerate() {
        let mut n = p.len();
        while n >= 255 {
            l.push(255);
            n -= 255;
        }
        if !(last_continues && i + 1 == packets.len()) {
            l.push(n as u8);
        }
        b.extend_from_slice(p);
    }
    (l, b)
}

fn opus_head(pre_skip: u16) -> Vec<u8> {
    let mut h = b"OpusHead".to_vec();
    h.push(1);
    h.push(2);
    h.extend_from_slice(&pre_skip.to_le_bytes());
    h.extend_from_slice(&48_000u32.to_le_bytes());
    h.extend_from_slice(&0i16.to_le_bytes());
    h.push(0);
    h
}

/// A CELT 20 ms packet (960 samples): TOC config 31, code 0.
fn pkt(i: usize, len: usize) -> Vec<u8> {
    let mut p = vec![0xF8];
    p.extend((0..len.saturating_sub(1)).map(|k| (i * 7 + k) as u8));
    p
}

fn opus_file(npk: usize, pre_skip: u16, trim: i64) -> Vec<u8> {
    opus_file_with(npk, pre_skip, |done, last| done as i64 * 960 - if last { trim } else { 0 })
}

/// Like `opus_file`, with the granule of each data page chosen by `granule(packets_done, is_last_page)`.
fn opus_file_with(npk: usize, pre_skip: u16, granule: impl Fn(usize, bool) -> i64) -> Vec<u8> {
    let mut f = Vec::new();
    let head = opus_head(pre_skip);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 9, 0, &l, &b);
    let tags = b"OpusTags\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
    let (l, b) = lace(&[&tags], false);
    page(&mut f, 0, 0, 9, 1, &l, &b);
    let pkts: Vec<Vec<u8>> = (0..npk).map(|i| pkt(i, 40 + i % 5)).collect();
    let mut done = 0;
    for (seq, chunk) in (2..).zip(pkts.chunks(3)) {
        done += chunk.len();
        let last = done == npk;
        let g = granule(done, last);
        let refs: Vec<&[u8]> = chunk.iter().map(Vec::as_slice).collect();
        let (l, b) = lace(&refs, false);
        page(&mut f, if last { 0x04 } else { 0 }, g, 9, seq, &l, &b);
    }
    f
}

#[test]
fn packets_spanning_pages() {
    let mut f = Vec::new();
    let head = opus_head(0);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 1, 1, &l, &b);
    let big = pkt(0, 700);
    // first 510 bytes on one page (two 255 lacing values, packet continues), rest on the next
    let (l, b) = (vec![255, 255], big[..510].to_vec());
    page(&mut f, 0, -1, 1, 2, &l, &b);
    let rest = &big[510..];
    let small = pkt(1, 30);
    let (l, b) = lace(&[rest, &small], false);
    page(&mut f, 0x01 | 0x04, 1920, 1, 3, &l, &b);
    let o = open(&f).unwrap();
    let st = &o.streams[0];
    assert_eq!(st.packets.len(), 2);
    assert_eq!(st.packets[0].parts.len(), 2);
    assert_eq!(o.read_packet(&f, 0, 0).unwrap(), big);
    assert_eq!(o.read_packet(&f, 0, 1).unwrap(), small);
    let t = OpusTiming::of(st, 0);
    assert_eq!(t.starts, vec![0, 960]);
    assert_eq!(t.total, 1920);
}

fn page_starts(f: &[u8]) -> Vec<usize> {
    f.windows(4).enumerate().filter(|(_, w)| *w == b"OggS").map(|(i, _)| i).collect()
}

#[test]
fn a_lost_middle_page_drops_the_packet_it_continued() {
    let mut f = Vec::new();
    let head = opus_head(0);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 1, 1, &l, &b);
    // one packet over three pages: 255 bytes, 255 bytes, then the last 90 with a whole packet
    let big = pkt(0, 600);
    page(&mut f, 0, -1, 1, 2, &[255], &big[..255]);
    page(&mut f, 0x01, -1, 1, 3, &[255], &big[255..510]);
    let small = pkt(1, 30);
    let (l, b) = lace(&[&big[510..], &small], false);
    page(&mut f, 0x01 | 0x04, 1920, 1, 4, &l, &b);
    let mut g = f.clone();
    g[page_starts(&f)[3] + 40] ^= 0xFF; // the middle page fails its CRC
    let o = open(&g).unwrap();
    assert!(o.warnings.iter().any(|w| w.contains("pages missing")), "{:?}", o.warnings);
    assert_eq!(o.streams[0].packets.len(), 1);
    assert_eq!(o.read_packet(&g, 0, 0).unwrap(), small);
}

#[test]
fn opus_single_page_clip_is_trimmed_at_the_end() {
    let mut f = Vec::new();
    let head = opus_head(312);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 5, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 5, 1, &l, &b);
    let pkts: Vec<Vec<u8>> = (0..3).map(|i| pkt(i, 40)).collect();
    let refs: Vec<&[u8]> = pkts.iter().map(Vec::as_slice).collect();
    let (l, b) = lace(&refs, false);
    page(&mut f, 0x04, 2000, 5, 2, &l, &b);
    let t = OpusTiming::of(&open(&f).unwrap().streams[0], 312);
    assert_eq!(t.starts, vec![-312, 648, 1608]);
    assert_eq!(t.total, 2000 - 312); // 3 × 960 − 2000 = 880 samples trimmed at the end
}

/// Bit-at-a-time CRC of RFC 3533 §6 (polynomial 0x04C11DB7, initial value 0, no reflection, no final XOR).
fn crc32_bitwise(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

#[test]
fn crc_matches_bitwise_reference_on_every_length_and_alignment() {
    let mut seed = 7u64;
    let buf: Vec<u8> = (0..70_100)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u8
        })
        .collect();
    // every short length (covers the 16-byte main loop and every tail length), at several start offsets
    for start in 0..4 {
        for len in 0..=64 {
            let d = &buf[start..start + len];
            assert_eq!(crc32(d), crc32_bitwise(0, d), "start {start} len {len}");
        }
    }
    for len in [65, 255, 4096, 65_307, 70_000] {
        for start in [0, 1, 3] {
            let d = &buf[start..start + len];
            assert_eq!(crc32(d), crc32_bitwise(0, d), "start {start} len {len}");
        }
    }
    // all-zero and all-ones input
    assert_eq!(crc32(&[0u8; 100]), 0);
    assert_eq!(crc32(&[0xFFu8; 100]), crc32_bitwise(0, &[0xFFu8; 100]));
}

#[test]
fn opus_timing_includes_packets_after_the_last_known_granule() {
    let f = opus_file(6, 312, 0);
    let o = open(&f[..]).unwrap();
    let mut stream = o.streams[0].clone();
    // Missing granules on complete-packet pages are non-conforming. The recovery fallback still
    // assigns their packet positions; the total must include the same samples.
    stream.packets.last_mut().unwrap().granule = None;
    let timing = OpusTiming::of(&stream, 312);
    assert_eq!(timing.starts, vec![-312, 648, 1608, 2568, 3528, 4488]);
    assert_eq!(timing.total, 6 * 960 - 312);

    // With no usable granule, every complete packet still contributes its TOC duration.
    for packet in &mut stream.packets {
        packet.granule = None;
    }
    let timing = OpusTiming::of(&stream, 312);
    assert_eq!(timing.starts, vec![-312, 648, 1608, 2568, 3528, 4488]);
    assert_eq!(timing.total, 6 * 960 - 312);
}

/// The timing must be defined, bounded and panic-free for any granule a file can carry.
fn assert_sane_timing(t: &OpusTiming, npk: usize) {
    assert_eq!(t.starts.len(), npk);
    // treated as unknown granules: the packets simply follow one another from the start
    for (k, s) in t.starts.iter().enumerate() {
        assert_eq!(*s, k as i64 * 960 - 312, "packet {k}");
    }
    // as for a truncated file with no usable granule, the total stays 0: it never exceeds what the packets add up to
    assert!((0..=npk as i64 * 960).contains(&t.total), "total {}", t.total);
}

#[test]
fn opus_timing_ignores_a_granule_near_i64_max() {
    // panicked with "attempt to add with overflow" in debug builds, wrapped in release
    let f = opus_file_with(10, 312, |done, _| i64::MAX - 10 + done as i64 / 3);
    let o = open(&f).unwrap();
    assert_sane_timing(&OpusTiming::of(&o.streams[0], 312), 10);
}

#[test]
fn opus_timing_ignores_a_granule_of_i64_min() {
    // panicked with "attempt to subtract with overflow" in debug builds
    for g in [i64::MIN, i64::MIN + 1, -2] {
        let f = opus_file_with(10, 312, |_, _| g);
        let o = open(&f).unwrap();
        assert_sane_timing(&OpusTiming::of(&o.streams[0], 312), 10);
    }
}

#[test]
fn opus_timing_with_a_bad_granule_on_one_page_recovers_on_the_next() {
    // pages of 3 packets: the second page's granule is absurd, the others are right
    let f = opus_file_with(9, 0, |done, _| if done == 6 { i64::MAX } else { done as i64 * 960 });
    let o = open(&f).unwrap();
    let t = OpusTiming::of(&o.streams[0], 0);
    let expect: Vec<i64> = (0..9).map(|k| k * 960).collect();
    assert_eq!(t.starts, expect);
    assert_eq!(t.total, 9 * 960);
}

#[test]
fn largest_trusted_granule_with_a_huge_pre_skip_stays_in_range() {
    let f = opus_file_with(6, 0, |done, _| MAX_OPUS_GRANULE - 6 * 960 + done as i64 * 960);
    let o = open(&f).unwrap();
    let t = OpusTiming::of(&o.streams[0], u32::MAX);
    // shift = base (the first raw start, 2^62 - 5760) + u32::MAX; every start is relative to it
    let expect: Vec<i64> = (0..6).map(|k| k * 960 - u32::MAX as i64).collect();
    assert_eq!(t.starts, expect);
    // the end (2^62) lies before the shift, so nothing is playable
    assert_eq!(t.total, 0);
}
