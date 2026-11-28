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

/// A source that counts the bytes read from it.
struct Counted(Vec<u8>, std::cell::Cell<u64>);

impl ByteSource for Counted {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.1.set(self.1.get() + buf.len() as u64);
        self.0.read_at(offset, buf)
    }
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

/// Opus header pages for `serial` (with the beginning-of-stream flag), then data pages of one
/// 960-sample packet of `pk_len` bytes each; sequence numbers continue from the header pages.
fn opus_pages(f: &mut Vec<u8>, serial: u32, npk: usize, pk_len: usize, eos: bool) {
    let head = opus_head(0);
    let (l, b) = lace(&[&head], false);
    page(f, 0x02, 0, serial, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags\x00\x00\x00\x00\x00\x00\x00\x00"], false);
    page(f, 0, 0, serial, 1, &l, &b);
    for i in 0..npk {
        let p = pkt(i, pk_len);
        let (l, b) = lace(&[&p], false);
        let last = eos && i + 1 == npk;
        page(f, if last { 0x04 } else { 0 }, (i as i64 + 1) * 960, serial, i as u32 + 2, &l, &b);
    }
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

/// A data packet from a `PacketReader`: its stream, the packet and its bytes.
type ReadPacket = (usize, Packet, Vec<u8>);

/// Every data packet a `PacketReader` returns, and the reader after the end of the file.
fn read_all<S: ByteSource>(src: S) -> (Vec<ReadPacket>, PacketReader<S>) {
    let mut r = PacketReader::new(src).unwrap();
    let mut out = Vec::new();
    let mut buf = Vec::new();
    while let Some((s, p)) = r.next_packet(&mut buf).unwrap() {
        out.push((s, p, buf.clone()));
    }
    (out, r)
}

/// The reader gives the streams, packets and warnings `open` gives, and keeps no packets.
fn assert_reader_matches_open(f: &[u8]) {
    let o = open(f).unwrap();
    let (got, r) = read_all(f);
    assert_eq!(r.streams().len(), o.streams.len());
    for (s, (rs, os)) in r.streams().iter().zip(&o.streams).enumerate() {
        assert_eq!((rs.serial, rs.codec, &rs.headers, rs.pages, rs.saw_eos), (os.serial, os.codec, &os.headers, os.pages, os.saw_eos));
        assert!(rs.packets.is_empty());
        let mine: Vec<&ReadPacket> = got.iter().filter(|g| g.0 == s).collect();
        assert_eq!(mine.len(), os.packets.len(), "stream {s}");
        for (i, (g, p)) in mine.iter().zip(&os.packets).enumerate() {
            assert_eq!(&g.1, p, "stream {s} packet {i}");
            assert_eq!(g.2, o.read_packet(f, s, i).unwrap(), "stream {s} packet {i}");
        }
    }
    // file order: packets come in the order of the pages they complete on (from a walk of the
    // pages), and within a page in their own order
    let mut d = Demux::new(f.len() as u64, None);
    let mut rank = HashMap::new();
    while let Some((s, _)) = d.next_page(f, u64::MAX).unwrap() {
        rank.insert((s, d.streams[s].pages - 1), rank.len());
        d.streams[s].packets.clear();
    }
    let mut want: Vec<(usize, &Packet)> = o.streams.iter().enumerate().flat_map(|(s, st)| st.packets.iter().map(move |p| (s, p))).collect();
    want.sort_by_key(|&(s, p)| rank[&(s, p.page)]);
    assert!(got.iter().map(|g| (g.0, &g.1)).eq(want));
    assert_eq!(r.warnings(), o.warnings);
}

/// Opus header pages of two streams with their beginning-of-stream pages first, then one data
/// page of each in turn.
fn two_streams(npk: usize) -> Vec<u8> {
    let mut f = Vec::new();
    let head = opus_head(0);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    page(&mut f, 0x02, 0, 2, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 1, 1, &l, &b);
    // a data page of stream 1 before the last header page of stream 2
    let p = pkt(0, 30);
    let (lp, bp) = lace(&[&p], false);
    page(&mut f, 0, 960, 1, 2, &lp, &bp);
    page(&mut f, 0, 0, 2, 1, &l, &b);
    for i in 1..npk {
        for serial in [1, 2] {
            let p = pkt(i * serial as usize, 30 + i % 7);
            let (l, b) = lace(&[&p], false);
            let last = i + 1 == npk;
            page(&mut f, if last { 0x04 } else { 0 }, (i as i64 + 1) * 960, serial, i as u32 + if serial == 1 { 2 } else { 1 }, &l, &b);
        }
    }
    f
}

#[test]
fn probe_reads_only_the_header_pages() {
    let mut f = Vec::new();
    opus_pages(&mut f, 3, 400, 6000, true);
    let starts = page_starts(&f);
    let src = Counted(f, Default::default());
    let o = probe(&src).unwrap();
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    let st = &o.streams[0];
    assert_eq!((st.codec, st.headers.len(), st.packets.len(), st.pages), (Codec::Opus, 2, 0, 2));
    assert_eq!(st.codec_params(), open(&src.0).unwrap().streams[0].codec_params());
    // the two header pages and their packets (19 and 16 bytes) once more, and nothing of the
    // 2.4 MB after them
    assert_eq!(src.1.get(), starts[2] as u64 + 19 + 16, "{} of {} bytes read", src.1.get(), src.len());
}

#[test]
fn probe_waits_for_the_headers_of_every_stream() {
    let f = two_streams(50);
    let src = Counted(f, Default::default());
    let o = probe(&src).unwrap();
    assert_eq!(o.streams.len(), 2);
    for st in &o.streams {
        assert_eq!((st.codec, st.headers.len()), (Codec::Opus, 2));
    }
    // the data packet before stream 2's last header page is kept, and reading stops there
    assert_eq!((o.streams[0].packets.len(), o.streams[1].packets.len()), (1, 0));
    assert_eq!(o.read_packet(&src.0, 0, 0).unwrap(), pkt(0, 30));
    assert!(src.1.get() < src.len() / 10, "{} of {} bytes read", src.1.get(), src.len());
    // a file of beginning-of-stream pages only, and no Ogg at all
    let mut g = Vec::new();
    let (l, b) = lace(&[&opus_head(0)], false);
    page(&mut g, 0x02, 0, 1, 0, &l, &b);
    assert_eq!(probe(&g).unwrap().streams[0].headers.len(), 1);
    assert!(matches!(probe(&b"not ogg at all, no capture pattern".to_vec()), Err(Error::NotOgg)));
    assert!(matches!(PacketReader::new(&b"OggS but nothing after it".to_vec()), Err(Error::NotOgg)));
}

#[test]
fn probe_does_not_wait_for_a_stream_a_new_link_replaced() {
    // stream 1 begins with a packet that never completes, then a new link reuses its serial: the
    // probe stops after the new link's header pages
    let mut f = Vec::new();
    page(&mut f, 0x02, -1, 1, 0, &[255], &[7; 255]);
    opus_pages(&mut f, 1, 50, 40, true);
    let o = probe(&f).unwrap();
    assert_eq!(o.streams.len(), 2);
    assert!(o.streams[0].headers.is_empty());
    assert_eq!((o.streams[1].codec, o.streams[1].headers.len(), o.streams[1].packets.len()), (Codec::Opus, 2, 0));
}

#[test]
fn probe_stops_waiting_after_probe_packets() {
    // stream 2 begins with a page that completes no packet and never has another, while stream 1
    // has a page of 255 empty data packets after another
    let mut f = Vec::new();
    let (l, b) = lace(&[&opus_head(0)], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    page(&mut f, 0x02, 0, 2, 0, &[], &[]);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 1, 1, &l, &b);
    for seq in 2..(PROBE_PACKETS / 255 + 40) as u32 {
        page(&mut f, 0, -1, 1, seq, &[0; 255], &[]);
    }
    let src = Counted(f, Default::default());
    let o = probe(&src).unwrap();
    assert!(o.streams[1].headers.is_empty());
    let held = o.streams[0].packets.len();
    assert!((PROBE_PACKETS..PROBE_PACKETS + 255).contains(&held), "{held}");
    assert!(src.1.get() < src.len(), "{} of {} bytes read", src.1.get(), src.len());
}

#[test]
fn a_stop_offset_bounds_the_search_past_damage() {
    // 3000 bytes of junk, then a file: a page search with a stop at byte 500 reads no further
    // than the patterns that start before it, and the next search goes on from there
    let mut f = vec![b'j'; 3000];
    f.extend(opus_file(4, 0, 0));
    let src = Counted(f, Default::default());
    let mut d = Demux::new(src.len(), None);
    assert!(d.next_page(&src, 500).unwrap().is_none());
    assert!(!d.ended && !d.found_any && d.pos == 500, "{} {} {}", d.ended, d.found_any, d.pos);
    // the first header, then the scan to byte 503 in two reads that overlap by 3 bytes
    assert_eq!(src.1.get(), 27 + 502 + 3, "{} bytes read", src.1.get());
    assert_eq!(d.next_page(&src, u64::MAX).unwrap(), Some((0, 0)));
    assert_eq!(d.pos, 3000 + page_starts(&src.0[3000..])[1] as u64);
    // pages that fail their CRC stop at the offset too
    let mut f = Vec::new();
    opus_pages(&mut f, 3, 40, 100, true);
    let starts = page_starts(&f);
    for &s in &starts[..30] {
        f[s + 30] ^= 0xFF;
    }
    let mut d = Demux::new(f.len() as u64, None);
    assert!(d.next_page(&f, starts[10] as u64).unwrap().is_none());
    assert!(!d.ended && d.pos == starts[10] as u64);
    // the first good page holds one packet, read as the stream's first header
    assert_eq!(d.next_page(&f, u64::MAX).unwrap(), Some((0, 0)));
    assert_eq!(d.pos, starts[31] as u64);
}

#[test]
fn packet_reader_returns_what_open_indexes() {
    assert_reader_matches_open(&opus_file(10, 312, 500));
    assert_reader_matches_open(&two_streams(40));
    // packets across pages, and a lost page in the middle of one
    let mut f = Vec::new();
    let (l, b) = lace(&[&opus_head(0)], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 1, 1, &l, &b);
    let big = pkt(0, 600);
    page(&mut f, 0, -1, 1, 2, &[255], &big[..255]);
    page(&mut f, 0x01, -1, 1, 3, &[255], &big[255..510]);
    let (l, b) = lace(&[&big[510..], &pkt(1, 30)], false);
    page(&mut f, 0x01, 1920, 1, 4, &l, &b);
    let (l, b) = lace(&[&pkt(2, 700)], false);
    page(&mut f, 0x04, 2880, 1, 5, &l, &b);
    assert_reader_matches_open(&f);
    let mut g = f.clone();
    g[page_starts(&f)[3] + 40] ^= 0xFF;
    assert_reader_matches_open(&g);
    // chained links that reuse a serial, then a file cut inside a packet
    let mut f = Vec::new();
    opus_pages(&mut f, 0, 3, 40, true);
    opus_pages(&mut f, 0, 3, 40, true);
    assert_reader_matches_open(&f);
    assert_reader_matches_open(&f[..f.len() - 10]);
    // damage on every other page, more than the damage warnings listed: the same pages are
    // skipped with the same warnings and the same closing count
    let mut f = Vec::new();
    opus_pages(&mut f, 3, 3000, 40, true);
    let starts = page_starts(&f);
    for k in (2..3002).step_by(2) {
        f[starts[k] + 30] ^= 0xFF;
    }
    assert_reader_matches_open(&f);
    // stream 1 begins a packet, stream 2 completes one, then stream 1 completes its packet with
    // a zero lacing value: stream 2's packet comes first, though its bytes lie after stream 1's
    let mut f = Vec::new();
    let (l, b) = lace(&[&opus_head(0)], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    page(&mut f, 0x02, 0, 2, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 1, 1, &l, &b);
    page(&mut f, 0, 0, 2, 1, &l, &b);
    page(&mut f, 0, -1, 1, 2, &[255], &pkt(0, 255));
    let (l, b) = lace(&[&pkt(1, 30)], false);
    page(&mut f, 0x04, 960, 2, 2, &l, &b);
    page(&mut f, 0x01 | 0x04, 960, 1, 3, &[0], &[]);
    assert_reader_matches_open(&f);
}

/// A source that fails the first read starting at `at`.
struct FailOnce {
    data: Vec<u8>,
    at: u64,
    armed: std::cell::Cell<bool>,
}

impl ByteSource for FailOnce {
    fn len(&self) -> u64 {
        self.data.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if offset == self.at && self.armed.replace(false) {
            return Err(io::Error::other("injected read failure"));
        }
        self.data.read_at(offset, buf)
    }
}

#[test]
fn a_read_that_fails_while_filing_headers_can_be_tried_again() {
    // a chained file whose second link has both Opus headers on its first page; reading the second
    // header fails once, after the first one is filed
    let mut f = Vec::new();
    opus_pages(&mut f, 1, 2, 40, true);
    let link = f.len();
    let head = opus_head(0);
    let tags = b"OpusTags\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
    let (l, b) = lace(&[&head, &tags], false);
    page(&mut f, 0x02, 0, 2, 0, &l, &b);
    let tags_at = (link + 27 + l.len() + head.len()) as u64;
    let (l, b) = lace(&[&pkt(5, 40)], false);
    page(&mut f, 0x04, 960, 2, 1, &l, &b);
    let src = FailOnce { data: f.clone(), at: tags_at, armed: std::cell::Cell::new(true) };
    let mut r = PacketReader::new(&src).unwrap();
    let mut buf = Vec::new();
    let mut got = Vec::new();
    let mut failed = 0;
    loop {
        match r.next_packet(&mut buf) {
            Ok(Some((s, _))) => got.push((s, buf.clone())),
            Ok(None) => break,
            Err(e) => {
                assert!(matches!(e, Error::Io(_)), "{e}");
                failed += 1;
                assert!(failed < 2, "the read failed again");
            }
        }
    }
    assert_eq!(failed, 1);
    let (want, w) = read_all(f.as_slice());
    assert_eq!(got, want.into_iter().map(|(s, _, b)| (s, b)).collect::<Vec<_>>());
    assert_eq!(r.streams().len(), 2);
    assert_eq!(r.streams()[1].headers, [head, tags]);
    assert_eq!(r.warnings(), w.warnings());
}

/// A source of zeros with `head` at its start and `tail` at `at`, for files too large to hold.
struct Sparse {
    head: Vec<u8>,
    at: u64,
    tail: Vec<u8>,
}

impl Sparse {
    /// Copy the part of `data`, placed at `at`, that `buf` at `offset` covers.
    fn place(buf: &mut [u8], offset: u64, data: &[u8], at: u64) {
        let start = offset.max(at);
        let end = (offset + buf.len() as u64).min(at + data.len() as u64);
        if start < end {
            buf[(start - offset) as usize..(end - offset) as usize].copy_from_slice(&data[(start - at) as usize..(end - at) as usize]);
        }
    }
}

impl ByteSource for Sparse {
    fn len(&self) -> u64 {
        self.at + self.tail.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if offset.checked_add(buf.len() as u64).is_none_or(|end| end > self.len()) {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        buf.fill(0);
        Sparse::place(buf, offset, &self.head, 0);
        Sparse::place(buf, offset, &self.tail, self.at);
        Ok(())
    }
}

#[test]
fn lost_sync_across_the_probe_stop_is_reported_from_where_it_began() {
    // an OpusHead page, junk, then the OpusTags page at PROBE_BYTES or just after it: the probe
    // stops in the junk, and the reader reports the lost-sync region open reports
    let mut head = Vec::new();
    let (l, b) = lace(&[&opus_head(0)], false);
    page(&mut head, 0x02, 0, 1, 0, &l, &b);
    let mut tail = Vec::new();
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut tail, 0, 0, 1, 1, &l, &b);
    let (l, b) = lace(&[&pkt(0, 30)], false);
    page(&mut tail, 0x04, 960, 1, 2, &l, &b);
    for at in [PROBE_BYTES, PROBE_BYTES + 100] {
        let src = Sparse { head: head.clone(), at, tail: tail.clone() };
        let o = open(&src).unwrap();
        assert_eq!(o.warnings, [format!("lost sync at byte {}; resumed at {at}", head.len())]);
        let (got, r) = read_all(&src);
        assert_eq!(r.streams()[0].headers, o.streams[0].headers);
        assert_eq!(got.len(), 1);
        assert_eq!(r.warnings(), o.warnings, "page at {at}");
    }
}

#[test]
fn packet_reader_keeps_no_packets_and_has_no_packet_cap() {
    // the file of `empty_packets_are_capped_by_the_file_size`, where open stops at its packet cap
    let mut f = Vec::new();
    let (l, b) = lace(&[&opus_head(0)], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    let pages = MIN_PACKET_ALLOWANCE / 255 + 10;
    for seq in 1..=pages as u32 {
        page(&mut f, 0, -1, 1, seq, &[0; 255], &[]);
    }
    let mut r = PacketReader::new(&f).unwrap();
    let mut buf = vec![1; 8];
    let mut n = 0;
    while let Some((s, p)) = r.next_packet(&mut buf).unwrap() {
        assert_eq!((s, p.size, buf.len()), (0, 0, 0));
        n += 1;
    }
    // the first empty packet is the second Opus header
    assert_eq!((n, r.streams()[0].headers.len()), (255 * pages - 1, 2));
    assert!(r.streams()[0].packets.is_empty());
    assert!(r.warnings().is_empty(), "{:?}", r.warnings());
    assert!(r.next_packet(&mut buf).unwrap().is_none());
}

#[test]
fn packet_reader_files_a_first_packet_too_large_for_a_header_once() {
    // a first packet over MAX_HEADER_BYTES is data, so the next packet must not be taken for the
    // first one and its codec
    let size = MAX_HEADER_BYTES + 1;
    let first: Vec<u8> = (0..size).map(|k| (k % 251) as u8).collect();
    let mut f = Vec::new();
    let page_body = 255 * 255;
    let full = size / page_body;
    for (seq, chunk) in first[..full * page_body].chunks(page_body).enumerate() {
        page(&mut f, if seq == 0 { 0x02 } else { 0x01 }, -1, 5, seq as u32, &[255; 255], chunk);
    }
    let (l, b) = lace(&[&first[full * page_body..]], false);
    page(&mut f, 0x01, -1, 5, full as u32, &l, &b);
    let head = opus_head(0);
    let (l, b) = lace(&[&head, &b"x"[..]], false);
    page(&mut f, 0x04, 0, 5, full as u32 + 1, &l, &b);
    let o = open(&f).unwrap();
    assert_eq!((o.streams[0].codec, o.streams[0].headers.len(), o.streams[0].packets.len()), (Codec::Unknown, 0, 3));
    let (got, r) = read_all(&f);
    assert_eq!((r.streams()[0].codec, r.streams()[0].headers.len()), (Codec::Unknown, 0));
    assert_eq!(got.len(), 3);
    assert!(got[0].2 == first && got[1].2 == head && got[2].2 == b"x");
}

#[test]
fn read_packet_into_reuses_one_buffer() {
    let f = opus_file(10, 312, 500);
    let o = open(&f).unwrap();
    let mut buf = Vec::new();
    o.read_packet_into(&f, 0, 4, &mut buf).unwrap();
    assert_eq!(buf, pkt(4, 44));
    let at = buf.as_ptr();
    o.read_packet_into(&f, 0, 0, &mut buf).unwrap();
    assert_eq!((buf.as_slice(), buf.as_ptr()), (pkt(0, 40).as_slice(), at));
    assert!(matches!(o.read_packet_into(&f, 0, 10, &mut buf), Err(Error::Invalid(_))));
}
